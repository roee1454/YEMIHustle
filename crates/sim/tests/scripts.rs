//! The original's scripts running in the simulation, on a fighter imported from a Workshop mod
//! (One Punch Man). Skipped when the library or the import is not there.

use std::path::PathBuf;
use std::sync::Arc;

use sim::{Act, Character, EngineConfig, Fixed, Input, World, NATIVE_SCRIPTS};

fn library() -> Option<PathBuf> {
    let root = std::env::var_os("YEMI_LIBRARY").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/yemi-hustle")))?;
    root.join("characters/one_punch_man.ron").is_file().then_some(root)
}

fn world() -> Option<World> {
    let lib = library()?;
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let ch = lib.join("characters");
    let c = Arc::new(Character::load(&ch.join("one_punch_man.ron"), &ch.join("one_punch_man_controls.ron")).unwrap());
    if c.file.character.moves.iter().all(|m| m.script.is_none()) {
        eprintln!("one_punch_man was imported without scripts; re-import it");
        return None;
    }
    let mut w = World::new(cfg, [c.clone(), c]);
    let loader = gdscript::FsLoader::new(vec![lib.join("mods/3139650206/res"), lib.join("game")]).with_native(NATIVE_SCRIPTS);
    let errs = w.attach_scripts(Arc::new(loader));
    assert!(errs.is_empty(), "{errs:?}");
    Some(w)
}

const NONE: Input = Input { left: false, right: false, up: false, down: false, light: false, heavy: false, special: false, grab: false, dash: false };

fn close(w: &mut World, gap: i64) {
    w.f[0].pos.x = Fixed::from_int(-gap / 2);
    w.f[1].pos.x = Fixed::from_int(gap / 2);
    w.f[0].facing = 1;
    w.f[1].facing = -1;
}

#[test]
fn normal_punch_script_gives_armor_and_projectile_invulnerability() {
    let Some(mut w) = world() else { return };
    close(&mut w, 300);
    w.force_move(0, "normal_punch");
    w.step([NONE, NONE]);
    assert_eq!(w.move_name(0).map(|m| m.0), Some("normal_punch"));
    assert!(!w.f[0].hyper_armor, "_frame_0 only");
    w.step([NONE, NONE]); // tick 1: `_frame_1` sets the armor
    assert!(w.f[0].hyper_armor);
    assert!(w.f[0].proj_invuln);
    for _ in 0..60 {
        w.step([NONE, NONE]);
    }
    assert!(!w.f[0].hyper_armor, "dropped at frame 48 / on exit");
    assert!(!w.f[0].proj_invuln);
    // node lookups (`$ThrowBox` in the shared Grab state) are not supported yet
    let problems: Vec<_> = w.scripts.as_ref().unwrap().problems().keys().filter(|k| !k.contains("$")).cloned().collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn armor_keeps_the_punch_going_through_a_hit() {
    let Some(mut w) = world() else { return };
    close(&mut w, 60);
    // P2 swings first; P1 starts the armored punch a tick later, so P2's hit lands on armor
    w.force_move(1, "normal_punch");
    w.step([NONE, NONE]);
    w.force_move(0, "normal_punch");
    w.step([NONE, NONE]);
    let hp = w.f[0].hp;
    let mut armored = false;
    for _ in 0..10 {
        w.step([NONE, NONE]);
        armored |= w.log.iter().any(|l| l.contains("P1 armored"));
    }
    assert!(armored, "P2's punch connected on the armored P1: {:?}", w.log);
    assert!(matches!(w.f[0].act, Act::Move(_)) || w.f[0].hp < hp, "P1 kept punching");
}

/// `cargo test -p sim --test scripts soak -- --ignored --nocapture`: every imported mod fighter
/// against itself for a while with scrambled inputs; prints what its scripts needed that the
/// host does not do yet.
#[test]
#[ignore]
fn soak() {
    let Some(lib) = library() else { return };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(lib.join("mods/index.json")).unwrap()).unwrap();
    for f in index["fighters"].as_array().unwrap() {
        let (id, mod_id) = (f["id"].as_str().unwrap(), f["mod_id"].as_str().unwrap());
        let ch = lib.join("characters");
        let c = Arc::new(Character::load(&ch.join(format!("{id}.ron")), &ch.join(format!("{id}_controls.ron"))).unwrap());
        let mut w = World::new(cfg.clone(), [c.clone(), c]);
        let loader = gdscript::FsLoader::new(vec![lib.join(format!("mods/{mod_id}/res")), lib.join("game")]).with_native(NATIVE_SCRIPTS);
        let errs = w.attach_scripts(Arc::new(loader));
        let mut seed = 12345u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut moves = std::collections::BTreeSet::new();
        let t0 = std::time::Instant::now();
        for _ in 0..6000 {
            let mut inp = [NONE; 2];
            for p in &mut inp {
                let r = rnd();
                *p = Input { left: r & 1 != 0, right: r & 2 != 0 && r & 1 == 0, up: r & 0x1c == 0, down: r & 0x60 == 0x60, light: r & 0x380 == 0x80, heavy: r & 0x380 == 0x100, special: r & 0x380 == 0x180, grab: r & 0x1c00 == 0x400, dash: r & 0xe000 == 0x2000 };
            }
            w.step(inp);
            for i in 0..2 {
                if let Some((m, _, _)) = w.move_name(i) {
                    moves.insert(m.to_string());
                }
            }
            if w.winner.is_some() || w.draw {
                let (c0, c1) = (w.chars[0].clone(), w.chars[1].clone());
                let sc = w.scripts.take();
                w = World::new(cfg.clone(), [c0, c1]);
                w.scripts = sc;
            }
        }
        println!("== {id}: {} setup problems, {} moves used, {:?}", errs.len(), moves.len(), t0.elapsed());
        for e in &errs {
            println!("   setup: {e}");
        }
        let mut probs: Vec<_> = w.scripts.as_ref().unwrap().problems().iter().collect();
        probs.sort_by(|a, b| b.1.cmp(a.1));
        for (p, n) in probs.iter().take(40) {
            println!("   {n:>6}  {}", p.chars().take(150).collect::<String>());
        }
    }
}

/// `YEMI_CHAR=heion cargo test -p sim --test scripts move_lengths -- --ignored --nocapture`:
/// how many ticks each move of an imported fighter stays in that move when started by id.
#[test]
#[ignore]
fn move_lengths() {
    let Some(lib) = library() else { return };
    let id = std::env::var("YEMI_CHAR").unwrap_or_else(|_| "heion".into());
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(lib.join("mods/index.json")).unwrap()).unwrap();
    let mod_id = index["fighters"].as_array().unwrap().iter().find(|f| f["id"] == id.as_str()).unwrap()["mod_id"].as_str().unwrap().to_string();
    let ch = lib.join("characters");
    let c = Arc::new(Character::load(&ch.join(format!("{id}.ron")), &ch.join(format!("{id}_controls.ron"))).unwrap());
    for mv in c.file.character.moves.clone() {
        let mut w = World::new(cfg.clone(), [c.clone(), c.clone()]);
        let loader = gdscript::FsLoader::new(vec![lib.join(format!("mods/{mod_id}/res")), lib.join("game")]).with_native(NATIVE_SCRIPTS);
        w.attach_scripts(Arc::new(loader));
        close(&mut w, 70);
        w.force_move(0, &mv.id);
        let mut n = 0;
        let mut end = String::new();
        for _ in 0..300 {
            w.step([NONE, NONE]);
            match w.move_name(0) {
                Some((m, _, _)) if m == mv.id => n += 1,
                other => {
                    end = other.map(|o| o.0.to_string()).unwrap_or_else(|| w.f[0].act.label().to_string());
                    break;
                }
            }
        }
        println!("{:<28} listed {:>4}  ran {:>4}  then {end}", mv.id, mv.timeline.total(), n);
    }
}

/// `cargo test --release -p sim --test scripts domain -- --ignored --nocapture`
#[test]
#[ignore]
fn domain() {
    let Some(lib) = library() else { return };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let ch = lib.join("characters");
    let c = Arc::new(Character::load(&ch.join("heion.ron"), &ch.join("heion_controls.ron")).unwrap());
    let mut w = World::new(cfg, [c.clone(), c.clone()]);
    let loader = gdscript::FsLoader::new(vec![lib.join("mods/3513897396/res"), lib.join("game")]).with_native(NATIVE_SCRIPTS);
    w.attach_scripts(Arc::new(loader));
    close(&mut w, 120);
    let mv = c.mv(c.find("domain_expansion").unwrap());
    println!("cost: {} bars; meter max {} ({} bars); usable with full meter: {}", mv.cost.super_bars, w.cfg.combo.meter_max, w.cfg.combo.meter_max / w.cfg.combo.super_bar, mv.cost.super_bars as i32 * w.cfg.combo.super_bar <= w.cfg.combo.meter_max);
    w.f[0].meter = w.cfg.combo.meter_max;
    // the intro state puts the cut-in on the UI layer
    if let Some(start) = c.find("start") {
        w.force_move(0, "start");
        let _ = start;
        for _ in 0..5 {
            w.step([NONE, NONE]);
        }
    }
    w.force_move(0, "domain_expansion");
    let hp = w.f[1].hp;
    for t in 0..400 {
        w.step([NONE, NONE]);
        if t % 20 == 0 {
            let objs: Vec<String> = w.projs.iter().map(|p| format!("{}[{}:{} {} f{}]", p.obj_name, c.file.character.projectiles[p.idx].id, c.file.character.projectiles[p.idx].states.get(p.state).map(|s| s.node.as_str()).unwrap_or("-"), p.anim, p.frame)).collect();
            let ui: Vec<String> = w.ui_nodes().iter().map(|n| format!("{} {}#{} at {:?}", n.path, n.anim, n.frame, n.pos)).collect();
            println!("t{t}: P1 at ({:.0},{:.0}) P2 at ({:.0},{:.0}) move {:?} P2 hp {} x {:.0} invuln {} hitlag P2 {} objs {objs:?} ui {ui:?}", w.f[0].pos.x.to_f32(), w.f[0].pos.y.to_f32(), w.f[1].pos.x.to_f32(), w.f[1].pos.y.to_f32(), w.move_name(0).map(|m| (m.0.to_string(), m.1)), w.f[1].hp, w.f[1].pos.x.to_f32(), w.f[0].script_invuln, w.f[1].hitlag);
        }
    }
    println!("P2 damage: {}", hp - w.f[1].hp);
    for (p, n) in w.scripts.as_ref().unwrap().problems() {
        println!("  {n:>4} {}", p.chars().take(140).collect::<String>());
    }
}

/// The opponent breaks Heion's domain with attacks: the domain's own `hit_by` takes the hits off
/// its lifetime and it switches to its destroyed state.
/// `cargo test --release -p sim --test scripts domain_breaks -- --ignored --nocapture`
#[test]
#[ignore]
fn domain_breaks() {
    let Some(lib) = library() else { return };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let ch = lib.join("characters");
    let c = Arc::new(Character::load(&ch.join("heion.ron"), &ch.join("heion_controls.ron")).unwrap());
    let o = Arc::new(Character::load(&ch.join("ninja.ron"), &ch.join("ninja_controls.ron")).unwrap());
    let mut w = World::new(cfg, [c.clone(), o]);
    let loader = gdscript::FsLoader::new(vec![lib.join("mods/3513897396/res"), lib.join("game")]).with_native(NATIVE_SCRIPTS);
    w.attach_scripts(Arc::new(loader));
    close(&mut w, 60);
    w.f[0].meter = w.cfg.combo.meter_max;
    w.force_move(0, "domain_expansion");
    let state = |w: &World| w.projs.iter().filter(|p| p.is_object()).map(|p| c.file.character.projectiles[p.idx].states.get(p.state).map(|s| s.node.clone()).unwrap_or_default()).collect::<Vec<_>>();
    for _ in 0..160 {
        w.step([NONE, NONE]);
    }
    println!("domain up: {:?}", state(&w));
    let mut hits = 0;
    let mut broke = None;
    for t in 0..1200 {
        // the ninja mashes attacks
        let press = t % 6 < 3;
        let b = [Input { light: press && t % 24 < 12, heavy: press && t % 24 >= 12, ..NONE }, NONE];
        w.step([b[1], b[0]]);
        hits += w.events.iter().filter(|e| matches!(e, sim::Event::ObjectHit { .. })).count();
        if broke.is_none() && state(&w).iter().any(|s| s == "DomainDestroy") {
            broke = Some(t);
        }
    }
    println!("object hits {hits}, broken at {broke:?}, states now {:?}", state(&w));
    for (p, n) in w.scripts.as_ref().unwrap().problems() {
        println!("  {n:>4} {}", p.chars().take(140).collect::<String>());
    }
    assert!(hits > 0, "the ninja's attacks never reached the domain");
    assert!(broke.is_some(), "the domain never broke");
}

/// The intro plays each mod fighter's whole "Start" state (Heion's 120-tick intro, its cut-in
/// set up on the way), then the round starts with both fighters back at their marks.
#[test]
fn start_state_plays_as_the_intro() {
    let Some(lib) = library() else { return };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let ch = lib.join("characters");
    let c = Arc::new(Character::load(&ch.join("heion.ron"), &ch.join("heion_controls.ron")).unwrap());
    let o = Arc::new(Character::load(&ch.join("ninja.ron"), &ch.join("ninja_controls.ron")).unwrap());
    let mut w = World::new(cfg, [c.clone(), o]);
    let loader = gdscript::FsLoader::new(vec![lib.join("mods/3513897396/res"), lib.join("game")]).with_native(NATIVE_SCRIPTS);
    w.attach_scripts(Arc::new(loader));
    let marks = [w.f[0].pos, w.f[1].pos];
    assert!(w.begin_intro());
    assert_eq!(w.move_name(0).map(|m| m.0.to_string()).as_deref(), Some("start"));
    let mut ticks = 0;
    let clock = w.time_left();
    while w.in_intro() && ticks < 170 {
        w.step_intro();
        ticks += 1;
        if !matches!(w.f[0].act, Act::Move(_)) {
            break;
        }
    }
    // the state ran to its end ("Wait" at its tick 119, plus its own 8-tick freeze)
    assert!((119..=135).contains(&ticks), "the intro ran {ticks} ticks");
    assert!(w.ui_nodes().iter().any(|n| n.path.contains("HeionCutin")) || w.scripts.as_ref().unwrap().problems().iter().all(|(p, _)| !p.contains("Start")), "{:?}", w.scripts.as_ref().unwrap().problems());
    assert_eq!(w.time_left(), clock, "the clock waits during the intro");
    w.end_intro();
    assert!(!w.in_intro());
    assert_eq!([w.f[0].pos, w.f[1].pos], marks);
    assert!(matches!(w.f[0].act, Act::Idle) && matches!(w.f[1].act, Act::Idle));
    for (p, n) in w.scripts.as_ref().unwrap().problems() {
        println!("  {n:>4} {p}");
    }
}

/// Gojo's perfect dodge animates by setting `host.sprite.frame` itself: the pick reaches the
/// renderer through `fighter_sprite`, and lasts only the tick it was made in.
#[test]
fn scripts_pick_the_fighters_sprite_frame() {
    let Some(lib) = library() else { return };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let ch = lib.join("characters");
    let c = Arc::new(Character::load(&ch.join("strongest.ron"), &ch.join("strongest_controls.ron")).unwrap());
    let mut w = World::new(cfg, [c.clone(), c]);
    let loader = gdscript::FsLoader::new(vec![lib.join("mods/3146271984/res"), lib.join("game")]).with_native(NATIVE_SCRIPTS);
    w.attach_scripts(Arc::new(loader));
    w.step([NONE, NONE]);
    assert!(w.force_move(0, "lbperfect_dodges"));
    let mut picked = Vec::new();
    for _ in 0..10 {
        w.step([NONE, NONE]);
        picked.push(w.fighter_sprite(0).and_then(|l| l.frame));
    }
    println!("frames picked: {picked:?}");
    assert!(picked.iter().any(Option::is_some), "the script's frame never reached the sprite");
}

/// Every imported fighter with a "Start" state plays it through as its intro and leaves it by
/// itself (not cut off by the engine).
#[test]
fn every_mod_intro_runs_to_its_end() {
    let Some(lib) = library() else { return };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ch = lib.join("characters");
    for (id, m) in [("heion", "3513897396"), ("strongest", "3146271984"), ("hakari", "3270044231")] {
        let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
        let c = Arc::new(Character::load(&ch.join(format!("{id}.ron")), &ch.join(format!("{id}_controls.ron"))).unwrap());
        let o = Arc::new(Character::load(&ch.join("ninja.ron"), &ch.join("ninja_controls.ron")).unwrap());
        let mut w = World::new(cfg, [c, o]);
        let loader = gdscript::FsLoader::new(vec![lib.join(format!("mods/{m}/res")), lib.join("game")]).with_native(NATIVE_SCRIPTS);
        w.attach_scripts(Arc::new(loader));
        assert!(w.begin_intro(), "{id} has no Start state");
        let mut ticks = 0;
        while matches!(w.f[0].act, Act::Move(_)) && ticks < 170 {
            w.step_intro();
            ticks += 1;
        }
        println!("{id}: intro left after {ticks} ticks");
        assert!(ticks < 170, "{id}'s intro never ended");
        w.end_intro();
    }
}

/// Every move of every imported mod, started by id with full meter and played out, and what
/// the scripts asked of the engine that it could not do: grouped by the API (across mods) and
/// listed per script hook. `cargo test --release -p sim --test scripts api_gaps -- --ignored --nocapture`
/// (`YEMI_CHAR=strongest` for one fighter).
#[test]
#[ignore]
fn api_gaps() {
    let Some(lib) = library() else { return };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(lib.join("mods/index.json")).unwrap()).unwrap();
    let only = std::env::var("YEMI_CHAR").ok();
    // API -> (count, fighters), and each problem line -> (count, moves)
    let mut by_api: std::collections::BTreeMap<String, (u32, std::collections::BTreeSet<String>)> = Default::default();
    let mut lines: std::collections::BTreeMap<String, (u32, std::collections::BTreeSet<String>)> = Default::default();
    for f in index["fighters"].as_array().unwrap() {
        let (id, mod_id) = (f["id"].as_str().unwrap(), f["mod_id"].as_str().unwrap());
        if only.as_deref().is_some_and(|o| o != id) {
            continue;
        }
        let ch = lib.join("characters");
        let c = Arc::new(Character::load(&ch.join(format!("{id}.ron")), &ch.join(format!("{id}_controls.ron"))).unwrap());
        let loader: Arc<dyn gdscript::Loader> = Arc::new(gdscript::FsLoader::new(vec![lib.join(format!("mods/{mod_id}/res")), lib.join("game")]).with_native(NATIVE_SCRIPTS));
        for mv in c.file.character.moves.clone() {
            let mut w = World::new(cfg.clone(), [c.clone(), c.clone()]);
            w.attach_scripts(loader.clone());
            w.step([NONE, NONE]);
            close(&mut w, 70);
            for i in 0..2 {
                w.f[i].meter = w.cfg.combo.meter_max;
            }
            let before = w.scripts.as_ref().unwrap().problems().clone();
            w.force_move(0, &mv.id);
            for _ in 0..400 {
                w.step([NONE, NONE]);
            }
            for (p, n) in w.scripts.as_ref().unwrap().problems() {
                let n = n - before.get(p).copied().unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let e = lines.entry(format!("{id}: {p}")).or_default();
                e.0 += n;
                e.1.insert(mv.id.clone());
                // the API part: after "not supported: " / "unsupported: ", or the whole note
                let api = p.rsplit_once("supported: ").map(|x| x.1).unwrap_or(p).split(" (line").next().unwrap_or("").to_string();
                let api = if api.contains(": ") { api.rsplit_once(": ").unwrap().1.to_string() } else { api };
                let a = by_api.entry(api).or_default();
                a.0 += n;
                a.1.insert(id.to_string());
            }
        }
    }
    let mut apis: Vec<_> = by_api.into_iter().collect();
    apis.sort_by(|a, b| b.1 .1.len().cmp(&a.1 .1.len()).then(b.1 .0.cmp(&a.1 .0)));
    println!("== by API ({} kinds)", apis.len());
    for (api, (n, who)) in &apis {
        println!("{n:>7}  {:<60} {:?}", api.chars().take(60).collect::<String>(), who);
    }
    println!("== by script line");
    let mut ls: Vec<_> = lines.into_iter().collect();
    ls.sort_by(|a, b| b.1 .1.len().cmp(&a.1 .1.len()));
    for (l, (n, mvs)) in ls {
        let shown: Vec<_> = mvs.iter().take(4).collect();
        println!("{n:>6} {:>3} moves  {}  {:?}", mvs.len(), l.chars().take(170).collect::<String>(), shown);
    }
}

/// `YEMI_CHAR=strongest YEMI_MOVE=infinite_void cargo test --release -p sim --test scripts trace_move -- --ignored --nocapture`:
/// one move of an imported fighter, started through its button (so `is_usable` decides), then
/// every few ticks: the states, objects, UI nodes and HP, and the script problems at the end.
#[test]
#[ignore]
fn trace_move() {
    let Some(lib) = library() else { return };
    let id = std::env::var("YEMI_CHAR").unwrap_or_else(|_| "strongest".into());
    let mv_id = std::env::var("YEMI_MOVE").unwrap_or_else(|_| "infinite_void".into());
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&repo.join("content/engine.ron")).unwrap();
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(lib.join("mods/index.json")).unwrap()).unwrap();
    let mod_id = index["fighters"].as_array().unwrap().iter().find(|f| f["id"] == id.as_str()).unwrap()["mod_id"].as_str().unwrap().to_string();
    let ch = lib.join("characters");
    let c = Arc::new(Character::load(&ch.join(format!("{id}.ron")), &ch.join(format!("{id}_controls.ron"))).unwrap());
    let o = Arc::new(Character::load(&ch.join("ninja.ron"), &ch.join("ninja_controls.ron")).unwrap());
    let mut w = World::new(cfg, [c.clone(), o]);
    let loader = gdscript::FsLoader::new(vec![lib.join(format!("mods/{mod_id}/res")), lib.join("game")]).with_native(NATIVE_SCRIPTS);
    w.attach_scripts(Arc::new(loader));
    w.step([NONE, NONE]);
    close(&mut w, 70);
    w.f[0].meter = w.cfg.combo.meter_max;
    let idx = c.find(&mv_id).expect("no such move");
    println!("usable by its button: {}", w.usable_now(0, idx));
    w.force_move(0, &mv_id);
    let hp = w.f[1].hp;
    for t in 0..600 {
        w.step([NONE, NONE]);
        if t % 15 == 0 {
            let objs: Vec<String> = w.projs.iter().filter(|p| p.is_object()).map(|p| format!("{}:{}", c.file.character.projectiles[p.idx].id, c.file.character.projectiles[p.idx].states.get(p.state).map(|s| s.node.as_str()).unwrap_or("-"))).collect();
            let ui: Vec<String> = w.ui_nodes().iter().map(|n| n.path.clone()).collect();
            println!("t{t:>3} P1 {:?} P2 {:?} hp {} objs {objs:?} ui {ui:?}", w.move_name(0).map(|m| m.0.to_string()).unwrap_or_else(|| w.f[0].act.label().into()), w.move_name(1).map(|m| m.0.to_string()).unwrap_or_else(|| w.f[1].act.label().into()), w.f[1].hp);
        }
    }
    println!("P2 damage {}", hp - w.f[1].hp);
    for (p, n) in w.scripts.as_ref().unwrap().problems() {
        println!("  {n:>4} {}", p.chars().take(170).collect::<String>());
    }
}
