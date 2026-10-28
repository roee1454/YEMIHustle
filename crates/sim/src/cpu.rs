//! A CPU opponent. Every tick it looks at the world and returns the buttons it holds, the way a
//! player would: it reads the character's own control scheme to know which button starts which
//! move, then spaces, attacks, blocks, anti-airs and continues combos with the engine's chains.
//! Its level sets how fast it reacts and how often it does the right thing. A seeded generator
//! makes it deterministic, so a match against it replays the same.

use charfmt::{Button, Direction, HitKind, Height, Stance};

use crate::{Act, Fixed, Input, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Easy,
    Normal,
    Hard,
}

impl Level {
    pub fn from_index(i: u8) -> Level {
        match i {
            0 => Level::Easy,
            1 => Level::Normal,
            _ => Level::Hard,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Easy => "easy",
            Level::Normal => "normal",
            Level::Hard => "hard",
        }
    }
}

/// How a level plays.
struct Profile {
    /// Ticks between neutral decisions.
    think: u16,
    /// Ticks before it reacts to an attack or a jump.
    reaction: u64,
    block: u32,
    anti_air: u32,
    combo: u32,
    attack: u32,
    /// Uses Hustle Cancels to extend combos.
    hustle: bool,
}

fn profile(l: Level) -> Profile {
    match l {
        Level::Easy => Profile { think: 20, reaction: 16, block: 30, anti_air: 15, combo: 35, attack: 30, hustle: false },
        Level::Normal => Profile { think: 10, reaction: 9, block: 62, anti_air: 50, combo: 75, attack: 45, hustle: false },
        Level::Hard => Profile { think: 5, reaction: 4, block: 88, anti_air: 85, combo: 95, attack: 60, hustle: true },
    }
}

/// A move the CPU can start from a button, and what it knows about it.
#[derive(Clone, Debug)]
struct Option_ {
    button: Button,
    dir: Direction,
    air: bool,
    idx: usize,
    startup: u16,
    /// How far in front of the body its hitboxes reach.
    reach: i64,
    low: bool,
    /// Hits above the head (anti-air).
    high_reach: bool,
    projectile: bool,
    grab: bool,
    meter: i32,
}

pub struct Cpu {
    pub me: usize,
    pub level: Level,
    rng: u64,
    options: Vec<Option_>,
    last: Input,
    /// Held directions between decisions, and until when.
    hold: Input,
    hold_until: u64,
    next_think: u64,
    /// The opponent's attack being reacted to: (move index, tick seen, will block).
    threat: Option<(usize, u64, bool)>,
    /// The opponent's jump being reacted to: (tick seen, will anti-air).
    jump: Option<(u64, bool)>,
    /// The last own hit a combo decision was made for: (move index, frame).
    combo_seen: Option<(usize, u16)>,
    air_attacked: bool,
    /// A press to make on the next tick (after a Hustle Cancel, the follow-up).
    queued: Option<Input>,
}

impl Cpu {
    pub fn new(w: &World, me: usize, level: Level, seed: u64) -> Cpu {
        let ch = &w.chars[me];
        let mut options = Vec::new();
        for b in ch.controls.binds.iter().filter(|b| !b.cancel_only) {
            if !matches!(b.button, Button::Light | Button::Heavy | Button::Special | Button::Super | Button::Grab) {
                continue;
            }
            let Some(idx) = b.moves.iter().find_map(|m| ch.find(m)) else { continue };
            let mv = ch.mv(idx);
            let hits: Vec<_> = mv.hitboxes.iter().filter(|h| h.kind != HitKind::Detect).collect();
            let projectile = !mv.spawns.is_empty();
            if hits.is_empty() && !projectile {
                continue;
            }
            let reach = if projectile { 400 } else { hits.iter().map(|h| h.rect.x as i64 + h.rect.half_w as i64).max().unwrap_or(0) };
            options.push(Option_ {
                button: b.button,
                dir: b.dir,
                air: b.stance == Stance::Air,
                idx,
                startup: mv.timeline.startup.max(1),
                reach,
                low: hits.iter().any(|h| h.height == Height::Low),
                high_reach: hits.iter().any(|h| (h.rect.y - h.rect.half_h) < -45),
                projectile,
                grab: hits.iter().any(|h| h.effects.throw),
                meter: mv.cost.super_bars as i32 * w.cfg.combo.super_bar,
            });
        }
        Cpu {
            me,
            level,
            rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            options,
            last: Input::default(),
            hold: Input::default(),
            hold_until: 0,
            next_think: 0,
            threat: None,
            jump: None,
            combo_seen: None,
            air_attacked: false,
            queued: None,
        }
    }

    fn roll(&mut self, pct: u32) -> bool {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % 100) < pct as u64
    }

    fn pick(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n.max(1) as u64) as usize
    }

    /// The buttons to hold this tick.
    pub fn input(&mut self, w: &World) -> Input {
        let want = self.decide(w);
        // a button held last tick is no new press: let go for a tick first
        let mut out = want;
        for (now, before) in [
            (&mut out.light, self.last.light),
            (&mut out.heavy, self.last.heavy),
            (&mut out.special, self.last.special),
            (&mut out.grab, self.last.grab),
            (&mut out.dash, self.last.dash),
        ] {
            if *now && before {
                *now = false;
            }
        }
        if out.up && self.last.up && !want.up {
            out.up = false;
        }
        self.last = out;
        out
    }

    /// Directions relative to where the CPU faces, as screen keys.
    fn dirs(&self, w: &World, d: Direction) -> Input {
        let fwd_right = w.f[self.me].facing > 0;
        match d {
            Direction::Forward => Input { right: fwd_right, left: !fwd_right, ..Input::default() },
            Direction::Back => Input { right: !fwd_right, left: fwd_right, ..Input::default() },
            Direction::Up => Input { up: true, ..Input::default() },
            Direction::Down => Input { down: true, ..Input::default() },
            _ => Input::default(),
        }
    }

    fn press(&self, w: &World, o: &Option_) -> Input {
        let mut i = self.dirs(w, o.dir);
        match o.button {
            Button::Light => i.light = true,
            Button::Heavy => i.heavy = true,
            Button::Special => i.special = true,
            Button::Super => {
                i.heavy = true;
                i.special = true;
            }
            Button::Grab => i.grab = true,
            _ => {}
        }
        i
    }

    fn decide(&mut self, w: &World) -> Input {
        let me = self.me;
        let o = 1 - me;
        let f = &w.f[me];
        let opp = &w.f[o];
        let p = profile(self.level);
        let now = w.frame;
        let dist = (opp.pos.x - f.pos.x).abs().round();
        if let Some(q) = self.queued.take() {
            return q;
        }
        if matches!(f.act, Act::Hurt { .. } | Act::Knockdown { .. } | Act::Getup { .. } | Act::Held | Act::Ko) || w.winner.is_some() {
            self.threat = None;
            return Input::default();
        }

        // continuing a combo: the next stronger button on hit
        if let Act::Move(run) = &f.act {
            if run.hit && f.hitlag <= 2 && self.combo_seen != Some((run.idx, run.frame)) {
                self.combo_seen = Some((run.idx, run.frame));
                let rank = w.chars[me].rank[run.idx];
                if self.roll(p.combo) {
                    let mut next: Vec<Option_> = self
                        .options
                        .iter()
                        .filter(|x| x.air == !f.grounded && !x.projectile && !x.grab && x.meter <= f.meter)
                        .filter(|x| {
                            let r = w.chars[me].rank[x.idx];
                            (r > rank || (r == 1 && rank == 1 && x.idx != run.idx)) && !f.chain.contains(&x.idx)
                        })
                        .filter(|x| x.reach + 12 >= dist)
                        .cloned()
                        .collect();
                    next.sort_by_key(|x| (w.chars[me].rank[x.idx], x.startup));
                    if let Some(x) = next.first() {
                        return self.press(w, x);
                    }
                    let hustle = w.cfg.combo.hustle_cancel;
                    if p.hustle && hustle > 0 && f.meter >= hustle && matches!(opp.act, Act::Hurt { .. }) {
                        // dash out, then start a fresh string
                        let starter = self.options.iter().filter(|x| !x.air && !x.projectile && !x.grab && x.meter == 0 && !f.chain.contains(&x.idx)).min_by_key(|x| x.startup).cloned();
                        if let Some(s) = starter {
                            self.queued = Some(self.press(w, &s));
                            let mut d = self.dirs(w, Direction::Forward);
                            d.dash = true;
                            return d;
                        }
                    }
                }
            }
            return Input::default();
        }

        // defense: block an attack that can reach, after reacting to it
        let threat = match &opp.act {
            Act::Move(run) => {
                let mv = w.chars[o].mv(run.idx);
                let live = mv.hitboxes.iter().any(|h| h.kind != HitKind::Detect && !h.effects.throw && (h.until_end || run.frame <= h.window.to));
                let reach = mv.hitboxes.iter().map(|h| h.rect.x as i64 + h.rect.half_w as i64).max().unwrap_or(0) + 25;
                let close = dist <= reach || !w.projs.is_empty();
                (live && close).then(|| (run.idx, mv.hitboxes.iter().any(|h| h.height == Height::Low && run.frame <= h.window.to)))
            }
            _ => None,
        };
        let incoming = w.projs.iter().any(|pr| pr.owner == o && (pr.pos.x - f.pos.x).abs() < Fixed::from_int(90));
        match (threat, self.threat) {
            (Some((idx, _)), Some((seen_idx, _, _))) if idx == seen_idx => {}
            (Some((idx, _)), _) => {
                let block = self.roll(p.block);
                self.threat = Some((idx, now, block));
            }
            (None, _) if !incoming => self.threat = None,
            _ => {}
        }
        if incoming && self.threat.is_none() {
            let block = self.roll(p.block);
            self.threat = Some((usize::MAX, now, block));
        }
        if let Some((_, seen, block)) = self.threat {
            if block && now >= seen + p.reaction {
                let low = threat.is_some_and(|t| t.1);
                let mut i = self.dirs(w, Direction::Back);
                i.down = low;
                return i;
            }
        }

        // anti-air: the opponent jumping in
        if !opp.grounded && f.grounded {
            if self.jump.is_none() {
                let aa = self.roll(p.anti_air);
                self.jump = Some((now, aa));
            }
            if let Some((seen, true)) = self.jump {
                let coming = dist < 75 && (opp.pos.y < Fixed::from_int(-15));
                if now >= seen + p.reaction && coming {
                    let aa = self.options.iter().filter(|x| !x.air && x.high_reach && !x.projectile && x.meter <= f.meter).min_by_key(|x| x.startup).cloned();
                    if let Some(x) = aa {
                        self.jump = Some((seen, false));
                        return self.press(w, &x);
                    }
                }
            }
        } else if opp.grounded {
            self.jump = None;
        }

        // in the air: one attack on the way down when close
        if !f.grounded {
            if !self.air_attacked && dist < 55 && f.vel.y > Fixed::ZERO {
                let pick: Vec<Option_> = self.options.iter().filter(|x| x.air && !x.projectile && x.reach + 10 >= dist && x.meter <= f.meter).cloned().collect();
                if !pick.is_empty() {
                    self.air_attacked = true;
                    let k = self.pick(pick.len());
                    return self.press(w, &pick[k]);
                }
            }
            return self.hold;
        }
        self.air_attacked = false;

        // neutral: think every few ticks, hold the chosen directions in between
        if now < self.next_think {
            return if now < self.hold_until { self.hold } else { Input::default() };
        }
        self.next_think = now + p.think as u64;
        let free = w.is_free(me);
        if !free {
            return Input::default();
        }
        let in_range: Vec<Option_> = self
            .options
            .iter()
            .filter(|x| !x.air && !x.projectile && x.reach + 4 >= dist && x.meter <= f.meter)
            .cloned()
            .collect();
        let blocking = matches!(opp.act, Act::Block { .. }) || (opp.input.right && opp.facing < 0) || (opp.input.left && opp.facing > 0);
        if !in_range.is_empty() && self.roll(p.attack) {
            if blocking && dist < 35 {
                if let Some(g) = in_range.iter().find(|x| x.grab).cloned() {
                    if self.roll(60) {
                        return self.press(w, &g);
                    }
                }
            }
            let mut strikes: Vec<Option_> = in_range.into_iter().filter(|x| !x.grab).collect();
            // against a standing guard, lows first
            let standing_guard = blocking && !opp.input.down;
            strikes.sort_by_key(|x| (standing_guard && !x.low, x.startup));
            strikes.truncate(4);
            if !strikes.is_empty() {
                let k = self.pick(strikes.len());
                return self.press(w, &strikes[k]);
            }
        }
        let shot = self.options.iter().filter(|x| !x.air && x.projectile && x.meter <= f.meter).min_by_key(|x| x.startup).cloned();
        if let (Some(s), true) = (shot, dist > 140) {
            if self.roll(25) {
                return self.press(w, &s);
            }
        }
        let r = self.pick(100) as u32;
        let (hold, ticks) = if dist > 60 {
            match r {
                0..=64 => (self.dirs(w, Direction::Forward), p.think as u64 * 2),
                65..=84 => {
                    let mut d = self.dirs(w, Direction::Forward);
                    d.dash = true;
                    (d, 2)
                }
                _ => {
                    let mut d = self.dirs(w, Direction::Forward);
                    d.up = true;
                    (d, 3)
                }
            }
        } else {
            match r {
                0..=34 => (self.dirs(w, Direction::Forward), p.think as u64),
                35..=54 => (self.dirs(w, Direction::Back), p.think as u64),
                55..=69 => (Input { down: true, ..self.dirs(w, Direction::Back) }, p.think as u64),
                _ => (Input::default(), p.think as u64),
            }
        };
        self.hold = hold;
        self.hold_until = now + ticks;
        hold
    }
}
