//! Playable prototype: main menu, character select with match settings, options, and the
//! fight screen (fighters drawn as boxes, debug HUD).
//!
//! Run from anywhere with `cargo run -p app --bin prototype`. Data paths default to the
//! workspace (`data/converted/<character>.ron`, its controls file, and `content/engine.ron`).
//! `--demo --shots 40,80 --out DIR` skips the menus, plays a scripted sequence, saves
//! screenshots at those simulation frames and exits. `--screen NAME` opens a menu page
//! instead (main, select, options, hotkeys, personalization, credits); with `--shots` the
//! frame numbers then count rendered frames.
//! `--fx-preview SCENE` plays one of the original's effect scenes alone (for comparing with
//! the original game); `--skin DIR` points at a different copy of the original's files.

mod art;
mod aura;
mod build;
mod customize;
mod fight;
mod fx;
mod gamefiles;
mod godot;
mod library;
mod menu;
mod pad;
mod pixel;
mod settings;
mod setup;
mod skin;
mod steam;
mod style;
mod training;

use std::path::PathBuf;

use bevy::audio::AddAudioSource;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};

pub use fight::Game;
use library::Library;
use menu::{Effect, Menu, Page, Tab};
use settings::Settings;

/// The converted characters, in display order.
pub const ROSTER: [&str; 5] = ["ninja", "robo", "wizard", "mutant", "cowboy"];

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Menu,
    Fight,
}

/// Where the game data lives.
#[derive(Resource, Clone)]
pub struct Paths {
    data_dir: PathBuf,
    /// The fighters in roster order: the official ones, then imported ones.
    ids: Vec<String>,
    /// Display names of imported fighters (the converted file only has an id).
    mod_names: std::collections::HashMap<String, String>,
    engine: PathBuf,
    /// `--character` / `--controls` replace both players' files with one explicit pair.
    custom: Option<(PathBuf, PathBuf)>,
    /// The original's scripts: the game's folder, and each imported fighter's mod folder.
    game_dir: PathBuf,
    mod_res: std::collections::HashMap<String, PathBuf>,
}

impl Paths {
    fn files(&self, pick: usize) -> (PathBuf, PathBuf) {
        if let Some(c) = &self.custom {
            return c.clone();
        }
        let id = self.ids.get(pick).map(String::as_str).unwrap_or(ROSTER[0]);
        (self.data_dir.join(format!("{id}.ron")), self.data_dir.join(format!("{id}_controls.ron")))
    }

    /// Finds the scripts of the picked fighters (their mods' folders, then the game's), when an
    /// imported fighter is picked. Official fighters run on their converted data alone.
    pub fn script_loader(&self, picks: [usize; 2]) -> Option<std::sync::Arc<dyn gdscript::Loader>> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for p in picks {
            if let Some(r) = self.ids.get(p).and_then(|id| self.mod_res.get(id)) {
                if !roots.contains(r) {
                    roots.push(r.clone());
                }
            }
        }
        if roots.is_empty() || !self.game_dir.join("project.godot").is_file() {
            return None;
        }
        roots.push(self.game_dir.clone());
        Some(std::sync::Arc::new(gdscript::FsLoader::new(roots).with_native(sim::NATIVE_SCRIPTS)))
    }

    fn set_mods(&mut self, library: &Library, index: modimport::index::Index) {
        self.game_dir = library.game_dir();
        self.mod_names = index.fighters.iter().map(|f| (f.id.clone(), f.name.clone())).collect();
        self.mod_res = index.fighters.iter().map(|f| (f.id.clone(), library.root.join("mods").join(&f.mod_id).join("res"))).collect();
        self.ids = ROSTER.iter().map(|s| s.to_string()).chain(index.fighters.into_iter().map(|f| f.id)).collect();
    }
}

/// What the character select shows about each character.
pub struct RosterEntry {
    pub id: String,
    pub name: String,
    pub moves: usize,
    pub hp: i32,
    pub modes: Vec<String>,
}

#[derive(Resource)]
pub struct Roster(pub Vec<RosterEntry>);

impl Roster {
    fn load(paths: &Paths) -> Self {
        let mod_names: std::collections::HashMap<String, String> = paths.mod_names.clone();
        let entries = (0..paths.ids.len())
            .map(|i| {
                let (ch, _) = paths.files(i);
                match std::fs::read_to_string(&ch).map_err(|e| e.to_string()).and_then(|t| charfmt::parse(&t).map_err(|e| e.to_string())) {
                    Ok(f) => RosterEntry {
                        id: paths.ids[i].clone(),
                        name: mod_names.get(&paths.ids[i]).cloned().unwrap_or_else(|| f.character.name.clone()),
                        moves: f.character.moves.len(),
                        hp: f.character.body.max_health,
                        modes: f.character.modes.clone(),
                    },
                    Err(e) => {
                        if paths.data_dir.exists() {
                            eprintln!("could not read {}: {e}", ch.display());
                        }
                        RosterEntry { id: paths.ids[i].clone(), name: format!("{} (missing)", paths.ids[i]), moves: 0, hp: 0, modes: vec![] }
                    }
                }
            })
            .collect();
        Roster(entries)
    }
}

/// Where the game's files are: the built library, unless overridden for development.
#[derive(Resource)]
pub struct Sources {
    pub library: Library,
    pub skin_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

impl Sources {
    fn game_dir(&self) -> PathBuf {
        self.skin_dir.clone().unwrap_or_else(|| self.library.game_dir())
    }
}

/// Moves the wizard's background work into the menu state.
fn poll_setup(mut menu: ResMut<Menu>) {
    if menu.setup.poll() {
        menu.dirty = true;
    }
}

/// A menu action for the app to carry out (start a fight, leave, quit...).
#[derive(Message)]
pub struct MenuEffect(pub Effect);

#[derive(Resource)]
struct Shots {
    frames: Vec<u64>,
    out_dir: PathBuf,
    exit_at: Option<u64>,
    rendered: u64,
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut skin_dir: Option<PathBuf> = None;
    let mut data_dir_override: Option<PathBuf> = None;
    let mut skip_setup = false;
    let mut setup_folder: Option<PathBuf> = None;
    let mut build_from: Option<PathBuf> = None;
    let mut download_gdre = false;
    let mut paths = Paths { data_dir: PathBuf::new(), ids: ROSTER.iter().map(|s| s.to_string()).collect(), mod_names: Default::default(), engine: root.join("content/engine.ron"), custom: None, game_dir: PathBuf::new(), mod_res: Default::default() };
    let mut settings = Settings::load();
    if std::env::var_os("YEMI_DEMO_NOBOXES").is_some() {
        settings.set_flag("hitboxes", false);
    }
    // YEMI_DEMO_CLEAN=1 (videos): no debug readouts or key help on the HUD
    if std::env::var_os("YEMI_DEMO_CLEAN").is_some() {
        settings.set_flag("extra_info", false);
        settings.show_controls_bar = false;
        settings.show_frame_advantage = false;
    }
    let mut picks = [0, 0];
    let mut demo = false;
    let mut fx_preview: Option<String> = None;
    let mut mode = fight::Mode::Versus;
    let mut screen = None;
    let mut shots = Vec::new();
    let mut out_dir = PathBuf::from(".");
    let (mut custom_character, mut custom_controls) = (None, None);
    let mut pick_names: [Option<String>; 2] = [None, None];
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--p1" => pick_names[0] = Some(args.next().expect("--p1 NAME")),
            "--p2" => pick_names[1] = Some(args.next().expect("--p2 NAME")),
            "--skin" => skin_dir = Some(args.next().expect("--skin DIR").into()),
            "--data" => data_dir_override = Some(args.next().expect("--data DIR").into()),
            "--library" => {
                // where the built library lives (default: the user's data folder)
                std::env::set_var("YEMI_LIBRARY", args.next().expect("--library DIR"));
            }
            "--skip-setup" => skip_setup = true,
            "--setup-folder" => setup_folder = Some(PathBuf::from(args.next().expect("--setup-folder DIR"))),
            "--build-library" => build_from = Some(PathBuf::from(args.next().expect("--build-library GAME_FOLDER"))),
            "--download-gdre" => download_gdre = true,
            "--character" => custom_character = Some(PathBuf::from(args.next().expect("--character PATH"))),
            "--controls" => custom_controls = Some(PathBuf::from(args.next().expect("--controls PATH"))),
            "--engine" => paths.engine = args.next().expect("--engine PATH").into(),
            "--demo" => demo = true,
            "--fx-preview" => {
                // plays one effect scene alone (for comparing with the original), on a demo fight
                fx_preview = Some(args.next().expect("--fx-preview SCENE"));
                demo = true;
            }
            "--screen" => screen = Some(args.next().expect("--screen NAME")),
            "--mode" => {
                // the demo's mode: versus, cpu-easy / cpu / cpu-hard, training
                mode = match args.next().expect("--mode MODE").as_str() {
                    "cpu-easy" => fight::Mode::Cpu(0),
                    "cpu" | "cpu-normal" => fight::Mode::Cpu(1),
                    "cpu-hard" => fight::Mode::Cpu(2),
                    "training" => fight::Mode::Training,
                    _ => fight::Mode::Versus,
                }
            }
            "--shots" => {
                shots = args.next().expect("--shots N,N").split(',').map(|s| s.trim().parse().expect("frame number")).collect()
            }
            "--out" => out_dir = args.next().expect("--out DIR").into(),
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let library = Library::new(Library::default_root());
    if let Some(source) = build_from {
        std::process::exit(build_headless(source, library, download_gdre));
    }
    paths.data_dir = data_dir_override.clone().unwrap_or_else(|| library.characters_dir());
    paths.set_mods(&library, library.mod_index());
    for (i, name) in pick_names.iter().enumerate() {
        if let Some(name) = name {
            picks[i] = fight::roster_index(&paths.ids, name).unwrap_or_else(|| {
                eprintln!("unknown character {name}; choose one of {}", paths.ids.join(", "));
                std::process::exit(2);
            });
        }
    }
    if custom_character.is_some() || custom_controls.is_some() {
        let (ch, ctl) = paths.files(picks[0]);
        paths.custom = Some((custom_character.unwrap_or(ch), custom_controls.unwrap_or(ctl)));
    }
    shots.sort();

    let first_run = !demo && !skip_setup && skin_dir.is_none() && !library.has_assets();
    let mut menu = match screen.as_deref() {
        None if first_run => Menu::new(Page::Setup),
        None | Some("main") => Menu::new(Page::Main),
        Some("setup") => Menu::new(Page::Setup),
        Some(s @ ("setup-confirm" | "setup-build")) => {
            // wizard screens for screenshots: pick the first Steam install (or --setup-folder)
            let mut m = Menu::new(Page::Setup);
            let folder = setup_folder.clone().or_else(|| m.setup.detected.first().map(|f| f.game_dir.clone()));
            if let Some(f) = folder {
                m.setup.use_folder(f);
                if s == "setup-build" {
                    m.setup.start();
                }
            }
            m
        }
        Some("select") => Menu::new(Page::Select),
        Some("credits") => Menu::new(Page::Credits),
        Some("roadmap") => {
            let mut m = Menu::new(Page::Main);
            m.roadmap = true;
            m
        }
        Some("pause") => Menu::new(Page::Pause),
        Some("pause-training") => {
            let mut m = Menu::new(Page::Pause);
            m.mode = fight::Mode::Training;
            m
        }
        Some("customize") => Menu::new(Page::Customize),
        Some("modes") => Menu::new(Page::Modes),
        Some(tab @ ("options" | "hotkeys" | "personalization" | "controller")) => {
            let mut m = Menu::new(Page::Options);
            m.tab = match tab {
                "hotkeys" => Tab::Hotkeys,
                "controller" => Tab::Controller,
                "personalization" => Tab::Personalization,
                _ => Tab::Options,
            };
            m
        }
        Some(other) => {
            eprintln!("unknown screen {other}; use main, select, options, hotkeys, personalization or credits");
            std::process::exit(2);
        }
    };
    let mut app = App::new();
    if demo {
        // straight into a fight with the scripted inputs
        // the scripted inputs walk in from the prototype's old, closer start
        let mut rules = settings.rules.clone();
        if let Some(f) = settings::RULES.iter().find(|f| f.key() == Some("char_distance")) {
            rules.set(f, 90.0);
        }
        let styles = style::Styles::load();
        // YEMI_DEMO_STYLES="Inferno,Frost": the demo fighters' styles
        let names: Vec<String> = std::env::var("YEMI_DEMO_STYLES").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_else(|_| settings.styles.to_vec());
        let picked = [0, 1].map(|i| styles.for_player(i, names.get(i).map(String::as_str).unwrap_or("")));
        // YEMI_DEMO_INTRO=1: the demo opens with the battle intro
        match Game::new(&paths, picks, &rules, true, mode, picked, std::env::var_os("YEMI_DEMO_INTRO").is_some()) {
            Ok(g) => {
                app.insert_resource(g);
            }
            Err(e) => {
                eprintln!("could not load the game data: {e}");
                std::process::exit(1);
            }
        }
        menu.stack.clear();
    }
    let window = Window {
        title: menu::TITLE.into(),
        resolution: (1280, 720).into(),
        mode: if settings.flag("fullscreen") { WindowMode::BorderlessFullscreen(MonitorSelection::Current) } else { WindowMode::Windowed },
        present_mode: if settings.flag("vsync") { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync },
        ..default()
    };
    let sources = Sources { library, skin_dir, data_dir: data_dir_override };
    let game_dir = sources.game_dir();
    let mod_root = sources.library.root.clone();
    let data_dir = paths.data_dir.clone();
    app.insert_resource(ClearColor(pixel::CLEAR))
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(Roster::load(&paths))
        .insert_resource(paths)
        .insert_resource(settings)
        .insert_resource(menu)
        .insert_resource(Shots { frames: shots, out_dir, exit_at: None, rendered: 0 })
        .insert_resource(pixel::RingsShown(false))
        .insert_resource(style::Styles::load())
        .init_resource::<aura::Emitters>()
        .init_resource::<pad::Pads>()
        .init_resource::<aura::PreviewFrame>()
        .add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }).set(ImagePlugin::default_nearest()))
        .add_audio_source::<fx::Pcm>()
        .init_state::<AppState>()
        .insert_state(if demo { AppState::Fight } else { AppState::Menu })
        .add_message::<MenuEffect>()
        .add_systems(
            Startup,
            (
                move |mut commands: Commands, mut fonts: ResMut<Assets<Font>>, mut images: ResMut<Assets<Image>>, mut audio: ResMut<Assets<fx::Pcm>>, mut lib: ResMut<fx::FxLibrary>, mut art: ResMut<art::Art>| {
                    commands.spawn(Camera2d);
                    commands.insert_resource(aura::make_shapes(&mut images));
                    let mut skin = skin::Skin::load(&game_dir, &mut fonts, &mut images, &mut audio);
                    add_logo(&mut skin, &mut images);
                    register_mods(&Library::new(mod_root.clone()), &mut lib, &mut art, &mut skin, &mut images);
                    if skin.source.is_none() {
                        eprintln!("no game library at {}; run the setup to build it (menus use the fallback look until then)", game_dir.display());
                    }
                    commands.insert_resource(skin);
                },
                pixel::spawn_rings,
            ),
        )
        .insert_resource(fx::FxLibrary::new(Some(sources.game_dir()).filter(|d| d.join("project.godot").exists()), &data_dir))
        .insert_resource(sources)
        .init_resource::<fx::Shake>()
        .init_resource::<art::Art>()
        .insert_resource(FxPreview(fx_preview))
        .add_systems(OnEnter(AppState::Fight), fight::spawn_fight)
        .add_systems(OnExit(AppState::Fight), despawn_effects)
        .add_systems(
            FixedUpdate,
            (fight::tick, fight::fight_fx, fight::fight_auras, preview_effect, fx::tick_effects.run_if(|g: Option<Res<Game>>| g.is_some_and(|g| g.stepped)))
                .chain()
                .run_if(in_state(AppState::Fight)),
        )
        .add_systems(FixedUpdate, (customize::preview_auras.run_if(in_state(AppState::Menu)), aura::tick_particles).chain())
        .add_systems(
            Update,
            (
                poll_setup,
                pad::track,
                menu::menu_input,
                handle_effects,
                global_hotkeys,
                fight::fight_hotkeys.run_if(in_state(AppState::Fight)),
                apply_settings,
                pixel::fit_canvas,
                menu::render_menu,
                menu::play_sounds,
                pixel::animate_rings,
                fx::update_shake,
                (fight::draw, fight::scene_particles, fight::draw_objects, fx::draw_effects, fight::hud).chain().run_if(in_state(AppState::Fight)),
                customize::preview,
                aura::draw_particles,
                screenshots,
            )
                .chain(),
        )
        .run();
}

/// The game's own logo (part of this project, so always there).
fn add_logo(skin: &mut skin::Skin, images: &mut Assets<Image>) {
    let bytes = include_bytes!("../../../content/ui/logo.png");
    if let Ok(img) = Image::from_buffer(bytes, bevy::image::ImageType::Extension("png"), bevy::image::CompressedImageFormats::NONE, true, bevy::image::ImageSampler::nearest(), bevy::asset::RenderAssetUsages::default()) {
        skin.add_image("logo".into(), images.add(img));
    }
}

/// Tells the art, effect and menu code about the fighters imported from mods: where their
/// files are, which scene each one is, and a portrait for the select screen.
fn register_mods(library: &Library, lib: &mut fx::FxLibrary, art: &mut art::Art, skin: &mut skin::Skin, images: &mut Assets<Image>) {
    let index = library.mod_index();
    lib.set_mod_dirs(index.res_dirs(&library.root));
    let dirs = lib.dirs();
    for f in &index.fighters {
        art.scenes.insert(f.id.clone(), f.scene.clone());
        let portrait = art.character(&dirs, &f.id).and_then(|c| c.idle_path().map(str::to_string));
        if let Some((image, _)) = portrait.and_then(|p| lib.texture(&p, images)) {
            skin.add_image(format!("portrait_{}", f.id), image);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_effects(
    mut art: ResMut<art::Art>,
    mut effects: MessageReader<MenuEffect>,
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    settings: Res<Settings>,
    mut paths: ResMut<Paths>,
    sources: Res<Sources>,
    mut fonts: ResMut<Assets<Font>>,
    mut images: ResMut<Assets<Image>>,
    mut pcm: ResMut<Assets<fx::Pcm>>,
    mut game: Option<ResMut<Game>>,
    mut next: ResMut<NextState<AppState>>,
    mut exit: MessageWriter<AppExit>,
    styles: Res<style::Styles>,
) {
    for MenuEffect(e) in effects.read() {
        match e {
            Effect::None => {}
            Effect::StartFight(picks, mode) => match Game::new(
                &paths,
                *picks,
                &settings.rules,
                false,
                *mode,
                [styles.for_player(0, &settings.styles[0]), styles.for_player(1, &settings.styles[1])],
                settings.flag("battle_intros"),
            ) {
                Ok(g) => {
                    commands.insert_resource(g);
                    menu.close();
                    next.set(AppState::Fight);
                }
                Err(e) => {
                    menu.status = format!("could not load: {e}");
                    menu.dirty = true;
                }
            },
            Effect::Rematch => {
                if let Some(g) = game.as_mut() {
                    g.rematch(settings.flag("battle_intros"));
                }
                menu.close();
            }
            Effect::Resume => menu.close(),
            Effect::ToMenu(pages) => {
                menu.reset_to(pages);
                next.set(AppState::Menu);
            }
            Effect::Forfeit => {
                if let Some(g) = game.as_mut() {
                    if g.world.winner.is_none() {
                        g.world.winner = Some(1);
                    }
                    g.over_frames = 91; // the post-game menu is opened right here
                }
                menu.reset_to(&[Page::PostGame]);
            }
            Effect::LibraryReady => {
                // the freshly built library replaces the plain look and the empty roster
                paths.data_dir = sources.data_dir.clone().unwrap_or_else(|| sources.library.characters_dir());
                let game_dir = sources.game_dir();
                let mut new_skin = skin::Skin::load(&game_dir, &mut fonts, &mut images, &mut pcm);
                add_logo(&mut new_skin, &mut images);
                let mut new_lib = fx::FxLibrary::new(Some(game_dir).filter(|d| d.join("project.godot").exists()), &paths.data_dir);
                paths.set_mods(&sources.library, sources.library.mod_index());
                register_mods(&sources.library, &mut new_lib, &mut art, &mut new_skin, &mut images);
                commands.insert_resource(new_skin);
                commands.insert_resource(new_lib);
                commands.insert_resource(Roster::load(&paths));
                menu.dirty = true;
            }
            Effect::Restart => {
                if let Some(g) = game.as_mut() {
                    if g.mode == fight::Mode::Training {
                        g.training_reset(&settings);
                    } else {
                        g.restart();
                    }
                }
                menu.close();
            }
            Effect::ToSelect => {
                if let Some(g) = game.as_ref() {
                    menu.mode = g.mode;
                }
                menu.reset_to(&[Page::Main, Page::Modes, Page::Select]);
                next.set(AppState::Menu);
            }
            Effect::Record => {
                if let Some(g) = game.as_mut() {
                    let t = &mut g.training;
                    t.recording = !t.recording;
                    t.playing = None;
                    if t.recording {
                        t.tape.clear();
                        g.status = format!("recording the dummy: your keys move P2 ({} or the pause menu stops)", settings.hotkeys.record_dummy.label());
                    } else {
                        g.status = format!("recorded {} ticks", t.tape.len());
                    }
                }
                menu.close();
            }
            Effect::PlayRecording => {
                if let Some(g) = game.as_mut() {
                    g.training.recording = false;
                    g.training.playing = Some(0);
                }
                menu.close();
            }
            Effect::ApplyCpu => {
                if let Some(g) = game.as_mut() {
                    g.set_cpu_level(settings.cpu_level);
                }
            }
            Effect::Quit => {
                settings.save();
                exit.write(AppExit::Success);
            }
        }
    }
}

fn global_hotkeys(keys: Res<ButtonInput<KeyCode>>, mut settings: ResMut<Settings>, menu: Res<Menu>) {
    if menu.rebinding.is_none() && menu.editing.is_none() && settings.hotkeys.toggle_fullscreen.just_pressed(&keys) {
        let on = settings.flag("fullscreen");
        settings.set_flag("fullscreen", !on);
    }
}

/// Applies window settings and saves whenever the settings change.
fn apply_settings(settings: Res<Settings>, mut windows: Query<&mut Window, With<PrimaryWindow>>, mut loaded: Local<bool>) {
    if !settings.is_changed() {
        return;
    }
    if let Ok(mut w) = windows.single_mut() {
        let mode = if settings.flag("fullscreen") { WindowMode::BorderlessFullscreen(MonitorSelection::Current) } else { WindowMode::Windowed };
        if w.mode != mode {
            w.mode = mode;
        }
        let present = if settings.flag("vsync") { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
        if w.present_mode != present {
            w.present_mode = present;
        }
    }
    // the first change is the resource being inserted at startup; nothing to save yet
    if *loaded {
        settings.save();
    }
    *loaded = true;
}

fn screenshots(
    mut commands: Commands,
    mut shots: ResMut<Shots>,
    game: Option<Res<Game>>,
    state: Res<State<AppState>>,
    mut exit: MessageWriter<AppExit>,
) {
    shots.rendered += 1;
    let frame = match (state.get(), &game) {
        // a demo with fresh rounds counts its own ticks, which never go back
        (AppState::Fight, Some(g)) if g.demo => g.shown_ticks,
        (AppState::Fight, Some(g)) => g.world.frame,
        _ => shots.rendered,
    };
    if let Some(&next) = shots.frames.first() {
        if frame >= next {
            shots.frames.remove(0);
            let path = shots.out_dir.join(format!("frame-{next:04}.png"));
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            if shots.frames.is_empty() {
                shots.exit_at = Some(frame + 30);
            }
        }
    }
    if shots.exit_at.is_some_and(|e| frame >= e) {
        exit.write(AppExit::Success);
    }
}

fn despawn_effects(mut commands: Commands, fxs: Query<Entity, With<fx::ActiveFx>>) {
    for e in &fxs {
        commands.entity(e).despawn();
    }
}

#[derive(Resource)]
struct FxPreview(Option<String>);

/// `--fx-preview`: hides the fighters and plays the effect above the stage at frame 60.
fn preview_effect(
    mut commands: Commands,
    preview: Res<FxPreview>,
    mut game: ResMut<Game>,
    mut lib: ResMut<fx::FxLibrary>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<&mut Visibility, Or<(With<fight::Body>, With<fight::Nose>)>>,
) {
    let Some(scene) = &preview.0 else { return };
    for mut v in &mut sprites {
        *v = Visibility::Hidden;
    }
    game.hud_hidden = true;
    if game.stepped && game.world.frame == 60 {
        let at = Vec2::new(game.cam_x, -60.0);
        fx::spawn_effect(&mut commands, &mut lib, &mut images, scene, at, Vec2::X);
    }
}

/// `--build-library FOLDER`: the wizard's build without the window; prints each step.
fn build_headless(source: PathBuf, library: Library, download: bool) -> i32 {
    let (tx, rx) = std::sync::mpsc::channel();
    let lib = library.clone();
    let worker = std::thread::spawn(move || build::run(build::Request { source, library: lib, gdre: None, download }, tx));
    let mut code = 0;
    for msg in rx {
        match msg {
            build::Msg::Stage(i, s) => println!("[{}] {:?}", build::STAGES[i], s),
            build::Msg::Log(l) => println!("    {l}"),
            build::Msg::Progress(_) => {}
            build::Msg::Finished(Ok(m)) => {
                println!("built {} files into {}; characters: {}", m.files, library.root.display(), m.characters.join(", "));
            }
            build::Msg::Finished(Err(e)) => {
                eprintln!("build failed: {e}");
                code = 1;
            }
        }
    }
    let _ = worker.join();
    code
}
