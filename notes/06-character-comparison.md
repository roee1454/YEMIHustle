# YOMI Hustle notes 6: comparing the five base characters

Source: `data/{ninja,robo,wizard,mutant,cowboy}.json`, produced by `tools/extract_character.py`
from the private decompile. Scene names: stickman/Ninja (`NinjaGuy.tscn`), robo (`Robot.tscn`),
wizard (`Wizard.tscn`), mutant/"Beast" (`Mutant.tscn`), swordandgun/"Cowboy" (`SwordGuy.tscn`).
All five extracted without errors (a handful of exotic values stay as raw text).
Numbers below are counts and medians computed from the extracted data, not hand-checked in game.

---

## 1. Shape of a character

Every character is the same recipe:

1. **A scene that inherits `BaseChar.tscn`.** The base provides about 40 shared states
   (idle, jump, dash, hurt states, parry, burst, grab and throw machinery, taunt...). The
   character scene overrides some of them and adds its own.
2. **A root fighter script** extending `BaseChar.gd` that holds the character's unique mechanic.
3. **One script per move**, mostly small, extending `CharacterState` (or `ThrowState`,
   `SuperMove`, ...). The scene supplies the numbers (timing, hitboxes); the script adds the behavior.
4. **Projectile scenes** with their own states and hitboxes.
5. **A player-info scene and an "extra" scene** for the character's unique meter/UI, and
   **ActionUIData scenes** describing each move's parameters (direction pad, option buttons).

The extractor handled this uniformly, so one data format can describe all five.

## 2. Size and composition

| | ninja | robo | wizard | mutant | cowboy |
|---|---|---|---|---|---|
| states total | 112 | 119 | 95 | 98 | 127 |
| defined by the character | 54 | 61 | 37 | 40 | 69 |
| overriding a base state | 18 | 17 | 21 | 21 | 17 |
| inherited unchanged | 40 | 41 | 37 | 37 | 41 |
| states with hitboxes | 49 | 49 | 28 | 38 | 46 |
| hitboxes in states | 102 | 88 | 44 | 80 | 93 |
| projectile scenes | 16 | 14 | 17 | 17 | 17 |
| hitboxes in projectiles | 22 | 10 | 10 | 15 | 21 |
| distinct state scripts | 36 | 49 | 38 | 41 | 41 |
| non-blank lines in them | 1,661 | 1,309 | 1,328 | 1,068 | 2,112 |
| root script lines | 232 | 509 | 349 | 227 | 331 |

Menu moves by category (character-specific, selectable):

| | ninja | robo | wizard | mutant | cowboy |
|---|---|---|---|---|---|
| Attack | 26 | 23 | 19 | 19 | 22 |
| Special | 17 | 15 | 13 | 16 | 23 |
| Super | 6 | 6 | 8 | 6 | 5 |
| Defense | 1 | 2 | 4 | 2 | 3 |

Take-aways:
- The inherited base is large: 37-41 states (32-39% of each character) come for free.
- Total content is roughly 7,500 lines of move scripts and 1,650 lines of root scripts across
  the five characters. That is the scale of work if you reimplement them by hand.
- The wizard is projectile-centric: the fewest in-state hitboxes (44) and the most projectile
  scenes. The cowboy has the most script (2,112 lines in moves) and the most states.

## 3. What every character shares (the stable skeleton)

States that **all five** define on their own: `ForwardThrow, BackThrow, AirForwardThrow,
AirBackThrow, AirDownThrow, DashForward, DoubleJump, Taunt`. Four or more share `Jump,
SuperJump, AirDash, DashBackward, ChargeDashForward`; three share `Fall, AirGrab, Roll,
AirKick, SuperFallStartup`.

So the common move vocabulary is: **ground dash both ways, charge dash, jump, double jump, air
dash, super jump, grab, four throws plus an air-down throw, kick-type attacks, taunt.**
Every character also overrides these base states: `AirBackThrow, AirDownThrow,
AirForwardThrow, BackThrow, DashForward, DoubleJump, ForwardThrow, Taunt`. Throws and dashes
are always character-tuned; idle, hurt, parry and burst mostly come from the base unchanged.

Constants shared by all five: 1500 max health (no character overrides `MAX_HEALTH`), 2 feints,
the same damage and hit pipeline from notes 02-03, and an identical set of root properties
(`gravity`, frictions, speed caps, air movements, damage/knockback taken modifiers).

## 4. What differs: movement and body

| root attribute | ninja | robo | wizard | mutant | cowboy |
|---|---|---|---|---|---|
| gravity | 0.5 | 0.35 | 0.43 | 0.65 | 0.4 |
| ground friction | 2.5 | 1.7 | 0.75 | 2.0 | 1.5 |
| air friction | 0.2 | 0.2 | 0.1 | 0.1 | 0.1 |
| max ground speed | 20 | 9 | 9 | 16 | 12 |
| max air speed | 10 | 11 | 8 | 14 | 10 |
| max fall speed | 15 | 20 | 6.5 | 11 | 10 |
| air movements | 2 | 0 | 2 | 3 | 2 |
| damage taken x | 1.0 | 0.90 | 1.0 | 1.1 | 1.0 |
| knockback taken x | 1.0 | 0.80 | 1.0 | 1.0 | 1.0 |

(These resolve each root property over its script default; a few may be overridden in code at
runtime.) Reading the table: the ninja is the fast ground mover, the wizard floats (low fall
speed, low friction), the mutant is heavy and fast in the air with an extra air movement, the
robo is armored (takes less damage and knockback) and trades air movements for flight.

## 5. What differs: frame data and damage

Over each character's own hitboxes:

| | ninja | robo | wizard | mutant | cowboy |
|---|---|---|---|---|---|
| median damage | 100 | 60 | 85 | 60 | 128 |
| max damage | 200 | 400 | 170 | 145 | 250 |
| median startup (ticks) | 7 | 6 | 7 | 6 | 8 |
| slowest first-hit startup | 16 | 18 | 32 | 14 | 28 |

- Typical startup is 6-8 ticks everywhere, so basic frame data is homogeneous.
- The robo has the lowest median damage but the highest single hit (400). The cowboy hits
  hardest on average. The wizard has the slowest moves (32 ticks), consistent with zoning.
- Cancel and recovery design is the same system in all five (`iasa_at`, `interrupt_frames`,
  hit-cancel lists), only the numbers differ.

## 6. How much of each character is script-driven

Counted over character-defined plus overridden states (72, 78, 58, 61 and 86 states respectively):

| | ninja | robo | wizard | mutant | cowboy |
|---|---|---|---|---|---|
| move the fighter in script | 57 | 36 | 19 | 36 | 55 |
| edit hitbox fields in script | 4 | 4 | 4 | 2 | 9 |
| endless states | 7 | 4 | 1 | 5 | 6 |
| have per-tick (`_frame_N`) hooks | 50 | 58 | 42 | 42 | 73 |
| have player-chosen parameters | 24 | 16 | 17 | 20 | 29 |

(The "move" count is generous: it counts any state whose script calls a force/velocity function.)
So the **scene data alone is not enough** for any of them. Roughly 79% (ninja), 64% (cowboy)
and 59% (mutant) of those states move the fighter from script, against 46% (robo) and 33%
(wizard); almost every character-defined state also has per-tick hooks.
The parameter counts show how many moves ask the player for a choice (direction pad,
attack option...), which matters for any real-time input mapping.

## 7. Unique mechanics (inferred from root-script constant names only)

I did not read these scripts in depth. This is a guess from constant names, to be verified:

| character | likely unique mechanic |
|---|---|
| ninja | grappling hook (pull speeds, disable distance), stored/released momentum, uppercut hitstun cancel |
| robo | armor "pips", flight (fly speed and force, forward/back caps), magnet bombs |
| wizard | hover (hover amount/min, vertical modifiers), orbs that drain super meter |
| mutant | "juke" pips (per-use cost, durations per direction), a timed install state |
| cowboy | gun/boomerang pickup (distances, barrel offset), foresight after-image and rift projectiles |

Each has a dedicated "extra" scene (a unique meter or counter UI) that the base game's player
panel hosts.

## 8. Implications for the rewrite

1. **Use a base plus per-character data layout,** mirroring the original: a shared state
   library (~40 states) and per-character overrides and additions. It avoids duplicating
   idle/hurt/parry/etc. for every character.
2. **The data schema from the extractor generalizes.** One schema covers all five, so a
   single importer/exporter tool is enough, and character files can be authored or generated.
3. **Plan a "behavior module" layer.** Each character's unique mechanic and the 50-70
   script-driven states per character need code. Realistic options: hand-write them in Rust
   as small state behaviors, or build a scripting layer (see notes 05 on a restricted
   interpreter). Hand-writing 7,500+ script lines times a real-time redesign is the largest
   cost in the project.
4. **Start with the ninja as the vertical slice,** as planned: it has the most data-driven
   states (49 with hitboxes), a moderate script footprint (1,661 lines) and a simple
   but varied toolkit (projectile, grab, hook, dive). The mutant has the least script (1,068 lines)
   and could be a second, cheaper test of the system.
5. **Tuning data is mostly numeric,** so real-time retuning (shorter recoveries, new cancel
   windows) can be done as data passes over these JSON files, plus script edits where states
   move the fighter.
6. **Movement attributes are character-defining** (speed caps, gravity, friction). They are
   five numbers per character and should be first-class, easily tunable data.

## 9. Open questions

- The exact unique mechanics above (read the root scripts to confirm).
- Meaning of the many `_frame_N` hooks (what logic runs at those ticks).
- Whether the "alien" folder (2 files) is a cut character or a leftover.
- Per-character differences in the extra scene meters (not extracted).
- Raw leftover values (about 100 across all characters) that the extractor could not type.
