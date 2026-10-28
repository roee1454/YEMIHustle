#!/usr/bin/env python3
"""Convert a whole extracted YOMI Hustle character into our RON format, with a loss report.

Usage:
  python3 -I tools/convert_character.py <ref_root> <extracted.json> <character_id> <out.ron> <report_prefix>
  python3 -I tools/convert_character.py <ref_root> <extracted.json> <character_id> <out.ron> - --only <State>

Writes <out.ron>, and (unless --only) <report_prefix>.md and <report_prefix>.json.
The report lists, per state, everything that could not be expressed in the data format.
Pure standard library; reads the decompiled scripts only as text, never runs them.
"""
import json
import os
import re
import sys
from collections import defaultdict

args = [a for a in sys.argv[1:]]
ONLY = None
if "--only" in args:
    i = args.index("--only")
    ONLY = args[i + 1]
    del args[i:i + 2]
REF, SRC, CHAR_ID, OUT, REPORT = args[:5]

# ------------------------------------------------------------------ RON emitter
class S:
    def __init__(self, _type, /, **fields):
        self.name, self.fields = _type, fields


class Raw(str):
    pass


def fnum(x):
    x = round(float(x), 6)
    if x == 0:
        return "0.0"          # also turns float noise like -4e-08 and -0.0 into a clean zero
    return str(int(x)) + ".0" if x == int(x) else repr(x)


def emit(v, ind=0):
    pad = "    " * ind
    if isinstance(v, Raw):
        return str(v)
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        return fnum(v)
    if isinstance(v, str):
        return '"' + v.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(v, tuple):
        return "(" + ", ".join(emit(x, ind) for x in v) + ")"
    if isinstance(v, list):
        if not v:
            return "[]"
        if all(isinstance(x, (str, int, float, bool)) for x in v) and sum(len(emit(x)) + 2 for x in v) < 70:
            return "[" + ", ".join(emit(x) for x in v) + "]"
        return "[\n" + "".join(pad + "    " + emit(x, ind + 1) + ",\n" for x in v) + pad + "]"
    if isinstance(v, S):
        if not v.fields:
            return v.name
        flat = ", ".join(f"{k}: {emit(x, ind)}" for k, x in v.fields.items())
        if len(flat) < 78 and "\n" not in flat:
            return f"{v.name}({flat})"
        body = "".join(pad + "    " + f"{k}: {emit(x, ind + 1)},\n" for k, x in v.fields.items())
        return f"{v.name}(\n{body}{pad})"
    raise TypeError(type(v))


# ------------------------------------------------------------------ small helpers
def snake(name):
    s = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name)
    return re.sub(r"[^a-z0-9_]", "_", s.lower())


ID_ALIAS = {"Wait": "idle"}


def sid(name):
    return ID_ALIAS.get(name, snake(name))


def num(v, default=0.0):
    if v is None or v == "":
        return default
    return float(v)


def read(rel):
    p = os.path.join(REF, rel)
    return open(p, encoding="utf-8").read() if os.path.isfile(p) else ""


def scene_box(node):
    text = read("characters/BaseChar.tscn")
    m = re.search(r'\[node name="%s"[^\]]*\]\n(.*?)\n\n' % node, text, re.S)
    props = dict(re.findall(r"^(\w+) = (-?\d+)$", m.group(1), re.M)) if m else {}
    return S("Rect", x=int(props.get("x", 0)), y=int(props.get("y", 0)),
             half_w=int(props.get("width", 0)), half_h=int(props.get("height", 0)))


# ------------------------------------------------------------------ script analysis
FRAMEWORK = {"characters/states/CharState.gd", "ObjectState.gd", "framework/FiniteStateMachine/StateInterface.gd"}
STOCK_LINE = [
    r"^host\.apply_(fric|forces|grav|forces_no_limit)\(\)$",
    r"^if auto_fall:$", r"^if not host\.is_grounded\(\):$", r'^return "Fall"$',
    r"^if host\.hp <= 0:$", r'^return "Knockdown"$', r"^pass$",
]
_cache = {}


def func_bodies(text):
    bodies, cur = {}, None
    for line in text.split("\n"):
        m = re.match(r"^func (\w+)\(", line)
        if m:
            cur = m.group(1)
            bodies[cur] = []
            continue
        if cur is None:
            continue
        if line.strip() == "":
            continue
        if line[0] not in " \t":
            cur = None
            continue
        s = line.strip()
        if not s.startswith("#"):
            bodies[cur].append(s)
    return bodies


def analyze_script(rel):
    if rel in _cache:
        return _cache[rel]
    text = read(rel)
    bodies = func_bodies(text)
    unknown, calls, returns, physics = [], set(), set(), set()
    for fname, lines in bodies.items():
        for s in lines:
            if any(re.match(p, s) for p in STOCK_LINE):
                m = re.match(r"^host\.apply_(\w+)\(\)$", s)
                if m:
                    physics.add(m.group(1))
                continue
            unknown.append((fname, s))
            for c in re.findall(r"host\.(\w+)\(", s):
                calls.add(c)
            for r in re.findall(r'return "(\w+)"', s):
                returns.add(r)
    info = {
        "path": rel,
        "loc": sum(1 for l in text.split("\n") if l.strip()),
        "unknown": len(unknown),
        "calls": calls, "returns": returns, "physics": physics,
        "funcs": sorted(bodies),
        "frame_hooks": sorted({int(m) for m in re.findall(r"^func _frame_(\d+)", text, re.M)}),
        "spawns": bool(re.search(r"\b(spawn_object|spawn_exported_projectile)\(", text)),
        "exports": set(re.findall(r"^export[^\n]*?\bvar\s+(\w+)", text, re.M)),
        "sets_hitbox": bool(re.search(r"\.(start_tick|active_ticks|damage|knockback|hitstun_ticks|dir_x|dir_y)\s*=(?!=)", text)),
        "sets_length": bool(re.search(r"\banim_length\s*=(?!=)", text)),
        "sets_iasa": bool(re.search(r"\biasa_at\s*=(?!=)", text)),
        "invuln": bool(re.search(r"(start|end)_invulnerability|start_throw_invulnerability", text)),
        "grounded_check": "is_grounded" in text,
        "scenes": re.findall(r'preload\("res://([^"]+\.tscn)"\)', text),
        "extends": (re.search(r'^extends\s+"?([^"\n]+)"?', text, re.M) or [None, None])[1],
    }
    _cache[rel] = info
    return info


def lib_stem(rel):
    return os.path.splitext(os.path.basename(rel))[0]


def analyze_state_scripts(state):
    chain = [p for p in state["class_chain"] if p not in FRAMEWORK]
    own, lib = [], []
    for p in chain:
        if p.startswith("characters/states/") and lib_stem(p) != "Idle":
            lib.append(p)
        else:
            own.append(p)
    infos = [analyze_script(p) for p in own]
    lib_exports = set().union(*[analyze_script(p)["exports"] for p in lib]) if lib else set()
    agg = {
        "own_scripts": own, "lib_scripts": lib,
        "unknown": sum(i["unknown"] for i in infos),
        "calls": sorted(set().union(*[i["calls"] for i in infos])) if infos else [],
        "returns": sorted(set().union(*[i["returns"] for i in infos])) if infos else [],
        "physics": sorted(set().union(*[i["physics"] for i in infos])) if infos else [],
        "loc": sum(i["loc"] for i in infos),
    }
    for flag in ("spawns", "sets_hitbox", "sets_length", "sets_iasa", "invuln", "grounded_check"):
        agg[flag] = any(i[flag] for i in infos)
    agg["exports"] = (set().union(*[i["exports"] for i in infos]) if infos else set()) | lib_exports
    agg["frame_hooks"] = sorted(set().union(*[set(i["frame_hooks"]) for i in infos])) if infos else []
    agg["scenes"] = sorted(set().union(*[set(i["scenes"]) for i in infos])) if infos else []
    return agg


# ------------------------------------------------------------------ property bookkeeping
STATE_CONSUMED = {
    "anim_length", "title", "type", "air_type", "interrupt_from_string", "interrupt_into_string",
    "hit_cancel_into_string", "iasa_at", "interrupt_frames", "enter_force_dir_x", "enter_force_dir_y",
    "enter_force_speed", "force_dir_x", "force_dir_y", "force_speed", "force_tick", "apply_fric",
    "apply_forces", "apply_grav", "reset_momentum", "endless", "combo_only", "neutral_only",
    "super_level_", "uses_air_movement", "projectile_scene", "projectile_tick", "projectile_pos_x",
    "projectile_pos_y", "fallback_state", "sprite_animation", "show_in_menu", "selectable",
    "data_ui_scene", "auto_fall", "allowed_stances_string", "change_stance_to",
    # per-move physics overrides
    "apply_custom_grav", "custom_grav", "custom_grav_max_fall_speed", "apply_custom_limits",
    "custom_max_air_speed", "custom_max_ground_speed", "apply_custom_x_fric", "apply_custom_y_fric",
    "custom_x_fric", "custom_y_fric", "apply_forces_no_limit", "reset_x_momentum", "reset_y_momentum",
    "ticks_per_frame",
}
# Exports of ThrowState that the throw block (hold, keyframes, release hit) accounts for.
THROW_CONSUMED = {
    "release", "release_frame", "use_start_throw_pos", "start_throw_pos_x", "start_throw_pos_y",
    "use_release_throw_pos", "release_throw_pos_x", "release_throw_pos_y", "throw_positions", "reverse",
    "hitstun_ticks", "knockback", "dir_x", "dir_y", "knockdown", "knockdown_extends_hitstun",
    "aerial_hit_state", "grounded_hit_state", "damage", "damage_in_combo", "ground_bounce",
    "increment_combo", "hard_knockdown", "air_ground_bounce", "wall_slam", "di_modifier",
    "minimum_grounded_frames", "damage_proration", "hit_height",
}
STATE_PRESENTATION = {
    "button_texture", "flip_icon", "particle_scene", "spawn_particle_on_enter", "particle_position",
    "timed_particle_scene", "timed_particle_position", "timed_spawn_particle_tick", "state_screenshake_tick",
    "state_screenshake_dir", "state_screenshake_length", "state_screenshake_amount", "enter_sfx",
    "enter_sfx_volume", "sfx", "sfx_tick", "sfx_volume", "sfx_pitch_variation", "override_same_sfx",
    "override_same_wav", "loop_animation", "animation_loop_start", "absolute_loop", "refresh_loop",
    "ticks_per_frame", "sprite_anim_length", "flip_with_facing", "projectile_match_facing",
    "projectile_local_pos",
}
# Classes for scene properties we do not convert:
#   turn      - a turn-based concept with no real-time meaning (dropped by design)
#   engine    - a game rule that must be re-decided in the new engine
#   gap       - gameplay data the format cannot express yet
#   script    - a parameter that belongs with the move's behavior code
#   unknown   - not recognised by this converter
STATE_CLASS = {
    **{k: "turn" for k in (
        "interruptible_on_opponent_turn", "initiative_effect", "next_state_on_hold",
        "next_state_on_hold_on_opponent_turn", "end_feint", "can_feint_if_possible",
        "usable_from_whiff_cancel_if_possible", "instant_cancellable", "hold_restart",
        "allow_framecheat", "dynamic_iasa", "backdash_iasa", "busy_interrupt_type")},
    **{k: "engine" for k in (
        "reversible", "self_hit_cancellable", "self_interruptable", "burst_cancellable", "throw_techable",
        "whiff_meter_gain_multiplier", "super_freeze_ticks", "super_freeze_ticks_", "extra_parry_hitlag",
        "release_opponent_on_startup", "release_opponent_on_exit", "can_be_counterhit", "beats_backdash",
        "velocity_forward_meter_gain_multiplier", "tick_priority", "burstable", "super_effect_",
        "scale_combo_meter_", "force_super_effect", "supers_used_", "land_cancel", "landing_recovery",
        "min_land_cancel_frame", "land_cancel_state", "uses_air_movement_ignore", "count_projectile_hits")},
    **{k: "engine" for k in ("earliest_hitbox", "ignore_air_option_use_in_combos", "update_facing_on_exit")},
    **{k: "gap" for k in (
        "apply_custom_limits", "custom_max_air_speed", "custom_max_ground_speed", "apply_custom_x_fric",
        "apply_custom_y_fric", "apply_custom_grav", "custom_x_fric", "custom_y_fric", "custom_grav",
        "custom_grav_max_fall_speed", "apply_forces_no_limit", "reset_x_momentum", "reset_y_momentum",
        "iasa_on_hit_on_block", "iasa_on_hit",
        "interrupt_exceptions_string", "hit_cancel_exceptions_string", "release", "release_frame",
        "release_throw_pos_x", "release_throw_pos_y", "start_throw_pos_x", "start_throw_pos_y",
        "throw_positions", "no_collision_start_frame", "no_collision_end_frame", "flip_frame",
        "force_same_direction_as_previous_state", "host_commands", "fixed")},
    **{k: "presentation" for k in (
        "release_sfx", "release_sfx_volume", "screenshake_amount", "screenshake_frames", "hit_sound",
        "particle_location", "use_guard_sprites")},
}
PROP_NOTE = {
    "interruptible_on_opponent_turn": "lets a move be left on the opponent's turn; no meaning without turns",
    "initiative_effect": "turn-based initiative marker",
    "next_state_on_hold": "hold-the-turn behavior of the menu",
    "end_feint": "feint (free-cancel) mechanic of the turn UI",
    "can_feint_if_possible": "feint (free-cancel) mechanic of the turn UI",
    "instant_cancellable": "instant-cancel mechanic of the turn UI",
    "usable_from_whiff_cancel_if_possible": "whiff-cancel mechanic of the turn UI",
    "reversible": "may the player turn the character around when choosing it",
    "self_hit_cancellable": "can the move cancel into itself after a hit",
    "self_interruptable": "can the move be re-selected into itself",
    "burst_cancellable": "combo-breaker burst allowed during the move",
    "throw_techable": "grabs can be teched when both fighters grab",
    "super_freeze_ticks": "length of the super-move freeze",
    "extra_parry_hitlag": "extra freeze when the move is perfectly parried",
    "whiff_meter_gain_multiplier": "meter gained on whiffing",
    "interrupt_exceptions_string": "categories excluded from the move's cancel targets",
    "release": "throw choreography: when the held opponent is released",
    "release_frame": "throw choreography: release frame",
    "release_throw_pos_x": "throw choreography: opponent position at release",
    "release_throw_pos_y": "throw choreography: opponent position at release",
    "start_throw_pos_x": "throw choreography: opponent position at grab",
    "start_throw_pos_y": "throw choreography: opponent position at grab",
    "throw_positions": "throw choreography: per-frame opponent positions",
    "grounded_hit_state": "hit reaction state override (grounded victim)",
    "aerial_hit_state": "hit reaction state override (airborne victim)",
    "cancellable": "whether hitting allows cancelling",
    "block_cancel_allowed": "whether blocking allows cancelling",
    "increment_combo": "whether the hit counts as a combo hit",
    "knockdown_extends_hitstun": "knockdown hits keep the victim stunned until they land",
    "minimum_damage": "damage floor after combo scaling",
    "minimum_grounded_frames": "minimum frames a launched victim stays down",
    "air_ground_bounce": "airborne victims bounce off the ground",
    "launch_reversible": "launch direction flips with relative position",
    "ignore_armor": "hit ignores armor",
    "hits_vs_dizzy": "can hit dizzy victims",
    "can_counter_hit": "can produce a counter hit",
    "priority": "hitbox priority when several overlap",
    "group": "hitbox group (stops one attack hitting twice)",
    "di_modifier": "directional-influence strength on this hit",
    "sdi_modifier": "smash-DI strength on this hit",
    "combo_victim_hitlag": "victim hitlag inside combos",
    "parriable": "whether the hit can be parried",
    "chip_damage_modifier": "chip damage on block",
    "meter_gain_modifier": "meter gained on hit",
    "block_pushback_modifier": "pushback on block",
    "vacuum": "pulls the victim toward the hitbox",
    "send_away_from_center": "knockback points away from the hitbox centre",
    "force_grounded": "keeps the victim grounded",
    "hitbox_type": "special hit behavior (throw damage, burst, flip, no-hitstun, detect)",
    "apply_custom_limits": "per-move speed caps",
    "custom_max_air_speed": "per-move speed caps",
    "apply_custom_grav": "per-move gravity override",
    "reset_y_momentum": "stops only vertical speed on entry",
    "reset_x_momentum": "stops only horizontal speed on entry",
    "iasa_on_hit_on_block": "hit-cancel timing when blocked",
    "earliest_hitbox": "engine hint for when the first hitbox appears",
    "ignore_air_option_use_in_combos": "air actions are free during combos",
    "update_facing_on_exit": "re-face the opponent when the move ends",
}
HIT_CONSUMED = {
    "x", "y", "width", "height", "start_tick", "active_ticks", "damage", "damage_in_combo", "cancellable",
    "hitstun_ticks", "combo_hitstun_ticks", "hitlag_ticks", "victim_hitlag", "knockback", "dir_x",
    "dir_y", "pushback_x", "hit_height", "plus_frames", "damage_proration", "scale_combo", "knockdown",
    "hard_knockdown", "ground_bounce", "wall_slam", "guard_break", "can_clash", "beats_grab",
    "hits_projectiles", "hits_vs_grounded", "hits_vs_aerial", "hits_otg", "looping", "loop_active_ticks",
    "loop_inactive_ticks", "always_on", "followup_state", "throw_state", "hitbox_type",
    "grounded_hit_state", "aerial_hit_state", "knockdown_extends_hitstun", "minimum_grounded_frames",
    "air_ground_bounce", "launch_reversible", "vacuum", "send_away_from_center", "increment_combo",
    "minimum_damage", "block_cancel_allowed",
}
HIT_PRESENTATION = {
    "hit_sound", "whiff_sound", "hit_bass_sound", "hit_particle", "whiff_particle", "replace_hit_particle",
    "screenshake_amount", "screenshake_frames", "camera_bump_dir", "rumble", "bump_on_whiff",
    "whiff_sound_volume", "hit_sound_volume", "bass_sound_volume", "bass_on_whiff", "override_same_sfx",
    "override_same_wav", "sfx_pitch_variation", "_c_Grouping", "_c_Throw_Data",
}


def clean_keys(d):
    return {k for k in d if not k.startswith("_c_") and k not in ("script", "__meta__", "z_index")}


# ------------------------------------------------------------------ literal and scene helpers
def parse_value(raw):
    """Godot text literal -> python (numbers, bools, strings, Vector2)."""
    raw = re.sub(r"^-\s+(?=\d)", "-", raw.strip())
    if raw in ("true", "false"):
        return raw == "true"
    if re.fullmatch(r"-?\d+", raw):
        return int(raw)
    if re.fullmatch(r"-?\d+\.\d+(e[-+]?\d+)?", raw, re.I):
        return float(raw)
    m = re.fullmatch(r'"(.*)"', raw, re.S)
    if m:
        return m.group(1)
    m = re.fullmatch(r"Vector2\(\s*([-\d.e+]+)\s*,\s*([-\d.e+]+)\s*\)", raw)
    if m:
        return (float(m.group(1)), float(m.group(2)))
    return raw


_defaults_cache = {}


def script_defaults(rel):
    """Export defaults declared in a .gd file (no inheritance)."""
    if rel not in _defaults_cache:
        out = {}
        for m in re.finditer(r"^export[^\n]*?\bvar\s+(\w+)\s*(?::\s*\w+)?\s*=\s*(.+)$", read(rel), re.M):
            out[m.group(1)] = parse_value(m.group(2))
        _defaults_cache[rel] = out
    return _defaults_cache[rel]


def ext_map(text):
    return {int(i): p for p, i in re.findall(r'\[ext_resource path="res://([^"]+)" type="[^"]*" id=(\d+)\]', text)}


def animations_in(text):
    """animation name -> list of texture paths, from SpriteFrames written inside a scene."""
    ext = ext_map(text)
    anims = {}
    for m in re.finditer(r'"frames": \[(.*?)\],\n"loop": (?:true|false),\n"name": "([^"]+)"', text, re.S):
        ids = [int(x) for x in re.findall(r"ExtResource\(\s*(\d+)\s*\)", m.group(1))]
        anims[m.group(2)] = [ext.get(i) for i in ids]
    return anims


def widget_root(rel, depth=0):
    """Root-node properties of a UI widget scene, following scene inheritance."""
    text = read(rel)
    if not text or depth > 4:
        return {}
    ext = ext_map(text)
    lines = text.split("\n")
    start = next((i for i, l in enumerate(lines) if l.startswith("[node ")), None)
    if start is None:
        return {}
    base = {}
    im = re.search(r"instance=ExtResource\(\s*(\d+)\s*\)", lines[start])
    if im and int(im.group(1)) in ext:
        base = widget_root(ext[int(im.group(1))], depth + 1)
    props = {}
    for l in lines[start + 1:]:
        if l.startswith("["):
            break
        mm = re.match(r"^(\w+) = (.+)$", l)
        if mm:
            props[mm.group(1)] = parse_value(mm.group(2))
    return {**base, **props}


XY_DEFAULTS = {"limit_angle": False, "limit_range_degrees": 90.0, "limit_center_degrees": 0.0,
               "limit_symmetrical": False, "default_value": (0.0, 0.0), "min_length": 0.0}
COMPASS = [("Neutral", "Neutral", (0.0, 0.0)), ("E", "Forward", (1.0, 0.0)), ("NE", "ForwardUp", (1.0, -1.0)),
           ("N", "Up", (0.0, -1.0)), ("NW", "BackUp", (-1.0, -1.0)), ("W", "Back", (-1.0, 0.0)),
           ("SW", "BackDown", (-1.0, 1.0)), ("S", "Down", (0.0, 1.0)), ("SE", "ForwardDown", (1.0, 1.0))]
REACTION = {"HurtGrounded": "Stumble", "HurtAerial": "Launch", "Grabbed": "Held", "HurtDizzy": "Dizzy"}
HITKIND = {"ThrowHit": "ThrowDamage", "Burst": "Burst", "OffensiveBurst": "OffensiveBurst",
           "NoHitstun": "NoHitstun", "Flip": "Flip", "Detect": "Detect"}


# ------------------------------------------------------------------ conversion
data = json.load(open(SRC, encoding="utf-8"))
ALL = {s["name"]: s for s in data["states"]}
OWN = [s for s in data["states"] if s["origin"] != "base"]
LIB_NAMES = {s["name"] for s in data["states"] if s["origin"] == "base"}
if ONLY:
    OWN = [ALL[ONLY]]

# who enters whom (for deciding which moves have no player input)
entered_by = defaultdict(list)
for s in OWN:
    for h in s["hitboxes"]:
        hk = h["key"]
        if hk.get("followup_state"):
            entered_by[hk["followup_state"]].append(s["name"])
        if h["kind"] == "throwbox" and h["explicit"].get("throw_state"):
            entered_by[h["explicit"]["throw_state"]].append(s["name"])

root = data["character"]
rp, rd = root["root_properties"], root["root_class_defaults"]


def rootv(key, default=None):
    v = rp.get(key)
    if v is None:
        v = rd.get(key)
    return default if v is None else v


def tags_of(groups):
    return [snake(g) for g in groups or []]


def guess_input(s):
    """Placeholder real-time input binding. Returns (S|None, note)."""
    k, name = s["key"], s["name"]
    if name in entered_by:
        return None, f"entered by {', '.join(sorted(set(entered_by[name])))}"
    if not k.get("show_in_menu", True) or not k.get("selectable", True):
        return None, "not selectable in the original menu"
    typ = k.get("type")
    if re.fullmatch(r"(Double|Super)?Jump", name):
        button = "Jump"
    elif "Dash" in name:
        button = "Dash"
    elif name.endswith("Grab"):
        button = "Grab"
    elif name == "Taunt":
        button = "Taunt"
    elif typ == "Super":
        button = "Super"
    elif typ == "Defense":
        button = "Guard"
    elif typ == "Special":
        button = "Special"
    elif typ == "Attack":
        button = "Light"
    else:
        return None, "no menu type (engine-entered state)"
    direction = "Any"
    if re.search(r"Upward|Uppercut|Up", name):
        direction = "Up"
    elif re.search(r"Down|Drop|Sweep|Stomp|Dive", name):
        direction = "Down"
    elif re.search(r"Forward", name):
        direction = "Forward"
    elif re.search(r"Back", name):
        direction = "Back"
    f = {"button": S(button)}
    if direction != "Any":
        f["direction"] = S(direction)
    return S("Input", **f), None


def build_params(s):
    """Player parameters of a move -> (list of Param, dropped widget names, notes)."""
    out, dropped, notes = [], [], []
    names = set()
    for p in s.get("player_parameters") or []:
        inst = p.get("instance") or ""
        stem = os.path.basename(inst)
        props = p["props"]
        nm = snake(p["name"])
        if not re.match(r"[a-z]", nm):
            nm = "p_" + nm
        base, j = nm, 2
        while nm in names:
            nm, j = f"{base}_{j}", j + 1
        names.add(nm)
        kind = None
        if "Plot" in stem:
            merged = {**XY_DEFAULTS, **widget_root(inst), **props}
            dv = merged["default_value"]
            if isinstance(dv, dict) and "Vector2" in dv:
                dv = tuple(float(x) for x in dv["Vector2"])
            arc = None
            if merged["limit_angle"]:
                arc = S("Arc", center_deg=float(merged["limit_center_degrees"]), range_deg=float(merged["limit_range_degrees"]))
                if merged["limit_symmetrical"]:
                    notes.append(f"{p['name']}: arc is mirrored in the original; only one side is expressed")
            f = {"steps": S("Free")}
            if arc:
                f["arc"] = arc
            f["default"] = tuple(dv)
            kind = S("Direction", **f)
        elif "8Way" in stem:
            disabled = {k for k, v in props.items() if v is False}
            allowed = [name for key, name, _ in COMPASS if key not in disabled] if disabled else []
            vec = {name: v for _, name, v in COMPASS}
            order = allowed or [n for _, n, _ in COMPASS]
            pick = "Forward" if "Forward" in order else order[0]
            f = {"steps": S("Eight")}
            if allowed:
                f["allowed"] = [S(a) for a in allowed]
            f["default"] = vec[pick]
            kind = S("Direction", **f)
        elif "HorizSlider" in stem:
            centered = props.get("centered", True)
            lo = float(props["min_value"]) if "min_value" in props else (-100.0 if centered else 0.0)
            hi = float(props.get("max_value", 100.0))
            kind = S("Slider", min=lo, max=hi, default=min(max(0.0, lo), hi))
        elif "CountOption" in stem:
            lo, hi = int(props.get("min_value", 0)), int(props.get("max_value", 10))
            kind = S("Count", min=lo, max=hi, default=lo)
        elif p.get("type") == "CheckButton":
            kind = S("Toggle", default=bool(props.get("pressed", False)))
        elif p.get("type") == "OptionButton":
            raw = props.get("items", {}).get("raw", "") if isinstance(props.get("items"), dict) else ""
            options = re.findall(r'"([^"]*)"', raw)
            if options:
                sel = int(props.get("selected", 0))
                kind = S("Choice", options=options, default=options[sel] if sel < len(options) else options[0])
        if kind is None:
            dropped.append(f"{p['name']} ({stem or p.get('type')})")
            continue
        fields = {"name": nm, "kind": kind}
        out.append(S("Param", **fields))
    # the first direction is the one the stick controls
    for prm in out:
        if prm.fields["kind"].name == "Direction":
            prm.fields["binding"] = S("Stick")
            break
    return out, dropped, notes


report = {"states": [], "input_conflicts": [], "projectiles": [], "base_states_not_converted": sorted(LIB_NAMES)}
MODE_USE = set()
CONV_IDS = {sid(s["name"]) for s in OWN}
LIB_IDS = {sid(n) for n in LIB_NAMES}
SCENE_TEXT = read(data["character"]["scene"])
SCENE_EXT = ext_map(SCENE_TEXT)
ANIMS = {**animations_in(read("characters/BaseChar.tscn")), **animations_in(SCENE_TEXT)}
THROW_SCRIPT = "characters/states/ThrowState.gd"


def parse_throw_positions(raw):
    """throw_positions dict text -> {texture path: (x, y)}."""
    out = {}
    for m in re.finditer(r'(?:ExtResource\(\s*(\d+)\s*\)|Resource\(\s*"res://([^"]+)"\s*\))\s*:\s*\{\s*"x":\s*(-?\d+),\s*"y":\s*(-?\d+)\s*\}', raw):
        tex = SCENE_EXT.get(int(m.group(1))) if m.group(1) else m.group(2)
        if tex:
            out[tex] = (int(m.group(3)), int(m.group(4)))
    return out


def reaction_fields(grounded_state, aerial_state, min_frames):
    rx = {}
    for label, st, default in (("grounded", grounded_state, "Stumble"), ("aerial", aerial_state, "Launch")):
        val = REACTION.get(st)
        if val and val != default:
            rx[label] = S(val)
    if min_frames not in (None, -1):
        rx["min_launch_frames"] = min_frames
    return rx


def build_throw(s, k, ex, anim_len, losses, notes):
    """Hold choreography and release launch of a ThrowState-derived move."""
    if THROW_SCRIPT not in s["class_chain"]:
        return None
    D = script_defaults(THROW_SCRIPT)

    def tv(key):
        return ex.get(key, D.get(key))

    use_start = bool(tv("use_start_throw_pos"))
    start = (int(tv("start_throw_pos_x")), int(tv("start_throw_pos_y")))
    release_on = bool(tv("release")) and int(tv("release_frame")) > 0
    rframe = int(tv("release_frame"))
    tp_raw = ex.get("throw_positions")
    tp = parse_throw_positions(tp_raw.get("raw", "")) if isinstance(tp_raw, dict) else {}
    anim = k.get("sprite_animation") or s["name"]
    frames = ANIMS.get(anim)
    tpf = max(1, int(ex.get("ticks_per_frame", 1)))
    last_tick = rframe - 1 if release_on else anim_len - 1
    if tp and not frames:
        losses.append(f"throw positions are keyed by animation frames but animation {anim!r} was not found")
    pos, first, prev, keys = (start if use_start else (0, 0)), None, None, []
    for t in range(0, max(last_tick, 0) + 1):
        if frames:
            tex = frames[min(t // tpf, len(frames) - 1)]
            if tex in tp:
                pos = tp[tex]
        if first is None:
            first = pos
        elif pos != prev:
            keys.append(S("HoldKey", at=t + 1, offset=pos))
        prev = pos
    f = {"hold": first or start}
    if keys:
        f["keyframes"] = keys
    if release_on:
        off = (int(tv("release_throw_pos_x")), int(tv("release_throw_pos_y"))) if tv("use_release_throw_pos") else prev
        hit = {"damage": int(tv("damage"))}
        if int(tv("damage_in_combo")) != -1:
            hit["damage_in_combo"] = int(tv("damage_in_combo"))
        hit["hitstun"] = int(tv("hitstun_ticks"))
        hit["knockback"] = S("Knockback", dir=(num(tv("dir_x")), num(tv("dir_y"))), strength=num(tv("knockback")))
        sc = {}
        if tv("damage_proration"):
            sc["proration"] = min(int(tv("damage_proration")), 15)  # the game stores min(value, 15)
        if not tv("increment_combo"):
            sc["counts_as_hit"] = False
        if sc:
            hit["scaling"] = S("Scaling", **sc)
        efx = {}
        if tv("knockdown"):
            efx["knockdown"] = S("Hard" if tv("hard_knockdown") else "Soft")
            if not tv("knockdown_extends_hitstun"):
                efx["stun_until_landing"] = False
        for key in ("ground_bounce", "air_ground_bounce", "wall_slam"):
            if tv(key):
                efx[key] = True
        if efx:
            hit["effects"] = S("Effects", **efx)
        rx = reaction_fields(tv("grounded_hit_state"), tv("aerial_hit_state"), tv("minimum_grounded_frames"))
        if rx:
            hit["reaction"] = S("Reaction", **rx)
        if abs(num(tv("di_modifier"), 1.0) - 1.0) > 1e-9:
            hit["di_modifier"] = num(tv("di_modifier"))
        height = str(tv("hit_height")).split(".")[-1]
        if height in ("High", "Low"):
            hit["height"] = S(height)
        f["release"] = S("Release", at=rframe, offset=off, hit=S("ReleaseHit", **hit))
    if tv("reverse"):
        f["reverse"] = True
    if not frames and not tp:
        notes.append("no animation-driven hold positions")
    return S("Throw", **f)


CURRENT_HIT_UNMAPPED = set()
CURRENT_CLAMPED = []
PROP_HITS = defaultdict(set)   # (level, property, class) -> states that set it


def convert_hitbox(h, kind_ctx, move_total, open_ended, losses, idx, n_boxes, id_set):
    hk, ex = h["key"], h["explicit"]
    is_throw = h["kind"] == "throwbox"
    start, act = hk.get("start_tick", 0), hk.get("active_ticks", 0)
    wf = max(1, start + 1)
    wt = max(wf, start + act)
    if start < 0:
        CURRENT_CLAMPED.append(h["name"])
    f = {}
    base_id = snake(h["name"]) if n_boxes > 1 else "main"
    hid = base_id
    j = 2
    while hid in id_set:
        hid = f"{base_id}_{j}"
        j += 1
    id_set.add(hid)
    f["id"] = hid
    # Hitbox.init() in the original takes the absolute value of negative sizes.
    f["rect"] = S("Rect", x=hk["x"], y=hk["y"], half_w=abs(hk["width"]), half_h=abs(hk["height"]))
    f["window"] = S("Span", **{"from": wf, "to": wt})
    until_end = bool(hk.get("always_on"))
    if until_end:
        f["until_end"] = True
        if not open_ended and move_total:
            f["window"] = S("Span", **{"from": wf, "to": move_total})
    if hk.get("looping"):
        f["repeat"] = S("Repeat", active=hk.get("loop_active_ticks", 2), inactive=hk.get("loop_inactive_ticks", 2))
    fu = hk.get("followup_state")
    if fu:
        f["followup"] = sid(fu)
        if sid(fu) not in CONV_IDS:
            losses.append(f"hitbox {h['name']}: follow-up state {fu} was not converted")
    if is_throw and ex.get("throw_state"):
        if sid(ex["throw_state"]) in CONV_IDS:
            f["throw_into"] = sid(ex["throw_state"])
        else:
            losses.append(f"throw into {ex['throw_state']} is an engine-owned state; no choreography emitted")
    kind = HITKIND.get(hk.get("hitbox_type"))
    if kind and not is_throw:
        f["kind"] = S(kind)
    if hk["damage"] < 0:
        losses.append(f"hitbox {h['name']}: damage {hk['damage']} is a placeholder the script overwrites; emitted as 0")
    f["damage"] = max(hk["damage"], 0)
    if hk.get("damage_in_combo", -1) != -1:
        f["damage_in_combo"] = hk["damage_in_combo"]
    if hk.get("minimum_damage", 0) > 0:
        f["min_damage"] = hk["minimum_damage"]
    f["hitstun"] = hk["hitstun_ticks"]
    if hk.get("combo_hitstun_ticks", -1) != -1:
        f["hitstun_in_combo"] = hk["combo_hitstun_ticks"]
    victim = hk["victim_hitlag"] if hk.get("victim_hitlag", -1) != -1 else hk["hitlag_ticks"]
    f["hitlag"] = S("Hitlag", attacker=hk["hitlag_ticks"], victim=victim)
    kb = {"dir": (num(hk["dir_x"]), num(hk["dir_y"])), "strength": num(hk["knockback"])}
    if hk.get("launch_reversible"):
        kb["reversible"] = True
    if hk.get("vacuum"):
        kb["from"] = S("TowardHitbox")
    elif hk.get("send_away_from_center"):
        kb["from"] = S("AwayFromHitbox")
    f["knockback"] = S("Knockback", **kb)
    if num(hk.get("pushback_x")) != 0:
        f["pushback"] = num(hk["pushback_x"])
    f["height"] = S(hk["hit_height"])
    if hk.get("plus_frames", 0) != 0:
        f["block_advantage"] = hk["plus_frames"]
    sc = {}
    if hk.get("damage_proration", 0):
        sc["proration"] = min(hk["damage_proration"], 15)  # the game stores min(value, 15)
    if not hk.get("scale_combo", True):
        sc["scale_combo"] = False
    if not hk.get("increment_combo", True):
        sc["counts_as_hit"] = False
    if sc:
        f["scaling"] = S("Scaling", **sc)
    efx = {}
    if hk.get("knockdown"):
        efx["knockdown"] = S("Hard" if hk.get("hard_knockdown") else "Soft")
        if not hk.get("knockdown_extends_hitstun", True):
            efx["stun_until_landing"] = False
    for key in ("ground_bounce", "air_ground_bounce", "wall_slam", "guard_break"):
        if hk.get(key):
            efx[key] = True
    if is_throw:
        efx["throw"] = True
    if efx:
        f["effects"] = S("Effects", **efx)
    rx = {}
    g, a = REACTION.get(hk.get("grounded_hit_state")), REACTION.get(hk.get("aerial_hit_state"))
    for label, val in (("grounded", g), ("aerial", a)):
        default = "Stumble" if label == "grounded" else "Launch"
        if val and val != default:
            rx[label] = S(val)
    for label, st in (("grounded", hk.get("grounded_hit_state")), ("aerial", hk.get("aerial_hit_state"))):
        if st and st not in REACTION:
            CURRENT_HIT_UNMAPPED.add(f"{label}_hit_state={st}")
    if hk.get("minimum_grounded_frames", -1) != -1:
        rx["min_launch_frames"] = hk["minimum_grounded_frames"]
    if rx:
        f["reaction"] = S("Reaction", **rx)
    fl = {}
    if not hk.get("can_clash", True): fl["can_clash"] = False
    if not hk.get("beats_grab", True): fl["beats_throw"] = False
    if not hk.get("hits_projectiles", True): fl["hits_projectiles"] = False
    if not hk.get("hits_vs_grounded", True): fl["hits_grounded"] = False
    if not hk.get("hits_vs_aerial", True): fl["hits_air"] = False
    if hk.get("hits_otg"): fl["hits_downed"] = True
    if fl:
        f["flags"] = S("HitFlags", **fl)
    un = set(clean_keys(ex) - HIT_CONSUMED - HIT_PRESENTATION)
    if hk.get("hitbox_type") not in (None, "Normal") and hk.get("hitbox_type") not in HITKIND:
        un.add("hitbox_type")      # a kind the format does not know
    CURRENT_HIT_UNMAPPED.update(un)
    return S("HitBox", **f)


def convert_state(s):
    k, ex = s["key"], s["explicit_overrides"]
    losses, notes, params_dropped = [], [], []
    sa = analyze_state_scripts(s)
    sid_ = sid(s["name"])
    real = [h for h in s["hitboxes"] if h["kind"] == "hitbox"]
    all_boxes = s["hitboxes"]
    endless = bool(k.get("endless"))
    anim_len = k["anim_length"] if isinstance(k.get("anim_length"), int) else 1

    # --- timeline (computed from clamped windows)
    wins = []
    for h in all_boxes:
        hk = h["key"]
        wf = max(1, hk.get("start_tick", 0) + 1)
        wt = max(wf, hk.get("start_tick", 0) + hk.get("active_ticks", 0))
        wins.append((wf, wt))
    if wins:
        startup = min(w[0] for w in wins) - 1
        active = max(w[1] for w in wins) - startup
    else:
        startup, active = 0, 0
    end_kind = "Fixed"
    if endless:
        recovery = 0
        end_kind = "OnLanding" if (k.get("air_type") in ("Aerial", "Both") and sa["grounded_check"]) else "Script"
    else:
        recovery = max(0, anim_len - startup - active)
        if startup + active > anim_len:
            losses.append(f"hitboxes extend past anim_length ({anim_len}); recovery set to 0")
    tl = {"startup": startup, "active": active, "recovery": recovery}
    if end_kind != "Fixed":
        tl["end"] = S(end_kind)
    listed = startup + active + recovery
    total_for_cancel = listed if not endless else None

    # --- hitboxes
    id_set = set()
    CURRENT_HIT_UNMAPPED.clear()
    CURRENT_CLAMPED.clear()
    boxes = [convert_hitbox(h, s["name"], listed, endless, losses, i, len(all_boxes), id_set)
             for i, h in enumerate(all_boxes)]
    if CURRENT_CLAMPED:
        losses.append(f"{len(CURRENT_CLAMPED)} hitbox(es) have a start_tick set by script (scene value is negative); "
                      "windows clamped to frame 1")
    if CURRENT_HIT_UNMAPPED:
        losses.append("hitbox properties not expressed: " + ", ".join(sorted(CURRENT_HIT_UNMAPPED)))
        for key in CURRENT_HIT_UNMAPPED:
            PROP_HITS[("hitbox", key, "gap")].add(s["name"])

    # --- hurt windows
    hurt = []
    for hs in s["hurtbox_states"]:
        p = hs["props"]
        st = p.get("start_tick", 1)
        at = p.get("active_ticks", 1)
        frm = st + 1
        to = (listed if listed else frm) if p.get("endless") else st + at
        to = max(to, frm)
        hurt.append(S("HurtWindow", window=S("Span", **{"from": frm, "to": to}),
                      rect=S("Rect", x=int(p.get("x", 0)), y=int(p.get("y", 0)),
                             half_w=abs(int(p.get("width", 20))), half_h=abs(int(p.get("height", 20))))))  # CollisionBox.gd defaults; abs as for hitboxes

    # --- physics
    flags = {"friction": bool(k.get("apply_fric")) or "fric" in sa["physics"],
             "gravity": bool(k.get("apply_grav")) or "grav" in sa["physics"],
             "forces": bool(k.get("apply_forces")) or bool({"forces", "forces_no_limit"} & set(sa["physics"]))}
    OD = script_defaults("ObjectState.gd")

    def ov(key):
        return ex.get(key, OD.get(key))

    ph = {n: True for n, on in flags.items() if on}
    if ex.get("apply_forces_no_limit"):
        ph["uncapped_forces"] = True
    if ex.get("apply_custom_grav"):
        ph["gravity_override"] = S("GravityOverride", gravity=num(ov("custom_grav")),
                                   fall_speed=num(ov("custom_grav_max_fall_speed")))
    if ex.get("apply_custom_limits"):
        ph["speed_caps"] = S("SpeedCaps", ground=num(ov("custom_max_ground_speed")), air=num(ov("custom_max_air_speed")))
    if ex.get("apply_custom_x_fric"):
        ph["friction_x"] = num(ov("custom_x_fric"))
    if ex.get("apply_custom_y_fric"):
        ph["friction_y"] = num(ov("custom_y_fric"))
    physics = S("Physics", **ph) if ph else None

    # --- motion
    motion = []
    if ex.get("reset_momentum"):
        motion.append(S("MotionEvent", at=1, kind=S("StopMomentum")))
    else:
        if ex.get("reset_x_momentum"):
            motion.append(S("MotionEvent", at=1, kind=S("StopHorizontal")))
        if ex.get("reset_y_momentum"):
            motion.append(S("MotionEvent", at=1, kind=S("StopVertical")))
    spd = num(k.get("enter_force_speed"))
    if spd:
        dx, dy = num(ex.get("enter_force_dir_x")), num(ex.get("enter_force_dir_y"))
        ln = (dx * dx + dy * dy) ** 0.5
        if ln:
            motion.append(S("MotionEvent", at=1, kind=S("Impulse", forward=round(dx / ln * spd, 4), up=round(-dy / ln * spd, 4))))
        else:
            losses.append("enter force has speed but no direction")
    fspd = num(k.get("force_speed"))
    if fspd:
        dx, dy = num(ex.get("force_dir_x")), num(ex.get("force_dir_y"))
        ln = (dx * dx + dy * dy) ** 0.5
        if ln:
            motion.append(S("MotionEvent", at=int(k.get("force_tick", 0)) + 1,
                            kind=S("Impulse", forward=round(dx / ln * fspd, 4), up=round(-dy / ln * fspd, 4))))
        else:
            losses.append("timed force has speed but no direction")

    # --- projectile spawn from the scene
    spawns = []
    script_spawned = []
    ps = k.get("projectile_scene")
    if ps:
        pid = snake(os.path.splitext(os.path.basename(ps))[0])
        if int(k.get("projectile_tick", 1)) < 0:
            # -1 means the scene does not time the spawn; the move's script fires it
            script_spawned.append(pid)
            losses.append(f"projectile {pid} is spawned by script at a time the data does not hold")
        else:
            spawns.append((pid, S("Spawn", at=int(k.get("projectile_tick", 1)) + 1, projectile=pid,
                                  offset=(int(k.get("projectile_pos_x", 0)), int(k.get("projectile_pos_y", 0))))))

    # --- cancels
    cancels = []
    into_tags = tags_of(k.get("interrupt_into"))
    if not endless and into_tags:
        iasa = k.get("iasa_at", -1)
        iasa_tick = anim_len + iasa if isinstance(iasa, int) and iasa < 0 else iasa
        if isinstance(iasa_tick, int) and iasa_tick + 1 < listed:
            cancels.append(S("Cancel", into=into_tags, when=S("Always"), window=S("Span", **{"from": iasa_tick + 1, "to": listed})))
        for t in k.get("interrupt_frames") or []:
            if isinstance(t, int) and t + 1 <= listed:
                cancels.append(S("Cancel", into=into_tags, when=S("Always"), window=S("Span", **{"from": t + 1, "to": t + 1})))
    hit_into = tags_of(k.get("hit_cancel_into"))
    if endless and (hit_into or into_tags) and real:
        losses.append("cancel windows end at the last listed frame; the format cannot say 'until the move ends' for cancels")
    cancellable = any(h["key"].get("cancellable", True) for h in real)
    when_hit = "OnHitOrBlock" if any(h["key"].get("block_cancel_allowed", True) for h in real) else "OnHit"
    if hit_into and real and cancellable:
        first_active = startup + 1
        last = listed if listed else first_active
        cancels.append(S("Cancel", into=hit_into, when=S(when_hit), window=S("Span", **{"from": first_active, "to": last})))
        cancels.append(S("Cancel",
                         into=["offensive_burst", "instant_cancel", "grounded_defense", "aerial_defense", "grounded_grab", "aerial_grab"],
                         when=S(when_hit), window=S("Span", **{"from": first_active, "to": last})))

    # --- input, availability, cost
    inp, why_no_input = guess_input(s)
    avail = {}
    if k.get("combo_only"): avail["combo_only"] = True
    if k.get("neutral_only"): avail["neutral_only"] = True
    cost = {}
    if k.get("super_level_"): cost["super_bars"] = k["super_level_"]
    if k.get("uses_air_movement"): cost["air_action"] = True

    # --- player parameters and throw choreography
    params, params_dropped, pnotes = build_params(s)
    notes.extend(pnotes)
    if params_dropped:
        losses.append("parameter widgets with no mapping: " + ", ".join(params_dropped))
    unbound = [p.fields["name"] for p in params if "binding" not in p.fields]
    throw = build_throw(s, k, ex, anim_len, losses, notes)

    # --- script-derived losses and behavior
    behavior = None
    behavior_reasons = []
    if sa["unknown"]:
        behavior_reasons.append(f"{sa['unknown']} script statements ({sa['loc']} lines); calls {', '.join(sa['calls'][:8]) or 'none'}")
    if sa["lib_scripts"]:
        behavior_reasons.append("library behavior " + ", ".join(lib_stem(p) for p in sa["lib_scripts"]))
    if sa["spawns"] and not spawns:
        behavior_reasons.append("spawns objects from script")
    if sa["invuln"]:
        behavior_reasons.append("invulnerability set from script")
    if endless:
        behavior_reasons.append("end condition is script-driven" if end_kind == "Script" else "ends on landing (verify)")
    if sa["sets_hitbox"]:
        behavior_reasons.append("script edits hitbox data")
    needs_behavior = bool(sa["unknown"] or (sa["spawns"] and not spawns) or sa["invuln"] or sa["lib_scripts"]
                          or end_kind == "Script" or sa["sets_hitbox"] or script_spawned)
    if script_spawned:
        behavior_reasons.append("spawns " + ", ".join(script_spawned) + " from script")
    if needs_behavior:
        behavior = (f"{CHAR_ID}.{sid_}" if sa["own_scripts"] and sa["unknown"] else
                    "lib." + snake(lib_stem(sa["lib_scripts"][0])) if sa["lib_scripts"] else f"{CHAR_ID}.{sid_}")
    if sa["sets_hitbox"]:
        losses.append("hitbox fields are edited by script; the converted values are the scene defaults")
    if sa["sets_length"]:
        losses.append("anim_length is set by script; the converted timeline uses the scene value")
    if sa["sets_iasa"]:
        losses.append("cancel timing (iasa) is set by script; the converted cancels use the scene value")
    if sa["frame_hooks"]:
        notes.append(f"per-tick hooks at {sa['frame_hooks'][:10]}")
    by_class = defaultdict(list)
    for key in sorted(clean_keys(ex) - STATE_CONSUMED - STATE_PRESENTATION - (THROW_CONSUMED if throw else set())):
        cls = STATE_CLASS.get(key)
        if cls is None:
            cls = "script" if key in sa["exports"] else "unknown"
        by_class[cls].append(key)
        PROP_HITS[("state", key, cls)].add(s["name"])
    if by_class.get("gap"):
        losses.append("data the format cannot hold: " + ", ".join(by_class["gap"]))
    if by_class.get("unknown"):
        losses.append("unrecognised scene properties: " + ", ".join(by_class["unknown"]))
    if by_class.get("script") and not behavior:
        behavior = f"{CHAR_ID}.{sid_}"
        behavior_reasons.append("scene parameters for script code: " + ", ".join(by_class["script"]))
    others = [c["name"] for c in s["other_children"]]
    if others:
        losses.append("extra child nodes not converted: " + ", ".join(others))
    if s["host_command_nodes"]:
        losses.append(f"{len(s['host_command_nodes'])} host-command nodes not converted")
    pres_dropped = sorted(clean_keys(ex) & STATE_PRESENTATION - {"button_texture", "flip_icon"})
    if pres_dropped:
        notes.append("presentation not converted: " + ", ".join(pres_dropped))

    allowed = [snake(x) for x in (k.get("allowed_stances_string") or "Normal").split("\n") if x.strip()]
    modes_for_move = [] if (not allowed or "all" in allowed) else allowed
    set_mode = snake(ex["change_stance_to"]) if ex.get("change_stance_to") else None
    MODE_USE.update(modes_for_move)
    if set_mode:
        MODE_USE.add(set_mode)

    fields = {"id": sid_, "name": k.get("title") or s["name"], "tags": tags_of(k.get("interrupt_from")),
              "stance": S({"Grounded": "Ground", "Aerial": "Air", "Both": "Any"}.get(k.get("air_type"), "Any"))}
    if modes_for_move:
        fields["modes"] = modes_for_move
    if set_mode:
        fields["set_mode"] = set_mode
    if inp is not None:
        fields["input"] = inp
    if params:
        fields["params"] = params
    if avail:
        fields["availability"] = S("Availability", **avail)
    fields["timeline"] = S("Timeline", **tl)
    if boxes:
        fields["hitboxes"] = boxes
    if throw:
        fields["throw"] = throw
    if hurt:
        fields["hurt"] = hurt
    if physics:
        fields["physics"] = physics
    if motion:
        fields["motion"] = motion
    if spawns:
        fields["spawns"] = [sp for _, sp in spawns]
    if cancels:
        fields["cancels"] = cancels
    fb = sid(k.get("fallback_state") or "Wait")
    if fb != "idle":
        fields["finish_into"] = fb
    if fb not in CONV_IDS and fb not in LIB_IDS and fb != "idle":
        losses.append(f"finish state {k.get('fallback_state')} is neither converted nor in the base library")
    if cost:
        fields["cost"] = S("Cost", **cost)
    if behavior:
        fields["behavior"] = behavior
    fields["presentation"] = S("Presentation", anim=snake(k.get("sprite_animation") or s["name"]))
    move = S("Move", **fields)

    data_partial = bool(losses)
    entry = {
        "state": s["name"], "id": sid_, "origin": s["origin"],
        "data": "partial" if data_partial else "complete",
        "needs_behavior": bool(behavior),
        "behavior": behavior,
        "behavior_reasons": behavior_reasons,
        "losses": losses, "notes": notes,
        "dropped_turn_based": by_class.get("turn", []), "engine_rules": by_class.get("engine", []),
        "script_params": by_class.get("script", []), "presentation_dropped": by_class.get("presentation", []),
        "input": None if inp is None else "placeholder",
        "no_input_reason": why_no_input,
        "hitboxes": len(boxes), "script_lines": sa["loc"],
        "params": [p.fields["name"] for p in params], "unbound_params": unbound,
        "throw": bool(throw),
        "spawn_projectiles": [p for p, _ in spawns],
    }
    return move, entry, sa


# ------------------------------------------------------------------ projectiles
def convert_projectiles():
    out, rep = [], []
    for p in data["projectiles"]:
        stem = os.path.splitext(os.path.basename(p["scene"]))[0]
        pid = snake(stem)
        rs = p.get("root_script") or ""
        if "error" in p:
            rep.append({"id": pid, "status": "parse error"})
            continue
        if rs.endswith("ParticleEffect.gd"):
            rep.append({"id": pid, "status": "visual effect only, skipped"})
            continue
        losses = []
        rprops = p["root_properties"]
        grav = rprops.get("gravity")
        if grav is None:
            losses.append("gravity not set in scene (script/engine default applies)")
        states = p["states"]
        with_boxes = [st for st in states if st["hitboxes"]]
        main = next((st for st in states if st["name"] == "Default" and st["hitboxes"]), with_boxes[0] if with_boxes else None)
        other = [st["name"] for st in with_boxes if st is not main]
        if other:
            losses.append("hitboxes in other projectile states not expressed: " + ", ".join(other))
        if len(states) > 1:
            losses.append("projectile has multiple states: " + ", ".join(st["name"] for st in states))
        lifetime = 0
        if states and isinstance(states[0]["explicit"].get("anim_length"), int) and not states[0]["explicit"].get("endless"):
            lifetime = states[0]["explicit"]["anim_length"]
        else:
            losses.append("lifetime is not in the scene (script or endless state)")
        losses.append("initial velocity is set by the spawning move or the script")
        boxes = []
        ids = set()
        if main:
            raw = main["hitboxes"]
            for i, h in enumerate(raw):
                boxes.append(convert_hitbox(h, pid, None, True, losses, i, len(raw), ids))
        fields = {"id": pid, "velocity": (0.0, 0.0)}
        if grav is not None:
            fields["gravity"] = num(grav)
        fields["lifetime"] = lifetime
        fields["hitboxes"] = boxes
        custom = rs and not rs.endswith("BaseProjectile.gd")
        if custom:
            fields["behavior"] = f"{CHAR_ID}.{pid}"
            losses.append(f"custom script {os.path.basename(rs)} ({analyze_script(rs)['loc']} lines)")
        out.append(S("Projectile", **fields))
        rep.append({"id": pid, "status": "converted" if not losses else "partial", "losses": losses,
                    "hitboxes": len(boxes), "behavior": fields.get("behavior")})
    return out, rep


# ------------------------------------------------------------------ run
moves = []
for s in OWN:
    mv, entry, _ = convert_state(s)
    moves.append(mv)
    report["states"].append(entry)

projectiles, prep = ([], [])
if not ONLY:
    projectiles, prep = convert_projectiles()
else:
    # single-move mode: keep only spawn targets that exist (none)
    for mv in moves:
        mv.fields.pop("spawns", None)
report["projectiles"] = prep

# spawn references must resolve
proj_ids = {p.fields["id"] for p in projectiles}
for mv, entry in zip(moves, report["states"]):
    sp = mv.fields.get("spawns")
    if sp:
        missing = [x.fields["projectile"] for x in sp if x.fields["projectile"] not in proj_ids]
        if missing:
            mv.fields["spawns"] = [x for x in sp if x.fields["projectile"] in proj_ids]
            if not mv.fields["spawns"]:
                del mv.fields["spawns"]
            entry["losses"].append("projectile scene(s) not converted: " + ", ".join(missing))
            entry["data"] = "partial"

# input collisions
groups = defaultdict(list)
for mv in moves:
    inp = mv.fields.get("input")
    if inp is not None:
        f = inp.fields
        key = (f["button"].name, f.get("direction").name if f.get("direction") else "Any", mv.fields["stance"].name)
        groups[key].append(mv.fields["id"])
report["input_conflicts"] = [{"binding": " / ".join(k), "moves": v} for k, v in sorted(groups.items()) if len(v) > 1]

body = S(
    "Body", max_health=1500, gravity=num(rootv("gravity")), ground_friction=num(rootv("ground_friction")),
    air_friction=num(rootv("air_friction")), max_ground_speed=num(rootv("max_ground_speed")),
    max_air_speed=num(rootv("max_air_speed")), max_fall_speed=num(rootv("max_fall_speed")),
    air_actions=int(rootv("num_air_movements", 0)), collision_box=scene_box("CollisionBox"), hurtbox=scene_box("Hurtbox"),
)
if num(rootv("damage_taken_modifier"), 1.0) != 1.0:
    body.fields["damage_taken"] = num(rootv("damage_taken_modifier"))
if num(rootv("knockback_taken_modifier"), 1.0) != 1.0:
    body.fields["knockback_taken"] = num(rootv("knockback_taken_modifier"))

rscript = root["root_script"]
mech = f"{CHAR_ID}" if (rscript and analyze_script(rscript)["loc"] > 30) else None
assumptions = [
    "Frames are 1-based. A hitbox with start_tick s becomes window s+1..s+active; startup = first start_tick. The engine's exact off-by-one is unconfirmed.",
    "Recovery runs to the original anim_length; cancels from iasa_at / interrupt_frames open on frame index+1 and last until the move ends.",
    "block_advantage is the original plus_frames; blockstun must be tuned by playtesting.",
    "Tags are the original interrupt_from categories (snake_case); a cancel's `into` matches any tag.",
    "Hit cancels open at the first active frame (the original opens them after hitlag) and include the engine-wide extras (burst, instant cancel, defense, grab).",
    "Physics flags come from scene flags plus recognised calls in _tick; every other script statement counts as unexpressed logic (see behavior).",
    "Input bindings are placeholders derived from the menu type and the move name. Many moves collide; see the report.",
    "Enter/timed forces are direction-normalised and rotated into facing space; y is flipped to 'up'.",
    "Units (speeds, forces, gravity) are copied unchanged, in the original's per-tick units.",
]
character = S("Character", id=CHAR_ID, name=CHAR_ID.capitalize(), body=body)
if mech:
    character.fields["mechanic"] = mech
if MODE_USE - {"normal"}:
    character.fields["modes"] = ["normal"] + sorted(MODE_USE - {"normal"})
else:   # a single mode means the mode fields carry no information
    for mv in moves:
        mv.fields.pop("modes", None)
        mv.fields.pop("set_mode", None)
character.fields["moves"] = moves
if projectiles:
    character.fields["projectiles"] = projectiles
character.fields["provenance"] = S(
    "Provenance",
    source=f"{os.path.basename(SRC)} ({root['scene']}); {len(OWN)} character states, {len(projectiles)} projectiles",
    assumptions=assumptions,
    todo=["Replace placeholder input bindings with a real control scheme.",
          "Write the behaviors named in `behavior` and the character mechanic.",
          "Resolve the losses listed in the conversion report, then verify frame counts in the running game."])
doc = S("CharacterFile", format_version=1, character=character)
header = ("#![enable(implicit_some)]\n"
          f"// Converted by tools/convert_character.py from {os.path.basename(SRC)}.\n"
          "// PRIVATE: contains numbers taken from the original game. Do not publish.\n")
os.makedirs(os.path.dirname(OUT) or ".", exist_ok=True)
open(OUT, "w", encoding="utf-8").write(header + emit(doc) + "\n")
print("wrote", OUT, f"({len(moves)} moves, {len(projectiles)} projectiles)")

if ONLY or REPORT == "-":
    sys.exit(0)

# ------------------------------------------------------------------ report
st = report["states"]
n = len(st)
complete = [e for e in st if e["data"] == "complete" and not e["needs_behavior"]]
beh_only = [e for e in st if e["data"] == "complete" and e["needs_behavior"]]
partial = [e for e in st if e["data"] == "partial"]
loss_kinds = defaultdict(list)
PREFIXES = ("hitbox properties not expressed", "data the format cannot hold", "unrecognised scene properties",
            "parameter widgets with no mapping", "extra child nodes not converted",
            "projectile scene(s) not converted")
for e in st:
    for l in e["losses"]:
        key = next((p for p in PREFIXES if l.startswith(p)), None)
        if key is None:
            key = re.sub(r"projectile \w+ is spawned", "projectile is spawned", l)
            key = re.sub(r"hitbox \S+: ", "hitbox: ", key)
            key = re.sub(r"\d+", "N", key)
        loss_kinds[key].append(e["state"])
report["summary"] = {
    "states_converted": n, "complete": len(complete), "complete_needs_behavior": len(beh_only),
    "partial": len(partial), "with_behavior": sum(e["needs_behavior"] for e in st),
    "script_lines_needing_port": sum(e["script_lines"] for e in st if e["needs_behavior"]),
    "base_states_not_converted": len(report["base_states_not_converted"]),
    "input_collisions": len(report["input_conflicts"]),
    "moves_with_input": sum(1 for e in st if e["input"]), "moves_without_input": sum(1 for e in st if not e["input"]),
}
json.dump(report, open(REPORT + ".json", "w", encoding="utf-8"), indent=1)

L = [f"# Conversion report: {CHAR_ID}", "",
     f"Generated by `tools/convert_character.py` from `{os.path.basename(SRC)}`. PRIVATE (derived from game data).", "",
     "## Summary", "",
     f"- States converted: **{n}** (the character's own and overriding states). {len(report['base_states_not_converted'])} inherited base states are not converted; they belong to the shared base library.",
     f"- **{len(complete)}** convert completely as data (no script logic, nothing dropped).",
     f"- **{len(beh_only)}** convert as data but also need a named behavior (script logic or library behavior).",
     f"- **{len(partial)}** have data that could not be fully expressed (see their losses).",
     f"- **{report['summary']['with_behavior']}** states need a behavior in total, covering about **{report['summary']['script_lines_needing_port']}** lines of original script.",
     f"- Input: {report['summary']['moves_with_input']} moves got a placeholder binding, {report['summary']['moves_without_input']} have none (entered by other moves or the engine). {len(report['input_conflicts'])} bindings are shared by several moves.",
     f"- Projectile scenes: {sum(1 for p in prep if p['status'] in ('converted','partial'))} converted, {sum(1 for p in prep if p['status'].startswith('visual'))} visual-only skipped.", "",
     "## What could not be expressed, by kind", "", "| kind | moves | examples |", "|---|---|---|"]
for kind, names in sorted(loss_kinds.items(), key=lambda kv: -len(kv[1])):
    L.append(f"| {kind} | {len(set(names))} | {', '.join(sorted(set(names))[:5])} |")
CLASS_TITLE = {
    "gap": "Gameplay data the format cannot hold yet (real losses)",
    "engine": "Engine rules to re-decide in the new game",
    "turn": "Turn-based concepts dropped by design",
    "script": "Parameters that belong with behavior code",
    "unknown": "Not recognised by the converter",
    "presentation": "Presentation (sound, shake, particles), not converted",
}
report["properties"] = [
    {"property": p, "level": lv, "class": c, "moves": sorted(v), "note": PROP_NOTE.get(p, "")}
    for (lv, p, c), v in sorted(PROP_HITS.items(), key=lambda kv: (kv[0][2], -len(kv[1]), kv[0][1]))]
json.dump(report, open(REPORT + ".json", "w", encoding="utf-8"), indent=1)
L += ["", "## Scene properties that were not converted, by class", ""]
for cls in ("gap", "engine", "turn", "script", "unknown", "presentation"):
    rows = [p for p in report["properties"] if p["class"] == cls]
    if not rows:
        continue
    L += [f"### {CLASS_TITLE[cls]}", "", "| property | on | moves | what it is | examples |", "|---|---|---|---|---|"]
    for p in rows:
        L.append(f"| {p['property']} | {p['level']} | {len(p['moves'])} | {p['note'] or '-'} | {', '.join(p['moves'][:4])} |")
    L.append("")
L += ["", "## Per state", "",
      "| state | data | behavior | input | boxes | script lines | losses and notes |", "|---|---|---|---|---|---|---|"]
for e in st:
    txt = "; ".join(e["losses"] + e["notes"] + (["entered by other moves"] if e["no_input_reason"] and "entered" in e["no_input_reason"] else []))
    L.append(f"| {e['state']} | {e['data']} | {e['behavior'] or '-'} | {'placeholder' if e['input'] else '-'} | {e['hitboxes']} | {e['script_lines'] or '-'} | {txt.replace('|', '/')[:240]} |")
L += ["", "## Behavior reasons", ""]
for e in st:
    if e["behavior_reasons"]:
        L.append(f"- **{e['state']}** → `{e['behavior']}`: " + "; ".join(e["behavior_reasons"]))
L += ["", "## Projectiles", "", "| id | status | boxes | behavior | losses |", "|---|---|---|---|---|"]
for p in prep:
    L.append(f"| {p['id']} | {p['status']} | {p.get('hitboxes', '-')} | {p.get('behavior') or '-'} | {'; '.join(p.get('losses', []))[:200].replace('|', '/')} |")
L += ["", "## Input bindings shared by several moves", ""]
for c in report["input_conflicts"]:
    L.append(f"- `{c['binding']}`: {', '.join(c['moves'])}")
L += ["", "## Base states not converted", "", ", ".join(report["base_states_not_converted"]), ""]
open(REPORT + ".md", "w", encoding="utf-8").write("\n".join(L))
print("wrote", REPORT + ".md/.json")
print(json.dumps(report["summary"]))

# ------------------------------------------------------------------ presentation (sounds, particles)
# Written next to the character as <out>_fx.ron for the game's effects player. Covers every state
# in the extracted data (also the shared base states the engine owns, such as jumping), keyed by
# move id; hitboxes are keyed by the same ids the converter gives them.
HITBOX_SOUND_DEFAULTS = {"hit_sound": ("sound/common/hit1.wav", -5.0), "whiff_sound": ("sound/common/whiff1.wav", -8.0)}


def res_path(v):
    return v.get("res") if isinstance(v, dict) and "res" in v else None


def vec2(v, default=(0.0, 0.0)):
    if isinstance(v, dict) and "Vector2" in v:
        return tuple(float(x) for x in v["Vector2"])
    return default


def sound(path, volume):
    return S("Sound", path=path, volume=float(volume)) if path else None


def opt(v):
    return S("Some", v=v) if v is not None else None


def fx_ron(v, ind=0):
    """emit() with Option<T> as RON's Some(...)/None."""
    if v is None:
        return "None"
    if isinstance(v, S) and v.name == "Some":
        return "Some(" + fx_ron(v.fields["v"], ind) + ")"
    if isinstance(v, S):
        pad = "    " * ind
        body = "".join(pad + "    " + f"{k}: {fx_ron(x, ind + 1)},\n" for k, x in v.fields.items())
        return f"{v.name}(\n{body}{pad})"
    if isinstance(v, dict):
        pad = "    " * ind
        if not v:
            return "{}"
        return "{\n" + "".join(pad + "    " + f"{emit(k)}: {fx_ron(x, ind + 1)},\n" for k, x in v.items()) + pad + "}"
    return emit(v, ind)


def state_fx(s):
    ex = s.get("explicit_overrides", {})
    enter = sound(res_path(ex.get("enter_sfx")), ex.get("enter_sfx_volume", -15.0))
    timed = res_path(ex.get("sfx"))
    sfx = S("TimedSound", path=timed, tick=int(ex.get("sfx_tick", 1)), volume=float(ex.get("sfx_volume", -15.0))) if timed else None
    release = sound(res_path(ex.get("release_sfx")), ex.get("release_sfx_volume", -10.0))
    particle = None
    if res_path(ex.get("particle_scene")) and ex.get("spawn_particle_on_enter"):
        particle = S("Particle", scene=res_path(ex["particle_scene"]), pos=vec2(ex.get("particle_position")), tick=1)
    timed_particle = None
    if res_path(ex.get("timed_particle_scene")):
        timed_particle = S("Particle", scene=res_path(ex["timed_particle_scene"]), pos=vec2(ex.get("timed_particle_position")),
                           tick=int(ex.get("timed_spawn_particle_tick", 1)))
    boxes, ids = {}, set()
    n = len(s["hitboxes"])
    for h in s["hitboxes"]:
        base_id = snake(h["name"]) if n > 1 else "main"
        hid, j = base_id, 2
        while hid in ids:
            hid, j = f"{base_id}_{j}", j + 1
        ids.add(hid)
        e = h.get("explicit", {})
        snd = {}
        for key, (dpath, dvol) in HITBOX_SOUND_DEFAULTS.items():
            path = res_path(e[key]) if key in e else dpath
            snd[key] = sound(path, e.get(key + "_volume", dvol))
        bass = res_path(e["hit_bass_sound"]) if "hit_bass_sound" in e else "sound/common/hit_bass.wav"
        boxes[hid] = S(
            "HitboxFx",
            hit_sound=opt(snd["hit_sound"]),
            whiff_sound=opt(snd["whiff_sound"]),
            bass_sound=opt(sound(bass, e.get("bass_sound_volume", -5.0))),
            bass_on_whiff=bool(e.get("bass_on_whiff", False)),
            hit_particle=opt(res_path(e.get("hit_particle"))),
            replace_hit_particle=bool(e.get("replace_hit_particle", False)),
            whiff_particle=opt(res_path(e.get("whiff_particle"))),
            screenshake=int(e.get("screenshake_amount", 4)),
            screenshake_frames=int(e.get("screenshake_frames", -1)),
            bump_dir=vec2(e.get("camera_bump_dir")),
        )
    return S("MoveFx", enter_sfx=opt(enter), sfx=opt(sfx), release_sfx=opt(release), particle=opt(particle),
             timed_particle=opt(timed_particle), hitboxes=boxes)


fx_moves = {}
for s in data["states"]:
    mid = sid(s["name"])
    if mid not in fx_moves:
        fx_moves[mid] = state_fx(s)
fx_out = OUT[:-4] + "_fx.ron" if OUT.endswith(".ron") else OUT + "_fx.ron"
open(fx_out, "w", encoding="utf-8").write(
    f"// Sounds and effects of the converted {CHAR_ID} (PRIVATE: paths into the original game's files).\n"
    "// Generated by tools/convert_character.py.\n"
    + fx_ron(S("CharacterFx", moves=fx_moves)) + "\n")
print("wrote", fx_out)
