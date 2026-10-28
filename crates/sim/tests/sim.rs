//! Headless simulation tests against the converted ninja.
//!
//! The converted data is private and may be absent; each test then returns early.

use std::path::PathBuf;
use std::sync::Arc;

use sim::{Act, Character, EngineConfig, Fixed, Input, World};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn world() -> Option<World> {
    let r = root();
    let ch = r.join("data/converted/ninja.ron");
    if !ch.exists() {
        eprintln!("converted ninja not present, skipping");
        return None;
    }
    let cfg = EngineConfig::load(&r.join("content/engine.ron")).unwrap();
    let c = Arc::new(Character::load(&ch, &r.join("data/converted/ninja_controls.ron")).unwrap());
    Some(World::new(cfg, [c.clone(), c]))
}

/// Puts the fighters `gap` units apart (P1 left, facing right).
fn close(w: &mut World, gap: i64) {
    w.f[0].pos.x = Fixed::from_int(-gap / 2);
    w.f[1].pos.x = Fixed::from_int(gap / 2);
    w.f[0].facing = 1;
    w.f[1].facing = -1;
}

const NONE: Input = Input { left: false, right: false, up: false, down: false, light: false, heavy: false, special: false, grab: false, dash: false };

fn light() -> Input {
    Input { light: true, ..NONE }
}

fn run(w: &mut World, p1: Input, p2: Input, ticks: usize) {
    for _ in 0..ticks {
        w.step([p1, p2]);
    }
}

#[test]
fn punch_hits_on_its_first_active_frame() {
    let Some(mut w) = world() else { return };
    close(&mut w, 40);
    let hp = w.f[1].hp;
    // grounded_punch: startup 4, so frames 5..6 are active. Tick 1 plays frame 1.
    w.step([light(), NONE]);
    run(&mut w, NONE, NONE, 3);
    assert_eq!(w.f[1].hp, hp, "no hit during startup");
    assert_eq!(w.move_name(0).map(|m| m.1), Some(4));
    w.step([NONE, NONE]); // frame 5
    assert_eq!(w.f[1].hp, hp - 35, "35 damage on the first active frame");
    assert!(matches!(w.f[1].act, Act::Hurt { .. }));
    assert_eq!(w.f[1].hitlag, 4);
    assert_eq!(w.f[0].hitlag, 4);
}

#[test]
fn a_finished_move_returns_to_engine_idle() {
    let Some(mut w) = world() else { return };
    close(&mut w, 200); // out of range: a whiff
    w.step([light(), NONE]);
    run(&mut w, NONE, NONE, 14); // grounded_punch is 14 frames
    assert_eq!(w.f[0].act, Act::Idle, "back to the engine's idle, not a converted idle move");
    // and the very next press starts a move
    w.step([Input { heavy: true, ..NONE }, NONE]);
    assert_eq!(w.move_name(0).map(|m| m.0), Some("nun_chuk_light"));
}

#[test]
fn holding_back_blocks_with_chip() {
    let Some(mut w) = world() else { return };
    close(&mut w, 40);
    let hp = w.f[1].hp;
    let back = Input { right: true, ..NONE }; // P2 faces left, so right is back
    w.step([light(), back]);
    run(&mut w, NONE, back, 4);
    assert!(matches!(w.f[1].act, Act::Block { .. }), "P2 blocks: {:?}", w.f[1].act);
    assert_eq!(w.f[1].hp, hp - 35 / 3, "chip damage");
    run(&mut w, NONE, NONE, 40);
    let adv = w.last_advantage.clone().expect("advantage measured");
    assert!(adv.on_block);
}

#[test]
fn low_hits_standing_blockers() {
    let Some(mut w) = world() else { return };
    close(&mut w, 40);
    let hp = w.f[1].hp;
    let stand_back = Input { right: true, ..NONE };
    // grounded_sweep is Light + Down; check its hit height in the data first
    let ch = w.chars[0].clone();
    let sweep = ch.mv(ch.find("grounded_sweep").unwrap());
    let low = sweep.hitboxes.iter().any(|h| h.height == charfmt::Height::Low);
    let down_light = Input { down: true, light: true, ..NONE };
    w.step([down_light, stand_back]);
    run(&mut w, Input { down: true, ..NONE }, stand_back, 12);
    if low {
        assert!(w.f[1].hp < hp - 35, "a standing block does not stop a low");
    } else {
        assert!(matches!(w.f[1].act, Act::Block { .. }) || w.f[1].hp < hp);
    }
}

#[test]
fn second_hit_of_a_combo_is_scaled() {
    let Some(mut w) = world() else { return };
    close(&mut w, 40);
    let start = w.f[1].hp;
    w.step([light(), NONE]);
    run(&mut w, NONE, NONE, 4); // first hit lands on tick 5
    let after_first = w.f[1].hp;
    // cancel into a kick while P2 is still in hitstun (the punch can cancel into grounded
    // attacks on hit; the same move twice in one combo is not allowed)
    run(&mut w, NONE, NONE, 4); // ride out hitlag
    w.step([Input { down: true, light: true, ..NONE }, NONE]);
    let (kick, _, _) = w.move_name(0).expect("the kick started");
    let base = w.chars[0].mv(w.chars[0].find(kick).unwrap()).hitboxes.iter().map(|h| h.damage).max().unwrap();
    run(&mut w, NONE, NONE, 12);
    let second = after_first - w.f[1].hp;
    assert_eq!(start - after_first, 35);
    assert!(second > 0 && second < base, "second hit scaled: {second} of {base}");
}

#[test]
fn throw_holds_damages_and_releases() {
    let Some(mut w) = world() else { return };
    close(&mut w, 30);
    let hp = w.f[1].hp;
    w.step([Input { grab: true, ..NONE }, NONE]);
    let mut held_seen = false;
    for _ in 0..60 {
        w.step([NONE, NONE]);
        if w.f[1].act == Act::Held {
            held_seen = true;
        }
    }
    assert!(held_seen, "P2 was held");
    assert!(hp - w.f[1].hp >= 100, "throw damage dealt: {}", hp - w.f[1].hp);
    assert!(!w.f[0].holding);
    assert!(!matches!(w.f[1].act, Act::Held));
}

#[test]
fn nunchuk_mode_route() {
    let Some(mut w) = world() else { return };
    close(&mut w, 40);
    let heavy = Input { heavy: true, ..NONE };
    // Neutral heavy in Normal mode is nun_chuk_light, which switches to NunChuk mode.
    w.step([heavy, NONE]);
    assert_eq!(w.move_name(0).map(|m| m.0), Some("nun_chuk_light"));
    assert_eq!(w.mode_name(0), Some("nun_chuk"));
    // After it hits, Heavy again is nun_chuk_heavy (only exists in NunChuk mode).
    let mut cancelled = false;
    for _ in 0..30 {
        w.step([NONE, NONE]);
        w.step([heavy, NONE]);
        if w.move_name(0).map(|m| m.0) == Some("nun_chuk_heavy") {
            cancelled = true;
            break;
        }
    }
    assert!(cancelled, "nun_chuk_heavy reached via cancel; log: {:#?}", w.log);
}

#[test]
fn jump_rises_and_lands() {
    let Some(mut w) = world() else { return };
    w.step([Input { up: true, ..NONE }, NONE]);
    let mut peak = Fixed::ZERO;
    let mut landed = false;
    for _ in 0..120 {
        w.step([NONE, NONE]);
        peak = peak.min(w.f[0].pos.y);
        if w.f[0].grounded && peak < Fixed::from_int(-20) {
            landed = true;
            break;
        }
    }
    assert!(peak < Fixed::from_int(-60), "jump height {}", -peak.to_f32());
    assert!(landed);
}

#[test]
fn simulation_is_deterministic() {
    let Some(mut a) = world() else { return };
    let Some(mut b) = world() else { return };
    let script = |t: u64| -> [Input; 2] {
        let p1 = Input { right: t % 40 < 25, light: t % 17 == 0, up: t % 53 == 0, heavy: t % 29 == 0, ..NONE };
        let p2 = Input { left: t % 31 < 10, right: t % 31 > 20, special: t % 23 == 0, grab: t % 61 == 0, down: t % 7 == 0, ..NONE };
        [p1, p2]
    };
    for t in 0..2000 {
        a.step(script(t));
        b.step(script(t));
    }
    assert_eq!(format!("{:?}", a.f), format!("{:?}", b.f));
    assert_eq!(a.log, b.log);
}

#[test]
fn long_random_match_does_not_panic() {
    let Some(mut w) = world() else { return };
    let mut seed: u64 = 12345;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..20000 {
        let r1 = next();
        let r2 = next();
        let mk = |r: u64| Input {
            left: r & 1 != 0, right: r & 2 != 0, up: r & 4 != 0 && r & 64 != 0, down: r & 8 != 0,
            light: r & 16 != 0, heavy: r & 32 != 0, special: r & 128 != 0, grab: r & 256 != 0 && r & 512 != 0, dash: r & 1024 != 0,
        };
        w.step([mk(r1), mk(r2)]);
        for f in &w.f {
            assert!(f.pos.x.abs() <= Fixed::from_int(600), "stays on stage");
            assert!(f.pos.y <= Fixed::ZERO, "never below ground");
        }
    }
}

const ROSTER: [&str; 5] = ["ninja", "robo", "wizard", "mutant", "cowboy"];

fn roster_char(id: &str) -> Option<Arc<Character>> {
    let d = root().join("data/converted");
    let ch = d.join(format!("{id}.ron"));
    if !ch.exists() {
        return None;
    }
    Some(Arc::new(Character::load(&ch, &d.join(format!("{id}_controls.ron"))).unwrap_or_else(|e| panic!("{id}: {e}"))))
}

#[test]
fn every_roster_character_loads_with_valid_controls() {
    for id in ROSTER {
        let Some(c) = roster_char(id) else { return };
        assert!(c.warnings.is_empty(), "{id}: {:?}", c.warnings);
        for b in &c.controls.binds {
            for m in &b.moves {
                assert!(c.find(m).is_some(), "{id}: bind names unknown move {m}");
            }
        }
        assert!(!c.controls.binds.is_empty(), "{id} has no binds");
    }
}

#[test]
fn every_pairing_survives_a_random_match() {
    let cfg = EngineConfig::load(&root().join("content/engine.ron")).unwrap();
    for a in ROSTER {
        for b in ROSTER {
            let (Some(ca), Some(cb)) = (roster_char(a), roster_char(b)) else { return };
            let mut w = World::new(cfg.clone(), [ca, cb]);
            let mut seed: u64 = 99 ^ (a.len() as u64) << 8 ^ (b.len() as u64);
            let mut next = || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed
            };
            for _ in 0..6000 {
                let mk = |r: u64| Input {
                    left: r & 1 != 0, right: r & 2 != 0, up: r & 4 != 0 && r & 64 != 0, down: r & 8 != 0,
                    light: r & 16 != 0, heavy: r & 32 != 0, special: r & 128 != 0, grab: r & 256 != 0 && r & 512 != 0, dash: r & 1024 != 0,
                };
                let (r1, r2) = (next(), next());
                w.step([mk(r1), mk(r2)]);
                if w.winner.is_some() {
                    w = World::new(cfg.clone(), w.chars.clone());
                }
            }
        }
    }
}

fn ruled(rules: sim::Rules) -> Option<World> {
    let w = world()?;
    Some(World::with_rules(w.cfg.clone(), w.chars.clone(), rules))
}

#[test]
fn match_rules_scale_damage_health_and_hitstop() {
    let Some(mut w) = ruled(sim::Rules { damage_pct: 200, hitstop_pct: 50, health_pct: [100, 50], ..Default::default() }) else { return };
    close(&mut w, 40);
    let hp = w.f[1].hp;
    assert_eq!(w.f[1].max_hp, w.f[0].max_hp / 2, "P2 starts at half health");
    w.step([light(), NONE]);
    run(&mut w, NONE, NONE, 4);
    assert_eq!(hp - w.f[1].hp, 70, "double damage");
    assert_eq!(w.f[1].hitlag, 2, "half hitstop");
}

#[test]
fn one_hit_ko_and_round_timer() {
    let Some(mut w) = ruled(sim::Rules { one_hit_ko: true, ..Default::default() }) else { return };
    close(&mut w, 40);
    w.step([light(), NONE]);
    run(&mut w, NONE, NONE, 4);
    assert_eq!(w.winner, Some(0));

    let Some(mut w) = ruled(sim::Rules { round_frames: Some(30), ..Default::default() }) else { return };
    run(&mut w, NONE, NONE, 30);
    assert!(w.draw && w.winner.is_none(), "equal health at time-out is a draw");
    assert_eq!(w.time_left(), Some(0));
}

/// The ninja with a scheme that binds Special to the shuriken (the shipped one does not).
fn shuriken_world() -> Option<World> {
    let r = root();
    let ch = r.join("data/converted/ninja.ron");
    if !ch.exists() {
        return None;
    }
    let scheme = std::env::temp_dir().join("yemi_shuriken_controls.ron");
    std::fs::write(&scheme, "ControlScheme(character: \"ninja\", binds: [Bind(button: Special, stance: Ground, moves: [\"fireball\"])])").unwrap();
    let cfg = EngineConfig::load(&r.join("content/engine.ron")).unwrap();
    let c = Arc::new(Character::load(&ch, &scheme).unwrap());
    Some(World::new(cfg, [c.clone(), c]))
}

#[test]
fn a_thrown_projectile_flies_and_hits_the_opponent_once() {
    let Some(mut w) = shuriken_world() else { return };
    close(&mut w, 120);
    let hp = w.f[1].hp;
    let special = Input { special: true, ..NONE };
    w.step([special, NONE]);
    let mut spawned = false;
    let mut start_x = Fixed::ZERO;
    for _ in 0..40 {
        w.step([NONE, NONE]);
        if let Some(p) = w.projs.first() {
            if !spawned {
                spawned = true;
                start_x = p.pos.x;
            }
        }
    }
    assert!(spawned, "the move spawns its projectile");
    assert!(w.f[1].hp < hp, "the projectile damaged the opponent");
    assert!(w.projs.is_empty(), "and is gone after hitting");
    assert!(start_x > w.f[0].pos.x, "it spawns in front of the thrower");
    assert!(w.f[0].hitlag == 0, "a projectile hit does not freeze its thrower");
}

#[test]
fn projectiles_leave_at_the_wall() {
    let Some(mut w) = shuriken_world() else { return };
    close(&mut w, 120);
    w.step([Input { special: true, ..NONE }, NONE]);
    w.f[1].pos.x = Fixed::from_int(10_000); // out of the way
    run(&mut w, NONE, NONE, 300);
    assert!(w.projs.is_empty(), "projectiles do not live forever");
}

// ---------------------------------------------------------------------------------------------
// Parity with the original game: traces recorded by tools/oracle (private; tests skip without)
// ---------------------------------------------------------------------------------------------

struct Trace {
    /// Per tick: the first live object's position (None when there is none) and P2's hp.
    rows: Vec<(Option<(i64, i64)>, i64)>,
    /// Half the starting distance between the fighters.
    char_distance: i64,
}

fn trace(name: &str) -> Option<Trace> {
    let p = root().join("data/oracle").join(format!("{name}.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    let rows = v["rows"]
        .as_array()?
        .iter()
        .map(|r| {
            let obj = r["objs"].as_array().and_then(|o| o.iter().find(|o| o["disabled"] == false)).map(|o| {
                (o["x"].as_str().unwrap().parse::<i64>().unwrap(), o["y"].as_str().unwrap().parse::<i64>().unwrap())
            });
            (obj, r["p2"]["hp"].as_i64().unwrap())
        })
        .collect();
    let char_distance = v["spec"]["char_distance"].as_i64().unwrap_or(200);
    Some(Trace { rows, char_distance })
}

/// Plays `move_id` from the oracle's start (P1 at -200, P2 at 200, facing each other) and
/// returns per tick the first projectile's position and P2's hp.
fn replay(character: &str, move_id: &str, air: Option<i64>, ticks: usize, char_distance: i64) -> Option<Vec<(Option<(i64, i64)>, i64)>> {
    let r = root();
    let ch = r.join(format!("data/converted/{character}.ron"));
    if !ch.exists() {
        return None;
    }
    let scheme = std::env::temp_dir().join(format!("yemi_oracle_{character}_{move_id}.ron"));
    std::fs::write(&scheme, format!("ControlScheme(character: \"{character}\", binds: [Bind(button: Special, stance: Any, moves: [\"{move_id}\"])])")).unwrap();
    let cfg = EngineConfig::load(&r.join("content/engine.ron")).unwrap();
    let c = Arc::new(Character::load(&ch, &scheme).unwrap());
    // the oracle's P2 is always the (idle) ninja
    let dummy = Arc::new(Character::load(&r.join("data/converted/ninja.ron"), &r.join("data/converted/ninja_controls.ron")).unwrap());
    let mut w = World::new(cfg, [c, dummy]);
    // the original's traces start with full meter (supers would otherwise wait for it)
    w.f[0].meter = w.cfg.combo.meter_max;
    close(&mut w, char_distance * 2);
    if let Some(h) = air {
        w.f[0].pos.y = Fixed::from_int(-h);
        w.f[0].grounded = false;
    }
    let mut rows = Vec::new();
    for t in 0..ticks {
        let p1 = if t == 0 { Input { special: true, ..NONE } } else { NONE };
        w.step([p1, NONE]);
        let obj = w.projs.first().map(|p| (p.pos.x.to_f32().round() as i64, p.pos.y.to_f32().round() as i64));
        rows.push((obj, w.f[1].hp as i64));
    }
    Some(rows)
}

fn assert_matches_oracle(name: &str, character: &str, move_id: &str, air: Option<i64>, tolerance: i64, relative: bool) {
    assert_matches_oracle_until(name, character, move_id, air, tolerance, relative, usize::MAX);
}

/// Like [`assert_matches_oracle`], comparing only the first `until` ticks (after that the
/// victim's own reaction, which the sim only roughly reproduces, decides what is hit).
fn assert_matches_oracle_until(name: &str, character: &str, move_id: &str, air: Option<i64>, tolerance: i64, relative: bool, until: usize) {
    let Some(want) = trace(name) else { return };
    let Some(got) = replay(character, move_id, air, want.rows.len(), want.char_distance) else { return };
    let want = Trace { rows: want.rows.into_iter().take(until).collect(), ..want };
    let got: Vec<_> = got.into_iter().take(until).collect();
    // with `relative` only the object's travel since it appeared is compared; moves whose
    // fighter walks while casting spawn from a position the converted body motion only
    // roughly reproduces, which is a separate difference
    let first = |rows: &[(Option<(i64, i64)>, i64)]| rows.iter().find_map(|r| r.0).unwrap_or((0, 0));
    let (fw, fg) = if relative { (first(&want.rows), first(&got)) } else { ((0, 0), (0, 0)) };
    let mut bad = Vec::new();
    for (t, ((w, whp), (g, ghp))) in want.rows.iter().zip(got.iter()).enumerate() {
        let ok = match (w, g) {
            (None, None) => true,
            (Some(a), Some(b)) => ((a.0 - fw.0) - (b.0 - fg.0)).abs() <= tolerance && ((a.1 - fw.1) - (b.1 - fg.1)).abs() <= tolerance,
            _ => false,
        };
        if !ok || whp != ghp {
            bad.push(format!("tick {}: original {:?} hp {}, ours {:?} hp {}", t + 1, w, whp, g, ghp));
        }
    }
    assert!(bad.is_empty(), "{name}: {} ticks differ from the original, first: {}", bad.len(), bad.iter().take(5).cloned().collect::<Vec<_>>().join(" | "));
}

#[test]
fn ninja_shuriken_matches_the_original() {
    assert_matches_oracle("ninja_fireball", "ninja", "fireball", None, 0, false);
}

#[test]
fn ninja_air_shuriken_matches_the_original() {
    assert_matches_oracle("ninja_air_fireball", "ninja", "air_fireball", Some(100), 0, false);
}

#[test]
fn mutant_caustic_spike_matches_the_original() {
    assert_matches_oracle("mutant_caustic_spike", "mutant", "caustic_spike", None, 1, true);
}

#[test]
fn mutant_shockwave_matches_the_original() {
    assert_matches_oracle("mutant_shockwave", "mutant", "shockwave", None, 1, true);
}

#[test]
fn wizard_vile_clutch_matches_the_original() {
    assert_matches_oracle("wizard_vile_clutch", "wizard", "vile_clutch", None, 0, false);
}

#[test]
fn robo_flamethrower_matches_the_original() {
    assert_matches_oracle("robo_flamethrower", "robo", "flamethrower", None, 0, false);
}

#[test]
fn wizard_spark_bomb_matches_the_original() {
    // the bomb creeps by fractions of a pixel for 140 ticks; the original's fixed-point rounding
    // differs from ours by about a tenth of a pixel by the end, so allow a pixel
    assert_matches_oracle("wizard_spark_bomb", "wizard", "spark_bomb", None, 1, false);
}


#[test]
fn robo_missiles_match_the_original() {
    // three missiles, the first blowing up on the opponent at tick 60; later ticks depend on where
    // the launched opponent is, which the converted knockback only roughly reproduces
    assert_matches_oracle_until("robo_missile", "robo", "missile", None, 1, true, 63);
}

#[test]
#[ignore]
fn debug_missile() {
    let r = root();
    let scheme = std::env::temp_dir().join("yemi_dbg.ron");
    std::fs::write(&scheme, "ControlScheme(character: \"robo\", binds: [Bind(button: Special, stance: Any, moves: [\"missile\"])])").unwrap();
    let cfg = EngineConfig::load(&r.join("content/engine.ron")).unwrap();
    let c = Arc::new(Character::load(&r.join("data/converted/robo.ron"), &scheme).unwrap());
    let d = Arc::new(Character::load(&r.join("data/converted/ninja.ron"), &r.join("data/converted/ninja_controls.ron")).unwrap());
    let mut w = World::new(cfg, [c, d]);
    close(&mut w, 400);
    for t in 0..64 {
        w.step([if t == 0 { Input { special: true, ..NONE } } else { NONE }, NONE]);
        if t > 52 { eprintln!("{} n={} {:?} p2 {:?} hp {}", t + 1, w.projs.len(), w.projs.iter().map(|p| (p.pos.x.to_f32(), p.phase, p.age)).collect::<Vec<_>>(), w.hurtboxes(1)[0].0.x.to_f32(), w.f[1].hp); }
    }
}

#[test]
fn two_air_dashes_per_jump() {
    let Some(mut w) = world() else { return };
    close(&mut w, 200);
    run(&mut w, Input { up: true, ..NONE }, NONE, 1);
    run(&mut w, NONE, NONE, 8);
    assert!(!w.f[0].grounded);
    let mut dashes = 0;
    for _ in 0..3 {
        let x = w.f[0].pos.x;
        let y = w.f[0].pos.y;
        w.step([Input { dash: true, right: true, ..NONE }, NONE]);
        if matches!(w.f[0].act, Act::Dash { air: true, .. }) {
            dashes += 1;
            // level and forward while it lasts
            run(&mut w, NONE, NONE, 5);
            assert_eq!(w.f[0].pos.y, y, "an air dash holds its height");
            assert!(w.f[0].pos.x > x, "and goes forward");
        }
        run(&mut w, NONE, NONE, 10);
        if w.f[0].grounded {
            break;
        }
    }
    assert_eq!(dashes, 2, "two air dashes, not a third");
    run(&mut w, NONE, NONE, 120);
    assert!(w.f[0].grounded && w.f[0].air_dashes == 0, "landing gives them back");
}

#[test]
fn a_dash_slips_through_projectiles() {
    let Some(mut w) = shuriken_world() else { return };
    close(&mut w, 160);
    let hp = w.f[1].hp;
    // without dashing: when the shuriken hits
    let mut probe = shuriken_world().unwrap();
    close(&mut probe, 160);
    probe.step([Input { special: true, ..NONE }, NONE]);
    let mut hit_at = None;
    for t in 1..60 {
        probe.step([NONE, NONE]);
        if probe.f[1].hp < hp {
            hit_at = Some(t);
            break;
        }
    }
    let hit_at = hit_at.expect("the shuriken reaches P2");
    // P2 dashes forward into it a few ticks before
    w.step([Input { special: true, ..NONE }, NONE]);
    for t in 1..90 {
        let p2 = if t == hit_at - 4 { Input { dash: true, left: true, ..NONE } } else { NONE };
        w.step([NONE, p2]);
    }
    assert_eq!(w.f[1].hp, hp, "the dash went through the shuriken");
}
