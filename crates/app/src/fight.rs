//! The fight screen: fighters drawn as boxes, keyboard input from the bound keys, debug HUD.

use bevy::prelude::*;
use sim::{Act, Input, World};

use crate::menu::{Menu, Page};
use crate::settings::{p2_control, rules_to_engine, rules_to_sim, P2Control, Settings, Values};
use crate::style::Style;
use crate::training::Training;
use crate::{AppState, Paths};

/// How a match is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Two players at one keyboard (or P1 against a simple dummy, as the match settings say).
    Versus,
    /// P1 against the CPU at a level (0 easy, 1 normal, 2 hard).
    Cpu(u8),
    /// P1 against a configurable dummy, with no KO and the training readouts.
    Training,
}

impl Mode {
    pub fn label(self) -> String {
        match self {
            Mode::Versus => "versus".into(),
            Mode::Cpu(l) => format!("vs CPU ({})", sim::cpu::Level::from_index(l).label()),
            Mode::Training => "training".into(),
        }
    }
}

/// Ticks of a battle intro: the fighters' intro poses with the names, READY, then FIGHT!.
pub const INTRO_TICKS: u32 = 170;

pub const ZOOM: f32 = 2.5;
pub const GROUND_Y: f32 = -110.0;
const HALF_W_PX: f32 = 640.0;

#[derive(Resource)]
pub struct Game {
    pub world: World,
    /// Finds the picked fighters' scripts, when they carry any (imported mods).
    scripts: Option<std::sync::Arc<dyn gdscript::Loader>>,
    pub picks: [usize; 2],
    pub rules: Values,
    /// Simulation frozen by the freeze hotkey (frame advance steps it).
    pub frozen: bool,
    pub step_once: bool,
    pub p2: P2Control,
    pub demo: bool,
    pub status: String,
    pub cam_x: f32,
    pub hud_hidden: bool,
    /// Frames since the round ended; the post-game menu opens after a short pause.
    pub over_frames: u32,
    /// The simulation advanced on this fixed tick (effects follow it).
    pub stepped: bool,
    /// Ticks since the demo started (the scripted inputs follow this, not the round's frame).
    pub demo_ticks: u64,
    pub mode: Mode,
    /// Each fighter's style (colors, outline, hit spark, auras).
    pub styles: [Style; 2],
    cpu: Option<sim::cpu::Cpu>,
    /// Ticks left of the round's intro (0 while fighting), and of the FIGHT! banner after it.
    pub intro: u32,
    pub banner: u32,
    /// The intro's sounds are still to play.
    pub intro_sound: bool,
    pub training: Training,
    /// A CPU playing P1 in demo videos.
    demo_cpu: Option<sim::cpu::Cpu>,
    /// Fixed ticks shown (intro and slow motion included), for demo screenshots.
    pub shown_ticks: u64,
}

impl Game {
    pub fn new(paths: &Paths, picks: [usize; 2], rules: &Values, demo: bool, mode: Mode, styles: [Style; 2], intro: bool) -> Result<Self, String> {
        let scripts = paths.script_loader(picks);
        let mut world = build_world_with(paths, picks, rules, scripts.clone())?;
        if mode == Mode::Training {
            world.rules.round_frames = None;
        }
        let cpu = cpu_for(mode, &world);
        let intro = if intro && mode != Mode::Training { INTRO_TICKS } else { 0 };
        if intro > 0 {
            // fighters with a scripted "Start" state play it as their intro
            world.begin_intro();
        }
        Ok(Game {
            mode,
            styles,
            cpu,
            intro,
            banner: 0,
            intro_sound: intro > 0,
            training: Training::default(),
            demo_cpu: None,
            shown_ticks: 0,
            world,
            scripts,
            picks,
            rules: rules.clone(),
            frozen: false,
            step_once: false,
            p2: if mode == Mode::Versus { p2_control(rules) } else { P2Control::Stand },
            demo,
            status: String::new(),
            cam_x: 0.0,
            hud_hidden: false,
            over_frames: 0,
            stepped: false,
            demo_ticks: 0,
        })
    }

    pub fn restart(&mut self) {
        self.world = World::with_rules(self.world.cfg.clone(), self.world.chars.clone(), self.world.rules.clone());
        attach_scripts(&mut self.world, self.scripts.clone());
        self.cpu = cpu_for(self.mode, &self.world);
        self.demo_cpu = None;
        self.over_frames = 0;
        self.status = "round reset".into();
    }

    /// A new round that opens with the battle intro (rematches).
    pub fn rematch(&mut self, intro: bool) {
        self.restart();
        self.status.clear();
        if intro && self.mode != Mode::Training && !self.demo {
            self.intro = INTRO_TICKS;
            self.intro_sound = true;
            self.world.begin_intro();
        }
    }

    /// Training's reset: a fresh round at the chosen position, the recording kept.
    pub fn training_reset(&mut self, settings: &Settings) {
        self.restart();
        self.training.reset();
        Training::place(&mut self.world, settings);
        self.status = "position reset".into();
    }

    /// Switches the CPU opponent to another level (from the pause menu).
    pub fn set_cpu_level(&mut self, level: u8) {
        if matches!(self.mode, Mode::Cpu(_)) {
            self.mode = Mode::Cpu(level);
            self.cpu = cpu_for(self.mode, &self.world);
        }
    }

    /// Who controls P2, for the HUD.
    pub fn p2_label(&self, settings: &Settings) -> String {
        match self.mode {
            _ if self.demo => "dummy".into(),
            Mode::Cpu(l) => format!("CPU {}", sim::cpu::Level::from_index(l).label()),
            Mode::Training => format!("dummy: {}", crate::settings::DUMMY_ACTIONS[(settings.train("dummy") as usize).min(crate::settings::DUMMY_ACTIONS.len() - 1)]),
            Mode::Versus => self.p2.label().into(),
        }
    }
}

fn cpu_for(mode: Mode, w: &World) -> Option<sim::cpu::Cpu> {
    match mode {
        Mode::Cpu(l) => {
            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
            Some(sim::cpu::Cpu::new(w, 1, sim::cpu::Level::from_index(l), seed))
        }
        _ => None,
    }
}

pub fn build_world(paths: &Paths, picks: [usize; 2], rules: &Values) -> Result<World, String> {
    build_world_with(paths, picks, rules, paths.script_loader(picks))
}

fn attach_scripts(world: &mut World, loader: Option<std::sync::Arc<dyn gdscript::Loader>>) {
    if let Some(l) = loader {
        for e in world.attach_scripts(l) {
            eprintln!("script set-up: {e}");
        }
    }
}

fn build_world_with(paths: &Paths, picks: [usize; 2], rules: &Values, scripts: Option<std::sync::Arc<dyn gdscript::Loader>>) -> Result<World, String> {
    let mut cfg = sim::EngineConfig::load(&paths.engine)?;
    rules_to_engine(rules, &mut cfg);
    let load = |i: usize| -> Result<std::sync::Arc<sim::Character>, String> {
        let (ch, ctl) = paths.files(picks[i]);
        Ok(std::sync::Arc::new(sim::Character::load(&ch, &ctl)?))
    };
    let mut world = World::with_rules(cfg, [load(0)?, load(1)?], rules_to_sim(rules));
    attach_scripts(&mut world, scripts);
    for (i, c) in world.chars.iter().enumerate() {
        for w in &c.warnings {
            eprintln!("data warning (P{}): {w}", i + 1);
        }
    }
    Ok(world)
}

#[derive(Component)]
pub struct Body(usize);
#[derive(Component)]
pub struct Nose(usize);
/// The fighter's real sprite; the box and nose are drawn only when it is unavailable.
#[derive(Component)]
pub struct Art(usize);
/// A pooled sprite for the k-th live projectile.
#[derive(Component)]
pub struct ProjArt(usize);
/// The extra sprite a fighter's move shows (chainsaw arm, shooting arm...).
#[derive(Component)]
pub struct OverlayArt(usize);

const PROJ_POOL: usize = 24;
#[derive(Component)]
pub struct HealthFill(usize);
/// The meter under a health bar; one notch per super bar.
#[derive(Component)]
pub struct MeterFill(usize);
#[derive(Component)]
pub enum Hud {
    Player(usize),
    Timer,
    Log,
    Help,
    /// The big centred line: the intro's names, READY, FIGHT!, K.O.
    Banner,
    /// Training's readouts (frame data, combo, input history).
    Training,
}

fn text(size: f32) -> TextFont {
    TextFont { font_size: FontSize::Px(size), ..default() }
}

pub fn spawn_fight(mut commands: Commands) {
    let scoped = DespawnOnExit(AppState::Fight);
    for i in 0..2 {
        commands.spawn((Body(i), Sprite::from_color(Color::WHITE, Vec2::ONE), Transform::default(), scoped.clone()));
        commands.spawn((Art(i), Sprite::default(), Transform::default(), Visibility::Hidden, scoped.clone()));
        commands.spawn((OverlayArt(i), Sprite::default(), Transform::default(), Visibility::Hidden, scoped.clone()));
        commands.spawn((Nose(i), Sprite::from_color(Color::WHITE, Vec2::splat(8.0)), Transform::default(), scoped.clone()));
    }
    for k in 0..PROJ_POOL {
        commands.spawn((ProjArt(k), Sprite::default(), Transform::default(), Visibility::Hidden, scoped.clone()));
    }
    for i in 0..2 {
        let side = if i == 0 { Node { left: px(10), ..default() } } else { Node { right: px(10), ..default() } };
        commands
            .spawn((
                Node { position_type: PositionType::Absolute, top: px(7), width: px(230), height: px(9), ..side },
                BackgroundColor(Color::srgb(0.25, 0.08, 0.08)),
                scoped.clone(),
            ))
            .with_children(|p| {
                p.spawn((
                    HealthFill(i),
                    Node { width: percent(100), height: percent(100), ..default() },
                    BackgroundColor(if i == 0 { Color::srgb(0.35, 0.75, 1.0) } else { Color::srgb(1.0, 0.55, 0.35) }),
                ));
            });
        let side = if i == 0 { Node { left: px(10), ..default() } } else { Node { right: px(10), ..default() } };
        commands
            .spawn((
                Node { position_type: PositionType::Absolute, top: px(18), width: px(120), height: px(4), ..side },
                BackgroundColor(Color::srgb(0.12, 0.12, 0.16)),
                scoped.clone(),
            ))
            .with_children(|p| {
                p.spawn((MeterFill(i), Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(Color::srgb(1.0, 0.85, 0.2))));
            });
    }
    let hud = |commands: &mut Commands, h: Hud, size: f32, color: Color, node: Node| {
        commands.spawn((h, Text::new(""), text(size), TextColor(color), node, scoped.clone()));
    };
    let abs = |n: Node| Node { position_type: PositionType::Absolute, ..n };
    // positions in canvas pixels: the UI is scaled so a 640x360 canvas fills the window
    hud(&mut commands, Hud::Player(0), 7.5, Color::WHITE, abs(Node { top: px(24), left: px(10), ..default() }));
    hud(&mut commands, Hud::Player(1), 7.5, Color::WHITE, abs(Node { top: px(24), right: px(10), ..default() }));
    hud(&mut commands, Hud::Timer, 15.0, Color::WHITE, abs(Node { top: px(3), left: percent(47), ..default() }));
    hud(&mut commands, Hud::Log, 7.0, Color::srgb(0.8, 0.82, 0.86), abs(Node { bottom: px(23), left: px(10), ..default() }));
    hud(&mut commands, Hud::Help, 6.5, Color::srgb(0.6, 0.62, 0.66), abs(Node { bottom: px(5), left: px(10), ..default() }));
    commands.spawn((
        Hud::Banner,
        Text::new(""),
        text(26.0),
        TextColor(Color::WHITE),
        TextLayout::justify(Justify::Center),
        abs(Node { top: px(120), left: px(0), width: percent(100), justify_content: JustifyContent::Center, ..default() }),
        scoped.clone(),
    ));
    hud(&mut commands, Hud::Training, 7.0, Color::srgb(0.85, 0.95, 0.85), abs(Node { top: px(70), left: px(10), ..default() }));
}

// ---------------------------------------------------------------------------------------------
// Input and simulation
// ---------------------------------------------------------------------------------------------

/// Holding "back" for player `i` (away from the opponent).
fn back(w: &World, i: usize) -> (bool, bool) {
    let facing_right = w.f[i].facing > 0;
    (facing_right, !facing_right) // (left, right)
}

fn dummy_input(d: P2Control, w: &World) -> Input {
    let (left, right) = back(w, 1);
    match d {
        P2Control::Human | P2Control::Stand => Input::default(),
        P2Control::Block => Input { left, right, ..default() },
        P2Control::CrouchBlock => Input { left, right, down: true, ..default() },
        P2Control::Jump => Input { up: w.frame % 70 < 2, ..default() },
    }
}

/// Scripted inputs for `--demo`: walk in, punch, heavy route, jump attack, grab.
/// A demo input script (see `YEMI_DEMO_SEQ`) for player `i` at `frame`.
fn seq_input(seq: &str, frame: u64, w: &World, i: usize) -> Input {
    let toward_right = w.f[i].facing > 0;
    let mut input = Input::default();
    for part in seq.split(';').filter(|p| !p.trim().is_empty()) {
        let Some((range, keys)) = part.split_once(':') else { continue };
        let (a, b) = match range.trim().split_once('-') {
            Some((a, b)) => (a.trim().parse::<u64>().unwrap_or(0), b.trim().parse::<u64>().unwrap_or(0)),
            None => {
                let n = range.trim().parse::<u64>().unwrap_or(u64::MAX);
                (n, n)
            }
        };
        if frame < a || frame > b {
            continue;
        }
        for k in keys.split('+') {
            match k.trim() {
                "left" => input.left = true,
                "right" => input.right = true,
                "up" => input.up = true,
                "down" => input.down = true,
                "light" => input.light = true,
                "heavy" => input.heavy = true,
                "special" => input.special = true,
                "grab" => input.grab = true,
                "dash" => input.dash = true,
                "fwd" => {
                    input.right |= toward_right;
                    input.left |= !toward_right;
                }
                "back" => {
                    input.right |= !toward_right;
                    input.left |= toward_right;
                }
                _ => {}
            }
        }
    }
    input
}

fn demo_input(frame: u64, w: &World) -> Input {
    let toward_right = w.f[0].facing > 0;
    let fwd = |on: bool| Input { right: on && toward_right, left: on && !toward_right, ..default() };
    // YEMI_DEMO_SEQ="0-24:fwd;30:light;..." plays that script: frames (one or a-b) and keys joined
    // with +: left right up down light heavy special grab dash, plus fwd / back (toward / away
    // from the opponent)
    if let Ok(seq) = std::env::var("YEMI_DEMO_SEQ") {
        return seq_input(&seq, frame, w, 0);
    }
    // YEMI_DEMO_BUTTON=special (or light/heavy/grab/dash): press that button every 60 frames instead
    if let Ok(b) = std::env::var("YEMI_DEMO_BUTTON") {
        // YEMI_DEMO_AIR=1: jump first, so the button is pressed in the air
        if std::env::var("YEMI_DEMO_AIR").is_ok() && frame == 2 {
            return Input { up: true, ..default() };
        }
        if frame >= 20 && frame % 60 == 20 {
            return Input { light: b == "light", heavy: b == "heavy", special: b == "special", grab: b == "grab", dash: b == "dash", ..default() };
        }
        return Input::default();
    }
    match frame {
        0..=24 => fwd(true),
        30 => Input { light: true, ..default() },
        60 => Input { heavy: true, ..default() },
        72..=74 => Input { heavy: true, ..default() },
        110 => Input { up: true, ..default() },
        124 => Input { light: true, ..default() },
        165..=186 => fwd(true),
        188 => Input { grab: true, ..default() },
        _ => Input::default(),
    }
}

/// Keyboard and controller together: either can play.
fn merge(a: Input, b: Input) -> Input {
    Input {
        left: a.left || b.left,
        right: a.right || b.right,
        up: a.up || b.up,
        down: a.down || b.down,
        light: a.light || b.light,
        heavy: a.heavy || b.heavy,
        special: a.special || b.special,
        grab: a.grab || b.grab,
        dash: a.dash || b.dash,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn tick(
    mut game: ResMut<Game>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    menu: Res<Menu>,
    pads: Res<crate::pad::Pads>,
    gamepads: Query<&bevy::input::gamepad::Gamepad>,
    mut pad_state: Local<[crate::pad::PadState; 2]>,
) {
    let g = &mut *game;
    // each player's controller, read once per tick
    let mut pad_in = [Input::default(); 2];
    for i in 0..2 {
        if let Some(gp) = pads.players[i].and_then(|e| gamepads.get(e).ok()) {
            pad_in[i] = crate::pad::read(gp, &settings.pad, &mut pad_state[i]);
        }
    }
    g.stepped = false;
    if menu.is_open() || (g.frozen && !g.step_once) {
        return;
    }
    g.shown_ticks += 1;
    let p1_keys = merge(settings.p1_keys.read(&keys), pad_in[0]);
    // the battle intro: only the fighters' scripted "Start" states play; an attack button
    // skips to FIGHT!
    if g.intro > 0 {
        g.intro -= 1;
        if (p1_keys.light || p1_keys.heavy || p1_keys.special || p1_keys.grab) && g.intro > 30 && g.intro < INTRO_TICKS - 20 {
            g.intro = 30;
        }
        if g.world.in_intro() {
            g.world.step_intro();
            g.stepped = true;
        }
        if g.intro == 0 {
            g.world.end_intro();
            g.banner = 40;
        }
        return;
    }
    g.banner = g.banner.saturating_sub(1);
    if g.mode == Mode::Training && !g.step_once && !g.training.speed_tick(&settings) {
        return;
    }
    g.stepped = true;
    g.step_once = false;
    let p1 = if g.demo { demo_input(g.demo_ticks, &g.world) } else { p1_keys };
    // YEMI_DEMO_P2=block|crouch|jump|stand: how the demo's dummy behaves
    let dummy = match std::env::var("YEMI_DEMO_P2").ok().as_deref() {
        Some("block") if g.demo => P2Control::Block,
        Some("crouch") if g.demo => P2Control::CrouchBlock,
        Some("jump") if g.demo => P2Control::Jump,
        _ => g.p2,
    };
    // YEMI_DEMO_CPU=easy|normal|hard (demo videos): a CPU plays P1 too
    if g.demo && g.demo_cpu.is_none() {
        if let Ok(l) = std::env::var("YEMI_DEMO_CPU") {
            let level = match l.as_str() {
                "easy" => sim::cpu::Level::Easy,
                "hard" => sim::cpu::Level::Hard,
                _ => sim::cpu::Level::Normal,
            };
            g.demo_cpu = Some(sim::cpu::Cpu::new(&g.world, 0, level, 0xc0ffee));
        }
    }
    let p1 = match g.demo_cpu.as_mut() {
        Some(c) => c.input(&g.world),
        None => p1,
    };
    let p2 = match g.mode {
        Mode::Cpu(_) => {
            let w = &g.world;
            g.cpu.as_mut().map(|c| c.input(w)).unwrap_or_default()
        }
        Mode::Training => g.training.dummy_input(&g.world, &settings, p1),
        _ if g.p2 == P2Control::Human && !g.demo => merge(settings.p2_keys.read(&keys), pad_in[1]),
        // YEMI_DEMO_SEQ2: an input script for P2 too (demo videos)
        _ if g.demo && std::env::var_os("YEMI_DEMO_SEQ2").is_some() => seq_input(&std::env::var("YEMI_DEMO_SEQ2").unwrap_or_default(), g.demo_ticks, &g.world, 1),
        _ => dummy_input(dummy, &g.world),
    };
    // while recording the dummy, the player's keys drive it and P1 stands
    let p1 = if g.mode == Mode::Training && g.training.recording { Input::default() } else { p1 };
    // YEMI_DEMO_MOVES="0:grounded_punch,90:air_kick": on those ticks P1 starts that move by id,
    // 70 units from the dummy (move showcase videos; pair with YEMI_DEMO_RESET)
    if g.demo {
        if let Ok(list) = std::env::var("YEMI_DEMO_MOVES") {
            for (t, id) in list.split(',').filter_map(|p| p.split_once(':')) {
                if t.trim().parse::<u64>().ok() == Some(g.demo_ticks) {
                    let w = &mut g.world;
                    w.f[0].pos.x = sim::Fixed::from_int(-35);
                    w.f[1].pos.x = sim::Fixed::from_int(35);
                    w.f[0].facing = 1;
                    w.f[1].facing = -1;
                    w.force_move(0, id.trim());
                }
            }
        }
    }
    g.world.step([p1, p2]);
    g.demo_ticks += 1;
    if g.mode == Mode::Training {
        g.training.after_step(&mut g.world, &settings, p1);
    }
    // YEMI_DEMO_RESET=140,300: start a fresh round on those ticks (demo videos)
    if g.demo {
        if let Ok(list) = std::env::var("YEMI_DEMO_RESET") {
            if list.split(',').filter_map(|s| s.trim().parse::<u64>().ok()).any(|n| n == g.demo_ticks) {
                g.restart();
            }
        }
    }
    // YEMI_DEMO_METER=1 (demo videos): P1's meter stays full
    if g.demo && std::env::var_os("YEMI_DEMO_METER").is_some() {
        g.world.f[0].meter = g.world.cfg.combo.meter_max;
    }
    // YEMI_DEMO_REFILL=1 (demo videos): the dummy never runs out of health (=2: P1 neither)
    if g.demo {
        let both = std::env::var("YEMI_DEMO_REFILL").is_ok_and(|v| v == "2");
        for i in if both { 0..2 } else { 1..2 } {
            if std::env::var_os("YEMI_DEMO_REFILL").is_some() && g.world.f[i].hp < g.world.f[i].max_hp / 3 {
                g.world.f[i].hp = g.world.f[i].max_hp;
                g.world.winner = None;
            }
        }
    }
    if g.world.winner.is_some() || g.world.draw {
        g.over_frames += 1;
    }
}

/// Fight hotkeys and the pause menu; also opens the post-game menu once a round is over.
#[allow(clippy::too_many_arguments)]
pub fn fight_hotkeys(
    mut game: ResMut<Game>,
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<Settings>,
    mut menu: ResMut<Menu>,
    paths: Res<Paths>,
    pads: Res<crate::pad::Pads>,
    gamepads: Query<&bevy::input::gamepad::Gamepad>,
    mut atmo: ResMut<crate::atmosphere::Atmosphere>,
) {
    let g = &mut *game;
    // a player's Pause button opens the pause menu; Training Reset resets in training
    let pad_pressed = |action: usize| {
        settings.pad.bind(action).is_some_and(|b| pads.players.iter().flatten().filter_map(|e| gamepads.get(*e).ok()).any(|gp| gp.just_pressed(b)))
    };
    let pad_pause = pad_pressed(7);
    let pad_reset = pad_pressed(8);
    if pad_pause && !menu.is_open() && !menu.just_closed {
        menu.push(Page::Pause);
        return;
    }
    if pad_reset && !menu.is_open() && g.mode == Mode::Training {
        g.training_reset(&settings);
    }
    if g.demo {
        // YEMI_DEMO_PAUSE=120 (screenshots): the pause menu opens on that tick; the shot
        // counter keeps going while it is open
        if menu.is_open() {
            g.shown_ticks += 1;
        } else if std::env::var("YEMI_DEMO_PAUSE").ok().and_then(|v| v.parse::<u64>().ok()) == Some(g.shown_ticks) {
            menu.push(Page::Pause);
        }
    }
    if menu.is_open() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) && !menu.just_closed {
        menu.push(Page::Pause);
        return;
    }
    if g.over_frames == 90 && !g.demo {
        g.over_frames += 1;
        menu.push(Page::PostGame);
        return;
    }
    if keys.just_pressed(KeyCode::F8) {
        atmo.storm_weather = !atmo.storm_weather;
        g.status = format!("Storm Weather: {}", if atmo.storm_weather { "ON (Torrential Rain)" } else { "OFF" });
    }
    let h = settings.hotkeys.clone();
    if h.pause_sim.just_pressed(&keys) {
        g.frozen = !g.frozen;
    }
    if h.frame_advance.just_pressed(&keys) && g.frozen {
        g.step_once = true;
    }
    if h.dummy_mode.just_pressed(&keys) {
        g.p2 = g.p2.cycle(1);
        g.status = format!("P2: {}", g.p2.label());
    }
    if h.toggle_hitboxes.just_pressed(&keys) {
        let on = settings.flag("hitboxes");
        settings.set_flag("hitboxes", !on);
    }
    if h.toggle_hud.just_pressed(&keys) {
        g.hud_hidden = !g.hud_hidden;
    }
    if h.reset_round.just_pressed(&keys) {
        if g.mode == Mode::Training {
            g.training_reset(&settings);
        } else {
            g.restart();
        }
    }
    if g.mode == Mode::Training {
        if h.record_dummy.just_pressed(&keys) {
            let t = &mut g.training;
            t.recording = !t.recording;
            t.playing = None;
            if t.recording {
                t.tape.clear();
                g.status = format!("recording the dummy: your keys move P2 ({} to stop)", h.record_dummy.label());
            } else {
                g.status = format!("recorded {} ticks ({} plays it; or set Dummy to play recording)", t.tape.len(), h.play_recording.label());
            }
        }
        if h.play_recording.just_pressed(&keys) && !g.training.tape.is_empty() {
            g.training.recording = false;
            g.training.playing = Some(0);
        }
    }
    if h.reload_data.just_pressed(&keys) {
        match build_world(&paths, g.picks, &g.rules) {
            Ok(w) => {
                let n: usize = w.chars.iter().map(|c| c.warnings.len()).sum();
                g.world = w;
                g.over_frames = 0;
                g.status = format!("reloaded data ({n} validation warnings)");
            }
            Err(e) => g.status = format!("reload failed: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

/// A fighter's sprite color and outline, from its style (when style colors are on).
fn style_look(style: &Style, i: usize, settings: &Settings) -> ([u8; 3], Option<[u8; 3]>) {
    if settings.flag("style_colors") {
        (style.color, style.outline)
    } else {
        (crate::style::default_for(i).color, None)
    }
}

/// Ticks into the intro, while it plays.
fn intro_tick(g: &Game) -> Option<u32> {
    (g.intro > 0).then(|| INTRO_TICKS - g.intro)
}

fn act_color(i: usize, act: &Act) -> Color {
    let base = if i == 0 { Vec3::new(0.35, 0.75, 1.0) } else { Vec3::new(1.0, 0.55, 0.35) };
    let c = match act {
        Act::Hurt { .. } => Vec3::new(1.0, 0.25, 0.25),
        Act::Block { .. } => Vec3::new(0.55, 0.55, 1.0),
        Act::Held => Vec3::new(0.8, 0.4, 1.0),
        Act::Knockdown { .. } | Act::Getup { .. } => base * 0.5,
        Act::Ko => Vec3::splat(0.35),
        Act::Move(_) => base * 1.15,
        _ => base * 0.8,
    };
    Color::srgb(c.x.min(1.0), c.y.min(1.0), c.z.min(1.0))
}

pub fn draw(
    mut game: ResMut<Game>,
    settings: Res<Settings>,
    shake: Res<crate::fx::Shake>,
    atmo: Res<crate::atmosphere::Atmosphere>,
    mut gizmos: Gizmos,
    mut bodies: Query<(&Body, &mut Sprite, &mut Transform, &mut Visibility), (Without<Nose>, Without<Art>, Without<ProjArt>, Without<OverlayArt>)>,
    mut noses: Query<(&Nose, &mut Sprite, &mut Transform, &mut Visibility), (Without<Body>, Without<Art>, Without<ProjArt>, Without<OverlayArt>)>,
    mut arts: Query<(&Art, &mut Sprite, &mut Transform, &mut Visibility), (Without<Body>, Without<Nose>, Without<ProjArt>, Without<OverlayArt>)>,
    mut proj_arts: Query<(&ProjArt, &mut Sprite, &mut Transform, &mut Visibility), (Without<Body>, Without<Nose>, Without<Art>, Without<OverlayArt>)>,
    mut overlay_arts: Query<(&OverlayArt, &mut Sprite, &mut Transform, &mut Visibility), (Without<Body>, Without<Nose>, Without<Art>, Without<ProjArt>)>,
    mut library: ResMut<crate::fx::FxLibrary>,
    mut art: ResMut<crate::art::Art>,
    mut images: ResMut<Assets<Image>>,
) {
    let w = &game.world;
    let zoom = ZOOM * (1.0 + atmo.zoom_kick);
    let stage = w.cfg.stage_half_width as f32;
    let vis = HALF_W_PX / zoom;
    let mid = (w.f[0].pos.x.to_f32() + w.f[1].pos.x.to_f32()) / 2.0;
    let target = if stage > vis { mid.clamp(-stage + vis, stage - vis) } else { 0.0 };
    let cam = game.cam_x + (target - game.cam_x) * 0.2;
    game.cam_x = cam;
    let w = &game.world;
    let sh = shake.offset;
    let to_screen = |x: f32, y: f32| Vec2::new((x - cam + sh.x) * zoom, GROUND_Y - (y + sh.y) * zoom);

    // floor and walls
    gizmos.line_2d(to_screen(-stage, 0.0), to_screen(stage, 0.0), Color::srgb(0.45, 0.47, 0.5));
    for wall in [-stage, stage] {
        gizmos.line_2d(to_screen(wall, 0.0), to_screen(wall, -300.0), Color::srgb(0.3, 0.3, 0.33));
    }

    // real sprites: one per fighter, when the library has this character's scene
    let mut drawn = [false; 2];
    let dirs = library.dirs();
    for (a, mut sprite, mut tf, mut vis) in &mut arts {
        let i = a.0;
        *vis = Visibility::Hidden;
        if dirs.is_empty() {
            continue;
        }
        let Some(ca) = art.character(&dirs, &w.chars[i].file.character.id) else { continue };
        let offset = ca.offset;
        let Some(path) = crate::art::frame_path(ca, w, i).map(str::to_string) else { continue };
        let (tint, outline) = style_look(&game.styles[i], i, &settings);
        // during the intro the fighter strikes its intro pose (or taunt), unless its scripted
        // "Start" state is playing
        let path = match intro_tick(&game) {
            Some(t) if !matches!(w.f[i].act, Act::Move(_)) => crate::art::intro_frame(ca, t).map(str::to_string).unwrap_or(path),
            _ => path,
        };
        // what the fighter's scripts did to its sprite (hidden, a frame they picked, turned...)
        let look = w.fighter_sprite(i);
        drawn[i] = true;
        if look.as_ref().is_some_and(|l| !l.visible) {
            continue;
        }
        let path = match look.as_ref().filter(|l| l.anim.is_some() || l.frame.is_some()) {
            Some(l) => crate::art::picked_frame(ca, w, i, l.anim.as_deref(), l.frame).map(str::to_string).unwrap_or(path),
            None => path,
        };
        let Some((image, _)) = library.texture_styled(&path, tint, outline, &mut images) else { continue };
        let fac = w.f[i].facing as f32;
        sprite.image = image;
        sprite.flip_x = (w.f[i].facing < 0) ^ look.as_ref().is_some_and(|l| l.flip_h);
        let c = look.as_ref().map(|l| l.modulate).unwrap_or([1.0; 4]);
        sprite.color = Color::srgba(c[0], c[1], c[2], c[3]);
        let (shift, scale, rot) = look.as_ref().map(|l| (l.shift, l.scale, l.rotation)).unwrap_or(((0.0, 0.0), (1.0, 1.0), 0.0));
        let feet = w.f[i].pos;
        let x = feet.x.to_f32() + (offset.x + shift.0) * fac;
        tf.translation = to_screen(x, feet.y.to_f32() + offset.y + shift.1).extend(0.5);
        tf.scale = Vec3::new(scale.0 * zoom, scale.1 * zoom, 1.0);
        // Godot turns clockwise on screen; mirrored with the facing
        tf.rotation = Quat::from_rotation_z(-rot * fac);
        *vis = Visibility::Inherited;
    }

    // like the original's shader: pure-white pixels take the player's color (imported fighters too)
    let tint_of = |i: usize| style_look(&game.styles[i], i, &settings).0;
    // projectiles: the owner's sprite scene, one pooled sprite per live projectile
    let game_dir = (!dirs.is_empty()).then_some(&dirs);
    for (pa, mut sprite, mut tf, mut vis) in &mut proj_arts {
        *vis = Visibility::Hidden;
        let (Some(p), Some(game)) = (w.projs.get(pa.0), game_dir) else { continue };
        if p.is_object() {
            continue; // scripted objects are drawn from their scene (see `draw_objects`)
        }
        let owner = &w.chars[p.owner].file.character;
        let Some(def) = owner.projectiles.get(p.idx) else { continue };
        let Some(ca) = art.projectile(game, &owner.id, &def.id) else { continue };
        let offset = ca.offset;
        let Some(path) = crate::art::projectile_frame(ca, p.age).map(str::to_string) else { continue };
        let Some((image, _)) = library.texture_tinted(&path, tint_of(p.owner), &mut images) else { continue };
        sprite.image = image;
        sprite.flip_x = p.facing < 0;
        tf.translation = to_screen(p.pos.x.to_f32() + offset.x * p.facing as f32, p.pos.y.to_f32() + offset.y).extend(0.6);
        tf.scale = Vec3::splat(zoom);
        *vis = Visibility::Inherited;
    }
    // overlays: sprites the original's move scripts show on top of the fighter
    for (oa, mut sprite, mut tf, mut vis) in &mut overlay_arts {
        let i = oa.0;
        *vis = Visibility::Hidden;
        let Act::Move(run) = &w.f[i].act else { continue };
        let ch = &w.chars[i].file.character;
        let Some(mv) = ch.moves.get(run.idx) else { continue };
        let Some(game) = game_dir else { continue };
        let Some(ca) = art.character(game, &ch.id) else { continue };
        let Some(u) = crate::art::overlay_for(&ch.id, &mv.id, run.frame) else { continue };
        let Some((o, path)) = ca.overlay_path(u.node, u.frame) else { continue };
        let (pos, off, z) = (o.pos, o.offset, o.z);
        let path = path.to_string();
        let Some((image, _)) = library.texture_tinted(&path, tint_of(i), &mut images) else { continue };
        let fac = w.f[i].facing as f32;
        sprite.image = image;
        sprite.flip_x = fac < 0.0;
        // the pivot is `pos` from the feet; the image hangs `off` from the pivot, turned by the rotation
        let turn = Quat::from_rotation_z(-u.rotation * fac);
        let hang = (turn * Vec3::new(off.x * fac, -off.y, 0.0)).truncate();
        let feet = w.f[i].pos;
        let at = to_screen(feet.x.to_f32() + pos.x * fac, feet.y.to_f32() + pos.y);
        tf.translation = (at + hang * zoom).extend(0.5 + z * 0.01 + if z < 0.0 { -0.2 } else { 0.2 });
        tf.rotation = turn;
        tf.scale = Vec3::splat(zoom);
        *vis = Visibility::Inherited;
    }

    for (b, mut sprite, mut tf, mut vis) in &mut bodies {
        let i = b.0;
        *vis = if drawn[i] { Visibility::Hidden } else { Visibility::Inherited };
        let (c, h) = w.hurtboxes(i)[0];
        sprite.custom_size = Some(Vec2::new(h.x.to_f32() * 2.0, h.y.to_f32() * 2.0) * zoom);
        sprite.color = act_color(i, &w.f[i].act);
        tf.translation = to_screen(c.x.to_f32(), c.y.to_f32()).extend(0.0);
    }
    for (n, mut sprite, mut tf, mut vis) in &mut noses {
        let i = n.0;
        *vis = if drawn[i] { Visibility::Hidden } else { Visibility::Inherited };
        let (c, h) = w.hurtboxes(i)[0];
        let f = &w.f[i];
        let x = c.x.to_f32() + f.facing as f32 * (h.x.to_f32() - 3.0);
        let y = c.y.to_f32() - h.y.to_f32() * 0.55;
        sprite.color = Color::srgb(0.95, 0.95, 0.95);
        tf.translation = to_screen(x, y).extend(1.0);
    }

    if settings.flag("hitboxes") || (game.mode == Mode::Training && settings.train("show_hitboxes") > 0.5) {
        for i in 0..2 {
            for (c, h) in w.hurtboxes(i) {
                gizmos.rect_2d(to_screen(c.x.to_f32(), c.y.to_f32()), Vec2::new(h.x.to_f32(), h.y.to_f32()) * 2.0 * zoom, Color::srgb(0.3, 0.95, 0.4));
            }
            for (c, h, hb, _) in w.hitboxes(i) {
                let color = if hb.effects.throw {
                    Color::srgb(1.0, 0.9, 0.2)
                } else if hb.kind != charfmt::HitKind::Normal {
                    Color::srgb(1.0, 0.5, 1.0)
                } else {
                    Color::srgb(1.0, 0.2, 0.2)
                };
                gizmos.rect_2d(to_screen(c.x.to_f32(), c.y.to_f32()), Vec2::new(h.x.to_f32(), h.y.to_f32()) * 2.0 * zoom, color);
            }
        }
    }
}

pub fn hud(
    game: Res<Game>,
    settings: Res<Settings>,
    mut texts: Query<(&Hud, &mut Text, &mut TextColor)>,
    mut fills: Query<(&HealthFill, &mut Node), Without<MeterFill>>,
    mut meters: Query<(&MeterFill, &mut Node, &mut BackgroundColor), Without<HealthFill>>,
) {
    let w = &game.world;
    for (fill, mut node) in &mut fills {
        let f = &w.f[fill.0];
        node.width = percent(100.0 * f.hp.max(0) as f32 / f.max_hp.max(1) as f32);
    }
    let rules = &w.cfg.combo;
    for (m, mut node, mut bg) in &mut meters {
        let meter = w.f[m.0].meter;
        node.width = percent(100.0 * meter as f32 / rules.meter_max.max(1) as f32);
        // brighter once a Hustle Cancel is paid for, white at a full super bar
        bg.0 = if meter >= rules.super_bar { Color::srgb(1.0, 1.0, 0.85) } else if meter >= rules.hustle_cancel { Color::srgb(1.0, 0.85, 0.2) } else { Color::srgb(0.55, 0.45, 0.15) };
    }
    let hidden = game.hud_hidden;
    for (kind, mut t, mut color) in &mut texts {
        t.0 = match kind {
            _ if hidden => String::new(),
            Hud::Player(i) => {
                let i = *i;
                let f = &w.f[i];
                let ch = &w.chars[i].file.character.name;
                let who = if i == 0 {
                    color.0 = if settings.name_saturation > 0.0 { settings.name_color() } else { Color::WHITE };
                    if settings.name.is_empty() { format!("P1 - {ch}") } else { format!("P1 {} - {ch}", settings.name) }
                } else {
                    format!("P2 ({}) - {ch}", game.p2_label(&settings))
                };
                let hp = if settings.flag("precise_health") { format!("\nHP {} / {}", f.hp, f.max_hp) } else { String::new() };
                if !settings.flag("extra_info") {
                    format!("{who}{hp}")
                } else {
                    let mv = w
                        .move_name(i)
                        .map(|(id, fr, total)| format!("{id}  frame {fr}/{total}"))
                        .unwrap_or_else(|| f.act.label().to_string());
                    let mode = w.mode_name(i).unwrap_or("-");
                    format!(
                        "{who}{hp}\n{mv}\nmode {mode}   hits taken in combo {}   last combo {} dmg\nhitlag {}  {}",
                        f.combo_hits,
                        f.combo_damage,
                        f.hitlag,
                        if f.grounded { "ground" } else { "air" },
                    )
                }
            }
            Hud::Timer => w.time_left().map(|t| format!("{}", t.div_ceil(60))).unwrap_or_default(),
            Hud::Log => {
                let mut s = if settings.flag("extra_info") { w.log.join("\n") } else { String::new() };
                if settings.show_frame_advantage {
                    if let Some(a) = &w.last_advantage {
                        s.push_str(&format!(
                            "\nlast exchange: P{} {} {:+} frames",
                            a.attacker + 1,
                            if a.on_block { "on block" } else { "on hit" },
                            a.frames
                        ));
                    }
                }
                if w.draw {
                    s.push_str("\nTIME - draw.");
                } else if let Some(win) = w.winner {
                    s.push_str(&format!("\nP{} wins.", win + 1));
                }
                if game.frozen {
                    s.push_str(&format!("\nFROZEN ({} = step one frame)", settings.hotkeys.frame_advance.label()));
                }
                if !game.status.is_empty() {
                    s.push_str(&format!("\n{}", game.status));
                }
                s
            }
            Hud::Banner => {
                let name = |i: usize| w.chars[i].file.character.name.to_uppercase();
                if let Some(t) = intro_tick(&game) {
                    color.0 = Color::WHITE;
                    if t < 85 {
                        format!("{}   VS   {}", name(0), name(1))
                    } else if game.intro > 35 {
                        "READY".into()
                    } else {
                        color.0 = Color::srgb(1.0, 0.85, 0.2);
                        "FIGHT!".into()
                    }
                } else if game.banner > 0 {
                    color.0 = Color::srgb(1.0, 0.85, 0.2);
                    "FIGHT!".into()
                } else if game.mode != Mode::Training && game.over_frames > 0 && game.over_frames < 90 {
                    color.0 = Color::srgb(1.0, 0.3, 0.25);
                    if w.draw {
                        "DRAW".into()
                    } else if w.time_left() == Some(0) {
                        "TIME".into()
                    } else {
                        "K.O.".into()
                    }
                } else {
                    String::new()
                }
            }
            Hud::Training if game.mode == Mode::Training => game.training.panel(w, &settings),
            Hud::Training => String::new(),
            Hud::Help if !settings.show_controls_bar => String::new(),
            Hud::Help => {
                let p = &settings.p1_keys;
                let q = &settings.p2_keys;
                let h = &settings.hotkeys;
                format!(
                    "P1: {}/{}/{}/{} move/jump/crouch  {} light  {} heavy  {} special  {} grab  {} dash  | hold back to block   \
                     P2: {}/{}/{}/{} + {} {} {} {} {}\n\
                     Esc menu   {} P2 mode   {} boxes   {} freeze   {} step   {} reset   {} reload data   {} HUD   {} fullscreen",
                    p.left.label(), p.right.label(), p.up.label(), p.down.label(), p.light.label(), p.heavy.label(),
                    p.special.label(), p.grab.label(), p.dash.label(),
                    q.left.label(), q.right.label(), q.up.label(), q.down.label(), q.light.label(), q.heavy.label(),
                    q.special.label(), q.grab.label(), q.dash.label(),
                    h.dummy_mode.label(), h.toggle_hitboxes.label(), h.pause_sim.label(), h.frame_advance.label(),
                    h.reset_round.label(), h.reload_data.label(), h.toggle_hud.label(), h.toggle_fullscreen.label(),
                )
            }
        };
    }
}

/// Picks for `--p1` / `--p2` by roster name.
pub fn roster_index(ids: &[String], name: &str) -> Option<usize> {
    ids.iter().position(|r| r == name)
}

// ---------------------------------------------------------------------------------------------
// Sounds and effects
// ---------------------------------------------------------------------------------------------

/// What each fighter was doing after the previous step, to notice what just started.
#[derive(Default)]
pub struct FxTrack {
    prev: [Option<Act>; 2],
    air_actions: [u8; 2],
}

fn v2(v: sim::FVec) -> Vec2 {
    Vec2::new(v.x.to_f32(), v.y.to_f32())
}

/// Turns the step's events and state changes into effects and sounds, the way the original
/// triggers them (hit sparks and hit sounds per hitbox, a state's enter sound and particle, ...).
#[allow(clippy::too_many_arguments)]
pub fn fight_fx(
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut lib: ResMut<crate::fx::FxLibrary>,
    mut images: ResMut<Assets<Image>>,
    mut pcm: ResMut<Assets<crate::fx::Pcm>>,
    settings: Res<Settings>,
    mut shake: ResMut<crate::fx::Shake>,
    mut track: Local<FxTrack>,
    pads: Res<crate::pad::Pads>,
    mut rumble: MessageWriter<bevy::input::gamepad::GamepadRumbleRequest>,
    mut atmo: ResMut<crate::atmosphere::Atmosphere>,
) {
    use crate::fx::{play, spawn_effect, MoveFx};
    if game.intro_sound && lib.available() {
        // each fighter's intro (or taunt) sound as the intro starts
        game.intro_sound = false;
        for i in 0..2 {
            if matches!(game.world.f[i].act, Act::Move(_)) {
                continue; // its "Start" state plays its own sounds
            }
            let id = game.world.chars[i].file.character.id.clone();
            let m = ["intro", "start", "cross_intro", "taunt"].iter().find_map(|k| lib.chars.get(&id).and_then(|c| c.moves.get(*k)).cloned());
            if let Some(s) = m.and_then(|m| m.enter_sfx.or(m.sfx.map(|s| crate::fx::Sound { path: s.path, volume: s.volume }))) {
                play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.0);
            }
        }
    }
    if !game.stepped || !lib.available() {
        return;
    }
    let game = game.into_inner();
    let w = &game.world;
    let mut spawn = |commands: &mut Commands, lib: &mut crate::fx::FxLibrary, scene: &str, pos: Vec2, dir: Vec2| {
        spawn_effect(commands, lib, &mut images, scene, pos, dir);
    };
    let move_fx = |lib: &crate::fx::FxLibrary, i: usize, id: &str| -> Option<MoveFx> {
        lib.chars.get(&w.chars[i].file.character.id).and_then(|c| c.moves.get(id)).cloned()
    };
    // a state's enter sound and on-enter particle
    let enter = |commands: &mut Commands, lib: &mut crate::fx::FxLibrary, pcm: &mut Assets<crate::fx::Pcm>, i: usize, m: &MoveFx, spawn: &mut dyn FnMut(&mut Commands, &mut crate::fx::FxLibrary, &str, Vec2, Vec2)| {
        let f = &w.f[i];
        let facing = f.facing as f32;
        if let Some(s) = &m.enter_sfx {
            play(commands, lib, pcm, &settings, &s.path, s.volume, 1.0, 0.1);
        }
        if let Some(p) = &m.particle {
            let at = v2(f.pos) + Vec2::new(p.pos.0 * facing, p.pos.1);
            spawn(commands, lib, &p.scene, at, Vec2::new(facing, 0.0));
        }
    };

    for i in 0..2 {
        let f = &w.f[i];
        let facing = f.facing as f32;
        let prev = track.prev[i].take();
        let hurt_center = w.hurtboxes(i).first().map(|h| v2(h.0)).unwrap_or(v2(f.pos));
        if let Act::Move(run) = &f.act {
            let ch = &w.chars[i];
            let mv = ch.mv(run.idx);
            let same = matches!(&prev, Some(Act::Move(p)) if p.idx == run.idx && p.frame <= run.frame);
            let mfx = move_fx(&lib, i, &mv.id).unwrap_or_default();
            if !same {
                enter(&mut commands, &mut lib, &mut pcm, i, &mfx, &mut spawn);
            }
            let from = match &prev {
                Some(Act::Move(p)) if same => p.frame + 1,
                _ => 1,
            };
            for fr in from..=run.frame {
                if let Some(s) = mfx.sfx.as_ref().filter(|s| s.tick.max(1) == fr) {
                    play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                }
                if let Some(p) = mfx.timed_particle.as_ref().filter(|p| p.tick.max(1) == fr) {
                    let at = v2(f.pos) + Vec2::new(p.pos.0 * facing, p.pos.1);
                    spawn(&mut commands, &mut lib, &p.scene, at, Vec2::new(facing, 0.0));
                }
                if let (Some(s), Some(t)) = (&mfx.release_sfx, &mv.throw) {
                    if t.release.as_ref().is_some_and(|r| r.at == fr) {
                        play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                    }
                }
                // a hitbox coming out plays its whiff sound (and whiff particle)
                for hb in mv.hitboxes.iter().filter(|h| h.window.from == fr) {
                    let hfx = mfx.hitboxes.get(&hb.id).cloned().unwrap_or_else(default_hitbox_fx);
                    if let Some(s) = &hfx.whiff_sound {
                        play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                    }
                    if hfx.bass_on_whiff {
                        if let Some(s) = &hfx.bass_sound {
                            play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                        }
                    }
                    if let Some(scene) = &hfx.whiff_particle {
                        let at = v2(f.pos) + Vec2::new(hb.rect.x as f32 * facing, hb.rect.y as f32);
                        spawn(&mut commands, &mut lib, scene, at, Vec2::new(facing, 0.0));
                    }
                }
            }
        } else {
            // engine-owned states use the original's shared states' sounds and particles
            let started = |pred: fn(&Act) -> bool| pred(&f.act) && !prev.as_ref().is_some_and(pred);
            let mut state = None;
            if matches!(f.act, Act::Air) && matches!(prev, Some(Act::JumpSquat { .. })) {
                state = Some("jump");
            } else if matches!(f.act, Act::Air) && f.air_actions < track.air_actions[i] {
                state = Some("double_jump");
            } else if started(|a| matches!(a, Act::Landing { .. })) {
                state = Some("landing");
            } else if started(|a| matches!(a, Act::Knockdown { .. })) {
                state = Some("knockdown");
            } else if started(|a| matches!(a, Act::Getup { .. })) {
                state = Some("getup");
            } else if started(|a| matches!(a, Act::Dash { .. })) {
                let Act::Dash { dir, .. } = f.act else { unreachable!() };
                state = Some(if dir as f32 * facing >= 0.0 { "dash_forward" } else { "dash_backward" });
                // Dash.gd adds a burst of dust at the hurtbox
                spawn(&mut commands, &mut lib, "fx/DashParticle.tscn", hurt_center, Vec2::new(dir as f32, 0.0));
            }
            if let Some(id) = state {
                if let Some(m) = move_fx(&lib, i, id) {
                    enter(&mut commands, &mut lib, &mut pcm, i, &m, &mut spawn);
                }
            }
            // smoke trails behind fighters knocked away fast (CharacterHurtState)
            if matches!(f.act, Act::Hurt { launched: true, .. }) && f.vel.x.to_f32().hypot(f.vel.y.to_f32()) > 6.5 {
                spawn(&mut commands, &mut lib, "fx/KnockbackSmoke.tscn", hurt_center, Vec2::new(facing, 0.0));
            }
        }
        track.air_actions[i] = f.air_actions;
        track.prev[i] = Some(f.act.clone());
    }

    for ev in &w.events {
        match ev {
            sim::Event::Hit { attacker, victim, mv, hitbox, pos, dir, damage } => {
                if settings.pad.rumble {
                    // the one hit feels it more than the one hitting, both more on big hits
                    let k = (*damage as f32 / 120.0).clamp(0.25, 1.0);
                    crate::pad::rumble(&mut rumble, &pads, *victim, 0.8 * k, 0.5 * k, 90 + (*damage as u64).min(200) / 2);
                    crate::pad::rumble(&mut rumble, &pads, *attacker, 0.0, 0.45 * k, 60);
                }
                let id = &w.chars[*attacker].mv(*mv).id;
                let hfx = move_fx(&lib, *attacker, id).and_then(|m| m.hitboxes.get(hitbox).cloned()).unwrap_or_else(default_hitbox_fx);
                let (p, d) = (v2(*pos), v2(*dir));
                if let Some(scene) = &hfx.hit_particle {
                    spawn(&mut commands, &mut lib, scene, p, d);
                }
                if !hfx.replace_hit_particle {
                    // the attacker's style picks the spark
                    let spark = if settings.flag("hitsparks") { crate::style::hitspark_scene(&game.styles[*attacker].hitspark) } else { "fx/HitEffect1.tscn" };
                    spawn(&mut commands, &mut lib, spark, p, d);
                }
                if let Some(s) = &hfx.hit_sound {
                    play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                }
                if !hfx.bass_on_whiff {
                    if let Some(s) = &hfx.bass_sound {
                        play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                    }
                }
                let frames = if hfx.screenshake_frames < 0 { w.f[*victim].hitlag as f32 } else { hfx.screenshake_frames as f32 };
                shake.bump(hfx.screenshake as f32, frames, Vec2::new(hfx.bump_dir.0, hfx.bump_dir.1));

                // Visual atmosphere punch (zoom kick, impact frames, speedlines burst)
                // In reference (yomi_recovered):
                // - Hits DO NOT flash the entire screen white.
                // - Normal hits DO NOT kick camera zoom (combos stay visually readable).
                // - Speedlines DO NOT blast on every hit (only when launched fast through air).
                // Normalized weights:
                if w.f[*victim].hp <= 0 {
                    // Match-ending lethal blow
                    atmo.trigger_impact(2, Color::WHITE);
                    atmo.trigger_zoom_kick(0.20);
                } else if *damage >= 110 {
                    // Heavy strike micro-punch
                    atmo.trigger_zoom_kick(0.05);
                }

                // Speedline burst only on high-velocity launch knockback
                let launch_speed = Vec2::new(d.x, d.y).length();
                let is_launch = matches!(w.f[*victim].act, sim::Act::Hurt { launched: true, .. }) || launch_speed > 8.0;
                if is_launch && *damage >= 100 {
                    let launch_dir = Vec2::new(d.x, -d.y);
                    let intensity = (*damage as f32 / 240.0).clamp(0.2, 0.55);
                    atmo.burst_lines(launch_dir, (launch_speed / 7.0).clamp(1.0, 2.2), intensity);
                }
            }
            sim::Event::Block { attacker, pos, .. } => {
                // BaseChar: a blocked hit shows the flawed-parry flash with two block sounds
                spawn(&mut commands, &mut lib, "fx/FlawedParryEffect.tscn", v2(*pos), Vec2::new(w.f[*attacker].facing as f32, 0.0));
                for name in ["Block2", "Block3"] {
                    if let Some((path, db, pitch)) = lib.base_sound(name) {
                        play(&mut commands, &mut lib, &mut pcm, &settings, &path, db, pitch, 0.0);
                    }
                }
            }
            sim::Event::Grab { attacker, mv, hitbox, .. } => {
                let id = &w.chars[*attacker].mv(*mv).id;
                let hfx = move_fx(&lib, *attacker, id).and_then(|m| m.hitboxes.get(hitbox).cloned()).unwrap_or_else(default_hitbox_fx);
                if let Some(s) = &hfx.hit_sound {
                    play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                }
            }
            sim::Event::ThrowTech => {
                for i in 0..2 {
                    if let Some(m) = move_fx(&lib, i, "throw_tech") {
                        enter(&mut commands, &mut lib, &mut pcm, i, &m, &mut spawn);
                        break;
                    }
                }
            }
            sim::Event::Ko { .. } => {
                atmo.trigger_impact(2, Color::WHITE);
                atmo.trigger_zoom_kick(0.22);
            }
            sim::Event::ObjectHit { attacker, mv, hitbox, pos, dir, damage } => {
                // a struck summon or domain: the hit's spark and sound, without the victim's rumble
                let hfx = mv.and_then(|m| move_fx(&lib, *attacker, &w.chars[*attacker].mv(m).id)).and_then(|m| m.hitboxes.get(hitbox).cloned()).unwrap_or_else(default_hitbox_fx);
                let (p, d) = (v2(*pos), Vec2::new(dir.0, dir.1));
                let spark = if settings.flag("hitsparks") { crate::style::hitspark_scene(&game.styles[*attacker].hitspark) } else { "fx/HitEffect1.tscn" };
                spawn(&mut commands, &mut lib, hfx.hit_particle.as_deref().unwrap_or(spark), p, d);
                if let Some(s) = &hfx.hit_sound {
                    play(&mut commands, &mut lib, &mut pcm, &settings, &s.path, s.volume, 1.0, 0.1);
                }
                if settings.pad.rumble {
                    crate::pad::rumble(&mut rumble, &pads, *attacker, 0.0, 0.4 * (*damage as f32 / 120.0).clamp(0.25, 1.0), 60);
                }
                shake.bump(hfx.screenshake as f32, 4.0, Vec2::ZERO);
            }
            sim::Event::Effect { scene, pos, dir } => {
                spawn(&mut commands, &mut lib, scene, v2(*pos), Vec2::new(dir.0, dir.1));
            }
            sim::Event::Sound { path, volume } => {
                play(&mut commands, &mut lib, &mut pcm, &settings, path, *volume, 1.0, 0.05);
            }
            sim::Event::HustleCancel { who, pos } => {
                // the original's instant-cancel flash, at the fighter's middle
                let p = v2(*pos) + Vec2::new(0.0, -16.0);
                spawn(&mut commands, &mut lib, "fx/InstantCancelEffect.tscn", p, Vec2::new(w.f[*who].facing as f32, 0.0));
                shake.bump(2.0, 4.0, Vec2::ZERO);
                atmo.trigger_super_dim(0.30);
                atmo.trigger_zoom_kick(0.06);
            }
        }
    }
}

/// Hitbox.gd's defaults, for hitboxes the data does not list.
fn default_hitbox_fx() -> crate::fx::HitboxFx {
    use crate::fx::Sound;
    crate::fx::HitboxFx {
        hit_sound: Some(Sound { path: "sound/common/hit1.wav".into(), volume: -5.0 }),
        whiff_sound: Some(Sound { path: "sound/common/whiff1.wav".into(), volume: -8.0 }),
        bass_sound: Some(Sound { path: "sound/common/hit_bass.wav".into(), volume: -5.0 }),
        bass_on_whiff: false,
        hit_particle: None,
        replace_hit_particle: false,
        whiff_particle: None,
        screenshake: 4,
        screenshake_frames: -1,
        bump_dir: (0.0, 0.0),
    }
}

/// Each fighter's style auras emit around its body (fixed tick, while the fight or intro runs).
pub fn fight_auras(
    mut commands: Commands,
    game: Option<Res<Game>>,
    settings: Res<Settings>,
    shapes: Option<Res<crate::aura::AuraShapes>>,
    mut emitters: ResMut<crate::aura::Emitters>,
) {
    let (Some(game), Some(shapes)) = (game, shapes) else { return };
    if !settings.flag("auras") || !(game.stepped || game.intro > 0) {
        return;
    }
    let w = &game.world;
    for i in 0..2 {
        let f = &w.f[i];
        let (c, h) = w.hurtboxes(i)[0];
        let body = crate::aura::Body {
            feet: v2(f.pos),
            vel: v2(f.vel),
            facing: f.facing as f32,
            center: v2(c),
            half: v2(h),
        };
        emitters.emit(&mut commands, &shapes, i, &game.styles[i], &body, crate::aura::Space::Fight, Some(DespawnOnExit(AppState::Fight)));
    }
}

/// A pooled sprite for a scripted object's node, or a UI-layer node (cut-ins).
#[derive(Component)]
pub struct NodeArt;

/// Draws scripted objects (every visible sprite of their scene, the main one in its state's
/// animation) and the nodes scripts put on the UI layer, in the owner's style color.
#[allow(clippy::too_many_arguments)]
/// Particle nodes of the fighters' and objects' scenes that scripts show (Gojo's void, auras):
/// each runs as a lasting effect that follows its owner, until a script hides it.
pub fn scene_particles(
    mut commands: Commands,
    game: Res<Game>,
    mut library: ResMut<crate::fx::FxLibrary>,
    mut images: ResMut<Assets<Image>>,
    mut fxs: Query<&mut crate::fx::ActiveFx>,
    mut live: Local<std::collections::HashMap<String, Entity>>,
) {
    let w = &game.world;
    // (key, scene, node path, position, facing)
    let mut wanted: Vec<(String, String, String, Vec2, f32)> = Vec::new();
    let mut add = |key: String, v: &sim::NodeView, at: sim::FVec, facing: i64| {
        if v.ty != "CPUParticles2D" {
            return;
        }
        let sx = if v.flipped && facing < 0 { -1.0 } else { 1.0 };
        let pos = Vec2::new(at.x.to_f32() + v.pos.0 * sx, at.y.to_f32() + v.pos.1);
        wanted.push((format!("{key}:{}", v.path), v.scene.clone(), v.path.clone(), pos, if v.flipped { facing as f32 } else { 1.0 }));
    };
    for i in 0..2 {
        for v in w.fighter_nodes(i) {
            add(format!("f{i}"), &v, w.f[i].pos, w.f[i].facing);
        }
    }
    for p in w.projs.iter().filter(|p| p.is_object()) {
        for v in w.object_nodes(p.uid) {
            add(format!("o{}", p.uid), &v, p.pos, p.facing);
        }
    }
    let keep: std::collections::HashSet<String> = wanted.iter().map(|x| x.0.clone()).collect();
    live.retain(|k, e| {
        let ok = keep.contains(k);
        if !ok {
            commands.entity(*e).despawn();
        }
        ok
    });
    for (key, scene, node, pos, facing) in wanted {
        match live.get(&key) {
            Some(e) => {
                if let Ok(mut fx) = fxs.get_mut(*e) {
                    crate::fx::place_effect(&mut fx, pos, facing);
                }
            }
            None => {
                if let Some(e) = crate::fx::spawn_node_effect(&mut commands, &mut library, &mut images, &scene, &node, pos, facing) {
                    live.insert(key, e);
                }
            }
        }
    }
}

pub fn draw_objects(
    mut commands: Commands,
    game: Res<Game>,
    settings: Res<Settings>,
    shake: Res<crate::fx::Shake>,
    atmo: Res<crate::atmosphere::Atmosphere>,
    scale: Res<UiScale>,
    mut pool: Query<(&mut Sprite, &mut Transform, &mut Visibility), With<NodeArt>>,
    mut library: ResMut<crate::fx::FxLibrary>,
    mut art: ResMut<crate::art::Art>,
    mut images: ResMut<Assets<Image>>,
) {
    let w = &game.world;
    let dirs = library.dirs();
    let cam = game.cam_x;
    let sh = shake.offset;
    let zoom = ZOOM * (1.0 + atmo.zoom_kick);
    let to_screen = |x: f32, y: f32| Vec2::new((x - cam + sh.x) * zoom, GROUND_Y - (y + sh.y) * zoom);
    // (image, world position, size scale, flip, color, z, rotation)
    let mut draws: Vec<(Handle<Image>, Vec2, Vec2, bool, Color, f32, f32)> = Vec::new();
    let mut frame_of = |v: &sim::NodeView, library: &mut crate::fx::FxLibrary, images: &mut Assets<Image>| -> Option<Handle<Image>> {
        let tint = style_look(&game.styles[v.player.min(1)], v.player.min(1), &settings).0;
        let path = match &v.texture {
            Some(t) if v.ty == "Sprite" => t.clone(),
            _ => {
                let anims = art.node(&dirs, &v.scene, &v.path)?;
                let frames = anims.get(&v.anim).or_else(|| anims.get("default")).or_else(|| anims.values().next())?;
                frames.get((v.frame as usize).min(frames.len().saturating_sub(1)))?.clone()
            }
        };
        library.texture_tinted(&path, tint, images).map(|t| t.0)
    };
    if !dirs.is_empty() {
        for p in w.projs.iter().filter(|p| p.is_object()) {
            let fac = p.facing as f32;
            for v in w.object_nodes(p.uid) {
                let Some(img) = frame_of(&v, &mut library, &mut images) else { continue };
                let mirror = v.flipped && fac < 0.0;
                let sx = if mirror { -1.0 } else { 1.0 };
                let x = p.pos.x.to_f32() + (v.pos.0 + v.offset.0 * v.scale.0) * sx;
                let y = p.pos.y.to_f32() + v.pos.1 + v.offset.1 * v.scale.1;
                let c = v.modulate;
                draws.push((img, to_screen(x, y), Vec2::new(v.scale.0, v.scale.1) * zoom, mirror ^ v.flip_h, Color::srgba(c[0], c[1], c[2], c[3]), 0.5 + v.z as f32 * 0.01 + 0.005, -v.rotation * sx));
            }
        }
        // nodes of the fighters' scenes their scripts show (marks, slashes...)
        for i in 0..2 {
            let fac = w.f[i].facing as f32;
            let feet = w.f[i].pos;
            for v in w.fighter_nodes(i) {
                let Some(img) = frame_of(&v, &mut library, &mut images) else { continue };
                let mirror = v.flipped && fac < 0.0;
                let sx = if mirror { -1.0 } else { 1.0 };
                let x = feet.x.to_f32() + (v.pos.0 + v.offset.0 * v.scale.0) * sx;
                let y = feet.y.to_f32() + v.pos.1 + v.offset.1 * v.scale.1;
                let c = v.modulate;
                draws.push((img, to_screen(x, y), Vec2::new(v.scale.0, v.scale.1) * zoom, mirror ^ v.flip_h, Color::srgba(c[0], c[1], c[2], c[3]), 0.5 + v.z as f32 * 0.01 + 0.005, -v.rotation * sx));
            }
        }
        // the UI layer: canvas pixels (640x360) over the whole view
        let s = scale.0;
        for v in w.ui_nodes() {
            let Some(img) = frame_of(&v, &mut library, &mut images) else { continue };
            let at = Vec2::new((v.pos.0 + v.offset.0 * v.scale.0 - 320.0) * s, (180.0 - v.pos.1 - v.offset.1 * v.scale.1) * s);
            let c = v.modulate;
            draws.push((img, at, Vec2::new(v.scale.0, v.scale.1) * s, v.flip_h, Color::srgba(c[0], c[1], c[2], c[3]), 8.0, -v.rotation));
        }
    }
    let mut it = pool.iter_mut();
    for (img, at, sc, flip, color, z, rot) in draws {
        match it.next() {
            Some((mut sprite, mut tf, mut vis)) => {
                sprite.image = img;
                sprite.flip_x = flip;
                sprite.color = color;
                tf.translation = at.extend(z);
                tf.scale = sc.extend(1.0);
                tf.rotation = Quat::from_rotation_z(rot);
                *vis = Visibility::Inherited;
            }
            None => {
                commands.spawn((
                    NodeArt,
                    Sprite { image: img, flip_x: flip, color, ..default() },
                    Transform::from_translation(at.extend(z)).with_scale(sc.extend(1.0)).with_rotation(Quat::from_rotation_z(rot)),
                    DespawnOnExit(AppState::Fight),
                ));
            }
        }
    }
    for (_, _, mut vis) in it {
        *vis = Visibility::Hidden;
    }
}
