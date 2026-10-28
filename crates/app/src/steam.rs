//! Finding the player's Steam copy of the original game and its Workshop folder.
//! Reads Steam's own library list (`libraryfolders.vdf`) and the game's app manifest; no Steam
//! API, no network.

use std::path::{Path, PathBuf};

/// The original's Steam app id.
pub const APP_ID: u32 = 2212330;

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub game_dir: PathBuf,
    /// `steamapps/workshop/content/<app id>` in the same library, when it exists.
    pub workshop_dir: Option<PathBuf>,
}

/// Steam's usual install locations on this platform.
fn steam_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut v = Vec::new();
    if let Some(h) = &home {
        v.push(h.join(".local/share/Steam"));
        v.push(h.join(".steam/steam"));
        v.push(h.join(".steam/root"));
        v.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        v.push(h.join("Library/Application Support/Steam"));
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(p) = std::env::var_os(var) {
            v.push(PathBuf::from(p).join("Steam"));
        }
    }
    v.push(PathBuf::from("C:/Program Files (x86)/Steam"));
    v
}

/// Library folders listed in a `libraryfolders.vdf` (`"path"  "..."` lines).
pub fn parse_library_folders(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|l| {
            let mut parts = l.split('"').filter(|s| !s.trim().is_empty());
            if parts.next()? != "path" {
                return None;
            }
            Some(PathBuf::from(parts.next()?.replace("\\\\", "\\")))
        })
        .collect()
}

/// `"installdir"  "..."` from an app manifest.
pub fn parse_install_dir(acf: &str) -> Option<String> {
    acf.lines().find_map(|l| {
        let mut parts = l.split('"').filter(|s| !s.trim().is_empty());
        if parts.next()? != "installdir" {
            return None;
        }
        parts.next().map(str::to_string)
    })
}

/// Every Steam library on this machine.
pub fn libraries() -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    for root in steam_roots() {
        let vdf = root.join("steamapps/libraryfolders.vdf");
        let Ok(text) = std::fs::read_to_string(&vdf) else { continue };
        let mut found = parse_library_folders(&text);
        found.push(root.clone());
        for l in found {
            let canon = l.canonicalize().unwrap_or(l);
            if canon.join("steamapps").is_dir() && !libs.contains(&canon) {
                libs.push(canon);
            }
        }
    }
    libs
}

/// Installs of the original in every library.
pub fn find_all() -> Vec<Found> {
    let mut out = Vec::new();
    for lib in libraries() {
        let apps = lib.join("steamapps");
        let dir = std::fs::read_to_string(apps.join(format!("appmanifest_{APP_ID}.acf")))
            .ok()
            .and_then(|a| parse_install_dir(&a))
            .map(|d| apps.join("common").join(d))
            .or_else(|| Some(apps.join("common/YourOnlyMoveIsHUSTLE")))
            .filter(|d| d.is_dir());
        if let Some(game_dir) = dir {
            out.push(Found { workshop_dir: workshop_in(&lib), game_dir });
        }
    }
    out
}

pub fn find_game() -> Option<Found> {
    find_all().into_iter().next()
}

fn workshop_in(lib: &Path) -> Option<PathBuf> {
    let w = lib.join(format!("steamapps/workshop/content/{APP_ID}"));
    w.is_dir().then_some(w)
}

/// The Workshop folder for a chosen game folder (`.../steamapps/common/<game>`), if any.
pub fn workshop_for(game_dir: &Path) -> Option<PathBuf> {
    let lib = game_dir.ancestors().find(|a| a.file_name().is_some_and(|n| n == "steamapps"))?.parent()?;
    workshop_in(lib)
}

/// Workshop items: (id folder, title from workshop_data.json when present).
pub fn workshop_items(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut v: Vec<(PathBuf, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let title = std::fs::read_to_string(e.path().join("workshop_data.json"))
                .ok()
                .and_then(|t| {
                    let i = t.find("\"title\"")?;
                    let rest = &t[i + 7..];
                    let s = rest.find('"')? + 1;
                    let end = rest[s..].find('"')?;
                    Some(rest[s..s + end].to_string())
                })
                .unwrap_or_else(|| e.file_name().to_string_lossy().into_owned());
            (e.path(), title)
        })
        .collect();
    v.sort();
    v
}

/// Asks the desktop for a folder (zenity, kdialog, macOS or Windows dialogs). None if no
/// dialog is available or the player cancelled.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    let attempts: Vec<(&str, Vec<String>)> = if cfg!(target_os = "macos") {
        vec![("osascript", vec!["-e".into(), format!("POSIX path of (choose folder with prompt \"{title}\")")])]
    } else if cfg!(windows) {
        vec![(
            "powershell",
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; if ($d.ShowDialog() -eq 'OK') { $d.SelectedPath }".into(),
            ],
        )]
    } else {
        vec![
            ("zenity", vec!["--file-selection".into(), "--directory".into(), format!("--title={title}")]),
            ("kdialog", vec!["--getexistingdirectory".into(), ".".into(), "--title".into(), title.into()]),
        ]
    };
    for (cmd, args) in attempts {
        let Ok(out) = std::process::Command::new(cmd).args(&args).output() else { continue };
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return (out.status.success() && !s.is_empty()).then(|| PathBuf::from(s));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steam_files() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/a/.local/share/Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Games\\\\Steam\"\n\t}\n}";
        assert_eq!(parse_library_folders(vdf), vec![PathBuf::from("/home/a/.local/share/Steam"), PathBuf::from("D:\\Games\\Steam")]);
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"2212330\"\n\t\"installdir\"\t\t\"YourOnlyMoveIsHUSTLE\"\n}";
        assert_eq!(parse_install_dir(acf).as_deref(), Some("YourOnlyMoveIsHUSTLE"));
        assert_eq!(
            workshop_for(Path::new("/nonexistent/steamapps/common/YourOnlyMoveIsHUSTLE")),
            None,
            "only existing folders count"
        );
    }
}
