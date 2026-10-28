//! Training mode: what the dummy does (stand, block, jump, a CPU, a recording the player made),
//! how it wakes up and leaves hitstun, health and meter refills, position resets, game speed,
//! and the readouts (frame data, combo damage, input history).

use sim::{cpu, Act, Fixed, Input, World};

use crate::settings::Settings;

/// Inputs as a player sees them: `right` is forward, `left` back.
fn to_rel(i: Input, facing: i64) -> Input {
    if facing >= 0 { i } else { Input { left: i.right, right: i.left, ..i } }
}

fn from_rel(i: Input, facing: i64) -> Input {
    to_rel(i, facing)
}

fn back(w: &World, i: usize) -> Input {
    to_rel(Input { left: true, ..Input::default() }, w.f[i].facing)
}

/// Whether fighter `i`'s current move still has a low hitbox to come.
fn low_threat(w: &World, i: usize) -> bool {
    let Act::Move(run) = &w.f[i].act else { return false };
    w.chars[i].mv(run.idx).hitboxes.iter().any(|h| h.height == charfmt::Height::Low && (h.until_end || run.frame <= h.window.to))
}

fn reeling(a: &Act) -> bool {
    matches!(a, Act::Hurt { .. } | Act::Knockdown { .. } | Act::Getup { .. } | Act::Held)
}

#[derive(Default)]
pub struct Training {
    pub recording: bool,
    /// The dummy's recorded inputs, relative to its facing.
    pub tape: Vec<Input>,
    pub playing: Option<usize>,
    gap: u32,
    cpu: Option<(usize, cpu::Cpu)>,
    /// Ticks since the dummy was last hit.
    hit_timer: u32,
    block_roll: Option<(usize, bool)>,
    /// Ticks the dummy has been free (for refills).
    calm: u32,
    /// An action the dummy is in the middle of (wakeup, leaving hitstun): input and ticks left.
    reaction: Option<(Input, u32)>,
    prev: Option<Act>,
    /// P1's inputs, newest last: (numpad direction and buttons, ticks held).
    pub history: Vec<(String, u32)>,
    /// The running combo on the dummy, the last one and the best: (hits, damage).
    pub combo: (u32, i32),
    pub last_combo: (u32, i32),
    pub best: (u32, i32),
    /// P1's last move: name, startup and length in frames.
    pub last_move: Option<(String, u16, u16)>,
    speed_acc: u32,
}

impl Training {
    /// Whether the simulation advances this tick at the chosen game speed.
    pub fn speed_tick(&mut self, s: &Settings) -> bool {
        let pct = [100, 75, 50, 25][(s.train("speed") as usize).min(3)];
        self.speed_acc += pct;
        if self.speed_acc >= 100 {
            self.speed_acc -= 100;
            true
        } else {
            false
        }
    }

    /// The dummy's buttons this tick. `p1` is the player's keys (they drive the dummy while
    /// recording).
    pub fn dummy_input(&mut self, w: &World, s: &Settings, p1: Input) -> Input {
        let me = 1;
        let facing = w.f[me].facing;
        if self.recording {
            self.tape.push(to_rel(p1, facing));
            return p1;
        }
        if let Some(k) = self.playing {
            if let Some(i) = self.tape.get(k) {
                self.playing = Some(k + 1);
                return from_rel(*i, facing);
            }
            self.playing = None;
            self.gap = 40;
        }
        if let Some((i, left)) = self.reaction {
            self.reaction = (left > 1).then_some((i, left - 1));
            return from_rel(i, facing);
        }
        let choice = s.train("dummy") as usize;
        let block = |w: &World| {
            let mut i = back(w, me);
            i.down = low_threat(w, 0);
            i
        };
        match choice {
            1 => Input { down: true, ..Input::default() },
            2 => Input { up: w.frame % 70 < 2, ..Input::default() },
            3 => block(w),
            4 => {
                if self.hit_timer < 90 {
                    block(w)
                } else {
                    Input::default()
                }
            }
            5 => {
                let attack = match &w.f[0].act {
                    Act::Move(run) => Some(run.idx),
                    _ => None,
                };
                match (attack, self.block_roll) {
                    (Some(a), Some((b, on))) if a == b => {
                        if on {
                            block(w)
                        } else {
                            Input::default()
                        }
                    }
                    (Some(a), _) => {
                        let on = (w.frame.wrapping_mul(2654435761) >> 7) % 2 == 0;
                        self.block_roll = Some((a, on));
                        if on {
                            block(w)
                        } else {
                            Input::default()
                        }
                    }
                    (None, _) => {
                        self.block_roll = None;
                        Input::default()
                    }
                }
            }
            6..=8 => {
                let level = (choice - 6) as u8;
                if self.cpu.as_ref().is_none_or(|(c, _)| *c != choice) {
                    self.cpu = Some((choice, cpu::Cpu::new(w, me, cpu::Level::from_index(level), 0x5eed + choice as u64)));
                }
                self.cpu.as_mut().map(|(_, c)| c.input(w)).unwrap_or_default()
            }
            9 => {
                if !self.tape.is_empty() {
                    if self.gap > 0 {
                        self.gap -= 1;
                    } else {
                        self.playing = Some(0);
                    }
                }
                Input::default()
            }
            _ => Input::default(),
        }
    }

    /// Bookkeeping after a step: the dummy's reactions, refills, meters, combo and frame data.
    pub fn after_step(&mut self, w: &mut World, s: &Settings, p1: Input) {
        // P1's input history (numpad notation, relative to facing)
        let rel = to_rel(p1, w.f[0].facing);
        let num = match (rel.left, rel.right, rel.up, rel.down) {
            (_, true, true, _) => "9",
            (true, _, true, _) => "7",
            (_, _, true, _) => "8",
            (_, true, _, true) => "3",
            (true, _, _, true) => "1",
            (_, _, _, true) => "2",
            (_, true, _, _) => "6",
            (true, _, _, _) => "4",
            _ => "5",
        };
        let mut label = num.to_string();
        for (on, b) in [(p1.light, "L"), (p1.heavy, "H"), (p1.special, "S"), (p1.grab, "G"), (p1.dash, "D")] {
            if on {
                label.push_str(b);
            }
        }
        match self.history.last_mut() {
            Some((l, n)) if *l == label => *n += 1,
            _ => {
                self.history.push((label, 1));
                if self.history.len() > 16 {
                    self.history.remove(0);
                }
            }
        }

        // frame data of P1's moves as they start
        if let Act::Move(run) = &w.f[0].act {
            if run.frame == 1 {
                let mv = w.chars[0].mv(run.idx);
                self.last_move = Some((mv.name.clone(), mv.timeline.startup + 1, mv.timeline.total()));
            }
        }

        // combos on the dummy
        let was = self.prev.clone();
        for e in &w.events {
            if let sim::Event::Hit { victim: 1, damage, .. } = e {
                self.hit_timer = 0;
                if was.as_ref().is_some_and(reeling) && self.combo.0 > 0 {
                    self.combo.0 += 1;
                    self.combo.1 += damage;
                } else {
                    self.combo = (1, *damage);
                }
            }
        }
        self.hit_timer = self.hit_timer.saturating_add(1);
        let now = w.f[1].act.clone();
        if !reeling(&now) && self.combo.0 > 0 {
            self.last_combo = self.combo;
            if self.combo.1 > self.best.1 {
                self.best = self.combo;
            }
            self.combo = (0, 0);
        }

        // how the dummy leaves knockdown and hitstun
        let freed = was.as_ref().is_some_and(reeling) && !reeling(&now);
        if freed && self.reaction.is_none() && !self.recording && self.playing.is_none() {
            let woke = matches!(was, Some(Act::Getup { .. } | Act::Knockdown { .. }));
            let pick = if woke { s.train("wakeup") as usize } else { s.train("after_hit") as usize };
            self.reaction = match pick {
                1 => Some((Input { up: true, ..Input::default() }, 3)),
                2 => Some((Input { light: true, ..Input::default() }, 1)),
                3 => Some((Input { dash: true, left: true, ..Input::default() }, 2)),
                4 if woke => Some((Input { left: true, ..Input::default() }, 25)),
                _ => None,
            };
        }
        self.prev = Some(now);

        // refills: never a KO in training; health back once a combo is over
        for i in 0..2 {
            if w.f[i].hp <= 0 {
                w.f[i].hp = w.f[i].max_hp;
                w.f[i].act = if w.f[i].grounded { Act::Idle } else { Act::Air };
            }
        }
        w.winner = None;
        w.draw = false;
        if reeling(&w.f[1].act) || matches!(w.f[1].act, Act::Block { .. }) {
            self.calm = 0;
        } else {
            self.calm += 1;
        }
        if s.train("refill_health") > 0.5 && self.calm == 40 {
            for i in 0..2 {
                w.f[i].hp = w.f[i].max_hp;
            }
        }
        let max = w.cfg.combo.meter_max;
        match s.train("p1_meter") as usize {
            0 => w.f[0].meter = max,
            2 => w.f[0].meter = 0,
            _ => {}
        }
        match s.train("p2_meter") as usize {
            1 => w.f[1].meter = max,
            2 => w.f[1].meter = 0,
            _ => {}
        }
    }

    /// Puts the fighters where the "Reset Position" option says.
    pub fn place(w: &mut World, s: &Settings) {
        let edge = w.cfg.stage_half_width as i64 - 30;
        let (p1, p2) = match s.train("position") as usize {
            1 => (-edge + 55, -edge),
            2 => (edge - 55, edge),
            3 => (w.cfg.start_distance as i64, -(w.cfg.start_distance as i64)),
            _ => (-(w.cfg.start_distance as i64), w.cfg.start_distance as i64),
        };
        w.f[0].pos.x = Fixed::from_int(p1);
        w.f[1].pos.x = Fixed::from_int(p2);
        w.f[0].facing = if p1 < p2 { 1 } else { -1 };
        w.f[1].facing = -w.f[0].facing;
    }

    /// Clears what belongs to the previous round (the recording stays).
    pub fn reset(&mut self) {
        let tape = std::mem::take(&mut self.tape);
        let best = self.best;
        *self = Training { tape, best, ..Training::default() };
    }

    /// The readout panel.
    pub fn panel(&self, w: &World, s: &Settings) -> String {
        let mut out = String::new();
        if self.recording {
            out.push_str(&format!("RECORDING DUMMY  {} ticks  ({} stops)\n", self.tape.len(), s.hotkeys.record_dummy.label()));
        } else if self.playing.is_some() {
            out.push_str("PLAYING RECORDING\n");
        }
        if s.train("frame_data") > 0.5 {
            if let Some((name, startup, total)) = &self.last_move {
                out.push_str(&format!("{name}: startup {startup}, total {total}\n"));
            }
            if let Some(a) = &w.last_advantage {
                if a.attacker == 0 {
                    out.push_str(&format!("{} {:+}\n", if a.on_block { "on block" } else { "on hit" }, a.frames));
                }
            }
        }
        if s.train("combo_info") > 0.5 {
            let (h, d) = if self.combo.0 > 0 { self.combo } else { self.last_combo };
            out.push_str(&format!("combo {h} hits, {d} dmg   best {} dmg\n", self.best.1));
        }
        if s.train("input_display") > 0.5 {
            out.push('\n');
            for (l, n) in self.history.iter().rev().take(12) {
                out.push_str(&format!("{n:>4}  {l}\n"));
            }
        }
        out
    }
}
