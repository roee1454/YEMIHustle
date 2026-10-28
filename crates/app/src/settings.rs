//! Player settings: video, display, audio, key bindings, personalization and the last match
//! rules. Saved as RON in the user's config directory whenever they change.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Keys that can be bound. Bindings are stored by the `KeyCode` debug name (e.g. `"KeyJ"`).
const KEYS: &[KeyCode] = &[
    KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG,
    KeyCode::KeyH, KeyCode::KeyI, KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN,
    KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT, KeyCode::KeyU,
    KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ,
    KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5,
    KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9,
    KeyCode::Numpad0, KeyCode::Numpad1, KeyCode::Numpad2, KeyCode::Numpad3, KeyCode::Numpad4, KeyCode::Numpad5,
    KeyCode::Numpad6, KeyCode::Numpad7, KeyCode::Numpad8, KeyCode::Numpad9, KeyCode::NumpadAdd,
    KeyCode::NumpadSubtract, KeyCode::NumpadMultiply, KeyCode::NumpadDivide, KeyCode::NumpadDecimal,
    KeyCode::NumpadEnter,
    KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown,
    KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4, KeyCode::F5, KeyCode::F6, KeyCode::F7, KeyCode::F8,
    KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12,
    KeyCode::Space, KeyCode::Enter, KeyCode::Tab, KeyCode::Backspace, KeyCode::Comma, KeyCode::Period,
    KeyCode::Slash, KeyCode::Semicolon, KeyCode::Quote, KeyCode::BracketLeft, KeyCode::BracketRight,
    KeyCode::Backslash, KeyCode::Minus, KeyCode::Equal, KeyCode::Backquote,
    KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft,
    KeyCode::AltRight, KeyCode::Insert, KeyCode::Delete, KeyCode::Home, KeyCode::End, KeyCode::PageUp,
    KeyCode::PageDown,
];

/// A bound key, stored by name so the settings file stays readable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Key(pub String);

impl Key {
    pub fn of(k: KeyCode) -> Self {
        Key(format!("{k:?}"))
    }

    pub fn code(&self) -> Option<KeyCode> {
        KEYS.iter().copied().find(|k| format!("{k:?}") == self.0)
    }

    pub fn pressed(&self, input: &ButtonInput<KeyCode>) -> bool {
        self.code().is_some_and(|k| input.pressed(k))
    }

    pub fn just_pressed(&self, input: &ButtonInput<KeyCode>) -> bool {
        self.code().is_some_and(|k| input.just_pressed(k))
    }

    /// Short label for menus and the HUD.
    pub fn label(&self) -> String {
        let s = self.0.as_str();
        let named = match s {
            "Comma" => ",",
            "Period" => ".",
            "Slash" => "/",
            "Semicolon" => ";",
            "Quote" => "'",
            "BracketLeft" => "[",
            "BracketRight" => "]",
            "Backslash" => "\\",
            "Minus" => "-",
            "Equal" => "=",
            "Backquote" => "`",
            _ => "",
        };
        if !named.is_empty() {
            return named.to_string();
        }
        for (prefix, repl) in [("Key", ""), ("Digit", ""), ("Arrow", ""), ("Numpad", "Num ")] {
            if let Some(rest) = s.strip_prefix(prefix) {
                return format!("{repl}{rest}");
            }
        }
        s.to_string()
    }

    /// Any bindable key pressed this frame.
    pub fn any_just_pressed(input: &ButtonInput<KeyCode>) -> Option<KeyCode> {
        KEYS.iter().copied().find(|k| input.just_pressed(*k))
    }
}

/// One player's fight controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerKeys {
    pub left: Key,
    pub right: Key,
    pub up: Key,
    pub down: Key,
    pub light: Key,
    pub heavy: Key,
    pub special: Key,
    pub grab: Key,
    pub dash: Key,
}

pub const PLAYER_ACTIONS: [&str; 9] = ["Left", "Right", "Up / Jump", "Down / Crouch", "Light", "Heavy", "Special", "Grab", "Dash"];

impl PlayerKeys {
    fn new(keys: [KeyCode; 9]) -> Self {
        let [left, right, up, down, light, heavy, special, grab, dash] = keys.map(Key::of);
        PlayerKeys { left, right, up, down, light, heavy, special, grab, dash }
    }

    pub fn p1() -> Self {
        use KeyCode::*;
        Self::new([KeyA, KeyD, KeyW, KeyS, KeyJ, KeyK, KeyL, KeyU, KeyI])
    }

    pub fn p2() -> Self {
        use KeyCode::*;
        Self::new([ArrowLeft, ArrowRight, ArrowUp, ArrowDown, Comma, Period, Slash, Semicolon, Quote])
    }

    pub fn get_mut(&mut self, i: usize) -> &mut Key {
        [
            &mut self.left, &mut self.right, &mut self.up, &mut self.down, &mut self.light, &mut self.heavy,
            &mut self.special, &mut self.grab, &mut self.dash,
        ]
        .into_iter()
        .nth(i)
        .expect("action index")
    }

    pub fn read(&self, k: &ButtonInput<KeyCode>) -> sim::Input {
        sim::Input {
            left: self.left.pressed(k),
            right: self.right.pressed(k),
            up: self.up.pressed(k),
            down: self.down.pressed(k),
            light: self.light.pressed(k),
            heavy: self.heavy.pressed(k),
            special: self.special.pressed(k),
            grab: self.grab.pressed(k),
            dash: self.dash.pressed(k),
        }
    }
}

/// Keys outside the fighters' controls (Esc always opens the pause menu).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hotkeys {
    pub pause_sim: Key,
    pub frame_advance: Key,
    pub reset_round: Key,
    pub toggle_hitboxes: Key,
    pub dummy_mode: Key,
    pub reload_data: Key,
    pub toggle_fullscreen: Key,
    pub toggle_hud: Key,
    #[serde(default = "record_key")]
    pub record_dummy: Key,
    #[serde(default = "playback_key")]
    pub play_recording: Key,
}

fn record_key() -> Key {
    Key::of(KeyCode::F6)
}

fn playback_key() -> Key {
    Key::of(KeyCode::F7)
}

pub const HOTKEY_ACTIONS: [&str; 10] = [
    "Freeze / Unfreeze", "Frame Advance", "Reset Round", "Toggle Hitboxes", "P2 Dummy Mode", "Reload Data",
    "Toggle Fullscreen", "Toggle HUD", "Record Dummy (training)", "Play Recording (training)",
];

impl Default for Hotkeys {
    fn default() -> Self {
        use KeyCode::*;
        let k = Key::of;
        Hotkeys {
            pause_sim: k(KeyP),
            frame_advance: k(KeyO),
            reset_round: k(KeyR),
            toggle_hitboxes: k(F2),
            dummy_mode: k(F3),
            reload_data: k(F5),
            toggle_fullscreen: k(F11),
            toggle_hud: k(F1),
            record_dummy: record_key(),
            play_recording: playback_key(),
        }
    }
}

impl Hotkeys {
    pub fn get_mut(&mut self, i: usize) -> &mut Key {
        [
            &mut self.pause_sim, &mut self.frame_advance, &mut self.reset_round, &mut self.toggle_hitboxes,
            &mut self.dummy_mode, &mut self.reload_data, &mut self.toggle_fullscreen, &mut self.toggle_hud,
            &mut self.record_dummy, &mut self.play_recording,
        ]
        .into_iter()
        .nth(i)
        .expect("hotkey index")
    }
}

/// What controls player 2 in a local match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum P2Control {
    Human,
    #[default]
    Stand,
    Block,
    CrouchBlock,
    Jump,
}

impl P2Control {
    pub const ALL: [P2Control; 5] = [P2Control::Human, P2Control::Stand, P2Control::Block, P2Control::CrouchBlock, P2Control::Jump];

    pub fn label(self) -> &'static str {
        match self {
            P2Control::Human => "second player",
            P2Control::Stand => "dummy: stand",
            P2Control::Block => "dummy: block",
            P2Control::CrouchBlock => "dummy: crouch block",
            P2Control::Jump => "dummy: jump",
        }
    }

    pub fn cycle(self, d: i32) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0) as i32;
        Self::ALL[(i + d).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

/// A row of the options screen or the character select's match settings, in the original's
/// order. Values are stored by key as numbers (checks are 0 or 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Check { key: &'static str, label: &'static str, on: bool },
    /// A slider; `pct` shows the value as a percentage, `show` prints the number next to it.
    Slider { key: &'static str, label: &'static str, min: f64, max: f64, step: f64, default: f64, show: bool, pct: bool },
    Spin { key: &'static str, label: &'static str, min: f64, max: f64, default: f64 },
    /// One of several named options; the value is the option's index.
    Choice { key: &'static str, label: &'static str, options: &'static [&'static str], default: f64 },
    Separator,
}

impl Field {
    pub fn key(&self) -> Option<&'static str> {
        match self {
            Field::Check { key, .. } | Field::Slider { key, .. } | Field::Spin { key, .. } | Field::Choice { key, .. } => Some(key),
            Field::Separator => None,
        }
    }

    pub fn default_value(&self) -> f64 {
        match *self {
            Field::Check { on, .. } => on as u8 as f64,
            Field::Slider { default, .. } | Field::Spin { default, .. } | Field::Choice { default, .. } => default,
            Field::Separator => 0.0,
        }
    }

    /// Steps a value; checks flip.
    pub fn step(&self, v: f64, d: i32) -> f64 {
        match *self {
            Field::Check { .. } => 1.0 - v.round(),
            Field::Slider { min, max, step, .. } => tidy(((v + d as f64 * step) / step).round() * step).clamp(min, max),
            Field::Spin { min, max, .. } => (v + d as f64).clamp(min, max),
            Field::Choice { options, .. } => (v.round() as i64 + d as i64).rem_euclid(options.len().max(1) as i64) as f64,
            Field::Separator => v,
        }
    }

    /// Sets a slider from a position along it (0..1).
    pub fn at(&self, t: f64) -> f64 {
        match *self {
            Field::Slider { min, max, step, .. } => tidy(((min + t.clamp(0.0, 1.0) * (max - min)) / step).round() * step).clamp(min, max),
            _ => self.default_value(),
        }
    }
}

/// Drops float noise from stepped values (1.1000000000000001 -> 1.1).
fn tidy(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

const fn check(key: &'static str, label: &'static str, on: bool) -> Field {
    Field::Check { key, label, on }
}

const fn volume(key: &'static str, label: &'static str) -> Field {
    Field::Slider { key, label, min: 0.0, max: 1.0, step: 0.01, default: 1.0, show: false, pct: false }
}

/// The original's Options tab. Only some have an effect here (see `Settings` accessors);
/// the rest are kept so the screen matches and for when the features exist.
pub const OPTIONS: &[Field] = &[
    check("fullscreen", "fullscreen", false),
    check("cap_framerate", "cap framerate to 60fps", true),
    check("vsync", "enable vsync", true),
    Field::Separator,
    check("hitboxes", "show hitboxes", false),
    check("extra_info", "show extra game info", false),
    check("precise_health", "show precise health count", false),
    check("next_turn_hud", "show next-turn info on hud", false),
    check("next_turn_chars", "show next-turn info on characters", true),
    check("last_moves", "show last moves played", true),
    check("projectile_owners", "show projectile owners", true),
    check("playback_hotkeys_window", "playback hotkeys require window", true),
    check("speed_lines", "show speed lines", true),
    check("storm_weather", "enable storm weather (F8)", false),
    check("low_timer_sound", "enable low timer sound", true),
    check("freeze_frames", "enable hit freeze frames", true),
    check("playback_controls", "show playback controls", false),
    check("emotes", "show emotes", true),
    check("style_colors", "enable style colors", true),
    check("auras", "enable particle auras", true),
    check("hitsparks", "enable custom hitsparks", true),
    check("battle_intros", "play battle intros", true),
    check("community_events", "show community events", true),
    Field::Separator,
    check("music", "enable music", true),
    volume("master_volume", "master volume "),
    volume("fx_volume", "fx volume "),
    volume("ui_volume", "ui volume "),
    volume("music_volume", "music volume "),
    Field::Separator,
    check("replay_backups", "enable per-turn backups", true),
    check("auto_free_cancel", "enable auto free cancel", true),
    check("snap_xy", "hold shift to snap xy plots", false),
];

pub const MODS_TOGGLE: Field = check("mods", "enable mods (must restart)", false);
pub const NAME_HUE: Field = Field::Slider { key: "name_hue", label: "hue", min: 0.0, max: 1.0, step: 0.01, default: 0.0, show: false, pct: false };
pub const NAME_SATURATION: Field = Field::Slider { key: "name_saturation", label: "saturation", min: 0.0, max: 1.0, step: 0.01, default: 0.0, show: false, pct: false };

const fn spin(key: &'static str, label: &'static str, min: f64, max: f64, default: f64) -> Field {
    Field::Spin { key, label, min, max, default }
}

const fn modifier(key: &'static str, label: &'static str, min: f64, max: f64, step: f64, default: f64) -> Field {
    Field::Slider { key, label, min, max, step, default, show: true, pct: false }
}

/// The character select's match settings, as in the original's singleplayer panel.
pub const RULES: &[Field] = &[
    spin("round_length", "Round Length ", 1.0, 999999.0, 3600.0),
    spin("char_distance", "Start Distance", 16.0, 300.0, 125.0),
    spin("char_height", "Start Height", 0.0, 100000.0, 0.0),
    spin("stage_width", "Stage Width", 100.0, 100000.0, 800.0),
    spin("ceiling_height", "Ceiling Height", 32.0, 50000.0, 800.0),
    spin("di_combo_limit", "DI Combo Limit", 1.0, 99.0, 15.0),
    modifier("damage_modifier", "Damage Modifier", 0.1, 3.0, 0.1, 1.0),
    modifier("hitstun_modifier", "Hitstun Modifier", 0.1, 5.0, 0.1, 1.0),
    modifier("hitstop_modifier", "Hitstop Modifier", 0.0, 5.0, 0.1, 1.0),
    modifier("gravity_modifier", "Gravity Modifier", 0.1, 5.0, 0.1, 1.0),
    modifier("parry_damage", "Parry Damage", 0.0, 2.0, 0.05, 0.85),
    modifier("burst_parry_damage", "Burst Parry Damage", 0.0, 2.0, 0.05, 0.85),
    modifier("starting_meter", "Starting Meter", 0.0, 9.99, 0.5, 0.0),
    modifier("min_di_scaling", "Min DI Scaling", 0.0, 20.0, 0.25, 1.0),
    modifier("max_di_scaling", "Max DI Scaling", 0.0, 20.0, 0.25, 6.0),
    check("asymmetrical", "Asymmetrical Health/Meter", false),
    Field::Slider { key: "p1_health", label: "P1 Health", min: 2.0, max: 100.0, step: 2.0, default: 100.0, show: true, pct: true },
    Field::Slider { key: "p2_health", label: "P2 Health", min: 2.0, max: 100.0, step: 2.0, default: 100.0, show: true, pct: true },
    modifier("p1_meter", "P1 Meter", 0.0, 9.99, 0.5, 0.0),
    modifier("p2_meter", "P2 Meter", 0.0, 9.99, 0.5, 0.0),
    check("p2_dummy", "No P2 Input", false),
    check("prediction", "Prediction Enabled", true),
    check("burst", "Burst Enabled", true),
    check("di", "DI Enabled", true),
    check("prorated_di", "Prorated DI", true),
    check("show_last_di", "Show Last DI State", true),
    check("sadness", "Sadness Enabled", true),
    check("clashing", "Clashing Enabled", true),
    check("brace", "Brace Enabled", false),
    check("ceiling", "Ceiling Enabled", true),
    check("gravity", "Gravity Enabled", true),
    check("asymmetrical_clashing", "Asymmetrical Clashing", false),
    check("turbo", "Turbo Mode", false),
    check("extremely_turbo", "Extremely Turbo Mode", false),
    check("infinite_resources", "Infinite Resources", false),
    check("one_hit_ko", "One-Hit KO", false),
    check("always_perfect_parry", "Always Perfect Parry", false),
    check("frame_by_frame", "Frame By Frame", false),
];

const fn choice(key: &'static str, label: &'static str, options: &'static [&'static str], default: f64) -> Field {
    Field::Choice { key, label, options, default }
}

/// What the training dummy does (the `dummy` choice of `TRAINING`).
pub const DUMMY_ACTIONS: &[&str] = &[
    "stand", "crouch", "jump", "block all", "block after first hit", "random block", "CPU easy", "CPU normal", "CPU hard",
    "play recording",
];

/// Training mode's options, shown in the pause menu during training.
pub const TRAINING: &[Field] = &[
    choice("dummy", "Dummy", DUMMY_ACTIONS, 0.0),
    choice("wakeup", "Wakeup", &["none", "jump", "attack", "backdash", "block"], 0.0),
    choice("after_hit", "After Hit", &["stand", "jump out", "attack", "backdash"], 0.0),
    check("refill_health", "Refill Health", true),
    choice("p1_meter", "P1 Meter", &["infinite", "normal", "empty"], 0.0),
    choice("p2_meter", "P2 Meter", &["normal", "infinite", "empty"], 0.0),
    choice("position", "Reset Position", &["center", "left corner", "right corner", "swap sides"], 0.0),
    choice("speed", "Game Speed", &["100%", "75%", "50%", "25%"], 0.0),
    Field::Separator,
    check("show_hitboxes", "Show Hitboxes", false),
    check("frame_data", "Frame Data", true),
    check("input_display", "Input Display", true),
    check("combo_info", "Combo Info", true),
];

/// Rows only shown while "Asymmetrical Health/Meter" is on.
pub fn rule_visible(f: &Field, values: &Values) -> bool {
    match f.key() {
        Some("p1_health" | "p2_health" | "p1_meter" | "p2_meter") => values.get(&RULES[15]) > 0.5,
        _ => true,
    }
}

/// Values by key; missing keys read as the field's default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Values(pub BTreeMap<String, f64>);

impl Values {
    pub fn get(&self, f: &Field) -> f64 {
        f.key().and_then(|k| self.0.get(k).copied()).unwrap_or_else(|| f.default_value())
    }

    pub fn on(&self, f: &Field) -> bool {
        self.get(f) > 0.5
    }

    pub fn set(&mut self, f: &Field, v: f64) {
        if let Some(k) = f.key() {
            self.0.insert(k.to_string(), v);
        }
    }

    pub fn by_key(&self, table: &[Field], key: &str) -> f64 {
        table.iter().find(|f| f.key() == Some(key)).map(|f| self.get(f)).unwrap_or(0.0)
    }
}

/// Converts the match settings into the simulation's rules and engine overrides.
pub fn rules_to_sim(v: &Values) -> sim::Rules {
    let g = |k: &str| v.by_key(RULES, k);
    let pct = |k: &str| (g(k) * 100.0).round() as i32;
    let asym = g("asymmetrical") > 0.5;
    sim::Rules {
        damage_pct: pct("damage_modifier"),
        hitstun_pct: pct("hitstun_modifier"),
        hitstop_pct: pct("hitstop_modifier"),
        gravity_pct: if g("gravity") > 0.5 { pct("gravity_modifier") } else { 0 },
        health_pct: if asym { [g("p1_health") as i32, g("p2_health") as i32] } else { [100, 100] },
        one_hit_ko: g("one_hit_ko") > 0.5,
        infinite_resources: g("infinite_resources") > 0.5,
        round_frames: Some(g("round_length").max(1.0) as u64),
    }
}

/// Stage size and start positions live in the engine config; the match settings override
/// them (the original's units: half the stage width, each fighter's distance from centre).
pub fn rules_to_engine(v: &Values, cfg: &mut sim::EngineConfig) {
    cfg.stage_half_width = v.by_key(RULES, "stage_width") as i32;
    cfg.start_distance = (v.by_key(RULES, "char_distance") as i32).min(cfg.stage_half_width - 10);
}

pub fn p2_control(v: &Values) -> P2Control {
    if v.by_key(RULES, "p2_dummy") > 0.5 { P2Control::Stand } else { P2Control::Human }
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The Options tab, by key (see `OPTIONS`).
    pub options: Values,
    pub p1_keys: PlayerKeys,
    pub p2_keys: PlayerKeys,
    pub hotkeys: Hotkeys,
    // personalization
    pub name: String,
    pub name_hue: f64,
    pub name_saturation: f64,
    /// The character select's match settings, by key (see `RULES`).
    pub rules: Values,
    pub format_name: String,
    /// Shown on the fight HUD; not part of the original's options.
    pub show_frame_advantage: bool,
    pub show_controls_bar: bool,
    /// Training mode's options, by key (see `TRAINING`).
    pub training: Values,
    /// The style each player fights in (a style's name; empty is the player's default colors).
    pub styles: [String; 2],
    /// The CPU opponent's level: 0 easy, 1 normal, 2 hard.
    pub cpu_level: u8,
    /// Controllers: bindings, stick handling, which pad each player uses.
    pub pad: crate::pad::PadSettings,
}

pub const DEFAULT_NAME: &str = "";

impl Default for Settings {
    fn default() -> Self {
        Settings {
            options: Values::default(),
            p1_keys: PlayerKeys::p1(),
            p2_keys: PlayerKeys::p2(),
            hotkeys: Hotkeys::default(),
            name: DEFAULT_NAME.into(),
            name_hue: 0.0,
            name_saturation: 0.0,
            rules: Values::default(),
            format_name: String::new(),
            show_frame_advantage: true,
            show_controls_bar: true,
            training: Values::default(),
            styles: [String::new(), String::new()],
            cpu_level: 1,
            pad: crate::pad::PadSettings::default(),
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
        Some(base.join("yomihustle-realtime").join("settings.ron"))
    }

    /// Loads saved settings; a missing or unreadable file gives the defaults.
    pub fn load() -> Self {
        let Some(p) = Self::path() else { return Self::default() };
        match std::fs::read_to_string(&p) {
            Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
                eprintln!("ignoring unreadable settings {}: {e}", p.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        let text = match ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()) {
            Ok(t) => t,
            Err(e) => return eprintln!("could not encode settings: {e}"),
        };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&p, text) {
            eprintln!("could not save settings to {}: {e}", p.display());
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        OPTIONS.iter().chain([&MODS_TOGGLE]).find(|f| f.key() == Some(key)).is_some_and(|f| self.options.on(f))
    }

    pub fn train(&self, key: &str) -> f64 {
        self.training.by_key(TRAINING, key)
    }

    pub fn set_flag(&mut self, key: &str, on: bool) {
        if let Some(f) = OPTIONS.iter().find(|f| f.key() == Some(key)) {
            self.options.set(f, on as u8 as f64);
        }
    }

    /// The original's name color: hue and saturation sliders at full value.
    pub fn name_color(&self) -> Color {
        Color::hsv(self.name_hue as f32 * 360.0, self.name_saturation as f32, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_by_name() {
        for k in KEYS {
            assert_eq!(Key::of(*k).code(), Some(*k));
        }
        assert_eq!(Key::of(KeyCode::KeyJ).label(), "J");
        assert_eq!(Key::of(KeyCode::Numpad3).label(), "Num 3");
        assert_eq!(Key::of(KeyCode::Comma).label(), ",");
    }

    #[test]
    fn settings_round_trip_through_ron() {
        let mut s = Settings::default();
        s.p1_keys.light = Key::of(KeyCode::KeyZ);
        s.set_flag("hitboxes", true);
        s.rules.set(&RULES[6], 2.0);
        let text = ron::ser::to_string_pretty(&s, ron::ser::PrettyConfig::default()).unwrap();
        let back: Settings = ron::from_str(&text).unwrap();
        assert_eq!(back, s);
        assert!(back.flag("hitboxes"));
        // files written by older versions miss fields; they get defaults
        let partial: Settings = ron::from_str("(name: \"x\")").unwrap();
        assert!(partial.flag("vsync") && !partial.flag("fullscreen"));
    }

    #[test]
    fn match_settings_reach_the_simulation() {
        let mut v = Values::default();
        let r = rules_to_sim(&v);
        assert_eq!((r.damage_pct, r.round_frames), (100, Some(3600)));
        v.set(&RULES[6], 1.5); // damage modifier
        v.set(&RULES[16], 50.0); // P1 health, ignored until asymmetrical is on
        assert_eq!(rules_to_sim(&v).damage_pct, 150);
        assert_eq!(rules_to_sim(&v).health_pct, [100, 100]);
        v.set(&RULES[15], 1.0);
        assert_eq!(rules_to_sim(&v).health_pct, [50, 100]);
        // slider steps snap to the step size
        assert_eq!(RULES[6].step(1.0, 1), 1.1);
        assert_eq!(RULES[6].at(1.0), 3.0);
    }
}
