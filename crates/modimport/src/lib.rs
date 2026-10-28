//! Reads Steam Workshop mods of the original game: what is in each archive, what it registers,
//! and which parts of the game's scripting API its scripts lean on. Nothing here runs a script
//! or writes anything; it is the first stage of the importer (see `design/mod-importer.md`).

pub mod api;
pub mod index;
pub mod install;
pub mod report;
pub mod zipread;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

pub use api::{Api, Surface};
use zipread::Zip;

macro_rules! lazy_re {
    ($pat:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        CELL.get_or_init(|| Regex::new($pat).expect("valid pattern"))
    }};
}

/// What a Workshop item is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    /// Adds one or more fighters (registers them with the character loader).
    Character,
    /// Changes menus or the game itself: nothing this project imports.
    Tool,
    /// A customization style file (colors and outlines).
    Style,
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Meta {
    pub name: String,
    pub friendly_name: String,
    pub author: String,
    pub version: String,
    pub requires: Vec<String>,
    pub overwrites: bool,
}

/// A fighter a mod adds: its menu name and the scene that is the fighter.
#[derive(Clone, Debug, Serialize)]
pub struct Registration {
    pub name: String,
    pub scene: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Counts {
    pub files: usize,
    pub bytes: u64,
    pub images: usize,
    pub sounds: usize,
    pub scripts: usize,
    pub scenes: usize,
    /// Textures and samples already imported into the engine's own formats (`.stex`, `.sample`).
    pub imported: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScriptInfo {
    pub path: String,
    /// The class or script it extends (a class name like `CharacterState` or a `res://` path).
    pub extends: String,
    pub lines: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModInfo {
    pub id: String,
    pub title: String,
    pub archive: PathBuf,
    pub kind: Kind,
    pub meta: Meta,
    /// The folder every file of the mod lives under (`res://<root>/...`).
    pub root: String,
    pub characters: Vec<Registration>,
    /// Scripts the mod patches into the game with `installScriptExtension`.
    pub script_extensions: Vec<String>,
    pub counts: Counts,
    pub scripts: Vec<ScriptInfo>,
    /// Scenes under a projectile folder or built on the base projectile.
    pub projectile_scenes: Vec<String>,
    pub api: Api,
}

/// Reads JSON the way the game's mod loader tolerates it: trailing commas allowed.
fn lenient_json(text: &str) -> Option<serde_json::Value> {
    let cleaned = lazy_re!(r",(\s*[}\]])").replace_all(text.trim_start_matches('\u{feff}'), "$1");
    serde_json::from_str(&cleaned).ok()
}

fn meta_from(v: &serde_json::Value) -> Meta {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    Meta {
        name: s("name"),
        friendly_name: s("friendly_name"),
        author: s("author"),
        version: s("version"),
        requires: v.get("requires").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        overwrites: v.get("overwrites").and_then(|x| x.as_bool()).unwrap_or(false),
    }
}

/// Every Workshop item folder under `<steam>/steamapps/workshop/content/<app id>`.
pub fn scan_workshop(dir: &Path) -> Vec<Result<ModInfo, String>> {
    let mut items: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).collect();
    items.sort_by_key(|e| e.file_name());
    items.into_iter().map(|e| inspect_item(&e.path())).collect()
}

/// One Workshop item folder: its `workshop_data.json` and the archive or style file beside it.
pub fn inspect_item(dir: &Path) -> Result<ModInfo, String> {
    let id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = std::fs::read_to_string(dir.join("workshop_data.json"))
        .ok()
        .and_then(|t| lenient_json(&t))
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .unwrap_or_default();
    let mut zip = None;
    let mut style = None;
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        let p = e.path();
        match p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("zip") => zip = Some(p),
            Some("style") => style = Some(p),
            _ => {}
        }
    }
    if let Some(z) = zip {
        return inspect_archive(&z, &id, &title);
    }
    if let Some(s) = style {
        return Ok(ModInfo {
            id,
            title,
            archive: s,
            kind: Kind::Style,
            meta: Meta::default(),
            root: String::new(),
            characters: vec![],
            script_extensions: vec![],
            counts: Counts::default(),
            scripts: vec![],
            projectile_scenes: vec![],
            api: Api::default(),
        });
    }
    Err(format!("{}: no archive or style file", dir.display()))
}

pub fn inspect_archive(path: &Path, id: &str, title: &str) -> Result<ModInfo, String> {
    let zip = Zip::open(path)?;
    let names: Vec<String> = zip.names().filter(|n| !n.ends_with('/')).map(str::to_string).collect();
    // the mod's folder is the one that holds `_metadata` and `ModMain.gd`
    let root = names
        .iter()
        .find_map(|n| n.strip_suffix("/_metadata"))
        .or_else(|| names.iter().find_map(|n| n.strip_suffix("/ModMain.gd")))
        .unwrap_or("")
        .to_string();
    let meta = zip.read_text(&format!("{root}/_metadata")).ok().and_then(|t| lenient_json(&t)).map(|v| meta_from(&v)).unwrap_or_default();

    let mut counts = Counts::default();
    let mut scripts = Vec::new();
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut projectile_scenes = Vec::new();
    for n in &names {
        counts.files += 1;
        counts.bytes += zip.size(n).unwrap_or(0);
        let ext = n.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "png" | "webp" | "jpg" | "gif" => counts.images += 1,
            "wav" | "ogg" | "mp3" => counts.sounds += 1,
            "stex" | "sample" | "mp3str" | "oggstr" => counts.imported += 1,
            "tscn" => {
                counts.scenes += 1;
                let lower = n.to_ascii_lowercase();
                if lower.contains("/projectile") {
                    projectile_scenes.push(n.clone());
                }
            }
            "gd" => {
                counts.scripts += 1;
                let text = zip.read_text(n)?;
                let extends = lazy_re!(r#"(?m)^extends\s+(.+?)\s*$"#).captures(&text).map(|c| c[1].trim().to_string()).unwrap_or_default();
                scripts.push(ScriptInfo { path: n.clone(), extends, lines: text.lines().count() });
                texts.push((n.clone(), text));
            }
            _ => {}
        }
    }
    // scenes built on the base projectile that are not in a projectile folder
    for n in &names {
        if n.ends_with(".tscn") && !projectile_scenes.contains(n) {
            if let Ok(t) = zip.read_text(n) {
                if t.contains("projectile/BaseProjectile.tscn") {
                    projectile_scenes.push(n.clone());
                }
            }
        }
    }
    for s in &scripts {
        if s.extends == "BaseProjectile" && !projectile_scenes.iter().any(|p| p.rsplit_once('.').map(|x| x.0) == s.path.rsplit_once('.').map(|x| x.0)) {
            // a projectile script: its scene sits next to it or is named after it
        }
    }

    let mut characters = Vec::new();
    let mut script_extensions = Vec::new();
    for (_, t) in &texts {
        for c in lazy_re!(r#"addCustomChar\(\s*"([^"]+)"\s*,\s*"([^"]+)""#).captures_iter(t) {
            characters.push(Registration { name: c[1].to_string(), scene: c[2].trim_start_matches("res://").to_string() });
        }
        for c in lazy_re!(r#"installScriptExtension\(\s*"([^"]+)""#).captures_iter(t) {
            script_extensions.push(c[1].trim_start_matches("res://").to_string());
        }
    }
    let kind = if !characters.is_empty() {
        Kind::Character
    } else if !script_extensions.is_empty() || !texts.is_empty() {
        Kind::Tool
    } else {
        Kind::Unknown
    };
    let api = Api::from_scripts(texts.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    projectile_scenes.sort();
    Ok(ModInfo { id: id.to_string(), title: title.to_string(), archive: path.to_path_buf(), kind, meta, root, characters, script_extensions, counts, scripts, projectile_scenes, api })
}

/// Scripts grouped by what they extend, most common first.
pub fn extends_histogram(m: &ModInfo) -> Vec<(String, usize)> {
    let mut h: BTreeMap<String, usize> = BTreeMap::new();
    for s in &m.scripts {
        *h.entry(if s.extends.is_empty() { "(nothing)".into() } else { s.extends.clone() }).or_default() += 1;
    }
    let mut v: Vec<_> = h.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_commas_are_tolerated() {
        let v = lenient_json("{\n \"name\": \"A\",\n \"requires\": [\"x\",],\n \"priority\": 0,\n}").expect("parses");
        let m = meta_from(&v);
        assert_eq!(m.name, "A");
        assert_eq!(m.requires, vec!["x"]);
    }
}
