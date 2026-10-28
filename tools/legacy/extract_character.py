#!/usr/bin/env python3
"""Extract a YOMI Hustle character (Godot 3 .tscn) into structured JSON.

Usage: python3 -I extract_character.py <recovered_root> <character_dir> <scene_name> <out.json>
Reads only; writes the output file. Pure standard library.
"""
import json
import os
import re
import sys

ROOT = sys.argv[1]
CHAR_DIR = sys.argv[2]            # e.g. characters/stickman
SCENE = sys.argv[3]               # e.g. NinjaGuy.tscn
OUT = sys.argv[4]

ENUMS = {
    "ActionType": ["Movement", "Attack", "Special", "Super", "Defense", "Hurt"],
    "AirType": ["Grounded", "Aerial", "Both"],
    "BusyInterrupt": ["Normal", "Hurt", "None"],
    "HitboxType": ["Normal", "Flip", "ThrowHit", "OffensiveBurst", "Burst", "NoHitstun", "Detect"],
    "HitHeight": ["High", "Mid", "Low"],
}
PROP_ENUM = {
    "type": "ActionType",
    "air_type": "AirType",
    "busy_interrupt_type": "BusyInterrupt",
    "hitbox_type": "HitboxType",
    "hit_height": "HitHeight",
}

NUM_RE = re.compile(r"^-?\d+(\.\d+)?([eE][-+]?\d+)?$")


def full(path):
    return os.path.join(ROOT, path)


def convert(raw, ext=None):
    """Convert a Godot text value to a JSON-friendly Python value."""
    s = raw.strip()
    s = re.sub(r"^-\s+(?=\d)", "-", s)           # decompiler prints "- 1"
    if s in ("true", "false"):
        return s == "true"
    if s == "null":
        return None
    if NUM_RE.match(s):
        return float(s) if ("." in s or "e" in s.lower()) else int(s)
    if len(s) >= 2 and s[0] == '"' and s[-1] == '"':
        inner = s[1:-1]
        return inner.replace('\\"', '"').replace("\\n", "\n").replace("\\\\", "\\")
    if s.startswith("[") and s.endswith("]"):
        inner = s[1:-1].strip()
        if inner == "":
            return []
        if re.fullmatch(r"[-\d.\s,]+", inner):
            return [convert(p) for p in inner.split(",") if p.strip()]
    m = re.match(r"^ExtResource\(\s*(\d+)\s*\)$", s)
    if m:
        if ext and int(m.group(1)) in ext:
            return {"res": ext[int(m.group(1))]["path"]}
        return {"res_id": int(m.group(1))}
    m = re.match(r"^SubResource\(\s*(\d+)\s*\)$", s)
    if m:
        return {"sub_id": int(m.group(1))}
    m = re.match(r"^(Vector2|Color|Rect2|Vector3)\(\s*(.*?)\s*\)$", s)
    if m:
        parts = [convert(p) for p in m.group(2).split(",")]
        return {m.group(1): parts}
    return {"raw": s}


def balanced(text):
    """True if quotes and brackets are balanced (for multi-line values)."""
    in_str = False
    esc = False
    depth = 0
    for ch in text:
        if esc:
            esc = False
            continue
        if ch == "\\":
            esc = True
            continue
        if ch == '"':
            in_str = not in_str
            continue
        if not in_str:
            if ch in "([{":
                depth += 1
            elif ch in ")]}":
                depth -= 1
    return (not in_str) and depth <= 0


def parse_header_attrs(header):
    attrs = {}
    for m in re.finditer(r'(\w+)=("(?:[^"\\]|\\.)*"|ExtResource\(\s*\d+\s*\)|SubResource\(\s*\d+\s*\)|[^\s\]]+)', header):
        attrs[m.group(1)] = m.group(2)
    return attrs


def parse_tscn(relpath):
    ext, sub, nodes = {}, {}, []
    cur = None
    kind = None
    with open(full(relpath), encoding="utf-8") as f:
        lines = f.read().split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        if line.startswith("["):
            hm = re.match(r"^\[(\w+)\s*(.*)\]\s*$", line)
            if not hm:
                continue
            kind, rest = hm.group(1), hm.group(2)
            attrs = parse_header_attrs(rest)
            if kind == "ext_resource":
                ext[int(attrs["id"])] = {
                    "path": attrs["path"].strip('"').replace("res://", ""),
                    "type": attrs.get("type", "").strip('"'),
                }
                cur = None
            elif kind == "sub_resource":
                cur = {"type": attrs.get("type", "").strip('"'), "props": {}}
                sub[int(attrs["id"])] = cur
            elif kind == "node":
                cur = {
                    "name": attrs["name"].strip('"'),
                    "type": attrs.get("type", "").strip('"') or None,
                    "parent": attrs.get("parent", "").strip('"') or None,
                    "index": attrs.get("index", "").strip('"') or None,
                    "instance": attrs.get("instance"),
                    "props": {},
                    "_raw": {},
                }
                nodes.append(cur)
            else:
                cur = None
            continue
        if cur is None or not line.strip() or line.startswith(";"):
            continue
        m = re.match(r"^([A-Za-z_][\w/:]*)\s*=\s*(.*)$", line)
        if not m:
            continue
        key, val = m.group(1), m.group(2)
        while not balanced(val) and i < len(lines):
            val += "\n" + lines[i]
            i += 1
        cur.setdefault("_raw", {})[key] = val
        cur["props"][key] = convert(val, ext)
    return {"ext": ext, "sub": sub, "nodes": nodes}


# ---------------------------------------------------------------- script info
CLASS_PATHS = {}


def index_classes():
    for dirpath, _dirs, files in os.walk(ROOT):
        if "/.import" in dirpath:
            continue
        for fn in files:
            if fn.endswith(".gd"):
                p = os.path.join(dirpath, fn)
                try:
                    with open(p, encoding="utf-8") as f:
                        head = f.read(4000)
                except OSError:
                    continue
                m = re.search(r"^class_name\s+(\w+)", head, re.M)
                if m:
                    CLASS_PATHS[m.group(1)] = os.path.relpath(p, ROOT)


_script_cache = {}


def script_info(relpath):
    """Parse a .gd file: extends, export defaults, consts, funcs."""
    if relpath in _script_cache:
        return _script_cache[relpath]
    info = {"path": relpath, "extends": None, "exports": {}, "consts": {}, "funcs": [], "frame_hooks": []}
    p = full(relpath)
    if not os.path.isfile(p):
        _script_cache[relpath] = info
        return info
    with open(p, encoding="utf-8") as f:
        text = f.read()
    m = re.search(r"^extends\s+(\"[^\"]+\"|[\w.]+)", text, re.M)
    if m:
        info["extends"] = m.group(1).strip('"')
    for m in re.finditer(r"^export\s*(?:\(([^)]*)\))?\s*var\s+(\w+)\s*(?::\s*\w+)?\s*(?:=\s*(.+))?$", text, re.M):
        enum_name, var, default = m.group(1), m.group(2), m.group(3)
        if var.startswith("_c_"):
            continue
        if default is None:
            info["exports"][var] = None
            continue
        default = default.strip()
        em = re.match(r"^(\w+)\.(\w+)$", default)
        if em and em.group(1) in ENUMS and em.group(2) in ENUMS[em.group(1)]:
            info["exports"][var] = em.group(2)
        else:
            info["exports"][var] = convert(default)
    for m in re.finditer(r"^const\s+(\w+)\s*=\s*(.+)$", text, re.M):
        info["consts"][m.group(1)] = convert(m.group(2))
    for m in re.finditer(r"^func\s+(\w+)\(", text, re.M):
        name = m.group(1)
        info["funcs"].append(name)
        fm = re.match(r"^_frame_(\d+)(_shared)?$", name)
        if fm:
            info["frame_hooks"].append(int(fm.group(1)))
    info["frame_hooks"] = sorted(set(info["frame_hooks"]))
    info["sets_anim_length"] = bool(re.search(r"\banim_length\s*=(?!=)", text))
    info["sets_iasa"] = bool(re.search(r"\biasa_at\s*=(?!=)", text))
    info["sets_hitbox_fields"] = bool(re.search(
        r"\.(start_tick|active_ticks|damage|knockback|hitstun_ticks|dir_x|dir_y)\s*=(?!=)", text))
    info["applies_forces_in_code"] = bool(re.search(r"\bapply_force\w*\(|\bset_vel\(|\bmove_directly\w*\(", text))
    _script_cache[relpath] = info
    return info


def resolve_extends(ext):
    if ext is None:
        return None
    if ext.startswith("res://"):
        return ext.replace("res://", "")
    return CLASS_PATHS.get(ext)


def class_chain(relpath):
    chain = []
    seen = set()
    cur = relpath
    while cur and cur not in seen:
        seen.add(cur)
        info = script_info(cur)
        chain.append(cur)
        cur = resolve_extends(info["extends"])
    return chain


def chain_defaults(relpath):
    """Merged export defaults, base class first."""
    merged = {}
    for p in reversed(class_chain(relpath)):
        for k, v in script_info(p)["exports"].items():
            merged[k] = v
    return merged


def decode_enums(props):
    out = {}
    for k, v in props.items():
        en = PROP_ENUM.get(k)
        if en and isinstance(v, int) and 0 <= v < len(ENUMS[en]):
            out[k] = ENUMS[en][v]
        else:
            out[k] = v
    return out


def multiline_list(v):
    if isinstance(v, str):
        return [x for x in v.split("\n") if x.strip()]
    return v


# ------------------------------------------------------------------ scene walk
def node_path(n):
    if n["parent"] in (None, "."):
        return n["name"] if n["parent"] is None else n["name"]
    return n["parent"] + "/" + n["name"]


def scene_nodes(scene):
    """Map path -> merged node, merging an instanced base scene underneath."""
    nodes = {}
    base_path = None
    for n in scene["nodes"]:
        if n["parent"] is None and n["instance"]:
            m = re.search(r"ExtResource\(\s*(\d+)\s*\)", n["instance"])
            base_path = scene["ext"][int(m.group(1))]["path"]
    base = {}
    if base_path:
        base_scene = parse_tscn(base_path)
        base = scene_nodes(base_scene)[0]
        base_ext = base_scene["ext"]
        for path, bn in base.items():
            bn["_ext"] = base_ext
    for n in scene["nodes"]:
        path = node_path(n)
        if n["parent"] is None:
            path = "."
        elif n["parent"] == ".":
            path = n["name"]
        n["_ext"] = scene["ext"]
        if path in base:
            merged = dict(base[path])
            merged["props"] = dict(base[path]["props"])
            merged["props"].update(n["props"])
            # a script set in the child scene replaces the inherited one
            if "script" in n["props"]:
                merged["_script_ext"] = scene["ext"]
            merged["type"] = n["type"] or base[path]["type"]
            merged["origin"] = "base_override"
            merged["_overrides"] = sorted(n["props"].keys())
            nodes[path] = merged
        else:
            nn = dict(n)
            nn["origin"] = "character"
            nn["_overrides"] = sorted(n["props"].keys())
            nodes[path] = nn
    for path, bn in base.items():
        if path not in nodes:
            b = dict(bn)
            b["origin"] = "base"
            b["_overrides"] = []
            nodes[path] = b
    return nodes, base_path


def node_script(n, scene_ext):
    s = n["props"].get("script")
    if isinstance(s, dict) and "res" in s:
        return s["res"]
    if isinstance(s, dict) and "res_id" in s:
        ext = n.get("_script_ext") or n.get("_ext") or scene_ext
        if s["res_id"] in ext:
            return ext[s["res_id"]]["path"]
    return None


def clean_props(props):
    return {k: v for k, v in props.items() if k not in ("script", "__meta__", "z_index")}


def hitbox_record(n, scene_ext, defaults_cache):
    script = node_script(n, scene_ext) or "mechanics/Hitbox.gd"
    if script not in defaults_cache:
        defaults_cache[script] = chain_defaults(script)
    dflt = defaults_cache[script]
    props = decode_enums(clean_props(n["props"]))
    resolved = decode_enums(dict(dflt))
    resolved.update(props)
    key_fields = [
        "damage", "damage_in_combo", "minimum_damage", "start_tick", "active_ticks", "always_on",
        "hitstun_ticks", "combo_hitstun_ticks", "hitlag_ticks", "victim_hitlag", "combo_victim_hitlag",
        "knockback", "dir_x", "dir_y", "pushback_x", "hit_height", "hitbox_type",
        "grounded_hit_state", "aerial_hit_state", "knockdown", "hard_knockdown", "ground_bounce",
        "wall_slam", "guard_break", "plus_frames", "parriable", "block_cancel_allowed",
        "damage_proration", "scale_combo", "combo_scaling_amount", "cancellable", "can_clash",
        "beats_grab", "hits_otg", "hits_vs_aerial", "hits_vs_grounded", "hits_projectiles",
        "looping", "loop_active_ticks", "loop_inactive_ticks", "followup_state", "group", "priority",
        "x", "y", "width", "height", "block_pushback_modifier", "chip_damage_modifier",
        "meter_gain_modifier", "di_modifier", "sdi_modifier", "ignore_armor",
    ]
    key = {k: resolved[k] for k in key_fields if k in resolved}
    return {
        "name": n["name"],
        "kind": "throwbox" if "ThrowBox" in (script or "") else "hitbox",
        "script": script,
        "key": key,
        "explicit": props,
    }


def ui_params(scene_path):
    """Parameters a player chooses for a move (from its ActionUIData scene)."""
    if not scene_path or not os.path.isfile(full(scene_path)):
        return None
    sc = parse_tscn(scene_path)
    params = []
    for n in sc["nodes"]:
        if n["parent"] != ".":
            continue
        props = clean_props(n["props"])
        keep = {}
        for k, v in props.items():
            if k.startswith(("margin_", "anchor_", "rect_", "size_flags", "mouse_", "theme", "focus")):
                continue
            keep[k] = v
        inst = None
        if n["instance"]:
            m = re.search(r"ExtResource\(\s*(\d+)\s*\)", n["instance"])
            if m:
                inst = sc["ext"][int(m.group(1))]["path"]
        params.append({"name": n["name"], "type": n["type"], "instance": inst, "props": keep})
    return params


def derive(state_resolved, hitboxes, anim_length):
    d = {"anim_length": anim_length}
    iasa = state_resolved.get("iasa_at", -1)
    if isinstance(iasa, int):
        d["iasa_tick"] = anim_length + iasa if iasa < 0 else iasa
    if state_resolved.get("endless"):
        d["endless"] = True
        d["note"] = "endless state: runs until script ends it (e.g. landing); anim_length/iasa_tick are not the real duration"
    real = [h for h in hitboxes if h["kind"] == "hitbox"]
    throws = [h for h in hitboxes if h["kind"] == "throwbox"]
    if throws:
        d["throwbox_count"] = len(throws)
        d["throw_start_ticks"] = [h["key"].get("start_tick", 0) for h in throws]
    if real:
        starts = [h["key"].get("start_tick", 0) for h in real]
        ends = [h["key"].get("start_tick", 0) + h["key"].get("active_ticks", 0) for h in real]
        d["first_hitbox_start_tick"] = min(starts)
        d["last_hitbox_end_tick"] = max(ends)
        if "iasa_tick" in d:
            d["ticks_from_last_active_to_iasa"] = d["iasa_tick"] - max(ends)
        d["hitbox_count"] = len(real)
        d["max_damage"] = max(h["key"].get("damage", 0) for h in real)
    return d


def main():
    index_classes()
    scene = parse_tscn(os.path.join(CHAR_DIR, SCENE))
    nodes, base_path = scene_nodes(scene)
    scene_ext = scene["ext"]
    defaults_cache = {}

    # root fighter
    root = nodes["."]
    root_script = node_script(root, scene_ext)
    character = {
        "scene": os.path.join(CHAR_DIR, SCENE),
        "inherits": base_path,
        "root_script": root_script,
        "root_script_consts": script_info(root_script)["consts"] if root_script else {},
        "root_properties": decode_enums(clean_props(root["props"])),
        "root_class_defaults": {k: v for k, v in chain_defaults(root_script).items()} if root_script else {},
    }

    # states and their children
    children = {}
    for path, n in nodes.items():
        if path.startswith("StateMachine/") and path.count("/") >= 2:
            parent = "/".join(path.split("/")[:-1])
            children.setdefault(parent, []).append(n)

    states = []
    for path, n in sorted(nodes.items(), key=lambda kv: int(kv[1].get("index") or 0)):
        if not path.startswith("StateMachine/") or path.count("/") != 1:
            continue
        script = node_script(n, scene_ext)
        if not script:
            continue
        dflt = chain_defaults(script)
        explicit = decode_enums(clean_props(n["props"]))
        resolved = decode_enums(dict(dflt))
        resolved.update(explicit)
        anim_length = resolved.get("anim_length", 1)
        hitboxes, hurtboxes, commands, others = [], [], [], []
        for c in sorted(children.get(path, []), key=lambda x: int(x.get("index") or 0)):
            cs = node_script(c, scene_ext) or ""
            if cs.endswith(("mechanics/Hitbox.gd", "ThrowBox.gd")) or "Hitbox" in cs:
                hitboxes.append(hitbox_record(c, scene_ext, defaults_cache))
            elif "HurtboxState" in cs or "LimbHurtbox" in cs:
                hurtboxes.append({"name": c["name"], "script": cs, "props": decode_enums(clean_props(c["props"]))})
            elif c["name"].startswith("HostCommand") or "HostCommand" in cs:
                commands.append({"name": c["name"], "script": cs, "props": clean_props(c["props"])})
            else:
                others.append({"name": c["name"], "script": cs, "props": clean_props(c["props"])})
        ui = resolved.get("data_ui_scene")
        ui_path = ui["res"] if isinstance(ui, dict) and "res" in ui else None
        sinfo = script_info(script)
        key_fields = [
            "title", "type", "air_type", "anim_length", "sprite_animation", "iasa_at", "iasa_on_hit",
            "interrupt_frames", "fallback_state", "combo_only", "neutral_only", "tick_priority",
            "can_be_counterhit", "beats_backdash", "initiative_effect", "super_level_",
            "super_freeze_ticks_", "uses_air_movement", "land_cancel", "landing_recovery",
            "apply_forces", "apply_fric", "apply_grav", "reset_momentum", "force_tick",
            "enter_force_speed", "force_speed", "projectile_tick", "projectile_pos_x",
            "projectile_pos_y", "endless", "loop_animation", "disable_at_end", "extra_parry_hitlag", "throw_techable", "interruptible_on_opponent_turn",
            "end_feint", "can_feint_if_possible", "reversible", "burstable", "show_in_menu",
            "selectable", "allowed_stances_string",
        ]
        key = {k: resolved[k] for k in key_fields if k in resolved}
        for lk in ("interrupt_from_string", "interrupt_into_string", "hit_cancel_into_string",
                   "interrupt_exceptions_string", "hit_cancel_exceptions_string"):
            if lk in resolved:
                key[lk.replace("_string", "")] = multiline_list(resolved[lk])
        if "projectile_scene" in resolved and isinstance(resolved["projectile_scene"], dict):
            key["projectile_scene"] = resolved["projectile_scene"].get("res")
        states.append({
            "name": n["name"],
            "origin": n["origin"],
            "script": script,
            "class_chain": class_chain(script),
            "key": key,
            "explicit_overrides": explicit,
            "hitboxes": hitboxes,
            "hurtbox_states": hurtboxes,
            "host_command_nodes": commands,
            "other_children": others,
            "player_parameters": ui_params(ui_path),
            "ui_scene": ui_path,
            "script_info": {
                "sets_anim_length_in_code": any(script_info(p)["sets_anim_length"] for p in class_chain(script)[:2]),
                "sets_iasa_in_code": any(script_info(p)["sets_iasa"] for p in class_chain(script)[:2]),
                "sets_hitbox_fields_in_code": any(script_info(p)["sets_hitbox_fields"] for p in class_chain(script)[:2]),
                "applies_forces_in_code": any(script_info(p)["applies_forces_in_code"] for p in class_chain(script)[:2]),
                "consts": sinfo["consts"],
                "frame_hooks": sinfo["frame_hooks"],
                "funcs": [f for f in sinfo["funcs"] if f.startswith("_") and not f.startswith("_frame_")],
            },
            "derived": derive(resolved, hitboxes, anim_length if isinstance(anim_length, int) else 1),
        })

    # projectiles / objects
    projectiles = []
    pdir = full(os.path.join(CHAR_DIR, "projectiles"))
    for fn in sorted(os.listdir(pdir)):
        if not fn.endswith(".tscn"):
            continue
        rel = os.path.join(CHAR_DIR, "projectiles", fn)
        try:
            psc = parse_tscn(rel)
        except Exception as e:  # keep going on odd files
            projectiles.append({"scene": rel, "error": str(e)})
            continue
        pn, pbase = scene_nodes(psc)
        proot = pn.get(".")
        rs = node_script(proot, psc["ext"]) if proot else None
        pstates = []
        for path, n in pn.items():
            if path.startswith("StateMachine/") and path.count("/") == 1:
                sscript = node_script(n, psc["ext"])
                pstates.append({
                    "name": n["name"],
                    "script": sscript,
                    "explicit": decode_enums(clean_props(n["props"])),
                    "hitboxes": [hitbox_record(c, psc["ext"], defaults_cache)
                                 for c in sorted(
                                     [x for p2, x in pn.items() if p2.startswith(path + "/") and p2.count("/") == 2],
                                     key=lambda x: int(x.get("index") or 0))
                                 if "Hitbox" in (node_script(c, psc["ext"]) or "")],
                })
        projectiles.append({
            "scene": rel,
            "inherits": pbase,
            "root_script": rs,
            "root_properties": decode_enums(clean_props(proot["props"])) if proot else {},
            "states": pstates,
        })

    out = {
        "meta": {
            "source": "Decompiled Your Only Move Is HUSTLE (Godot 3.5.1) - private reference data",
            "character_scene": character["scene"],
            "notes": [
                "'key' holds resolved values (script defaults merged with scene overrides); 'explicit_overrides' holds only what the scene sets.",
                "Fixed-point fields (knockback, dir_x, dir_y, forces) are strings in the original; kept as strings.",
                "Ticks are the game's fixed simulation ticks. start_tick/active_ticks are as authored; the engine activates a hitbox when the state's tick counter reaches start_tick-1 (see notes/01-02), so treat start_tick as the first hit-capable tick approximately.",
                "'iasa_tick' resolves negative iasa_at as anim_length + iasa_at, matching CharState.gd.",
                "Move logic that lives in GDScript (forces, spawns, conditional branches) is NOT in this file; see 'script_info' and the script path for what exists.",
                "derived.* values are computed here, not read from the game.",
            ],
        },
        "enums": ENUMS,
        "character": character,
        "states": states,
        "projectiles": projectiles,
    }
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(out, f, indent=1, ensure_ascii=False)
    print("states:", len(states), "hitboxes:", sum(len(s["hitboxes"]) for s in states),
          "projectile scenes:", len(projectiles))


if __name__ == "__main__":
    main()
