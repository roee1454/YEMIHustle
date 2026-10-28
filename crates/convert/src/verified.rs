//! Launch data checked against the original game itself.
//!
//! `tools/oracle` runs the user's own copy of the game headless and records what every object
//! does tick by tick. The values here are what those traces and the original's scripts say
//! (spawn frame, spawn offset, per-tick travel, forces, friction, gravity); the sim tests replay
//! them against the traces when the private files are present. A projectile or move that is not
//! listed keeps the converter's draft values, which are rough.

use crate::ron::R;

/// A move's spawn: the frame (1-based) the object appears and where, relative to the fighter.
struct Spawn {
    move_id: &'static str,
    at: i64,
    offset: (i64, i64),
    projectile: &'static str,
    /// The fighter's own gravity starts on this frame (the script gates it on the spawn).
    gravity_from: Option<i64>,
}

/// How a projectile moves once it exists.
struct Fly {
    projectile: &'static str,
    /// Travel per tick (+x forward, +y down) at the default aim.
    velocity: (f64, f64),
    /// Pending push on the first tick (after friction), +x forward.
    force: (f64, f64),
    friction: (f64, f64),
    gravity: f64,
    /// 0 keeps it until it hits something or leaves the stage.
    lifetime: i64,
    /// Keeps flying after it hits.
    pierces: bool,
    /// `_frame_N` hooks: (its own tick, what happens).
    events: &'static [(i64, Ev)],
    /// Another projectile that appears where this one ends.
    on_disable: Option<&'static str>,
    /// A burning patch: (half width, damage, every n ticks).
    zone: Option<(f64, i64, i64)>,
}

#[derive(Clone, Copy)]
enum Ev {
    Friction(f64, f64),
    Disable,
}

const fn fly(projectile: &'static str) -> Fly {
    Fly { projectile, velocity: (0.0, 0.0), force: (0.0, 0.0), friction: (0.0, 0.0), gravity: 0.0, lifetime: 0, pierces: false, events: &[], on_disable: None, zone: None }
}

struct Table {
    /// Moves whose total length the draft got wrong: (move id, frames).
    lengths: &'static [(&'static str, i64)],
    spawns: &'static [Spawn],
    flies: &'static [Fly],
}

/// Ninja shurikens: `ShurikenDefault` moves the object by `aim * 7.5 * 1.5` each tick and applies
/// no gravity for the first 120 ticks; the move's startup lag delays its spawn frame by 3.
const NINJA: Table = Table {
    lengths: &[],
    spawns: &[
        Spawn { move_id: "fireball", at: 9, offset: (16, -22), projectile: "stickman_fireball", gravity_from: None },
        Spawn { move_id: "air_fireball", at: 9, offset: (16, -16), projectile: "stickman_air_fireball", gravity_from: Some(10) },
    ],
    flies: &[
        Fly { velocity: (11.25, 0.0), ..fly("stickman_fireball") },
        // default aim as the aim widget reports it: (71, 71) / 100 * 7.5 * 1.5
        Fly { velocity: (7.9875, 7.9875), ..fly("stickman_air_fireball") },
    ],
};

/// Mutant: the spike is pushed by `PUSH 10 + aim * 9` and slows by 4% a tick for 40 ticks; the
/// shockwave walks 3.5 a tick until it hits something or reaches a wall.
const MUTANT: Table = Table {
    lengths: &[],
    spawns: &[
        Spawn { move_id: "caustic_spike", at: 19, offset: (18, -20), projectile: "caustic_spike", gravity_from: None },
        Spawn { move_id: "shockwave", at: 13, offset: (30, 0), projectile: "shockwave", gravity_from: None },
    ],
    flies: &[
        Fly { force: (19.0, 0.0), friction: (0.04, 0.04), pierces: true, events: &[(20, Ev::Friction(0.14, 0.14)), (38, Ev::Disable)], ..fly("caustic_spike") },
        Fly { velocity: (3.5, 0.0), ..fly("shockwave") },
    ],
};

/// Robo: the flamethrower leaves fire 100 ahead that burns 6 every 3 ticks for 120 ticks.
const ROBO: Table = Table {
    // the missile state re-enters itself until three missiles are out
    lengths: &[("missile", 15)],
    spawns: &[
        Spawn { move_id: "flamethrower", at: 12, offset: (100, 0), projectile: "ground_fire", gravity_from: None },
        Spawn { move_id: "flamethrower2", at: 12, offset: (100, 0), projectile: "ground_fire", gravity_from: None },
        // three missiles, one every 4 ticks
        Spawn { move_id: "missile", at: 6, offset: (16, -36), projectile: "missile", gravity_from: None },
        Spawn { move_id: "missile", at: 10, offset: (16, -36), projectile: "missile", gravity_from: None },
        Spawn { move_id: "missile", at: 14, offset: (16, -36), projectile: "missile", gravity_from: None },
    ],
    flies: &[
        Fly { zone: Some((100.0, 6, 3)), events: &[(121, Ev::Disable)], ..fly("ground_fire") },
        Fly { zone: Some((50.0, 6, 3)), events: &[(61, Ev::Disable)], ..fly("ground_fire_loic") },
        // steered by the sim's `robo.missile` behavior; blows up wherever it ends
        Fly { on_disable: Some("missile_explosion"), ..fly("missile") },
        Fly { events: &[(10, Ev::Disable)], ..fly("missile_explosion") },
    ],
};

/// Wizard: the vile clutch rises from the ground `distance * 500` ahead (the aim widget, fixed at
/// 80 for now) and lasts 32 ticks.
const WIZARD: Table = Table {
    lengths: &[],
    spawns: &[
        Spawn { move_id: "vile_clutch", at: 9, offset: (400, 0), projectile: "vile_clutch", gravity_from: None },
        Spawn { move_id: "spark_bomb", at: 8, offset: (40, -16), projectile: "spark_bomb", gravity_from: None },
    ],
    // the spark bomb is steered by the sim's `wizard.spark_bomb` behavior
    flies: &[Fly { pierces: true, events: &[(32, Ev::Disable)], ..fly("vile_clutch") }, Fly { pierces: true, ..fly("spark_bomb") }],
};

pub fn apply(char_id: &str, moves: &mut [R], projectiles: &mut [R]) {
    let table = match char_id {
        "ninja" => &NINJA,
        "mutant" => &MUTANT,
        "robo" => &ROBO,
        "wizard" => &WIZARD,
        _ => return,
    };
    for (id, frames) in table.lengths {
        let Some(mv) = moves.iter_mut().find(|m| matches!(m.get("id"), Some(R::Str(m_id)) if m_id == id)) else { continue };
        if let Some(mut tl) = mv.get("timeline").cloned() {
            let get = |tl: &R, k: &str| match tl.get(k) {
                Some(R::Int(v)) => *v,
                _ => 0,
            };
            let recovery = (frames - get(&tl, "startup") - get(&tl, "active")).max(0);
            tl.put("recovery", R::Int(recovery));
            mv.put("timeline", tl);
        }
    }
    // a move lists all its spawns; start from none so the draft's guess is replaced
    for s in table.spawns {
        if let Some(mv) = moves.iter_mut().find(|m| matches!(m.get("id"), Some(R::Str(id)) if id == s.move_id)) {
            mv.put("spawns", R::List(vec![]));
        }
    }
    for s in table.spawns {
        if let Some(mv) = moves.iter_mut().find(|m| matches!(m.get("id"), Some(R::Str(id)) if id == s.move_id)) {
            let spawn = R::st("Spawn", vec![("at", R::Int(s.at)), ("projectile", R::str(s.projectile)), ("offset", R::tup2(R::Int(s.offset.0), R::Int(s.offset.1)))]);
            if let Some(R::List(l)) = mv.get("spawns").cloned() {
                let mut l = l;
                l.push(spawn);
                mv.put("spawns", R::List(l));
            }
            if let Some(from) = s.gravity_from {
                if let Some(mut ph) = mv.get("physics").cloned() {
                    ph.put("gravity_from", R::Int(from));
                    mv.put("physics", ph);
                }
            }
        }
    }
    for f in table.flies {
        let Some(p) = projectiles.iter_mut().find(|p| matches!(p.get("id"), Some(R::Str(id)) if id == f.projectile)) else { continue };
        let pair = |v: (f64, f64)| R::tup2(R::Float(v.0), R::Float(v.1));
        p.put("velocity", pair(f.velocity));
        if f.force != (0.0, 0.0) {
            p.put("force", pair(f.force));
        }
        if f.friction != (0.0, 0.0) {
            p.put("friction", pair(f.friction));
        }
        if f.gravity != 0.0 {
            p.put("gravity", R::Float(f.gravity));
        } else {
            p.remove("gravity");
        }
        p.put("lifetime", R::Int(f.lifetime));
        if let Some((w, damage, every)) = f.zone {
            p.put("zone", R::Some(Box::new(R::st("Zone", vec![("half_width", R::Float(w)), ("damage", R::Int(damage)), ("every", R::Int(every))]))));
        }
        if let Some(child) = f.on_disable {
            p.put("on_disable", R::Some(Box::new(R::str(child))));
        }
        if f.pierces {
            p.put("pierces", R::Bool(true));
        }
        if !f.events.is_empty() {
            let ev = |(at, e): &(i64, Ev)| {
                let kind = match *e {
                    Ev::Friction(x, y) => R::Call("SetFriction".into(), vec![pair((x, y))]),
                    Ev::Disable => R::s("Disable"),
                };
                R::st("ProjEvent", vec![("at", R::Int(*at)), ("kind", kind)])
            };
            p.put("events", R::List(f.events.iter().map(ev).collect()));
        }
    }
}
