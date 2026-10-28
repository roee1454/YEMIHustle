//! Deterministic real-time fighting-game simulation.
//!
//! One call to [`World::step`] advances one 60 Hz tick. Gameplay math is fixed-point; nothing
//! here reads the clock, random numbers or float state, so the same inputs always produce the
//! same match.
//!
//! Tick order: read inputs, then for each fighter not frozen by hitlag `think` (choose an
//! action) and `advance` (run its state and physics); then push fighters apart, re-face,
//! run throws, resolve hits (detect for both first, then apply, so trades happen), check KO
//! and update the frame-advantage readout.

pub mod data;
pub mod cpu;
pub mod fixed;
pub mod layout;
mod objects;
mod projectiles;
pub use objects::{NodeView, SpriteLook};
mod script;
pub use projectiles::Proj;
pub use script::{Scripts, NATIVE_SCRIPTS};

use std::sync::Arc;

use charfmt::{
    Button, CancelWhen, Direction, Effects, Height, HitKind, Hitlag, Knockback, KnockbackFrom, Knockdown,
    Move, MotionKind, Reaction, ReactionKind, Rect, Scaling, Stance, TimelineEnd,
};
pub use data::{Character, ComboRules, ControlScheme, EngineConfig};
pub use fixed::{fx, FVec, Fixed};

/// One tick of controller state. `left`/`right` are screen directions.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub light: bool,
    pub heavy: bool,
    pub special: bool,
    pub grab: bool,
    pub dash: bool,
}

/// Move ids that name states the engine runs itself.
const ENGINE_STATES: [&str; 9] =
    ["idle", "wait", "fall", "landing", "jump", "double_jump", "super_jump", "dash_forward", "dash_backward"];

/// Frames after which a move only its script can end is stopped anyway.
const SCRIPTED_MOVE_LIMIT: u16 = 600;

/// A state name of the original as a move id (`NormalPunch` -> `normal_punch`, `Wait` -> `idle`).
pub(crate) fn state_id(name: &str) -> String {
    if name == "Wait" {
        return "idle".into();
    }
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
            out.push('_');
        }
        out.push(c);
        prev = Some(c);
    }
    out.to_lowercase().chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' { c } else { '_' }).collect()
}

/// Buttons in the order they are checked when several are pressed on the same tick.
const BUTTONS: [Button; 6] = [Button::Grab, Button::Special, Button::Heavy, Button::Light, Button::Dash, Button::Jump];

impl Input {
    fn button(&self, b: Button) -> bool {
        match b {
            Button::Light => self.light,
            Button::Heavy => self.heavy,
            Button::Special => self.special,
            Button::Grab => self.grab,
            Button::Dash => self.dash,
            Button::Jump => self.up,
            Button::Super => self.heavy && self.special,
            _ => false,
        }
    }

    /// (forward, vertical) relative to `facing`: forward is 1 / 0 / -1, vertical -1 is up.
    fn relative(&self, facing: i64) -> (i64, i64) {
        let h = self.right as i64 - self.left as i64;
        let v = self.down as i64 - self.up as i64;
        (h * facing, v)
    }
}

fn dir_matches(d: Direction, fwd: i64, vert: i64) -> bool {
    match d {
        Direction::Any => true,
        Direction::Neutral => fwd == 0 && vert == 0,
        Direction::Forward => fwd > 0 && vert == 0,
        Direction::Back => fwd < 0 && vert == 0,
        Direction::Up => vert < 0,
        Direction::Down => vert > 0,
    }
}

/// A move in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRun {
    pub idx: usize,
    /// 1-based frame being played (0 only before the first advance).
    pub frame: u16,
    pub hit: bool,
    pub blocked: bool,
    /// Hitboxes that already connected: (id, repeat cycle).
    pub connected: Vec<(String, u16)>,
}

/// What a fighter is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Idle,
    Walk { back: bool },
    Crouch,
    JumpSquat { frame: u16, dir: i64 },
    Air,
    Landing { frame: u16 },
    /// A dash; `air` for an air dash (level flight, then falling).
    Dash { frame: u16, dir: i64, air: bool },
    Block { stun: u16, low: bool },
    Hurt { stun: u16, launched: bool, kd: Knockdown, until_landing: bool, min_air: i16, frame: u16 },
    Knockdown { frame: u16 },
    Getup { frame: u16 },
    Held,
    Move(MoveRun),
    Ko,
}

impl Act {
    pub fn label(&self) -> &'static str {
        match self {
            Act::Idle => "idle",
            Act::Walk { back: false } => "walk",
            Act::Walk { back: true } => "walk back",
            Act::Crouch => "crouch",
            Act::JumpSquat { .. } => "jump squat",
            Act::Air => "air",
            Act::Landing { .. } => "landing",
            Act::Dash { .. } => "dash",
            Act::Block { .. } => "block",
            Act::Hurt { launched: true, .. } => "hurt (launched)",
            Act::Hurt { .. } => "hurt",
            Act::Knockdown { .. } => "knockdown",
            Act::Getup { .. } => "getup",
            Act::Held => "held",
            Act::Move(_) => "move",
            Act::Ko => "KO",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Fighter {
    pub pos: FVec,
    pub vel: FVec,
    force: FVec,
    /// 1 = facing right, -1 = facing left.
    pub facing: i64,
    pub grounded: bool,
    pub hp: i32,
    pub max_hp: i32,
    pub act: Act,
    pub hitlag: u16,
    pub invuln: u16,
    /// Hits taken in the current combo, its damage, and its proration.
    pub combo_hits: u16,
    pub combo_damage: i32,
    proration: i32,
    pub air_actions: u8,
    /// Air dashes used since leaving the ground.
    pub air_dashes: u8,
    pub mode: usize,
    pub input: Input,
    prev: Input,
    pub buffered: Option<(Button, u16)>,
    /// Holding the opponent in a throw.
    pub holding: bool,
    /// Set by scripts: hits deal damage without interrupting (all hits / projectiles only).
    pub hyper_armor: bool,
    pub proj_armor: bool,
    /// Set by scripts: nothing / no projectile / no throw connects.
    pub script_invuln: bool,
    pub proj_invuln: bool,
    pub throw_invuln: bool,
    /// A state a script asked to switch to, applied after the fighter's tick.
    pub queued: Option<String>,
    /// Pays for supers and Hustle Cancels (see [`ComboRules`]).
    pub meter: i32,
    /// The moves of the current chain, so none repeats.
    pub chain: Vec<usize>,
}

/// Contact found during hit detection, applied after both fighters were checked.
#[derive(Clone)]
struct Contact {
    attacker: usize,
    /// Set when a projectile (index into `World::projs`) made the contact.
    proj: Option<usize>,
    /// How long the projectile itself freezes after hitting.
    proj_hitlag: u16,
    hitbox: charfmt::HitBox,
    cycle: u16,
    center: FVec,
    /// Middle of the hitbox/hurtbox overlap, where hit effects appear.
    spark: FVec,
}

/// Something that happened during a step, for sounds and effects. Cleared at each step.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `dir` is the knockback direction (unit length, already facing the right way).
    Hit { attacker: usize, victim: usize, mv: usize, hitbox: String, pos: FVec, dir: FVec, damage: i32 },
    Block { attacker: usize, victim: usize, mv: usize, hitbox: String, pos: FVec },
    Grab { attacker: usize, victim: usize, mv: usize, hitbox: String },
    ThrowTech,
    Ko { victim: usize },
    /// A fighter spent meter to cancel its move into a dash.
    HustleCancel { who: usize, pos: FVec },
    /// A script played an effect scene (`spawn_particle_effect`) at `pos`, pointing `dir`.
    Effect { scene: String, pos: FVec, dir: (f32, f32) },
    /// A script played a sound (`play_sound`): its file and volume in dB.
    Sound { path: String, volume: f32 },
    /// An attack or projectile of `attacker` struck a scripted object (a summon, a domain);
    /// `mv` is the fighter's move when it was a melee hit.
    ObjectHit { attacker: usize, mv: Option<usize>, hitbox: String, pos: FVec, dir: (f32, f32), damage: i32 },
}

/// Everything a landed hit needs, from a hitbox or a throw release.
struct Strike<'a> {
    damage: i32,
    damage_in_combo: Option<i32>,
    min_damage: Option<i32>,
    hitstun: u16,
    hitstun_in_combo: Option<u16>,
    hitlag: Hitlag,
    knockback: &'a Knockback,
    pushback: Fixed,
    scaling: Scaling,
    effects: Effects,
    reaction: Reaction,
    center: FVec,
    /// For the hit event: the move and hitbox that landed, and where.
    mv: usize,
    hitbox: String,
    spark: FVec,
    from_proj: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Advantage {
    pub on_block: bool,
    pub attacker: usize,
    /// Positive: the attacker can act this many frames before the victim.
    pub frames: i64,
}

struct AdvPending {
    attacker: usize,
    on_block: bool,
    free_at: [Option<u64>; 2],
}

/// Per-match settings chosen before a fight (the character select's settings panel).
/// Percentages are of the normal value, so the defaults change nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub damage_pct: i32,
    pub hitstun_pct: i32,
    pub hitstop_pct: i32,
    pub gravity_pct: i32,
    pub health_pct: [i32; 2],
    pub one_hit_ko: bool,
    /// Air actions are never used up.
    pub infinite_resources: bool,
    /// The round ends after this many frames; the healthier fighter (by share of max) wins.
    pub round_frames: Option<u64>,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            damage_pct: 100,
            hitstun_pct: 100,
            hitstop_pct: 100,
            gravity_pct: 100,
            health_pct: [100, 100],
            one_hit_ko: false,
            infinite_resources: false,
            round_frames: None,
        }
    }
}

pub struct World {
    pub cfg: EngineConfig,
    pub rules: Rules,
    /// Set when the round timer ran out with both fighters equally healthy.
    pub draw: bool,
    pub chars: [Arc<Character>; 2],
    pub f: [Fighter; 2],
    pub frame: u64,
    pub projs: Vec<Proj>,
    pub winner: Option<usize>,
    pub log: Vec<String>,
    pub events: Vec<Event>,
    pub last_advantage: Option<Advantage>,
    adv: Option<AdvPending>,
    /// The original's scripts, for characters that carry them (see [`World::attach_scripts`]).
    pub scripts: Option<Box<Scripts>>,
    /// The next projectile / object id.
    pub(crate) next_uid: u32,
    /// Scripts' `randi_range` (seeded, so a match replays the same).
    pub(crate) rng: u64,
}

impl World {
    pub fn new(cfg: EngineConfig, chars: [Arc<Character>; 2]) -> Self {
        Self::with_rules(cfg, chars, Rules::default())
    }

    pub fn with_rules(cfg: EngineConfig, chars: [Arc<Character>; 2], rules: Rules) -> Self {
        let mk = |side: i64, ch: &Character| {
            let hp = (ch.file.character.body.max_health * rules.health_pct[((side + 1) / 2) as usize] / 100).max(1);
            Fighter {
                pos: FVec::new(Fixed::from_int(side * cfg.start_distance as i64), Fixed::ZERO),
                vel: FVec::ZERO,
                force: FVec::ZERO,
                facing: -side,
                grounded: true,
                hp,
                max_hp: hp,
                act: Act::Idle,
                hitlag: 0,
                invuln: 0,
                combo_hits: 0,
                combo_damage: 0,
                proration: 0,
                air_actions: ch.file.character.body.air_actions,
                air_dashes: 0,
                mode: 0,
                input: Input::default(),
                prev: Input::default(),
                buffered: None,
                holding: false,
                hyper_armor: false,
                proj_armor: false,
                script_invuln: false,
                proj_invuln: false,
                throw_invuln: false,
                queued: None,
                meter: 0,
                chain: Vec::new(),
            }
        };
        let f = [mk(-1, &chars[0]), mk(1, &chars[1])];
        World { cfg, rules, draw: false, chars, f, frame: 0, projs: Vec::new(), winner: None, log: Vec::new(), events: Vec::new(), last_advantage: None, adv: None, scripts: None, next_uid: 1, rng: 0x2545_F491_4F6C_DD1D }
    }

    fn note(&mut self, msg: String) {
        self.log.push(format!("{:>5}  {msg}", self.frame));
        if self.log.len() > 6 {
            self.log.remove(0);
        }
    }

    // -----------------------------------------------------------------------------------------
    // The tick
    // -----------------------------------------------------------------------------------------

    pub fn step(&mut self, inputs: [Input; 2]) {
        self.frame += 1;
        self.events.clear();
        self.start_setup();
        self.clear_sprite_picks();
        for i in 0..2 {
            self.read_input(i, inputs[i]);
        }
        // objects tick before the fighters, so one spawned this tick first moves on the next
        self.step_projectiles();
        for i in 0..2 {
            // the fighter's own `tick()` override (resources, time stops...), hitlag or not,
            // as the original's BaseObj.tick runs it
            self.root_hook(i, "tick", vec![]);
            if self.f[i].hitlag > 0 {
                self.f[i].hitlag -= 1;
                continue;
            }
            self.think(i);
            self.advance(i);
            self.apply_queued(i);
            if self.f[i].grounded {
                self.f[i].air_dashes = 0;
            }
            if self.f[i].invuln > 0 {
                self.f[i].invuln -= 1;
            }
        }
        self.collide();
        self.update_facing();
        self.update_throws();
        // the intro's "Start" states: nothing hits, nobody is K.O. and the clock waits
        if !self.in_intro() {
            self.resolve_hits();
            self.check_ko();
            self.check_time();
            self.track_advantage();
        }
        for i in 0..2 {
            self.script_sync(i);
        }
        self.tick_nodes();
        if self.rules.infinite_resources {
            for i in 0..2 {
                self.f[i].air_actions = self.chars[i].file.character.body.air_actions;
                self.f[i].meter = self.cfg.combo.meter_max;
            }
        }
    }

    fn read_input(&mut self, i: usize, now: Input) {
        let buf = self.cfg.input_buffer;
        let f = &mut self.f[i];
        // a press made while frozen in hitstop waits for the fighter (chains feel the same
        // however early in the freeze the next button is pressed)
        if let Some((_, age)) = f.buffered.as_mut().filter(|_| f.hitlag == 0) {
            *age += 1;
            if *age > buf {
                f.buffered = None;
            }
        }
        for b in BUTTONS.iter().rev() {
            if now.button(*b) && !f.prev.button(*b) {
                f.buffered = Some((*b, 0));
            }
        }
        // Heavy and Special together are Super
        if now.button(Button::Super) && !f.prev.button(Button::Super) {
            f.buffered = Some((Button::Super, 0));
        }
        f.prev = now;
        f.input = now;
    }

    // -----------------------------------------------------------------------------------------
    // Choosing actions
    // -----------------------------------------------------------------------------------------

    fn think(&mut self, i: usize) {
        let ch = self.chars[i].clone();
        // A move that has played its last frame ends here, and the fighter may act this tick.
        if let Act::Move(run) = &self.f[i].act {
            let mv = ch.mv(run.idx);
            let tl = &mv.timeline;
            let (fixed_done, script_done) = match self.script_length(i, run.idx) {
                // the script's own length (it may change it), or only the script ends the move
                Some(len) => (len.is_some_and(|l| run.frame >= l), run.frame >= SCRIPTED_MOVE_LIMIT),
                None => (
                    tl.end == TimelineEnd::Fixed && run.frame >= tl.total(),
                    // Script-ended moves without a script: stop them after their listed frames.
                    tl.end == TimelineEnd::Script && run.frame >= tl.total().max(60),
                ),
            };
            if fixed_done || script_done {
                let next = mv.finish_into.clone();
                self.end_move(i, &next);
            }
        }
        match self.f[i].act.clone() {
            Act::Move(run) => self.try_cancel(i, &run),
            Act::Idle | Act::Walk { .. } | Act::Crouch => self.think_ground(i),
            Act::Air => self.think_air(i),
            Act::JumpSquat { frame, .. } if frame < self.cfg.combo.up_attack_frames => {
                // an attack right as the jump starts is the Up attack on the ground
                if let Some((b, _)) = self.f[i].buffered {
                    if !matches!(b, Button::Jump | Button::Dash) {
                        if let Some(idx) = self.pick_move(i, b, None) {
                            self.start_move(i, idx);
                        }
                    }
                }
            }
            Act::Dash { frame, .. } if frame >= self.cfg.dash_cancel_from => {
                if let Some((b, _)) = self.f[i].buffered {
                    if b != Button::Jump && b != Button::Dash {
                        if let Some(idx) = self.pick_move(i, b, None) {
                            self.start_move(i, idx);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn think_ground(&mut self, i: usize) {
        if let Some((b, _)) = self.f[i].buffered {
            match b {
                Button::Jump => {
                    let h = self.f[i].input.right as i64 - self.f[i].input.left as i64;
                    self.f[i].buffered = None;
                    self.f[i].act = Act::JumpSquat { frame: 0, dir: h };
                    return;
                }
                Button::Dash => {
                    let (fwd, _) = self.f[i].input.relative(self.f[i].facing);
                    self.f[i].buffered = None;
                    self.f[i].act = Act::Dash { frame: 0, dir: if fwd < 0 { -1 } else { 1 }, air: false };
                    return;
                }
                _ => {
                    if let Some(idx) = self.pick_move(i, b, None) {
                        self.start_move(i, idx);
                        return;
                    }
                }
            }
        }
        let (fwd, vert) = self.f[i].input.relative(self.f[i].facing);
        self.f[i].act = if vert > 0 {
            Act::Crouch
        } else if fwd != 0 {
            Act::Walk { back: fwd < 0 }
        } else {
            Act::Idle
        };
    }

    fn think_air(&mut self, i: usize) {
        let Some((b, _)) = self.f[i].buffered else { return };
        if b == Button::Jump {
            if self.f[i].air_actions > 0 {
                let h = self.f[i].input.right as i64 - self.f[i].input.left as i64;
                let f = &mut self.f[i];
                f.air_actions -= 1;
                f.buffered = None;
                f.vel = FVec::new(Fixed::from_int(h) * fx(self.cfg.jump_forward_speed), -fx(self.cfg.double_jump_speed));
            }
            return;
        }
        if b == Button::Dash {
            // an air dash, `air_dashes` times before landing
            if self.f[i].air_dashes < self.cfg.air_dashes {
                let (fwd, _) = self.f[i].input.relative(self.f[i].facing);
                let f = &mut self.f[i];
                f.air_dashes += 1;
                f.buffered = None;
                f.act = Act::Dash { frame: 0, dir: if fwd < 0 { -1 } else { 1 }, air: true };
            }
            return;
        }
        if let Some(idx) = self.pick_move(i, b, None) {
            self.start_move(i, idx);
        }
    }

    fn try_cancel(&mut self, i: usize, run: &MoveRun) {
        let Some((b, _)) = self.f[i].buffered else { return };
        let ch = self.chars[i].clone();
        let mv = ch.mv(run.idx);
        for c in &mv.cancels {
            if run.frame < c.window.from || run.frame > c.window.to {
                continue;
            }
            let ok = match c.when {
                CancelWhen::Always => true,
                CancelWhen::OnHit => run.hit,
                CancelWhen::OnBlock => run.blocked,
                CancelWhen::OnHitOrBlock => run.hit || run.blocked,
                CancelWhen::OnWhiff => !run.hit && !run.blocked,
            };
            if !ok {
                continue;
            }
            if b == Button::Jump && c.into.iter().any(|t| t == "jump" || t == "aerial") {
                let h = self.f[i].input.right as i64 - self.f[i].input.left as i64;
                self.f[i].buffered = None;
                if self.f[i].grounded {
                    self.f[i].act = Act::JumpSquat { frame: 0, dir: h };
                } else if self.f[i].air_actions > 0 {
                    self.f[i].air_actions -= 1;
                    self.f[i].vel = FVec::new(Fixed::from_int(h) * fx(self.cfg.jump_forward_speed), -fx(self.cfg.double_jump_speed));
                    self.f[i].act = Act::Air;
                }
                return;
            }
            if let Some(idx) = self.pick_move(i, b, Some(&c.into)) {
                self.note(format!("P{} cancel {} -> {}", i + 1, mv.id, ch.mv(idx).id));
                self.start_move(i, idx);
                if run.hit {
                    self.stretch_stun(i, idx);
                }
                return;
            }
        }
        self.engine_cancel(i, run, b);
    }

    /// The engine's own combo rules (see [`ComboRules`]), after the move's listed cancels.
    fn engine_cancel(&mut self, i: usize, run: &MoveRun, b: Button) {
        let rules = self.cfg.combo.clone();
        let ch = self.chars[i].clone();
        let connected = run.hit || run.blocked;
        let opp_launched = matches!(self.f[1 - i].act, Act::Hurt { launched: true, .. });
        if b == Button::Jump {
            if rules.jump_cancel && run.hit && opp_launched {
                let h = self.f[i].input.right as i64 - self.f[i].input.left as i64;
                self.f[i].buffered = None;
                if self.f[i].grounded {
                    self.f[i].act = Act::JumpSquat { frame: 0, dir: h };
                } else if self.f[i].air_actions > 0 {
                    self.f[i].air_actions -= 1;
                    self.f[i].vel = FVec::new(Fixed::from_int(h) * fx(self.cfg.jump_forward_speed), -fx(self.cfg.double_jump_speed));
                    self.f[i].act = Act::Air;
                }
                self.note(format!("P{} jump cancel", i + 1));
            }
            return;
        }
        if b == Button::Dash {
            if rules.hustle_cancel > 0 && self.f[i].meter >= rules.hustle_cancel {
                let (fwd, _) = self.f[i].input.relative(self.f[i].facing);
                let dir = if fwd < 0 { -1 } else { 1 };
                // free at once, carried by a dash's speed; a reeling opponent freezes for a beat
                let speed = if dir < 0 { -fx(self.cfg.back_dash_speed) } else { fx(self.cfg.dash_speed) };
                let f = &mut self.f[i];
                f.meter -= rules.hustle_cancel;
                f.buffered = None;
                f.vel = FVec::new(Fixed::from_int(f.facing) * speed, if f.grounded { Fixed::ZERO } else { f.vel.y.min(Fixed::ZERO) });
                f.act = if f.grounded { Act::Idle } else { Act::Air };
                if matches!(self.f[1 - i].act, Act::Hurt { .. }) {
                    let o = &mut self.f[1 - i];
                    o.hitlag = o.hitlag.max(rules.hustle_freeze);
                }
                self.release_hold(i);
                let pos = self.f[i].pos;
                self.events.push(Event::HustleCancel { who: i, pos });
                self.note(format!("P{} HUSTLE CANCEL", i + 1));
            }
            return;
        }
        if !rules.chains || !connected {
            return;
        }
        let Some(idx) = self.pick_move(i, b, None) else { return };
        let (from, to) = (ch.rank[run.idx], ch.rank[idx]);
        let stronger = to > from || (to == from && from == 1);
        if idx != run.idx && to > 0 && stronger && !self.f[i].chain.contains(&idx) {
            self.note(format!("P{} chain {} -> {}", i + 1, ch.mv(run.idx).id, ch.mv(idx).id));
            self.start_move(i, idx);
            self.stretch_stun(i, idx);
        }
    }

    /// The move a button starts right now, if any. `into` restricts it to cancel targets.
    fn pick_move(&mut self, i: usize, b: Button, into: Option<&Vec<String>>) -> Option<usize> {
        let ch = self.chars[i].clone();
        let f = self.f[i].clone();
        let (fwd, vert) = f.input.relative(f.facing);
        for bind in &ch.controls.binds {
            if bind.button != b || !dir_matches(bind.dir, fwd, vert) || (bind.cancel_only && into.is_none()) {
                continue;
            }
            let stance_ok = match bind.stance {
                Stance::Ground => f.grounded,
                Stance::Air => !f.grounded,
                Stance::Any => true,
            };
            if !stance_ok {
                continue;
            }
            for id in &bind.moves {
                let Some(idx) = ch.find(id) else { continue };
                let mv = ch.mv(idx);
                if let Some(into) = into {
                    if !mv.tags.iter().any(|t| into.contains(t)) {
                        continue;
                    }
                }
                if self.usable(i, mv) && self.script_usable(i, idx) {
                    return Some(idx);
                }
            }
        }
        None
    }

    /// Whether fighter `i` could start move `idx` now (the engine's checks and the move
    /// script's `is_usable`), for tools and tests.
    pub fn usable_now(&mut self, i: usize, idx: usize) -> bool {
        let mv = self.chars[i].mv(idx).clone();
        self.usable(i, &mv) && self.script_usable(i, idx)
    }

    fn usable(&self, i: usize, mv: &Move) -> bool {
        let f = &self.f[i];
        let opp = &self.f[1 - i];
        if self.cfg.combo.chains && matches!(opp.act, Act::Hurt { .. }) && self.chars[i].find(&mv.id).is_some_and(|k| f.chain.contains(&k)) {
            // already used in this combo
            return false;
        }
        let stance_ok = match mv.stance {
            Stance::Ground => f.grounded,
            Stance::Air => !f.grounded,
            Stance::Any => true,
        };
        let modes = self.chars[i].modes();
        let mode_ok = mv.modes.is_empty() || modes.is_empty() || mv.modes.iter().any(|m| *m == modes[f.mode]);
        let comboing = matches!(opp.act, Act::Hurt { .. }) && opp.combo_hits > 0;
        let avail_ok = !(mv.availability.combo_only && !comboing) && !(mv.availability.neutral_only && comboing);
        let cost_ok = (!mv.cost.air_action || f.air_actions > 0) && f.meter >= mv.cost.super_bars as i32 * self.cfg.combo.super_bar;
        stance_ok && mode_ok && avail_ok && cost_ok
    }

    fn start_move(&mut self, i: usize, idx: usize) {
        let ch = self.chars[i].clone();
        let mv = ch.mv(idx);
        let bar = self.cfg.combo.super_bar;
        let f = &mut self.f[i];
        f.buffered = None;
        // no move twice in one combo: the list only starts over once the opponent recovered
        let comboing = matches!(self.f[1 - i].act, Act::Hurt { .. } | Act::Knockdown { .. });
        let f = &mut self.f[i];
        if !matches!(f.act, Act::Move(_)) && !comboing {
            f.chain.clear();
        }
        f.chain.push(idx);
        f.meter = (f.meter - mv.cost.super_bars as i32 * bar).max(0);
        if let Some(m) = &mv.set_mode {
            if let Some(p) = ch.modes().iter().position(|x| x == m) {
                f.mode = p;
            }
        }
        if mv.cost.air_action && f.air_actions > 0 {
            f.air_actions -= 1;
        }
        if mv.throw.as_ref().is_some_and(|t| t.reverse) {
            f.facing = -f.facing;
        }
        f.act = Act::Move(MoveRun { idx, frame: 0, hit: false, blocked: false, connected: Vec::new() });
        self.script_enter(i, idx);
    }

    /// Starts move `id` for fighter `i` right now, whatever it is doing (demos and tools). An air
    /// move starts in the air. Returns false if the character has no such move.
    pub fn force_move(&mut self, i: usize, id: &str) -> bool {
        let Some(idx) = self.chars[i].find(id) else { return false };
        if self.chars[i].mv(idx).stance == Stance::Air && self.f[i].grounded {
            let f = &mut self.f[i];
            f.pos.y = Fixed::from_int(-60);
            f.vel = FVec::ZERO;
            f.grounded = false;
            f.act = Act::Air;
        }
        self.start_move(i, idx);
        true
    }

    /// Switches to the state a script asked for.
    fn apply_queued(&mut self, i: usize) {
        for _ in 0..4 {
            let Some(name) = self.f[i].queued.take() else { return };
            match self.state_by_name(i, &name).filter(|_| !ENGINE_STATES.contains(&state_id(&name).as_str())) {
                Some(idx) => self.start_move(i, idx),
                None => {
                    self.script_leave(i);
                    self.end_move(i, &state_id(&name));
                }
            }
        }
    }

    /// Leave the current move, entering `next` if it is a move of this character.
    /// Neutral states (idle, fall, landing...) are always the engine's own, even when the
    /// character file has converted versions of them.
    fn end_move(&mut self, i: usize, next: &str) {
        let ch = self.chars[i].clone();
        let engine_state = ENGINE_STATES.contains(&next);
        if let Some(idx) = ch.find(next).filter(|_| !engine_state) {
            if self.usable(i, ch.mv(idx)) {
                self.start_move(i, idx);
                return;
            }
        }
        self.release_hold(i);
        let f = &mut self.f[i];
        f.act = if f.grounded { Act::Idle } else { Act::Air };
        if f.grounded {
            f.mode = 0;
        }
    }

    fn release_hold(&mut self, i: usize) {
        if self.f[i].holding {
            self.f[i].holding = false;
            let v = 1 - i;
            if self.f[v].act == Act::Held {
                self.f[v].act = if self.f[v].grounded { Act::Idle } else { Act::Air };
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Running states and physics
    // -----------------------------------------------------------------------------------------

    fn advance(&mut self, i: usize) {
        let ch = self.chars[i].clone();
        let mut body = ch.file.character.body.clone();
        let mut cfg = self.cfg.clone();
        let g = self.rules.gravity_pct as f64 / 100.0;
        body.gravity = charfmt::Fx(body.gravity.0 * g);
        cfg.hurt_gravity = charfmt::Fx(cfg.hurt_gravity.0 * g);
        let act = self.f[i].act.clone();
        let f = &mut self.f[i];
        match act {
            Act::Idle | Act::Crouch => {
                apply_fric(f, &body);
                integrate(f, Some(&body));
            }
            Act::Walk { back } => {
                let speed = if back { -fx(cfg.back_walk_speed) } else { fx(cfg.walk_speed) };
                f.vel.x = Fixed::from_int(f.facing) * speed;
                integrate(f, Some(&body));
            }
            Act::JumpSquat { frame, dir } => {
                apply_fric(f, &body);
                integrate(f, Some(&body));
                let frame = frame + 1;
                if frame >= cfg.jump_squat {
                    f.vel = FVec::new(Fixed::from_int(dir) * fx(cfg.jump_forward_speed), -fx(cfg.jump_speed));
                    f.pos.y = f.pos.y - Fixed::ONE;
                    f.grounded = false;
                    f.act = Act::Air;
                } else {
                    f.act = Act::JumpSquat { frame, dir };
                }
            }
            Act::Air => {
                apply_grav(f, &body);
                integrate(f, Some(&body));
                if f.grounded {
                    f.act = Act::Landing { frame: 0 };
                }
            }
            Act::Landing { frame } => {
                apply_fric(f, &body);
                integrate(f, Some(&body));
                f.air_actions = body.air_actions;
                f.air_dashes = 0;
                f.mode = 0;
                f.act = if frame + 1 >= cfg.landing_lag { Act::Idle } else { Act::Landing { frame: frame + 1 } };
            }
            Act::Dash { frame, dir, air: false } => {
                let frame = frame + 1;
                if frame == 1 {
                    let speed = if dir < 0 { -fx(cfg.back_dash_speed) } else { fx(cfg.dash_speed) };
                    f.vel.x = Fixed::from_int(f.facing) * speed;
                }
                apply_fric(f, &body);
                integrate(f, None);
                f.act = if frame >= cfg.dash_frames { Act::Idle } else { Act::Dash { frame, dir, air: false } };
            }
            Act::Dash { frame, dir, air: true } => {
                // straight across at the dash's height, then the fall takes over
                let frame = frame + 1;
                if frame == 1 {
                    f.vel.x = Fixed::from_int(f.facing * dir) * fx(cfg.air_dash_speed);
                }
                f.vel.y = Fixed::ZERO;
                integrate(f, None);
                f.act = if frame >= cfg.air_dash_frames {
                    f.vel.x = f.vel.x / Fixed::from_int(2);
                    Act::Air
                } else {
                    Act::Dash { frame, dir, air: true }
                };
            }
            Act::Block { stun, low } => {
                if f.grounded {
                    apply_fric(f, &body);
                } else {
                    apply_grav(f, &body);
                }
                integrate(f, Some(&body));
                let stun = stun.saturating_sub(1);
                f.act = if stun == 0 {
                    if f.grounded { Act::Idle } else { Act::Air }
                } else {
                    Act::Block { stun, low }
                };
            }
            Act::Hurt { stun, launched, kd, until_landing, min_air, frame } => {
                let frame = frame + 1;
                if launched {
                    if f.vel.y < fx(cfg.hurt_fall_speed) {
                        f.vel.y = (f.vel.y + fx(cfg.hurt_gravity)).min(fx(cfg.hurt_fall_speed));
                    }
                    f.vel.x = f.vel.x * (Fixed::ONE - fx(cfg.hurt_air_friction));
                } else {
                    f.vel.y = Fixed::ZERO;
                    f.vel.x = f.vel.x * (Fixed::ONE - fx(cfg.hurt_ground_friction));
                }
                integrate(f, None);
                let stun = stun.saturating_sub(1);
                let landed = launched && f.grounded && frame as i64 > (min_air as i64).max(1);
                f.act = if landed && (kd != Knockdown::None || f.hp <= 0) {
                    f.vel = FVec::ZERO;
                    Act::Knockdown { frame: 0 }
                } else if landed {
                    if stun == 0 { Act::Landing { frame: 0 } } else { Act::Hurt { stun, launched: false, kd, until_landing: false, min_air, frame } }
                } else if stun == 0 && !(until_landing && !f.grounded) {
                    if f.grounded { Act::Idle } else { Act::Air }
                } else {
                    Act::Hurt { stun, launched: launched && !f.grounded, kd, until_landing, min_air, frame }
                };
                if matches!(f.act, Act::Idle | Act::Air | Act::Landing { .. } | Act::Knockdown { .. }) && f.hp > 0 {
                    // the combo is over once the victim is out of hitstun
                    f.combo_hits = 0;
                }
            }
            Act::Knockdown { frame } => {
                apply_fric(f, &body);
                integrate(f, Some(&body));
                let frame = frame + 1;
                f.combo_hits = 0;
                if f.hp <= 0 {
                    f.act = Act::Ko;
                } else if frame >= cfg.knockdown_frames {
                    f.invuln = cfg.getup_frames;
                    f.act = Act::Getup { frame: 0 };
                } else {
                    f.act = Act::Knockdown { frame };
                }
            }
            Act::Getup { frame } => {
                let frame = frame + 1;
                f.act = if frame >= cfg.getup_frames { Act::Idle } else { Act::Getup { frame } };
            }
            Act::Held => {}
            Act::Ko => {
                if f.grounded {
                    apply_fric(f, &body);
                } else {
                    apply_grav(f, &body);
                }
                integrate(f, Some(&body));
            }
            Act::Move(run) => self.advance_move(i, run, &body),
        }
    }

    fn advance_move(&mut self, i: usize, mut run: MoveRun, body: &charfmt::Body) {
        let ch = self.chars[i].clone();
        run.frame += 1;
        let mv = ch.mv(run.idx);
        {
            let f = &mut self.f[i];
            for ev in mv.motion.iter().filter(|e| e.at == run.frame) {
                let fac = Fixed::from_int(f.facing);
                match ev.kind {
                    MotionKind::Impulse { forward, up } => {
                        f.force = f.force + FVec::new(fac * fx(forward), -fx(up));
                    }
                    MotionKind::SetVelocity { forward, up } => {
                        f.vel = FVec::new(fac * fx(forward), -fx(up));
                    }
                    MotionKind::StopMomentum => f.vel = FVec::ZERO,
                    MotionKind::StopHorizontal => f.vel.x = Fixed::ZERO,
                    MotionKind::StopVertical => f.vel.y = Fixed::ZERO,
                }
            }
        }
        let (pos, facing) = (self.f[i].pos, self.f[i].facing);
        for s in mv.spawns.iter().filter(|s| s.at == run.frame) {
            let Some(idx) = ch.file.character.projectiles.iter().position(|p| p.id == s.projectile) else { continue };
            if !ch.file.character.projectiles[idx].states.is_empty() && self.scripts.is_some() {
                // a scripted object (an imported mod's)
                let at = FVec::new(pos.x + Fixed::from_int(facing * s.offset.0 as i64), pos.y + Fixed::from_int(s.offset.1 as i64));
                self.spawn_object_def(i, idx, at, facing, run.idx);
            } else if let Some(mut p) = projectiles::Proj::spawn(&ch, i, run.idx, s, pos, facing) {
                p.uid = self.next_uid;
                self.next_uid += 1;
                self.projs.push(p);
            }
        }
        self.f[i].act = Act::Move(run.clone());
        // the original runs a state's frame hooks before its physics and `_tick` after
        let scripted = self.has_state_script(i, run.idx);
        if scripted {
            self.script_frame(i, run.idx, run.frame);
        }
        let f = &mut self.f[i];
        let ph = mv.physics;
        if ph.friction {
            apply_fric(f, body);
        }
        if ph.gravity && ph.gravity_from.map_or(true, |from| run.frame >= from) {
            apply_grav(f, body);
        }
        if let Some(fr) = ph.friction_x {
            f.vel.x = f.vel.x * (Fixed::ONE - fx(fr));
        }
        if let Some(fr) = ph.friction_y {
            f.vel.y = f.vel.y * (Fixed::ONE - fx(fr));
        }
        if let Some(g) = ph.gravity_override {
            if f.vel.y < fx(g.fall_speed) {
                f.vel.y = (f.vel.y + fx(g.gravity)).min(fx(g.fall_speed));
            }
        }
        // Without `forces` the original never moves the body (velocity is kept, not applied).
        if ph.forces {
            if ph.uncapped_forces {
                integrate(f, None);
            } else if let Some(c) = ph.speed_caps {
                let cap = if f.grounded { fx(c.ground) } else { fx(c.air) };
                f.vel.x = (f.vel.x + f.force.x).clamp(-cap, cap);
                f.force.x = Fixed::ZERO;
                integrate(f, None);
            } else {
                integrate(f, Some(body));
            }
        } else if !scripted {
            // (a script may still apply the forces it adds itself)
            f.force = FVec::ZERO;
        }
        if scripted {
            self.script_tick(i, run.idx);
        }
        let air_move = mv.stance == Stance::Air || mv.timeline.end == TimelineEnd::OnLanding;
        let landed = air_move && self.f[i].grounded && run.frame > 1;
        let still = matches!(&self.f[i].act, Act::Move(r) if r.idx == run.idx);
        if landed && still && self.f[i].queued.is_none() {
            self.release_hold(i);
            let f = &mut self.f[i];
            f.act = Act::Landing { frame: 0 };
            f.mode = 0;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Spacing
    // -----------------------------------------------------------------------------------------

    fn collide(&mut self) {
        let w = Fixed::from_int(self.cfg.stage_half_width as i64);
        let boxes: Vec<Rect> = (0..2).map(|i| self.chars[i].file.character.body.collision_box).collect();
        let solid = |a: &Act| !matches!(a, Act::Held);
        if solid(&self.f[0].act) && solid(&self.f[1].act) {
            let (a, b) = (&self.f[0], &self.f[1]);
            let (ra, rb) = (world_rect(a, &boxes[0]), world_rect(b, &boxes[1]));
            if overlap(&ra, &rb) {
                let need = ra.1.x + rb.1.x - (ra.0.x - rb.0.x).abs();
                let half = Fixed(need.0 / 2 + 1);
                let left = if a.pos.x < b.pos.x || (a.pos.x == b.pos.x && a.facing > 0) { 0 } else { 1 };
                self.f[left].pos.x -= half;
                self.f[1 - left].pos.x += half;
            }
        }
        for i in 0..2 {
            let hw = Fixed::from_int(boxes[i].half_w as i64);
            let f = &mut self.f[i];
            f.pos.x = f.pos.x.clamp(-w + hw, w - hw);
        }
    }

    fn update_facing(&mut self) {
        for i in 0..2 {
            let dx = self.f[1 - i].pos.x - self.f[i].pos.x;
            let f = &mut self.f[i];
            let neutral = matches!(f.act, Act::Idle | Act::Walk { .. } | Act::Crouch | Act::Landing { .. } | Act::JumpSquat { .. });
            if neutral && f.grounded && dx.signum() != 0 {
                f.facing = dx.signum();
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Throws
    // -----------------------------------------------------------------------------------------

    fn update_throws(&mut self) {
        for a in 0..2 {
            if !self.f[a].holding {
                continue;
            }
            let v = 1 - a;
            let Act::Move(run) = self.f[a].act.clone() else {
                self.release_hold(a);
                continue;
            };
            let ch = self.chars[a].clone();
            let mv = ch.mv(run.idx);
            let Some(throw) = &mv.throw else {
                self.release_hold(a);
                continue;
            };
            // Damage dealt to the held victim lands before any release on the same frame.
            for hb in mv.hitboxes.iter().filter(|h| h.kind == HitKind::ThrowDamage) {
                let key = (hb.id.clone(), 0);
                if hitbox_live(hb, run.frame).is_some() && !run.connected.contains(&key) {
                    if let Act::Move(r) = &mut self.f[a].act {
                        r.connected.push(key);
                        r.hit = true;
                    }
                    let dmg = self.scaled_damage(v, hb.damage, hb.damage_in_combo, hb.min_damage);
                    self.f[v].hp -= dmg;
                    self.f[v].combo_damage += dmg;
                    let (la, lv) = (self.stop(hb.hitlag.attacker), self.stop(hb.hitlag.victim));
                    self.f[a].hitlag = self.f[a].hitlag.max(la);
                    self.f[v].hitlag = lv;
                    self.note(format!("P{} throw damage {} ({dmg})", a + 1, mv.id));
                    let pos = self.f[v].pos;
                    let dir = FVec::new(Fixed::from_int(self.f[a].facing), Fixed::ZERO);
                    self.events.push(Event::Hit { attacker: a, victim: v, mv: run.idx, hitbox: hb.id.clone(), pos, dir, damage: dmg });
                }
            }
            let fac = Fixed::from_int(self.f[a].facing);
            let mut off = throw.hold;
            for k in &throw.keyframes {
                if k.at <= run.frame {
                    off = k.offset;
                }
            }
            if let Some(rel) = &throw.release {
                if run.frame >= rel.at {
                    off = rel.offset;
                }
            }
            let place = FVec::new(
                self.f[a].pos.x + fac * Fixed::from_int(off.0 as i64),
                (self.f[a].pos.y + Fixed::from_int(off.1 as i64)).min(Fixed::ZERO),
            );
            self.f[v].pos = place;
            self.f[v].vel = FVec::ZERO;
            self.f[v].grounded = place.y >= Fixed::ZERO;
            if let Some(rel) = &throw.release {
                if run.frame >= rel.at {
                    self.f[a].holding = false;
                    let h = &rel.hit;
                    let strike = Strike {
                        damage: h.damage,
                        damage_in_combo: h.damage_in_combo,
                        min_damage: None,
                        hitstun: h.hitstun,
                        hitstun_in_combo: None,
                        hitlag: Hitlag { attacker: 0, victim: 0 },
                        knockback: &h.knockback,
                        pushback: Fixed::ZERO,
                        scaling: h.scaling,
                        effects: h.effects,
                        reaction: h.reaction,
                        center: self.f[a].pos,
                        mv: run.idx,
                        hitbox: "release".into(),
                        spark: self.f[v].pos,
                        from_proj: false,
                    };
                    self.f[v].act = Act::Idle;
                    self.land_strike(a, v, &strike);
                    self.note(format!("P{} {} released", a + 1, mv.id));
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Hits
    // -----------------------------------------------------------------------------------------

    /// Hurtboxes of fighter `i` in world space (center, half size).
    pub fn hurtboxes(&self, i: usize) -> Vec<(FVec, FVec)> {
        let f = &self.f[i];
        let ch = &self.chars[i];
        let body = &ch.file.character.body;
        let mut rect = if matches!(f.act, Act::Crouch) || matches!(f.act, Act::Block { low: true, .. }) {
            self.cfg.crouch_hurtbox
        } else {
            body.hurtbox
        };
        if let Act::Move(run) = &f.act {
            for w in &ch.mv(run.idx).hurt {
                if w.window.from <= run.frame && run.frame <= w.window.to {
                    rect = w.rect;
                }
            }
        }
        vec![world_rect(f, &rect)]
    }

    /// Live hitboxes of fighter `i` in world space, with their data.
    pub fn hitboxes(&self, i: usize) -> Vec<(FVec, FVec, charfmt::HitBox, u16)> {
        let f = &self.f[i];
        let Act::Move(run) = &f.act else { return Vec::new() };
        let mv = self.chars[i].mv(run.idx);
        mv.hitboxes
            .iter()
            .filter_map(|hb| {
                let cycle = hitbox_live(hb, run.frame)?;
                let (c, h) = world_rect(f, &hb.rect);
                Some((c, h, hb.clone(), cycle))
            })
            .collect()
    }

    fn resolve_hits(&mut self) {
        let mut contacts: Vec<(usize, Contact)> = Vec::new();
        for a in 0..2 {
            let v = 1 - a;
            let Act::Move(run) = &self.f[a].act else { continue };
            if self.f[a].holding || self.f[a].hitlag > 0 {
                continue;
            }
            let victim = &self.f[v];
            if victim.invuln > 0 || victim.script_invuln || matches!(victim.act, Act::Ko | Act::Held) {
                continue;
            }
            let hurts = self.hurtboxes(v);
            for (c, h, hb, cycle) in self.hitboxes(a) {
                if hb.kind == HitKind::ThrowDamage || hb.kind == HitKind::Detect || (hb.effects.throw && victim.throw_invuln) {
                    continue;
                }
                if run.connected.contains(&hit_key(&hb, cycle)) {
                    continue;
                }
                let Some(hr) = hurts.iter().find(|hr| overlap(&(c, h), hr)) else { continue };
                let spark = overlap_center(&(c, h), hr);
                let downed = matches!(victim.act, Act::Knockdown { .. } | Act::Getup { .. });
                if downed && !hb.flags.hits_downed {
                    continue;
                }
                if (victim.grounded && !hb.flags.hits_grounded) || (!victim.grounded && !hb.flags.hits_air) {
                    continue;
                }
                if hb.effects.throw
                    && matches!(victim.act, Act::Hurt { .. } | Act::Block { .. } | Act::Knockdown { .. } | Act::Getup { .. })
                {
                    continue;
                }
                contacts.push((v, Contact { attacker: a, proj: None, proj_hitlag: 0, hitbox: hb, cycle, center: c, spark }));
                break;
            }
        }
        for (pi, p) in self.projs.iter().enumerate() {
            let v = 1 - p.owner;
            let victim = &self.f[v];
            // a dash slips through projectiles
            if p.dead || victim.invuln > 0 || victim.script_invuln || victim.proj_invuln || matches!(victim.act, Act::Ko | Act::Held | Act::Dash { .. }) || contacts.iter().any(|(cv, c)| *cv == v && c.proj.is_some()) {
                continue;
            }
            let hurts = self.hurtboxes(v);
            let d = &self.chars[p.owner].file.character.projectiles[p.idx];
            // a scripted object hits with its current state's boxes, timed from that state's start
            let boxes = if p.is_object() { d.states.get(p.state).map(|s| &s.hitboxes[..]).unwrap_or(&[]) } else { &d.hitboxes[..] };
            for hb in boxes {
                let frame = if p.is_object() { Some(p.sframe) } else { p.hit_frame(d.behavior.as_deref()) };
                let Some(frame) = frame else { break };
                let Some(cycle) = hitbox_live(hb, frame) else { continue };
                if hb.kind == HitKind::ThrowDamage || hb.kind == HitKind::Detect || hb.effects.throw || p.connected.contains(&hit_key(hb, cycle)) {
                    continue;
                }
                // the original tests overlaps at whole-pixel positions
                let whole = FVec::new(Fixed::from_int(p.pos.x.round()), Fixed::from_int(p.pos.y.round()));
                let (c, h) = rect_at(whole, p.facing, &hb.rect);
                let Some(hr) = hurts.iter().find(|hr| overlap(&(c, h), hr)) else { continue };
                let downed = matches!(victim.act, Act::Knockdown { .. } | Act::Getup { .. });
                if (downed && !hb.flags.hits_downed) || (victim.grounded && !hb.flags.hits_grounded) || (!victim.grounded && !hb.flags.hits_air) {
                    continue;
                }
                let mut hit = hb.clone();
                let proj_hitlag = hit.hitlag.attacker as u16;
                hit.hitlag.attacker = 0;
                hit.pushback = charfmt::Fx(0.0);
                hit.followup = None;
                contacts.push((v, Contact { attacker: p.owner, proj: Some(pi), proj_hitlag, hitbox: hit, cycle, center: c, spark: overlap_center(&(c, h), hr) }));
                break;
            }
        }
        // A strike beats a throw on the same tick; two throws cancel out.
        let throws = contacts.iter().filter(|(_, c)| c.hitbox.effects.throw).count();
        if throws == 2 {
            self.note("throw tech".into());
            self.events.push(Event::ThrowTech);
            for i in 0..2 {
                self.f[i].act = Act::Idle;
                self.f[i].vel.x = Fixed::from_int(-self.f[i].facing * 4);
            }
            return;
        }
        if contacts.len() == 2 && throws == 1 {
            let strike_beats_throw = contacts.iter().any(|(_, c)| !c.hitbox.effects.throw && c.hitbox.flags.beats_throw);
            if strike_beats_throw {
                contacts.retain(|(_, c)| !c.hitbox.effects.throw);
            }
        }
        for (v, c) in contacts {
            self.apply_contact(v, c);
        }
        self.hit_objects();
        // a projectile that hit is gone (leaving its child, if it has one)
        self.reap();
    }

    fn apply_contact(&mut self, v: usize, c: Contact) {
        let a = c.attacker;
        let hb = &c.hitbox;
        let ch = self.chars[a].clone();
        let (run_idx, run_frame) = if let Some(pi) = c.proj {
            let pierces = ch.file.character.projectiles[self.projs[pi].idx].pierces;
            let p = &mut self.projs[pi];
            p.connected.push(hit_key(hb, c.cycle));
            // a scripted object's own scripts decide when it goes
            p.dead = !pierces && !p.is_object();
            p.hitlag = p.hitlag.max(c.proj_hitlag);
            (p.from_move, 0)
        } else {
            match &mut self.f[a].act {
                Act::Move(run) => {
                    run.connected.push(hit_key(hb, c.cycle));
                    (run.idx, run.frame)
                }
                _ => return,
            }
        };
        let from_proj = c.proj.is_some();
        let mv = ch.mv(run_idx);
        if hb.effects.throw {
            self.release_hold(v);
            let f = &mut self.f[v];
            f.act = Act::Held;
            f.vel = FVec::ZERO;
            f.combo_hits = 0;
            f.combo_damage = 0;
            self.note(format!("P{} grabbed P{}", a + 1, v + 1));
            self.events.push(Event::Grab { attacker: a, victim: v, mv: run_idx, hitbox: hb.id.clone() });
            if let Some(next) = hb.throw_into.as_ref().and_then(|t| ch.find(t)) {
                self.start_move(a, next);
                self.f[a].holding = true;
            } else {
                self.f[v].act = Act::Idle;
            }
            return;
        }
        if self.can_block(v, hb.height) {
            let remaining = if !from_proj && mv.timeline.end == TimelineEnd::Fixed {
                mv.timeline.total() as i64 - run_frame as i64
            } else {
                10
            };
            let stun = (remaining + hb.block_advantage as i64).max(1) as u16;
            let low = self.f[v].input.down;
            let chip = hb.damage / self.cfg.chip_divisor.max(1);
            let fac = Fixed::from_int(self.f[a].facing);
            let (la, lv) = (self.stop(hb.hitlag.attacker), self.stop(hb.hitlag.victim));
            let f = &mut self.f[v];
            f.act = Act::Block { stun, low };
            f.hp -= chip;
            f.vel.x = fac * fx(self.cfg.block_pushback);
            f.hitlag = lv;
            self.f[a].hitlag = self.f[a].hitlag.max(la);
            if !from_proj {
                if let Act::Move(run) = &mut self.f[a].act {
                    run.blocked = true;
                }
                self.begin_advantage(a, true);
                self.state_hook(a, run_idx, "on_got_blocked", vec![]);
            }
            self.gain_meter(a, self.cfg.combo.meter_on_block);
            self.gain_meter(v, self.cfg.combo.meter_on_block);
            self.note(format!("P{} {} blocked (stun {stun}, chip {chip})", a + 1, mv.id));
            self.events.push(Event::Block { attacker: a, victim: v, mv: run_idx, hitbox: hb.id.clone(), pos: c.spark });
            return;
        }
        let strike = Strike {
            damage: hb.damage,
            damage_in_combo: hb.damage_in_combo,
            min_damage: hb.min_damage,
            hitstun: hb.hitstun,
            hitstun_in_combo: hb.hitstun_in_combo,
            hitlag: hb.hitlag,
            knockback: &hb.knockback,
            pushback: fx(hb.pushback),
            scaling: hb.scaling,
            effects: hb.effects,
            reaction: hb.reaction,
            center: c.center,
            mv: run_idx,
            hitbox: hb.id.clone(),
            spark: c.spark,
            from_proj,
        };
        let dmg = self.land_strike(a, v, &strike);
        let r = self.cfg.combo.clone();
        self.gain_meter(a, r.meter_on_hit + dmg * r.meter_per_damage / 100);
        self.gain_meter(v, r.meter_on_hurt + dmg * r.meter_per_damage_taken / 100);
        if !from_proj {
            if let Act::Move(run) = &mut self.f[a].act {
                run.hit = true;
            }
            self.begin_advantage(a, false);
            let hitbox = gdscript::Value::dict(vec![
                (gdscript::Value::str("name"), gdscript::Value::str(&hb.id)),
                (gdscript::Value::str("damage"), gdscript::Value::Int(hb.damage as i64)),
            ]);
            if let Some(gdscript::Value::Str(next)) = self.state_hook(a, run_idx, "_on_hit_something", vec![script::fighter_value(v), hitbox.clone()]) {
                self.f[a].queued = Some(next.to_string());
            }
            self.root_hook(a, "_on_hit_something", vec![script::fighter_value(v), hitbox]);
        }
        self.note(format!("P{} {} hit P{}: {dmg} dmg, combo {}", a + 1, mv.id, v + 1, self.f[v].combo_hits));
        if let Some(next) = hb.followup.as_ref().and_then(|t| ch.find(t)) {
            self.start_move(a, next);
        }
    }

    /// True chains: the opponent stays in hitstun long enough for move `idx` of fighter `i` to
    /// come out.
    fn stretch_stun(&mut self, i: usize, idx: usize) {
        if !self.cfg.combo.true_chains {
            return;
        }
        let need = self.chars[i].mv(idx).timeline.startup + 3;
        if let Act::Hurt { stun, .. } = &mut self.f[1 - i].act {
            *stun = (*stun).max(need);
        }
    }

    fn gain_meter(&mut self, i: usize, n: i32) {
        let max = self.cfg.combo.meter_max;
        self.f[i].meter = (self.f[i].meter + n).min(max);
    }

    fn can_block(&self, v: usize, height: Height) -> bool {
        let f = &self.f[v];
        let (fwd, vert) = f.input.relative(f.facing);
        if fwd >= 0 {
            return false;
        }
        let ready = matches!(f.act, Act::Idle | Act::Walk { .. } | Act::Crouch | Act::Block { .. })
            || (matches!(f.act, Act::Air) && self.cfg.air_block);
        if !ready {
            return false;
        }
        if !f.grounded {
            return true;
        }
        match height {
            Height::Low => vert > 0,
            Height::Mid | Height::High => vert <= 0,
        }
    }

    fn scaled_damage(&self, v: usize, base: i32, in_combo: Option<i32>, min: Option<i32>) -> i32 {
        let f = &self.f[v];
        let base = if f.combo_hits > 0 { in_combo.unwrap_or(base) } else { base };
        let steps = self.cfg.stale_steps.max(1) as i64;
        let n = (f.combo_hits as i64 + if f.combo_hits > 0 { f.proration as i64 } else { 0 }).clamp(0, steps);
        let ratio = Fixed::from_int(steps - n) / Fixed::from_int(steps);
        let min_stale = fx(self.cfg.min_stale);
        let stale = min_stale + (Fixed::ONE - min_stale) * ratio * ratio;
        let mut dmg = (Fixed::from_int(base as i64) * stale).round() as i32;
        if let Some(m) = min {
            dmg = dmg.max(m);
        }
        dmg = (dmg as i64 * self.rules.damage_pct as i64 / 100) as i32;
        if self.rules.one_hit_ko && base > 0 {
            dmg = f.hp.max(1);
        }
        if base > 0 {
            dmg = dmg.max(1);
        }
        dmg
    }

    /// Damage, stun and knockback of a landed hit. Returns the damage dealt.
    fn land_strike(&mut self, a: usize, v: usize, s: &Strike) -> i32 {
        // the victim's state hears of the hit first (it may drop its armor)
        if let Act::Move(run) = &self.f[v].act {
            let idx = run.idx;
            self.state_hook(v, idx, "on_got_hit", vec![]);
        }
        // and the fighter's own script (`hit_by(hitbox)`, `on_got_hit()` overrides)
        let hitbox = gdscript::Value::dict(vec![
            (gdscript::Value::str("name"), gdscript::Value::str(&s.hitbox)),
            (gdscript::Value::str("host"), gdscript::Value::str(&format!("P{}", a + 1))),
            (gdscript::Value::str("damage"), gdscript::Value::Int(s.damage as i64)),
            (gdscript::Value::str("damage_in_combo"), gdscript::Value::Int(s.damage_in_combo.unwrap_or(s.damage) as i64)),
            (gdscript::Value::str("hitstun_ticks"), gdscript::Value::Int(s.hitstun as i64)),
            (gdscript::Value::str("victim_hitlag"), gdscript::Value::Int(s.hitlag.victim as i64)),
            (gdscript::Value::str("hitlag_ticks"), gdscript::Value::Int(s.hitlag.attacker as i64)),
            (gdscript::Value::str("throw"), gdscript::Value::Bool(false)),
            (gdscript::Value::str("misc_data"), gdscript::Value::str("")),
        ]);
        self.root_hook(v, "hit_by", vec![hitbox]);
        self.root_hook(v, "on_got_hit", vec![]);
        if self.f[v].hyper_armor || (s.from_proj && self.f[v].proj_armor) {
            // armor: the damage lands, the fighter keeps doing what it does
            let dmg = self.scaled_damage(v, s.damage, s.damage_in_combo, s.min_damage);
            let (la, lv) = (self.stop(s.hitlag.attacker), self.stop(s.hitlag.victim));
            self.f[v].hp -= dmg;
            self.f[v].hitlag = lv;
            self.f[a].hitlag = self.f[a].hitlag.max(la);
            let dir = FVec::new(Fixed::from_int(self.f[a].facing), Fixed::ZERO);
            self.events.push(Event::Hit { attacker: a, victim: v, mv: s.mv, hitbox: s.hitbox.clone(), pos: s.spark, dir, damage: dmg });
            self.note(format!("P{} armored a hit ({dmg})", v + 1));
            return dmg;
        }
        if !matches!(self.f[v].act, Act::Hurt { .. }) {
            self.f[v].combo_hits = 0;
            self.f[v].combo_damage = 0;
            self.f[v].proration = 0;
        }
        self.release_hold(v);
        let dmg = self.scaled_damage(v, s.damage, s.damage_in_combo, s.min_damage);
        let in_combo = self.f[v].combo_hits > 0;
        let fa = self.f[a].facing;
        let (la, lv) = (self.stop(s.hitlag.attacker), self.stop(s.hitlag.victim));
        let stun_pct = self.rules.hitstun_pct;
        let knock_taken = fx(self.chars[v].file.character.body.knockback_taken);
        let kb = s.knockback;
        let victim_center = self.f[v].pos;
        let dir = match kb.from {
            KnockbackFrom::Direction => {
                let mut x = Fixed::from_int(fa) * fx(kb.dir.0);
                if kb.reversible && (victim_center.x - s.center.x).signum() * fa < 0 {
                    x = -x;
                }
                FVec::new(x, fx(kb.dir.1))
            }
            KnockbackFrom::TowardHitbox => s.center - victim_center,
            KnockbackFrom::AwayFromHitbox => victim_center - s.center,
        };
        let mut vel = dir.normalized().scale(fx(kb.strength) * knock_taken);
        let f = &mut self.f[v];
        let reaction = if f.grounded { s.reaction.grounded } else { s.reaction.aerial };
        let launched = match reaction {
            ReactionKind::Launch => true,
            ReactionKind::Stumble | ReactionKind::Held | ReactionKind::Dizzy => !f.grounded,
        };
        if !launched {
            vel.y = Fixed::ZERO;
        } else if f.grounded {
            f.pos.y = f.pos.y - Fixed::ONE;
            f.grounded = false;
        }
        f.hp -= dmg;
        f.combo_damage += dmg;
        if s.scaling.counts_as_hit && s.scaling.scale_combo {
            if f.combo_hits == 0 {
                f.proration = s.scaling.proration as i32;
            }
            f.combo_hits += 1;
        }
        let stun = if in_combo { s.hitstun_in_combo.unwrap_or(s.hitstun) } else { s.hitstun };
        let stun = (stun as i32 * stun_pct / 100).clamp(0, u16::MAX as i32) as u16;
        f.vel = vel;
        f.force = FVec::ZERO;
        f.hitlag = lv;
        f.act = Act::Hurt {
            stun,
            launched,
            kd: s.effects.knockdown,
            until_landing: s.effects.knockdown != Knockdown::None && s.effects.stun_until_landing,
            min_air: s.reaction.min_launch_frames,
            frame: 0,
        };
        let att = &mut self.f[a];
        att.hitlag = att.hitlag.max(la);
        if att.grounded && s.pushback != Fixed::ZERO {
            att.vel.x = Fixed::from_int(-fa) * s.pushback;
        }
        let mut spark_dir = dir.normalized();
        if !launched {
            // grounded hits that do not launch point their sparks straight along the ground
            spark_dir = FVec::new(if spark_dir.x < Fixed::ZERO { -Fixed::ONE } else { Fixed::ONE }, Fixed::ZERO);
        }
        self.events.push(Event::Hit { attacker: a, victim: v, mv: s.mv, hitbox: s.hitbox.clone(), pos: s.spark, dir: spark_dir, damage: dmg });
        dmg
    }

    fn check_time(&mut self) {
        let Some(limit) = self.rules.round_frames else { return };
        if self.frame < limit || self.winner.is_some() || self.draw {
            return;
        }
        // compare hp / max_hp without division: hp0 * max1 vs hp1 * max0
        let a = self.f[0].hp as i64 * self.f[1].max_hp as i64;
        let b = self.f[1].hp as i64 * self.f[0].max_hp as i64;
        match a.cmp(&b) {
            std::cmp::Ordering::Greater => self.winner = Some(0),
            std::cmp::Ordering::Less => self.winner = Some(1),
            std::cmp::Ordering::Equal => self.draw = true,
        }
        self.note("TIME!".into());
    }

    /// Remaining round time in frames, if the round is timed.
    pub fn time_left(&self) -> Option<u64> {
        self.rules.round_frames.map(|l| l.saturating_sub(self.frame))
    }

    fn stop(&self, t: u8) -> u16 {
        (t as i32 * self.rules.hitstop_pct / 100).clamp(0, u16::MAX as i32) as u16
    }

    fn check_ko(&mut self) {
        for v in 0..2 {
            if self.f[v].hp <= 0 && self.winner.is_none() {
                self.f[v].hp = 0;
                self.winner = Some(1 - v);
                self.note(format!("KO! P{} wins", 2 - v));
                self.events.push(Event::Ko { victim: v });
                if !matches!(self.f[v].act, Act::Hurt { launched: true, .. }) {
                    self.f[v].act = Act::Ko;
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Frame advantage readout (for tuning)
    // -----------------------------------------------------------------------------------------

    /// Can the fighter start a new action on its next tick?
    pub fn is_free(&self, i: usize) -> bool {
        let f = &self.f[i];
        match &f.act {
            Act::Idle | Act::Walk { .. } | Act::Crouch | Act::Air => f.hitlag == 0,
            Act::Move(run) => {
                let tl = &self.chars[i].mv(run.idx).timeline;
                f.hitlag == 0 && tl.end == TimelineEnd::Fixed && run.frame >= tl.total()
            }
            _ => false,
        }
    }

    fn begin_advantage(&mut self, attacker: usize, on_block: bool) {
        self.adv = Some(AdvPending { attacker, on_block, free_at: [None, None] });
    }

    fn track_advantage(&mut self) {
        let free = [self.is_free(0), self.is_free(1)];
        let frame = self.frame;
        let Some(p) = &mut self.adv else { return };
        for i in 0..2 {
            if free[i] && p.free_at[i].is_none() {
                p.free_at[i] = Some(frame);
            }
        }
        if let [Some(f0), Some(f1)] = p.free_at {
            let at = [f0, f1];
            let a = p.attacker;
            let frames = at[1 - a] as i64 - at[a] as i64;
            let on_block = p.on_block;
            self.adv = None;
            self.last_advantage = Some(Advantage { on_block, attacker: a, frames });
            let tag = if on_block { "on block" } else { "on hit" };
            self.note(format!("P{} {tag}: {frames:+}", a + 1));
        }
    }

    pub fn move_name(&self, i: usize) -> Option<(&str, u16, u16)> {
        let Act::Move(run) = &self.f[i].act else { return None };
        let mv = self.chars[i].mv(run.idx);
        Some((mv.id.as_str(), run.frame, mv.timeline.total()))
    }

    pub fn mode_name(&self, i: usize) -> Option<&str> {
        self.chars[i].modes().get(self.f[i].mode).map(|s| s.as_str())
    }
}

// ---------------------------------------------------------------------------------------------
// Physics helpers (semantics measured on the original library, see notes/04)
// ---------------------------------------------------------------------------------------------

fn apply_fric(f: &mut Fighter, body: &charfmt::Body) {
    let (fric, cap) = if f.grounded {
        (fx(body.ground_friction), fx(body.max_ground_speed))
    } else {
        (fx(body.air_friction), fx(body.max_air_speed))
    };
    f.vel.x = f.vel.x * (Fixed::ONE - fric / cap);
}

fn apply_grav(f: &mut Fighter, body: &charfmt::Body) {
    if f.vel.y < fx(body.max_fall_speed) {
        f.force.y += fx(body.gravity);
    }
}

/// velocity += queued force, optional speed caps, position += velocity, then ground contact.
fn integrate(f: &mut Fighter, caps: Option<&charfmt::Body>) {
    f.vel = f.vel + f.force;
    f.force = FVec::ZERO;
    if let Some(b) = caps {
        let cap = if f.grounded { fx(b.max_ground_speed) } else { fx(b.max_air_speed) };
        f.vel.x = f.vel.x.clamp(-cap, cap);
        f.vel.y = f.vel.y.min(fx(b.max_fall_speed));
    }
    f.pos = f.pos + f.vel;
    if f.pos.y >= Fixed::ZERO {
        f.pos.y = Fixed::ZERO;
        if f.vel.y > Fixed::ZERO {
            f.vel.y = Fixed::ZERO;
        }
        f.grounded = true;
    } else {
        f.grounded = false;
    }
}

fn world_rect(f: &Fighter, r: &Rect) -> (FVec, FVec) {
    rect_at(f.pos, f.facing, r)
}

/// What a hit records: boxes of one group count as one.
fn hit_key(hb: &charfmt::HitBox, cycle: u16) -> (String, u16) {
    (format!("g{}", hb.group), cycle)
}

fn rect_at(pos: FVec, facing: i64, r: &Rect) -> (FVec, FVec) {
    let c = FVec::new(pos.x + Fixed::from_int(r.x as i64 * facing), pos.y + Fixed::from_int(r.y as i64));
    (c, FVec::new(Fixed::from_int(r.half_w as i64), Fixed::from_int(r.half_h as i64)))
}

/// Centre of the overlap of two (centre, half size) boxes.
fn overlap_center(a: &(FVec, FVec), b: &(FVec, FVec)) -> FVec {
    let lo = |c: Fixed, h: Fixed| c - h;
    let hi = |c: Fixed, h: Fixed| c + h;
    let x0 = lo(a.0.x, a.1.x).max(lo(b.0.x, b.1.x));
    let x1 = hi(a.0.x, a.1.x).min(hi(b.0.x, b.1.x));
    let y0 = lo(a.0.y, a.1.y).max(lo(b.0.y, b.1.y));
    let y1 = hi(a.0.y, a.1.y).min(hi(b.0.y, b.1.y));
    FVec::new((x0 + x1) / Fixed::from_int(2), (y0 + y1) / Fixed::from_int(2))
}

fn overlap(a: &(FVec, FVec), b: &(FVec, FVec)) -> bool {
    (a.0.x - b.0.x).abs() <= a.1.x + b.1.x && (a.0.y - b.0.y).abs() <= a.1.y + b.1.y
}

/// The repeat cycle if `hb` can hit on `frame`, else None.
fn hitbox_live(hb: &charfmt::HitBox, frame: u16) -> Option<u16> {
    if frame < hb.window.from || (!hb.until_end && frame > hb.window.to) {
        return None;
    }
    if hb.rect.half_w == 0 || hb.rect.half_h == 0 {
        return None; // placeholder sized by a behavior we do not have yet
    }
    match hb.repeat {
        Some(r) => {
            let period = (r.active + r.inactive).max(1);
            let t = frame - hb.window.from;
            if t % period < r.active { Some(t / period) } else { None }
        }
        None => Some(0),
    }
}
