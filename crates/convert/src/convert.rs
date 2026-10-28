//! Turns an extracted character (see `extract.rs`) into the project's character file and a report
//! of what could not be expressed. A port of `tools/convert_character.py`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::ron::{emit, round_to, sid, snake, R};
use crate::tscn::read_text;

static NULL: Value = Value::Null;

macro_rules! lazy_re {
    ($pat:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        CELL.get_or_init(|| Regex::new($pat).expect("valid pattern"))
    }};
}

// ------------------------------------------------------------------ Python-like value helpers
fn at<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&NULL)
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `float(v)` with a default for missing or empty values.
fn num(v: Option<&Value>, d: f64) -> f64 {
    match v {
        None | Some(Value::Null) => d,
        Some(Value::String(s)) if s.is_empty() => d,
        Some(Value::String(s)) => s.trim().parse().unwrap_or(d),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(d),
        Some(Value::Bool(b)) => *b as u8 as f64,
        _ => d,
    }
}

/// `int(v)`.
fn to_int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        Some(Value::String(s)) => s.trim().parse::<i64>().unwrap_or_else(|_| s.trim().parse::<f64>().unwrap_or(0.0) as i64),
        Some(Value::Bool(b)) => *b as i64,
        _ => 0,
    }
}

fn is_int(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Number(n)) if n.is_i64()) || matches!(v, Some(Value::Bool(_)))
}

fn string(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

fn json_to_r(v: Option<&Value>) -> R {
    v.map(R::json).unwrap_or(R::Int(0))
}

fn f(x: f64) -> R {
    R::Float(x)
}

fn pyabs(v: &Value) -> R {
    match R::json(v) {
        R::Int(i) => R::Int(i.abs()),
        R::Float(x) => R::Float(x.abs()),
        o => o,
    }
}

/// Python's `max(v, 0)` / `min(v, 15)` keep the value's own type.
fn max0(v: &Value) -> R {
    if num(Some(v), 0.0) >= 0.0 { R::json(v) } else { R::Int(0) }
}

fn min_cap(v: Option<&Value>, cap: i64) -> R {
    if num(v, 0.0) <= cap as f64 { json_to_r(v) } else { R::Int(cap) }
}

fn span(from: i64, to: i64) -> R {
    R::st("Span", vec![("from", R::Int(from)), ("to", R::Int(to))])
}

// ------------------------------------------------------------------ tables
const FRAMEWORK: [&str; 3] = ["characters/states/CharState.gd", "ObjectState.gd", "framework/FiniteStateMachine/StateInterface.gd"];

const STATE_CONSUMED: &[&str] = &[
    "anim_length", "title", "type", "air_type", "interrupt_from_string", "interrupt_into_string",
    "hit_cancel_into_string", "iasa_at", "interrupt_frames", "enter_force_dir_x", "enter_force_dir_y",
    "enter_force_speed", "force_dir_x", "force_dir_y", "force_speed", "force_tick", "apply_fric",
    "apply_forces", "apply_grav", "reset_momentum", "endless", "combo_only", "neutral_only",
    "super_level_", "uses_air_movement", "projectile_scene", "projectile_tick", "projectile_pos_x",
    "projectile_pos_y", "fallback_state", "sprite_animation", "show_in_menu", "selectable",
    "data_ui_scene", "auto_fall", "allowed_stances_string", "change_stance_to",
    "apply_custom_grav", "custom_grav", "custom_grav_max_fall_speed", "apply_custom_limits",
    "custom_max_air_speed", "custom_max_ground_speed", "apply_custom_x_fric", "apply_custom_y_fric",
    "custom_x_fric", "custom_y_fric", "apply_forces_no_limit", "reset_x_momentum", "reset_y_momentum",
    "ticks_per_frame",
];

const THROW_CONSUMED: &[&str] = &[
    "release", "release_frame", "use_start_throw_pos", "start_throw_pos_x", "start_throw_pos_y",
    "use_release_throw_pos", "release_throw_pos_x", "release_throw_pos_y", "throw_positions", "reverse",
    "hitstun_ticks", "knockback", "dir_x", "dir_y", "knockdown", "knockdown_extends_hitstun",
    "aerial_hit_state", "grounded_hit_state", "damage", "damage_in_combo", "ground_bounce",
    "increment_combo", "hard_knockdown", "air_ground_bounce", "wall_slam", "di_modifier",
    "minimum_grounded_frames", "damage_proration", "hit_height",
];

const STATE_PRESENTATION: &[&str] = &[
    "button_texture", "flip_icon", "particle_scene", "spawn_particle_on_enter", "particle_position",
    "timed_particle_scene", "timed_particle_position", "timed_spawn_particle_tick", "state_screenshake_tick",
    "state_screenshake_dir", "state_screenshake_length", "state_screenshake_amount", "enter_sfx",
    "enter_sfx_volume", "sfx", "sfx_tick", "sfx_volume", "sfx_pitch_variation", "override_same_sfx",
    "override_same_wav", "loop_animation", "animation_loop_start", "absolute_loop", "refresh_loop",
    "ticks_per_frame", "sprite_anim_length", "flip_with_facing", "projectile_match_facing",
    "projectile_local_pos",
];

fn state_class(k: &str) -> Option<&'static str> {
    const TURN: &[&str] = &[
        "interruptible_on_opponent_turn", "initiative_effect", "next_state_on_hold",
        "next_state_on_hold_on_opponent_turn", "end_feint", "can_feint_if_possible",
        "usable_from_whiff_cancel_if_possible", "instant_cancellable", "hold_restart",
        "allow_framecheat", "dynamic_iasa", "backdash_iasa", "busy_interrupt_type",
    ];
    const ENGINE: &[&str] = &[
        "reversible", "self_hit_cancellable", "self_interruptable", "burst_cancellable", "throw_techable",
        "whiff_meter_gain_multiplier", "super_freeze_ticks", "super_freeze_ticks_", "extra_parry_hitlag",
        "release_opponent_on_startup", "release_opponent_on_exit", "can_be_counterhit", "beats_backdash",
        "velocity_forward_meter_gain_multiplier", "tick_priority", "burstable", "super_effect_",
        "scale_combo_meter_", "force_super_effect", "supers_used_", "land_cancel", "landing_recovery",
        "min_land_cancel_frame", "land_cancel_state", "uses_air_movement_ignore", "count_projectile_hits",
        "earliest_hitbox", "ignore_air_option_use_in_combos", "update_facing_on_exit",
    ];
    const GAP: &[&str] = &[
        "apply_custom_limits", "custom_max_air_speed", "custom_max_ground_speed", "apply_custom_x_fric",
        "apply_custom_y_fric", "apply_custom_grav", "custom_x_fric", "custom_y_fric", "custom_grav",
        "custom_grav_max_fall_speed", "apply_forces_no_limit", "reset_x_momentum", "reset_y_momentum",
        "iasa_on_hit_on_block", "iasa_on_hit",
        "interrupt_exceptions_string", "hit_cancel_exceptions_string", "release", "release_frame",
        "release_throw_pos_x", "release_throw_pos_y", "start_throw_pos_x", "start_throw_pos_y",
        "throw_positions", "no_collision_start_frame", "no_collision_end_frame", "flip_frame",
        "force_same_direction_as_previous_state", "host_commands", "fixed",
    ];
    const PRES: &[&str] = &["release_sfx", "release_sfx_volume", "screenshake_amount", "screenshake_frames", "hit_sound", "particle_location", "use_guard_sprites"];
    // later groups win in the original's dict (turn, engine, engine, gap, presentation)
    if PRES.contains(&k) {
        Some("presentation")
    } else if GAP.contains(&k) {
        Some("gap")
    } else if ENGINE.contains(&k) {
        Some("engine")
    } else if TURN.contains(&k) {
        Some("turn")
    } else {
        None
    }
}

fn prop_note(k: &str) -> &'static str {
    match k {
        "interruptible_on_opponent_turn" => "lets a move be left on the opponent's turn; no meaning without turns",
        "initiative_effect" => "turn-based initiative marker",
        "next_state_on_hold" => "hold-the-turn behavior of the menu",
        "end_feint" | "can_feint_if_possible" => "feint (free-cancel) mechanic of the turn UI",
        "instant_cancellable" => "instant-cancel mechanic of the turn UI",
        "usable_from_whiff_cancel_if_possible" => "whiff-cancel mechanic of the turn UI",
        "reversible" => "may the player turn the character around when choosing it",
        "self_hit_cancellable" => "can the move cancel into itself after a hit",
        "self_interruptable" => "can the move be re-selected into itself",
        "burst_cancellable" => "combo-breaker burst allowed during the move",
        "throw_techable" => "grabs can be teched when both fighters grab",
        "super_freeze_ticks" => "length of the super-move freeze",
        "extra_parry_hitlag" => "extra freeze when the move is perfectly parried",
        "whiff_meter_gain_multiplier" => "meter gained on whiffing",
        "interrupt_exceptions_string" => "categories excluded from the move's cancel targets",
        "release" => "throw choreography: when the held opponent is released",
        "release_frame" => "throw choreography: release frame",
        "release_throw_pos_x" | "release_throw_pos_y" => "throw choreography: opponent position at release",
        "start_throw_pos_x" | "start_throw_pos_y" => "throw choreography: opponent position at grab",
        "throw_positions" => "throw choreography: per-frame opponent positions",
        "grounded_hit_state" => "hit reaction state override (grounded victim)",
        "aerial_hit_state" => "hit reaction state override (airborne victim)",
        "cancellable" => "whether hitting allows cancelling",
        "block_cancel_allowed" => "whether blocking allows cancelling",
        "increment_combo" => "whether the hit counts as a combo hit",
        "knockdown_extends_hitstun" => "knockdown hits keep the victim stunned until they land",
        "minimum_damage" => "damage floor after combo scaling",
        "minimum_grounded_frames" => "minimum frames a launched victim stays down",
        "air_ground_bounce" => "airborne victims bounce off the ground",
        "launch_reversible" => "launch direction flips with relative position",
        "ignore_armor" => "hit ignores armor",
        "hits_vs_dizzy" => "can hit dizzy victims",
        "can_counter_hit" => "can produce a counter hit",
        "priority" => "hitbox priority when several overlap",
        "group" => "hitbox group (stops one attack hitting twice)",
        "di_modifier" => "directional-influence strength on this hit",
        "sdi_modifier" => "smash-DI strength on this hit",
        "combo_victim_hitlag" => "victim hitlag inside combos",
        "parriable" => "whether the hit can be parried",
        "chip_damage_modifier" => "chip damage on block",
        "meter_gain_modifier" => "meter gained on hit",
        "block_pushback_modifier" => "pushback on block",
        "vacuum" => "pulls the victim toward the hitbox",
        "send_away_from_center" => "knockback points away from the hitbox centre",
        "force_grounded" => "keeps the victim grounded",
        "hitbox_type" => "special hit behavior (throw damage, burst, flip, no-hitstun, detect)",
        "apply_custom_limits" | "custom_max_air_speed" => "per-move speed caps",
        "apply_custom_grav" => "per-move gravity override",
        "reset_y_momentum" => "stops only vertical speed on entry",
        "reset_x_momentum" => "stops only horizontal speed on entry",
        "iasa_on_hit_on_block" => "hit-cancel timing when blocked",
        "earliest_hitbox" => "engine hint for when the first hitbox appears",
        "ignore_air_option_use_in_combos" => "air actions are free during combos",
        "update_facing_on_exit" => "re-face the opponent when the move ends",
        _ => "",
    }
}

const HIT_CONSUMED: &[&str] = &[
    "x", "y", "width", "height", "start_tick", "active_ticks", "damage", "damage_in_combo", "cancellable",
    "hitstun_ticks", "combo_hitstun_ticks", "hitlag_ticks", "victim_hitlag", "knockback", "dir_x",
    "dir_y", "pushback_x", "hit_height", "plus_frames", "damage_proration", "scale_combo", "knockdown",
    "hard_knockdown", "ground_bounce", "wall_slam", "guard_break", "can_clash", "beats_grab",
    "hits_projectiles", "hits_vs_grounded", "hits_vs_aerial", "hits_otg", "looping", "loop_active_ticks", "group",
    "loop_inactive_ticks", "always_on", "followup_state", "throw_state", "hitbox_type",
    "grounded_hit_state", "aerial_hit_state", "knockdown_extends_hitstun", "minimum_grounded_frames",
    "air_ground_bounce", "launch_reversible", "vacuum", "send_away_from_center", "increment_combo",
    "minimum_damage", "block_cancel_allowed",
];

const HIT_PRESENTATION: &[&str] = &[
    "hit_sound", "whiff_sound", "hit_bass_sound", "hit_particle", "whiff_particle", "replace_hit_particle",
    "screenshake_amount", "screenshake_frames", "camera_bump_dir", "rumble", "bump_on_whiff",
    "whiff_sound_volume", "hit_sound_volume", "bass_sound_volume", "bass_on_whiff", "override_same_sfx",
    "override_same_wav", "sfx_pitch_variation", "_c_Grouping", "_c_Throw_Data",
];

fn hitkind(t: &str) -> Option<&'static str> {
    Some(match t {
        "ThrowHit" => "ThrowDamage",
        "Burst" => "Burst",
        "OffensiveBurst" => "OffensiveBurst",
        "NoHitstun" => "NoHitstun",
        "Flip" => "Flip",
        "Detect" => "Detect",
        _ => return None,
    })
}

fn reaction(t: &str) -> Option<&'static str> {
    Some(match t {
        "HurtGrounded" => "Stumble",
        "HurtAerial" => "Launch",
        "Grabbed" => "Held",
        "HurtDizzy" => "Dizzy",
        _ => return None,
    })
}

const COMPASS: [(&str, &str, (f64, f64)); 9] = [
    ("Neutral", "Neutral", (0.0, 0.0)),
    ("E", "Forward", (1.0, 0.0)),
    ("NE", "ForwardUp", (1.0, -1.0)),
    ("N", "Up", (0.0, -1.0)),
    ("NW", "BackUp", (-1.0, -1.0)),
    ("W", "Back", (-1.0, 0.0)),
    ("SW", "BackDown", (-1.0, 1.0)),
    ("S", "Down", (0.0, 1.0)),
    ("SE", "ForwardDown", (1.0, 1.0)),
];

/// Property names set in a scene object, minus the engine's own bookkeeping.
fn clean_keys(d: &Map<String, Value>) -> BTreeSet<String> {
    d.keys().filter(|k| !k.starts_with("_c_") && !matches!(k.as_str(), "script" | "__meta__" | "z_index")).cloned().collect()
}

// ------------------------------------------------------------------ script analysis
#[derive(Clone, Default)]
struct ScriptAnalysis {
    loc: usize,
    unknown: usize,
    calls: BTreeSet<String>,
    returns: BTreeSet<String>,
    physics: BTreeSet<String>,
    frame_hooks: BTreeSet<i64>,
    spawns: bool,
    exports: BTreeSet<String>,
    sets_hitbox: bool,
    sets_length: bool,
    sets_iasa: bool,
    invuln: bool,
    grounded_check: bool,
    scenes: BTreeSet<String>,
}

fn assigns(text: &str, re: &Regex) -> bool {
    re.find_iter(text).any(|m| text[m.end()..].chars().next() != Some('='))
}

fn func_bodies(text: &str) -> Vec<(String, Vec<String>)> {
    let mut bodies: Vec<(String, Vec<String>)> = Vec::new();
    let mut cur: Option<usize> = None;
    for line in text.split('\n') {
        if let Some(m) = lazy_re!(r"^func (\w+)\(").captures(line) {
            match bodies.iter().position(|(n, _)| *n == m[1]) {
                Some(i) => {
                    bodies[i].1.clear();
                    cur = Some(i);
                }
                None => {
                    bodies.push((m[1].to_string(), vec![]));
                    cur = Some(bodies.len() - 1);
                }
            }
            continue;
        }
        let Some(c) = cur else { continue };
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            cur = None;
            continue;
        }
        let s = line.trim();
        if !s.starts_with('#') {
            bodies[c].1.push(s.to_string());
        }
    }
    bodies
}

fn stock_line(s: &str) -> bool {
    lazy_re!(r#"^(?:host\.apply_(?:fric|forces|grav|forces_no_limit)\(\)|if auto_fall:|if not host\.is_grounded\(\):|return "Fall"|if host\.hp <= 0:|return "Knockdown"|pass)$"#).is_match(s)
}

fn analyze_text(text: &str) -> ScriptAnalysis {
    let bodies = func_bodies(text);
    let mut a = ScriptAnalysis::default();
    for (_, lines) in &bodies {
        for s in lines {
            if stock_line(s) {
                if let Some(m) = lazy_re!(r"^host\.apply_(\w+)\(\)$").captures(s) {
                    a.physics.insert(m[1].to_string());
                }
                continue;
            }
            a.unknown += 1;
            for c in lazy_re!(r"host\.(\w+)\(").captures_iter(s) {
                a.calls.insert(c[1].to_string());
            }
            for r in lazy_re!(r#"return "(\w+)""#).captures_iter(s) {
                a.returns.insert(r[1].to_string());
            }
        }
    }
    a.loc = text.split('\n').filter(|l| !l.trim().is_empty()).count();
    for m in lazy_re!(r"(?m)^func _frame_(\d+)").captures_iter(text) {
        a.frame_hooks.insert(m[1].parse().unwrap_or(0));
    }
    a.spawns = lazy_re!(r"\b(spawn_object|spawn_exported_projectile)\(").is_match(text);
    for m in lazy_re!(r"(?m)^export[^\n]*?\bvar\s+(\w+)").captures_iter(text) {
        a.exports.insert(m[1].to_string());
    }
    a.sets_hitbox = assigns(text, lazy_re!(r"\.(start_tick|active_ticks|damage|knockback|hitstun_ticks|dir_x|dir_y)\s*="));
    a.sets_length = assigns(text, lazy_re!(r"\banim_length\s*="));
    a.sets_iasa = assigns(text, lazy_re!(r"\biasa_at\s*="));
    a.invuln = lazy_re!(r"(start|end)_invulnerability|start_throw_invulnerability").is_match(text);
    a.grounded_check = text.contains("is_grounded");
    for m in lazy_re!(r#"preload\("res://([^"]+\.tscn)"\)"#).captures_iter(text) {
        a.scenes.insert(m[1].to_string());
    }
    a
}

struct StateScripts {
    own: Vec<String>,
    lib: Vec<String>,
    unknown: usize,
    calls: Vec<String>,
    physics: BTreeSet<String>,
    loc: usize,
    spawns: bool,
    sets_hitbox: bool,
    sets_length: bool,
    sets_iasa: bool,
    invuln: bool,
    grounded_check: bool,
    exports: BTreeSet<String>,
    frame_hooks: Vec<i64>,
}

fn lib_stem(rel: &str) -> String {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    base.rsplit_once('.').map(|x| x.0).unwrap_or(base).to_string()
}

// ------------------------------------------------------------------ the converter
pub struct Output {
    pub ron: String,
    pub report: Value,
    pub report_md: String,
    pub moves: usize,
    pub projectiles: usize,
}

struct Conv<'a> {
    game: &'a std::path::Path,
    char_id: String,
    src_name: String,
    data: &'a Value,
    own: Vec<&'a Value>,
    conv_ids: HashSet<String>,
    lib_ids: HashSet<String>,
    entered_by: HashMap<String, Vec<String>>,
    rp: Map<String, Value>,
    rd: Map<String, Value>,
    texts: HashMap<String, String>,
    analyses: HashMap<String, ScriptAnalysis>,
    defaults: HashMap<String, Map<String, Value>>,
    scene_ext: HashMap<i64, String>,
    anims: HashMap<String, Vec<Option<String>>>,
    // per-state scratch, as in the original's globals
    hit_unmapped: BTreeSet<String>,
    clamped: Vec<String>,
    prop_hits: Vec<((String, String, String), BTreeSet<String>)>,
    mode_use: BTreeSet<String>,
    scripted: bool,
}

/// What one of a move's option widgets hands its script as `data` by default: the widget's node
/// name, the value as a `ScriptVar`, and for directions the stick scale (1 for eight-way, 100 for
/// the free plot).
struct WidgetData {
    node: String,
    value: R,
    stick: Option<i64>,
}

fn map_var(items: Vec<(&str, R)>) -> R {
    R::Call("Map".into(), vec![R::List(items.into_iter().map(|(k, v)| R::tup2(R::str(k), v)).collect())])
}

fn xy_var(x: i64, y: i64) -> R {
    map_var(vec![("x", R::Call("Int".into(), vec![R::Int(x)])), ("y", R::Call("Int".into(), vec![R::Int(y)]))])
}

/// A move's default `data` (what the original's option widgets return before the player touches
/// them) and where the stick's direction goes in it: one widget's value is `data` itself, several
/// make a dictionary by widget name.
fn script_data(datas: &[WidgetData]) -> (Option<R>, Option<R>) {
    let stick = datas.iter().find(|d| d.stick.is_some());
    match datas {
        [] => (None, None),
        [one] => (Some(one.value.clone()), one.stick.map(|k| R::tup2(R::str(""), R::Int(k)))),
        many => (
            Some(map_var(many.iter().map(|d| (d.node.as_str(), d.value.clone())).collect())),
            stick.map(|d| R::tup2(R::Str(d.node.clone()), R::Int(d.stick.unwrap_or(1)))),
        ),
    }
}

fn read_file(game: &std::path::Path, rel: &str) -> String {
    let p = game.join(rel);
    if p.is_file() { read_text(&p).unwrap_or_default() } else { String::new() }
}

/// A Godot text literal as a plain value (numbers, bools, strings, Vector2).
fn parse_value(raw: &str) -> Value {
    let mut raw = raw.trim().to_string();
    if let Some(rest) = raw.strip_prefix('-') {
        let t = rest.trim_start();
        if t.len() < rest.len() && t.starts_with(|c: char| c.is_ascii_digit()) {
            raw = format!("-{t}");
        }
    }
    if raw == "true" || raw == "false" {
        return Value::Bool(raw == "true");
    }
    if lazy_re!(r"^-?\d+$").is_match(&raw) {
        return raw.parse::<i64>().map(Value::from).unwrap_or(Value::Null);
    }
    if lazy_re!(r"(?i)^-?\d+\.\d+(e[-+]?\d+)?$").is_match(&raw) {
        return json!(raw.parse::<f64>().unwrap_or(0.0));
    }
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        return Value::String(raw[1..raw.len() - 1].to_string());
    }
    if let Some(c) = lazy_re!(r"^Vector2\(\s*([-\d.e+]+)\s*,\s*([-\d.e+]+)\s*\)$").captures(&raw) {
        return json!([c[1].parse::<f64>().unwrap_or(0.0), c[2].parse::<f64>().unwrap_or(0.0)]);
    }
    Value::String(raw)
}

fn ext_map(text: &str) -> HashMap<i64, String> {
    lazy_re!(r#"\[ext_resource path="res://([^"]+)" type="[^"]*" id=(\d+)\]"#)
        .captures_iter(text)
        .map(|c| (c[2].parse().unwrap_or(0), c[1].to_string()))
        .collect()
}

fn animations_in(text: &str) -> HashMap<String, Vec<Option<String>>> {
    let ext = ext_map(text);
    let mut out = HashMap::new();
    for m in lazy_re!(r#"(?s)"frames": \[(.*?)\],\n"loop": (?:true|false),\n"name": "([^"]+)""#).captures_iter(text) {
        let ids: Vec<i64> = lazy_re!(r"ExtResource\(\s*(\d+)\s*\)").captures_iter(&m[1]).map(|c| c[1].parse().unwrap_or(0)).collect();
        out.insert(m[2].to_string(), ids.iter().map(|i| ext.get(i).cloned()).collect());
    }
    out
}

impl<'a> Conv<'a> {
    fn read(&mut self, rel: &str) -> String {
        if let Some(t) = self.texts.get(rel) {
            return t.clone();
        }
        let t = read_file(self.game, rel);
        self.texts.insert(rel.to_string(), t.clone());
        t
    }

    fn analyze_script(&mut self, rel: &str) -> ScriptAnalysis {
        if let Some(a) = self.analyses.get(rel) {
            return a.clone();
        }
        let text = self.read(rel);
        let a = analyze_text(&text);
        self.analyses.insert(rel.to_string(), a.clone());
        a
    }

    fn script_defaults(&mut self, rel: &str) -> Map<String, Value> {
        if let Some(d) = self.defaults.get(rel) {
            return d.clone();
        }
        let text = self.read(rel);
        let mut out = Map::new();
        for m in lazy_re!(r"(?m)^export[^\n]*?\bvar\s+(\w+)\s*(?::\s*\w+)?\s*=\s*(.+)$").captures_iter(&text) {
            out.insert(m[1].to_string(), parse_value(&m[2]));
        }
        self.defaults.insert(rel.to_string(), out.clone());
        out
    }

    fn widget_root(&mut self, rel: &str, depth: usize) -> Map<String, Value> {
        let text = self.read(rel);
        if text.is_empty() || depth > 4 {
            return Map::new();
        }
        let ext = ext_map(&text);
        let lines: Vec<&str> = text.split('\n').collect();
        let Some(start) = lines.iter().position(|l| l.starts_with("[node ")) else { return Map::new() };
        let mut base = Map::new();
        if let Some(c) = lazy_re!(r"instance=ExtResource\(\s*(\d+)\s*\)").captures(lines[start]) {
            if let Some(p) = ext.get(&c[1].parse::<i64>().unwrap_or(-1)) {
                let p = p.clone();
                base = self.widget_root(&p, depth + 1);
            }
        }
        for l in &lines[start + 1..] {
            if l.starts_with('[') {
                break;
            }
            if let Some(m) = lazy_re!(r"^(\w+) = (.+)$").captures(l) {
                base.insert(m[1].to_string(), parse_value(&m[2]));
            }
        }
        base
    }

    fn scene_box(&mut self, node: &str) -> R {
        let text = self.read("characters/BaseChar.tscn");
        let pat = format!(r#"(?s)\[node name="{}"[^\]]*\]\n(.*?)\n\n"#, regex::escape(node));
        let mut props: HashMap<String, i64> = HashMap::new();
        if let Some(m) = Regex::new(&pat).expect("valid").captures(&text) {
            for c in lazy_re!(r"(?m)^(\w+) = (-?\d+)$").captures_iter(&m[1]) {
                props.insert(c[1].to_string(), c[2].parse().unwrap_or(0));
            }
        }
        let g = |k: &str| R::Int(*props.get(k).unwrap_or(&0));
        R::st("Rect", vec![("x", g("x")), ("y", g("y")), ("half_w", g("width")), ("half_h", g("height"))])
    }

    fn analyze_state_scripts(&mut self, state: &Value) -> StateScripts {
        let chain: Vec<String> = at(state, "class_chain").as_array().cloned().unwrap_or_default().iter().filter_map(|v| v.as_str().map(str::to_string)).filter(|p| !FRAMEWORK.contains(&p.as_str())).collect();
        let (mut own, mut lib) = (vec![], vec![]);
        for p in chain {
            if p.starts_with("characters/states/") && lib_stem(&p) != "Idle" {
                lib.push(p);
            } else {
                own.push(p);
            }
        }
        let infos: Vec<ScriptAnalysis> = own.iter().map(|p| self.analyze_script(p)).collect();
        let lib_infos: Vec<ScriptAnalysis> = lib.iter().map(|p| self.analyze_script(p)).collect();
        let mut s = StateScripts {
            unknown: infos.iter().map(|i| i.unknown).sum(),
            calls: infos.iter().flat_map(|i| i.calls.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect(),
            physics: infos.iter().flat_map(|i| i.physics.iter().cloned()).collect(),
            loc: infos.iter().map(|i| i.loc).sum(),
            spawns: infos.iter().any(|i| i.spawns),
            sets_hitbox: infos.iter().any(|i| i.sets_hitbox),
            sets_length: infos.iter().any(|i| i.sets_length),
            sets_iasa: infos.iter().any(|i| i.sets_iasa),
            invuln: infos.iter().any(|i| i.invuln),
            grounded_check: infos.iter().any(|i| i.grounded_check),
            exports: infos.iter().chain(lib_infos.iter()).flat_map(|i| i.exports.iter().cloned()).collect(),
            frame_hooks: infos.iter().flat_map(|i| i.frame_hooks.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect(),
            own,
            lib,
        };
        let _ = &mut s;
        s
    }

    fn tags_of(&self, groups: &Value) -> Vec<String> {
        groups.as_array().map(|a| a.iter().filter_map(Value::as_str).map(snake).collect()).unwrap_or_default()
    }

    fn guess_input(&self, s: &Value) -> (Option<R>, Option<String>) {
        let k = at(s, "key");
        let name = at(s, "name").as_str().unwrap_or("");
        if let Some(by) = self.entered_by.get(name) {
            let mut v: Vec<&String> = by.iter().collect::<BTreeSet<_>>().into_iter().collect();
            v.sort();
            return (None, Some(format!("entered by {}", v.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", "))));
        }
        if !k.get("show_in_menu").map(|v| truthy(Some(v))).unwrap_or(true) || !k.get("selectable").map(|v| truthy(Some(v))).unwrap_or(true) {
            return (None, Some("not selectable in the original menu".into()));
        }
        let typ = k.get("type").and_then(Value::as_str);
        let button = if lazy_re!(r"^(Double|Super)?Jump$").is_match(name) {
            "Jump"
        } else if name.contains("Dash") {
            "Dash"
        } else if name.ends_with("Grab") {
            "Grab"
        } else if name == "Taunt" {
            "Taunt"
        } else if typ == Some("Super") {
            "Super"
        } else if typ == Some("Defense") {
            "Guard"
        } else if typ == Some("Special") {
            "Special"
        } else if typ == Some("Attack") {
            "Light"
        } else {
            return (None, Some("no menu type (engine-entered state)".into()));
        };
        let direction = if lazy_re!(r"Upward|Uppercut|Up").is_match(name) {
            "Up"
        } else if lazy_re!(r"Down|Drop|Sweep|Stomp|Dive").is_match(name) {
            "Down"
        } else if name.contains("Forward") {
            "Forward"
        } else if name.contains("Back") {
            "Back"
        } else {
            "Any"
        };
        let mut fields = vec![("button", R::s(button))];
        if direction != "Any" {
            fields.push(("direction", R::s(direction)));
        }
        (Some(R::st("Input", fields)), None)
    }

    #[allow(clippy::type_complexity)]
    fn build_params(&mut self, s: &Value) -> (Vec<R>, Vec<String>, Vec<String>, Vec<WidgetData>) {
        let (mut out, mut dropped, mut notes) = (Vec::<R>::new(), Vec::<String>::new(), Vec::<String>::new());
        let mut datas: Vec<WidgetData> = Vec::new();
        let mut names: HashSet<String> = HashSet::new();
        let params = at(s, "player_parameters").as_array().cloned().unwrap_or_default();
        for p in &params {
            let inst = at(p, "instance").as_str().unwrap_or("").to_string();
            let stem = inst.rsplit('/').next().unwrap_or("").to_string();
            let props = at(p, "props").as_object().cloned().unwrap_or_default();
            let pname = at(p, "name").as_str().unwrap_or("").to_string();
            let mut nm = snake(&pname);
            if !nm.starts_with(|c: char| c.is_ascii_lowercase()) {
                nm = format!("p_{nm}");
            }
            let (base, mut j) = (nm.clone(), 2);
            while names.contains(&nm) {
                nm = format!("{base}_{j}");
                j += 1;
            }
            names.insert(nm.clone());
            let mut kind: Option<R> = None;
            let ptype = p.get("type").and_then(Value::as_str);
            if stem.contains("Plot") {
                let mut merged = Map::new();
                merged.insert("limit_angle".into(), json!(false));
                merged.insert("limit_range_degrees".into(), json!(90.0));
                merged.insert("limit_center_degrees".into(), json!(0.0));
                merged.insert("limit_symmetrical".into(), json!(false));
                merged.insert("default_value".into(), json!([0.0, 0.0]));
                merged.insert("min_length".into(), json!(0.0));
                for (k, v) in self.widget_root(&inst, 0) {
                    merged.insert(k, v);
                }
                for (k, v) in &props {
                    merged.insert(k.clone(), v.clone());
                }
                let dv: Vec<f64> = match merged.get("default_value") {
                    Some(Value::Object(o)) if o.contains_key("Vector2") => o["Vector2"].as_array().map(|a| a.iter().map(|x| num(Some(x), 0.0)).collect()).unwrap_or_default(),
                    Some(Value::Array(a)) => a.iter().map(|x| num(Some(x), 0.0)).collect(),
                    _ => vec![0.0, 0.0],
                };
                let mut fields = vec![("steps", R::s("Free"))];
                if truthy(merged.get("limit_angle")) {
                    fields.push(("arc", R::st("Arc", vec![("center_deg", f(num(merged.get("limit_center_degrees"), 0.0))), ("range_deg", f(num(merged.get("limit_range_degrees"), 0.0)))])));
                    if truthy(merged.get("limit_symmetrical")) {
                        notes.push(format!("{pname}: arc is mirrored in the original; only one side is expressed"));
                    }
                }
                let pct = |i: usize| (dv.get(i).copied().unwrap_or(0.0) * 100.0).round() as i64;
                datas.push(WidgetData { node: pname.clone(), value: xy_var(pct(0), pct(1)), stick: Some(100) });
                fields.push(("default", R::Tuple(dv.into_iter().map(f).collect())));
                kind = Some(R::st("Direction", fields));
            } else if stem.contains("8Way") {
                let disabled: HashSet<&str> = props.iter().filter(|(_, v)| **v == Value::Bool(false)).map(|(k, _)| k.as_str()).collect();
                let allowed: Vec<&str> = if disabled.is_empty() { vec![] } else { COMPASS.iter().filter(|(key, _, _)| !disabled.contains(key)).map(|(_, n, _)| *n).collect() };
                let order: Vec<&str> = if allowed.is_empty() { COMPASS.iter().map(|(_, n, _)| *n).collect() } else { allowed.clone() };
                let pick = if order.contains(&"Forward") { "Forward" } else { order[0] };
                let v = COMPASS.iter().find(|(_, n, _)| *n == pick).map(|(_, _, v)| *v).unwrap_or((0.0, 0.0));
                let mut fields = vec![("steps", R::s("Eight"))];
                if !allowed.is_empty() {
                    fields.push(("allowed", R::List(allowed.iter().map(|a| R::s(a)).collect())));
                }
                datas.push(WidgetData { node: pname.clone(), value: xy_var(v.0 as i64, v.1 as i64), stick: Some(1) });
                fields.push(("default", R::tup2(f(v.0), f(v.1))));
                kind = Some(R::st("Direction", fields));
            } else if stem.contains("HorizSlider") {
                let centered = props.get("centered").map(|v| truthy(Some(v))).unwrap_or(true);
                let lo = if props.contains_key("min_value") { num(props.get("min_value"), 0.0) } else if centered { -100.0 } else { 0.0 };
                let hi = num(props.get("max_value"), 100.0);
                let d = 0f64.max(lo).min(hi);
                datas.push(WidgetData { node: pname.clone(), value: map_var(vec![("x", R::Call("Int".into(), vec![R::Int(d.round() as i64)]))]), stick: None });
                kind = Some(R::st("Slider", vec![("min", f(lo)), ("max", f(hi)), ("default", f(d))]));
            } else if stem.contains("CountOption") {
                let lo = if props.contains_key("min_value") { to_int(props.get("min_value")) } else { 0 };
                let hi = if props.contains_key("max_value") { to_int(props.get("max_value")) } else { 10 };
                datas.push(WidgetData { node: pname.clone(), value: map_var(vec![("count", R::Call("Int".into(), vec![R::Int(lo)]))]), stick: None });
                kind = Some(R::st("Count", vec![("min", R::Int(lo)), ("max", R::Int(hi)), ("default", R::Int(lo))]));
            } else if ptype == Some("CheckButton") {
                datas.push(WidgetData { node: pname.clone(), value: R::Call("Bool".into(), vec![R::Bool(truthy(props.get("pressed")))]), stick: None });
                kind = Some(R::st("Toggle", vec![("default", R::Bool(truthy(props.get("pressed"))))]));
            } else if ptype == Some("OptionButton") {
                let raw = props.get("items").and_then(|i| i.get("raw")).and_then(Value::as_str).unwrap_or("").to_string();
                let options: Vec<String> = lazy_re!(r#""([^"]*)""#).captures_iter(&raw).map(|c| c[1].to_string()).collect();
                if !options.is_empty() {
                    let sel = to_int(props.get("selected")).max(0) as usize;
                    let def = options.get(sel).unwrap_or(&options[0]).clone();
                    let (id, name) = (R::Call("Int".into(), vec![R::Int(options.iter().position(|o| *o == def).unwrap_or(0) as i64)]), R::Call("Str".into(), vec![R::Str(def.clone())]));
                    datas.push(WidgetData { node: pname.clone(), value: map_var(vec![("id", id), ("name", name)]), stick: None });
                    kind = Some(R::st("Choice", vec![("options", R::strs(options.clone())), ("default", R::Str(def))]));
                }
            }
            let Some(kind) = kind else {
                dropped.push(format!("{pname} ({})", if stem.is_empty() { ptype.unwrap_or("None") } else { &stem }));
                continue;
            };
            out.push(R::st("Param", vec![("name", R::Str(nm)), ("kind", kind)]));
        }
        // the first direction is the one the stick controls
        for prm in out.iter_mut() {
            if prm.get("kind").map(|k| k.name()) == Some("Direction") {
                prm.put("binding", R::s("Stick"));
                break;
            }
        }
        (out, dropped, notes, datas)
    }

    fn parse_throw_positions(&self, raw: &str) -> HashMap<String, (i64, i64)> {
        let mut out = HashMap::new();
        let re = lazy_re!(r#"(?:ExtResource\(\s*(\d+)\s*\)|Resource\(\s*"res://([^"]+)"\s*\))\s*:\s*\{\s*"x":\s*(-?\d+),\s*"y":\s*(-?\d+)\s*\}"#);
        for m in re.captures_iter(raw) {
            let tex = match m.get(1) {
                Some(id) => self.scene_ext.get(&id.as_str().parse().unwrap_or(-1)).cloned(),
                None => m.get(2).map(|x| x.as_str().to_string()),
            };
            if let Some(t) = tex {
                out.insert(t, (m[3].parse().unwrap_or(0), m[4].parse().unwrap_or(0)));
            }
        }
        out
    }

    fn reaction_fields(grounded: Option<&str>, aerial: Option<&str>, min_frames: Option<&Value>) -> Vec<(&'static str, R)> {
        let mut rx = Vec::new();
        for (label, st, default) in [("grounded", grounded, "Stumble"), ("aerial", aerial, "Launch")] {
            if let Some(val) = st.and_then(reaction) {
                if val != default {
                    rx.push((label, R::s(val)));
                }
            }
        }
        if let Some(m) = min_frames {
            if !m.is_null() && num(Some(m), -1.0) != -1.0 {
                rx.push(("min_launch_frames", json_to_r(Some(m))));
            }
        }
        rx
    }

    fn build_throw(&mut self, s: &Value, k: &Value, ex: &Map<String, Value>, anim_len: i64, losses: &mut Vec<String>, notes: &mut Vec<String>) -> Option<R> {
        const THROW_SCRIPT: &str = "characters/states/ThrowState.gd";
        if !at(s, "class_chain").as_array().is_some_and(|c| c.iter().any(|x| x.as_str() == Some(THROW_SCRIPT))) {
            return None;
        }
        let d = self.script_defaults(THROW_SCRIPT);
        let tv = |key: &str| -> Option<&Value> { ex.get(key).or_else(|| d.get(key)) };
        let use_start = truthy(tv("use_start_throw_pos"));
        let start = (to_int(tv("start_throw_pos_x")), to_int(tv("start_throw_pos_y")));
        let rframe = to_int(tv("release_frame"));
        let release_on = truthy(tv("release")) && rframe > 0;
        let tp = match ex.get("throw_positions") {
            Some(Value::Object(o)) => self.parse_throw_positions(o.get("raw").and_then(Value::as_str).unwrap_or("")),
            _ => HashMap::new(),
        };
        let anim = k.get("sprite_animation").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(at(s, "name").as_str().unwrap_or("")).to_string();
        let frames = self.anims.get(&anim).cloned();
        let tpf = to_int(Some(ex.get("ticks_per_frame").unwrap_or(&json!(1)))).max(1);
        let last_tick = if release_on { rframe - 1 } else { anim_len - 1 };
        if !tp.is_empty() && !frames.as_ref().is_some_and(|f| !f.is_empty()) {
            losses.push(format!("throw positions are keyed by animation frames but animation '{anim}' was not found"));
        }
        let has_frames = frames.as_ref().is_some_and(|f| !f.is_empty());
        let mut pos = if use_start { start } else { (0, 0) };
        let (mut first, mut prev): (Option<(i64, i64)>, Option<(i64, i64)>) = (None, None);
        let mut keys = Vec::new();
        for t in 0..=last_tick.max(0) {
            if let (true, Some(fr)) = (has_frames, &frames) {
                let idx = ((t / tpf) as usize).min(fr.len() - 1);
                if let Some(Some(tex)) = fr.get(idx) {
                    if let Some(p) = tp.get(tex) {
                        pos = *p;
                    }
                }
            }
            if first.is_none() {
                first = Some(pos);
            } else if Some(pos) != prev {
                keys.push(R::st("HoldKey", vec![("at", R::Int(t + 1)), ("offset", R::tup2(R::Int(pos.0), R::Int(pos.1)))]));
            }
            prev = Some(pos);
        }
        let t2 = |p: (i64, i64)| R::tup2(R::Int(p.0), R::Int(p.1));
        let mut fields: Vec<(&str, R)> = vec![("hold", t2(first.unwrap_or(start)))];
        if !keys.is_empty() {
            fields.push(("keyframes", R::List(keys)));
        }
        if release_on {
            let off = if truthy(tv("use_release_throw_pos")) { (to_int(tv("release_throw_pos_x")), to_int(tv("release_throw_pos_y"))) } else { prev.unwrap_or((0, 0)) };
            let mut hit = vec![("damage", R::Int(to_int(tv("damage"))))];
            if to_int(tv("damage_in_combo")) != -1 {
                hit.push(("damage_in_combo", R::Int(to_int(tv("damage_in_combo")))));
            }
            hit.push(("hitstun", R::Int(to_int(tv("hitstun_ticks")))));
            hit.push(("knockback", R::st("Knockback", vec![("dir", R::tup2(f(num(tv("dir_x"), 0.0)), f(num(tv("dir_y"), 0.0)))), ("strength", f(num(tv("knockback"), 0.0)))])));
            let mut sc = vec![];
            if truthy(tv("damage_proration")) {
                sc.push(("proration", min_cap(Some(&json!(to_int(tv("damage_proration")))), 15)));
            }
            if !truthy(tv("increment_combo")) {
                sc.push(("counts_as_hit", R::Bool(false)));
            }
            if !sc.is_empty() {
                hit.push(("scaling", R::st("Scaling", sc)));
            }
            let mut efx = vec![];
            if truthy(tv("knockdown")) {
                efx.push(("knockdown", R::s(if truthy(tv("hard_knockdown")) { "Hard" } else { "Soft" })));
                if !truthy(tv("knockdown_extends_hitstun")) {
                    efx.push(("stun_until_landing", R::Bool(false)));
                }
            }
            for key in ["ground_bounce", "air_ground_bounce", "wall_slam"] {
                if truthy(tv(key)) {
                    efx.push((key, R::Bool(true)));
                }
            }
            if !efx.is_empty() {
                hit.push(("effects", R::st("Effects", efx)));
            }
            let rx = Self::reaction_fields(string(tv("grounded_hit_state")), string(tv("aerial_hit_state")), tv("minimum_grounded_frames"));
            if !rx.is_empty() {
                hit.push(("reaction", R::st("Reaction", rx)));
            }
            if (num(tv("di_modifier"), 1.0) - 1.0).abs() > 1e-9 {
                hit.push(("di_modifier", f(num(tv("di_modifier"), 0.0))));
            }
            let height = match tv("hit_height") {
                Some(Value::String(s)) => s.rsplit('.').next().unwrap_or("").to_string(),
                Some(other) => other.to_string(),
                None => "None".to_string(),
            };
            if height == "High" || height == "Low" {
                hit.push(("height", R::s(&height)));
            }
            fields.push(("release", R::st("Release", vec![("at", R::Int(rframe)), ("offset", t2(off)), ("hit", R::st("ReleaseHit", hit))])));
        }
        if truthy(tv("reverse")) {
            fields.push(("reverse", R::Bool(true)));
        }
        if !has_frames && tp.is_empty() {
            notes.push("no animation-driven hold positions".into());
        }
        Some(R::st("Throw", fields))
    }

    fn note_prop(&mut self, level: &str, key: &str, class: &str, state: &str) {
        let k = (level.to_string(), key.to_string(), class.to_string());
        match self.prop_hits.iter_mut().find(|(kk, _)| *kk == k) {
            Some((_, set)) => {
                set.insert(state.to_string());
            }
            None => self.prop_hits.push((k, BTreeSet::from([state.to_string()]))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn convert_hitbox(&mut self, h: &Value, move_total: Option<i64>, open_ended: bool, losses: &mut Vec<String>, n_boxes: usize, id_set: &mut HashSet<String>) -> R {
        let hk = at(h, "key");
        let ex = at(h, "explicit").as_object().cloned().unwrap_or_default();
        let hname = at(h, "name").as_str().unwrap_or("");
        let is_throw = at(h, "kind").as_str() == Some("throwbox");
        let g = |k: &str| hk.get(k);
        let start = to_int(Some(g("start_tick").unwrap_or(&json!(0))));
        let act = to_int(Some(g("active_ticks").unwrap_or(&json!(0))));
        let wf = 1.max(start + 1);
        let wt = wf.max(start + act);
        if start < 0 {
            self.clamped.push(hname.to_string());
        }
        let base_id = if n_boxes > 1 { snake(hname) } else { "main".to_string() };
        let (mut hid, mut j) = (base_id.clone(), 2);
        while id_set.contains(&hid) {
            hid = format!("{base_id}_{j}");
            j += 1;
        }
        id_set.insert(hid.clone());
        let mut fl: Vec<(&str, R)> = vec![("id", R::Str(hid))];
        fl.push(("rect", R::st("Rect", vec![("x", json_to_r(g("x"))), ("y", json_to_r(g("y"))), ("half_w", pyabs(&hk["width"])), ("half_h", pyabs(&hk["height"]))])));
        let mut window = span(wf, wt);
        let put = |fl: &mut Vec<(&str, R)>, k: &'static str, v: R| match fl.iter_mut().find(|(n, _)| *n == k) {
            Some(slot) => slot.1 = v,
            None => fl.push((k, v)),
        };
        put(&mut fl, "window", window.clone());
        if truthy(g("always_on")) {
            put(&mut fl, "until_end", R::Bool(true));
            if !open_ended && move_total.is_some_and(|t| t != 0) {
                window = span(wf, move_total.unwrap_or(0));
                put(&mut fl, "window", window.clone());
            }
        }
        if truthy(g("looping")) {
            put(&mut fl, "repeat", R::st("Repeat", vec![("active", json_to_r(Some(g("loop_active_ticks").unwrap_or(&json!(2))))), ("inactive", json_to_r(Some(g("loop_inactive_ticks").unwrap_or(&json!(2)))))]));
        }
        // the tag scripts read off a hitbox that hit them (only scripted fighters run scripts)
        if let Some(m) = string(ex.get("misc_data")).filter(|s| self.scripted && !s.is_empty()) {
            put(&mut fl, "misc_data", R::str(m));
        }
        if let Some(fu) = string(g("followup_state")).filter(|s| !s.is_empty()) {
            put(&mut fl, "followup", R::Str(sid(fu)));
            if !self.conv_ids.contains(&sid(fu)) {
                losses.push(format!("hitbox {hname}: follow-up state {fu} was not converted"));
            }
        }
        if is_throw {
            if let Some(ts) = string(ex.get("throw_state")).filter(|s| !s.is_empty()) {
                if self.conv_ids.contains(&sid(ts)) {
                    put(&mut fl, "throw_into", R::Str(sid(ts)));
                } else {
                    losses.push(format!("throw into {ts} is an engine-owned state; no choreography emitted"));
                }
            }
        }
        if !is_throw {
            if let Some(kind) = string(g("hitbox_type")).and_then(hitkind) {
                put(&mut fl, "kind", R::s(kind));
            }
        }
        let damage = &hk["damage"];
        if num(Some(damage), 0.0) < 0.0 {
            losses.push(format!("hitbox {hname}: damage {} is a placeholder the script overwrites; emitted as 0", damage_text(damage)));
        }
        put(&mut fl, "damage", max0(damage));
        if num(g("damage_in_combo"), -1.0) != -1.0 {
            put(&mut fl, "damage_in_combo", json_to_r(g("damage_in_combo")));
        }
        if num(g("minimum_damage"), 0.0) > 0.0 {
            put(&mut fl, "min_damage", json_to_r(g("minimum_damage")));
        }
        put(&mut fl, "hitstun", json_to_r(g("hitstun_ticks")));
        if num(g("combo_hitstun_ticks"), -1.0) != -1.0 {
            put(&mut fl, "hitstun_in_combo", json_to_r(g("combo_hitstun_ticks")));
        }
        let victim = if num(g("victim_hitlag"), -1.0) != -1.0 { json_to_r(g("victim_hitlag")) } else { json_to_r(g("hitlag_ticks")) };
        put(&mut fl, "hitlag", R::st("Hitlag", vec![("attacker", json_to_r(g("hitlag_ticks"))), ("victim", victim)]));
        let mut kb = vec![("dir", R::tup2(f(num(g("dir_x"), 0.0)), f(num(g("dir_y"), 0.0)))), ("strength", f(num(g("knockback"), 0.0)))];
        if truthy(g("launch_reversible")) {
            kb.push(("reversible", R::Bool(true)));
        }
        if truthy(g("vacuum")) {
            kb.push(("from", R::s("TowardHitbox")));
        } else if truthy(g("send_away_from_center")) {
            kb.push(("from", R::s("AwayFromHitbox")));
        }
        put(&mut fl, "knockback", R::st("Knockback", kb));
        if num(g("pushback_x"), 0.0) != 0.0 {
            put(&mut fl, "pushback", f(num(g("pushback_x"), 0.0)));
        }
        put(&mut fl, "height", R::s(string(g("hit_height")).unwrap_or("Mid")));
        if num(g("plus_frames"), 0.0) != 0.0 {
            put(&mut fl, "block_advantage", json_to_r(g("plus_frames")));
        }
        let mut sc = vec![];
        if truthy(g("damage_proration")) {
            sc.push(("proration", min_cap(g("damage_proration"), 15)));
        }
        if !g("scale_combo").map(|v| truthy(Some(v))).unwrap_or(true) {
            sc.push(("scale_combo", R::Bool(false)));
        }
        if !g("increment_combo").map(|v| truthy(Some(v))).unwrap_or(true) {
            sc.push(("counts_as_hit", R::Bool(false)));
        }
        if !sc.is_empty() {
            put(&mut fl, "scaling", R::st("Scaling", sc));
        }
        let mut efx = vec![];
        if truthy(g("knockdown")) {
            efx.push(("knockdown", R::s(if truthy(g("hard_knockdown")) { "Hard" } else { "Soft" })));
            if !g("knockdown_extends_hitstun").map(|v| truthy(Some(v))).unwrap_or(true) {
                efx.push(("stun_until_landing", R::Bool(false)));
            }
        }
        for key in ["ground_bounce", "air_ground_bounce", "wall_slam", "guard_break"] {
            if truthy(g(key)) {
                efx.push((key, R::Bool(true)));
            }
        }
        if is_throw {
            efx.push(("throw", R::Bool(true)));
        }
        if !efx.is_empty() {
            put(&mut fl, "effects", R::st("Effects", efx));
        }
        let mut rx = vec![];
        let (gs, as_) = (string(g("grounded_hit_state")), string(g("aerial_hit_state")));
        for (label, val) in [("grounded", gs.and_then(reaction)), ("aerial", as_.and_then(reaction))] {
            let default = if label == "grounded" { "Stumble" } else { "Launch" };
            if let Some(v) = val {
                if v != default {
                    rx.push((label, R::s(v)));
                }
            }
        }
        for (label, st) in [("grounded", gs), ("aerial", as_)] {
            if let Some(st) = st.filter(|s| !s.is_empty()) {
                if reaction(st).is_none() {
                    self.hit_unmapped.insert(format!("{label}_hit_state={st}"));
                }
            }
        }
        if num(g("minimum_grounded_frames"), -1.0) != -1.0 {
            rx.push(("min_launch_frames", json_to_r(g("minimum_grounded_frames"))));
        }
        if !rx.is_empty() {
            put(&mut fl, "reaction", R::st("Reaction", rx));
        }
        let group = to_int(g("group"));
        if group != 0 {
            put(&mut fl, "group", R::Int(group));
        }
        let mut flags = vec![];
        let flag_false = |k: &str| !g(k).map(|v| truthy(Some(v))).unwrap_or(true);
        if flag_false("can_clash") {
            flags.push(("can_clash", R::Bool(false)));
        }
        if flag_false("beats_grab") {
            flags.push(("beats_throw", R::Bool(false)));
        }
        if flag_false("hits_projectiles") {
            flags.push(("hits_projectiles", R::Bool(false)));
        }
        if flag_false("hits_vs_grounded") {
            flags.push(("hits_grounded", R::Bool(false)));
        }
        if flag_false("hits_vs_aerial") {
            flags.push(("hits_air", R::Bool(false)));
        }
        if truthy(g("hits_otg")) {
            flags.push(("hits_downed", R::Bool(true)));
        }
        if !flags.is_empty() {
            put(&mut fl, "flags", R::st("HitFlags", flags));
        }
        let mut un: BTreeSet<String> = clean_keys(&ex).into_iter().filter(|k| !HIT_CONSUMED.contains(&k.as_str()) && !HIT_PRESENTATION.contains(&k.as_str())).collect();
        if let Some(t) = string(g("hitbox_type")) {
            if t != "Normal" && hitkind(t).is_none() {
                un.insert("hitbox_type".into());
            }
        }
        self.hit_unmapped.extend(un);
        R::st("HitBox", fl)
    }
}

/// How Python prints a number inside an f-string.
fn damage_text(v: &Value) -> String {
    match v.as_i64() {
        Some(i) => i.to_string(),
        None => crate::ron::fnum(v.as_f64().unwrap_or(0.0)),
    }
}

include!("convert_state.rs");
