//! The whole conversion for the five characters: extract, convert, write the files. Used by the
//! game's setup wizard and by the `yemi-convert` command.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::{controls, convert_character_with, extract};

/// The characters: (id, folder in the game, scene file).
pub const CHARACTERS: [(&str, &str, &str); 5] = [
    ("ninja", "characters/stickman", "NinjaGuy.tscn"),
    ("robo", "characters/robo", "Robot.tscn"),
    ("wizard", "characters/wizard", "Wizard.tscn"),
    ("mutant", "characters/mutant", "Mutant.tscn"),
    ("cowboy", "characters/swordandgun", "SwordGuy.tscn"),
];

#[derive(Debug, Clone)]
pub struct Done {
    pub id: String,
    pub moves: usize,
    pub projectiles: usize,
}

/// Converts the given characters (all five when `only` is empty) from a decompiled game folder
/// into `out`, in parallel. `controls_dir` holds hand-tuned control schemes (`<id>_controls.ron`)
/// that replace the generated drafts. Returns each character's result in roster order.
pub fn convert_all(game: &Path, out: &Path, controls_dir: Option<&Path>, only: &[String]) -> Vec<Result<Done, String>> {
    if let Err(e) = std::fs::create_dir_all(out) {
        return vec![Err(format!("{}: {e}", out.display()))];
    }
    CHARACTERS
        .par_iter()
        .filter(|(id, _, _)| only.is_empty() || only.iter().any(|o| o == id))
        .map(|(id, dir, scene)| convert_one(game, out, controls_dir, id, dir, scene, false))
        .collect()
}

/// Converts one fighter that is not part of the roster (a mod's): `dir` is the folder that holds
/// its scene file under `game`, which must hold everything the scene refers to.
pub fn convert_fighter(game: &Path, out: &Path, id: &str, dir: &str, scene: &str) -> Result<Done, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    convert_one(game, out, None, id, dir, scene, true)
}

fn convert_one(game: &Path, out: &Path, controls_dir: Option<&Path>, id: &str, dir: &str, scene: &str, scripted: bool) -> Result<Done, String> {
    let ex = extract::Extractor::new(game);
    let mut data = extract::extract_with(&ex, dir, scene).map_err(|e| format!("{id}: {e}"))?;
    if scripted {
        add_objects(&ex, &mut data, dir, scene);
    }
    let conv = convert_character_with(game, &data, id, scripted);
    let draft = controls::make_controls(&data, &conv.ron, id);
    let own: Option<PathBuf> = controls_dir.map(|d| d.join(format!("{id}_controls.ron"))).filter(|p| p.is_file());
    let controls_text = match &own {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
        None => draft.text,
    };
    let write = |name: String, text: String| std::fs::write(out.join(&name), text).map_err(|e| format!("{name}: {e}"));
    write(format!("{id}.ron"), conv.ron)?;
    write(format!("{id}_fx.ron"), conv.fx)?;
    write(format!("{id}_controls.ron"), controls_text)?;
    write(format!("{id}_report.md"), conv.report_md)?;
    write(format!("{id}_report.json"), serde_json::to_string_pretty(&conv.report_json).unwrap_or_default())?;
    Ok(Done { id: id.to_string(), moves: conv.moves, projectiles: conv.projectiles })
}

/// For fighters whose scripts run (imported mods): every object scene their scripts or scenes
/// refer to (not only the `projectiles` folder), and the nodes of the fighter's scene and of
/// those objects' scenes.
fn add_objects(ex: &extract::Extractor, data: &mut serde_json::Value, dir: &str, scene: &str) {
    let top = dir.split('/').next().unwrap_or(dir);
    let mut cache = std::collections::HashMap::new();
    let mut objects: Vec<serde_json::Value> = data["projectiles"].as_array().cloned().unwrap_or_default();
    for rel in ex.referenced_scenes(top) {
        if objects.iter().any(|o| o["scene"].as_str() == Some(rel.as_str())) {
            continue;
        }
        let o = ex.extract_object(&rel, &mut cache);
        if o["states"].as_array().is_some_and(|s| !s.is_empty()) {
            objects.push(o);
        }
    }
    let mut defs: Vec<serde_json::Value> = Vec::new();
    defs.extend(ex.scene_def(&format!("{dir}/{scene}")));
    for o in &objects {
        if o["states"].as_array().is_some_and(|s| !s.is_empty()) {
            defs.extend(o["scene"].as_str().and_then(|r| ex.scene_def(r)));
        }
    }
    data["projectiles"] = serde_json::Value::Array(objects);
    data["scene_defs"] = serde_json::Value::Array(defs);
}
