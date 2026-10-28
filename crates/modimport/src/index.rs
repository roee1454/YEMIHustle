//! `<library>/mods/index.json`: the fighters imported from mods, which the game reads at start.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Fighter {
    /// The id of its converted files (`characters/<id>.ron`).
    pub id: String,
    pub name: String,
    /// Its scene as a `res://` path without the prefix, found under the mod's `res` folder.
    pub scene: String,
    /// The Workshop id of the mod it came from.
    pub mod_id: String,
    pub author: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub fighters: Vec<Fighter>,
}

pub fn path(library: &Path) -> PathBuf {
    library.join("mods").join("index.json")
}

impl Index {
    pub fn read(library: &Path) -> Index {
        std::fs::read_to_string(path(library)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn write(&self, library: &Path) -> Result<(), String> {
        let p = path(library);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&p, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// Adds or replaces a fighter (by id).
    pub fn put(&mut self, f: Fighter) {
        match self.fighters.iter_mut().find(|x| x.id == f.id) {
            Some(slot) => *slot = f,
            None => self.fighters.push(f),
        }
    }

    /// The folders that hold imported mods' files (`res://` roots), for looking assets up.
    pub fn res_dirs(&self, library: &Path) -> Vec<PathBuf> {
        let mut seen = Vec::new();
        for f in &self.fighters {
            let d = library.join("mods").join(&f.mod_id).join("res");
            if !seen.contains(&d) {
                seen.push(d);
            }
        }
        seen
    }
}
