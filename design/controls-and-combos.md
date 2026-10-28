# Controls and combos

Fighters carry 32 to 151 moves. In the original they are picked from a menu; in real time each
one a player chooses needs a button, without asking players to map anything themselves.

## The automatic layout (`crates/sim/src/layout.rs`)

Every move is sorted first:

| Kind | What it is | Gets |
|---|---|---|
| choice | a move the original lets the player pick | a button |
| follow-up | continues one specific move (cancel into a rare tag) | that move's button again |
| movement | rolls, air dashes, runs, blocking | nothing: the engine's dash, jump and hold-back cover them |
| system | bursts, instant cancels, taunts | nothing (universal actions) |
| internal | hurt states, "success" states, throw continuations, landings, script-started moves | nothing |
| engine | idle, jump, dash, fall... | nothing |

Choices then go on a grid of five buttons (Light, Heavy, Special, Super = Heavy+Special, Grab)
by five directions (neutral, forward, down, up, back), per stance (ground, air) and mode:

- **Button:** the original's category (attack / special / super / grab). Attacks split by weight
  (startup, length, damage): the lighter half on Light, the rest on Heavy.
- **Direction:** the one the original's menu gave, else what the move does: anti-airs and
  launchers up, lows and dives down, lunges forward, retreats and counters back.
- **Variants share a slot:** moves with one name (a grounded, a combo-only, a cancel version) are
  one choice; the engine tries the situational one first and the first usable starts.
- **Modes:** a mode's own moves take free slots, else shadow a move of every mode while the
  fighter is in that mode.
- **Overflow:** a move takes its own button family first, then any free slot; what still does not
  fit goes to a reserve (none does today, for any of the nine fighters).

`yemi-layout CHARACTERS_DIR` writes `<id>_controls.ron` and the audit `<id>_moves.md`; the
library build and the mod importer run it for every fighter. Hand-edited controls (no
"generated" header, like `content/controls/ninja_controls.ron`) are kept. The pause menu's
**move list** shows every button of both fighters.

## Combo rules (`ComboRules` in `content/engine.ron`'s `combo`, all on by default)

- **Chains:** after a hit or a block, a move cancels into a stronger button's move: Light, then
  Heavy (or Grab), then Special, then Super; Light also into another Light.
- **True chains:** a chain always connects. The victim's hitstun stretches to cover the startup of
  the move chained into (also for a move's own on-hit cancels).
- **No repeats:** a move is used once per combo. The list only starts over once the opponent
  recovers. This bounds every combo, since chains connect for sure.
- **Jump cancel:** after a hit that launches, Jump cancels into a jump for an air combo.
- **Up attacks on the ground:** an attack pressed in the first frames of a jump starts the Up
  version instead.
- **Hustle Cancel:** Dash during any of your own moves, hit or whiff, for 50 meter. You are free
  at once, carried forward at dash speed; an opponent reeling from a hit freezes for 12 ticks, so
  a fresh starter can extend the combo.
- **Meter:** 300 max (three bars), +12 for landing a hit, +8 for taking one, +5 each on a block.
  A super costs 100 per bar (`cost.super_bars`).

`cargo test --release -p sim --test combos combo_finder -- --ignored --nocapture` (with
`YEMI_ROUTE=<id>`) searches for the longest combo these rules allow against a standing ninja, with
unlimited meter. It reaches 8 to 11 hits for most fighters, mostly through Hustle Cancels, which
real meter limits to about six.

## Not done yet

- Throw direction choice while holding (forward / back / down throws are internal for now).
- A loadout screen to swap reserve moves in (no fighter overflows today).
- Bursts (escaping a combo for meter) as counterplay to long combos.

## Controllers (`crates/app/src/pad.rs`)

Controllers use Bevy's built-in gamepad support (gilrs), which covers Xbox, PlayStation, Switch
Pro and generic pads on every desktop platform. Through Steam, Steam Input presents any
controller as a standard Xbox pad, which this reads like any other, so Steam's own Input API is
not needed. A pad and the keyboard can both play at once.

- **Default layout:** X/Square Light, Y/Triangle Heavy, B/Circle Special, A/Cross Jump, RB/R1 Grab,
  LB/L1 Dash and Hustle Cancel, RT/R2 Super (Heavy+Special in one press), Start pause, Back/Share
  training reset. The stick and the d-pad both move.
- **Combo feel:**
  - Jumping is on a button by default, so stick-up only aims an attack pressed with it (no
    accidental jumps on up-attacks). "Stick up jumps" restores tap jump.
  - The stick is 8-way with narrow diagonals, so a slightly-off forward still reads as forward.
    The dead zone and diagonal width are adjustable.
  - The Super button makes the Heavy+Special chord reliable.
  - In the sim, a press made during your own hitstop no longer expires before the freeze ends, so
    chains feel the same however early you press (keyboard too).
  - Rumble on hits: stronger for the one hit, scaled by damage.
- **Menus:** the d-pad, stick or arrow keys move the focus to the nearest widget in that direction
  (left / right change a focused value instead: sliders, choices, tabs), A confirms, B goes back,
  Start opens and closes the pause menu. The mouse hovers, clicks and drags.
- **Settings:** Options > Controller sets which pad each player uses, rebinds every action ("press a
  button..."), and sets the dead zone, diagonal width, tap jump and rumble, plus a reset.
