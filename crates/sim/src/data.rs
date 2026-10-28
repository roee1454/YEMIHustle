//! Loading: engine tuning, control schemes, and characters in the charfmt format.

use std::collections::HashMap;
use std::path::Path;

use charfmt::{Button, CharacterFile, Direction, Fx, Move, Rect, Stance};
use serde::Deserialize;

/// Engine-wide tuning: everything about fighters that is not part of a character's moves.
/// This is our own design data, not taken from the original game.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineConfig {
    /// The stage spans -stage_half_width..stage_half_width.
    pub stage_half_width: i32,
    /// Distance of each fighter from the centre at round start.
    pub start_distance: i32,
    pub walk_speed: Fx,
    pub back_walk_speed: Fx,
    /// Frames on the ground before a jump leaves.
    pub jump_squat: u16,
    pub jump_speed: Fx,
    pub jump_forward_speed: Fx,
    pub double_jump_speed: Fx,
    pub landing_lag: u16,
    pub dash_frames: u16,
    pub dash_speed: Fx,
    pub back_dash_speed: Fx,
    /// Attacks may cancel a dash from this frame on.
    pub dash_cancel_from: u16,
    /// Air dashes per jump (each a level dash of `air_dash_frames` at `air_dash_speed`).
    #[serde(default = "two")]
    pub air_dashes: u8,
    #[serde(default = "air_dash_frames")]
    pub air_dash_frames: u16,
    #[serde(default = "air_dash_speed")]
    pub air_dash_speed: Fx,
    pub crouch_hurtbox: Rect,
    /// Frames a pressed button stays valid while the fighter cannot act.
    pub input_buffer: u16,
    /// Block chip damage = damage / chip_divisor.
    pub chip_divisor: i32,
    pub block_pushback: Fx,
    pub hurt_gravity: Fx,
    pub hurt_fall_speed: Fx,
    pub hurt_air_friction: Fx,
    pub hurt_ground_friction: Fx,
    pub knockdown_frames: u16,
    pub getup_frames: u16,
    /// Combo scaling: damage falls quadratically to `min_stale` over `stale_steps` hits.
    pub stale_steps: i32,
    pub min_stale: Fx,
    /// Whether airborne fighters can block.
    pub air_block: bool,
    /// How moves chain into each other, and the meter that pays for supers and cancels.
    #[serde(default)]
    pub combo: ComboRules,
}

/// The engine's combo rules, on top of the cancels each move lists itself.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ComboRules {
    /// After a hit or block, a move cancels into a stronger button's move: Light -> Heavy ->
    /// Special -> Super (Light also into another Light). No move twice in one chain.
    pub chains: bool,
    /// A chain always connects: the victim's hitstun is stretched to cover the startup of the
    /// move chained into.
    pub true_chains: bool,
    /// After a hit that launches, Jump cancels the move into a jump (air combos).
    pub jump_cancel: bool,
    /// Meter a Hustle Cancel costs: Dash during any of your own moves, hit or whiff, cancels it
    /// into a dash. 0 turns it off.
    pub hustle_cancel: i32,
    /// Ticks a Hustle Cancel freezes an opponent who is reeling from a hit (room for a follow-up).
    pub hustle_freeze: u16,
    pub meter_max: i32,
    /// Meter one super bar is (a move's `cost.super_bars` costs that many).
    pub super_bar: i32,
    pub meter_on_hit: i32,
    pub meter_on_hurt: i32,
    pub meter_on_block: i32,
    /// Meter per 100 damage dealt and taken (the original: 1 per damage dealt and a third of
    /// that taken, at 125 a bar).
    pub meter_per_damage: i32,
    pub meter_per_damage_taken: i32,
    /// An attack pressed in the first frames of a jump starts the Up version on the ground.
    pub up_attack_frames: u16,
}

impl Default for ComboRules {
    fn default() -> Self {
        ComboRules {
            chains: true,
            true_chains: true,
            jump_cancel: true,
            hustle_cancel: 50,
            hustle_freeze: 12,
            meter_max: 900,
            super_bar: 100,
            meter_on_hit: 12,
            meter_on_hurt: 8,
            meter_on_block: 5,
            meter_per_damage: 80,
            meter_per_damage_taken: 27,
            up_attack_frames: 3,
        }
    }
}

impl EngineConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Which move a button press starts. Binds are tried in file order; within a bind, moves are
/// tried in order and the first one usable right now wins (mode, availability, stance).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlScheme {
    pub character: String,
    pub binds: Vec<Bind>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bind {
    pub button: Button,
    #[serde(default)]
    pub dir: Direction,
    pub stance: Stance,
    /// Only considered when a cancel looks for a move, never from neutral. For follow-ups.
    #[serde(default)]
    pub cancel_only: bool,
    pub moves: Vec<String>,
}

impl ControlScheme {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// A character ready to simulate.
#[derive(Debug, Clone)]
pub struct Character {
    pub file: CharacterFile,
    pub controls: ControlScheme,
    pub index: HashMap<String, usize>,
    /// Problems the format validator reported (the character still loads).
    pub warnings: Vec<String>,
    /// Per move: where it sits in a chain, from the button it is bound to (Light 1, Heavy 2,
    /// Grab 2, Special 3, Super 4; 0 when no neutral button starts it).
    pub rank: Vec<u8>,
}

impl Character {
    pub fn load(character: &Path, controls: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(character).map_err(|e| format!("{}: {e}", character.display()))?;
        let file = charfmt::parse(&text).map_err(|e| format!("{}: {e}", character.display()))?;
        let controls = ControlScheme::load(controls)?;
        Self::new(file, controls)
    }

    pub fn new(file: CharacterFile, controls: ControlScheme) -> Result<Self, String> {
        let warnings = file.validate();
        let index: HashMap<String, usize> =
            file.character.moves.iter().enumerate().map(|(i, m)| (m.id.clone(), i)).collect();
        if controls.character != file.character.id {
            return Err(format!(
                "control scheme is for {:?} but the character is {:?}",
                controls.character, file.character.id
            ));
        }
        let mut missing = Vec::new();
        for b in &controls.binds {
            for m in &b.moves {
                if !index.contains_key(m) {
                    missing.push(m.clone());
                }
            }
        }
        if !missing.is_empty() {
            return Err(format!("control scheme names unknown moves: {}", missing.join(", ")));
        }
        let mut rank = vec![0u8; file.character.moves.len()];
        for b in controls.binds.iter().filter(|b| !b.cancel_only) {
            let r = match b.button {
                Button::Light => 1,
                Button::Heavy | Button::Grab => 2,
                Button::Special => 3,
                Button::Super => 4,
                _ => 0,
            };
            for m in &b.moves {
                if let Some(&i) = index.get(m) {
                    rank[i] = rank[i].max(r);
                }
            }
        }
        Ok(Character { file, controls, index, warnings, rank })
    }

    pub fn mv(&self, idx: usize) -> &Move {
        &self.file.character.moves[idx]
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    pub fn modes(&self) -> &[String] {
        &self.file.character.modes
    }
}

fn two() -> u8 {
    2
}

fn air_dash_frames() -> u16 {
    12
}

fn air_dash_speed() -> Fx {
    Fx(9.0)
}
