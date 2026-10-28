//! Projectiles: objects a move spawns that move on their own and hit the opponent.
//!
//! Plain ones are described entirely by data (travel, force, friction, gravity, events, hitboxes).
//! The few whose original scripts steer them have a behavior id (`wizard.spark_bomb`...) handled
//! here. Every rule below was checked against traces of the original game (tools/oracle).

use std::sync::Arc;

use charfmt::Spawn;

use crate::{fx, Act, Character, FVec, Fixed, World};

/// A live projectile.
#[derive(Clone, Debug)]
pub struct Proj {
    pub owner: usize,
    /// Index into the owner's character `projectiles`.
    pub idx: usize,
    /// The move that spawned it (for the hit events' move field).
    pub from_move: usize,
    pub pos: FVec,
    pub vel: FVec,
    pub(crate) force: FVec,
    pub(crate) friction: FVec,
    pub(crate) hitlag: u16,
    pub facing: i64,
    /// Ticks it has run (frozen while it is in hitlag).
    pub age: u16,
    /// The behavior's phase (spark bomb: 0 floating, 1 armed, 2 exploding) and its own tick in it.
    pub phase: u8,
    pub(crate) ptick: i32,
    pub(crate) connected: Vec<(String, u16)>,
    pub(crate) dead: bool,
    /// A stable id (objects are found by it, and named after it).
    pub uid: u32,
    /// Scripted objects: the state (index into the definition's `states`), its frame (1-based,
    /// 0 before its first tick), the name scripts know it by, and what its main sprite shows.
    pub state: usize,
    pub sframe: u16,
    pub obj_name: String,
    pub grounded: bool,
    pub anim: String,
    pub frame: u32,
    scripted: bool,
}

/// Projectiles whose data gives no lifetime still end after this many frames.
const MAX_PROJ_AGE: u16 = 240;

/// Friction on one axis. Below a speed of 0.05 the original stops applying it, so a slow object
/// keeps a little speed instead of sliding to a halt.
fn slow(v: Fixed, friction: Fixed) -> Fixed {
    if v.abs() > Fixed::from_f64(0.05) {
        v * (Fixed::ONE - friction)
    } else {
        v
    }
}

impl Proj {
    pub(crate) fn spawn(ch: &Arc<Character>, owner: usize, from_move: usize, s: &Spawn, pos: FVec, facing: i64) -> Option<Proj> {
        let idx = ch.file.character.projectiles.iter().position(|p| p.id == s.projectile)?;
        let d = &ch.file.character.projectiles[idx];
        let fac = Fixed::from_int(facing);
        Some(Proj {
            owner,
            idx,
            from_move,
            pos: FVec::new(pos.x + fac * Fixed::from_int(s.offset.0 as i64), pos.y + Fixed::from_int(s.offset.1 as i64)),
            vel: FVec::new(fac * fx(d.velocity.0), fx(d.velocity.1)),
            force: FVec::new(fac * fx(d.force.0), fx(d.force.1)),
            friction: FVec::new(fx(d.friction.0), fx(d.friction.1)),
            hitlag: 0,
            facing,
            age: 0,
            phase: 0,
            ptick: -1,
            connected: Vec::new(),
            dead: false,
            uid: 0,
            state: 0,
            sframe: 0,
            obj_name: String::new(),
            grounded: false,
            anim: String::new(),
            frame: 0,
            scripted: false,
        })
    }

    /// A scripted object (see `objects.rs`) of definition `idx`.
    pub(crate) fn scripted(owner: usize, idx: usize, from_move: usize, pos: FVec, facing: i64, uid: u32) -> Proj {
        Proj {
            owner,
            idx,
            from_move,
            pos,
            vel: FVec::ZERO,
            force: FVec::ZERO,
            friction: FVec::ZERO,
            hitlag: 0,
            facing,
            age: 0,
            phase: 0,
            ptick: -1,
            connected: Vec::new(),
            dead: false,
            uid,
            state: 0,
            sframe: 0,
            obj_name: format!("P{}_Obj{uid}", owner + 1),
            grounded: pos.y >= Fixed::ZERO,
            anim: String::new(),
            frame: 0,
            scripted: true,
        }
    }

    /// Run by its scripts (a scripted object) rather than by data.
    pub fn is_object(&self) -> bool {
        self.scripted
    }

    /// The frame its hitboxes' windows are read at (None: no hitbox is live right now).
    pub(crate) fn hit_frame(&self, behavior: Option<&str>) -> Option<u16> {
        match behavior {
            // only the exploding state has a hitbox, and its windows count from its own first tick
            Some("wizard.spark_bomb") => (self.phase == 2).then(|| self.ptick.max(0) as u16 + 2),
            _ => Some(self.age + 1),
        }
    }
}

impl World {
    pub(crate) fn step_projectiles(&mut self) {
        let half = Fixed::from_int(self.cfg.stage_half_width as i64);
        let g = self.rules.gravity_pct as f64 / 100.0;
        let centers = [self.hurtboxes(0)[0].0, self.hurtboxes(1)[0].0];
        let mut projs = std::mem::take(&mut self.projs);
        for p in projs.iter_mut().filter(|p| !p.scripted) {
            let ch = self.chars[p.owner].clone();
            let d = &ch.file.character.projectiles[p.idx];
            if p.hitlag > 0 {
                p.hitlag -= 1; // frozen by its own hit
                continue;
            }
            match d.behavior.as_deref() {
                Some("wizard.spark_bomb") => {
                    self.step_spark_bomb(p, centers[1 - p.owner]);
                    continue;
                }
                Some("robo.missile") => {
                    self.step_missile(p, g);
                    continue;
                }
                _ => {}
            }
            // like the original, ground and walls are checked at the start of the tick, from
            // where the last tick left the object
            let moving = p.vel != FVec::ZERO || p.force != FVec::ZERO;
            if moving && (p.pos.y >= Fixed::ZERO && p.vel.y > Fixed::ZERO || p.pos.x.abs() >= half) {
                p.dead = true;
                continue;
            }
            p.age += 1;
            if let Some(z) = &d.zone {
                // a state tick of the projectile is `age - 1`
                let tick = p.age - 1;
                let v = &mut self.f[1 - p.owner];
                if tick % z.every.max(1) == 0
                    && (v.pos.x - p.pos.x).abs() < fx(z.half_width)
                    && v.grounded
                    && v.invuln == 0
                    && !matches!(v.act, Act::JumpSquat { .. })
                {
                    v.hp -= z.damage;
                }
            }
            for e in d.events.iter().filter(|e| e.at + 1 == p.age) {
                match e.kind {
                    charfmt::ProjEventKind::SetFriction(v) => p.friction = FVec::new(fx(v.0), fx(v.1)),
                    charfmt::ProjEventKind::Disable => p.dead = true,
                }
            }
            // the original's order: friction, gravity, then the pending force, then the move
            p.vel.x = slow(p.vel.x, p.friction.x);
            p.vel.y = slow(p.vel.y, p.friction.y);
            p.vel.y = p.vel.y + fx(charfmt::Fx(d.gravity.0 * g));
            p.vel = p.vel + p.force;
            p.force = FVec::ZERO;
            p.pos = p.pos + p.vel;
            if p.pos.y > Fixed::ZERO {
                p.pos.y = Fixed::ZERO; // the floor stops it
            }
            let limit = if d.lifetime > 0 { d.lifetime } else { MAX_PROJ_AGE };
            if p.age >= limit {
                p.dead = true;
            }
        }
        // objects spawned meanwhile were pushed onto the (taken) list
        projs.append(&mut self.projs);
        self.projs = projs;
        self.tick_objects();
        self.reap();
    }

    /// Removes finished projectiles; one with an `on_disable` leaves its child behind.
    pub(crate) fn reap(&mut self) {
        let mut children = Vec::new();
        for p in self.projs.iter().filter(|p| p.dead && !p.scripted) {
            let ch = &self.chars[p.owner];
            let Some(child) = ch.file.character.projectiles[p.idx].on_disable.as_deref() else { continue };
            let spawn = charfmt::Spawn { at: 0, projectile: child.to_string(), offset: (0, 0) };
            if let Some(mut c) = Proj::spawn(ch, p.owner, p.from_move, &spawn, p.pos, self.f[p.owner].facing) {
                c.uid = self.next_uid;
                self.next_uid += 1;
                children.push(c);
            }
        }
        self.projs.retain(|p| !p.dead);
        self.projs.extend(children);
    }

    /// The robot's missile: six ticks of a gentle push forward while it falls, then it flies,
    /// pushing forward a little harder every tick and upward while it is not rising fast.
    fn step_missile(&self, p: &mut Proj, g: f64) {
        let grav = fx(charfmt::Fx(0.2 * g));
        let max_speed = Fixed::from_int(40);
        p.ptick += 1;
        p.age += 1;
        let integrate = |p: &mut Proj| {
            p.vel.y = p.vel.y + grav;
            p.vel = p.vel + p.force;
            p.force = FVec::ZERO;
            p.vel.x = p.vel.x.clamp(-max_speed, max_speed);
            p.pos = p.pos + p.vel;
        };
        if p.phase == 0 {
            if p.ptick == 0 {
                p.force = p.force + FVec::new(Fixed::from_int(p.facing) * fx(charfmt::Fx(0.25)), Fixed::ZERO);
            }
            if p.ptick < 6 {
                integrate(p);
                return;
            }
            // the first state is over: its last tick's physics still run, then flight begins
            integrate(p);
            p.phase = 1;
            p.ptick = 0;
        }
        integrate(p);
        // flight: the next tick's push
        let forward = fx(charfmt::Fx(0.01)) + fx(charfmt::Fx(0.02)) * Fixed::from_int(p.ptick as i64);
        let up = if p.vel.y > -fx(charfmt::Fx(0.8)) { -fx(charfmt::Fx(0.3)) } else { Fixed::ZERO };
        p.force = p.force + FVec::new(Fixed::from_int(p.facing) * forward, up);
    }

    /// The wizard's spark bomb: floats up, arms after 16 ticks, then creeps toward the opponent
    /// at 2.5 a tick (aiming a little above their center) until it is within 24 and explodes.
    fn step_spark_bomb(&self, p: &mut Proj, target: FVec) {
        p.ptick += 1;
        p.age += 1;
        if p.phase == 2 {
            // the explosion just sits there; its hitbox is live on its first tick
            if p.ptick == 16 {
                p.dead = true;
            }
            return;
        }
        if p.phase == 0 && p.ptick == 0 {
            p.force = p.force + FVec::new(Fixed::ZERO, -fx(charfmt::Fx(0.56)));
        }
        // the state's own physics: pending force into speed, then speed into position
        p.vel = p.vel + p.force;
        p.force = FVec::ZERO;
        p.pos = p.pos + p.vel;
        if p.phase == 0 && p.ptick > 15 {
            p.phase = 1;
            p.ptick = -1;
            return;
        }
        let whole = FVec::new(Fixed::from_int(p.pos.x.to_f32().floor() as i64), Fixed::from_int(p.pos.y.to_f32().floor() as i64));
        let local = target - p.pos;
        // exploding switches state at once: the explosion's first tick is this one
        if p.phase == 1 && local.len() < fx(charfmt::Fx(24.0)) || p.ptick >= 600 {
            p.phase = 2;
            p.ptick = 0;
            return;
        }
        let aim = FVec::new(local.x, local.y + Fixed::from_int(10));
        p.pos = p.pos + aim.normalized().scale(fx(charfmt::Fx(2.5)));
        p.vel = FVec::new(slow(p.vel.x, fx(charfmt::Fx(0.05))), slow(p.vel.y, fx(charfmt::Fx(0.05))));
        p.pos = p.pos + p.vel;
    }
}
