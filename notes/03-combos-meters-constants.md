# YOMI Hustle notes 3: combo scaling, damage pipeline, meters, anti-stall, constants

Source: `characters/BaseChar.gd` in the private decompile (`reference/yomi_recovered/`).
Same caveat as notes 01 and 02: read from code, not yet confirmed in the running game.

Functions read: `take_damage`, `combo_stale_damage`, `get_combo_stale`, `guts_stale_damage`,
`get_guts`, `incr_combo`, `increment_opponent_combo`, `reset_combo`, `apply_hitlag`,
`gain_super_meter` and friends, `gain_burst_meter`, `use_burst*`, `get_scaled_di`,
`get_di_scaling`, `process_extra`, `add_penalty`, `get_sadness_distance_penalty`, plus the
constant block at the top of the file.

---

## 1. Damage pipeline (what `take_damage` does, in order)

Inputs: raw damage, a minimum, a meter-gain modifier, a combo-scaling offset, and flags for
self-hit and armor-block. All arithmetic is fixed-point (strings) or integers.

1. Zero damage returns immediately.
2. Gain a little burst meter proportional to damage (damage / 5).
3. **Combo scaling:** damage x `stale(count)` (formula in section 2), floored at 1,
   then raised back to the move's minimum damage if it has one.
4. **Guts scaling:** damage x guts factor by remaining HP. In the shipped table there is
   one entry (1.0 -> 1.0), so guts is effectively off. An older table with eight HP
   thresholds is still in the file but unused.
5. **Parry combo scaling:** if the attacker is in a combo that started from a perfect parry
   (or a parried burst), damage is multiplied by 0.85 (two separate fields, both 0.85).
6. **Anti-stall damage modifier:** damage x `penalty damage modifier` (section 5).
7. **Super meter:** the attacker gains meter equal to damage x the hitbox's meter modifier;
   the victim gains damage/3 x a received-damage modifier.
8. **Final scaling:** damage x the fighter's own damage-taken modifier x a global
   damage modifier (both default 1.0).
9. Apply to HP, track combo damage on the attacker, reduce the victim's anti-stall penalty by 25.
10. HP never goes below 0, and armor or armor-block or push-block keep the victim at 1 HP.

Notes:
- Counter-hit damage is raised before this function (x1.1) and blocked armor hits halve
  damage first (see notes 02).
- Chip damage on block goes through the same function with its own meter modifiers.

## 2. Combo scaling ("staling")

Combo state lives on the *attacker*: `combo_count`, `visible_combo_count`,
`hitstun_decay_combo_count`, `combo_proration`, `combo_moves_used`, `combo_damage`.

### Counting hits
- `incr_combo` adds the hitbox's `combo_scaling_amount` to `combo_count` and 1 to
  `hitstun_decay_combo_count`, only if scaling applies. `visible_combo_count` is the
  on-screen number and always goes up by 1.
- A hit scales if the hitbox says `scale_combo` or it is the first hit of a combo.
  Per-character `combo_scaling_disabled` overrides that.
- Multi-hit melee attacks only scale once per attack (a flag stops further increments
  until reset); projectiles can scale on each hit.
- The first hit of a combo grants one extra air movement if the victim has used one.

### Proration
- The first scaling hit stores `combo_proration = min(hitbox.damage_proration, 15)`.
  It is set once per combo.
- Proration shifts how far along the scaling curve every later hit sits.

### Scaling curve
`stale(n) = 0.2 + 0.8 x ((15 - min(n, 15)) / 15)^2`

where `n = max(combo_count - offset + (proration if combo_count > 1 else 0) - 1, 0)`.

So the damage multiplier starts at 1.0 and falls quadratically to a floor of 0.2 at 15
counted hits. A few example values:

| n | multiplier |
|---|---|
| 0 | 1.00 |
| 3 | 0.71 |
| 5 | 0.56 |
| 8 | 0.37 |
| 10 | 0.29 |
| 15+ | 0.20 |

(Rounded; computed from the formula above, not observed in game.)

Super meter gain uses the same curve with a slightly different meter modifier (0.85) after
the first tick, and halves again for every super used in the combo (`2^combo_supers`).

### Other combo effects tied to hitboxes (from notes 01/02)
- Repeating the same move: knockback grows x1.25 (grounded) or x1.05 (aerial) per repeat,
  capped at 30, and hitstun shrinks by a growing amount (not below half).
- `hitstun_decay_combo_count` also feeds combo pushback.
- Proration reduces hitstun in the formula but the per-proration hitstun adjustment
  constant is 0 in this build, so that part is currently inactive.
- After `MAX_GROUNDED_HITS` (7) grounded hits in a combo, the next grounded hit is forced
  aerial unless the hitbox forces grounded. Wall slams are capped at 3 per combo.

### Combo end
`reset_combo` runs when the victim becomes actionable (the opponent's state is
interruptible and not busy). It clears every combo field, resets the victim's grounded-hit and
wall-slam counters and brace flags, and may restore one air movement.

## 3. Hitlag

`apply_hitlag` sets the victim's hitlag to the hitbox's victim hitlag (+ counter-hit bonus,
currently 0), then multiplies by the brace modifier (0.35) if a counter succeeded and by the
global hitstop modifier. The attacker gets at least the hitbox's own hitlag. Some fighters
enable "extra aesthetic hitstop", which adds a global freeze of hitlag x 0.6, capped at 10.
Blocks use a smaller version (x 0.25 cap 20).

## 4. Meters and resources

| Resource | Rule |
|---|---|
| HP | 1500 max. |
| Super meter | 125 per bar, up to 9 bars. Gained from damage dealt (damage x modifier), damage taken (damage / 3), moving toward the opponent, whiffs, parries (50 base). Reduced by combo staling and anti-stall penalty. |
| Velocity meter gain | Moving toward the opponent gains meter from speed / 4, halved in the air. Disabled in hurt states. |
| Burst meter | 1500 max, 1 burst stored. Builds 4 per tick (3 in neutral, 2 in neutral while airborne), minus 3 inside a burst-cancel combo, plus damage/5 on hit. Fills to grant a burst; starting with 1. Does not build during the penalty state. |
| Air movements | 2 per airtime by default. Refresh on landing. |
| Feints (free cancels) | 2 stored per fighter, regained on hit/block. Using one when empty spends a super bar instead. |
| Meter grace | After spending a super bar there is a short window where a hit refunds part of it. |

Using burst meter for a counter costs one eighth of the maximum.

## 5. Anti-stall ("penalty" / "sadness")

A single number, `penalty`, from -20 to 75:

- Gains: moving away from the opponent (about +1 every 3 ticks), plus a distance term every
  7 ticks that grows when the opponent is far above you. Gains stop if the opponent is
  within a minimum distance (128 units) or the fighter is in a combo. A grounded fighter
  versus an aerial opponent gains only a quarter.
- Losses: moving toward the opponent (-1 every 4 ticks if fast enough), taking damage (-25),
  clashes (-25), parry (-20/-10), block (-10 for the attacker).
- Scaled down if the fighter is behind on HP (a ratio based on the HP gap / 3000).
- Immunity for 60 ticks after leaving a hurt state.
- **Effects:** damage taken is multiplied up to x1.5 as the penalty rises from 20 to 75, and
  super meter gain is scaled down by the penalty fraction. At the max the fighter loses all
  super bars and enters a 120-tick penalty state: x1.5 damage taken, no meter or burst
  gain, and they lose 2 HP every third tick (but cannot die from it, floor 1).

This is a design device for forcing engagement in a turn-based game. A real-time game with
movement and spacing might not need it.

## 6. Directional influence (DI)

- The player's chosen DI direction comes through the action's `extra` block and is
  stored as `current_di` plus a short `current_nudge` (20 units) used for in-hitlag nudging.
- DI strength scales with the *opponent's* combo count: a minimum multiplier 1.0 at combo
  start rising to a maximum 6.0 by 15 hits (`di_combo_limit`), optionally using the
  proration-adjusted count. A successful brace multiplies it by 1.5, and a hit out of brace
  zeroes it. DI is clamped to that length.
- Applied as an extra force when entering a hurt state (strength 3.5 grounded, 2.0 aerial).

## 7. Constants worth keeping a list of (from the file header)

| Name | Value | Meaning |
|---|---|---|
| MAX_HEALTH | 1500 | |
| MAX_STALES | 15 | combo scaling steps |
| MIN_STALE_MODIFIER | 0.2 | scaling floor |
| MAX_GROUNDED_HITS | 7 | |
| MAX_WALL_SLAMS | 3 | per combo |
| WALL_SLAM_DAMAGE | 0.75 | |
| PARRY_CHIP_DIVISOR | 3 | block chip = damage / 3 |
| PARRY_CHIP_PROJECTILE_DIVISOR | 4 | |
| PUSH_BLOCK_CHIP_MODIFIER | 0.33 | |
| PARRY_KNOCKBACK_DIVISOR | 3 (grounded 1.5) | block pushback |
| PARRY_COMBO_SCALING | 0.85 | damage after a parry |
| PROJECTILE_PERFECT_PARRY_WINDOW | 3 | ticks |
| AUTO_PARRY_TICKS | 20 | |
| WAKEUP_THROW_IMMUNITY_TICKS | 3 | |
| SADNESS_IMMUNITY_TICKS | 60 | |
| PENALTY_TICKS | 120 | |
| MIN_PENALTY / MAX_PENALTY | -20 / 75 | |
| MIN_DIST_SADNESS | 128 | |
| DISTANCE_EXTRA_SADNESS | 180 | |
| COUNTER_HIT_DAMAGE_MODIFIER | 1.1 | |
| SUCCESSFUL_BRACE_HITSTUN_MODIFIER | 0.35 | |
| SUCCESSFUL_BRACE_DI_MODIFIER | 1.5 | |
| BASE_PLUS_FRAMES / VS_AERIAL / WRONG_HEIGHT | 0 / 2 / 2 | block advantage terms |
| AIR_BLOCK_PUSHBACK_MODIFIER | 0.35 | |
| PUSH_BLOCK_FORCE / DIST | -10 / 220 | |
| GLOBAL_HITLAG_MODIIFER / MAX | 0.6 / 10 | |
| GLOBAL_BLOCKLAG_MODIFIER | 0.25 | |
| HITLAG_COLLISION_TICKS | 4 | |
| MAX_SUPER_METER / MAX_SUPERS | 125 / 9 | |
| PARRY_METER | 50 | |
| METER_GAIN_MODIFIER | 0.85 | |
| MAX_BURST_METER / BURST_BUILD_SPEED | 1500 / 4 | |
| NUDGE_DISTANCE | 20 | |
| CLASH_MOVE_BACK | 3 | |
| num_air_movements (default) | 2 | per character |
| num_feints (default) | 2 | per character |

## 8. Implications for the real-time rewrite

1. **Damage, scaling and meters are pure arithmetic**, with no dependence on turn flow.
   They port almost directly, and should live in the simulation crate with
   fixed-point/integer math.
2. **Scaling needs a decision:** in real time combos are longer and more fluid, so the
   quadratic 15-step curve and proration will feel different. Keep it configurable.
3. **Anti-stall is turn-based by design.** In real time, movement and spacing do that
   job; I would drop it or replace it with something like a simple stalling timer.
4. **Burst and counter systems** assume a pause to commit a risky read. In real time the burst
   (combo breaker) can map to one button; the counter/brace is a prediction-based tool and
   needs a different design.
5. **DI** is chosen once per hit via the action's extras. Real time can read the stick
   direction continuously during hitstun/hitlag.
6. **Hit-state caps** (7 grounded hits, 3 wall slams) are easy to keep.

## 9. Open questions

- `get_data()`, `get_hitbox_x_dir`, and whether `damage_proration` values in shipped
  characters actually vary.
- Whether the guts table is really disabled in the live game (single entry here).
- Exact `SUPER_METER_GRACE_PERIOD`, `SUPER_METER_GRACE_DIVISOR` (declared elsewhere).
- Effect of `turbo_mode` and `extremely_turbo_mode` flags.
- Check the table in section 2 by logging real combos in the live game.
