# Modes, styles and intros

## Singleplayer modes (`Page::Modes`, `fight::Mode`)

Singleplayer opens a mode page: **vs CPU** (level easy / normal / hard, saved in settings),
**training**, and **versus** (two players at one keyboard, or the match settings' dummy). The
character select starts the chosen mode.

## The CPU (`crates/sim/src/cpu.rs`)

A CPU returns the buttons it holds each tick, like a player. It reads the character's control
scheme to know which button starts which move: its startup, its reach, and whether it is low,
an anti-air, a projectile or a grab. Then it:

- **Blocks** an attack that can reach it, after its reaction time. It crouches against lows.
- **Anti-airs** a jump-in with its fastest move that hits above the head.
- **Continues combos** on hit with the engine's chains (next stronger button, no repeats). On hard it
  extends them with Hustle Cancels.
- **In neutral**, attacks when a move reaches, grabs a blocking opponent up close, lows a standing
  guard, throws projectiles from afar, and walks, dashes and jumps in.

The level sets the reaction time (16 / 9 / 4 ticks), how often it thinks, and how often it
blocks, anti-airs, combos and attacks. A seeded generator makes it deterministic. `sim` tests
check that every level damages a standing dummy, that hard beats easy, and that a CPU match
replays the same.

## Training (`crates/app/src/training.rs`, pause menu > training options)

- **Dummy:** stand, crouch, jump, block all (crouching against lows), block after first hit, random
  block, CPU easy / normal / hard, or play recording.
- **Recording:** F6 starts recording, and your keys move the dummy while P1 stands. F6 stops. F7
  plays the recording once, and the "play recording" dummy loops it.
- **Wakeup and after hit:** none, jump, attack, backdash, block.
- **Refill and meters:** health refills once a combo is over, and there is never a K.O. Meter per
  player is infinite, normal or empty.
- **Reset position:** center, left corner, right corner, or swapped sides (R).
- **Speed:** 100 / 75 / 50 / 25%. Freeze and frame advance still work.
- **Readouts:** startup and length of your last move, on-hit / on-block advantage, combo hits and
  damage (and the best), and an input history in numpad notation. Hitboxes can be shown too.

## Styles (`style.rs`, `customize.rs`, `aura.rs`)

A style is the color the sprite's white parts take, an optional outline, a hit spark from the
original's ten, and up to three particle auras. An aura has a shape, where its particles start,
start and end colors with alpha, an amount, a lifetime, sizes, speed, direction, spread, gravity,
and whether it is in front of the fighter. Eight presets come built in. Player styles are saved as
RON in the config folder's `styles/`. The **Customize** screen edits them with a live preview.
Each player picks a style on the character select ("default style" by default). The options
"enable style colors / particle auras / custom hitsparks" turn each part off.

## Battle intros

Versus and CPU matches (and rematches) open with about 3 seconds of intro. An imported fighter
with a scripted "Start" state plays it (its own intro, with its sounds, effects and cut-ins);
any other strikes its intro pose (intro animation, else its taunt, else idle) and plays its
intro sound. The banner shows
the names, then READY, then FIGHT!. An attack button skips ahead. The round ends with K.O., TIME or
DRAW. Toggle the intros with "play battle intros".
