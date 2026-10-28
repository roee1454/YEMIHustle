# Character data format (draft v1)

A RON format for characters and moves in a real-time fighting game. The schema is the Rust
code in `crates/charfmt/src/lib.rs`; this document explains the decisions. A public example
with invented numbers is `crates/charfmt/examples/example_character.ron`.

Status: draft, validated by the crate's tests (parse, strictness, validation rules). Not yet
used by a simulation.

## 1. Goals

1. **Data first.** Frame data, boxes, cancels, hit effects and body physics live in files that
   can be tuned without recompiling.
2. **Deterministic.** Gameplay numbers are converted to fixed-point once, at load. The simulation
   never sees a float.
3. **Honest about scripts.** Logic the data cannot express goes behind a named Rust behavior
   (`behavior`, `mechanic`) instead of growing a scripting language inside RON.
4. **Strict.** Unknown fields are errors, ids are checked, and windows must fit the timeline, so
   typos fail at load time and not in the middle of a match.
5. **Importable.** The model is close enough to the extracted turn-based data that a converter
   can fill most of it in mechanically, and it records what it had to guess.

## 2. Conventions

| Topic | Rule |
|---|---|
| Time | One tick = 1/60 s. Frame numbers are 1-based; spans are inclusive (`Span(from: 5, to: 6)` = 2 frames). |
| Numbers | `Fx` values are written as plain numbers (`3`, `3.0`, `-0.5`) and converted to fixed-point at load. |
| Space | Origin at the feet, +x in the facing direction, +y down. Boxes are `(x, y)` plus half-extents. |
| Ids | Lowercase snake_case: `grounded_punch`. Moves are unique per character. |
| Defaults | Every optional field has a default; a file states only what differs. |
| Files | Start with `#![enable(implicit_some)]` so optional values are written bare. |

## 3. Shape of a file

```
CharacterFile
  format_version
  Character
    id, name, base_library, mechanic?, modes[]
    Body          health, gravity, frictions, speed caps, air_actions, collision_box, hurtbox
    Move[]        see below
    Projectile[]  velocity, force, friction, gravity, lifetime, hitboxes, events, zone?, on_disable?, pierces, behavior?
    Provenance?
```

A **Move** has:

| Field | Meaning |
|---|---|
| `id`, `name`, `tags` | Identity and group membership. Tags are what cancels target. |
| `stance` | `Ground`, `Air` or `Any` (where the body is: not the character mode). |
| `input` | Optional. Button, direction, buffer window and hold. `None` for moves only other moves or the engine enter (throw follow-ups, landings). |
| `params` | Choices the move takes when it starts: a direction (free, limited to an arc, or eight-way with some directions disabled), a slider, a toggle, an option list or a count. Each has a default and an optional `binding` (stick, hold time, or an extra button). An unbound parameter uses its default. |
| `availability` | `combo_only` / `neutral_only`. |
| `modes`, `set_mode` | Character modes (stances) in which the move works, and the mode it switches to. Modes are declared once on the character. |
| `timeline` | `startup` / `active` / `recovery` frame counts; `end` is `Fixed`, `OnLanding` or `Script`. With an open end the listed frames only describe the opening of the move. |
| `hitboxes` | Boxes with their own frame windows inside the active span, plus all hit data. |
| `throw` | Choreography of a move that holds the opponent: a `hold` offset, animation-driven `keyframes`, and a `release` with its own launch data. |
| `hurt` | Temporary replacements for the body hurtbox over a frame span (crouching, leaning). |
| `physics` | Which physics run each tick, plus per-move overrides: gravity and fall cap, horizontal speed caps, extra x / y friction coefficients, uncapped forces. |
| `motion` | Timed impulses and velocity sets (forward / up, relative to facing), and stops (all, horizontal only, vertical only). |
| `spawns` | Timed projectile spawns. |
| `defense` | Windows of invulnerability or armor. |
| `cancels` | Early exits: target tags, trigger (hit, block, whiff, always) and window. |
| `finish_into` | Where the move goes when the timeline ends (default `idle`). |
| `cost` | Super bars and air actions. |
| `behavior` | Optional id of a Rust behavior for logic the data cannot express. |
| `presentation` | Logical animation name and sound cues. Art and audio are not part of the format. |
| `provenance` | Source and open assumptions. Optional. |

A **HitBox** carries: `rect`, `window`, `damage` (and in-combo damage), `hitstun`, `hitlag`
(attacker and victim), `knockback` (direction and strength), `pushback`, `height`
(High/Mid/Low), `block_advantage`, `scaling` (proration, which may be negative; whether the hit
counts toward the combo), `effects` (knockdown and whether the stun lasts until landing, ground
bounce, air-to-ground bounce, wall slam, guard break, throw), `reaction` (what the hit does to the
victim) and `flags` (clash, beats throw, hits projectiles/grounded/air/downed). Three more fields
shape the hit itself: `kind` (normal, throw damage, burst, flip, no-hitstun...), `min_damage`, and
on `knockback` a `reversible` flag and a `from` mode (direction, toward the hitbox, away from it).
Four fields cover multi-part moves: `until_end` (live until the
move ends), `repeat` (on/off pulses for sweeps), `followup` (move entered when it hits) and
`throw_into` (move entered when a grab connects).

**Reactions.** `reaction` has a state for a grounded victim (`Stumble`, `Launch`, `Held`, `Dizzy`),
one for an airborne victim, and `min_launch_frames`. The defaults are `Stumble` on the ground and
`Launch` in the air, so most hits state nothing; grounded-launcher hits (the common override) write
`reaction: Reaction(grounded: Launch)`.

**Throws.** A throw is three parts: the grab hitbox (`effects.throw`, `throw_into`), the move that
holds the opponent (`throw`: `hold`, `keyframes`, `release`), and the release launch
(`ReleaseHit`: damage, hitstun, knockback, scaling, effects, reaction, DI strength). A
`ThrowDamage` hitbox inside the holding move deals damage while the opponent is held.

**Physics semantics** (measured on the original library, see `notes/04`): `friction` multiplies
horizontal speed by `1 - friction / matching_max_speed` (ground or air); `friction_x` /
`friction_y` multiply by `1 - value`, so a value above 1 reverses the speed; gravity adds its step
while vertical speed is below the fall cap and never slows a faster fall; `forces` adds queued
forces and caps horizontal speed and fall speed unless `uncapped_forces` is set.

Two data rules the validator enforces: a hitbox with a zero-size half extent is a placeholder and
needs a `behavior` that resizes it, and every `followup` / `throw_into` must name a move in the file.

## 4. Where real time changes the model

The original is turn-based, so some of its concepts have no direct real-time meaning. This is
how each one is handled.

| Original idea | Here | Why |
|---|---|---|
| Menu action with a type (Attack, Special...) and optional parameters | `input` (button, direction, buffer, hold) plus `params` | A controller has no menu. Parameters are declared on the move and bound to the stick, a hold time or an extra button. |
| `iasa_at`, `interrupt_into` | Recovery ends at the end of the timeline; early exits are `cancels` | The player is free when the move ends, so only *early* cancels need data. |
| `hit_cancel_into` category lists | `cancels` with `when: OnHit` / `OnHitOrBlock` | Same idea, explicit trigger and window. |
| `interrupt_from` categories | `tags` | A move declares which groups it belongs to. A cancel's `into` matches any tag. |
| `plus_frames` and turn-synchronised blocks | `block_advantage`; the loader derives blockstun from it and the frames the move has left after contact | Authors think in advantage, and the engine keeps the two fighters consistent. |
| Initiative / "who acted first" for parries | Not modelled | Real-time parries are timing windows owned by the guard move, not by this format. |
| Parry timing slider, DI chosen per hit | Input-time decisions, outside move data | Read from the controller during the relevant frames. |
| Widgets that fill in a move's parameters (direction plot, slider, checkbox, option list, count) | `params` with a typed kind, a default and an optional binding | The data says what is chosen; a separate control scheme says how. |
| Prediction ghosts, undo | None | Not a real-time concept. |
| Anti-stall penalty meter | None | Movement and spacing do this job in real time. |
| GDScript state classes | `behavior` / `mechanic` ids | Keeps scripts out of the data and makes the Rust side testable. |

## 5. Loading model

1. Load the **base library** named by `base_library` (idle, jump, dash, hurt states, block,
   burst...). It is a file in the same format.
2. Apply the character's moves; a move with the same id as a library move **replaces** it.
3. Validate (`CharacterFile::validate`). All problems are collected and reported together.
4. Convert every `Fx` to fixed-point (round to nearest, ties away from zero) and normalize
   knockback directions. After this step no floats remain.

Validation rules include: ids and tags are snake_case; hitbox windows lie inside the active span;
`active > 0` if and only if there are hitboxes; motion, spawn, defense and cancel frames lie inside
the timeline; spawns reference declared projectiles; boxes have positive half sizes; damage is
not negative; a knockback with strength needs a direction.

## 6. What is deliberately not in the data

- **Unique mechanics** (hover, flight, grappling hook, gun pickup...): Rust modules registered
  by id and attached through `mechanic`.
- **Conditional move logic** (branches, targeting, state carried between moves): `behavior`.
- **Art, audio, UI:** referenced by logical name only.
- **Input motions** (quarter-circle and similar): not needed yet; add `Input.motion` when a
  move wants one.

## 7. Converting existing data

`tools/convert_character.py` turns an extracted character into a full character file (all its own
and overriding states, plus projectiles) and writes a report of everything it could not express.
`--only <State>` converts a single state. Every guess is recorded in `provenance`. Mapping used:

| Extracted field | Output |
|---|---|
| first hitbox `start_tick`, last `start_tick + active_ticks`, `anim_length` | `timeline` (startup, active, recovery) |
| hitbox `start_tick + 1 ..= start_tick + active_ticks` | hitbox `window` |
| `damage`, `hitstun_ticks`, `hitlag_ticks`, `victim_hitlag` | `damage`, `hitstun`, `hitlag` |
| `dir_x`, `dir_y`, `knockback` | `knockback` |
| `plus_frames` | `block_advantage` |
| `damage_proration` | `scaling.proration` |
| `interrupt_from` categories | `tags` |
| `hit_cancel_into` categories | `cancels` |
| `enter_force_*` | `motion` impulse at frame 1 |
| per-tick physics calls found in the script's `_tick` | `physics` |
| menu type | placeholder `input` button (flagged as a todo); moves entered by throws or follow-ups get no input |
| `endless` | `timeline.end` = `OnLanding` for air moves that check the ground, else `Script` |
| `always_on`, `looping`, `followup_state`, `throw_state` | `until_end`, `repeat`, `followup`, `throw_into` |
| `HurtboxState` nodes | `hurt` windows |
| `allowed_stances_string`, `change_stance_to` | `modes`, `set_mode` (and `character.modes`) |
| `combo_only`, `neutral_only`, `super_level_`, `uses_air_movement` | `availability`, `cost` |
| `projectile_scene` with a non-negative `projectile_tick` | `spawns` (a negative tick means the script fires it, which is reported) |
| `iasa_at` before the last frame, `interrupt_frames` | `cancels` with `when: Always` |
| negative hitbox sizes | absolute value, as the original engine does on load |
| `grounded_hit_state`, `aerial_hit_state`, `minimum_grounded_frames` | `reaction` (`HurtGrounded`=Stumble, `HurtAerial`=Launch, `Grabbed`=Held, `HurtDizzy`=Dizzy) |
| `launch_reversible`, `vacuum`, `send_away_from_center` | `knockback.reversible`, `knockback.from` |
| `hitbox_type` | `kind` (`ThrowHit` becomes `ThrowDamage`) |
| `knockdown_extends_hitstun`, `air_ground_bounce`, `increment_combo`, `minimum_damage` | `effects.stun_until_landing`, `effects.air_ground_bounce`, `scaling.counts_as_hit`, `min_damage` |
| `block_cancel_allowed` | the hit-cancel `when` is `OnHit` instead of `OnHitOrBlock` |
| `ThrowState` exports: start / release positions, `throw_positions`, `release_frame`, damage / knockback exports | `throw` (hold, keyframes, release hit). Keyframes are computed by expanding the sprite animation tick by tick: each tick shows one frame image, and a position keyed to that image applies. |
| `apply_custom_grav` / `custom_grav*`, `apply_custom_limits` / `custom_max_*`, `apply_custom_x_fric` / `custom_x_fric`, `apply_forces_no_limit` | `physics.gravity_override`, `speed_caps`, `friction_x` / `friction_y`, `uncapped_forces` |
| `reset_x_momentum`, `reset_y_momentum` | `motion` `StopHorizontal` / `StopVertical` at frame 1 |
| `player_parameters` widgets (`XYPlot`, `SemicirclePlot`, `8Way`, `HorizSlider`, `CountOption`, `CheckButton`, `OptionButton`) | `params`. The first direction parameter is bound to the stick; all others stay unbound. |

Anything else the converter sees on a scene is sorted into a class in the report: a **turn-based
concept dropped by design**, an **engine rule to re-decide**, **gameplay data the format cannot
hold yet**, a **parameter that belongs with behavior code**, **presentation**, or **unrecognised**.

The assumptions the converter records are the real open questions of the format. The main ones:
the off-by-one between the original `start_tick` and the first hit-capable frame, how blockstun
should be tuned, whether cancels should open before or after hitlag, and the placeholder input binding.

## 8. Open decisions

Closed in this version: hit reactions, throw choreography, per-move physics overrides, and typed
move parameters (see section 3). What is still open:

1. **Tag vocabulary.** Today tags are free snake_case strings. A small fixed core vocabulary
   (grounded, aerial, attack, special, super, light...) plus character-specific ones would catch typos.
2. **Cancel matching.** `into` matches *any* tag. If moves need "has all of these tags", add an
   `AllOf` form. Cancel exclusions (`interrupt_exceptions`) have no field either.
3. **Control scheme.** `params` say what a move needs; nothing yet says how a controller supplies
   sliders, toggles, option lists and counts. Only the first direction is bound (to the stick). A
   separate control-scheme file is the likely home.
4. **Parameter assumptions.** A slider's default is 0 and its range comes from the widget scene
   (-100..100, or 0..100 when not centered); the eight-way widget's directions are assumed to be
   relative to facing (east = forward); a mirrored arc is recorded but only one side is kept.
5. **Hurtboxes.** `hurt` covers the body hurtbox changes, but the original's per-limb hurtboxes are
   not modelled.
6. **Smaller hit properties** with no field: smash-DI and DI strength on a hit, hitbox groups and
   priority, armor-piercing, whether a hit can land on a dizzy victim, counter-hit eligibility, and
   forcing a victim to stay grounded.
7. **Cancels in open-ended moves** cannot say "until the move ends".
8. **Units.** Speeds and forces keep the original's per-tick units. They will need retuning once the
   real-time feel is playable.
9. **Fixed-point type.** The sim crate decides the exact type; `Fx` only fixes the authoring syntax
   and the rounding rule.
10. **Format versioning.** `format_version` is checked; migration tooling is not written.

### Projectile fields added with the oracle ports

`force` is a push applied on the first tick (after friction); `friction` is per-tick on each
axis (not applied below a speed of 0.05, like the original); `events` are `_frame_N` hooks on the
projectile's own tick (`SetFriction`, `Disable`); `zone` burns a grounded opponent within reach
every n ticks; `on_disable` names a projectile that appears where this one ends; `pierces` keeps
it alive after a hit. Hitboxes gain `group` (boxes of one group hit a victim once together), and
`Physics` gains `gravity_from` (gravity starts mid-move). The values for the five characters
come from `crates/convert/src/verified.rs`, checked against traces of the original game
(`tools/oracle`).
