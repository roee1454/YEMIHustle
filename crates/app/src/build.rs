//! The one-time build: reads the player's own copy of the original game and fills the library
//! (see `library.rs`). Runs on a background thread and reports progress over a channel.
//!
//! Stages:
//! 1. **Read** the game's pack and check it is the right game.
//! 2. **Extract** art, sounds, fonts and scenes in the original's layout (built in).
//! 3. **Scripts**: decompile the compiled scripts with GDRE (an open-source tool; the wizard
//!    downloads it with the player's consent).
//! 4. **Characters**: convert the five characters with the built-in converter (`convert` crate).
//!
//! Stage 3 may be skipped (GDRE missing), which also skips stage 4; the menus and effects need only stage 2.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;

use crate::gamefiles::{Audio, GameFiles, Source};
use crate::library::{controls_source, unix_now, Library, Manifest, LAYOUT};

pub const STAGES: [&str; 4] = ["Read the game", "Extract art and sounds", "Decompile scripts", "Convert characters"];

#[derive(Clone, Debug, PartialEq)]
pub enum StageState {
    Waiting,
    Running,
    Done(String),
    Skipped(String),
    Failed(String),
}

#[derive(Debug)]
pub enum Msg {
    Stage(usize, StageState),
    /// 0..1 within the running stage.
    Progress(f32),
    Log(String),
    /// Finished; `Err` if the assets stage failed.
    Finished(Result<Manifest, String>),
}

pub struct Request {
    pub source: PathBuf,
    pub library: Library,
    /// A GDRE executable to use; None means look for one, or download it if `download` is set.
    pub gdre: Option<PathBuf>,
    /// The player agreed to download GDRE when it is missing.
    pub download: bool,
}

pub use convert::pipeline::CHARACTERS;

/// Checks a chosen folder before building: the game's pack, its size and engine version.
#[derive(Debug, Clone)]
pub struct Check {
    pub pack: String,
    pub size: u64,
    pub files: usize,
    pub workshop_items: usize,
}

pub fn check(path: &Path) -> Result<Check, String> {
    let g = GameFiles::open(path)?;
    let Source::Pck { file, files, version } = &g.source else {
        return Ok(Check { pack: g.describe(), size: 0, files: g.list("").len(), workshop_items: 0 });
    };
    if version.0 != 3 {
        return Err(format!("this is a Godot {} game; the original was made with Godot 3", version.0));
    }
    for must in ["ui/title2.png.import", "characters/BaseChar.tscn", "fx/ParticleEffect.tscn"] {
        if !files.contains_key(must) {
            return Err(format!("{} does not look like Your Only Move Is HUSTLE (missing {must})", file.display()));
        }
    }
    let workshop = crate::steam::workshop_for(path).map(|w| crate::steam::workshop_items(&w).len()).unwrap_or(0);
    Ok(Check { pack: g.describe(), size: std::fs::metadata(file).map(|m| m.len()).unwrap_or(0), files: files.len(), workshop_items: workshop })
}

pub fn run(req: Request, tx: Sender<Msg>) {
    let result = build(&req, &tx);
    let _ = tx.send(Msg::Finished(result));
}

fn stage(tx: &Sender<Msg>, i: usize, s: StageState) {
    let _ = tx.send(Msg::Stage(i, s));
}

fn log(tx: &Sender<Msg>, s: impl Into<String>) {
    let _ = tx.send(Msg::Log(s.into()));
}

fn build(req: &Request, tx: &Sender<Msg>) -> Result<Manifest, String> {
    let lib = &req.library;
    // 1: read
    stage(tx, 0, StageState::Running);
    let g = GameFiles::open(&req.source).inspect_err(|e| stage(tx, 0, StageState::Failed(e.clone())))?;
    let c = check(&req.source).inspect_err(|e| stage(tx, 0, StageState::Failed(e.clone())))?;
    stage(tx, 0, StageState::Done(format!("{} files", c.files)));
    log(tx, format!("source: {}", g.describe()));

    // 2: extract (starts from a clean library so a rebuild never mixes old and new files)
    stage(tx, 1, StageState::Running);
    lib.clear().inspect_err(|e| stage(tx, 1, StageState::Failed(e.clone())))?;
    let n = extract(&g, &lib.game_dir(), tx).inspect_err(|e| stage(tx, 1, StageState::Failed(e.clone())))?;
    stage(tx, 1, StageState::Done(format!("{n} files")));

    let mut m = Manifest {
        layout: LAYOUT,
        source: req.source.to_string_lossy().into_owned(),
        source_size: c.size,
        assets: true,
        scripts: false,
        characters: vec![],
        built_at: unix_now(),
        files: n,
    };
    lib.save_manifest(&m)?;

    // 3: scripts
    stage(tx, 2, StageState::Running);
    let mut gdre = req.gdre.clone().or_else(|| find_gdre(lib));
    if gdre.is_none() && req.download {
        match download_gdre(lib, tx) {
            Ok(p) => gdre = Some(p),
            Err(e) => log(tx, format!("could not get GDRE: {e}")),
        }
    }
    match &gdre {
        None => stage(tx, 2, StageState::Skipped("the script tool (GDRE) is not installed".into())),
        Some(gdre) => {
            match decompile(gdre, &g, lib, tx) {
                Ok(n) => {
                    m.scripts = true;
                    stage(tx, 2, StageState::Done(format!("{n} scripts")));
                }
                Err(e) => stage(tx, 2, StageState::Failed(e)),
            }
        }
    }
    lib.save_manifest(&m)?;

    // 4: characters
    if !m.scripts {
        stage(tx, 3, StageState::Skipped("needs the decompiled scripts".into()));
    } else {
        stage(tx, 3, StageState::Running);
        match convert_characters(lib, tx) {
            Ok(done) => {
                let note = format!("{} of {}", done.len(), CHARACTERS.len());
                m.characters = done;
                stage(tx, 3, if m.characters.is_empty() { StageState::Failed("no character converted".into()) } else { StageState::Done(note) });
            }
            Err(e) => stage(tx, 3, StageState::Skipped(e)),
        }
    }
    lib.save_manifest(&m)?;
    Ok(m)
}

// ---------------------------------------------------------------------------------------------
// Stage 2: extraction
// ---------------------------------------------------------------------------------------------

/// Plain files copied as they are.
const COPY: [&str; 5] = [".tscn", ".tres", ".gdshader", ".ttf", ".md"];

fn extract(g: &GameFiles, out: &Path, tx: &Sender<Msg>) -> Result<usize, String> {
    let all = g.list("");
    let total = all.len().max(1);
    let mut written = 0;
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let write = |rel: &str, bytes: &[u8]| -> Result<(), String> {
        let p = out.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))
    };
    for (i, name) in all.iter().enumerate() {
        if i % 100 == 0 {
            let _ = tx.send(Msg::Progress(i as f32 / total as f32));
        }
        if name.starts_with(".import/") || name.starts_with(".godot/") {
            continue;
        }
        if COPY.iter().any(|e| name.ends_with(e)) {
            if let Some(b) = g.read(name) {
                write(name, &b)?;
                written += 1;
            }
        } else if let Some(src) = name.strip_suffix(".import") {
            let lower = src.to_ascii_lowercase();
            if [".png", ".webp", ".jpg", ".jpeg"].iter().any(|e| lower.ends_with(e)) {
                if let Some(img) = g.image(src) {
                    write(src, &img.bytes)?;
                    written += 1;
                }
            } else if lower.ends_with(".wav") {
                match g.audio(src) {
                    Some(a) => {
                        write(src, &encode_wav(&a))?;
                        written += 1;
                    }
                    None => log(tx, format!("could not decode sound {src}")),
                }
            } else if lower.ends_with(".mp3") {
                if let Some(b) = g.mp3(src) {
                    write(src, &b)?;
                    written += 1;
                }
            }
        }
    }
    // marks the folder as a finished game library (the menus look for it)
    write("project.godot", b"; built by YEMI Hustle from the player's own copy of the original game\n")?;
    let _ = tx.send(Msg::Progress(1.0));
    if written < 1000 {
        return Err(format!("only {written} files could be read; is this the full game folder?"));
    }
    Ok(written)
}

/// 16-bit PCM WAV.
pub fn encode_wav(a: &Audio) -> Vec<u8> {
    let data: Vec<u8> = a.samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()).collect();
    let mut b = Vec::with_capacity(44 + data.len());
    b.extend(b"RIFF");
    b.extend(&(36 + data.len() as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(&16u32.to_le_bytes());
    b.extend(&1u16.to_le_bytes());
    b.extend(&a.channels.to_le_bytes());
    b.extend(&a.rate.to_le_bytes());
    b.extend(&(a.rate * a.channels as u32 * 2).to_le_bytes());
    b.extend(&(a.channels * 2).to_le_bytes());
    b.extend(&16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(&(data.len() as u32).to_le_bytes());
    b.extend(data);
    b
}

// ---------------------------------------------------------------------------------------------
// Stage 3: scripts, with GDRE
// ---------------------------------------------------------------------------------------------

/// A GDRE program: from `YEMI_GDRE`, the library's tools folder, or the system path.
pub fn find_gdre(lib: &Library) -> Option<PathBuf> {
    let names = ["gdre_tools", "gdre_tools.x86_64", "gdre_tools.exe", "Godot RE Tools.app/Contents/MacOS/Godot RE Tools"];
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("YEMI_GDRE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
        dirs.push(p);
    }
    dirs.push(lib.tools_dir().join("gdre"));
    dirs.push(lib.tools_dir());
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter().find_map(|d| names.iter().map(|n| d.join(n)).find(|p| p.is_file()))
}

/// GDRE's release for this platform (the project pins the checksum it verified).
pub fn gdre_download() -> Option<(&'static str, &'static str, &'static str)> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some((
            "https://github.com/GDRETools/gdsdecomp/releases/download/v2.7.4/GDRE_tools-v2.7.4-linux.zip",
            "3dc4f1244dda62e41e0c952ac00a69e60e07f3b0d5a211164806bf9d41a54758",
            "GDRE_tools-v2.7.4-linux.zip",
        ))
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some((
            "https://github.com/GDRETools/gdsdecomp/releases/download/v2.7.4/GDRE_tools-v2.7.4-windows.zip",
            "af24c563eee501effb194d83595de64696c32547b2246034dbfb891a24bfa0ad",
            "GDRE_tools-v2.7.4-windows.zip",
        ))
    } else if cfg!(target_os = "macos") {
        Some((
            "https://github.com/GDRETools/gdsdecomp/releases/download/v2.7.4/GDRE_tools-v2.7.4-macos.zip",
            "a148b9509c2826fcc238e81c21005bf3128bdcee1a87c3f79ad943a205aa2170",
            "GDRE_tools-v2.7.4-macos.zip",
        ))
    } else {
        None
    }
}

/// Downloads GDRE into the library's tools folder and checks its checksum. Called only after
/// the player agreed in the wizard.
pub fn download_gdre(lib: &Library, tx: &Sender<Msg>) -> Result<PathBuf, String> {
    let (url, sha, file) = gdre_download().ok_or("there is no download for this system; install GDRE yourself and set YEMI_GDRE")?;
    let dir = lib.tools_dir().join("gdre");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join(file);
    log(tx, format!("downloading {url}"));
    run_cmd(Command::new("curl").args(["-fsSL", "--retry", "2", "-o"]).arg(&zip).arg(url), "curl")?;
    let digest = sha256(&zip)?;
    if digest != sha {
        let _ = std::fs::remove_file(&zip);
        return Err(format!("the download does not match the expected checksum ({digest})"));
    }
    log(tx, "checksum ok, unpacking");
    let unpacked = if cfg!(windows) {
        run_cmd(
            Command::new("powershell").args(["-NoProfile", "-Command"]).arg(format!(
                "Expand-Archive -Force -LiteralPath '{}' -DestinationPath '{}'",
                zip.display(),
                dir.display()
            )),
            "powershell Expand-Archive",
        )
    } else {
        run_cmd(Command::new("unzip").args(["-q", "-o"]).arg(&zip).arg("-d").arg(&dir), "unzip")
    };
    if unpacked.is_err() {
        // Python's own zip reader works everywhere Python does
        let py = python().ok_or("could not unpack the download (no unzip tool and no Python)")?;
        run_cmd(Command::new(py).args(["-m", "zipfile", "-e"]).arg(&zip).arg(&dir), "python -m zipfile")?;
    }
    let _ = std::fs::remove_file(&zip);
    let exe = find_gdre(lib).ok_or("the download did not contain the GDRE program")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(exe)
}

fn sha256(p: &Path) -> Result<String, String> {
    for (cmd, args) in [("sha256sum", vec![]), ("shasum", vec!["-a", "256"])] {
        if let Ok(out) = Command::new(cmd).args(&args).arg(p).output() {
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or("").to_string());
            }
        }
    }
    if cfg!(windows) {
        let out = run_cmd(
            Command::new("powershell").args(["-NoProfile", "-Command"]).arg(format!("(Get-FileHash -Algorithm SHA256 -LiteralPath '{}').Hash", p.display())),
            "Get-FileHash",
        )?;
        return Ok(out.trim().to_ascii_lowercase());
    }
    Err("no sha256sum or shasum program to check the download".into())
}

fn run_cmd(c: &mut Command, what: &str) -> Result<String, String> {
    let out = c.output().map_err(|e| format!("could not run {what}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(4).collect();
        Err(format!("{what} failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | ")))
    }
}

fn decompile(gdre: &Path, g: &GameFiles, lib: &Library, tx: &Sender<Msg>) -> Result<usize, String> {
    let Source::Pck { file, .. } = &g.source else { return Err("scripts need the game's .pck".into()) };
    let tmp = lib.root.join("tmp_scripts");
    let _ = std::fs::remove_dir_all(&tmp);
    log(tx, format!("running {}", gdre.display()));
    run_cmd(
        Command::new(gdre).arg("--headless").arg(format!("--recover={}", file.display())).arg("--scripts-only").arg(format!("--output={}", tmp.display())),
        "GDRE",
    )?;
    let mut n = 0;
    let mut stack = vec![tmp.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "gd") {
                let rel = p.strip_prefix(&tmp).map_err(|e| e.to_string())?;
                let to = lib.game_dir().join(rel);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::copy(&p, &to).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if n < 100 {
        return Err(format!("GDRE produced only {n} scripts"));
    }
    Ok(n)
}

// ---------------------------------------------------------------------------------------------
// Stage 4: characters
// ---------------------------------------------------------------------------------------------

/// Python, only used as a fallback to unpack a download.
pub fn python() -> Option<String> {
    ["python3", "python", "py"].into_iter().find(|p| {
        Command::new(p).args(["-c", "import sys; sys.exit(0 if sys.version_info >= (3, 8) else 1)"]).status().is_ok_and(|s| s.success())
    }).map(str::to_string)
}

/// Converts the five characters with the built-in converter (in parallel; about a second).
fn convert_characters(lib: &Library, tx: &Sender<Msg>) -> Result<Vec<String>, String> {
    let controls = controls_source();
    log(tx, "converting the five characters");
    let results = convert::pipeline::convert_all(&lib.game_dir(), &lib.characters_dir(), controls.as_deref(), &[]);
    let mut done = Vec::new();
    for r in results {
        match r {
            Ok(d) => {
                log(tx, format!("{}: {} moves, {} projectiles", d.id, d.moves, d.projectiles));
                // the automatic button layout (hand-made controls from content/controls are kept)
                if let Err(e) = sim::layout::write_for(&lib.characters_dir(), &d.id, false) {
                    log(tx, e);
                }
                done.push(d.id);
            }
            Err(e) => log(tx, e),
        }
    }
    let _ = tx.send(Msg::Progress(1.0));
    // keep the roster's order
    let order = |id: &String| CHARACTERS.iter().position(|(c, _, _)| c == id).unwrap_or(usize::MAX);
    done.sort_by_key(order);
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn download_table_is_well_formed() {
        // only the entry for this system exists, but its fields must be complete
        if let Some((url, sha, file)) = gdre_download() {
            assert!(url.starts_with("https://github.com/GDRETools/gdsdecomp/releases/download/") && url.ends_with(file));
            assert!(sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn wav_round_trips() {
        let a = Audio { channels: 1, rate: 22050, samples: vec![0.0, 0.5, -0.5, 1.0] };
        let b = crate::gamefiles::decode_wav(&encode_wav(&a)).unwrap();
        assert_eq!((b.channels, b.rate, b.samples.len()), (1, 22050, 4));
        assert!((b.samples[1] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn rejects_folders_that_are_not_the_game() {
        let dir = std::env::temp_dir().join(format!("yemi-notgame-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check(&dir).unwrap_err().contains("no .pck"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Builds the asset part of the library from the real install into a temp folder and
    /// reads it back the way the menus will.
    #[test]
    fn extracts_a_working_library_from_the_installed_game() {
        let Some(found) = crate::steam::find_game() else { return };
        let root = std::env::temp_dir().join(format!("yemi-build-test-{}", std::process::id()));
        let lib = Library::new(root.clone());
        let g = GameFiles::open(&found.game_dir).unwrap();
        let (tx, rx) = channel();
        let n = extract(&g, &lib.game_dir(), &tx).unwrap();
        drop(tx);
        let _ = rx.try_iter().count();
        assert!(n > 2000, "{n} files");
        let built = GameFiles::open(&lib.game_dir()).unwrap();
        assert!(built.image("ui/title2.png").is_some_and(|i| i.ext == "webp"), "textures keep their real format");
        assert!(built.audio("sound/common/hit1.wav").is_some_and(|a| a.samples.len() > 1000));
        assert!(built.text("characters/BaseChar.tscn").is_some());
        let _ = std::fs::remove_dir_all(root);
    }
}
