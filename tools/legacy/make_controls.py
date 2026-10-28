#!/usr/bin/env python3
"""Draft a control scheme for a converted character.

Usage:
  python3 -I tools/make_controls.py <extracted.json> <converted.ron> <character_id> <out_controls.ron>

Every move the player can start from neutral gets a button+direction slot; follow-ups that only
make sense as cancels go into `cancel_only` binds. The result is a starting point to edit by hand.
Buttons: Light = attacks, Special = specials, Heavy = supers (and overflow), Grab = grabs.
Moves the engine owns (dash, jump, block, dodge) and moves entered only by other moves'
scripts are not bound. Pure standard library.
"""
import json
import re
import sys

SRC, RON, CHAR_ID, OUT = sys.argv[1:5]

ENGINE_IDS = {"idle", "wait", "fall", "landing", "jump", "double_jump", "super_jump", "dash_forward",
              "dash_backward", "air_dash", "charge_dash", "charge_dash_forward", "dash_startup"}
SKIP_BUTTONS = {"Guard", "Jump", "Dash", "Taunt", "Burst"}
DIR_ORDER = ["Down", "Forward", "Up", "Back"]          # slots handed out after the neutral one
OUT_DIR_ORDER = ["Down", "Forward", "Up", "Back", "Any"]


def snake(name):
    s = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name)
    return re.sub(r"[^a-z0-9_]", "_", s.lower())


js = {}
for s in json.load(open(SRC))["states"]:
    js[{"Wait": "idle"}.get(s["name"], snake(s["name"]))] = s

moves = []
text = open(RON).read()
for part in re.split(r"\n {16}id: \"", text)[1:]:
    mid = part.split('"')[0]
    if not re.search(r"\n {16}name:", part[:300]):
        continue
    stance = re.search(r"\n {16}stance: (\w+)", part)
    inp = re.search(r"\n {16}input: Input\((.*?)\),\n", part)
    if not inp:
        continue
    button = re.search(r"button: (\w+)", inp.group(1)).group(1)
    d = re.search(r"direction: (\w+)", inp.group(1))
    moves.append({"id": mid, "name": re.search(r'name: "([^"]*)"', part).group(1),
                  "stance": stance.group(1) if stance else "Any", "button": button,
                  "dir": d.group(1) if d else None})

neutral, cancel, skipped = [], [], []
for m in moves:
    s = js.get(m["id"])
    if m["id"] in ENGINE_IDS or m["button"] in SKIP_BUTTONS or s is None:
        skipped.append((m["id"], "engine-owned or not a bindable button"))
        continue
    k = s["key"]
    frm = set(k.get("interrupt_from") or [])
    if k["type"] not in ("Attack", "Special", "Super"):
        skipped.append((m["id"], f"type {k['type']}"))
    elif any("Throw" in f for f in frm) or not k["selectable"] and not k["combo_only"] and not frm:
        skipped.append((m["id"], "throw choreography / entered by script"))
    elif k["combo_only"]:
        cancel.append(m)
    elif frm & {"Grounded", "Aerial"}:
        neutral.append(m)
    elif frm:
        cancel.append(m)
    else:
        skipped.append((m["id"], "entered only by another move's script"))

# ---- neutral slots
BUTTON = {"Light": "Light", "Special": "Special", "Super": "Heavy", "Grab": "Grab", "Heavy": "Heavy"}
stances = [("Ground", {"Ground", "Any"}), ("Air", {"Air", "Any"})]
binds = []          # (stance, button, dir, [ids], cancel_only)
unreached = []


def groups_for(stance_set, button):
    """Groups of same-titled moves, in file order."""
    gs, seen = [], {}
    for m in neutral:
        if BUTTON[m["button"]] != button or m["stance"] not in stance_set:
            continue
        key = (m["name"], m["dir"])
        if key not in seen:
            seen[key] = {"name": m["name"], "dir": m["dir"], "ids": []}
            gs.append(seen[key])
        seen[key]["ids"].append(m["id"])
    return gs


for stance, sset in stances:
    taken = {}      # (button, dir) -> ids
    pending = []
    for button in ("Light", "Special", "Heavy", "Grab"):
        gs = groups_for(sset, button)
        for g in list(gs):
            if g["dir"] in ("Forward", "Back", "Up", "Down"):
                taken.setdefault((button, g["dir"]), []).extend(g["ids"])
                gs.remove(g)
        free = [d for d in DIR_ORDER if (button, d) not in taken]
        for g in gs:
            if ("Any" not in [x[1] for x in taken if x[0] == button]):
                taken[(button, "Any")] = list(g["ids"])
            elif free:
                taken[(button, free.pop(0))] = list(g["ids"])
            else:
                pending.append((button, g))
    # overflow: free slots on Heavy, then Grab
    for button, g in pending:
        placed = False
        for b2 in ("Heavy", "Grab", "Special", "Light"):
            for d in ["Any"] + DIR_ORDER:
                if (b2, d) not in taken:
                    taken[(b2, d)] = list(g["ids"])
                    placed = True
                    break
            if placed:
                break
        if not placed:
            unreached.append((stance, g["name"], g["ids"]))
    for button in ("Light", "Heavy", "Special", "Grab"):
        for d in DIR_ORDER + ["Any"]:
            if (button, d) in taken:
                binds.append((stance, button, d, taken[(button, d)], False))

# ---- cancel-only binds: no slot limit, the cancel's tag filter picks the move
cb = {}
for m in cancel:
    for stance, sset in stances:
        if m["stance"] in sset:
            cb.setdefault((stance, BUTTON[m["button"]], m["dir"] or "Any"), []).append(m["id"])
for stance, _ in stances:
    for button in ("Light", "Heavy", "Special", "Grab"):
        for d in OUT_DIR_ORDER:
            if (stance, button, d) in cb:
                binds.append((stance, button, d, cb[(stance, button, d)], True))

# ---- write
names = {m["id"]: m["name"] for m in moves}
lines = [
    f"// Control scheme for the converted {CHAR_ID} (PRIVATE: names moves from the original game).",
    "// DRAFT generated by tools/make_controls.py: edit freely, press F5 in the game to reload.",
    "// Light = attacks, Special = specials, Heavy = supers (and overflow), Grab = grabs.",
    "// The first bind of a button+direction that has a usable move wins. Directions are relative to facing.",
    "// `cancel_only` binds are follow-ups reachable only through a cancel window.",
    "ControlScheme(",
    f'    character: "{CHAR_ID}",',
    "    binds: [",
]
for stance, button, d, ids, co in binds:
    parts = [f"button: {button}"]
    if d != "Any":
        parts.append(f"dir: {d}")
    parts.append(f"stance: {stance}")
    if co:
        parts.append("cancel_only: true")
    parts.append("moves: [" + ", ".join(f'"{i}"' for i in ids) + "]")
    lines.append(f"        Bind({', '.join(parts)}),  // " + ", ".join(names[i] for i in ids[:3]))
lines += ["    ],", ")", ""]
open(OUT, "w").write("\n".join(lines))

print(f"{CHAR_ID}: {len(neutral)} neutral moves, {len(cancel)} cancel-only, {len(skipped)} not bound, "
      f"{len(unreached)} without a free slot")
for u in unreached:
    print("  no slot:", u)
