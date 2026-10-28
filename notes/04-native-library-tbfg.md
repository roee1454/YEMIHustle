# YOMI Hustle notes 4: the native library (`tbfg.so`), fixed-point format, "Simulation"

Method: static inspection of `tbfg.so` (symbols, strings, crate versions) plus black-box
experiments. For the experiments I ran the game's own engine binary headless against a tiny
throwaway Godot project (in the scratchpad, with a copy of `tbfg.so`) and called the native
classes from GDScript, printing results. Nothing in the game's install folder was modified.
Everything marked "observed" was measured that way. Anything marked "inferred" is my
explanation of the measurements and could be wrong in edge cases.

---

## 1. What the library is

- A **Rust** GDNative library (`gdnative` crate 0.11.3) built on a developer machine, shipped
  **not stripped and with symbols**, which is why it reads so easily.
- Game crate name: `turn_based_fighting_game_lib` ("tbfg"), with modules `utils`, `point`,
  `obj`, `char`, `methods`.
- Dependencies of interest: the `fixed` crate **1.19.0**, `fixed-sqrt` **0.2.5**, and a
  `fixed_trigonometry` crate (version not visible in the strings).
- Registered Godot classes: only **three**: `FixedMath`, `FGObject`, `NativeMethods`.

### `Simulation.gdns` and `FGCharacter.gdns` are empty stubs
Both resource files exist in the project (named `GameSimulation` / `FGCharacter`), but:
- No class with those names, nor any `Simulation` string, exists in the library.
- Nothing in the 502 recovered scripts references them.
- Loading the `.gdns` works, but there is no class behind it.

So the answer to "what does the native Simulation do" is: **nothing in this build.** The game
loop, state machine, hit detection, combo logic and every rule in notes 01-03 live in GDScript.
The native library is a small, deterministic **numeric toolkit and a physics body**, not a
simulation engine. That is good news for a rewrite: the rule logic is all readable.

## 2. `FixedMath` (arithmetic on strings)

GDScript passes numbers to the library as **strings** and gets strings back (and sometimes
dictionaries `{x, y}`). Output text has 5 decimals.

### The number format (observed)
- **64-bit signed with 16 fractional bits** (what Rust code would call I48F16; the type name
  itself is not visible, but all behavior matches).
  - Resolution: 1/65536 = 0.0000152588.
  - Integer range: about +/- 140 trillion (2^47). `2^47 - 1` is representable.
- **5-decimal strings are lossless.** One unit is 1.5e-5, which is larger than 1e-5, so each
  representable value prints as a distinct 5-decimal string and parses back to itself.
  That is why passing values as 5-decimal strings works at all.

### Arithmetic rules (observed)
| Operation | Rule | Evidence |
|---|---|---|
| Parse string | Round to nearest unit | `0.1` stored as 6554/65536, so `0.1 * 3` prints `0.30002` |
| add / sub | Exact | |
| mul | **Floor** (round toward minus infinity) | `-3 units * 0.5` gives -2 units |
| div | **Truncate toward zero** | `-1.5 units` gives -1 unit; `1/131072` gives 0 |
| round | Half away from zero | 0.5->1, -0.5->-1, 2.5->3 |
| floor / ceil | Mathematical floor / ceil | |
| Overflow in mul | **Wraps** silently | `2^30 * 2^20` returns 0 |
| Overflow when parsing a string | **Panics** (the call returns null; the game keeps running) | `"140737488355328"` |
| Divide by zero | Panics, returns null | |

Hint for a faithful port: this is exactly the default behavior of the `fixed` crate's `*`
(shift-down floors) and `/` (integer division truncates). Reusing the same crate gives the
same results.

### Vector helpers
Methods registered (from strings): `add sub mul div lt gt le ge eq abs sign floor ceil round
powu lerp_string lerp_int lerp_angle int_map deg2rad angle_dist get_x_intercept vec_add vec_sub
vec_mul vec_div vec_scale vec_dot vec_round vec_len vec_len_squared vec_dist_squared
normalized_vec normalized_vec_times vec_to_angle angle_to_vec rotate_vec`.

**Vector length is a coarse, floored square root (observed):**
`vec_len` returns multiples of 1/256, rounded down. Examples:

| Input | Exact | `vec_len` |
|---|---|---|
| (1, 1) | 1.41421 | 1.41406 |
| (1, 2) | 2.23607 | 2.23438 |
| (7, 3) | 7.61577 | 7.61328 |
| (10, 10) | 14.14214 | 14.14062 |
| (5, 5) | 7.07107 | 7.07031 |

Inferred cause: it takes the integer square root of the raw 16-fractional-bit value, so the
result has 8 fractional bits. Everything built on it (`normalized_vec`, knockback direction)
inherits the error: normalized (1,1) is (0.70717, 0.70717), not 0.70711. Directions of
exact Pythagorean triples (3,4,5) are exact.

**Trigonometry is approximate (observed):**
- `vec_to_angle(1,1)` = 0.7903 (true pi/4 = 0.7854), `vec_to_angle(0,1)` = 1.57079,
  `vec_to_angle(-1,0)` = 3.14159. Error is largest at mid angles.
- `angle_to_vec(0)` = (1.00002, 0): off by one unit.
- Angles are in radians.

## 3. `FGObject` (the physics body: `chara` in `BaseObj.gd`)

Every fighter and projectile owns one. Scripts configure it with the node's exported movement
attributes (defaults: gravity "0.8", ground friction "2.5", air friction "0.2", max ground
speed "15", max air speed "10", max fall speed "15") and call it to move.

### State exposed by `get_data()` (observed)
`position_float` (fixed-point x,y), `position_x`/`position_y` (integers), `vel_x`/`vel_y`
(fixed-point), `facing` (`Right` or `Left`), `grounded`, `snap_to_ground`.

### Method list (from strings)
`set_gravity set_gravity_modifier set_ground_friction set_air_friction set_max_ground_speed
set_max_air_speed set_max_fall_speed set_vel set_position set_x set_y move_directly
move_directly_relative apply_force apply_force_relative apply_forces apply_forces_no_limit
apply_grav apply_grav_custom apply_fric apply_x_fric apply_y_fric add_pushback apply_pushback
reset_pushback reset_momentum set_grounded is_grounded update_grounded set_facing get_facing
get_x_vel_int set/get_snap_to_ground get_data copy_to`, each with a `_str` twin that accepts
strings where the base version takes an `i32`. The GDScript wrapper picks the variant by
argument type.

### Integration rules (observed)
- **Coordinates:** y increases downward; the ground is y = 0 and airborne positions are
  negative.
- **Forces are queued, not applied:** `apply_force` adds to a pending force that
  `apply_forces()` then consumes (a second `apply_forces()` does not re-add it).
- **`apply_forces()` order:** velocity += pending force, clamp horizontal speed to the max
  ground or air speed (chosen by whether the object is grounded), then position += velocity.
  A velocity of 100 set directly is clamped to 15 on the next `apply_forces`.
  `apply_forces_no_limit()` does the same without the clamp.
- **Position** is stored as fixed-point (`position_float`) and the integer position is that
  value rounded half away from zero (0.5 -> 1, 1.5 -> 2, -0.5 -> -1).
- **Gravity:** `apply_grav()` adds the gravity value (0.8) to vertical velocity each tick,
  capped at max fall speed. Falling from y = -100 reached the ground in 16 ticks in my test.
  The ground (y = 0) stops the fall.
- **Friction:**
  - `apply_x_fric(f)` multiplies horizontal velocity by (1 - f). With f = 0.05: 10.41669 ->
    9.89583.
  - `apply_fric()` uses the configured friction: on the ground at 2.5 with max speed 15, speed
    went 15 -> 12.5 -> 10.41669, i.e. multiplied by 5/6 each tick. In the air at 0.2, 10 -> 9.8.
    Inferred formula: `vel * (1 - friction / max_speed)` using the matching max speed.
    I only have three data points, so treat the exact formula as unconfirmed.
- **Facing** is set from an integer and read back as a variant `Right` / `Left`.

## 4. `NativeMethods`

Contains `copy_state` / `from_state`: copies exported properties from one node to another by
a stored property list. It is a speed optimization for cloning game state into the "ghost"
games used for prediction and replays (see notes 01). It is not game logic.

## 5. What this means for the Bevy rewrite

1. **No hidden rules.** All gameplay rules are in readable GDScript; the native library only
   supplies numeric primitives and a small physics body. There is no native simulation to
   reverse-engineer.
2. **Fixed-point format to adopt:** 64-bit signed, 16 fractional bits, with floor-multiply and
   truncating-divide. The `fixed` crate (MIT/Apache) provides exactly this
   (`FixedI64<U16>`). A rewrite in Rust can use the same crate directly.
3. **Exact bit-for-bit compatibility is optional.** It would matter only to replay old
   YOMI replays or share Workshop mod physics. The coarse square root and approximate
   trigonometry are quirks; a new engine can use better math. The decision: copy the quirks
   (fidelity of feel) or fix them (cleaner behavior).
4. **Strings as numbers are an artifact of GDScript**, not something to reproduce. Use typed
   fixed-point values in the sim crate.
5. **Physics model to port:** integer-ish positions from a fixed-point accumulator, queued
   forces, clamped horizontal speed, gravity with a terminal fall speed, multiplicative
   friction, y-down coordinates, ground at y = 0, and pushback as a separate accumulator.
6. **Stage and collisions** (stage width, ceilings, body overlap) are in `game.gd`, not here.

## 6. Open questions

- The exact `apply_fric` formula (needs a test with different max speeds).
- Behavior of `add_pushback` / `apply_pushback` across ticks (one case observed: a pushback
  of 2 moved the position by 2 immediately).
- `snap_to_ground` semantics and `update_grounded` rules (observed: grounded when position y
  reaches 0).
- `get_x_vel_int`, `lerp_angle`, `int_map`, `get_x_intercept`, `rotate_vec` details.
- Version of the trigonometry crate used.
- Whether the Windows `tbfg.dll` is identical in behavior (assumed, not checked).
