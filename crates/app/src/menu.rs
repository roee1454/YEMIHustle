//! Menus laid out like the original game's: main menu, options (Options / Hotkeys /
//! Personalization tabs), credits, character select with the match settings panel, pause and
//! post-game. Positions are the original's, in 640x360 canvas pixels.
//!
//! Each page is rebuilt from the menu state whenever it changes; widgets carry a `Clickable`
//! with the `Id` the input system acts on.

use bevy::input::keyboard::{Key as LogicalKey, KeyboardInput};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::ui::{ComputedNode, RelativeCursorPosition};

use crate::customize::{self, CustId};
use crate::fight::Mode;
use crate::pixel::*;
use crate::settings::{
    rule_visible, Field, Key, Settings, Values, HOTKEY_ACTIONS, MODS_TOGGLE, NAME_HUE, NAME_SATURATION, OPTIONS,
    PLAYER_ACTIONS, RULES, TRAINING,
};
use crate::style::Styles;
use crate::setup::{self, Setup};
use crate::skin::Skin;
use crate::{Game, Roster};

pub const TITLE: &str = "Your Every Move Is HUSTLE";
const TITLE_LINE_1: &str = "Your Every Move Is";
const TITLE_LINE_2: &str = "HUSTLE";
const TAGLINE: &str = "- REAL-TIME FIGHTING -";
/// The original is credited under the title (the link opens Ivy Sly's site).
const CREDIT_LINE: &str = "BASED ON THE GAME BY IVY SLY";

/// The official character select buttons, in the original's order: (label, roster id).
const SELECT_ORDER: [(&str, &str); 5] =
    [("Ninja", "ninja"), ("Cowboy", "cowboy"), ("Wizard", "wizard"), ("Robot", "robo"), ("Mutant", "mutant")];

/// The select screen's buttons: the official five, then imported fighters (label, roster id).
pub fn choices(roster: &Roster) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = SELECT_ORDER.iter().map(|(l, id)| (l.to_string(), id.to_string())).collect();
    for r in &roster.0 {
        if !v.iter().any(|(_, id)| *id == r.id) {
            v.push((r.name.clone(), r.id.clone()));
        }
    }
    v
}

const DISCORD_URL: &str = "https://discord.gg/YourOnlyMoveIsHUSTLE";
const TWITTER_URL: &str = "https://x.com/YourMoveHUSTLE";
const TIKTOK_URL: &str = "https://www.tiktok.com/@youronlymoveishustle";
const IVY_SLY_URL: &str = "https://www.ivysly.com";

// ---------------------------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Main,
    Options,
    Credits,
    Select,
    Unavailable(Feature),
    Pause,
    PostGame,
    /// A fighter's buttons, opened from the pause menu (the player's index).
    MoveList(usize),
    /// Singleplayer's choice of mode: vs CPU, training or versus.
    Modes,
    /// Training's options (from the pause menu during training).
    Training,
    /// Making character styles.
    Customize,
    /// The first-run wizard that builds the library from the player's own copy of the game.
    Setup,
}

/// Main-menu entries that exist in the original but not here yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    SteamMultiplayer,
    LegacyMultiplayer,
    DirectConnect,
    Customize,
    Replays,
}

impl Feature {
    fn title(self) -> &'static str {
        match self {
            Feature::SteamMultiplayer => "Multiplayer (Steam)",
            Feature::LegacyMultiplayer => "Multiplayer (Legacy)",
            Feature::DirectConnect => "Direct Connect",
            Feature::Customize => "Customize",
            Feature::Replays => "Replays",
        }
    }

    fn explanation(self) -> &'static str {
        match self {
            Feature::SteamMultiplayer | Feature::LegacyMultiplayer | Feature::DirectConnect => {
                "Online play needs rollback netcode. The simulation is deterministic so it can be added, but it is not written yet."
            }
            Feature::Customize => "Character styles (colors, auras, hit sparks) come after the characters have real art.",
            Feature::Replays => "Replays are the inputs of a match replayed through the deterministic simulation. Not written yet.",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Options,
    Hotkeys,
    Personalization,
    Controller,
}

const TABS: [(Tab, &str); 4] =
    [(Tab::Options, "Options"), (Tab::Hotkeys, "Hotkeys"), (Tab::Personalization, "Personalization"), (Tab::Controller, "Controller")];

/// The Controller tab's rows: 0, 1 which pad each player uses; 2..=10 the bindings (see
/// `pad::ACTIONS`); 11 dead zone; 12 diagonal width; 13 tap jump; 14 rumble; 15 defaults.
const PAD_ROWS: u8 = 16;
const PAD_DEADZONE: (f64, f64) = (0.1, 0.8);
const PAD_DIAGONAL: (f64, f64) = (0.2, 0.7);

/// Everything that can be hovered, focused or clicked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Id {
    Singleplayer,
    Feature(Feature),
    Options,
    Credits,
    Quit,
    Roadmap,
    IvySly,
    Discord,
    Twitter,
    TikTok,
    Tab(usize),
    /// A row of `OPTIONS`.
    Opt(usize),
    Mods,
    DeleteCache,
    Done,
    SetupUse(usize),
    SetupBrowse,
    SetupSkip,
    SetupBuild,
    SetupBack,
    SetupDownload,
    SetupStart,
    NameEdit,
    NameReset,
    Hue,
    Saturation,
    ColorReset,
    /// A row of the Hotkeys tab (see `bind_rows`).
    Bind(usize),
    Back,
    /// A character button, by `SELECT_ORDER` index.
    Char(usize),
    Go,
    SelectQuit,
    /// A row of `RULES`.
    Rule(usize),
    MoveList,
    MoveListOther,
    /// A mode on the Singleplayer page: 0 versus, 1 vs CPU, 2 training.
    Mode(u8),
    CpuLevel,
    /// A row of `TRAINING`.
    Train(usize),
    TrainingPage,
    Customize,
    Restart,
    CharSelect,
    Record,
    PlayRecording,
    PauseCpu,
    /// A row of the Controller tab (see `PAD_ROWS`).
    Pad(u8),
    Cust(CustId),
    Format,
    FormatName,
    FormatSave,
    LoadStyle(usize),
    Resume,
    SaveReplay,
    PauseOptions,
    MainMenu,
    Forfeit,
    Rematch,
}

impl Id {
    /// Sliders and spin boxes react to where they are clicked.
    fn wants_cursor(self) -> bool {
        match self {
            Id::Hue | Id::Saturation => true,
            Id::Opt(i) => matches!(OPTIONS[i], Field::Slider { .. }),
            Id::Rule(i) => !matches!(RULES[i], Field::Check { .. } | Field::Choice { .. }),
            Id::Cust(c) => customize::wants_cursor(c),
            Id::Pad(k) => k == 11 || k == 12,
            _ => false,
        }
    }
}

#[derive(Component, Clone, Copy)]
pub struct Clickable(pub Id);

#[derive(Component)]
pub struct MenuRoot;

/// Sounds the menu wants played this frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Hover,
    Select,
}

#[derive(Resource)]
pub struct Menu {
    /// Open pages, innermost last. Empty while fighting.
    pub stack: Vec<Page>,
    pub tab: Tab,
    pub hovered: Option<Id>,
    /// Keyboard focus; shown with the theme's focus underline.
    pub focus: Option<Id>,
    /// Focusable widgets of the current page in order, from the last build.
    order: Vec<Id>,
    dragging: Option<Id>,
    pub roadmap: bool,
    scroll_options: f32,
    scroll_rules: f32,
    /// Text being typed into (the name or the format name).
    pub editing: Option<Id>,
    /// Hotkeys row waiting for a key.
    pub rebinding: Option<usize>,
    /// Controller binding (an action of `pad::ACTIONS`) waiting for a button.
    pub pad_rebinding: Option<usize>,
    /// Character select: picks by `SELECT_ORDER` index, and what each display shows.
    pub picks: [Option<usize>; 2],
    shown: [Option<usize>; 2],
    /// Whether a display shows its character's name (P2's starts with a bare portrait).
    named: [bool; 2],
    pub status: String,
    pub dirty: bool,
    /// The menu closed this frame (so the Esc that closed it does not reopen the pause menu).
    pub just_closed: bool,
    pub sounds: Vec<Sound>,
    pub setup: Setup,
    /// The mode the character select starts.
    pub mode: Mode,
    /// The Customize screen's style being edited.
    pub cust: customize::Edit,
}

impl Menu {
    pub fn new(start: Page) -> Self {
        let mut m = Menu {
            stack: vec![],
            tab: Tab::Options,
            hovered: None,
            focus: None,
            order: vec![],
            dragging: None,
            roadmap: false,
            scroll_options: 0.0,
            scroll_rules: 0.0,
            editing: None,
            rebinding: None,
            pad_rebinding: None,
            picks: [None, None],
            shown: [Some(0), Some(0)],
            named: [true, false],
            status: String::new(),
            dirty: true,
            just_closed: false,
            sounds: vec![],
            setup: Setup::new(crate::library::Library::new(crate::library::Library::default_root())),
            mode: Mode::Cpu(1),
            cust: customize::Edit::default(),
        };
        if !matches!(start, Page::Main | Page::Pause | Page::PostGame | Page::Setup) {
            m.stack.push(Page::Main);
        }
        m.push(start);
        m
    }

    pub fn page(&self) -> Option<Page> {
        self.stack.last().copied()
    }

    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn push(&mut self, p: Page) {
        if p == Page::Select {
            self.picks = [None, None];
            self.shown = [Some(0), Some(0)];
            self.named = [true, false];
            self.scroll_rules = 0.0;
        }
        if p == Page::Options {
            self.scroll_options = 0.0;
        }
        if p == Page::Setup {
            self.setup.reset();
        }
        self.stack.push(p);
        self.after_change();
    }

    pub fn pop(&mut self) {
        self.stack.pop();
        self.after_change();
    }

    pub fn reset_to(&mut self, pages: &[Page]) {
        self.stack.clear();
        for p in pages {
            self.push(*p);
        }
    }

    pub fn close(&mut self) {
        self.stack.clear();
        self.after_change();
        self.just_closed = true;
    }

    fn after_change(&mut self) {
        self.status.clear();
        self.focus = None;
        self.editing = None;
        self.rebinding = None;
        self.pad_rebinding = None;
        self.dirty = true;
    }

    /// The player picking on the character select (0 or 1), or None once both have picked.
    fn picking(&self) -> Option<usize> {
        self.picks.iter().position(|p| p.is_none())
    }
}

/// The Hotkeys tab rows: both players' controls, then the fight hotkeys.
fn bind_rows() -> Vec<String> {
    let mut v: Vec<String> = PLAYER_ACTIONS.iter().map(|a| format!("P1 {a}")).collect();
    v.extend(PLAYER_ACTIONS.iter().map(|a| format!("P2 {a}")));
    v.extend(HOTKEY_ACTIONS.iter().map(|a| a.to_string()));
    v
}

fn bind_key(s: &mut Settings, row: usize) -> &mut Key {
    match row {
        0..=8 => s.p1_keys.get_mut(row),
        9..=17 => s.p2_keys.get_mut(row - 9),
        _ => s.hotkeys.get_mut(row - 18),
    }
}

// ---------------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------------

/// What a menu action asks the rest of the app to do.
pub enum Effect {
    None,
    StartFight([usize; 2], Mode),
    Rematch,
    Resume,
    ToMenu(Vec<Page>),
    Forfeit,
    Quit,
    /// The wizard finished: load the new library.
    LibraryReady,
    /// Pause menu: restart the round (training: reset to the chosen position).
    Restart,
    /// Pause menu: back to the character select, in the same mode.
    ToSelect,
    /// Pause menu (training): record the dummy, or play the recording.
    Record,
    PlayRecording,
    /// Pause menu (vs CPU): the CPU level changed.
    ApplyCpu,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn menu_input(
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut typed: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<MouseWheel>,
    widgets: Query<(Ref<Interaction>, &Clickable, Option<&RelativeCursorPosition>, &ComputedNode, &UiGlobalTransform)>,
    roster: Res<Roster>,
    mut styles: ResMut<Styles>,
    mut effects: MessageWriter<crate::MenuEffect>,
    gamepads: Query<&bevy::input::gamepad::Gamepad>,
    mut stick: Local<IVec2>,
) {
    // `styles` is borrowed mutably only when a widget is used: a mutable borrow marks it changed,
    // which rebuilds the menu, and a menu rebuilt every frame never sees a click
    let styles = &mut styles;
    let nav = pad_nav(&gamepads, &mut stick);
    let typed: Vec<KeyboardInput> = typed.read().cloned().collect();
    menu.just_closed = false;
    if !menu.is_open() {
        wheel.clear();
        return;
    }
    let mut fx = Vec::new();

    // capture modes first: binding a controller button, rebinding a key, typing text
    if let Some(action) = menu.pad_rebinding {
        if keys.just_pressed(KeyCode::Escape) {
            menu.pad_rebinding = None;
            menu.dirty = true;
        } else if let Some(name) = gamepads.iter().flat_map(|g| g.get_just_pressed().copied().collect::<Vec<_>>()).find_map(crate::pad::name_of) {
            if let Some(slot) = settings.pad.binds.get_mut(action) {
                *slot = name.to_string();
            }
            menu.pad_rebinding = None;
            menu.dirty = true;
        }
        return;
    }
    if let Some(row) = menu.rebinding {
        if keys.just_pressed(KeyCode::Escape) {
            menu.rebinding = None;
            menu.dirty = true;
        } else if let Some(k) = Key::any_just_pressed(&keys) {
            *bind_key(&mut settings, row) = Key::of(k);
            menu.rebinding = None;
            menu.dirty = true;
        }
        return;
    }
    if let Some(field) = menu.editing {
        let mut cust_name = std::mem::take(&mut menu.cust.style.name);
        let target = match field {
            Id::NameEdit => &mut settings.name,
            Id::Cust(_) => &mut cust_name,
            _ => &mut settings.format_name,
        };
        for ev in typed.iter().filter(|e| e.state.is_pressed()) {
            match &ev.logical_key {
                LogicalKey::Enter | LogicalKey::Escape => menu.editing = None,
                LogicalKey::Backspace => {
                    target.pop();
                }
                LogicalKey::Character(c) if target.chars().count() < 24 => target.extend(c.chars().filter(|ch| !ch.is_control())),
                LogicalKey::Space if target.chars().count() < 24 => target.push(' '),
                _ => {}
            }
            menu.dirty = true;
        }
        menu.cust.style.name = cust_name;
        if mouse.just_pressed(MouseButton::Left) && menu.hovered != Some(field) {
            menu.editing = None;
            menu.dirty = true;
        } else {
            return;
        }
    }

    // mouse: hover, click, drag
    let mut hovered = None;
    for (interaction, c, rel, node, _) in &widgets {
        if *interaction != Interaction::None {
            hovered = Some(c.0);
        }
        if interaction.is_changed() && !interaction.is_added() && *interaction == Interaction::Pressed {
            menu.focus = None;
            let pos = cursor_t(rel, node);
            fx.push(activate(&mut menu, &mut settings, &roster, styles, c.0, Act::Click(pos)));
            if c.0.wants_cursor() {
                menu.dragging = Some(c.0);
            }
            menu.sounds.push(Sound::Select);
        }
    }
    if let Some(d) = menu.dragging {
        if mouse.pressed(MouseButton::Left) {
            if let Some((_, _, rel, node, _)) = widgets.iter().find(|w| w.1 .0 == d) {
                let pos = cursor_t(rel, node);
                drag(&mut menu, &mut settings, d, pos);
            }
        } else {
            menu.dragging = None;
        }
    }
    if hovered != menu.hovered {
        if hovered.is_some() {
            menu.sounds.push(Sound::Hover);
        }
        // hovering a character shows it on the picking player's display
        if let (Some(Id::Char(i)), Some(p)) = (hovered, menu.picking()) {
            menu.shown[p] = Some(i);
            menu.named[p] = true;
        }
        menu.hovered = hovered;
        menu.dirty = true;
    }

    // wheel scrolls the long lists
    for w in wheel.read() {
        let d = -w.y.signum() * 13.0;
        match menu.page() {
            Some(Page::Options) => menu.scroll_options = (menu.scroll_options + d).max(0.0),
            Some(Page::Select) => menu.scroll_rules = (menu.scroll_rules + d).max(0.0),
            _ => continue,
        }
        menu.dirty = true;
    }

    // keyboard
    let pressed = |a: &[KeyCode]| a.iter().any(|k| keys.just_pressed(*k));
    use KeyCode::*;
    let order = menu.order.clone();
    let idx = menu.focus.and_then(|f| order.iter().position(|x| *x == f));
    let step_focus = |d: i32| -> Option<Id> {
        if order.is_empty() {
            return None;
        }
        let n = order.len() as i32;
        let i = match idx {
            Some(i) => (i as i32 + d).rem_euclid(n),
            None => if d > 0 { 0 } else { n - 1 },
        };
        Some(order[i as usize])
    };
    // where each usable widget is on screen, for moving the focus by direction
    let rects: Vec<(Id, Vec2, Vec2)> = widgets.iter().map(|(_, c, _, node, t)| (c.0, t.translation, node.size())).collect();
    let focus = menu.focus;
    let toward = |dir: Vec2| -> Option<Id> { nearest(&rects, focus?, dir) };
    if pressed(&[ArrowDown, KeyS]) || nav.down {
        // the nearest widget below, else the next in the page's order (wrapping round)
        menu.focus = toward(Vec2::Y).or_else(|| step_focus(1));
        menu.sounds.push(Sound::Hover);
        menu.dirty = true;
    }
    if pressed(&[ArrowUp, KeyW]) || nav.up {
        menu.focus = toward(-Vec2::Y).or_else(|| step_focus(-1));
        menu.sounds.push(Sound::Hover);
        menu.dirty = true;
    }
    // the Controller tab scrolls to the focused row
    if let (Some(Id::Pad(k)), Some(Page::Options)) = (menu.focus, menu.page()) {
        let y = 13.0 + k as f32 * 13.0 + if k >= 11 { 6.0 } else if k >= 2 { 3.0 } else { 0.0 };
        let (top, h) = (menu.scroll_options, 207.0);
        if y < top {
            menu.scroll_options = (y - 13.0).max(0.0);
        } else if y + 13.0 > top + h {
            menu.scroll_options = y + 13.0 - h;
        }
    }
    for (keys_, d, padded) in [(&[ArrowLeft, KeyA][..], -1, nav.left), (&[ArrowRight, KeyD][..], 1, nav.right)] {
        if pressed(keys_) || padded {
            match menu.focus {
                // a value (slider, choice, tab) steps; anything else moves the focus sideways
                Some(f) if takes_steps(f) => fx.push(activate(&mut menu, &mut settings, &roster, styles, f, Act::Step(d))),
                Some(_) => {
                    if let Some(next) = toward(Vec2::X * d as f32) {
                        menu.focus = Some(next);
                        menu.sounds.push(Sound::Hover);
                        menu.dirty = true;
                    }
                }
                None if menu.page() == Some(Page::Options) => cycle_tab(&mut menu, d),
                None => {
                    menu.focus = step_focus(d);
                    menu.dirty = true;
                }
            }
        }
    }
    if pressed(&[Tab]) && menu.page() == Some(Page::Options) {
        cycle_tab(&mut menu, 1);
    }
    if (nav.confirm || nav.start) && menu.focus.is_none() && menu.page() != Some(Page::Pause) {
        // the first press of a pad shows where the focus is
        menu.focus = step_focus(1);
        menu.dirty = true;
    } else if pressed(&[Enter, NumpadEnter, Space]) || nav.confirm {
        if let Some(f) = menu.focus {
            menu.sounds.push(Sound::Select);
            fx.push(activate(&mut menu, &mut settings, &roster, styles, f, Act::Press));
        }
    }
    if pressed(&[Escape]) || nav.back || (nav.start && menu.page() == Some(Page::Pause)) {
        fx.push(back(&mut menu));
    }
    for e in fx {
        if !matches!(e, Effect::None) {
            effects.write(crate::MenuEffect(e));
        }
    }
}

/// Widgets whose value left / right changes (the rest are buttons, which left / right move
/// between).
fn takes_steps(id: Id) -> bool {
    match id {
        Id::CpuLevel | Id::PauseCpu | Id::Train(_) | Id::Tab(_) | Id::Opt(_) | Id::Mods | Id::Rule(_) | Id::Hue | Id::Saturation | Id::LoadStyle(_) => true,
        Id::Pad(k) => matches!(k, 0 | 1 | 11 | 12),
        Id::Cust(c) => matches!(c, CustId::Spark | CustId::Shape | CustId::Attach | CustId::Slider(_)),
        _ => false,
    }
}

/// The widget nearest to `from` in direction `dir` (screen space, y down): the closest one
/// ahead, with sideways distance counting double so a straight neighbour wins over a diagonal.
fn nearest(rects: &[(Id, Vec2, Vec2)], from: Id, dir: Vec2) -> Option<Id> {
    let (_, at, size) = *rects.iter().find(|r| r.0 == from)?;
    rects
        .iter()
        .filter(|r| r.0 != from)
        .filter_map(|&(id, c, s)| {
            let d = c - at;
            let ahead = d.dot(dir);
            // past the edge of the widget in that direction, not just a pixel off
            let reach = (size.dot(dir.abs()) + s.dot(dir.abs())) / 4.0;
            (ahead > reach).then(|| (id, ahead + 2.0 * (d - dir * ahead).length()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// What the pads did this frame in menu terms: d-pad or stick moves the focus, A confirms,
/// B goes back.
#[derive(Default)]
struct PadNav {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    confirm: bool,
    back: bool,
    start: bool,
}

fn pad_nav(gamepads: &Query<&bevy::input::gamepad::Gamepad>, stick: &mut IVec2) -> PadNav {
    use bevy::input::gamepad::GamepadButton as B;
    let mut n = PadNav::default();
    let mut dir = IVec2::ZERO;
    for g in gamepads.iter() {
        n.up |= g.just_pressed(B::DPadUp);
        n.down |= g.just_pressed(B::DPadDown);
        n.left |= g.just_pressed(B::DPadLeft);
        n.right |= g.just_pressed(B::DPadRight);
        n.confirm |= g.just_pressed(B::South);
        n.back |= g.just_pressed(B::East);
        n.start |= g.just_pressed(B::Start);
        let v = g.left_stick();
        if v.length() > 0.6 {
            dir = if v.x.abs() > v.y.abs() { IVec2::new(v.x.signum() as i32, 0) } else { IVec2::new(0, v.y.signum() as i32) };
        }
    }
    // the stick moves one step per push
    if dir != *stick {
        n.up |= dir.y > 0;
        n.down |= dir.y < 0;
        n.left |= dir.x < 0;
        n.right |= dir.x > 0;
    }
    *stick = dir;
    n
}

/// Cursor position along a widget, 0 at the left edge and 1 at the right; and whether it is
/// in the top half (for spin boxes).
fn cursor_t(rel: Option<&RelativeCursorPosition>, node: &ComputedNode) -> (f32, bool) {
    let Some(n) = rel.and_then(|r| r.normalized) else { return (0.5, true) };
    // sliders leave half a grabber (6px) at each end
    let w = node.size().x * node.inverse_scale_factor();
    let x = (n.x + 0.5) * w;
    (((x - 6.0) / (w - 12.0).max(1.0)).clamp(0.0, 1.0), n.y < 0.0)
}

#[derive(Clone, Copy)]
enum Act {
    Press,
    Step(i32),
    Click((f32, bool)),
}

fn cycle_tab(menu: &mut Menu, d: i32) {
    let i = TABS.iter().position(|t| t.0 == menu.tab).unwrap_or(0) as i32;
    menu.tab = TABS[(i + d).rem_euclid(TABS.len() as i32) as usize].0;
    menu.scroll_options = 0.0;
    menu.dirty = true;
}

fn back(menu: &mut Menu) -> Effect {
    match menu.page() {
        Some(Page::Main | Page::Setup) => Effect::None,
        Some(Page::Pause | Page::PostGame) if menu.stack.len() == 1 => Effect::Resume,
        Some(Page::Select) if menu.picks[0].is_some() => {
            let p = if menu.picks[1].is_some() { 1 } else { 0 };
            menu.picks[p] = None;
            menu.dirty = true;
            Effect::None
        }
        _ => {
            menu.pop();
            Effect::None
        }
    }
}

fn adjust(values: &mut Values, f: &Field, act: Act) {
    let v = values.get(f);
    let next = match (f, act) {
        (Field::Check { .. }, Act::Press | Act::Click(_) | Act::Step(_)) => f.step(v, 1),
        (Field::Slider { .. }, Act::Click((t, _))) => f.at(t as f64),
        (Field::Spin { .. }, Act::Click((_, up))) => f.step(v, if up { 1 } else { -1 }),
        (Field::Choice { .. }, Act::Click(_)) => f.step(v, 1),
        (_, Act::Step(d)) => f.step(v, d),
        (_, Act::Press) => f.step(v, 1),
        _ => v,
    };
    values.set(f, next);
}

fn drag(menu: &mut Menu, s: &mut Settings, id: Id, pos: (f32, bool)) {
    if let Id::Pad(k @ (11 | 12)) = id {
        pad_slider(s, k, Some(pos.0 as f64), 0);
        menu.dirty = true;
        return;
    }
    if let Id::Cust(CustId::Slider(k)) = id {
        customize::drag(menu, k, pos.0);
        return;
    }
    let (field, values) = match id {
        Id::Opt(i) => (&OPTIONS[i], &mut s.options),
        Id::Rule(i) if matches!(RULES[i], Field::Slider { .. }) => (&RULES[i], &mut s.rules),
        Id::Hue => {
            s.name_hue = NAME_HUE.at(pos.0 as f64);
            menu.dirty = true;
            return;
        }
        Id::Saturation => {
            s.name_saturation = NAME_SATURATION.at(pos.0 as f64);
            menu.dirty = true;
            return;
        }
        _ => return,
    };
    let v = field.at(pos.0 as f64);
    if values.get(field) != v {
        values.set(field, v);
        menu.dirty = true;
    }
}

fn activate(menu: &mut Menu, s: &mut Settings, roster: &Roster, styles: &mut Styles, id: Id, act: Act) -> Effect {
    let press = !matches!(act, Act::Step(_));
    menu.dirty = true;
    match id {
        Id::Singleplayer if press => menu.push(Page::Modes),
        Id::Mode(m) if press => {
            menu.mode = match m {
                0 => Mode::Versus,
                1 => Mode::Cpu(s.cpu_level),
                _ => Mode::Training,
            };
            menu.push(Page::Select);
        }
        Id::CpuLevel => {
            let d = if let Act::Step(d) = act { d } else { 1 };
            s.cpu_level = (s.cpu_level as i32 + d).rem_euclid(3) as u8;
        }
        Id::Train(i) => adjust(&mut s.training, &TRAINING[i], act),
        Id::TrainingPage if press => menu.push(Page::Training),
        Id::Pad(k) => {
            let d = if let Act::Step(d) = act { d } else { 1 };
            let pad = &mut s.pad;
            match k {
                0 | 1 => {
                    // cycle through the connected pads (up to four) and "none"
                    let slots = [0u8, 1, 2, 3, 255];
                    let cur = slots.iter().position(|x| *x == pad.assign[k as usize]).unwrap_or(0) as i32;
                    pad.assign[k as usize] = slots[(cur + d).rem_euclid(slots.len() as i32) as usize];
                }
                2..=10 if press => menu.pad_rebinding = Some(k as usize - 2),
                11 | 12 => match act {
                    Act::Click((t, _)) => pad_slider(s, k, Some(t as f64), 0),
                    Act::Step(d) => pad_slider(s, k, None, d),
                    Act::Press => {}
                },
                13 => pad.tap_jump = !pad.tap_jump,
                14 => pad.rumble = !pad.rumble,
                15 if press => {
                    s.pad = crate::pad::PadSettings::default();
                    menu.status = "controller settings reset".into();
                }
                _ => {}
            }
        }
        Id::Restart if press => return Effect::Restart,
        Id::CharSelect if press => return Effect::ToSelect,
        Id::Record if press => return Effect::Record,
        Id::PlayRecording if press => return Effect::PlayRecording,
        Id::PauseCpu => {
            let d = if let Act::Step(d) = act { d } else { 1 };
            s.cpu_level = (s.cpu_level as i32 + d).rem_euclid(3) as u8;
            return Effect::ApplyCpu;
        }
        Id::Customize if press => menu.push(Page::Customize),
        Id::Cust(c) => {
            let (step, click) = match act {
                Act::Step(d) => (d, None),
                Act::Click((t, _)) => (0, Some(t)),
                Act::Press => (0, None),
            };
            let click_press = matches!(act, Act::Click(_)) && !matches!(c, CustId::Slider(_));
            return customize::activate(menu, styles, choices(roster).len(), c, press && (matches!(act, Act::Press) || click_press), step, click);
        }
        Id::Feature(f) if press => menu.push(Page::Unavailable(f)),
        Id::Options if press => menu.push(Page::Options),
        Id::PauseOptions if press => menu.push(Page::Options),
        Id::MoveList if press => menu.push(Page::MoveList(0)),
        Id::MoveListOther if press => {
            if let Some(Page::MoveList(i)) = menu.page() {
                menu.pop();
                menu.push(Page::MoveList(1 - i));
            }
        }
        Id::Credits if press => menu.push(Page::Credits),
        Id::Quit if press => return Effect::Quit,
        Id::Roadmap if press => menu.roadmap = !menu.roadmap,
        Id::IvySly if press => open_url(IVY_SLY_URL),
        Id::Discord if press => open_url(DISCORD_URL),
        Id::Twitter if press => open_url(TWITTER_URL),
        Id::TikTok if press => open_url(TIKTOK_URL),
        Id::Tab(i) if press => {
            menu.tab = TABS[i].0;
            menu.scroll_options = 0.0;
        }
        Id::Tab(_) => {
            if let Act::Step(d) = act {
                cycle_tab(menu, d)
            }
        }
        Id::Opt(i) => adjust(&mut s.options, &OPTIONS[i], act),
        Id::Mods => adjust(&mut s.options, &MODS_TOGGLE, act),
        Id::DeleteCache if press => menu.push(Page::Setup),
        Id::SetupUse(i) if press => {
            if let Some(f) = menu.setup.detected.get(i).cloned() {
                menu.setup.use_folder(f.game_dir);
            }
        }
        Id::SetupBrowse if press => menu.setup.browse(),
        Id::SetupSkip if press => menu.reset_to(&[Page::Main]),
        Id::SetupBack if press => {
            menu.setup.step = setup::Step::Welcome;
            menu.setup.reset();
        }
        Id::SetupBuild if press => menu.setup.start(),
        Id::SetupDownload if press => menu.setup.allow_download = !menu.setup.allow_download,
        Id::SetupStart if press => {
            menu.reset_to(&[Page::Main]);
            return Effect::LibraryReady;
        }
        Id::Done | Id::Back if press => menu.pop(),
        Id::NameEdit if press => menu.editing = Some(Id::NameEdit),
        Id::NameReset if press => s.name.clear(),
        Id::Hue => match act {
            Act::Click((t, _)) => s.name_hue = NAME_HUE.at(t as f64),
            Act::Step(d) => s.name_hue = NAME_HUE.step(s.name_hue, d),
            Act::Press => {}
        },
        Id::Saturation => match act {
            Act::Click((t, _)) => s.name_saturation = NAME_SATURATION.at(t as f64),
            Act::Step(d) => s.name_saturation = NAME_SATURATION.step(s.name_saturation, d),
            Act::Press => {}
        },
        Id::ColorReset if press => {
            s.name_hue = 0.0;
            s.name_saturation = 0.0;
        }
        Id::Bind(row) if press => menu.rebinding = Some(row),
        Id::Char(i) if press => {
            if let Some(p) = menu.picking() {
                menu.picks[p] = Some(i);
                menu.shown[p] = Some(i);
                menu.named[p] = true;
            }
        }
        Id::Go if press => {
            if let [Some(a), Some(b)] = menu.picks {
                let all = choices(roster);
                let idx = |i: usize| all.get(i).and_then(|(_, id)| roster.0.iter().position(|r| &r.id == id)).unwrap_or(0);
                return Effect::StartFight([idx(a), idx(b)], menu.mode);
            }
        }
        Id::SelectQuit if press => menu.pop(),
        Id::Rule(i) => adjust(&mut s.rules, &RULES[i], act),
        Id::FormatName if press => menu.editing = Some(Id::FormatName),
        Id::LoadStyle(pl) => {
            let d = if let Act::Step(d) = act { d } else { 1 };
            s.styles[pl] = styles.cycle(&s.styles[pl], d);
        }
        Id::FormatSave | Id::Format | Id::SaveReplay if press => {
            menu.status = "not available yet".into();
        }
        Id::Resume if press => return Effect::Resume,
        Id::MainMenu if press => return Effect::ToMenu(vec![Page::Main]),
        Id::Forfeit if press => return Effect::Forfeit,
        Id::Rematch if press => return Effect::Rematch,
        _ => {}
    }
    Effect::None
}

/// Sets a Controller tab slider from a position along it, or steps it.
fn pad_slider(s: &mut Settings, k: u8, at: Option<f64>, step: i32) {
    let ((lo, hi), v) = if k == 11 { (PAD_DEADZONE, &mut s.pad.deadzone) } else { (PAD_DIAGONAL, &mut s.pad.diagonal) };
    let x = match at {
        Some(t) => lo + t.clamp(0.0, 1.0) * (hi - lo),
        None => *v + step as f64 * 0.05,
    };
    *v = ((x.clamp(lo, hi)) * 100.0).round() / 100.0;
}

fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

/// Plays queued hover and select sounds (the original's ButtonSoundPlayer: -20 dB / -8 dB).
pub fn play_sounds(mut commands: Commands, mut menu: ResMut<Menu>, skin: Res<Skin>, settings: Res<Settings>) {
    if menu.sounds.is_empty() {
        return;
    }
    let vol = |k: &str| OPTIONS.iter().find(|f| f.key() == Some(k)).map(|f| settings.options.get(f)).unwrap_or(1.0);
    let ui = (vol("ui_volume") * vol("master_volume")) as f32;
    for s in std::mem::take(&mut menu.sounds) {
        let (h, db) = match s {
            Sound::Hover => (&skin.hover, -20.0),
            Sound::Select => (&skin.select, -8.0),
        };
        if let Some(h) = h {
            let v = 10f32.powf(db / 20.0) * ui;
            commands.spawn((AudioPlayer(h.clone()), PlaybackSettings::DESPAWN.with_volume(bevy::audio::Volume::Linear(v))));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Building pages
// ---------------------------------------------------------------------------------------------

pub(crate) struct Ui<'a> {
    pub(crate) menu: &'a Menu,
    pub(crate) settings: &'a Settings,
    pub(crate) skin: &'a Skin,
    pub(crate) roster: &'a Roster,
    pub(crate) order: Vec<Id>,
    pub(crate) pads: &'a crate::pad::Pads,
}

impl Ui<'_> {
    fn hovered(&self, id: Id) -> bool {
        self.menu.hovered == Some(id)
    }

    fn focused(&self, id: Id) -> bool {
        self.menu.focus == Some(id)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_menu(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    settings: Res<Settings>,
    skin: Res<Skin>,
    roster: Res<Roster>,
    game: Option<Res<Game>>,
    roots: Query<Entity, With<MenuRoot>>,
    mut rings: ResMut<RingsShown>,
    styles: Res<Styles>,
    pads: Res<crate::pad::Pads>,
) {
    if !menu.dirty && !settings.is_changed() && !styles.is_changed() && !pads.is_changed() {
        return;
    }
    menu.dirty = false;
    for e in &roots {
        commands.entity(e).despawn();
    }
    let in_fight = game.is_some() && menu.stack.first().is_some_and(|p| matches!(p, Page::Pause | Page::PostGame));
    rings.0 = menu.is_open() && !in_fight && menu.page() != Some(Page::Customize);
    let Some(page) = menu.page() else { return };
    let root = commands.spawn((MenuRoot, Canvas, canvas_node(), GlobalZIndex(10))).id();
    let mut ui = Ui { menu: &menu, settings: &settings, skin: &skin, roster: &roster, order: vec![], pads: &pads };
    commands.entity(root).with_children(|p| {
        match page {
            Page::Main => build_main(p, &mut ui, true),
            Page::Options if in_fight => build_options(p, &mut ui),
            Page::Options => {
                build_main(p, &mut ui, false);
                build_options(p, &mut ui);
            }
            Page::Credits => {
                build_main(p, &mut ui, false);
                build_credits(p, &mut ui);
            }
            Page::Unavailable(f) => {
                build_main(p, &mut ui, false);
                build_dialog(p, &mut ui, f);
            }
            Page::Select => build_select(p, &mut ui),
            Page::Setup => setup::build_page(p, &mut ui),
            Page::Pause => build_pause(p, &mut ui, game.as_deref()),
            Page::PostGame => build_postgame(p, &mut ui, game.as_deref()),
            Page::MoveList(i) => build_move_list(p, &mut ui, game.as_deref(), i),
            Page::Modes => {
                build_main(p, &mut ui, false);
                build_modes(p, &mut ui);
            }
            Page::Training => build_training(p, &mut ui),
            Page::Customize => {
                let all = choices(ui.roster);
                customize::build(p, &mut ui, &styles, &all)
            }
        }
    });
    let order = std::mem::take(&mut ui.order);
    menu.order = order;
}

/// A theme button. `pressed` draws the toggled-on style.
#[allow(clippy::too_many_arguments)]
pub(crate) fn button(p: &mut ChildSpawnerCommands, ui: &mut Ui, id: Id, x: f32, y: f32, w: f32, h: f32, label: &str, pressed: bool, disabled: bool) {
    ui.order.push(id);
    let style = if disabled {
        BTN_DISABLED
    } else if pressed {
        BTN_PRESSED
    } else if ui.hovered(id) {
        BTN_HOVER
    } else {
        BTN_NORMAL
    };
    let color = if disabled { FONT_DISABLED } else { FONT };
    let mut e = p.spawn((sb(style, Node { justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..at(x, y, w, h) }),));
    if !disabled {
        e.insert((Button, Clickable(id)));
    }
    let skin = ui.skin;
    let focused = ui.focused(id);
    e.with_children(|b| {
        text(b, skin, label, color, None);
        if focused {
            focus_line(b, w);
        }
    });
}

/// The theme's focus style: a light line along the bottom.
fn focus_line(p: &mut ChildSpawnerCommands, w: f32) {
    p.spawn((Node { position_type: PositionType::Absolute, left: px(0), bottom: px(-1), width: px(w), height: px(1), ..default() }, BackgroundColor(Color::srgb(0.8, 0.8, 0.8))));
}

// ---- main menu ------------------------------------------------------------------------------

fn build_main(p: &mut ChildSpawnerCommands, ui: &mut Ui, interactive: bool) {
    let skin = ui.skin;
    // the game's logo where the original's sits, the tagline under it
    match skin.image("logo") {
        Some(img) => {
            let (x, y, w, h) = LOGO;
            p.spawn((ImageNode::new(img), at(x, y, w, h)));
            p.spawn(Node { justify_content: JustifyContent::Center, ..at(0.0, 87.0, VW, 11.0) }).with_children(|t| text(t, skin, TAGLINE, RED, None));
        }
        None => {
            p.spawn(Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..at(0.0, 22.0, VW, 70.0) }).with_children(|t| {
                big_text(t, skin, TITLE_LINE_1, Color::WHITE, 18.0, true, 1.0);
                t.spawn(Node { margin: UiRect::top(px(-4)), ..default() }).with_children(|h| big_text(h, skin, TITLE_LINE_2, Color::WHITE, 36.0, true, 2.0));
                t.spawn(Node { margin: UiRect::top(px(-3)), ..default() }).with_children(|s| text(s, skin, TAGLINE, RED, None));
            });
        }
    }
    let hov = interactive && ui.hovered(Id::IvySly);
    let mut e = p.spawn(Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..at(0.0, 99.0, VW, 11.0) });
    if interactive {
        e.insert((Button, Clickable(Id::IvySly)));
    }
    e.with_children(|l| {
        l.spawn(Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Stretch, ..default() }).with_children(|u| {
            text(u, skin, CREDIT_LINE, if hov { FONT } else { LINK_BLUE }, None);
            u.spawn((Node { height: px(1), margin: UiRect { top: px(-3), left: px(1), right: px(1), ..default() }, ..default() }, BackgroundColor(LINK_BLUE)));
        });
    });
    // the button column (ButtonContainer, 102px wide, 20px rows from y 114)
    let entries: [(Id, &str); 9] = [
        (Id::Singleplayer, "Singleplayer"),
        (Id::Feature(Feature::SteamMultiplayer), "Multiplayer (Steam)"),
        (Id::Feature(Feature::LegacyMultiplayer), "Multiplayer (Legacy)"),
        (Id::Feature(Feature::DirectConnect), "Direct Connect"),
        (Id::Customize, "Customize"),
        (Id::Feature(Feature::Replays), "Replays"),
        (Id::Options, "Options"),
        (Id::Credits, "Credits"),
        (Id::Quit, "Quit"),
    ];
    if interactive {
        for (i, (id, name)) in entries.iter().enumerate() {
            ui.order.push(*id);
            let color = if ui.hovered(*id) { RED } else { FONT };
            let focused = ui.focused(*id);
            p.spawn((Button, Clickable(*id), Node { justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..at(272.0, 114.0 + 20.0 * i as f32, 102.0, 20.0) }))
                .with_children(|b| {
                    text(b, skin, name, color, None);
                    if focused {
                        focus_line(b, 102.0);
                    }
                });
        }
    }
    // roadmap toggle and panel (top left)
    if interactive {
        button(p, ui, Id::Roadmap, 2.0, 2.0, 46.0, 12.0, "ROADMAP", ui.menu.roadmap, false);
    } else {
        p.spawn(sb(BTN_NORMAL, Node { justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..at(2.0, 2.0, 46.0, 12.0) }))
            .with_children(|b| text(b, skin, "ROADMAP", FONT, None));
    }
    if ui.menu.roadmap {
        let paras: Vec<(String, Color)> = if skin.roadmap.len() == 3 {
            vec![
                (skin.roadmap[0].clone(), FONT),
                (skin.roadmap[1].clone(), Color::srgb(0.392 * 0.878, 0.824 * 0.878, 0.420 * 0.878)),
                (skin.roadmap[2].clone(), FONT.with_alpha(0.482)),
            ]
        } else {
            vec![
                ("A real-time take on a turn-based fighter.".into(), FONT),
                ("Complete:\n- Simulation\n- Five characters as data\n- Menus".into(), Color::srgb(0.344, 0.723, 0.368)),
                ("Incomplete:\n- Art\n- Netcode\n- Replays\n- Mods".into(), FONT.with_alpha(0.482)),
            ]
        };
        // RoadmapListContainer: three autowrap labels at their scene heights (81, 39, 67 + 1px gaps)
        for ((t, color), y) in paras.into_iter().zip([15.0, 97.0, 137.0]) {
            p.spawn(at(2.0, y - 1.5, 155.0, 80.0)).with_children(|c| text(c, skin, &t, color, Some((155.0, Justify::Left))));
        }
    }
    // community events (top right)
    if ui.settings.flag("community_events") {
        label(p, skin, 526.0, 2.0, 112.0, 11.0, "COMMUNITY EVENTS", FONT, Align::Center);
    }
    // version (bottom left) and social links (bottom right)
    label(p, skin, 2.0, 347.0, 300.0, 11.0, &format!("version {}", env!("CARGO_PKG_VERSION")), FONT, Align::Left);
    for (i, (id, name)) in [(Id::Discord, "discord"), (Id::Twitter, "twitter"), (Id::TikTok, "tiktok")].into_iter().enumerate() {
        if let Some(img) = skin.image(name) {
            let mut e = p.spawn(at(573.0 + 22.0 * i as f32, 335.0, 22.0, 24.0));
            if interactive {
                e.insert((Button, Clickable(id)));
            }
            e.with_child((ImageNode::new(img), at(2.0, 2.0, 20.0, 20.0)));
        }
    }
}

// ---- options --------------------------------------------------------------------------------

/// A CheckButton row: label left, switch icon right, pressed style while on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_row(p: &mut ChildSpawnerCommands, ui: &mut Ui, id: Id, x: f32, y: f32, w: f32, label_text: &str, on: bool, color: Color) {
    ui.order.push(id);
    let style = if on { BTN_PRESSED } else if ui.hovered(id) { BTN_HOVER } else { BTN_NORMAL };
    let icon = ui.skin.image(if on { "check_on" } else { "check_off" });
    let skin = ui.skin;
    let focused = ui.focused(id);
    p.spawn((Button, Clickable(id), sb(style, Node { align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, ..at(x, y, w, 12.0) })))
        .with_children(|r| {
            text(r, skin, label_text, color, None);
            match icon {
                Some(img) => {
                    // pinned like Godot places it: 2px from the row's top, against the right border
                    let top = if on { 1.0 } else { 2.0 };
                    r.spawn((ImageNode::new(img), Node { position_type: PositionType::Absolute, right: px(0), top: px(top), width: px(16), height: px(9), ..default() }));
                }
                None => text(r, skin, if on { "on" } else { "off" }, color, None),
            }
            if focused {
                focus_line(r, w);
            }
        });
}

/// An HSlider: black track, white fill up to the grabber, the grabber image.
pub(crate) fn slider(p: &mut ChildSpawnerCommands, ui: &mut Ui, id: Id, t: f32, node: Node) {
    ui.order.push(id);
    let grabber = ui.skin.image("grabber");
    let focused = ui.focused(id);
    p.spawn((Button, Clickable(id), RelativeCursorPosition::default(), Node { height: px(12), ..node })).with_children(|s| {
        // track: the theme's slider box is 6px of black border
        s.spawn((Node { position_type: PositionType::Absolute, left: px(0), right: px(0), top: px(3), height: px(6), ..default() }, BackgroundColor(BLACK)));
        // the inner area leaves half a grabber at each end, as Godot does
        s.spawn(Node { position_type: PositionType::Absolute, left: px(6), right: px(6), top: px(0), bottom: px(0), ..default() }).with_children(|a| {
            a.spawn((Node { position_type: PositionType::Absolute, left: px(-6), width: px(6), top: px(3), height: px(6), ..default() }, BackgroundColor(Color::WHITE)));
            a.spawn((Node { position_type: PositionType::Absolute, left: px(0), width: percent(t * 100.0), top: px(3), height: px(6), ..default() }, BackgroundColor(Color::WHITE)));
            let g = Node { position_type: PositionType::Absolute, left: percent(t * 100.0), margin: UiRect::left(px(-6)), top: px(0), width: px(12), height: px(12), ..default() };
            match grabber {
                Some(img) => {
                    a.spawn((ImageNode::new(img), g));
                }
                None => {
                    a.spawn((g, BackgroundColor(Color::WHITE)));
                }
            }
        });
        if focused {
            focus_line(s, 0.0);
        }
    });
}

fn separator(p: &mut ChildSpawnerCommands, x: f32, y: f32, w: f32) {
    p.spawn((at(x, y + 1.0, w, 1.0), BackgroundColor(SEPARATOR)));
}

fn build_options(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let skin = ui.skin;
    // OptionsContainer: 224x274 centred; title bar, tabs, the tab's list, then the bottom rows
    p.spawn((at(208.0, 43.0, 224.0, 274.0), BackgroundColor(BLACK)));
    p.spawn((at(209.0, 44.0, 222.0, 9.0), BackgroundColor(PANEL.bg)));
    label(p, skin, 209.0, 43.0, 222.0, 11.0, "Options", FONT, Align::Center);
    // tab strip (TabContainer side margin 8)
    p.spawn((at(210.0, 55.0, 220.0, 11.0), BackgroundColor(PANEL.bg)));
    p.spawn(Node { flex_direction: FlexDirection::Row, ..at(218.0, 54.0, 212.0, 12.0) }).with_children(|tabs| {
        for (i, (tab, name)) in TABS.iter().enumerate() {
            let on = *tab == ui.menu.tab;
            ui.order.push(Id::Tab(i));
            let focused = ui.focused(Id::Tab(i));
            let style = if on { TAB_FG } else { TAB_BG };
            tabs.spawn((Button, Clickable(Id::Tab(i)), Node { height: px(12), align_items: AlignItems::Center, padding: UiRect::left(px(2)), ..default() }))
                .with_children(|t| {
                    // the tab's box sits lower than its text
                    t.spawn((Node { position_type: PositionType::Absolute, left: px(0), right: px(0), top: px(3), height: px(9), ..default() }, BackgroundColor(style.bg)));
                    text(t, skin, name, if on { Color::srgb(0.94, 0.94, 0.94) } else { Color::srgb(0.69, 0.69, 0.69) }, None);
                    if focused {
                        focus_line(t, 40.0);
                    }
                });
        }
    });
    // tab panel
    let (lx, ly, lw, lh) = (210.0, 66.0, 220.0, 207.0);
    p.spawn((at(lx, ly, lw, lh), BackgroundColor(PANEL.bg)));
    match ui.menu.tab {
        Tab::Options => {
            let rows: Vec<(usize, f32)> = {
                let mut y = 0.0;
                OPTIONS
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let at_y = y;
                        y += match f {
                            Field::Separator => 5.0,
                            _ => 13.0,
                        };
                        (i, at_y)
                    })
                    .collect()
            };
            let content_h = rows.last().map(|r| r.1 + 12.0).unwrap_or(0.0);
            scrolled_list(p, ui, lx, ly, lw, lh, content_h, ui.menu.scroll_options, |c, ui, scroll| {
                for (i, y) in &rows {
                    let y = y - scroll;
                    if y < -13.0 || y > lh {
                        continue;
                    }
                    match OPTIONS[*i] {
                        Field::Separator => separator(c, 0.0, y, 214.0),
                        Field::Check { label: l, .. } => {
                            let on = ui.settings.options.on(&OPTIONS[*i]);
                            check_row(c, ui, Id::Opt(*i), 0.0, y, 214.0, l, on, FONT);
                        }
                        Field::Slider { label: l, .. } => {
                            let t = ui.settings.options.get(&OPTIONS[*i]) as f32;
                            let skin = ui.skin;
                            c.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(1), ..at(0.0, y, 214.0, 12.0) })
                                .with_children(|r| {
                                    text(r, skin, l, FONT, None);
                                    slider(r, ui, Id::Opt(*i), t, Node { flex_grow: 1.0, ..default() });
                                });
                        }
                        Field::Spin { .. } | Field::Choice { .. } => {}
                    }
                }
            });
        }
        Tab::Hotkeys => {
            let rows = bind_rows();
            let content_h = rows.len() as f32 * 13.0;
            let mut s = ui.settings.clone();
            scrolled_list(p, ui, lx, ly, lw, lh, content_h, ui.menu.scroll_options, |c, ui, scroll| {
                for (i, name) in rows.iter().enumerate() {
                    let y = i as f32 * 13.0 - scroll;
                    if !(-13.0..=lh).contains(&y) {
                        continue;
                    }
                    let skin = ui.skin;
                    label(c, skin, 0.0, y, 134.0, 12.0, name, FONT, Align::Left);
                    let key = if ui.menu.rebinding == Some(i) { "...".to_string() } else { bind_key(&mut s, i).label() };
                    button(c, ui, Id::Bind(i), 134.0, y, 80.0, 12.0, &key, false, false);
                }
            });
        }
        Tab::Controller => {
            let gray = Color::srgb_u8(0x8c, 0x8c, 0x8c);
            let n = ui.pads.connected.len();
            let content_h = PAD_ROWS as f32 * 13.0 + 26.0;
            scrolled_list(p, ui, lx, ly, lw, lh, content_h, ui.menu.scroll_options, |c, ui, scroll| {
                let skin = ui.skin;
                let head = match n {
                    0 => "no controller connected".to_string(),
                    1 => "1 controller connected".to_string(),
                    n => format!("{n} controllers connected"),
                };
                label(c, skin, 0.0, -scroll, 214.0, 11.0, &head, gray, Align::Left);
                let pad = ui.settings.pad.clone();
                for k in 0..PAD_ROWS {
                    let y = 13.0 + k as f32 * 13.0 - scroll + if k >= 11 { 6.0 } else if k >= 2 { 3.0 } else { 0.0 };
                    if !(-13.0..=lh).contains(&y) {
                        continue;
                    }
                    let id = Id::Pad(k);
                    match k {
                        0 | 1 => {
                            label(c, skin, 0.0, y, 80.0, 11.0, if k == 0 { "P1 controller" } else { "P2 controller" }, FONT, Align::Left);
                            button(c, ui, id, 80.0, y, 134.0, 12.0, &ui.pads.slot_name(pad.assign[k as usize]), false, false);
                        }
                        2..=10 => {
                            let a = k as usize - 2;
                            label(c, skin, 0.0, y, 110.0, 11.0, crate::pad::ACTIONS[a], FONT, Align::Left);
                            let shown = if ui.menu.pad_rebinding == Some(a) { "press a button...".to_string() } else { crate::pad::label(pad.binds.get(a).map(String::as_str).unwrap_or("")).to_string() };
                            button(c, ui, id, 110.0, y, 104.0, 12.0, &shown, false, false);
                        }
                        11 | 12 => {
                            let ((lo, hi), v, l) = if k == 11 { (PAD_DEADZONE, pad.deadzone, "stick dead zone") } else { (PAD_DIAGONAL, pad.diagonal, "narrow diagonals") };
                            label(c, skin, 0.0, y, 90.0, 11.0, l, FONT, Align::Left);
                            slider(c, ui, id, ((v - lo) / (hi - lo)) as f32, at(90.0, y, 100.0, 12.0));
                            label(c, skin, 192.0, y, 22.0, 11.0, &format!("{v:.2}"), FONT, Align::Left);
                        }
                        13 => check_row(c, ui, id, 0.0, y, 214.0, "stick up jumps (off: jump button)", pad.tap_jump, FONT),
                        14 => check_row(c, ui, id, 0.0, y, 214.0, "rumble on hits", pad.rumble, FONT),
                        _ => button(c, ui, id, 0.0, y, 214.0, 12.0, "reset controller settings", false, false),
                    }
                }
            });
        }
        Tab::Personalization => {
            let s = ui.settings;
            label(p, skin, lx, ly, lw, 11.0, "Legacy Multiplayer/Character Name", FONT, Align::Left);
            let editing = ui.menu.editing == Some(Id::NameEdit);
            ui.order.push(Id::NameEdit);
            let name = if editing { format!("{}|", s.name) } else { s.name.clone() };
            p.spawn((Button, Clickable(Id::NameEdit), sb(if editing { LINE_EDIT_FOCUS } else { LINE_EDIT }, Node { align_items: AlignItems::Center, padding: UiRect::left(px(2)), ..at(lx, ly + 12.0, 190.0, 11.0) })))
                .with_children(|l| text(l, skin, &name, FONT, None));
            button(p, ui, Id::NameReset, lx + 191.0, ly + 12.0, 29.0, 11.0, "reset", false, false);
            separator(p, lx, ly + 24.0, lw);
            label(p, skin, lx, ly + 29.0, lw, 11.0, "Name Color", FONT, Align::Left);
            for (k, (id, l, v)) in [(Id::Hue, "hue", s.name_hue), (Id::Saturation, "saturation", s.name_saturation)].into_iter().enumerate() {
                let y = ly + 41.0 + 13.0 * k as f32;
                label(p, skin, lx, y, 100.0, 12.0, l, FONT, Align::Left);
                slider(p, ui, id, v as f32, at(lx + 109.0, y, 111.0, 12.0));
            }
            let preview = if s.name.is_empty() { "preview".to_string() } else { s.name.clone() };
            label(p, skin, lx, ly + 67.0, lw, 11.0, &preview, if s.name_saturation > 0.0 { s.name_color() } else { FONT }, Align::Center);
            button(p, ui, Id::ColorReset, lx, ly + 79.0, lw, 11.0, "reset color", false, false);
        }
    }
    // bottom rows
    separator(p, 210.0, 271.0, 220.0);
    button(p, ui, Id::DeleteCache, 210.0, 277.0, 220.0, 12.0, "rebuild game library", false, false);
    let mods = ui.settings.options.on(&MODS_TOGGLE);
    check_row(p, ui, Id::Mods, 210.0, 290.0, 220.0, "enable mods (must restart)", mods, Color::srgb_u8(0x64, 0xe5, 0xff));
    button(p, ui, Id::Done, 210.0, 303.0, 220.0, 12.0, "done", false, false);
    if !ui.menu.status.is_empty() {
        label(p, skin, 208.0, 318.0, 224.0, 11.0, &ui.menu.status, FONT, Align::Center);
    }
}

/// A clipped list with the theme's scroll bar at the right; `build` places rows at y - scroll.
#[allow(clippy::too_many_arguments)]
fn scrolled_list(
    p: &mut ChildSpawnerCommands,
    ui: &mut Ui,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    content_h: f32,
    scroll: f32,
    build: impl FnOnce(&mut ChildSpawnerCommands, &mut Ui, f32),
) {
    let max = (content_h - h).max(0.0);
    let scroll = scroll.min(max);
    p.spawn(Node { overflow: Overflow::clip(), ..at(x, y, w - 6.0, h) }).with_children(|c| build(c, ui, scroll));
    if max > 0.0 {
        p.spawn((at(x + w - 6.0, y, 6.0, h), BackgroundColor(SCROLL.bg)));
        let gh = (h * h / content_h).max(10.0);
        let gy = (h - gh) * scroll / max;
        p.spawn((at(x + w - 6.0, y + gy, 6.0, gh), BackgroundColor(GRABBER)));
    }
}

// ---- credits --------------------------------------------------------------------------------

fn build_credits(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let skin = ui.skin;
    p.spawn((at(200.0, 8.0, 240.0, 344.0), BackgroundColor(BLACK)));
    label(p, skin, 200.0, 10.0, 240.0, 11.0, "Credits", FONT, Align::Center);
    // the original's layout: a grey heading, its lines 14px apart, 21px between sections
    let gray = Color::srgb_u8(0x8c, 0x8c, 0x8c);
    let sections: [(&str, &[&str]); 4] = [
        ("based on:", &["Your Only Move Is HUSTLE by Ivy Sly"]),
        ("real-time engine, simulation and tools:", &["this project's contributors"]),
        ("built with:", &["Rust and the Bevy engine"]),
        ("note:", &["an unofficial fan project,", "not affiliated with the original game"]),
    ];
    let mut y = 31.0;
    for (head, lines) in sections {
        label(p, skin, 200.0, y, 240.0, 11.0, head, gray, Align::Center);
        for l in lines {
            y += if *l == lines[0] { 12.0 } else { 14.0 };
            label(p, skin, 200.0, y, 240.0, 11.0, l, FONT, Align::Center);
        }
        y += 21.0;
    }
    button(p, ui, Id::Back, 200.0, 340.0, 240.0, 12.0, "back", false, false);
}

/// Greedy word wrap by character count (the pixel font is close to monospaced).
fn wrap_words(s: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for w in s.split_whitespace() {
        let cur = lines.last_mut().expect("one line");
        if !cur.is_empty() && cur.len() + 1 + w.len() > width {
            lines.push(w.to_string());
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(w);
        }
    }
    lines
}

fn build_dialog(p: &mut ChildSpawnerCommands, ui: &mut Ui, f: Feature) {
    let skin = ui.skin;
    p.spawn((at(200.0, 130.0, 240.0, 100.0), BackgroundColor(BLACK)));
    p.spawn((at(201.0, 131.0, 238.0, 9.0), BackgroundColor(PANEL.bg)));
    label(p, skin, 201.0, 130.0, 238.0, 11.0, f.title(), FONT, Align::Center);
    for (k, line) in wrap_words(f.explanation(), 42).iter().enumerate() {
        label(p, skin, 201.0, 146.0 + 14.0 * k as f32, 238.0, 11.0, line, FONT, Align::Center);
    }
    button(p, ui, Id::Back, 201.0, 216.0, 238.0, 12.0, "back", false, false);
}

// ---- character select -----------------------------------------------------------------------

fn build_select(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let skin = ui.skin;
    let menu = ui.menu;
    // P1 / P2 displays: label, 64x64 portrait, name, "load style"
    for pl in 0..2 {
        let x = if pl == 0 { 0.0 } else { 427.0 };
        label(p, skin, x, 53.0, 213.0, 11.0, if pl == 0 { "P1" } else { "P2" }, FONT, Align::Center);
        if let Some(ci) = menu.shown[pl] {
            let all = choices(ui.roster);
            let id = all[ci].1.as_str();
            if let Some(img) = skin.image(&format!("portrait_{id}")) {
                p.spawn((ImageNode { flip_x: pl == 1, ..ImageNode::new(img) }, at(x + 74.0 + pl as f32, 65.0, 64.0, 64.0)));
            }
            if menu.named[pl] {
                let name = ui.roster.0.iter().find(|r| r.id == id).map(|r| r.name.clone()).unwrap_or_else(|| all[ci].0.clone());
                label(p, skin, x, 130.0, 213.0, 11.0, &name, FONT, Align::Center);
            }
        }
        // the style picker (an OptionButton with the up/down arrow)
        let id = Id::LoadStyle(pl);
        ui.order.push(id);
        let style = if ui.hovered(id) { BTN_HOVER } else { BTN_NORMAL };
        p.spawn((Button, Clickable(id), sb(style, Node { align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, padding: UiRect::top(px(1)), ..at(x + 60.0 + pl as f32, 142.0, 91.0, 12.0) })))
            .with_children(|b| {
                let st = &ui.settings.styles[pl];
                text(b, skin, if st.is_empty() { "default style" } else { st.as_str() }, FONT, None);
                if let Some(img) = skin.image("updown") {
                    b.spawn((ImageNode::new(img), Node { width: px(10), height: px(10), ..default() }));
                }
            });
    }
    // match settings (GameSettingsPanelContainer): scrolling rows, then the format picker
    let (lx, ly, lw, lh) = (241.0, 8.0, 161.0, 120.0);
    let visible: Vec<usize> = (0..RULES.len()).filter(|i| rule_visible(&RULES[*i], &ui.settings.rules)).collect();
    let heights: Vec<f32> = visible.iter().map(|i| if matches!(RULES[*i], Field::Spin { .. }) { 12.0 } else { 13.0 }).collect();
    let content_h: f32 = heights.iter().sum();
    let scroll = menu.scroll_rules;
    scrolled_list(p, ui, lx, ly, lw, lh, content_h, scroll, |c, ui, scroll| {
        let mut y = -scroll;
        for (k, i) in visible.iter().enumerate() {
            let f = RULES[*i];
            let row_h = heights[k];
            if y > -row_h && y < lh {
                rule_row(c, ui, *i, &f, y, 155.0);
            }
            y += row_h;
        }
    });
    // format picker and name
    let id = Id::Format;
    ui.order.push(id);
    p.spawn((Button, Clickable(id), sb(BLUE_BUTTON, Node { align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, padding: UiRect::top(px(1)), ..at(lx, 129.0, lw, 12.0) })))
        .with_children(|b| {
            text(b, skin, "default", FONT, None);
            if let Some(img) = skin.image("updown") {
                b.spawn((ImageNode::new(img), Node { width: px(10), height: px(10), ..default() }));
            }
        });
    let editing = menu.editing == Some(Id::FormatName);
    ui.order.push(Id::FormatName);
    let name = &ui.settings.format_name;
    p.spawn((Button, Clickable(Id::FormatName), sb(if editing { LINE_EDIT_FOCUS } else { LINE_EDIT }, Node { align_items: AlignItems::Center, padding: UiRect::left(px(2)), ..at(lx, 142.0, 135.0, 12.0) })))
        .with_children(|l| {
            if name.is_empty() && !editing {
                text(l, skin, "match format name", Color::srgb_u8(0x6a, 0x6a, 0x6a), None);
            } else {
                text(l, skin, &if editing { format!("{name}|") } else { name.clone() }, FONT, None);
            }
        });
    button(p, ui, Id::FormatSave, lx + 135.0, 142.0, 26.0, 13.0, "save", false, false);

    // "P1 SELECT YOUR CHARACTER" (dodger blue), then red for P2
    let (prompt, color) = match menu.picking() {
        Some(0) => ("P1 SELECT YOUR CHARACTER", Color::srgb(0.118 * 0.878, 0.565 * 0.878, 0.878)),
        Some(_) => ("P2 SELECT YOUR CHARACTER", Color::srgb(0.878, 0.0, 0.0)),
        None => ("P2 SELECT YOUR CHARACTER", Color::srgb(0.878, 0.0, 0.0)),
    };
    label(p, skin, 28.0, 159.0, 584.0, 11.0, &format!("{prompt}   ({})", menu.mode.label()), color, Align::Center);
    // character buttons: 60x40, 1px apart, centred
    let all = choices(ui.roster);
    let per_row = 10usize;
    let rows = all.len().div_ceil(per_row).max(1);
    let all_picked = menu.picking().is_none();
    for (row, chunk) in all.chunks(per_row).enumerate() {
        let n = chunk.len() as f32;
        let x0 = (VW - (n * 60.0 + (n - 1.0))) / 2.0;
        for (k, (name, _)) in chunk.iter().enumerate() {
            let i = row * per_row + k;
            button(p, ui, Id::Char(i), x0 + 61.0 * k as f32, 174.0 + 41.0 * row as f32, 60.0, 40.0, name, false, all_picked);
        }
    }
    // GO at y 177 + button rows height, quit at the bottom
    let extra = 41.0 * (rows - 1) as f32;
    button(p, ui, Id::Go, 280.0, 217.0 + extra, 80.0, 34.0, "GO", false, !all_picked);
    button(p, ui, Id::SelectQuit, 280.0, 338.0, 80.0, 18.0, "quit", false, false);
    if !menu.status.is_empty() {
        label(p, skin, 0.0, 256.0 + extra, VW, 11.0, &menu.status, FONT, Align::Center);
    }
}

/// One match-settings row: spin box, slider with its number, or check button.
fn rule_row(c: &mut ChildSpawnerCommands, ui: &mut Ui, i: usize, f: &Field, y: f32, w: f32) {
    let skin = ui.skin;
    let id = Id::Rule(i);
    let v = ui.settings.rules.get(f);
    match *f {
        Field::Spin { label: l, .. } => {
            ui.order.push(id);
            let focused = ui.focused(id);
            c.spawn(Node { flex_direction: FlexDirection::Row, column_gap: px(1), align_items: AlignItems::Center, ..at(0.0, y, w, 11.0) }).with_children(|r| {
                text(r, skin, l, FONT, None);
                r.spawn((Button, Clickable(id), RelativeCursorPosition::default(), Node { flex_grow: 1.0, height: px(11), flex_direction: FlexDirection::Row, ..default() }))
                    .with_children(|e| {
                        e.spawn(sb(LINE_EDIT, Node { flex_grow: 1.0, height: px(11), align_items: AlignItems::Center, ..default() }))
                            .with_children(|l| text(l, skin, &format!("{}", v as i64), FONT, None));
                        if let Some(img) = skin.image("updown") {
                            e.spawn((ImageNode::new(img), Node { width: px(10), height: px(10), margin: UiRect::left(px(1)), ..default() }));
                        }
                        if focused {
                            focus_line(e, 0.0);
                        }
                    });
            });
        }
        Field::Slider { label: l, pct, .. } => {
            let (min, max) = match *f {
                Field::Slider { min, max, .. } => (min, max),
                _ => unreachable!(),
            };
            let t = ((v - min) / (max - min)) as f32;
            c.spawn(Node { flex_direction: FlexDirection::Row, column_gap: px(1), align_items: AlignItems::Center, padding: UiRect::top(px(1)), ..at(0.0, y, w, 12.0) }).with_children(|r| {
                text(r, skin, l, FONT, None);
                slider(r, ui, id, t, Node { flex_grow: 1.0, ..default() });
                let shown = if pct { format!("{}%", v.round() as i64) } else { trim_number(v) };
                r.spawn(Node { width: px(20), margin: UiRect::right(px(8)), justify_content: JustifyContent::FlexEnd, ..default() }).with_children(|n| text(n, skin, &shown, FONT, None));
            });
        }
        Field::Check { label: l, .. } => check_row(c, ui, id, 0.0, y, w, l, v > 0.5, FONT),
        Field::Choice { label: l, options, .. } => choice_row(c, ui, id, y, w, l, options, v),
        Field::Separator => {}
    }
}

/// A choice row: label left, the chosen option on a button that cycles it.
#[allow(clippy::too_many_arguments)]
fn choice_row(c: &mut ChildSpawnerCommands, ui: &mut Ui, id: Id, y: f32, w: f32, l: &str, options: &[&str], v: f64) {
    let skin = ui.skin;
    label(c, skin, 0.0, y, w * 0.45, 11.0, l, FONT, Align::Left);
    let shown = options.get(v.round() as usize).copied().unwrap_or("");
    button(c, ui, id, w * 0.45, y, w * 0.55, 12.0, &format!("< {shown} >"), false, false);
}

// ---- singleplayer modes ---------------------------------------------------------------------

fn build_modes(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    let skin = ui.skin;
    let (x, y, w) = (230.0, 112.0, 180.0);
    p.spawn((at(x, y, w, 118.0), BackgroundColor(BLACK)));
    label(p, skin, x, y + 2.0, w, 11.0, "Singleplayer", FONT, Align::Center);
    let level = sim::cpu::Level::from_index(ui.settings.cpu_level).label();
    let rows: [(Id, String); 4] = [
        (Id::Mode(1), "vs CPU".into()),
        (Id::CpuLevel, format!("CPU level: < {level} >")),
        (Id::Mode(2), "training".into()),
        (Id::Mode(0), "versus (2 players)".into()),
    ];
    for (k, (id, l)) in rows.iter().enumerate() {
        button(p, ui, *id, x + 4.0, y + 17.0 + 16.0 * k as f32, w - 8.0, 14.0, l, false, false);
    }
    button(p, ui, Id::Back, x + 4.0, y + 100.0, w - 8.0, 12.0, "back", false, false);
}

// ---- training options -----------------------------------------------------------------------

fn build_training(p: &mut ChildSpawnerCommands, ui: &mut Ui) {
    training_panel(p, ui, 196.0, 34.0, true);
}

/// Training's options as a panel at (x, y); with a back button when it is a page of its own.
fn training_panel(p: &mut ChildSpawnerCommands, ui: &mut Ui, x: f32, y: f32, with_back: bool) {
    let skin = ui.skin;
    let w = 248.0;
    let rows_h: f32 = TRAINING.iter().map(|f| if matches!(f, Field::Separator) { 5.0 } else { 13.0 }).sum();
    p.spawn((at(x, y, w, rows_h + if with_back { 66.0 } else { 46.0 }), BackgroundColor(BLACK)));
    label(p, skin, x, y + 1.0, w, 11.0, "Training", FONT, Align::Center);
    let mut ry = y + 15.0;
    p.spawn(at(x + 4.0, 0.0, w - 8.0, VH)).with_children(|c| {
        for (i, f) in TRAINING.iter().enumerate() {
            match f {
                Field::Separator => {
                    ry += 5.0;
                    continue;
                }
                _ => {
                    let v = ui.settings.training.get(f);
                    match *f {
                        Field::Check { label: l, .. } => check_row(c, ui, Id::Train(i), 0.0, ry, w - 8.0, l, v > 0.5, FONT),
                        Field::Choice { label: l, options, .. } => choice_row(c, ui, Id::Train(i), ry, w - 8.0, l, options, v),
                        _ => {}
                    }
                }
            }
            ry += 13.0;
        }
    });
    let h = &ui.settings.hotkeys;
    let gray = Color::srgb_u8(0x8c, 0x8c, 0x8c);
    let hint = format!("{} reset   {} record   {} play recording", h.reset_round.label(), h.record_dummy.label(), h.play_recording.label());
    label(p, skin, x, ry + 4.0, w, 11.0, &hint, gray, Align::Center);
    label(p, skin, x, ry + 16.0, w, 11.0, &format!("{} freeze   {} step one frame", h.pause_sim.label(), h.frame_advance.label()), gray, Align::Center);
    if with_back {
        button(p, ui, Id::Back, x + 4.0, ry + 32.0, w - 8.0, 12.0, "back", false, false);
    }
}

/// Godot prints floats without trailing zeros ("1", "0.85").
fn trim_number(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---- in-fight menus -------------------------------------------------------------------------

fn build_pause(p: &mut ChildSpawnerCommands, ui: &mut Ui, game: Option<&Game>) {
    // PausePanel: 141 wide; 12px buttons with 1px gaps. Its rows depend on the mode.
    let mode = game.map(|g| g.mode).unwrap_or(Mode::Versus);
    let training = mode == Mode::Training;
    let level = format!("CPU level: < {} >", sim::cpu::Level::from_index(ui.settings.cpu_level).label());
    let (recording, has_tape) = game.map(|g| (g.training.recording, !g.training.tape.is_empty())).unwrap_or((false, false));
    let mut rows: Vec<(Id, String, bool)> = vec![(Id::Resume, "resume".into(), false)];
    rows.push((Id::Restart, if training { "reset position".into() } else { "restart round".into() }, false));
    if training {
        rows.push((Id::Record, if recording { "stop recording".into() } else { "record dummy".into() }, false));
        rows.push((Id::PlayRecording, "play recording".into(), !has_tape));
    }
    if matches!(mode, Mode::Cpu(_)) {
        rows.push((Id::PauseCpu, level, false));
    }
    rows.push((Id::MoveList, "move list".into(), false));
    rows.push((Id::CharSelect, "character select".into(), false));
    rows.push((Id::SaveReplay, "save replay".into(), true));
    rows.push((Id::PauseOptions, "options".into(), false));
    rows.push((Id::MainMenu, "main menu".into(), false));
    if !training {
        rows.push((Id::Forfeit, "forfeit".into(), false));
    }
    let h = rows.len() as f32 * 13.0 + 14.0;
    let y0 = (VH - h) / 2.0;
    // training shows its options beside the buttons
    let x0 = if training { 110.0 } else { 249.5 };
    p.spawn((at(x0, y0, 141.0, h), BackgroundColor(BLACK)));
    label(p, ui.skin, x0, y0 + 1.0, 141.0, 11.0, &format!("PAUSED - {}", mode.label()), FONT, Align::Center);
    for (k, (id, l, disabled)) in rows.into_iter().enumerate() {
        button(p, ui, id, x0 + 1.0, y0 + 14.0 + 13.0 * k as f32, 139.0, 12.0, &l, false, disabled);
    }
    if training {
        training_panel(p, ui, 260.0, 40.0, false);
    }
}

/// Every button of a fighter: ground and air columns, follow-ups, and the universal actions.
fn build_move_list(p: &mut ChildSpawnerCommands, ui: &mut Ui, game: Option<&Game>, who: usize) {
    use charfmt::Button as B;
    let skin = ui.skin;
    p.spawn((at(8.0, 6.0, VW - 16.0, VH - 12.0), BackgroundColor(BLACK)));
    let Some(g) = game else { return };
    let ch = &g.world.chars[who];
    let gray = Color::srgb_u8(0x8c, 0x8c, 0x8c);
    let gold = Color::srgb(1.0, 0.85, 0.2);
    label(p, skin, 8.0, 9.0, VW - 16.0, 11.0, &format!("P{} {} - move list", who + 1, ch.file.character.name), FONT, Align::Center);
    let names: std::collections::HashMap<&str, &str> = ch.file.character.moves.iter().map(|m| (m.id.as_str(), m.name.as_str())).collect();
    let button_name = |b: B| match b {
        B::Light => "L",
        B::Heavy => "H",
        B::Special => "S",
        B::Super => "H+S",
        B::Grab => "G",
        _ => "?",
    };
    let dir_name = |d: charfmt::Direction| match d {
        charfmt::Direction::Forward => "fwd ",
        charfmt::Direction::Back => "back ",
        charfmt::Direction::Up => "up ",
        charfmt::Direction::Down => "down ",
        _ => "",
    };
    for (col, stance) in [(0usize, charfmt::Stance::Ground), (1, charfmt::Stance::Air)] {
        let x = 16.0 + col as f32 * 312.0;
        label(p, skin, x, 24.0, 300.0, 11.0, if col == 0 { "ground" } else { "air" }, gray, Align::Left);
        let mut y = 37.0;
        for b in ch.controls.binds.iter().filter(|b| b.stance == stance) {
            if y > VH - 52.0 {
                break;
            }
            let mut moves: Vec<&str> = Vec::new();
            for m in &b.moves {
                let n = names.get(m.as_str()).copied().unwrap_or(m);
                if !moves.contains(&n) {
                    moves.push(n);
                }
            }
            let mut text = moves.join(" / ");
            if text.chars().count() > 34 {
                text = text.chars().take(33).collect::<String>() + "…";
            }
            let keys = if b.cancel_only { format!("{}{} again", dir_name(b.dir), button_name(b.button)) } else { format!("{}{}", dir_name(b.dir), button_name(b.button)) };
            label(p, skin, x, y, 92.0, 11.0, &keys, if b.cancel_only { gray } else { gold }, Align::Left);
            label(p, skin, x + 94.0, y, 210.0, 11.0, &text, FONT, Align::Left);
            y += 11.5;
        }
    }
    let r = &g.world.cfg.combo;
    let lines = [
        "L > H > S > H+S chain on hit or block (no move twice). Jump cancels a launcher on hit.".to_string(),
        format!("Dash during any of your moves: HUSTLE CANCEL ({} meter). H+S moves cost {} meter per bar.", r.hustle_cancel, r.super_bar),
    ];
    for (k, l) in lines.iter().enumerate() {
        label(p, skin, 16.0, VH - 46.0 + 11.0 * k as f32, VW - 32.0, 11.0, l, gray, Align::Left);
    }
    button(p, ui, Id::MoveListOther, 200.0, VH - 20.0, 118.0, 12.0, if who == 0 { "P2's moves" } else { "P1's moves" }, false, false);
    button(p, ui, Id::Back, 322.0, VH - 20.0, 118.0, 12.0, "back", false, false);
}

fn build_postgame(p: &mut ChildSpawnerCommands, ui: &mut Ui, game: Option<&Game>) {
    let skin = ui.skin;
    let line = match game.map(|g| &g.world) {
        Some(w) if w.draw => "DRAW".to_string(),
        Some(w) => w.winner.map(|i| format!("P{} WIN", i + 1)).unwrap_or_default(),
        None => String::new(),
    };
    // WinLabel: a 36px font with a 2px outline, centred
    let mut f = font(skin);
    if skin.font.is_some() {
        f.font_size = FontSize::Px(36.0);
    }
    p.spawn(Node { justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..at(0.0, 158.0, VW, 44.0) }).with_children(|c| {
        c.spawn(Node::default()).with_children(|w| {
            for dx in [-2.0, 0.0, 2.0] {
                for dy in [-2.0, 0.0, 2.0] {
                    if dx != 0.0 || dy != 0.0 {
                        w.spawn((Text::new(&line), f.clone(), TextColor(BLACK), Node { position_type: PositionType::Absolute, left: px(dx), top: px(dy), ..default() }));
                    }
                }
            }
            w.spawn((Text::new(&line), f.clone(), TextColor(Color::WHITE)));
        });
    });
    button(p, ui, Id::Rematch, 277.0, 281.0, 86.0, 12.0, "Rematch", false, false);
    button(p, ui, Id::MainMenu, 277.0, 294.0, 86.0, 12.0, "Quit", false, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RosterEntry;

    fn roster() -> Roster {
        Roster(
            crate::ROSTER
                .iter()
                .map(|id| RosterEntry { id: id.to_string(), name: id.to_string(), moves: 1, hp: 100, modes: vec![] })
                .collect(),
        )
    }

    #[test]
    fn singleplayer_flow_picks_both_players_then_starts() {
        let mut m = Menu::new(Page::Main);
        let mut s = Settings::default();
        let r = roster();
        let st = &mut Styles { list: crate::style::presets() };
        activate(&mut m, &mut s, &r, st, Id::Singleplayer, Act::Press);
        assert_eq!(m.page(), Some(Page::Modes));
        activate(&mut m, &mut s, &r, st, Id::CpuLevel, Act::Press); // normal -> hard
        activate(&mut m, &mut s, &r, st, Id::Mode(1), Act::Press);
        assert_eq!(m.page(), Some(Page::Select));
        activate(&mut m, &mut s, &r, st, Id::Char(1), Act::Press); // Cowboy
        activate(&mut m, &mut s, &r, st, Id::Char(3), Act::Press); // Robot
        assert_eq!(m.picks, [Some(1), Some(3)]);
        activate(&mut m, &mut s, &r, st, Id::LoadStyle(0), Act::Press);
        assert_eq!(s.styles[0], "Inferno");
        match activate(&mut m, &mut s, &r, st, Id::Go, Act::Press) {
            Effect::StartFight(p, mode) => {
                assert_eq!(p.map(|i| crate::ROSTER[i]), ["cowboy", "robo"]);
                assert_eq!(mode, Mode::Cpu(2));
            }
            _ => panic!("GO should start the fight"),
        }
        // Esc undoes picks before leaving the page
        back(&mut m);
        assert_eq!(m.picks, [Some(1), None]);
    }

    #[test]
    fn arrows_move_the_focus_to_the_nearest_widget_that_way() {
        // a 3-wide grid of characters, a button under it
        let at = |x: f32, y: f32| Vec2::new(x, y);
        let size = Vec2::new(40.0, 40.0);
        let mut rects: Vec<(Id, Vec2, Vec2)> = (0..6).map(|i| (Id::Char(i), at(50.0 * (i % 3) as f32, 50.0 * (i / 3) as f32), size)).collect();
        rects.push((Id::Go, at(60.0, 120.0), Vec2::new(100.0, 20.0)));
        assert_eq!(nearest(&rects, Id::Char(0), Vec2::X), Some(Id::Char(1)));
        assert_eq!(nearest(&rects, Id::Char(1), -Vec2::X), Some(Id::Char(0)));
        assert_eq!(nearest(&rects, Id::Char(2), Vec2::X), None);
        assert_eq!(nearest(&rects, Id::Char(1), Vec2::Y), Some(Id::Char(4)));
        assert_eq!(nearest(&rects, Id::Char(4), Vec2::Y), Some(Id::Go));
        assert_eq!(nearest(&rects, Id::Go, -Vec2::Y), Some(Id::Char(4)));
        // values step; buttons move
        assert!(takes_steps(Id::Opt(0)) && !takes_steps(Id::Char(0)) && !takes_steps(Id::Go));
    }

    #[test]
    fn options_and_rules_change_values() {
        let mut m = Menu::new(Page::Options);
        let mut s = Settings::default();
        let r = roster();
        let st = &mut Styles { list: crate::style::presets() };
        activate(&mut m, &mut s, &r, st, Id::Opt(0), Act::Press);
        assert!(s.flag("fullscreen"));
        let master = OPTIONS.iter().position(|f| f.key() == Some("master_volume")).unwrap();
        activate(&mut m, &mut s, &r, st, Id::Opt(master), Act::Click((0.5, true)));
        assert_eq!(s.options.get(&OPTIONS[master]), 0.5);
        activate(&mut m, &mut s, &r, st, Id::Bind(4), Act::Press);
        assert_eq!(m.rebinding, Some(4));
        // spin boxes: top half of the arrows counts up
        activate(&mut m, &mut s, &r, st, Id::Rule(0), Act::Click((0.9, true)));
        assert_eq!(s.rules.get(&RULES[0]), 3601.0);
        activate(&mut m, &mut s, &r, st, Id::Rule(6), Act::Step(1));
        assert_eq!(s.rules.get(&RULES[6]), 1.1);
        // training choices cycle
        activate(&mut m, &mut s, &r, st, Id::Train(0), Act::Step(-1));
        assert_eq!(s.train("dummy") as usize, crate::settings::DUMMY_ACTIONS.len() - 1);
    }
}
