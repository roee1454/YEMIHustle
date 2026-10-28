//! `cargo test -p sim --test probe -- --ignored --nocapture` with CHAR=<id>: which input starts
//! which move of an imported fighter, and whether it connects against a standing dummy.

use std::path::PathBuf;
use std::sync::Arc;

use sim::{Act, Character, EngineConfig, Fixed, Input, World};

fn library() -> PathBuf {
    std::env::var_os("YEMI_LIBRARY").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/yemi-hustle"))
}

#[test]
#[ignore]
fn probe() {
    let id = std::env::var("CHAR").unwrap_or_else(|_| "strongest".into());
    let lib = library().join("characters");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cfg = EngineConfig::load(&root.join("content/engine.ron")).unwrap();
    let me = Arc::new(Character::load(&lib.join(format!("{id}.ron")), &lib.join(format!("{id}_controls.ron"))).unwrap());
    let dummy = Arc::new(Character::load(&lib.join("ninja.ron"), &lib.join("ninja_controls.ron")).unwrap());
    let none = Input::default();
    for air in [false, true] {
        for button in ["light", "heavy", "special", "grab"] {
            for dir in ["", "fwd", "back", "up", "down"] {
                let mut w = World::new(cfg.clone(), [me.clone(), dummy.clone()]);
                w.f[0].pos.x = Fixed::from_int(-std::env::var("HALF").ok().and_then(|v| v.parse().ok()).unwrap_or(45));
                w.f[1].pos.x = Fixed::from_int(std::env::var("HALF").ok().and_then(|v| v.parse().ok()).unwrap_or(45));
                w.f[0].facing = 1;
                w.f[1].facing = -1;
                if air {
                    w.f[0].pos.y = Fixed::from_int(-60);
                    w.f[0].grounded = false;
                }
                let mut inp = Input::default();
                match button {
                    "light" => inp.light = true,
                    "heavy" => inp.heavy = true,
                    "special" => inp.special = true,
                    _ => inp.grab = true,
                }
                match dir {
                    "fwd" => inp.right = true,
                    "back" => inp.left = true,
                    "up" => inp.up = true,
                    "down" => inp.down = true,
                    _ => {}
                }
                let mut started = None;
                let hp = w.f[1].hp;
                for t in 0..150 {
                    w.step([if t < 2 { inp } else { none }, none]);
                    if started.is_none() {
                        if let Act::Move(run) = &w.f[0].act {
                            started = Some(w.chars[0].mv(run.idx).id.clone());
                        }
                    }
                }
                println!("{} {} {} {} -> {} dmg {}", id, if air { "air " } else { "gnd " }, button, if dir.is_empty() { "-" } else { dir }, started.unwrap_or("(nothing)".into()), hp - w.f[1].hp);
            }
        }
    }
}
