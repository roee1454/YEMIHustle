//! Controllers, through Bevy's built-in gamepad support (gilrs: Xbox, PlayStation, Switch Pro
//! and generic pads; Steam Input's virtual pads look like Xbox pads). Each player gets a pad,
//! the bindings and stick handling live in the settings, and this module turns a pad's state
//! into the simulation's `Input` with the things that make combos comfortable on a pad: a
//! Super button that presses Heavy and Special together, an 8-way stick that favours straight
//! directions, and (by default) jumping on a button instead of on stick-up, so up-attacks do not
//! jump.

use bevy::input::gamepad::{Gamepad, GamepadButton, GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// What pad buttons can do, in the settings screen's order.
pub const ACTIONS: [&str; 9] = ["Light", "Heavy", "Special", "Grab", "Dash / Hustle", "Jump", "Super (Heavy+Special)", "Pause", "Training Reset"];

/// Buttons that can be bound, by the name the settings store.
pub const BUTTONS: [(&str, GamepadButton); 18] = [
    ("South", GamepadButton::South),
    ("East", GamepadButton::East),
    ("North", GamepadButton::North),
    ("West", GamepadButton::West),
    ("LeftTrigger", GamepadButton::LeftTrigger),
    ("RightTrigger", GamepadButton::RightTrigger),
    ("LeftTrigger2", GamepadButton::LeftTrigger2),
    ("RightTrigger2", GamepadButton::RightTrigger2),
    ("Select", GamepadButton::Select),
    ("Start", GamepadButton::Start),
    ("LeftThumb", GamepadButton::LeftThumb),
    ("RightThumb", GamepadButton::RightThumb),
    ("C", GamepadButton::C),
    ("Z", GamepadButton::Z),
    ("DPadUp", GamepadButton::DPadUp),
    ("DPadDown", GamepadButton::DPadDown),
    ("DPadLeft", GamepadButton::DPadLeft),
    ("DPadRight", GamepadButton::DPadRight),
];

pub fn button_of(name: &str) -> Option<GamepadButton> {
    BUTTONS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

pub fn name_of(b: GamepadButton) -> Option<&'static str> {
    BUTTONS.iter().find(|(_, x)| *x == b).map(|(n, _)| *n)
}

/// How a bound button reads on screen: Xbox name, PlayStation name.
pub fn label(name: &str) -> &'static str {
    match name {
        "South" => "A / Cross",
        "East" => "B / Circle",
        "North" => "Y / Triangle",
        "West" => "X / Square",
        "LeftTrigger" => "LB / L1",
        "RightTrigger" => "RB / R1",
        "LeftTrigger2" => "LT / L2",
        "RightTrigger2" => "RT / R2",
        "Select" => "Back / Share",
        "Start" => "Start / Options",
        "LeftThumb" => "L3",
        "RightThumb" => "R3",
        "C" => "C",
        "Z" => "Z",
        "DPadUp" => "D-pad up",
        "DPadDown" => "D-pad down",
        "DPadLeft" => "D-pad left",
        "DPadRight" => "D-pad right",
        "" => "-",
        _ => "?",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PadSettings {
    /// The button for each of `ACTIONS` (a `BUTTONS` name; empty for none).
    pub binds: Vec<String>,
    /// Stick travel ignored around the centre (0..1).
    pub deadzone: f64,
    /// How wide the stick's diagonals are (0.2 wide .. 0.6 narrow): narrower diagonals mean a
    /// forward or down that is a little off still reads straight.
    pub diagonal: f64,
    /// Stick-up jumps (as on the keyboard). Off: the Jump button jumps, and stick-up only
    /// aims attacks.
    pub tap_jump: bool,
    pub rumble: bool,
    /// Which connected pad each player uses (order of connection; 255 none).
    pub assign: [u8; 2],
}

impl Default for PadSettings {
    fn default() -> Self {
        PadSettings {
            binds: ["West", "North", "East", "RightTrigger", "LeftTrigger", "South", "RightTrigger2", "Start", "Select"].iter().map(|s| s.to_string()).collect(),
            deadzone: 0.35,
            diagonal: 0.45,
            tap_jump: false,
            rumble: true,
            assign: [0, 1],
        }
    }
}

impl PadSettings {
    pub fn bind(&self, action: usize) -> Option<GamepadButton> {
        self.binds.get(action).and_then(|n| button_of(n))
    }
}

/// The pads plugged in (in connection order) and which one each player has.
#[derive(Resource, Default, Clone, PartialEq)]
pub struct Pads {
    pub connected: Vec<(Entity, String)>,
    pub players: [Option<Entity>; 2],
}

impl Pads {
    /// How an assignment reads in the settings: "1: Xbox Controller", "none"...
    pub fn slot_name(&self, assign: u8) -> String {
        match self.connected.get(assign as usize) {
            Some((_, n)) => format!("{}: {n}", assign + 1),
            None if assign == 255 => "none".into(),
            None => format!("{} (not connected)", assign + 1),
        }
    }
}

/// Keeps `Pads` up to date with what is plugged in.
pub fn track(pads: Query<(Entity, Option<&Name>), With<Gamepad>>, settings: Res<crate::settings::Settings>, mut out: ResMut<Pads>) {
    let mut connected: Vec<(Entity, String)> = pads.iter().map(|(e, n)| (e, n.map(|n| n.as_str().to_string()).unwrap_or_else(|| "controller".into()))).collect();
    connected.sort_by_key(|(e, _)| *e);
    let a = settings.pad.assign;
    let pick = |k: u8| connected.get(k as usize).map(|(e, _)| *e);
    let next = Pads { players: [pick(a[0]), pick(a[1])], connected };
    if *out != next {
        *out = next;
    }
}

/// Per player: what the stick did last tick (for tap-jump off).
#[derive(Default, Clone, Copy)]
pub struct PadState {
    up_latched: bool,
    /// An attack button was held last tick (fresh presses are found against the sim's tick,
    /// which does not line up with rendered frames).
    attack_held: bool,
}

/// The stick (and d-pad) as 8-way directions: (left, right, up, down).
pub fn directions(g: &Gamepad, s: &PadSettings) -> (bool, bool, bool, bool) {
    let d = g.dpad();
    let mut out = (
        d.x < -0.5 || g.pressed(GamepadButton::DPadLeft),
        d.x > 0.5 || g.pressed(GamepadButton::DPadRight),
        d.y > 0.5 || g.pressed(GamepadButton::DPadUp),
        d.y < -0.5 || g.pressed(GamepadButton::DPadDown),
    );
    let v = g.left_stick();
    let len = v.length();
    if len > s.deadzone as f32 {
        // a component counts once it is this much of the push (0.38 is an even 8-way split)
        let t = s.diagonal as f32;
        let (x, y) = (v.x / len, v.y / len);
        out.0 |= x < -t;
        out.1 |= x > t;
        out.2 |= y > t;
        out.3 |= y < -t;
    }
    out
}

/// A pad's state as the simulation's input.
pub fn read(g: &Gamepad, s: &PadSettings, st: &mut PadState) -> sim::Input {
    let held = |a: usize| s.bind(a).is_some_and(|b| g.pressed(b));
    let (left, right, up, down) = directions(g, s);
    let sup = held(6);
    let mut i = sim::Input {
        left,
        right,
        up: false,
        down,
        light: held(0),
        heavy: held(1) || sup,
        special: held(2) || sup,
        grab: held(3),
        dash: held(4),
    };
    if s.tap_jump {
        i.up = up || held(5);
    } else {
        // stick-up aims an attack pressed with it (and keeps aiming while held), without jumping
        let attacking = (0..4).any(held) || held(6);
        let attack = attacking && !st.attack_held;
        st.attack_held = attacking;
        st.up_latched = up && (attack || st.up_latched);
        i.up = held(5) || st.up_latched;
    }
    i
}

/// A short rumble on a player's pad (hits landed and taken).
pub fn rumble(out: &mut MessageWriter<GamepadRumbleRequest>, pads: &Pads, player: usize, strong: f32, weak: f32, ms: u64) {
    if let Some(e) = pads.players[player] {
        out.write(GamepadRumbleRequest::Add {
            gamepad: e,
            intensity: GamepadRumbleIntensity { strong_motor: strong, weak_motor: weak },
            duration: std::time::Duration::from_millis(ms),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::gamepad::GamepadAxis;

    fn pad(buttons: &[GamepadButton], stick: (f32, f32)) -> Gamepad {
        let mut g = Gamepad::default();
        for b in buttons {
            g.digital_mut().press(*b);
        }
        g.analog_mut().set(GamepadAxis::LeftStickX, stick.0);
        g.analog_mut().set(GamepadAxis::LeftStickY, stick.1);
        g
    }

    #[test]
    fn stick_favours_straight_directions_and_respects_the_dead_zone() {
        let s = PadSettings::default();
        // inside the dead zone: nothing
        assert_eq!(directions(&pad(&[], (0.2, 0.1)), &s), (false, false, false, false));
        // mostly forward with a little down: still just forward
        assert_eq!(directions(&pad(&[], (0.9, -0.3)), &s), (false, true, false, false));
        // a real diagonal is both
        assert_eq!(directions(&pad(&[], (0.7, -0.7)), &s), (false, true, false, true));
        // the d-pad counts too
        assert_eq!(directions(&pad(&[GamepadButton::DPadLeft], (0.0, 0.0)), &s), (true, false, false, false));
    }

    #[test]
    fn super_button_presses_heavy_and_special_and_up_attacks_do_not_jump() {
        let s = PadSettings::default();
        let mut st = PadState::default();
        let i = read(&pad(&[GamepadButton::RightTrigger2], (0.0, 0.0)), &s, &mut st);
        assert!(i.heavy && i.special && !i.light);
        // stick up alone does not jump (tap jump is off by default)
        let mut st = PadState::default();
        assert!(!read(&pad(&[], (0.0, 1.0)), &s, &mut st).up);
        // the jump button jumps
        assert!(read(&pad(&[GamepadButton::South], (0.0, 0.0)), &s, &mut st).up);
        // stick up with a fresh attack aims it up, and keeps aiming while the stick stays up
        let mut st = PadState::default();
        let i = read(&pad(&[GamepadButton::West], (0.0, 1.0)), &s, &mut st);
        assert!(i.up && i.light);
        assert!(read(&pad(&[], (0.0, 1.0)), &s, &mut st).up);
        assert!(!read(&pad(&[], (0.0, 0.0)), &s, &mut st).up);
        // with tap jump on, stick up is up
        let tap = PadSettings { tap_jump: true, ..PadSettings::default() };
        assert!(read(&pad(&[], (0.0, 1.0)), &tap, &mut PadState::default()).up);
    }
}
