//! Puts a mod's files into the library and converts its fighters.
//!
//! Layout under the library root:
//! `mods/<id>/res/<ModFolder>/...` holds the mod's files as `res://` paths;
//! `mods/<id>/view/` is a text-only overlay (the original's scripts and scenes plus the mod's)
//! that the converter reads; converted fighters go to `characters/`.

use std::path::{Component, Path, PathBuf};

use crate::{inspect_archive, zipread::Zip, Kind, ModInfo};

const MAX_ENTRIES: usize = 50_000;
const MAX_BYTES: u64 = 1 << 30;

/// A fighter that was converted.
#[derive(Clone, Debug)]
pub struct Installed {
    /// The id used for its files and in the roster: `one_punch_man`.
    pub id: String,
    pub name: String,
    /// The scene as a `res://` path without the prefix.
    pub scene: String,
    pub moves: usize,
    pub projectiles: usize,
}

/// `One Punch Man` -> `one_punch_man` (lowercase letters, digits, underscores).
pub fn fighter_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.ends_with('_') {
            id.push('_');
        }
    }
    id.trim_matches('_').to_string()
}

/// A path inside an archive made safe: relative, no `..`, no drive letters.
fn safe(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    p.components().all(|c| matches!(c, Component::Normal(_))).then(|| p.to_path_buf())
}

fn is_text(name: &str) -> bool {
    matches!(name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref(), Some("gd" | "tscn" | "tres" | "cfg" | "godot"))
}

/// Unpacks the mod's files into `<library>/mods/<id>/res/`. Returns the relative paths written.
pub fn extract(m: &ModInfo, library: &Path) -> Result<Vec<String>, String> {
    let zip = Zip::open(&m.archive)?;
    let res = library.join("mods").join(&m.id).join("res");
    let mut written = Vec::new();
    let mut total = 0u64;
    for name in zip.names().map(str::to_string).collect::<Vec<_>>() {
        if name.ends_with('/') || name.contains("/.import/") || name.starts_with(".import/") || name.ends_with(".import") || name.ends_with(".md5") {
            continue;
        }
        let rel = safe(&name).ok_or_else(|| format!("{name}: unsafe path in the archive"))?;
        total += zip.size(&name).unwrap_or(0);
        if written.len() >= MAX_ENTRIES || total > MAX_BYTES {
            return Err("the archive is larger than the importer accepts".into());
        }
        let dest = res.join(&rel);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&dest, zip.read(&name)?).map_err(|e| format!("{}: {e}", dest.display()))?;
        written.push(name);
    }
    Ok(written)
}

fn copy_text(from: &Path, to: &Path) -> Result<(), String> {
    let mut stack = vec![from.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if is_text(&p.to_string_lossy()) {
                let rel = p.strip_prefix(from).map_err(|e| e.to_string())?;
                let dest = to.join(rel);
                if let Some(dir) = dest.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                std::fs::copy(&p, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            }
        }
    }
    Ok(())
}

/// The text-only overlay the converter reads: the original's scripts and scenes with the mod's
/// on top.
pub fn build_view(m: &ModInfo, library: &Path) -> Result<PathBuf, String> {
    let view = library.join("mods").join(&m.id).join("view");
    if view.exists() {
        std::fs::remove_dir_all(&view).map_err(|e| format!("{}: {e}", view.display()))?;
    }
    copy_text(&library.join("game"), &view)?;
    copy_text(&library.join("mods").join(&m.id).join("res"), &view)?;
    Ok(view)
}

/// Extracts the mod and converts every fighter it registers into `<library>/characters/`.
pub fn install(m: &ModInfo, library: &Path) -> Result<Vec<Result<Installed, String>>, String> {
    if m.kind != Kind::Character {
        return Err(format!("{} does not add fighters", m.title));
    }
    if m.meta.overwrites {
        return Err("this mod replaces files of the original game, which the importer does not support".into());
    }
    extract(m, library)?;
    let view = build_view(m, library)?;
    let out = library.join("characters");
    let mut index = crate::index::Index::read(library);
    let results: Vec<Result<Installed, String>> = m.characters
        .iter()
        .map(|c| {
            let id = fighter_id(&c.name);
            let (dir, file) = c.scene.rsplit_once('/').ok_or_else(|| format!("{}: scene path has no folder", c.name))?;
            let done = convert::pipeline::convert_fighter(&view, &out, &id, dir, file)?;
            set_display_name(&out.join(format!("{id}.ron")), &c.name)?;
            // the automatic button layout replaces the converter's draft controls
            sim::layout::write_for(&out, &id, false)?;
            Ok(Installed { id, name: c.name.clone(), scene: c.scene.clone(), moves: done.moves, projectiles: done.projectiles })
        })
        .collect();
    for r in results.iter().flatten() {
        index.put(crate::index::Fighter { id: r.id.clone(), name: r.name.clone(), scene: r.scene.clone(), mod_id: m.id.clone(), author: m.meta.author.clone() });
    }
    index.write(library)?;
    Ok(results)
}

/// The converter names a fighter after its id; the file should carry the name the mod gave it.
fn set_display_name(file: &Path, name: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = String::with_capacity(text.len());
    let mut done = false;
    for line in text.lines() {
        // the character's own `name:` is the first one at this indentation
        if !done && line.starts_with("        name: \"") {
            out.push_str(&format!("        name: \"{escaped}\",\n"));
            done = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    std::fs::write(file, out).map_err(|e| format!("{}: {e}", file.display()))
}

/// Reads a mod archive and installs it (the importer's one-call entry).
pub fn install_archive(archive: &Path, id: &str, title: &str, library: &Path) -> Result<Vec<Result<Installed, String>>, String> {
    let m = inspect_archive(archive, id, title)?;
    install(&m, library)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_clean() {
        assert_eq!(fighter_id("One Punch Man"), "one_punch_man");
        assert_eq!(fighter_id("  Gojo -- Satoru!"), "gojo_satoru");
    }

    #[test]
    fn archive_paths_cannot_escape() {
        assert!(safe("Mod/a.gd").is_some());
        assert!(safe("../x").is_none());
        assert!(safe("/etc/passwd").is_none());
        assert!(safe("Mod/../../x").is_none());
    }
}
