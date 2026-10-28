# YOMI Hustle notes 2: hit resolution, blocking/parry, clash, hurt states

Source: decompiled scripts in `reference/yomi_recovered/` (private). Same caveat as
notes 01: this is a reading of the code, not yet confirmed by running the game.

Files read: `game.gd` (`resolve_collisions`, `apply_hitboxes`, `get_colliding_hitbox`),
`characters/BaseChar.gd` (`hit_by`, `launched_by`, `block_hitbox`, `can_counter_hitbox`,
`get_advantage`, `take_damage` start), `characters/CharacterHurtState.gd`,
`characters/states/{HurtGrounded,HurtAerial,Knockdown,Getup,NewParry,CounterAttack}.gd`.
Not read yet: `Parry.gd` (old parry), `ThrowTech.gd`, `Grabbed.gd`, `WallSlam.gd`,
`ThrowBox.gd`, the rest of `take_damage` (combo scaling), `increment_opponent_combo`.

---

## 1. One tick of hit resolution (`Game.apply_hitboxes`)

Order matters because hits are *detected first, applied second*.

1. Collect each fighter's active hitboxes and move them to the fighter's current position.
2. For each fighter, find the first opposing hitbox that overlaps their hurtbox (skipped if
   the fighter is invulnerable). A hit by a `ThrowBox` is recorded as a throw, not a hit.
3. **Clash check** (only if clashing is enabled on the stage/game). For each pair of
   overlapping hitboxes from both fighters that are allowed to clash:
   - If the damage values differ by less than a threshold (`CLASH_DAMAGE_DIFF`), it is a
     real clash. Both attacks cancel.
   - If the difference is bigger, the higher-damage hitbox wins and the other fighter's hit
     is erased (the stronger attack goes through, the weaker one is dropped).
   - An "asymmetrical" mode lets a clash happen when only one side was going to land.
4. **If clashed:** both fighters get a clash state (back to idle or fall depending on
   grounded), each gets a penalty reduction of 25, and a clash effect spawns.
5. **Otherwise:** each fighter hit by a normal hitbox calls `hitbox.hit(victim)`. Both can
   be hit in the same tick (a trade) because detection was done before applying.
   A throw only lands if the hitbox's `beats_grab` flag allows it against the opponent's
   own throw.
6. **Throw logic:** if neither side was hit by a normal hit:
   - Both throwing and both states `throw_techable` -> both enter `ThrowTech`.
   - Both throwing and neither techable -> nothing happens.
   - One throwing: lands unless the target is grounded/aerial and the throw does not hit
     that kind, or a tech applies. On landing it queues the attacker's throw state.
7. **Projectiles and objects:** players' hitboxes can hit objects, objects' hitboxes can
   hit players, and objects can hit each other. These are collected and applied after the
   fighter-vs-fighter logic. A fighter flagged projectile-invulnerable ignores projectiles
   that are "immunity susceptible".

### What counts as a valid hit (`get_colliding_hitbox`)
A hitbox is skipped if any of these are true:
- It does not overlap the main hurtbox or any active limb hurtbox of the target.
- It is a throw and the target cannot be thrown (blockstun, wakeup throw immunity).
- Vs aerial/grounded flags do not match the target's state.
- Target is "off the ground" (OTG) and the hitbox does not hit OTG, or the target is
  standing and the hitbox is OTG-only.
- Target is a projectile and the hitbox does not hit projectiles.
- This hitbox group already hit this object.
- The target has "aerial/grounded attack immunity" matching the attacker's state.
- Attacker and target are the same team and friendly hits are not allowed.

## 2. Hitbox.hit -> Fighter.hit_by

`Hitbox.hit(target)`:
- Ignores targets already hit by this group, invulnerable targets (throws ignore
  invulnerability), and OTG targets when not allowed.
- Calls `target.hit_by(hitbox data)`, spawns hit sparks/sound, adds pushback to
  attacker, victim and the victim's opponent, and emits `hit_something`.
- Pushback = -(pushback_x + combo pushback term) / 2, applied to everyone involved.

`Fighter.hit_by` decides what happens, in order:
1. Ignore if already parried this frame or the hitbox was already parried.
2. Counter check: if the target is in a bracing counter state and it matches the
   hitbox (height or grab), it records a successful brace and plays a prediction effect.
3. A throw hitbox on a non-OTG target goes to `thrown_by`.
4. If the target is **not** in a parry/block state (or the hit is forced), the hit lands:
   behavior depends on `hitbox_type` (see below).
5. Otherwise the hit goes to `block_hitlag` -> `block_hitbox`.

Hit types seen:
- **Normal / Burst:** `launched_by` (the full hit path below).
- **NoHitstun:** only damage.
- **Flip:** turns the victim around and reverses horizontal velocity, then damage and combo.
- **ThrowHit:** hitlag on both, rumble, damage, combo count.
- **OffensiveBurst:** resets hitstun decay, launches, resets pushback.

### `launched_by` (a hit that lands)
- Super-meter "grace" refunds some meter spent recently.
- Hitlag is applied to the victim and the attacker (attacker gets at least the same).
- The victim's armor logic: armor can stop the launch; auto-block armor turns it into a block
  with half damage.
- If launching: choose grounded or aerial hit state from the hitbox. A run of grounded hits
  (`MAX_GROUNDED_HITS`) converts the next one into an aerial hit unless forced.
- Increment the combo counter, switch the victim state with `{hitbox}` as data,
  disable collision with the opponent if the hitbox says so, mark `busy_interrupt`.
- Damage is computed from the hitbox (combo vs non-combo value), halved for auto-blocks,
  modified for counter hits, then passed to `take_damage` (combo scaling, meter gain).

## 3. Hurt states

Common (`CharacterHurtState`): on enter it clears feints, releases any grab, stores the
hitbox, turns off opponent collision if asked, sets `state_interruptable = true` and
`busy_interrupt = true` immediately (the victim is "actionable" but flagged busy), and clears
the buffered input. `can_interrupt()` is always false for hurt states; they end by their own
timer.

**Hitstun length** = hitbox hitstun (+ counter-hit bonus, currently 0) scaled by a global
hitstun modifier, then shaved by DI (the shave function is currently a stub that returns
the value unchanged) and by a successful brace.

**Grounded hurt** (`HurtGrounded`): picks animation by hit height; faces away from the
hit; knockback = normalized direction x knockback strength with y forced to 0, plus a DI
push (DI strength 3.5 x hitbox DI modifier); touching a wall removes horizontal knockback
unless it is a wall-slam hit. While in hitstun: stay on the ground, apply ground friction
(0.05) and forces. If wall-slam and the wall is reached -> `WallSlam`. When
`current_tick >= hitstun` it first sets `can_act` and raises the interrupt flag; on the
following tick it falls back to the default state.

**Aerial hurt** (`HurtAerial`): same knockback formula but with y kept, DI strength 2.0,
air friction 0.015, hit gravity 0.25, fall speed cap 15, wall bounce factor -0.85 (with a
3-tick hitlag on bounce). Supports ground bounce (flips vertical direction, multiplies
knockback, and holds a few "bounce frames"), air-to-ground bounce, wall slam, and knockdown.
On touching the ground after the minimum grounded frames it goes to `Knockdown`
(or hard knockdown / landing if no knockdown). It leaves hitstun the same way as grounded.

**Knockdown / Getup:** very small. Getup zeroes vertical speed, sets grounded and
"on the ground" (OTG), turns off opponent collision, plays a bass sound and camera bump,
applies ground friction (0.125). OTG state is what the hitbox OTG flags check.

## 4. Blocking is a state with parameters, and parry timing is a chosen number

There is no "hold block" input. Blocking is a character state (`ParryState`, the newer
`GroundedParryState` marked `IS_NEW_PARRY`) that the player selects like any move. Its
parameters are picked with the action:
- `Block Height`: high or low (x/y data; y = 0 means high).
- `Melee Parry Timing`: a frame count the player chooses (displayed as "Nf").

`Fighter.hit_by` only sends a hit to blocking if the target is currently in a parry state
and that state says the hit is parriable (`can_parry_hitbox`: not punishable, parry still
active, started in combo consistency, etc.).

### Perfect parry vs normal block (`block_hitbox`, new parry path)
- `parry_timing` = frames since the fighter's last turn (+ the opponent's hitlag).
- `in_parry_window` = timing matches the chosen timing (or the chosen timing is 20+ and
  enough frames have passed), or the hit is a burst during a combo.
- A perfect parry needs: the fighter *can* perfect parry, did not whiff the previous
  block, the opponent was feinting or the fighter has **initiative**, the hitbox is
  parriable, the block height matches the hit height, the fighter did not block the last
  hit, and the timing window matches. There is also a "feint parriable" override and an
  "always perfect parry" cheat flag.
- Projectiles use a simpler window (a short fixed number of ticks, or flags on the
  projectile).

### Perfect parry result
Defender gains super meter, penalty drops by 20 (projectile: 10), and the defender's state
is told it was a perfect parry (state raises its interrupt flag, clears blockstun). The
attacker is marked "got parried", loses its `interruptible_on_opponent_turn`, and can be
punished.

### Normal block result
- Optionally a **guard break**: if the attacker has initiative and the move is a guard
  break, the hit lands anyway with extra proration.
- Chip damage = hitbox damage / chip divisor x chip modifier (wrong block height adds half
  again, push-block scales it). Damage taken uses a meter-gain modifier.
- Attacker gains super meter (parry meter / 6), a free-cancel ("feint") charge, and
  penalty -10.
- **Synchronization trick:** the defender's block state copies the attacker's state length,
  `endless` flag and `iasa_at`, and restarts its own tick counter at 0, so both fighters
  become actionable together based on the attacker's move. Attacker's `blockstun_ticks`
  increases by the block hitlag.
- Block-cancel: if the hitbox allows it (and is not looping+uncancellable), both
  fighters get interrupt rights, with the attacker's state flagged
  `interruptible_on_opponent_turn`.
- Pushback force on both, with air modifier and reversible-direction rules.
- Block stun (`set_block_stun`) is computed from the move's `plus_frames`, plus base plus
  frames, wrong-height bonus, vs-aerial bonus, and a push-block penalty. Positive plus
  frames favor the defender and are stored on the defender; negative values are stored on the
  opponent. This is how frame advantage is expressed.

### Push block
A variant of the guard state (`push` flag): matches any hit height, shoves the opponent
away with a push force, and costs an advantage penalty.

## 5. Counter / brace

`CounterAttack` states spend one eighth of the burst meter and require burst available.
While `bracing`, a hit matching its type counts as a successful counter:
- Grab counter vs throws.
- High counter vs non-throw high or mid hits.
- Low counter vs non-throw low hits.
A successful counter gives the defender reduced hitstun and changes DI scaling.

## 6. Initiative / advantage

`get_advantage` decides who has "initiative", which feeds parry rules above:
- The fighter in a combo always has it.
- Otherwise it is whoever's `lowest_tick` is less than or equal to the opponent's (roughly,
  who acted earlier in the current exchange), or if the opponent just parried them.
- Both interruptible at once -> both have it.
- A whiff instant cancel forces it off, with a special case for the new parry.
`update_advantage` sets an effect flag on the transition into initiative.

## 7. Implications for a real-time version

1. **Parry timing is a chosen number.** In real time it becomes "how long ago did the
   player press block before the hit lands" with a tolerance window (classic parry timing).
   Block height becomes stick direction (down = low).
2. **Initiative is a turn concept.** Real time needs a replacement (frame advantage only,
   or a "who acted first" rule) since the perfect-parry rule is tied to it.
3. **Block-state synchronization is clever and portable.** Copying attacker length into the
   defender's block state is basically hit/block stun. A normal stun system covers it.
4. **Hurt/knockback math is simple and portable:** knockback = direction x strength + DI,
   hitstun counted in ticks, gravity/friction constants, ground/air hit states, wall
   slam, ground bounce.
5. **Simultaneous detection is already the model.** Detect first, apply second gives the same
   trade behavior fighting games use; keep it.
6. **Combo rules** (same-move knockback boost and hitstun cuts, proration, guts scaling,
   hitstun decay) live partly in `Hitbox`, `Fighter`, and `take_damage`, which I have not
   fully read.
7. **Clash is damage-threshold-based:** easy to port; keep it behind a toggle.

## 8. Open questions

- Values of constants referenced by name: `CLASH_DAMAGE_DIFF`, `MAX_GROUNDED_HITS`,
  `BASE_PLUS_FRAMES`, `PARRY_METER`, `GLOBAL_HITLAG_MODIIFER`, chip divisors, etc. (in
  `BaseChar.gd` header, not read).
- Full combo scaling and `hitstun_decay_combo_count` behavior.
- `ThrowTech`, `Grabbed`, throw states, and `WallSlam` details.
- How `lowest_tick` is maintained (it decides initiative).
- Whether DI/hitstun shaving is truly disabled in the shipped build or only stubbed in this
  decompile.
