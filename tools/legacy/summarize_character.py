#!/usr/bin/env python3
"""Turn the extracted character JSON into a readable Markdown summary table."""
import json
import sys

src, out = sys.argv[1], sys.argv[2]
d = json.load(open(src, encoding="utf-8"))
states = d["states"]


def fmt(v):
    return "-" if v is None else str(v)


lines = []
lines.append("# Ninja (stickman) move summary\n")
lines.append("Generated from `data/ninja.json` (private reference data; do not publish).\n")
lines.append("Ticks are 1/60 s simulation ticks. `start`/`end` = first hitbox start tick and last hitbox "
             "start+active; `iasa` = tick the move can be left (not meaningful for endless moves). "
             "`dmg` = highest single-hitbox damage; `combo dmg` = damage_in_combo of that hitbox "
             "(-1 means same as damage). Values come from the scene; script-driven behavior is not included.\n")

groups = [
    ("Player-selectable moves (character-specific)", lambda s: s["origin"] != "base" and s["key"].get("show_in_menu", True) and s["hitboxes"]),
    ("Shared/base states with hitboxes", lambda s: s["origin"] == "base" and s["hitboxes"]),
]
for title, pred in groups:
    rows = [s for s in states if pred(s)]
    lines.append(f"\n## {title} ({len(rows)})\n")
    lines.append("| state | title | type | air | len | iasa | start | end | boxes | dmg | combo dmg | hitstun | hitlag | knockback | height | notes |")
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for s in rows:
        k, dv = s["key"], s["derived"]
        hbs = [h for h in s["hitboxes"]]
        top = max(hbs, key=lambda h: h["key"].get("damage", 0))
        hk = top["key"]
        notes = []
        if dv.get("endless"):
            notes.append("endless")
        if s["script_info"]["sets_hitbox_fields_in_code"]:
            notes.append("script edits hitbox")
        if s["script_info"]["applies_forces_in_code"]:
            notes.append("script moves")
        if k.get("combo_only"):
            notes.append("combo only")
        if dv.get("throwbox_count"):
            notes.append(f"throw @{dv['throw_start_ticks']}")
        lines.append("| " + " | ".join(map(fmt, [
            s["name"], k.get("title"), k.get("type"), k.get("air_type"),
            "endless" if dv.get("endless") else dv.get("anim_length"),
            "-" if dv.get("endless") else dv.get("iasa_tick"),
            dv.get("first_hitbox_start_tick", dv.get("throw_start_ticks", ["-"])[0] if dv.get("throw_start_ticks") else "-"),
            dv.get("last_hitbox_end_tick"),
            len(hbs), hk.get("damage"), hk.get("damage_in_combo"),
            hk.get("hitstun_ticks"), hk.get("victim_hitlag"), hk.get("knockback"),
            hk.get("hit_height"), ", ".join(notes) if notes else None,
        ])) + " |")

lines.append("\n## Projectile and object scenes\n")
lines.append("| scene | gravity | max fall | states | hitboxes |")
lines.append("|---|---|---|---|---|")
for p in d["projectiles"]:
    if "error" in p:
        lines.append(f"| {p['scene']} | error | | | |")
        continue
    rp = p["root_properties"]
    hb = sum(len(s["hitboxes"]) for s in p["states"])
    lines.append(f"| {p['scene'].split('/')[-1]} | {fmt(rp.get('gravity'))} | {fmt(rp.get('max_fall_speed'))} | "
                 f"{len(p['states'])} | {hb} |")

with open(out, "w", encoding="utf-8") as f:
    f.write("\n".join(lines) + "\n")
print("wrote", out, len(lines), "lines")
