//! The first-run setup wizard: the player points the game at their own copy of Your Only Move
//! Is HUSTLE once; the game reads it, builds its library on this computer (see `build.rs`), and
//! never needs the original again.
//!
//! This file holds the wizard's state and steps; its screens are drawn by `build_page`.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;

use bevy::prelude::*;

use crate::build::{self, Check, Msg, Request, StageState, STAGES};
use crate::library::{Library, Manifest};
use crate::menu::{button, check_row, Id, Ui};
use crate::pixel::*;
use crate::steam;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Welcome,
    Confirm,
    Building,
    Done,
}

pub struct Setup {
    pub library: Library,
    pub step: Step,
    pub detected: Vec<steam::Found>,
    pub chosen: Option<PathBuf>,
    pub check: Option<Result<Check, String>>,
    pub gdre: Option<PathBuf>,
    pub can_download: bool,
    pub allow_download: bool,
    pub stages: [StageState; 4],
    pub progress: f32,
    pub log: Vec<String>,
    pub result: Option<Result<Manifest, String>>,
    rx: Option<Mutex<Receiver<Msg>>>,
    pick: Option<Mutex<Receiver<Option<PathBuf>>>>,
    /// The folder dialog is open.
    pub picking: bool,
}

impl Setup {
    pub fn new(library: Library) -> Self {
        Setup {
            library,
            step: Step::Welcome,
            detected: vec![],
            chosen: None,
            check: None,
            gdre: None,
            can_download: false,
            allow_download: false,
            stages: std::array::from_fn(|_| StageState::Waiting),
            progress: 0.0,
            log: vec![],
            result: None,
            rx: None,
            pick: None,
            picking: false,
        }
    }

    /// Back to the first screen, looking again for installs and tools.
    pub fn reset(&mut self) {
        self.step = Step::Welcome;
        self.detected = steam::find_all();
        self.chosen = None;
        self.check = None;
        self.result = None;
        self.stages = std::array::from_fn(|_| StageState::Waiting);
        self.log.clear();
        self.progress = 0.0;
        self.refresh_tools();
    }

    pub fn refresh_tools(&mut self) {
        self.gdre = build::find_gdre(&self.library);
        self.can_download = build::gdre_download().is_some();
    }

    pub fn use_folder(&mut self, path: PathBuf) {
        self.check = Some(build::check(&path));
        self.chosen = Some(path);
        self.refresh_tools();
        self.step = Step::Confirm;
    }

    /// Opens the system's folder dialog without freezing the window.
    pub fn browse(&mut self) {
        if self.picking {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(steam::pick_folder("Choose the folder of Your Only Move Is HUSTLE"));
        });
        self.pick = Some(Mutex::new(rx));
        self.picking = true;
    }

    pub fn start(&mut self) {
        let Some(source) = self.chosen.clone() else { return };
        let (tx, rx) = channel();
        let req = Request { source, library: self.library.clone(), gdre: self.gdre.clone(), download: self.allow_download };
        std::thread::spawn(move || build::run(req, tx));
        self.rx = Some(Mutex::new(rx));
        self.stages = std::array::from_fn(|_| StageState::Waiting);
        self.log.clear();
        self.progress = 0.0;
        self.result = None;
        self.step = Step::Building;
    }

    /// Applies what the background threads reported; true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.pick {
            let got = rx.lock().ok().and_then(|r| r.try_recv().ok());
            if let Some(choice) = got {
                self.pick = None;
                self.picking = false;
                changed = true;
                if let Some(p) = choice {
                    self.use_folder(p);
                }
            }
        }
        let msgs: Vec<Msg> = match &self.rx {
            Some(rx) => rx.lock().map(|r| r.try_iter().collect()).unwrap_or_default(),
            None => vec![],
        };
        for m in msgs {
            changed = true;
            match m {
                Msg::Stage(i, s) => {
                    if matches!(s, StageState::Running) {
                        self.progress = 0.0;
                    }
                    self.stages[i] = s;
                }
                Msg::Progress(p) => self.progress = p,
                Msg::Log(l) => {
                    self.log.push(l);
                    if self.log.len() > 40 {
                        self.log.remove(0);
                    }
                }
                Msg::Finished(r) => {
                    self.result = Some(r);
                    self.rx = None;
                    self.step = Step::Done;
                }
            }
        }
        changed
    }

    pub fn ready(&self) -> bool {
        matches!(&self.result, Some(Ok(m)) if m.assets)
    }
}

// ---------------------------------------------------------------------------------------------
// Screens
// ---------------------------------------------------------------------------------------------

const PX: f32 = 100.0;
const PW: f32 = 440.0;

fn panel(p: &mut ChildSpawnerCommands, ui: &Ui, title: &str, h: f32) {
    let y = (VH - h) / 2.0;
    p.spawn((at(PX - 1.0, y - 1.0, PW + 2.0, h + 2.0), BackgroundColor(BLACK)));
    p.spawn((at(PX, y, PW, 11.0), BackgroundColor(PANEL.bg)));
    label(p, ui.skin, PX, y - 1.0, PW, 11.0, title, FONT, Align::Center);
    p.spawn((at(PX, y + 12.0, PW, h - 12.0), BackgroundColor(Color::srgb_u8(0x12, 0x12, 0x12))));
}

fn para(p: &mut ChildSpawnerCommands, ui: &Ui, x: f32, y: f32, w: f32, s: &str, color: Color) {
    p.spawn(at(x, y, w, 60.0)).with_children(|c| text(c, ui.skin, s, color, Some((w, Justify::Left))));
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_048_576.0)
}

fn short(p: &std::path::Path, max: usize) -> String {
    let s = p.to_string_lossy().to_string();
    if s.chars().count() <= max {
        s
    } else {
        let tail: String = s.chars().rev().take(max - 3).collect::<Vec<_>>().into_iter().rev().collect();
        format!("...{tail}")
    }
}

pub fn build_page(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let s = &ui.menu.setup;
    match s.step {
        Step::Welcome => welcome(p, ui),
        Step::Confirm => confirm(p, ui),
        Step::Building => building(p, ui),
        Step::Done => done(p, ui),
    }
    let _ = s;
}

fn welcome(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let skin = ui.skin;
    let h = 270.0;
    panel(p, ui, "Welcome - one-time setup", h);
    let y0 = (VH - h) / 2.0 + 18.0;
    label(p, skin, PX, y0, PW, 11.0, "Your Every Move Is HUSTLE", Color::WHITE, Align::Center);
    para(
        p,
        ui,
        PX + 12.0,
        y0 + 18.0,
        PW - 24.0,
        "This game is built from your own copy of Your Only Move Is HUSTLE. Choose the game's folder once: its art, sounds and character data are read and stored on this computer, so the original is not needed again. Nothing is uploaded, and nothing from the original comes with this game.",
        FONT,
    );
    let setup_detected = ui.menu.setup.detected.clone();
    let mut y = y0 + 84.0;
    if setup_detected.is_empty() {
        label(p, skin, PX + 12.0, y, PW - 24.0, 11.0, "No Steam copy was found on this computer.", FONT, Align::Left);
        y += 16.0;
    } else {
        label(p, skin, PX + 12.0, y, PW - 24.0, 11.0, "Found on this computer:", Color::srgb_u8(0x8c, 0x8c, 0x8c), Align::Left);
        y += 13.0;
        for (i, f) in setup_detected.iter().take(3).enumerate() {
            button(p, ui, Id::SetupUse(i), PX + 12.0, y, PW - 24.0, 14.0, &short(&f.game_dir, 66), false, false);
            y += 15.0;
        }
        y += 3.0;
    }
    let picking = ui.menu.setup.picking;
    button(p, ui, Id::SetupBrowse, PX + 12.0, y, PW - 24.0, 16.0, if picking { "waiting for the folder dialog..." } else { "Choose the game folder..." }, false, picking);
    y += 20.0;
    para(
        p,
        ui,
        PX + 12.0,
        y,
        PW - 24.0,
        "It is the folder that holds YourOnlyMoveIsHUSTLE.pck. In Steam: right-click the game, Manage, Browse local files.",
        Color::srgb_u8(0x8c, 0x8c, 0x8c),
    );
    button(p, ui, Id::SetupSkip, PX + 12.0, (VH + h) / 2.0 - 22.0, PW - 24.0, 14.0, "Skip for now (the game will look plain and have no characters)", false, false);
}

fn confirm(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let skin = ui.skin;
    let h = 270.0;
    panel(p, ui, "Ready to build", h);
    let y0 = (VH - h) / 2.0 + 20.0;
    let s = &ui.menu.setup;
    let folder = s.chosen.as_ref().map(|c| short(c, 70)).unwrap_or_default();
    label(p, skin, PX + 12.0, y0, PW - 24.0, 11.0, &folder, FONT, Align::Left);
    let check = s.check.clone();
    let (gdre, can_download, allow) = (s.gdre.clone(), s.can_download, s.allow_download);
    match check {
        Some(Err(e)) => {
            para(p, ui, PX + 12.0, y0 + 20.0, PW - 24.0, &e, RED);
            button(p, ui, Id::SetupBack, PX + 12.0, (VH + h) / 2.0 - 22.0, PW - 24.0, 14.0, "Back", false, false);
        }
        Some(Ok(c)) => {
            let mut y = y0 + 16.0;
            label(p, skin, PX + 12.0, y, PW - 24.0, 11.0, &format!("Found the game: {} files, {}", c.files, mb(c.size)), Color::srgb_u8(0x64, 0xd2, 0x6b), Align::Left);
            y += 13.0;
            if c.workshop_items > 0 {
                label(p, skin, PX + 12.0, y, PW - 24.0, 11.0, &format!("{} Workshop items found next to it (mods come later)", c.workshop_items), FONT, Align::Left);
                y += 13.0;
            }
            y += 6.0;
            label(p, skin, PX + 12.0, y, PW - 24.0, 11.0, "What will happen:", Color::srgb_u8(0x8c, 0x8c, 0x8c), Align::Left);
            y += 13.0;
            para(p, ui, PX + 12.0, y, PW - 24.0, &format!("1. Art, sounds and effects are copied into your game library at {}.", short(&ui.menu.setup.library.root, 44)), FONT);
            y += 31.0;
            // scripts
            match (&gdre, can_download) {
                (Some(g), _) => {
                    para(p, ui, PX + 12.0, y, PW - 24.0, &format!("2. Game scripts are read with GDRE ({}).", short(g, 40)), FONT);
                    y += 13.0;
                }
                (None, true) => {
                    para(p, ui, PX + 12.0, y, PW - 24.0, "2. Game scripts are read with GDRE, a free open-source tool that is not installed yet.", FONT);
                    y += 25.0;
                    check_row(p, ui, Id::SetupDownload, PX + 12.0, y, PW - 24.0, "download GDRE (35 MB, from its official GitHub page)", allow, FONT);
                    y += 14.0;
                }
                (None, false) => {
                    para(p, ui, PX + 12.0, y, PW - 24.0, "2. Game scripts need GDRE, which cannot be downloaded for this system. Install it and set YEMI_GDRE to its path, or continue without characters.", RED);
                    y += 38.0;
                }
            }
            para(p, ui, PX + 12.0, y + 2.0, PW - 24.0, "3. The five characters are converted (this takes about a second).", FONT);
            let by = (VH + h) / 2.0 - 38.0;
            button(p, ui, Id::SetupBuild, PX + 12.0, by, PW - 24.0, 16.0, "Build the game", false, false);
            button(p, ui, Id::SetupBack, PX + 12.0, by + 19.0, PW - 24.0, 14.0, "Back", false, false);
        }
        None => {}
    }
}

fn stage_line(s: &StageState) -> (String, Color) {
    match s {
        StageState::Waiting => ("waiting".into(), Color::srgb_u8(0x70, 0x70, 0x70)),
        StageState::Running => ("working...".into(), Color::srgb_u8(0xff, 0xd2, 0x4d)),
        StageState::Done(n) => (format!("done  {n}"), Color::srgb_u8(0x64, 0xd2, 0x6b)),
        StageState::Skipped(r) => (format!("skipped: {r}"), Color::srgb_u8(0xff, 0xd2, 0x4d)),
        StageState::Failed(r) => (format!("failed: {r}"), RED),
    }
}

fn stage_rows(p: &mut ChildSpawnerCommands, ui: &Ui, y0: f32) -> f32 {
    let skin = ui.skin;
    let s = &ui.menu.setup;
    let mut y = y0;
    for (i, name) in STAGES.iter().enumerate() {
        label(p, skin, PX + 12.0, y, 150.0, 11.0, name, FONT, Align::Left);
        let (txt, color) = stage_line(&s.stages[i]);
        p.spawn(at(PX + 160.0, y, PW - 172.0, 24.0)).with_children(|c| text(c, skin, &txt, color, Some((PW - 172.0, Justify::Left))));
        y += if txt.chars().count() > 52 { 26.0 } else { 14.0 };
        if matches!(s.stages[i], StageState::Running) {
            p.spawn((at(PX + 160.0, y, PW - 172.0, 5.0), BackgroundColor(BTN_NORMAL.bg)));
            p.spawn((at(PX + 160.0, y, (PW - 172.0) * s.progress.clamp(0.0, 1.0), 5.0), BackgroundColor(Color::srgb_u8(0x1d, 0x8c, 0xf5))));
            y += 9.0;
        }
    }
    y
}

fn building(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let h = 250.0;
    panel(p, ui, "Building your game", h);
    let y0 = (VH - h) / 2.0 + 22.0;
    let y = stage_rows(p, ui, y0);
    let skin = ui.skin;
    label(p, skin, PX + 12.0, y + 8.0, PW - 24.0, 11.0, "This takes a minute. You can leave the window open.", Color::srgb_u8(0x8c, 0x8c, 0x8c), Align::Left);
    let log: Vec<String> = ui.menu.setup.log.iter().rev().take(4).rev().cloned().collect();
    for (k, l) in log.iter().enumerate() {
        label(p, skin, PX + 12.0, (VH + h) / 2.0 - 62.0 + 12.0 * k as f32, PW - 24.0, 11.0, &short(std::path::Path::new(l), 74), Color::srgb_u8(0x70, 0x70, 0x70), Align::Left);
    }
}

fn done(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let h = 270.0;
    let ok = ui.menu.setup.ready();
    panel(p, ui, if ok { "Your game is ready" } else { "The build did not finish" }, h);
    let y0 = (VH - h) / 2.0 + 22.0;
    let y = stage_rows(p, ui, y0);
    let skin = ui.skin;
    let s = &ui.menu.setup;
    let by = (VH + h) / 2.0 - 38.0;
    match &s.result {
        Some(Ok(m)) => {
            let note = if m.characters.is_empty() {
                "The menus, art and sounds are ready. Characters were not converted, so fights are not available yet. Fix what is listed above and use Options, rebuild game library."
            } else {
                "Everything is ready. The original game is not needed any more."
            };
            para(p, ui, PX + 12.0, y + 6.0, PW - 24.0, note, FONT);
            button(p, ui, Id::SetupStart, PX + 12.0, by, PW - 24.0, 16.0, "Start playing", false, false);
            button(p, ui, Id::SetupBack, PX + 12.0, by + 19.0, PW - 24.0, 14.0, "Build again", false, false);
        }
        Some(Err(e)) => {
            para(p, ui, PX + 12.0, y + 6.0, PW - 24.0, e, RED);
            button(p, ui, Id::SetupBack, PX + 12.0, by, PW - 24.0, 16.0, "Try again", false, false);
            button(p, ui, Id::SetupSkip, PX + 12.0, by + 19.0, PW - 24.0, 14.0, "Skip for now", false, false);
        }
        None => {}
    }
    let _ = skin;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_the_folder_choice() {
        let mut s = Setup::new(Library::new(std::env::temp_dir().join("yemi-setup-test")));
        assert_eq!(s.step, Step::Welcome);
        s.use_folder(std::env::temp_dir()); // not a game folder
        assert_eq!(s.step, Step::Confirm);
        assert!(matches!(s.check, Some(Err(_))));
        assert!(!s.ready());
    }
}
