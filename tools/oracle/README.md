# Oracle: the original game as ground truth

`run_traces.py` runs your own copy of the original game headless (a copy of its engine and
pack in a work folder; nothing is changed in the game folder) with `trace.gd` as the main
scene. The script starts a match between two characters, queues one action for P1 and records
every object each tick: position, velocity, state, whether it was disabled, plus both fighters'
position and P2's health.

    python3 tools/oracle/run_traces.py /some/workdir          # all traces
    python3 tools/oracle/run_traces.py /some/workdir ninja_fireball
    cp /some/workdir/traces/*.json data/oracle/

The traces are the original's behavior and stay private (`data/oracle/`). The sim tests in
`crates/sim/tests/sim.rs` (`assert_matches_oracle`) replay a move and require the projectile's
position on every tick and P2's health to equal the trace, skipping when the files are absent.
Values that make them pass live in `crates/convert/src/verified.rs`.

Linux only for now (it uses the Linux engine binary). Specs (which move, which aim data) are
listed in `run_traces.py`.

## Status of the projectile ports

Matched tick for tick (position, spawn tick, hit tick, damage, disappearance) by tests in
`crates/sim/tests/sim.rs`:

| move | what the sim reproduces | tolerance |
|---|---|---|
| ninja `fireball`, `air_fireball` | shuriken flight, ground fizzle, hit | exact |
| mutant `caustic_spike` | push, friction change on tick 20, hitlag, removal on tick 38 | 1 px |
| mutant `shockwave` | walk at 3.5 a tick | 1 px |
| wizard `vile_clutch` | two hitboxes in two groups, 32 ticks | exact |
| wizard `spark_bomb` | float, arm, creep toward the opponent, explode | 1 px |
| robo `flamethrower` (and `flamethrower2`, same data) | ground fire burning 6 every 3 ticks for 120 ticks | exact |
| robo `missile` | three missiles, push and climb, explosion child, first 63 ticks | 1 px |

Recorded but not ported yet: ninja `grappling_hook`, robo `grenade` and `loic`, mutant
`gas_bomb` and `bite_throw` (no trace yet), cowboy `foresight_neutral`. Moves that need a mode
(`whip_wave`, `drive_boost`, `foresight`) have no trace either.

Engine facts the traces taught (all implemented in `crates/sim/src/projectiles.rs`):
objects tick before fighters; a new object first moves on the next tick; ground and walls are
checked at the start of a tick; friction stops applying below a speed of 0.05; overlaps are
tested at whole-pixel positions and count touching edges; hitboxes of one `group` hit a victim
once between them; a hitbox with `scale_combo: false` does not count as a combo hit; a projectile
freezes for its hitbox's attacker hitlag when it hits.
