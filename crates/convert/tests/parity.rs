//! The Rust converter must reproduce its reference output (first the Python tools'; the
//! projectile launch data added since is in the reference files too). The reference files are
//! the earlier Python output in `data/` (private, so the tests skip when absent) and a built
//! game library for the decompiled game files.

use std::path::{Path, PathBuf};

use serde_json::Value;

const CHARACTERS: [(&str, &str, &str); 5] = [
    ("ninja", "characters/stickman", "NinjaGuy.tscn"),
    ("robo", "characters/robo", "Robot.tscn"),
    ("wizard", "characters/wizard", "Wizard.tscn"),
    ("mutant", "characters/mutant", "Mutant.tscn"),
    ("cowboy", "characters/swordandgun", "SwordGuy.tscn"),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The decompiled game: the reference copy, or `YEMI_GAME` for a built library's `game` folder.
fn game() -> Option<PathBuf> {
    let p = std::env::var_os("YEMI_GAME").map(PathBuf::from).unwrap_or_else(|| root().join("reference/yomi_recovered"));
    p.join("characters/BaseChar.tscn").exists().then_some(p)
}

/// First difference between two JSON values, as a path.
fn diff(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                match y.get(k) {
                    None => return Some(format!("{path}/{k}: missing in expected")),
                    Some(w) => {
                        if let Some(d) = diff(v, w, &format!("{path}/{k}")) {
                            return Some(d);
                        }
                    }
                }
            }
            y.keys().find(|k| !x.contains_key(*k)).map(|k| format!("{path}/{k}: missing in ours"))
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} vs {}", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (v, w))| diff(v, w, &format!("{path}[{i}]")))
        }
        _ if a == b => None,
        _ => Some(format!("{path}: ours {a} vs expected {b}")),
    }
}

#[test]
fn extraction_matches_the_python_tool() {
    let Some(game) = game() else { return };
    let ex = convert::extract::Extractor::new(&game);
    for (id, dir, scene) in CHARACTERS {
        let want_path: &Path = &root().join(format!("data/{id}.json"));
        let Ok(want) = std::fs::read_to_string(want_path) else { continue };
        let want: Value = serde_json::from_str(&want).unwrap();
        let got = convert::extract::extract_with(&ex, dir, scene).unwrap();
        if let Some(d) = diff(&got, &want, id) {
            panic!("{d}");
        }
    }
}

/// First differing line of two texts.
fn text_diff(a: &str, b: &str, skip: usize) -> Option<String> {
    let (x, y): (Vec<&str>, Vec<&str>) = (a.lines().skip(skip).collect(), b.lines().skip(skip).collect());
    for i in 0..x.len().max(y.len()) {
        if x.get(i) != y.get(i) {
            return Some(format!("line {}:\n  ours:     {:?}\n  expected: {:?}", i + 1 + skip, x.get(i), y.get(i)));
        }
    }
    None
}

#[test]
fn conversion_matches_the_python_tool() {
    let Some(game) = game() else { return };
    let ex = convert::extract::Extractor::new(&game);
    let mut checked = 0;
    for (id, dir, scene) in CHARACTERS {
        let conv_dir = root().join("data/converted");
        let Ok(want_ron) = std::fs::read_to_string(conv_dir.join(format!("{id}.ron"))) else { continue };
        let data = convert::extract::extract_with(&ex, dir, scene).unwrap();
        let out = convert::convert_character(&game, &data, id);
        // the first lines name the tool that wrote the file
        if let Some(d) = text_diff(&out.ron, &want_ron, 3) {
            panic!("{id}.ron {d}");
        }
        if let Ok(want) = std::fs::read_to_string(conv_dir.join(format!("{id}_fx.ron"))) {
            if let Some(d) = text_diff(&out.fx, &want, 2) {
                panic!("{id}_fx.ron {d}");
            }
        }
        if let Ok(want) = std::fs::read_to_string(conv_dir.join(format!("{id}_report.json"))) {
            let want: Value = serde_json::from_str(&want).unwrap();
            if let Some(d) = diff(&out.report_json, &want, "report") {
                panic!("{id} report: {d}");
            }
        }
        if let Ok(want) = std::fs::read_to_string(conv_dir.join(format!("{id}_report.md"))) {
            if let Some(d) = text_diff(&out.report_md, &want, 3) {
                panic!("{id}_report.md {d}");
            }
        }
        eprintln!("{id}: ron {} bytes, fx {} bytes, md {} bytes — identical to the Python output", out.ron.len(), out.fx.len(), out.report_md.len());
        checked += 1;
    }
    assert!(checked == 0 || checked == 5, "checked {checked} characters");
}

/// The control schemes the Python generator produced (before any hand-tuned override).
#[test]
fn controls_match_the_python_tool() {
    let Some(game) = game() else { return };
    let ex = convert::extract::Extractor::new(&game);
    for (id, dir, scene) in CHARACTERS {
        // the ninja's checked-in scheme is hand-tuned; the generated one is in the library build
        let want_path = root().join(format!("data/converted/{id}_controls.ron"));
        let Ok(want) = std::fs::read_to_string(&want_path) else { continue };
        if id == "ninja" {
            continue;
        }
        let data = convert::extract::extract_with(&ex, dir, scene).unwrap();
        let out = convert::convert_character(&game, &data, id);
        let got = convert::controls::make_controls(&data, &out.ron, id);
        if let Some(d) = text_diff(&got.text, &want, 2) {
            panic!("{id}_controls.ron {d}");
        }
    }
}
