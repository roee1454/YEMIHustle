//! The game library: everything built from the player's own copy of the original game, kept in
//! their user-data folder (never in the project, never shared). Built once by the setup wizard
//! (see `build.rs`), then read by the menus, effects and characters without the original.
//!
//! Where it lives: Linux `~/.local/share/yemi-hustle/` (or `$XDG_DATA_HOME`), macOS
//! `~/Library/Application Support/yemi-hustle/`, Windows `%LOCALAPPDATA%\yemi-hustle\`.
//! `YEMI_LIBRARY` or `--library` overrides it.
//!
//! ```text
//! yemi-hustle/
//!   build.json        what was built, from where, and when
//!   game/             art, sounds, fonts and scenes in the original's layout (+ scripts)
//!   characters/       converted characters, controls and effects (RON)
//!   tools/            helper programs the wizard downloaded with consent
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when the library layout changes in a way that needs a rebuild.
pub const LAYOUT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Manifest {
    pub layout: u32,
    /// The folder the library was built from.
    pub source: String,
    /// Size of the source `.pck`, to notice a game update.
    pub source_size: u64,
    pub assets: bool,
    pub scripts: bool,
    pub characters: Vec<String>,
    /// Seconds since the Unix epoch.
    pub built_at: u64,
    pub files: usize,
}

#[derive(Clone, Debug)]
pub struct Library {
    pub root: PathBuf,
}

impl Library {
    pub fn new(root: PathBuf) -> Self {
        Library { root }
    }

    /// The default place: the platform's per-user data folder.
    pub fn default_root() -> PathBuf {
        // Windows keeps large per-user data in %LOCALAPPDATA% (not the roaming %APPDATA%)
        let windows = cfg!(windows).then(|| std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("yemi-hustle"))).flatten();
        let base = std::env::var_os("YEMI_LIBRARY")
            .map(PathBuf::from)
            .or(windows)
            .or_else(|| std::env::var_os("XDG_DATA_HOME").map(|d| PathBuf::from(d).join("yemi-hustle")))
            .or_else(|| {
                std::env::var_os("HOME").map(|h| {
                    let h = PathBuf::from(h);
                    if cfg!(target_os = "macos") { h.join("Library/Application Support/yemi-hustle") } else { h.join(".local/share/yemi-hustle") }
                })
            })
            .or_else(|| std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("yemi-hustle")));
        base.unwrap_or_else(|| PathBuf::from("yemi-hustle-library"))
    }

    pub fn game_dir(&self) -> PathBuf {
        self.root.join("game")
    }

    pub fn characters_dir(&self) -> PathBuf {
        self.root.join("characters")
    }

    /// Fighters imported from Workshop mods.
    pub fn mod_index(&self) -> modimport::index::Index {
        modimport::index::Index::read(&self.root)
    }

    pub fn tools_dir(&self) -> PathBuf {
        self.root.join("tools")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("build.json")
    }

    pub fn manifest(&self) -> Option<Manifest> {
        let t = std::fs::read_to_string(self.manifest_path()).ok()?;
        ron::from_str::<Manifest>(&t).ok().filter(|m| m.layout == LAYOUT)
    }

    pub fn save_manifest(&self, m: &Manifest) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let text = ron::ser::to_string_pretty(m, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
        std::fs::write(self.manifest_path(), text).map_err(|e| e.to_string())
    }

    /// The menus' art and sounds are present.
    pub fn has_assets(&self) -> bool {
        self.manifest().is_some_and(|m| m.assets) && self.game_dir().join("project.godot").exists()
    }

    /// At least one playable character was converted.
    pub fn has_characters(&self) -> bool {
        self.manifest().is_some_and(|m| !m.characters.is_empty())
    }

    /// Removes the built library (keeps downloaded tools).
    pub fn clear(&self) -> Result<(), String> {
        for d in [self.game_dir(), self.characters_dir()] {
            if d.exists() {
                std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
        }
        let _ = std::fs::remove_file(self.manifest_path());
        Ok(())
    }
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Hand-tuned control schemes shipped with the game (`content/controls/`), next to the
/// program or in the source tree. They replace the generated drafts.
pub fn controls_source() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("YEMI_CONTENT") {
        c.push(PathBuf::from(d).join("controls"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            c.push(dir.join("content/controls"));
        }
    }
    c.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/controls"));
    c.into_iter().find(|d| d.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_and_old_layouts_are_ignored() {
        let dir = std::env::temp_dir().join(format!("yemi-lib-test-{}", std::process::id()));
        let lib = Library::new(dir.clone());
        assert!(lib.manifest().is_none() && !lib.has_assets());
        let m = Manifest { layout: LAYOUT, source: "x".into(), assets: true, characters: vec!["ninja".into()], ..Default::default() };
        lib.save_manifest(&m).unwrap();
        assert_eq!(lib.manifest(), Some(m.clone()));
        assert!(lib.has_characters());
        lib.save_manifest(&Manifest { layout: LAYOUT + 1, ..m }).unwrap();
        assert!(lib.manifest().is_none(), "a different layout means rebuild");
        let _ = std::fs::remove_dir_all(dir);
    }
}
