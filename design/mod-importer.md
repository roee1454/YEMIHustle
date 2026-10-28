# Mod importer

Imports fighters from Steam Workshop mods of *Your Only Move Is HUSTLE* into this game, on the
player's own machine, from the Workshop files Steam already downloaded. Nothing from a mod or
from the original is committed to this repository or shipped with the game.

## 1. What a mod is (read from the real ones)

A Workshop item is a folder `steamapps/workshop/content/2212330/<id>/` holding
`workshop_data.json` (title, tags), a preview image and one of:

| item | what it is | what we do |
|---|---|---|
| `<Mod>.zip` with `<Mod>/_metadata`, `ModMain.gd` | a Godot resource pack | inspect; import if it adds fighters |
| `*.style` | a customization style (colors, outline) | later: import as a palette |
| zip without fighters (menus, options, style editor, the character loader itself) | patches the original's UI | skip, say so |

A fighter mod is regular: `ModMain.gd` calls `installScriptExtension("res://<Mod>/CharacterSelect.gd")`,
and that file calls `addCustomChar("<Name>", "res://<Mod>/.../<Fighter>.tscn")`. The fighter
scene inherits the base character scene of the original; its states are `.gd` files that extend
the original's classes (`CharacterState`, `ThrowState`, `ObjectState`, `BaseProjectile`...);
sprites are PNGs next to `.import` files (some mods ship the engine's own `.stex` / `.sample`).
`_metadata` is JSON that the game tolerates with trailing commas. `requires: ["char_loader"]`
is the only dependency seen, and it only exists to provide `addCustomChar`.

## 2. Findings from your eight items (`design/mod-scan-example.md`)

| item | kind | scripts | projectile scenes | size |
|---|---|---|---|---|
| One Punch Man | character | 7 | 0 | 10 MB |
| Gojo | character | 137 | 10 | 62 MB |
| Hakari | character | 50 | 1 | 18 MB |
| Heion (Sukuna) | character | 78 | 12 | 34 MB |
| char_loader, options menu, style editor | tools | - | - | skip |
| one `.style` file | style | - | - | later |

The result that shapes everything: **across the four fighters, the only call outside the game's
own scripting API is `get_node`** (scene-tree access). The mods are written against the same
`host.*`, `fixed.*` and `Utils.*` functions the official characters use (170 / 29 / 17 distinct
calls in 349 scripts). So the work is implementing the original's API once, faithfully; after
that, a mod is not new code, only more scripts to run. Mods also use sandbox-forbidden things
(`get_node`, `get_tree()`, `ResourceLoader`, `Steam`): 81 `get_node` and 24 `get_tree` calls
in Heion alone, mostly for effects and UI, which the interpreter answers with no-ops or refuses.

## 3. Pipeline

```
discover -> classify -> resolve -> extract -> convert -> tier -> register -> report
```

1. **Discover** (`modimport::scan_workshop`, done): Workshop folders, archives, `_metadata`,
   registrations, scripts, API use. The first-run wizard already finds the Workshop folder.
2. **Classify**: character / tool / style / unknown. Tools are listed and skipped.
3. **Resolve**: the mod's `res://` paths layer over the library's copy of the original
   (`<library>/game`); a path in the mod wins, anything else comes from the original. A mod
   with `overwrites: true` replaces original files and is refused for now (it would change the
   game for every character).
4. **Extract** into `<library>/mods/<id>/res/`: sprites and sounds decoded with the code the
   wizard uses for the original (`gamefiles`: `.stex`, `.sample`, ADPCM), `.import` remaps
   followed, `.tscn`/`.tres`/`.gd` copied as text. Guards: no path leaves the folder, size and
   entry-count limits, no archive inside an archive.
5. **Convert**: `yemi-convert` reads the fighter scene through the layered view and writes
   `characters/<mod id>_<fighter>.ron`, `_fx.ron`, `_controls.ron` and a report, exactly as for
   the official five. The converter's resolver takes a root list instead of one folder.
6. **Tier** every move and projectile (section 4).
7. **Register**: the roster is no longer a constant. `<library>/mods/index.json` lists imported
   fighters (id, name, author, mod id, archive hash, tier summary); the menu, select screen
   and `Paths` read it. An unchanged archive hash skips re-import.
8. **Report**: one page per mod (what imported, what fell back, what is missing), shown in the
   wizard and saved as `mods/<id>/report.md`.

## 4. Tiers

Each projectile and scripted move gets the first tier that works.

1. **Data.** Straight flight, friction, gravity, timers, hitboxes: read from the scene.
2. **Fitted.** Optional, needs the original game: the oracle (`tools/oracle`) loads the mod with
   `ProjectSettings.load_resource_pack`, records the move, and a fitter writes speed, push,
   friction and timed events as data. Local only; results are cached per archive hash.
3. **Interpreted.** The mod's own script runs in the `gdscript` crate against a Rust version of
   the original's object API. Covers homing, bounces, child objects, state machines.
4. **Unsupported.** The move or projectile falls back to a plain version or is disabled; the
   report names the missing function. The rest of the fighter still imports.

Tier 3 is the main path and the largest piece of work (section 5). Tiers 1 and 2 shorten the
common case and give independent checks.

## 5. The `gdscript` crate

A deterministic, sandboxed interpreter for the subset of GDScript the original and its mods use.

- **Language:** `extends`, `class_name`, `export var`, `onready` (resolved to nodes of the
  converted scene), consts, enums, funcs with defaults, `if/elif/else`, `for`, `while`, `match`,
  integers, floats, strings, arrays, dictionaries, Vector2, `.method` super calls (`._tick()`),
  `preload` of scenes already imported. No `yield`, threads or signals beyond the original's
  fixed set (`state_started`, `object_spawned`...).
- **Numbers:** `fixed.*` works on strings exactly as the native library does (the `sim`
  `Fixed` type already reproduces its rounding); GDScript ints stay ints.
- **API:** a `Host` trait implemented by the sim: the `host.*` calls of section 2 plus the
  state machine hooks (`_frame_N`, `_tick`, `_enter`, `_exit`, `is_usable`, `on_got_hit`...). Calls
  not implemented return a typed error that the report collects.
- **Safety:** no files, network, OS, Steam or scene-tree access; `get_node`/`get_tree` are
  answered by an allow-list (effects and UI become no-ops); every script run has a step budget
  so a loop cannot hang a match; recursion depth is capped.
- **Determinism:** no wall clock or global randomness; `randi_range` draws from the match RNG
  the sim already seeds. Same inputs give the same frames, which netcode will need.
- **Tests:** the oracle traces. Every official projectile that is hand-ported today
  (`crates/sim/src/projectiles.rs`) becomes a second implementation to compare, then the
  hand port is deleted. Mods add their own traces on machines that own both.

### Status of the interpreter (`crates/gdscript`)

Built: lexer with indentation, parser, evaluator, the `fixed` library (16.16 on decimal strings,
same rounding as `sim::Fixed`), a folder loader that resolves `extends` by path and by
`class_name` (mod folders before the game's), a step budget and a recursion limit, and a
`Host` trait for everything outside the script. The whole interpreter is `Send + Sync` so the
sim can keep it in a shared resource.

Measured on real files: 502 of 503 scripts of the original parse (the one failure is a
decompiler artifact, `Network.:`), and all 272 scripts of the four imported mods; 684 of 774
scripts build an object. The 90 that do not are root scripts of the engine itself: 63 need
`FGObject`, the native physics object the original's `BaseObj` wraps, 11 need the `Network`
singleton, the rest are editor tools.

That split is the architecture: **the sim implements the root objects natively** (the fighter
and projectile behind `host`, their forces, friction, hitboxes, state changes) and **the
interpreter runs the state scripts** that sit on top (`ObjectState`, `CharacterState` and the
mods' states). A real mod state script already runs end to end against a recording host
(`crates/gdscript/tests/real.rs`): One Punch Man's Normal Punch calls
`host.has_hyper_armor = true` on frame 1 unless the state saw `on_got_hit`, then
`start_projectile_invulnerability()`, and clears both on frame 48.

### The sim host (`crates/sim/src/script.rs`)

One host, written once, for every script: it is the engine API the original's base classes
offer (`StateInterface`, `ObjectState`, `CharacterState`, `BaseObj`, `Fighter`), not code for any
particular mod. The loader marks those five scripts native (`sim::NATIVE_SCRIPTS`); for an
object built on them, the interpreter sends their variables and methods to the host first and
runs the original base-class code only for what the host does not implement. A script's own
variables and methods always win, so a mod overriding `is_usable` or adding `host.Domain_Active`
works without anything mod-specific.

- **Data:** imported fighters are converted in *scripted* mode: a move keeps only what its scene
  says and carries `script: StateScript(path, node, vars, data, stick)`: the state script, the
  scene's values for its variables, the default `data` its option widgets give (same shape as
  the original's XY plot, 8-way, slider, count, check and option widgets, inherited UI scenes
  included) and where the stick's direction goes in it. The fighter carries its root script the
  same way. Official fighters keep their oracle-verified data and run no scripts.
- **Hooks, in the original's order:** `_enter` on start (with `data` set from the stick);
  each tick `_frame_N_shared` and `_frame_N` before the move's physics and `_tick` after;
  `_exit` however the move ends (and the invulnerabilities the original's `_exit_shared` ends).
  `is_usable` is asked before a move starts, `on_got_hit` / `_on_hit_something` /
  `on_got_blocked` follow contact. A returned state name or `queue_state_change` switches state;
  `anim_length` and `endless` are read back from the script, so scripts can change a move's length.
- **Fighter API implemented:** position, velocity and forces in the original's string fixed point
  (`get_pos`, `get_vel`, `set_vel`, `apply_force(_relative)`, `apply_forces(_no_limit)`,
  `apply_grav(_custom)`, frictions, `limit_x_speed`, `move_directly(_relative)`,
  `reset_momentum`), facing, grounding, `opponent`, `hp`, `combo_count`, invulnerability /
  projectile / throw invulnerability, `has_hyper_armor`, `has_projectile_armor`, hitlag,
  `change_state`, `current_state()` (engine-run states answer to their original names, e.g.
  `HurtAerial`), `state_machine.state`, `obj_local_center`, `distance_to`, `xy_to_dir`, stances.
  Sounds, particles, camera and after-images are accepted and ignored. Properties scripts set that
  the sim has no field for are kept per fighter.
- **Hit rules:** hyper armor takes the damage and hitlag without interrupting; projectile armor
  does so for projectiles; the invulnerability flags skip contact.
- **Stand-ins:** nodes the sim does not model (`$Sprite`, `get_parent()`) answer with an object
  that takes any assignment or call; each lookup is recorded as a gap.
- **Gaps are counted:** every unsupported call and script error is tallied per script and hook
  (`Scripts::problems`), which is what an import report will list.

### Scripted objects and scene nodes (`crates/sim/src/objects.rs`)

Mods spawn scenes of their own (Heion's Malevolent Shrine, Gojo's Red / Blue / Purple) and put
nodes on screen (cut-ins on `/root/Main/UILayer`). Done once for every mod:

- **Import:** every `res://` scene a mod's scripts name under its own folder becomes a `SceneDef`
  (node tree, props such as `z_index`, sprite animations from `SpriteFrames`). An object scene's
  states become `ObjectState`s with their scripts and hitboxes.
- **Objects:** `spawn_object` / `instance()` + `add_child` make a scripted object with its own
  state machine, run like the original's `BaseObj.tick`: `_enter`, `_frame_N`, `_tick`, timed and
  forced exits, `fallback_state`, physics flags, sprite animation, and hits from its state's
  hitboxes. A root script's own `tick` wins over the native one.
- **Nodes:** `$Path`, `%Name`, `..`, `get_node`, `duplicate`, `play`, `show` / `hide`,
  `queue_free`, `connect("animation_finished", ...)` (sent on each loop, as Godot does) run on a
  node table. The app draws object nodes in the world (ancestor positions and z summed) and UI
  layer nodes over the screen.
- **Effects and sounds:** `spawn_particle_effect(_relative)` and `play_sound` become sim events
  the app plays.
- **Hitting objects:** as in the original's `apply_hitboxes`, the opponent's attacks reach an
  object that `can_be_hit_by_melee` (its owner's too when it `damages_own_team`), and the
  opponent's projectiles and objects reach one that is not `projectile_immune`. The hurtbox is
  the scene's `Hurtbox` node as scripts move it. The object's `hit_by(hitbox)` runs (its script's,
  else the engine's: hitlag both ways, a free cancel), with `damage`, `damage_in_combo`, `host`,
  `misc_data`... So Heion's domain loses its lifetime to hits and shatters.
- **The fighter's own nodes:** `host.sprite.frame` / `rotation` / `offset` / `modulate` /
  `visible` (and `%Flip.visible`) change how the fighter is drawn; a frame a script picks (its
  state's `update_sprite_frame`, run each tick) lasts that tick. Other nodes of the fighter's
  scene that scripts show (marks, slashes) are drawn at the fighter.
- **The "Start" state:** the battle intro plays it whole (animation, sounds, effects, the
  cut-in's setup) with no hits and the clock stopped, then puts the fighters back on their
  marks. A round without the intro runs only its setup hooks.

`cargo test --release -p sim --test scripts soak -- --ignored --nocapture` (6000 ticks of
scrambled inputs per fighter) runs all four imported mods without script errors; what is left are
lookups of nodes the sim stands in for (`Main`, `HudLayer`, scenes instanced by hand such as
Heion's HUD). `domain_breaks` has the ninja shatter Heion's domain.

Not done yet: the HUD scenes mods add by hand, and objects' own after-images. Then an import
report section from `problems`, and moving the official characters' hand-ported behaviors onto
their own scripts with the oracle traces as the check.

## 6. Characters need more than projectiles

States run scripts too (One Punch Man's five scripts set armor; Gojo has 111 state scripts).
The sim must learn the hooks scripts rely on (hyper armor, projectile invulnerability, gauges,
super meter, mode switches). Each becomes part of `charfmt` data where the original treats it as
data, and a `Host` call where it is logic. Start with what One Punch Man needs, then Hakari.

## 7. UI and storage

- Wizard step after the game build: "Workshop mods": list from `scan_workshop`, tick fighters,
  import, show the report; reachable again from Options > Mods ("re-import", "remove").
- Library: `<library>/mods/<id>/{res/, report.md}`, `<library>/mods/index.json`,
  `<library>/characters/` gets the converted files. Removing a mod deletes its folder only.
- The mod's author and Workshop link are shown on its select-screen card.

## 8. Legal and safety posture

- Read only the user's own Workshop downloads; convert locally; never upload or bundle results.
- Credit the author in the UI; link the Workshop page; if `_metadata` states a license, show it.
- Mods are untrusted: nothing in them executes natively; all scripts run in the sandbox; archives
  are size-limited and path-checked; a mod that fails never blocks the game.
- A mod can be removed at any time and nothing else changes.

## 9. Milestones

| step | outcome |
|---|---|
| M1 scan (done) | `yemi-modscan` lists mods, fighters, API use; this document |
| M2 assets + convert, no scripts | One Punch Man imports end to end and is selectable (needs the layered resolver and extraction) |
| M3 `gdscript` core | runs the official five characters' projectile scripts and matches the oracle traces |
| M4 character hooks | One Punch Man's armor, then Hakari, Heion, Gojo state scripts |
| M5 wizard and menu | import UI, report page, roster from `index.json` |
| M6 fitting | optional tier 2 with the oracle loading mods |

## 10. Risks and open questions

- **Size of the API:** 170 host calls are only the ones used; each needs the original's exact
  behavior. The oracle is how we keep it honest, call by call.
- **Domain-style mechanics** (Heion's `Domain`, Gojo's gauges) may need whole-match rules, not
  per-fighter data. Plan to flag them in the report until a rule exists.
- **Compiled scripts** (`.gdc`) in a mod would need the decompiler; none seen yet.
- **Several fighters in one mod, or a mod that edits the original's states:** refused at first.
- **The original updating:** the importer keys on the archive hash and the library's build id,
  and re-runs when either changes.
