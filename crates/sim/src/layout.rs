//! The automatic button layout. Fighters carry dozens of moves (the original picks them from a
//! menu); in real time every one needs a button. This sorts a character's moves into what the
//! player chooses and what the engine or other moves start, then places the choices on
//! Light / Heavy / Special / Super (Heavy+Special) / Grab, each with five directions, per stance
//! and per mode, the way fighting-game players expect: quick normals on Light, slower ones on
//! Heavy, anti-airs up, lows down, lunges forward. Follow-ups go on the button of the move they
//! continue, so pressing it again carries on. What does not fit is kept as a reserve the player
//! can swap in.

use std::collections::{BTreeMap, HashMap, HashSet};

use charfmt::{Button, CharacterFile, Direction, HitKind, Height, MotionKind, Move, Stance};

use crate::data::{Bind, ControlScheme};

/// Moves the engine runs itself.
const ENGINE: [&str; 14] = [
    "idle", "wait", "fall", "landing", "jump", "double_jump", "super_jump", "dash_forward", "dash_backward", "air_dash", "charge_dash",
    "charge_dash_forward", "dash_startup", "walk",
];

/// The buttons a choice can sit on, in the order the layout reads them.
pub const BUTTONS: [Button; 5] = [Button::Light, Button::Heavy, Button::Special, Button::Super, Button::Grab];
/// Directions, neutral first.
pub const DIRS: [Direction; 5] = [Direction::Neutral, Direction::Forward, Direction::Down, Direction::Up, Direction::Back];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    /// The engine runs it (idle, jump, dash...).
    Engine,
    /// Movement the engine's own dash and jump cover (rolls, air dashes, runs).
    Movement,
    /// Bursts, instant cancels, taunts: universal actions.
    System,
    /// Continues another move: pressing that move's button again during it starts this.
    FollowUp { from: Vec<String> },
    /// Started by the game or another move, never chosen (hurt reactions, "success" states).
    Internal,
    /// Something the player picks: gets a button.
    Choice,
}

impl Class {
    pub fn label(&self) -> &'static str {
        match self {
            Class::Engine => "engine",
            Class::Movement => "movement",
            Class::System => "system",
            Class::FollowUp { .. } => "follow-up",
            Class::Internal => "internal",
            Class::Choice => "choice",
        }
    }
}

/// Where a move ended up.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// On `button` + `dir` while in `stance` (and `mode`, when it belongs to one).
    Slot { button: Button, dir: Direction, stance: Stance, mode: Option<String> },
    /// Pressed again during its parent(s).
    FollowUp { button: Button, dir: Direction, stance: Stance },
    /// Did not fit: available to swap in.
    Reserve,
    /// Not the player's to press.
    None,
}

pub struct Entry {
    pub id: String,
    pub name: String,
    /// Usable only in some situations (in a combo, out of one): tried before its plain variant.
    pub situational: bool,
    pub class: Class,
    pub places: Vec<Place>,
}

pub struct Layout {
    pub scheme: ControlScheme,
    pub entries: Vec<Entry>,
}

impl Layout {
    pub fn count(&self, label: &str) -> usize {
        self.entries.iter().filter(|e| e.class.label() == label).count()
    }

    pub fn reserve(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.places.contains(&Place::Reserve))
    }
}

fn has(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

fn lower(mv: &Move) -> String {
    format!("{} {}", mv.id, mv.name.to_lowercase())
}

/// Sorts every move of a character.
pub fn classify(file: &CharacterFile) -> Vec<(usize, Class)> {
    let moves = &file.character.moves;
    // how many moves carry each tag: a rare tag in a cancel list names a specific follow-up
    let mut tag_use: HashMap<&str, usize> = HashMap::new();
    for m in moves {
        for t in &m.tags {
            *tag_use.entry(t.as_str()).or_default() += 1;
        }
    }
    let mut parents: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for m in moves {
        for c in &m.cancels {
            // universal categories (defense, grabs, bursts, movement) are not follow-ups
            let specific = |t: &&String| tag_use.get(t.as_str()).copied().unwrap_or(0) <= 4 && !has(t, &["defense", "grab", "burst", "instant_cancel", "movement"]);
            for t in c.into.iter().filter(specific) {
                for f in moves.iter().filter(|f| f.id != m.id && f.tags.contains(t)) {
                    parents.entry(f.id.as_str()).or_default().push(m.id.clone());
                }
            }
        }
    }
    moves
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let text = lower(m);
            let button = m.input.as_ref().map(|inp| inp.button);
            let movement_tag = m.tags.iter().any(|t| t.contains("movement"));
            let class = if ENGINE.contains(&m.id.as_str()) {
                Class::Engine
            } else if matches!(button, Some(Button::Burst | Button::Taunt)) || has(&m.id, &["burst", "instant_cancel", "taunt"]) {
                Class::System
            } else if matches!(button, Some(Button::Jump | Button::Dash | Button::Guard)) || (movement_tag && m.hitboxes.iter().all(|h| h.kind == HitKind::Detect)) {
                Class::Movement
            } else if m.throw.is_some() || m.tags.iter().any(|t| t.ends_with("_throw")) || m.id.ends_with("landing") {
                // what happens to a held opponent: the grab leads into it
                Class::Internal
            } else if button.is_none() && has(&text, &["parry", "block"]) {
                // blocking is the engine's (hold back)
                Class::Movement
            } else if let Some(from) = parents.get(m.id.as_str()).filter(|f| button.is_none() && f.len() <= 3) {
                let mut from = from.clone();
                from.dedup();
                Class::FollowUp { from }
            } else if button.is_some() || (m.hitboxes.iter().any(|h| h.effects.throw) && m.input.is_none() && m.throw.is_none()) {
                Class::Choice
            } else {
                Class::Internal
            };
            (i, class)
        })
        .collect()
}

/// Startup, damage and length roll up into how "heavy" an attack feels.
fn weight(m: &Move) -> i64 {
    let tl = &m.timeline;
    let dmg = m.hitboxes.iter().filter(|h| h.kind != HitKind::Detect).map(|h| h.damage as i64).max().unwrap_or(0).min(400);
    tl.startup as i64 * 3 + tl.total() as i64 + dmg / 8
}

/// The direction a move asks for: the one the original's menu gave it, else what it does.
fn preferred_dir(m: &Move) -> Direction {
    if let Some(d) = m.input.as_ref().map(|i| i.direction).filter(|d| *d != Direction::Any) {
        return d;
    }
    let text = lower(m);
    let hits: Vec<_> = m.hitboxes.iter().filter(|h| h.kind != HitKind::Detect).collect();
    let launches = hits.iter().any(|h| h.knockback.dir.1 .0 < -0.5);
    let low = hits.iter().any(|h| h.height == Height::Low);
    let push: f64 = m.motion.iter().map(|e| match e.kind {
        MotionKind::Impulse { forward, .. } | MotionKind::SetVelocity { forward, .. } => forward.0,
        _ => 0.0,
    }).sum();
    if has(&text, &["anti_air", "anti air", "upper", "rising", "launch", "sky", "uppercut"]) || (launches && !low) {
        Direction::Up
    } else if low || has(&text, &["sweep", "low", "slide", "crouch", "stomp", "spike", "dive", "drop", "down"]) {
        Direction::Down
    } else if has(&text, &["back", "retreat", "counter", "reversal", "parry"]) || push < -2.0 {
        Direction::Back
    } else if has(&text, &["dash", "lunge", "rush", "charge", "forward", "step", "run", "tackle", "drive", "thrust"]) || push > 3.0 {
        Direction::Forward
    } else {
        Direction::Neutral
    }
}

/// The button a choice belongs on before any balancing.
fn category(m: &Move) -> Button {
    if m.cost.super_bars > 0 {
        return Button::Super;
    }
    match m.input.as_ref().map(|i| i.button) {
        Some(Button::Super) => Button::Super,
        Some(Button::Special) => Button::Special,
        Some(Button::Grab) => Button::Grab,
        Some(Button::Heavy) => Button::Heavy,
        _ if m.throw.is_some() || m.hitboxes.iter().any(|h| h.effects.throw) => Button::Grab,
        _ => Button::Light,
    }
}

fn button_order(b: Button) -> &'static [Button] {
    match b {
        Button::Light => &[Button::Light, Button::Heavy, Button::Special],
        Button::Heavy => &[Button::Heavy, Button::Light, Button::Special],
        Button::Special => &[Button::Special, Button::Heavy, Button::Super],
        Button::Super => &[Button::Super, Button::Special, Button::Heavy],
        _ => &[Button::Grab, Button::Special, Button::Heavy],
    }
}

fn dir_order(pref: Direction) -> Vec<Direction> {
    let mut v = vec![pref];
    v.extend(DIRS.iter().copied().filter(|d| *d != pref));
    v
}

/// A grid of slots for one stance and mode: (button, dir) -> move ids.
#[derive(Default, Clone)]
struct Grid(HashMap<(usize, usize), String>);

fn bi(b: Button) -> usize {
    BUTTONS.iter().position(|x| *x == b).unwrap_or(0)
}

fn di(d: Direction) -> usize {
    DIRS.iter().position(|x| *x == d).unwrap_or(0)
}

/// Builds the layout for a character.
pub fn plan(file: &CharacterFile) -> Layout {
    let ch = &file.character;
    let classes = classify(file);
    let modes: Vec<String> = ch.modes.iter().skip(1).cloned().collect();
    let mut entries: Vec<Entry> = classes
        .iter()
        .map(|(i, c)| {
            let m = &ch.moves[*i];
            let situational = m.availability.combo_only || m.availability.neutral_only || m.cost.air_action;
            Entry { id: m.id.clone(), name: m.name.clone(), situational, class: c.clone(), places: vec![] }
        })
        .collect();

    // choices, by stance; Light/Heavy split by weight within each stance
    let choice: Vec<usize> = classes.iter().filter(|(_, c)| *c == Class::Choice).map(|(i, _)| *i).collect();
    let mut wanted: HashMap<usize, Button> = choice.iter().map(|&i| (i, category(&ch.moves[i]))).collect();
    for stance in [Stance::Ground, Stance::Air] {
        let mut attacks: Vec<usize> = choice
            .iter()
            .copied()
            .filter(|&i| wanted[&i] == Button::Light && (ch.moves[i].stance == stance || ch.moves[i].stance == Stance::Any))
            .collect();
        attacks.sort_by_key(|&i| (weight(&ch.moves[i]), i));
        let light = attacks.len().div_ceil(2).max(attacks.len().min(5));
        for &i in attacks.iter().skip(light) {
            wanted.insert(i, Button::Heavy);
        }
    }

    // grids: one per stance for the moves of every mode, and one per stance and mode
    let mut base: HashMap<u8, Grid> = HashMap::new();
    let mut by_mode: HashMap<(u8, String), Grid> = HashMap::new();
    // variants of one move (same name: a grounded, a combo-only, a cancel version...) share a
    // slot; the engine tries them in order and the first usable one starts
    let mut units: Vec<Vec<usize>> = Vec::new();
    for &i in &choice {
        let name = ch.moves[i].name.to_lowercase();
        match units.iter_mut().find(|u| ch.moves[u[0]].name.to_lowercase() == name) {
            Some(u) => u.push(i),
            None => units.push(vec![i]),
        }
    }
    // the button a unit wants: its strongest member's
    let rank = |b: Button| match b {
        Button::Super => 4,
        Button::Grab => 3,
        Button::Special => 2,
        Button::Heavy => 1,
        _ => 0,
    };
    let unit_button = |u: &Vec<usize>| u.iter().map(|i| wanted[i]).max_by_key(|b| rank(*b)).unwrap_or(Button::Light);
    let explicit = |i: usize| ch.moves[i].input.as_ref().is_some_and(|x| x.direction != Direction::Any);
    // moves the original's menu gave a direction go first: they were designed for it
    units.sort_by_key(|u| (!u.iter().any(|&i| explicit(i)), u.iter().all(|&i| !ch.moves[i].modes.is_empty()), u.iter().map(|&i| weight(&ch.moves[i])).min(), u[0]));
    for u in &units {
        let button = unit_button(u);
        let lead = u.iter().copied().find(|&i| explicit(i)).unwrap_or(u[0]);
        for st in [Stance::Ground, Stance::Air] {
            let members: Vec<usize> = u.iter().copied().filter(|&i| ch.moves[i].stance == st || ch.moves[i].stance == Stance::Any).collect();
            if members.is_empty() {
                continue;
            }
            let pref = preferred_dir(&ch.moves[lead]);
            // a stance's own moves go on that stance's grid when every member belongs to it
            let mut own_modes: Vec<String> = members.iter().flat_map(|&i| ch.moves[i].modes.iter().filter(|x| modes.contains(x)).cloned()).collect();
            own_modes.sort();
            own_modes.dedup();
            let all_moded = members.iter().all(|&i| ch.moves[i].modes.iter().any(|x| modes.contains(x)));
            let put = |entries: &mut Vec<Entry>, b: Button, d: Direction, mode: Option<String>| {
                for &i in &members {
                    entries[i].places.push(Place::Slot { button: b, dir: d, stance: st, mode: mode.clone() });
                }
            };
            if !all_moded {
                let g = base.entry(stance_key(st)).or_default();
                if let Some((b, d)) = free_slot(g, None, button, pref) {
                    g.0.insert((bi(b), di(d)), ch.moves[lead].id.clone());
                    put(&mut entries, b, d, None);
                }
            } else {
                // one slot, free in every stance the move belongs to: a slot nothing uses first,
                // else one that shadows a move of every mode while in that stance
                let seen = base.get(&stance_key(st)).cloned().unwrap_or_default();
                let mut taken = Grid::default();
                for md in &own_modes {
                    if let Some(g) = by_mode.get(&(stance_key(st), md.clone())) {
                        taken.0.extend(g.0.clone());
                    }
                }
                let slot = free_slot(&taken, Some(&seen), button, pref).or_else(|| free_slot(&taken, None, button, pref));
                if let Some((b, d)) = slot {
                    for md in &own_modes {
                        by_mode.entry((stance_key(st), md.clone())).or_default().0.insert((bi(b), di(d)), ch.moves[lead].id.clone());
                        put(&mut entries, b, d, Some(md.clone()));
                    }
                }
            }
        }
        for &i in u {
            if entries[i].places.is_empty() {
                entries[i].places.push(Place::Reserve);
            }
        }
    }

    // follow-ups: the parent's button again (the first plain, the rest with a direction)
    let slot_of: HashMap<String, (Button, Stance)> = entries
        .iter()
        .flat_map(|e| e.places.iter().filter_map(move |p| match p {
            Place::Slot { button, stance, .. } => Some((e.id.clone(), (*button, *stance))),
            _ => None,
        }))
        .collect();
    let mut used: HashMap<(usize, u8), usize> = HashMap::new();
    let follow_dirs = [Direction::Any, Direction::Forward, Direction::Down, Direction::Up, Direction::Back];
    for (i, c) in &classes {
        let Class::FollowUp { from } = c else { continue };
        let mut done = HashSet::new();
        for p in from {
            let Some(&(button, stance)) = slot_of.get(p) else { continue };
            if !done.insert((bi(button), stance_key(stance))) {
                continue;
            }
            let k = used.entry((bi(button), stance_key(stance))).or_default();
            let dir = follow_dirs[(*k).min(follow_dirs.len() - 1)];
            *k += 1;
            entries[*i].places.push(Place::FollowUp { button, dir, stance });
        }
    }
    for e in &mut entries {
        if e.places.is_empty() {
            e.places.push(Place::None);
        }
    }

    // binds: mode moves before the moves of every mode on the same slot (the first usable wins)
    let mut slots: BTreeMap<(u8, usize, usize, bool), Vec<(bool, String)>> = BTreeMap::new();
    for e in &entries {
        for p in &e.places {
            match p {
                Place::Slot { button, dir, stance, mode } => {
                    let general = mode.is_none() && !e.situational;
                    slots.entry((stance_key(*stance), bi(*button), di(*dir), false)).or_default().push((general, e.id.clone()));
                }
                Place::FollowUp { button, dir, stance } => {
                    let d = if *dir == Direction::Any { 5 } else { di(*dir) };
                    slots.entry((stance_key(*stance), bi(*button), d, true)).or_default().push((false, e.id.clone()));
                }
                _ => {}
            }
        }
    }
    let mut binds = Vec::new();
    // follow-ups first: during a move they are what a press means; then neutral choices
    for want_follow in [true, false] {
        for ((st, b, d, follow), mut ids) in slots.clone() {
            if follow != want_follow {
                continue;
            }
            ids.sort();
            ids.dedup_by(|a, b| a.1 == b.1);
            binds.push(Bind {
                button: BUTTONS[b],
                dir: if d == 5 { Direction::Any } else { DIRS[d] },
                stance: if st == 0 { Stance::Ground } else { Stance::Air },
                cancel_only: follow,
                moves: ids.into_iter().map(|(_, id)| id).collect(),
            });
        }
    }
    Layout { scheme: ControlScheme { character: ch.id.clone(), binds }, entries }
}

fn stance_key(s: Stance) -> u8 {
    match s {
        Stance::Air => 1,
        _ => 0,
    }
}

/// The first free slot for a move that wants `button` and `pref`, also avoiding `also`.
fn free_slot(g: &Grid, also: Option<&Grid>, button: Button, pref: Direction) -> Option<(Button, Direction)> {
    // its own button family first, then any free slot at all
    for &b in button_order(button).iter().chain(BUTTONS.iter()) {
        for d in dir_order(pref) {
            let k = (bi(b), di(d));
            if !g.0.contains_key(&k) && also.is_none_or(|a| !a.0.contains_key(&k)) {
                return Some((b, d));
            }
        }
    }
    None
}

fn button_name(b: Button) -> &'static str {
    match b {
        Button::Light => "Light",
        Button::Heavy => "Heavy",
        Button::Special => "Special",
        Button::Super => "Super",
        Button::Grab => "Grab",
        Button::Guard => "Guard",
        Button::Jump => "Jump",
        Button::Dash => "Dash",
        Button::Burst => "Burst",
        Button::Taunt => "Taunt",
    }
}

fn dir_name(d: Direction) -> &'static str {
    match d {
        Direction::Any => "Any",
        Direction::Neutral => "Neutral",
        Direction::Forward => "Forward",
        Direction::Back => "Back",
        Direction::Up => "Up",
        Direction::Down => "Down",
    }
}

/// How a place reads in a move list: `Forward + Heavy (ground)`.
pub fn describe(p: &Place) -> String {
    let st = |s: &Stance| if *s == Stance::Air { "air" } else { "ground" };
    let combo = |b: Button, d: Direction| {
        let b = if b == Button::Super { "Heavy+Special".to_string() } else { button_name(b).to_string() };
        if matches!(d, Direction::Neutral | Direction::Any) { b } else { format!("{} + {b}", dir_name(d)) }
    };
    match p {
        Place::Slot { button, dir, stance, mode } => {
            let m = mode.as_ref().map(|m| format!(", {m} stance")).unwrap_or_default();
            format!("{} ({}{m})", combo(*button, *dir), st(stance))
        }
        Place::FollowUp { button, dir, stance } => format!("{} again during the move ({})", combo(*button, *dir), st(stance)),
        Place::Reserve => "reserve (swap it in)".into(),
        Place::None => "-".into(),
    }
}

/// The control scheme as the RON file the game loads, with each move's name as a comment.
pub fn scheme_text(layout: &Layout, names: &HashMap<String, String>) -> String {
    let mut out = String::new();
    out.push_str(&format!("// Control scheme for {} (PRIVATE: names moves from the original game).\n", layout.scheme.character));
    out.push_str("// Generated by the automatic layout (sim::layout): edit freely, press F5 in the game to reload.\n");
    out.push_str("// Light = quick attacks, Heavy = strong attacks, Special = specials, Super = Heavy+Special, Grab = grabs.\n");
    out.push_str("// `cancel_only` binds are follow-ups: press the button again during the move.\n");
    out.push_str(&format!("ControlScheme(\n    character: \"{}\",\n    binds: [\n", layout.scheme.character));
    for b in &layout.scheme.binds {
        let dir = if b.dir == Direction::Any { String::new() } else { format!(", dir: {}", dir_name(b.dir)) };
        let st = if b.stance == Stance::Air { "Air" } else { "Ground" };
        let co = if b.cancel_only { ", cancel_only: true" } else { "" };
        let ids = b.moves.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", ");
        let comment = b.moves.iter().map(|m| names.get(m).cloned().unwrap_or_default()).collect::<Vec<_>>().join(", ");
        out.push_str(&format!("        Bind(button: {}{dir}, stance: {st}{co}, moves: [{ids}]),  // {comment}\n", button_name(b.button)));
    }
    out.push_str("    ],\n)\n");
    out
}

/// The audit as Markdown: counts, then every move with its class and buttons.
pub fn report(layout: &Layout, name: &str) -> String {
    let mut out = format!("# {name}: move layout\n\n");
    let total = layout.entries.len();
    out.push_str(&format!(
        "{total} moves: {} choices, {} follow-ups, {} movement, {} system, {} internal, {} engine. {} choices in reserve.\n\n",
        layout.count("choice"),
        layout.count("follow-up"),
        layout.count("movement"),
        layout.count("system"),
        layout.count("internal"),
        layout.count("engine"),
        layout.reserve().count()
    ));
    out.push_str("| Move | Id | Kind | Buttons |\n|---|---|---|---|\n");
    let mut rows: Vec<&Entry> = layout.entries.iter().collect();
    let rank = |e: &Entry| match e.class {
        Class::Choice => 0,
        Class::FollowUp { .. } => 1,
        Class::System => 2,
        Class::Movement => 3,
        Class::Internal => 4,
        Class::Engine => 5,
    };
    rows.sort_by_key(|e| rank(e));
    for e in rows {
        let places = e.places.iter().map(describe).collect::<Vec<_>>().join("; ");
        let kind = match &e.class {
            Class::FollowUp { from } => format!("follow-up of {}", from.join(", ")),
            c => c.label().to_string(),
        };
        out.push_str(&format!("| {} | `{}` | {kind} | {places} |\n", e.name, e.id));
    }
    out
}

/// Writes `<id>_controls.ron` and `<id>_moves.md` next to `<id>.ron`. A controls file someone
/// edited by hand (no "generated" header) is kept unless `force`.
pub fn write_for(dir: &std::path::Path, id: &str, force: bool) -> Result<Layout, String> {
    let path = dir.join(format!("{id}.ron"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = charfmt::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let layout = plan(&file);
    let names: HashMap<String, String> = file.character.moves.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
    let ctl = dir.join(format!("{id}_controls.ron"));
    let keep = !force && std::fs::read_to_string(&ctl).is_ok_and(|t| !t.contains("Generated by the automatic layout") && !t.contains("DRAFT generated"));
    if !keep {
        std::fs::write(&ctl, scheme_text(&layout, &names)).map_err(|e| format!("{}: {e}", ctl.display()))?;
    }
    let md = dir.join(format!("{id}_moves.md"));
    std::fs::write(&md, report(&layout, &file.character.name)).map_err(|e| format!("{}: {e}", md.display()))?;
    Ok(layout)
}
