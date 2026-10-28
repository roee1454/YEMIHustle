# YOMI Hustle architecture notes (first pass)

Source: GDRE decompile of the Steam build (Godot 3.5.1) in `reference/yomi_recovered/`.
These notes are my own description of observed behavior. The reference folder is private
and must never be committed to the public repo. Everything below should be treated as
"first impression from reading" until confirmed by running the game and testing.

Files read: `game.gd`, `characters/BaseChar.gd`, `ObjectState.gd`,
`characters/states/CharState.gd`, `framework/FiniteStateMachine/StateMachine.gd`,
`mechanics/Hitbox.gd`, and one ninja move script (`characters/stickman/states/QuickSlash.gd`).

---

## 1. The big idea: a fixed-step simulation that pauses for decisions

There is no separate "turn mode". The game is one deterministic tick loop (`Game.tick()`),
and a turn is just a pause.

- Every tick advances every object, then both fighters, then collisions and hitboxes.
- `is_waiting_on_player()` is true when at least one fighter's `state_interruptable` flag is set.
- While it is false, `process_tick()` keeps calling `simulate_one_tick()` (about once per
  render frame, with playback speed and frame-advance controls).
- While it is true, the sim stops, the UI shows the move picker, and the game waits.
- `simulate_until_ready()` is the same loop run in a single call. It is used for prediction
  ("ghost" copies of the game), replays and re-simulation.

Consequence for a real-time version: the sim already steps one tick at a time and does not
depend on a pause. The pause exists only so a human can pick an action. A real-time game
could keep ticking and supply an action (or "nothing") on every tick.

## 2. Tick order inside one game tick

1. Hooks (`pre_tick`), network input reset.
2. Every active non-fighter object ticks (projectiles etc.). Positions are clamped to stage
   width, and to the ceiling if the stage has one.
3. Visual effects tick. `current_tick` increments.
4. Decide which fighter updates first (see "port priority" below).
5. For each fighter in that order: `tick_before()`, then `update_advantage()`, then `tick()`.
   (Both `tick_before` calls run first, then both `update_advantage`, then both `tick`.)
6. Resolve overlapping x positions, spawn queued objects.
7. Resolve fighter-fighter collision, then apply hitboxes (this is where hits happen).
8. If a fighter is interruptible and not in a "busy" interrupt, the opponent's combo resets.
9. Hooks (`post_tick`), end-of-game check (HP <= 0 or time limit).

### Port priority (who updates first)
The order is chosen fresh every tick by a list of tie-breakers, in order:
1. Lower `tick_priority` on the current state wins.
2. The fighter who is NOT in a hurt state goes first (so the comboer updates before the victim).
3. Whoever has hitboxes beats whoever has none; otherwise the earlier hitbox start tick;
   then higher damage.
4. Lower "penalty" (the anti-stalling/"sadness" meter) if the difference is at least 10.
5. A state flagged `beats_backdash` wins.
6. Lower HP percentage.
7. Default: player 1.

This matters a lot for a rewrite. The update order is gameplay: it decides trades and
who wins simultaneous hits. A faithful port needs this exact order or an intentional,
documented replacement.

## 3. Fixed-point math

Gameplay numbers are not floats. Positions, velocities and many tunables are strings
like `"1.0"` or `"-10.0"`, handled by a native fixed-point library (`host.fixed.mul`,
`.add`, `.div`, `.vec_mul`, `.round` ...). That library lives in the native module
`tbfg.so` and is exposed to scripts as `FixedMath`.

Why it matters:
- Determinism across machines (needed for replays, prediction and netcode).
- The rewrite should use integer or fixed-point math in the simulation. The exact
  fractional precision is currently unknown and must be measured from `tbfg.so`
  or by experiment.

## 4. Object and state model

- `BaseObj` -> `Fighter` (`BaseChar.gd`, about 3,000 lines). Projectiles and other things are
  also `BaseObj`.
- A fighter owns a `StateMachine`. Every state is a child node, looked up by name.
- State classes layer like this: `StateInterface` -> `ObjectState` -> `CharacterState`
  -> per-move classes (`QuickSlash` extends `SuperMove`, etc.).
- State machine tick: apply any queued state change, then run `_tick_before`,
  `_tick_shared`, `_tick` and `_tick_after` in order; the first one to return a state name
  queues a transition. A transition runs exit callbacks of the old state and enter
  callbacks of the new one. The state can itself immediately redirect to another state
  from `_enter`.
- The state history is kept in a stack of 32 entries.

### What a state is (ObjectState)
Mostly data exported on the node:
- Physics flags per state: apply friction, apply gravity, apply forces, reset momentum,
  custom friction/gravity/speed limits.
- Animation: name, length in ticks (`anim_length`), loop settings, ticks-per-frame.
- Scripted forces at a given tick, enter force, particles, screenshake, sounds,
  projectile spawn tick and position.
- `host_commands`: a map from tick number to a method to call on the fighter.
- Per-tick hooks: methods named `_frame_<n>` run once on that tick.
- Hitboxes and hurtboxes as child nodes (activated by start tick).

### What a character state adds (CharacterState)
- Menu info: title, category (Movement, Attack, Special, Super, Defense, Hurt), icon,
  and an optional "data UI" scene (direction picker, sliders, etc).
- Air/ground usability, air-movement cost, land cancel.
- **Interrupt rules.** These decide when a move can be left:
  - `iasa_at`: the tick it becomes interruptible (negative counts from the end).
  - `interrupt_frames`: extra individual ticks that are interruptible.
  - `iasa_on_hit` (and a flag to treat block like hit).
  - The final tick of the animation is always interruptible.
  - String lists for "can interrupt from", "can interrupt into", hit-cancel into/exceptions.
- Stances, feints, free-cancel flags, whiff-cancel behavior.
- Super properties (level, freeze ticks, effect flags).
- `can_interrupt()` is essentially "is the current tick one of the interruptible ticks".
  When true, the state raises the fighter's `state_interruptable` flag, which is what
  `is_waiting_on_player()` watches.

## 5. How a player's choice enters the sim

1. The UI collects `(action, data, extra)`: the state name, a dictionary of parameters
   (for example direction as x,y, an attack id, a slider value), and an extras block.
2. `on_action_selected` stores them in `queued_action/data/extra` and clears the
   interruptable flag, which un-pauses the sim.
3. At the start of the next tick, `tick_before` validates the action, logs it into the
   replay table keyed by `[player id][tick]`, and the state machine switches to it with `data`
   attached. The new state reads `data` in `_enter` (for example `data.Direction`).
4. There is a buffering system (`buffered_input`, `use_buffer`) that lets a player
   pre-store the next action; it is consumed at `tick_before`.
5. Replays are just these per-tick action dictionaries. Replay playback feeds them back in
   the same way.

This is the most useful part for the real-time idea. The entire input interface is already
a tick-indexed stream of `(action name, parameters)`. A real-time version only has to
generate that stream from a controller or keyboard every tick.

## 6. Hits, blocks and combos (Hitbox.gd)

A hitbox is a rectangle node with a big list of tunables:
- Damage (with a separate in-combo value and minimum), chip modifier.
- Start tick and active ticks, optional looping on/off pattern.
- Hit type: normal, flip, throw, burst, no-hitstun, detect-only.
- Hitstun, combo hitstun, hitlag (shared, and a separate victim value).
- Knockback vector and strength (capped at 30), pushback, launch rules.
- Height: high / mid / low.
- Knockdown flags, ground bounce, wall slam, guard break.
- Hit-state names for grounded and aerial victims.
- Parry/clash/priority/group flags (groups stop multi-hits on one object).
- Proration and combo scaling.

Combo behavior I noticed in the code:
- Repeating the same move in a combo makes knockback bigger (x1.25 grounded, x1.05 aerial per
  repeat, capped) and hitstun shorter.
- Hitstun is reduced further by proration.
- Hit objects are tracked per hitbox group so one attack cannot hit the same target twice.

Throws bypass invulnerability and use a special hit type. Attacks can be parried, countered
(brace/counter states) or clashed.

## 7. Other systems seen in `BaseChar.gd`

- Meters: super meter (gained by moving toward the opponent, hits, whiffs), burst meter,
  air-option bar, free-cancel ("feint") count.
- Anti-stall "penalty/sadness" meter: builds when moving away or staying too far,
  drains HP slowly when it is full.
- Directional influence (DI) and a smaller nudge during hitlag.
- Hitlag applies to both fighters ("global hitlag") with a cap.
- Super freeze: while `super_freeze_ticks > 0` the sim does not advance.
- Replay/rewind: `ReplayManager` stores inputs; "undo" rewinds and resimulates.
- Ghost games: a second `Game` instance used to preview outcomes. This is why many
  functions check `is_ghost`.
- A "state hash" (position, velocity, DI, state name) exists for desync detection.

## 8. Content layout

- `characters/<name>/` holds one scene per fighter (for example `NinjaGuy.tscn` in
  `stickman/`), the fighter's script, per-move scripts in `states/`, sounds, sprites,
  projectiles, and "ActionUIData" scenes for the parameter pickers.
- `characters/states/` holds shared states: Idle, Jump, Fall, Dash, Parry, Grab, Burst,
  hurt/knockdown, taunt and others.
- Character roster in the build: stickman (ninja), robo, wizard, mutant, swordandgun,
  and a stub for alien.
- Move scripts are small: scene data defines timing and hitboxes, and the GDScript adds
  special behavior in `_enter`, `_frame_<n>` and `_tick`.

## 9. Implications for the real-time rewrite

What carries over almost directly:
- Fixed tick, deterministic sim, per-fighter state machine, frame data in hitboxes.
- The `(action, data)` input format. A real-time front end just produces it every tick.
- Interrupt windows as a natural "cancel table" for real-time input buffering.

What needs design work:
1. **Interrupt windows are sparse.** Many moves can only be left at the end or at one or
   two ticks. In real time that feels stiff, so frame data must be retuned or a buffer must
   hold presses until the next legal tick.
2. **Port priority and trades.** The tie-break list above is what makes simultaneous
   decisions fair. In real time, simultaneous hits are still possible and the same rules
   can be kept.
3. **Parameterized moves.** Several moves take a direction or slider as `data`. A
   controller needs a rule to derive those from stick direction and held buttons.
4. **UI-driven systems.** Prediction ghosts, undo/rewind and the move picker are specific
   to the turn-based design and would be dropped or reimagined.
5. **The native core.** Fixed-point math and some object stepping live in `tbfg.so`. We still
   need the exact precision and rounding behavior.

## 10. Open questions (verify before relying on any of this)

- Fixed-point format and rounding in `tbfg.so`.
- What the native `Simulation` and `FGCharacter` actually do versus the scripts. The
  scripts call `native.copy_state` and hold `fixed` helpers, so some state copying and
  integration is native.
- Exact meaning of `interruptible_on_opponent_turn`, `busy_interrupt`, `initiative` and
  frame-advantage computation (`get_advantage`).
- Hurt state, parry and clash scripts have not been read yet.
- How mods hook in (`MLMainHook`, `ModOverride`, `ObjHooks`).
- Whether a move's stated frame counts match behavior in the live game (needs testing).
