//! The engine's combo rules and the automatic layout, on the characters in the library.
//! Skipped when the library is not there.

use std::path::PathBuf;
use std::sync::Arc;

use sim::{Act, Character, EngineConfig, Fixed, Input, World};

fn chars_dir() -> Option<PathBuf> {
    let root = std::env::var_os("YEMI_LIBRARY").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/yemi-hustle")))?;
    let d = root.join("characters");
    d.join("cowboy.ron").is_file().then_some(d)
}

fn cfg() -> EngineConfig {
    EngineConfig::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/engine.ron")).unwrap()
}

fn load(dir: &std::path::Path, id: &str) -> Arc<Character> {
    Arc::new(Character::load(&dir.join(format!("{id}.ron")), &dir.join(format!("{id}_controls.ron"))).unwrap())
}

const NONE: Input = Input { left: false, right: false, up: false, down: false, light: false, heavy: false, special: false, grab: false, dash: false };

fn close(w: &mut World, gap: i64) {
    w.f[0].pos.x = Fixed::from_int(-gap / 2);
    w.f[1].pos.x = Fixed::from_int(gap / 2);
    w.f[0].facing = 1;
    w.f[1].facing = -1;
}

#[test]
fn every_character_loads_with_its_layout_and_fits() {
    let Some(dir) = chars_dir() else { return };
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let Some(id) = n.strip_suffix(".ron").filter(|s| !s.contains("_controls") && !s.contains("_fx")) else { continue };
        let c = load(&dir, id);
        let file = charfmt::parse(&std::fs::read_to_string(dir.join(&n)).unwrap()).unwrap();
        let layout = sim::layout::plan(&file);
        assert_eq!(layout.reserve().count(), 0, "{id}: every choice has a button");
        assert!(c.rank.iter().any(|r| *r > 0), "{id}: some move is on a button");
    }
}

/// Presses `b` on P1 every tick for `ticks` ticks after the first hit, returning the moves P1 went
/// through.
fn chain(w: &mut World, presses: &[Input]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut k = 0;
    let mut wait = 0u32;
    w.step([presses[0], NONE]);
    for _ in 0..240 {
        let hit = matches!(w.f[1].act, Act::Hurt { .. });
        let input = if hit && k + 1 < presses.len() && wait == 0 {
            k += 1;
            wait = 6;
            presses[k]
        } else {
            NONE
        };
        wait = wait.saturating_sub(1);
        w.step([input, NONE]);
        if let Some((m, _, _)) = w.move_name(0) {
            if seen.last().map(String::as_str) != Some(m) {
                seen.push(m.to_string());
            }
        }
    }
    seen
}

#[test]
fn light_then_heavy_chains_on_hit_by_the_engine_rule() {
    let Some(dir) = chars_dir() else { return };
    // the moves' own listed cancels removed: any chain below is the engine's
    let full = load(&dir, "cowboy");
    let mut file = full.file.clone();
    for m in &mut file.character.moves {
        m.cancels.clear();
    }
    let c = Arc::new(Character::new(file, full.controls.clone()).unwrap());
    let seq = [Input { light: true, ..NONE }, Input { heavy: true, ..NONE }, Input { special: true, ..NONE }];
    let run = |chains: bool| {
        let mut cfg = cfg();
        cfg.combo.chains = chains;
        let mut w = World::new(cfg, [c.clone(), c.clone()]);
        close(&mut w, 50);
        let moves = chain(&mut w, &seq);
        (moves, w.log.clone())
    };
    let (on, log) = run(true);
    let ranks: Vec<u8> = on.iter().filter_map(|m| c.find(m)).map(|i| c.rank[i]).collect();
    assert!(on.len() >= 2 && log.iter().any(|l| l.contains("chain")), "chained: {on:?} {log:?}");
    assert!(ranks.windows(2).all(|p| p[1] > p[0] || (p[0] == 1 && p[1] == 1)), "each link is a stronger button: {on:?} {ranks:?}");
    let (off, log) = run(false);
    assert!(!log.iter().any(|l| l.contains("chain")), "no chain with the rule off: {off:?} {log:?}");
}

#[test]
fn no_chain_without_contact_and_no_repeats() {
    let Some(dir) = chars_dir() else { return };
    let c = load(&dir, "cowboy");
    let mut w = World::new(cfg(), [c.clone(), c]);
    close(&mut w, 400); // far apart: the first move whiffs
    w.step([Input { light: true, ..NONE }, NONE]);
    let first = w.move_name(0).map(|m| m.0.to_string());
    w.step([NONE, NONE]);
    w.step([Input { heavy: true, ..NONE }, NONE]);
    assert_eq!(w.move_name(0).map(|m| m.0.to_string()), first, "a whiffed move does not chain");
}

#[test]
fn hustle_cancel_costs_meter_and_frees_you() {
    let Some(dir) = chars_dir() else { return };
    let c = load(&dir, "hakari");
    let mut w = World::new(cfg(), [c.clone(), c]);
    close(&mut w, 400);
    let cost = w.cfg.combo.hustle_cancel;
    w.f[0].meter = cost;
    w.step([Input { heavy: true, ..NONE }, NONE]);
    assert!(matches!(w.f[0].act, Act::Move(_)));
    w.step([NONE, NONE]);
    w.step([Input { dash: true, right: true, ..NONE }, NONE]);
    assert!(w.is_free(0) && w.f[0].vel.x > Fixed::ZERO, "free at once, moving forward: {:?}", w.f[0].act);
    assert_eq!(w.f[0].meter, 0);
    assert!(w.events.iter().any(|e| matches!(e, sim::Event::HustleCancel { who: 0, .. })));
    // without meter, Dash does nothing during a move
    w.step([NONE, NONE]);
    for _ in 0..30 {
        w.step([NONE, NONE]);
    }
    w.step([Input { heavy: true, ..NONE }, NONE]);
    w.step([NONE, NONE]);
    w.step([Input { dash: true, ..NONE }, NONE]);
    assert!(matches!(w.f[0].act, Act::Move(_)));
}

#[test]
fn supers_need_meter() {
    let Some(dir) = chars_dir() else { return };
    let c = load(&dir, "hakari");
    let supers: Vec<usize> = (0..c.file.character.moves.len()).filter(|&i| c.mv(i).cost.super_bars > 0 && c.rank[i] == 4).collect();
    if supers.is_empty() {
        return;
    }
    let both = Input { heavy: true, special: true, ..NONE };
    let mut w = World::new(cfg(), [c.clone(), c.clone()]);
    close(&mut w, 400);
    w.step([both, NONE]);
    let started = w.move_name(0).map(|m| m.0.to_string());
    assert!(started.as_deref().is_none_or(|m| c.mv(c.find(m).unwrap()).cost.super_bars == 0), "no super without meter: {started:?}");
    let mut w = World::new(cfg(), [c.clone(), c.clone()]);
    close(&mut w, 400);
    w.f[0].meter = w.cfg.combo.meter_max;
    w.step([both, NONE]);
    let m = w.move_name(0).map(|m| m.0.to_string()).expect("a super started");
    assert!(c.mv(c.find(&m).unwrap()).cost.super_bars > 0, "{m}");
    assert!(w.f[0].meter < w.cfg.combo.meter_max, "the super was paid for");
}

/// Presses for a route step.
fn keys(k: &str) -> Input {
    let mut i = NONE;
    for p in k.split('+') {
        match p {
            "light" => i.light = true,
            "heavy" => i.heavy = true,
            "special" => i.special = true,
            "grab" => i.grab = true,
            "dash" => i.dash = true,
            "up" => i.up = true,
            "down" => i.down = true,
            "fwd" => i.right = true,
            "back" => i.left = true,
            _ => {}
        }
    }
    i
}

struct Played {
    hits: u16,
    damage: i32,
    seq: Vec<String>,
    log: Vec<String>,
    done: usize,
}

/// Plays a route reactively against a standing ninja: each step waits for its condition
/// ("free": P1 can act and is close, walking in until then; "hit": P1's move connected;
/// "launched": and the opponent is airborne; "air": P1 is in the air and free).
fn play(c: &Arc<Character>, d: &Arc<Character>, route: &[(String, String)]) -> Played {
    let mut w = World::new(cfg(), [c.clone(), d.clone()]);
    w.f[1].hp = 100_000;
    let mut seq: Vec<String> = Vec::new();
    let (mut step, mut cool, mut waited, mut hits, mut log) = (0usize, 0, 0, 0u16, Vec::new());
    let mut run = 0u16;
    for t in 0..700u64 {
        w.f[0].meter = w.cfg.combo.meter_max;
        let gap = (w.f[1].pos.x - w.f[0].pos.x).abs();
        let mut input = NONE;
        if step < route.len() && cool == 0 {
            let (k, cond) = (&route[step].0, route[step].1.as_str());
            let hit = matches!(&w.f[0].act, Act::Move(r) if r.hit) && w.f[0].hitlag == 0;
            let ready = match cond {
                "free" => w.is_free(0) && w.f[0].grounded && gap < Fixed::from_int(45),
                "hit" => hit,
                "launched" => hit && matches!(w.f[1].act, Act::Hurt { launched: true, .. }),
                "air" => !w.f[0].grounded && matches!(w.f[0].act, Act::Air),
                _ => true,
            };
            if ready {
                input = keys(k);
                seq.push(format!("{t}:{k}"));
                step += 1;
                cool = 3;
                waited = 0;
            } else {
                waited += 1;
                if waited > 90 {
                    break;
                }
                if cond == "free" && w.is_free(0) && w.f[0].grounded && gap >= Fixed::from_int(45) {
                    input.right = true;
                    seq.push(format!("{t}:fwd"));
                }
            }
        } else {
            cool -= cool.min(1);
        }
        // a hit on a victim still reeling from the last one extends the combo
        let reeling = matches!(w.f[1].act, Act::Hurt { .. } | Act::Knockdown { .. }) || w.f[1].hitlag > 0;
        w.step([input, NONE]);
        for e in &w.events {
            if matches!(e, sim::Event::Hit { victim: 1, .. }) {
                run = if reeling { run + 1 } else { 1 };
                hits = hits.max(run);
            }
        }
        log.extend(w.log.iter().filter(|l| !log.contains(*l)).cloned().collect::<Vec<_>>());
        if step >= route.len() && w.is_free(0) && !matches!(w.f[1].act, Act::Hurt { .. }) {
            break;
        }
        if hits > 0 && !matches!(w.f[1].act, Act::Hurt { .. } | Act::Knockdown { .. }) && step > 1 && route.get(step).is_some_and(|s| s.1 != "free") {
            // the combo dropped before the route asked for a reset
            break;
        }
    }
    Played { hits, damage: 100_000 - w.f[1].hp, seq, log, done: step }
}

/// `YEMI_ROUTE=cowboy cargo test --release -p sim --test combos combo_finder -- --ignored --nocapture`:
/// greedily builds the longest combo the engine rules allow for a character (every button and
/// direction tried at each step, a Hustle Cancel and a jump cancel too) and prints it with its
/// inputs as a `YEMI_DEMO_SEQ` for the game's demo mode.
#[test]
#[ignore]
fn combo_finder() {
    let Some(dir) = chars_dir() else { return };
    let who = std::env::var("YEMI_ROUTE").unwrap_or_else(|_| "cowboy".into());
    let c = load(&dir, &who);
    let d = load(&dir, "ninja");
    let mut cands: Vec<String> = Vec::new();
    for b in ["light", "heavy", "special", "heavy+special"] {
        for dd in ["", "fwd+", "down+", "up+", "back+"] {
            cands.push(format!("{dd}{b}"));
        }
    }
    if let Ok(steps) = std::env::var("YEMI_ROUTE_STEPS") {
        let r: Vec<(String, String)> = steps.split(',').filter_map(|p| p.split_once(':')).map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let p = play(&c, &d, &r);
        println!("{who}: {} hits, {} damage, {} of {} steps", p.hits, p.damage, p.done, r.len());
        for l in &p.log {
            println!("{l}");
        }
        println!("SEQ={}", p.seq.join(";").replace("fwd", "right").replace("back", "left"));
        return;
    }
    // a beam search: the best few routes at each length, extended by every next press
    const BEAM: usize = 6;
    let extend = |route: &Vec<(String, String)>| -> Vec<Vec<(String, String)>> {
        let mut out = Vec::new();
        for k in &cands {
            let mut r = route.clone();
            r.push((k.clone(), "hit".into()));
            out.push(r);
            let bare = k.trim_start_matches("fwd+").trim_start_matches("down+").trim_start_matches("up+").trim_start_matches("back+");
            let mut r = route.clone();
            r.push(("dash+fwd".into(), "hit".into()));
            r.push((k.clone(), "free".into()));
            if !out.contains(&r) {
                out.push(r);
            }
            let mut r = route.clone();
            r.push(("up".into(), "launched".into()));
            r.push((bare.to_string(), "air".into()));
            if !out.contains(&r) {
                out.push(r);
            }
        }
        out
    };
    let score = |r: &Vec<(String, String)>| -> Option<(u16, i32)> {
        let p = play(&c, &d, r);
        (p.done == r.len() && p.hits > 0).then_some((p.hits, p.damage))
    };
    let mut beam: Vec<((u16, i32), Vec<(String, String)>)> = cands.iter().filter_map(|k| {
        let r = vec![(k.clone(), "free".to_string())];
        score(&r).map(|s| (s, r))
    }).collect();
    beam.sort_by(|a, b| b.0.cmp(&a.0));
    beam.truncate(BEAM);
    let mut best = beam.first().cloned().expect("some opener hits");
    for _depth in 0..8 {
        let mut next: Vec<((u16, i32), Vec<(String, String)>)> = Vec::new();
        for (s0, r0) in &beam {
            for r in extend(r0) {
                if let Some(s) = score(&r).filter(|s| s.0 > s0.0) {
                    next.push((s, r));
                }
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_by(|a, b| b.0.cmp(&a.0));
        next.dedup_by(|a, b| a.0 == b.0);
        next.truncate(BEAM);
        if next[0].0 > best.0 {
            best = next[0].clone();
        }
        beam = next;
    }
    let route = best.1;
    let p = play(&c, &d, &route);
    println!("{who}: {} hits, {} damage", p.hits, p.damage);
    println!("route: {}", route.iter().map(|(k, c)| format!("{k} ({c})")).collect::<Vec<_>>().join(" > "));
    for l in &p.log {
        println!("{l}");
    }
    println!("SEQ={}", p.seq.join(";").replace("fwd", "right").replace("back", "left"));
}

/// Plays a CPU against another (or a standing dummy when `b` is None) until a winner or time.
fn cpu_match(dir: &std::path::Path, ca: &str, cb: &str, a: sim::cpu::Level, b: Option<sim::cpu::Level>, seed: u64) -> World {
    let mut cfg = cfg();
    cfg.combo.hustle_cancel = 50;
    let mut w = World::with_rules(cfg, [load(dir, ca), load(dir, cb)], sim::Rules { round_frames: Some(60 * 60), ..Default::default() });
    let mut p1 = sim::cpu::Cpu::new(&w, 0, a, seed);
    let mut p2 = b.map(|l| sim::cpu::Cpu::new(&w, 1, l, seed ^ 0xabcdef));
    for _ in 0..60 * 60 {
        let i1 = p1.input(&w);
        let i2 = p2.as_mut().map(|c| c.input(&w)).unwrap_or(NONE);
        w.step([i1, i2]);
        if w.winner.is_some() || w.draw {
            break;
        }
    }
    w
}

#[test]
fn the_cpu_fights() {
    let Some(dir) = chars_dir() else { return };
    use sim::cpu::Level::*;
    // against a dummy that stands still, every level deals damage
    for l in [Easy, Normal, Hard] {
        let w = cpu_match(&dir, "cowboy", "ninja", l, None, 7);
        assert!(w.f[1].hp < w.f[1].max_hp, "{l:?} hurt the dummy");
    }
    // hard beats easy more often than not
    let mut hard_wins = 0;
    for seed in 1..=6 {
        let w = cpu_match(&dir, "ninja", "ninja", Hard, Some(Easy), seed);
        let share = |i: usize| w.f[i].hp as f64 / w.f[i].max_hp as f64;
        if w.winner == Some(0) || (w.winner.is_none() && share(0) > share(1)) {
            hard_wins += 1;
        }
    }
    assert!(hard_wins >= 4, "hard won {hard_wins} of 6 against easy");
}

#[test]
fn a_cpu_match_replays_the_same() {
    let Some(dir) = chars_dir() else { return };
    let a = cpu_match(&dir, "hakari", "robo", sim::cpu::Level::Normal, Some(sim::cpu::Level::Hard), 3);
    let b = cpu_match(&dir, "hakari", "robo", sim::cpu::Level::Normal, Some(sim::cpu::Level::Hard), 3);
    assert_eq!((a.frame, a.f[0].hp, a.f[1].hp), (b.frame, b.f[0].hp, b.f[1].hp));
}

#[test]
fn a_press_early_in_hitstop_still_chains() {
    let Some(dir) = chars_dir() else { return };
    let c = load(&dir, "cowboy");
    let mut w = World::new(cfg(), [c.clone(), c]);
    close(&mut w, 50);
    w.step([Input { light: true, ..NONE }, NONE]);
    // wait for the hit, then press Heavy once, on the first frame of the freeze
    for _ in 0..30 {
        if w.f[0].hitlag > 0 {
            break;
        }
        w.step([NONE, NONE]);
    }
    let freeze = w.f[0].hitlag;
    assert!(freeze > 0, "the light hit");
    w.step([Input { heavy: true, ..NONE }, NONE]);
    for _ in 0..freeze + 4 {
        w.step([NONE, NONE]);
    }
    assert!(w.log.iter().any(|l| l.contains("chain") || l.contains("cancel")), "the early press came out after the freeze ({freeze} ticks): {:?}", w.log);
}
