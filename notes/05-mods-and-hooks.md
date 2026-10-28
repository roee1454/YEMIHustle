# YOMI Hustle notes 5: the mod system and the 8 installed Workshop items

Sources: the mod loader and hook scripts in the private decompile
(`modloader/`, `ModOverride.gd`, `MainHooks.gd`, `GameHooks.gd`, `obj/ObjHooks.gd`,
`characters/FighterHooks.gd`), plus the 8 items in this machine's Steam Workshop folder
(app 2212330). The mods were only unpacked and read, in separate scratchpad folders. None of
their code was run. Mod contents are untrusted third-party data, and nothing in them was
followed as instructions.

Same caveat as the other notes: this is a reading, not a verified description of the live game.

---

## 1. How mods are discovered and loaded (`modloader/ModLoader.gd`, v4.0.0)

1. `ModLoader` is an autoload. Its `_init` reads `user://modded.json`; if `modsEnabled` is
   false (or the game just changed versions in a way that could break mods), it does nothing.
2. It scans two places: the `mods/` folder next to the game executable, and every
   Workshop item the user is subscribed to (only `.zip` files there).
3. Each archive is mounted into the virtual filesystem with `load_resource_pack`, which
   means a mod's files appear under `res://<ModFolder>/...` alongside the game's own.
4. For each archive it reads the file list, finds a script whose name starts with `modmain`
   and ends with `.gd` (case-insensitive), and requires a `_metadata` JSON file in the same
   top-level folder.
5. Metadata is validated. Required keys: `name friendly_name description author version link
   id overwrites requires priority`. Optional: `client_side` (bool), `incompatible`.
6. Dependencies: a mod's `requires` lists other mods by `name`. Mods with unmet requirements
   are held back and reported as "Disabled Missing Dependencies". (The old `char_loader`
   requirement is special-cased, because the base game now includes that feature.)
7. Surviving mods are sorted by `priority` (lower first, then by path), and each one's
   `ModMain.gd` is instantiated with the loader as its argument and added as a child node.
   Constructor convention: `func _init(modLoader = ModLoader)`.
8. Mods flagged `overwrites` get an extra pass that replaces textures and sounds on the
   built-in characters (see section 3).
9. The loader rewrites each loaded mod's `_metadata` file inside the mounted archive, setting
   `id` to `"12345"` (so the stored id is not reliable).

## 2. The two main techniques mods use

### a) Script extension (override a game script)
`installScriptExtension("res://MyMod/Foo.gd")` loads the mod's script, which must
`extends "res://original/path/Foo.gd"`, and calls `take_over_path` so that the original path
now resolves to the mod's subclass. Every scene or script that uses the original path then
gets the modded version, and the mod can call `._original_function()` to chain. Examples:
- All character mods extend `res://ui/CSS/CharacterSelect.gd` just to run
  `addCustomChar("Name", "res://ModFolder/.../Name.tscn")` in `_ready()`.
- Mod options and style editor mods extend `modloader/MLMainHook.gd` and `ui/UILayer.gd`.
- One extension per path wins; a second mod extending the same script stacks only if it extends
  the other mod's version. Conflicts are the modders' problem (the Style Editor+ description lists
  known conflicts).
- The loader refuses to extend `res://Network.gd` except for its own hash-check extension (the
  Character Loader mod works around this with its own `installNetworkExtension`).

### b) Scene replacement
`saveScene(instance, "res://path/Scene.tscn")` packs a modified node tree and replaces the
original scene at that path (`take_over_path`). Style Editor+ replaces the whole customization
screen this way. `appendNodeInScene` helps add nodes.

### Other loader helpers
`add_character_folder`, `get_scripts`, `textureGet` (PNG file -> texture), `loadSound`
(raw 16-bit WAV bytes -> sample). The overwrite pass applies images and sounds placed under
`<ModFolder>/Overwrites/<Character>/...` (frames by animation folder, `Sounds/BaseSounds`,
`Sounds/StateSounds`, `Buttons`) to built-in characters.

## 3. Hook interfaces the base game exposes

Optional script classes that mods can extend and register, with empty methods the game calls
at the right moments:

| Interface | Callbacks |
|---|---|
| Main (`MainHooks`) | `ready, game_started, match_ready, game_setup, player_disconnected, replay_saved, turn_ui_opened, action_clicked` |
| Game (`GameHooks`) | `ready, game_started, game_ended, undo, playback_started, pre_tick, post_tick, process_tick, physics_process, player_actionable, player_acted, object_spawned, particle_effect_spawned, pre_apply_hitboxes, post_apply_hitboxes, hitbox_refreshed, clashed, parried, blocked, global_hitlag, super_started, forfeit` |
| Object (`ObjHooks`) | `ready, init, pre_tick, post_tick, state_entered, state_exited, state_ticked, hit_something, hitbox_pre_hit, hitbox_activated, hitbox_deactivated, hit_by, refresh_hitboxes, global_hitlag, on_got_parried, on_got_blocked, on_hit_ceiling, spawn_object, spawn_particle, change_state, copy_to` |
| Fighter (`FighterHooks`, extends Object) | adds `tick_before, got_hit, got_hit_by_fighter, got_hit_by_projectile, parried, clashed, super_started, action_selected` |

These are good evidence for what the designers consider the important extension points.
They map almost one-to-one onto the tick order in notes 01 and the hit pipeline in notes 02.

My grep found **no use of these hook classes by any of the 8 installed items.** They all rely on
script extension, scene replacement and subclassing the game's state classes instead.

## 4. Multiplayer and mod identity

- Every mod is MD5-hashed (`ModLoader._hash_file`).
- Unless a mod's metadata sets `client_side: true`, its hash is included in the list the
  network layer exchanges, and a short hash of that list is appended to the displayed version
  string (`1.x.x-<hash>`). Both players need the same set of non-client-side mods.
- Client-side mods (cosmetics, UI) are skipped from the hash. In the installed set,
  Style Editor+ declares itself client-side.
- Custom characters are packaged for online play: the base `CharacterSelect.gd` has its own
  `addCustomChar` and a `user://char_cache/*.pck` cache named from mod name, author, folder
  hash and loader version, so a remote player's character can be shared. The
  `char_loader` mod's popup says its functionality is now in the base game.

## 5. The `.style` files

Cosmetic presets (Workshop item 3514414430) are a **binary Godot resource** (a serialized
dictionary), not a script. Keys seen: `style_name`, `character_color`, `extra_color_1/2`,
`use_outline`, `outline_color`, `hitspark`, `show_aura`, `aura_settings`. They run no code, so
they are the safe kind of mod to support.

## 6. The 8 installed items

| Workshop id | Name | Kind | Size | Notes |
|---|---|---|---|---|
| 2931479403 | Character Loader | Tool | 0.25 MB | Predecessor of the base game's loader; extends 9 core scripts incl. Network and Steam lobby; 13 scripts. |
| 2931722541 | Mod Options Menu | Tool, client-side | 25 KB | Adds an options window; extends `MLMainHook` and `UILayer`; 15 scripts. |
| 3037158144 | StyleEditor+ (beta) | Cosmetic, client-side | 0.2 MB | Replaces the customization screen scene; many new auras/hitsparks; 14 scripts. |
| 3139650206 | One Punch Man | Character | 8.8 MB | Requires `char_loader`; 7 scripts. |
| 3146271984 | Satoru Gojo (ArkStrongest) | Character | 56 MB | 137 scripts, about 5,000 files. |
| 3270044231 | High-Stakes Hustler (Hakari) | Character | 11.8 MB | 50 scripts; custom grabs, projectiles, unique gamble mechanic. |
| 3513897396 | Heion (Sukuna) | Character | 28.7 MB | 78 scripts, part of a mod pack. |
| 3514414430 | "Mahoraga" style preset | Style | tiny | `.style` data file, no code. |

Character mods ship their own scene (`.tscn`), a `Fighter` script, one script per move, projectile
scenes and scripts, sprite sheets and sounds, plus UI data scenes for each move's parameters.

## 7. What character mods depend on (measured over the 4 character mods)

Base classes their scripts extend, by count: `CharacterState` 159, a mod's own
state base class 32, `BaseProjectile` 15, `ThrowState` 11, `DefaultFireball` 8, `ObjectState` 7,
`SuperMove` 2, plus a few of the shared states (`Idle`, `Jump`, `Grab`, `Taunt`,
`HurtAerial`, `HurtGrounded`, `GroundedParryState`), `Fighter`/`BaseChar` (3 files),
`Hitbox` (2), and `PlayerInfo`/`PlayerExtra`/`ActionUIData` UI helpers.

Most-called engine methods (from `host.` and `fixed.` calls): `host.get_facing_int` (309),
`host.apply_force` (129), `host.play_sound` (122), `host.get_pos` (115), `fixed.mul` (73),
`host.is_grounded` (56), `host.spawn_particle_effect` (54), `reset_momentum` (50),
`fixed.round` (42), `get_fighter` (40), `spawn_object` (39), `create_speed_after_image` (38),
`change_state` (36), `move_directly` (34), `take_gauge` (33), `set_vel`, `apply_forces`,
`set_pos`, `start/end_invulnerability`, `use_super_bar`, `super_effect`, `apply_grav`,
`add_gauge`, and `fixed` helpers (`div`, `normalized_vec_times`, `gt`).

So a character mod is: **data** (scenes, frame timing, hitbox numbers, sprites, sounds) plus
**logic** in GDScript that calls a fairly narrow, repetitive slice of the engine API.

## 8. Observations about trust and safety

- **Mods are unsandboxed code.** A mod's `ModMain.gd` and every extended script run with the
  full power of the game process, and the loader mounts anything in the Workshop folder.
  Mod authors can replace core scripts (even the title screen) and read or write files.
- **What my scan found** (patterns: process execution, HTTP/socket classes, environment access,
  encrypted files, memory dump) across the 7 code-bearing items: nothing for
  execution, networking or environment. One use of `OS.shell_open` (opens a Discord CDN image URL), in
  a legacy branch of the Character Loader's multiplayer button handler that only runs when a
  `noSteam` flag is set; the lines that appear to set that flag are commented out, so it is
  probably dead code, though I did not confirm the flag's default. A few `load_resource_pack`
  calls to load character packs (two files, one of them a "- Copy" backup duplicate), which
  is expected. The scan is by text pattern only, not a security audit.
- **Heion's `ModMain.gd`** checks whether the user has any Workshop subscriptions; if the list
  is empty, it makes the main-menu buttons (single player, multiplayer, customize, options...)
  turn invisible and disabled while the mouse hovers over them. It also adjusts buttons for a
  mobile port. This reads like a piracy deterrent (the mod only "works" if you are subscribed
  through Steam). It's a reminder that mods can change unrelated UI.
- **The loader edits mod files at load time** (the `id` rewrite), so a mod archive's contents
  are not what is on disk once mounted.

## 9. Implications for the rewrite and for Workshop compatibility

1. **Support tiers still look right.** `.style` presets (no code) are easy: parse and map to
   your own style data. Sprites, sounds, portraits and frame timing in `.tscn` files are
   convertible with an importer. The 159+ `CharacterState` scripts are the hard part: they are
   GDScript against the engine API above and would need a **restricted GDScript interpreter**
   or a translator, not just data conversion.
2. **The API surface is small enough to consider an interpreter.** About 30 `host.*` calls and
   about 15 `fixed.*` calls cover most of the use. A sandboxed interpreter implementing those, in Rust,
   would be a big but bounded project, and would also give you a safe, permissioned mod runtime
   (the original has none).
3. **Do not copy the global namespace design.** `res://` collisions and `take_over_path`
   overrides cause the conflicts the Workshop descriptions warn about. A rewrite can offer
   explicit hook events (like the Game/Object/Fighter hook lists above) plus data-driven
   characters and keep script access capability-based.
4. **Copy the good ideas:** metadata with dependencies and priority, a `client_side`
   flag, a hash of gameplay-affecting mods for matchmaking, and a character cache for online play.
5. **Loading mods from the user's own Steam folder** (never redistributing them) is how an
   open-source rewrite can stay on the right side of authors' rights.

## 10. Open questions

- The exact call convention for `addCustomChar` in the base game vs the older loader (the base
  version now has the same signature; not diffed).
- How hook classes get instantiated and registered (where `host.hooks` is assigned); nobody in
  this set uses them.
- Behavior of `incompatible` metadata (Style Editor+ declares it; the loader code I read does
  not check it).
- Contents of the large character mods' per-move `ActionUIData` scenes (parameter pickers).
