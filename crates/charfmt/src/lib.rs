//! Draft character data format, v1.
//!
//! Files are RON. The types here are the schema: `ron::from_str::<CharacterFile>` loads a
//! file and [`CharacterFile::validate`] checks the rules a parser cannot express.
//!
//! Conventions
//! - One tick = 1/60 s. Frame numbers are **1-based** and spans are **inclusive**.
//! - [`Fx`] values are written as plain numbers (`1.5`, `-4`). The game converts them to its
//!   fixed-point type once, at load time; the simulation never sees floats.
//! - Boxes are in character space: +x points the way the character faces, +y is down, and the
//!   origin is the character's feet. `half_w`/`half_h` are half-extents around `(x, y)`.
//! - Every optional field has a default, so files only state what differs from it.

use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
pub const TICKS_PER_SECOND: u32 = 60;

pub type Tick = u16;

/// Authoring value for a fixed-point number.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fx(pub f64);

impl Default for Fx {
    fn default() -> Self {
        Fx(0.0)
    }
}

fn one() -> Fx {
    Fx(1.0)
}
fn yes() -> bool {
    true
}
fn idle() -> String {
    "idle".to_string()
}
fn standard() -> String {
    "standard".to_string()
}
fn default_buffer() -> Tick {
    6
}
fn mid() -> Height {
    Height::Mid
}
fn minus_one() -> i16 {
    -1
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Vec2(pub Fx, pub Fx);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub half_w: i32,
    pub half_h: i32,
}

/// Inclusive 1-based frame range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub from: Tick,
    pub to: Tick,
}

// ---------------------------------------------------------------------------------------------
// File and character
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterFile {
    pub format_version: u32,
    pub character: Character,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character {
    /// Stable lowercase id, e.g. `"ninja"`. Prefixes the ids of its moves.
    pub id: String,
    pub name: String,
    /// Shared state library this character inherits (idle, hurt, block...). Moves with the
    /// same id as a library move replace it.
    #[serde(default = "standard")]
    pub base_library: String,
    pub body: Body,
    /// Id of a registered Rust module that implements the character's unique mechanic.
    #[serde(default)]
    pub mechanic: Option<String>,
    /// Stances/modes the character can be in (the first is the starting one). Empty means the
    /// character has a single mode and moves ignore modes.
    #[serde(default)]
    pub modes: Vec<String>,
    pub moves: Vec<Move>,
    #[serde(default)]
    pub projectiles: Vec<Projectile>,
    /// The original's script for the fighter itself (its own variables and methods), run by the
    /// script interpreter. Only imported mods carry scripts.
    #[serde(default)]
    pub script: Option<StateScript>,
    /// The fighter's scene (`res://`-relative), for the nodes scripts reach (`$Sprite`...).
    #[serde(default)]
    pub scene: Option<String>,
    /// The nodes of the fighter's scene and of its objects' scenes, as scripts see them.
    #[serde(default)]
    pub scenes: Vec<SceneDef>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

/// Physics and size. Speeds and forces are in units per tick (per tick squared for gravity).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    pub max_health: i32,
    pub gravity: Fx,
    pub ground_friction: Fx,
    pub air_friction: Fx,
    pub max_ground_speed: Fx,
    pub max_air_speed: Fx,
    pub max_fall_speed: Fx,
    /// Air jumps/dashes available before landing.
    #[serde(default)]
    pub air_actions: u8,
    /// Pushes against the opponent and walls.
    pub collision_box: Rect,
    /// Where the character can be hit while standing.
    pub hurtbox: Rect,
    #[serde(default = "one")]
    pub damage_taken: Fx,
    #[serde(default = "one")]
    pub knockback_taken: Fx,
}

// ---------------------------------------------------------------------------------------------
// Moves
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Move {
    /// Unique within the character, lowercase snake_case, e.g. `"grounded_punch"`.
    pub id: String,
    pub name: String,
    /// Group memberships other moves cancel into, e.g. `["grounded", "attack", "light"]`.
    #[serde(default)]
    pub tags: Vec<String>,
    pub stance: Stance,
    /// How a player starts the move. `None` for moves only other moves or the engine enter
    /// (throw follow-ups, landing moves, hurt reactions...).
    #[serde(default)]
    pub input: Option<Input>,
    /// Choices the move takes when it starts (direction, distance, options...).
    #[serde(default)]
    pub params: Vec<Param>,
    /// When the move may be used at all.
    #[serde(default)]
    pub availability: Availability,
    /// Character modes in which the move is usable; empty means every mode.
    #[serde(default)]
    pub modes: Vec<String>,
    /// Mode the character switches to when the move starts.
    #[serde(default)]
    pub set_mode: Option<String>,
    pub timeline: Timeline,
    #[serde(default)]
    pub hitboxes: Vec<HitBox>,
    /// Grab choreography, for moves that hold the opponent.
    #[serde(default)]
    pub throw: Option<Throw>,
    /// Temporary replacements for the body's hurtbox (crouching, leaning, ducking...).
    #[serde(default)]
    pub hurt: Vec<HurtWindow>,
    #[serde(default)]
    pub physics: Physics,
    #[serde(default)]
    pub motion: Vec<MotionEvent>,
    #[serde(default)]
    pub spawns: Vec<Spawn>,
    #[serde(default)]
    pub defense: Vec<DefenseWindow>,
    #[serde(default)]
    pub cancels: Vec<Cancel>,
    /// Move entered when the timeline ends without a cancel.
    #[serde(default = "idle")]
    pub finish_into: String,
    #[serde(default)]
    pub cost: Cost,
    /// Id of a registered Rust behavior for logic the data cannot express.
    #[serde(default)]
    pub behavior: Option<String>,
    /// The original's state script, run by the interpreter alongside the data above. When set,
    /// the data holds only what the scene says; everything the script does happens live.
    #[serde(default)]
    pub script: Option<StateScript>,
    #[serde(default)]
    pub presentation: Presentation,
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stance {
    Ground,
    Air,
    Any,
}

/// How a player triggers the move in real time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub button: Button,
    #[serde(default)]
    pub direction: Direction,
    /// Ticks a press stays valid while the character cannot act yet.
    #[serde(default = "default_buffer")]
    pub buffer: Tick,
    /// Must the button be held (for charge/repeat moves)?
    #[serde(default)]
    pub hold: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Button {
    Light,
    Heavy,
    Special,
    Grab,
    Guard,
    Jump,
    Dash,
    Super,
    Burst,
    Taunt,
}

/// Conditions under which a move can be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Availability {
    /// Only while the user is comboing an opponent.
    pub combo_only: bool,
    /// Only while no combo is running.
    pub neutral_only: bool,
}

/// Replaces the body hurtbox for a span of frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HurtWindow {
    pub window: Span,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Direction {
    #[default]
    Any,
    Neutral,
    Forward,
    Back,
    Up,
    Down,
}

// ---------------------------------------------------------------------------------------------
// Parameters: choices a move takes beyond its button
// ---------------------------------------------------------------------------------------------

/// A choice the move needs when it starts: a direction, a distance, an option...
///
/// The data declares *what* is chosen and its default. *How* the player chooses it in real time
/// is the `binding`; an unbound parameter always uses its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// Snake_case name the move's behavior reads it by.
    pub name: String,
    pub kind: ParamKind,
    #[serde(default)]
    pub binding: Option<Binding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParamKind {
    /// A direction. `steps` limits it to eight directions or leaves it free; `arc` restricts it to
    /// an arc (degrees, 0 = forward, positive = down, so -90 is straight up).
    Direction {
        steps: DirSteps,
        /// With `Eight`: the directions that may be chosen; empty means all of them.
        #[serde(default)]
        allowed: Vec<Compass>,
        #[serde(default)]
        arc: Option<Arc>,
        default: Vec2,
    },
    /// A number in a range (distance, delay, height).
    Slider { min: Fx, max: Fx, default: Fx },
    /// On or off.
    Toggle { default: bool },
    /// One of several named options.
    Choice { options: Vec<String>, default: String },
    /// A whole number in a range (shot count, charge level).
    Count { min: u8, max: u8, default: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirSteps {
    Eight,
    Free,
}

/// The nine choices of an eight-way direction, relative to the way the character faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compass {
    Neutral,
    Forward,
    ForwardUp,
    Up,
    BackUp,
    Back,
    BackDown,
    Down,
    ForwardDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arc {
    pub center_deg: Fx,
    /// Total width of the arc.
    pub range_deg: Fx,
}

/// How a parameter is read from the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binding {
    /// From the stick direction when the move starts.
    Stick,
    /// From how long the button is held, up to `max_ticks`.
    Hold { max_ticks: Tick },
    /// On while this extra button is held.
    Button(Button),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    /// Frames before the first hit-capable frame.
    pub startup: Tick,
    /// Frames from the first to the last hit-capable frame (0 for moves with no hitbox).
    pub active: Tick,
    pub recovery: Tick,
    /// What ends the move. `Fixed` ends it after `total()` frames; the other two keep it
    /// running past the listed frames, which then only describe its opening.
    #[serde(default)]
    pub end: TimelineEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimelineEnd {
    #[default]
    Fixed,
    /// Runs until the character lands.
    OnLanding,
    /// Runs until the move's `behavior` ends it.
    Script,
}

impl Timeline {
    pub fn total(&self) -> Tick {
        self.startup + self.active + self.recovery
    }
    pub fn is_open_ended(&self) -> bool {
        self.end != TimelineEnd::Fixed
    }
    /// First and last hit-capable frame, 1-based, if the move has any.
    pub fn active_span(&self) -> Option<Span> {
        (self.active > 0).then(|| Span {
            from: self.startup + 1,
            to: self.startup + self.active,
        })
    }
}

/// Which parts of the body physics run while the move is active, and per-move overrides.
///
/// Per tick the engine does: gravity, then friction, then forces. Semantics measured on the
/// original library: `friction` multiplies horizontal speed by `1 - ground_or_air_friction /
/// matching_max_speed`; `friction_x` / `friction_y` multiply by `1 - value` (a value above 1
/// reverses the speed); gravity adds its step each tick while vertical speed is below the fall
/// cap and never slows a faster fall; `forces` adds queued forces and caps horizontal speed and
/// fall speed unless `uncapped_forces` is set.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Physics {
    pub friction: bool,
    pub gravity: bool,
    /// Gravity only applies from this frame on (the original's script starts it mid-move).
    pub gravity_from: Option<Tick>,
    /// Apply queued forces to velocity each tick.
    pub forces: bool,
    /// Skip the speed caps when applying forces.
    pub uncapped_forces: bool,
    /// Replace the body's gravity and fall cap for this move.
    pub gravity_override: Option<GravityOverride>,
    /// Replace the body's horizontal speed caps for this move.
    pub speed_caps: Option<SpeedCaps>,
    /// Extra horizontal friction coefficient applied every tick, on top of `friction`.
    pub friction_x: Option<Fx>,
    /// Same for vertical speed.
    pub friction_y: Option<Fx>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GravityOverride {
    pub gravity: Fx,
    pub fall_speed: Fx,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedCaps {
    pub ground: Fx,
    pub air: Fx,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionEvent {
    pub at: Tick,
    pub kind: MotionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MotionKind {
    /// Adds speed. `forward` is along the facing direction, `up` is screen-up.
    Impulse { forward: Fx, up: Fx },
    SetVelocity { forward: Fx, up: Fx },
    StopMomentum,
    /// Zero only the horizontal speed.
    StopHorizontal,
    /// Zero only the vertical speed.
    StopVertical,
}

/// A grab that holds the opponent and later releases them with a launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Throw {
    /// Where the held opponent is placed when the move starts, from the thrower's origin
    /// (+x forward, +y down).
    pub hold: (i32, i32),
    /// Hold positions that take over from a frame on (animation-driven), in frame order.
    #[serde(default)]
    pub keyframes: Vec<HoldKey>,
    #[serde(default)]
    pub release: Option<Release>,
    /// Turn the thrower around when the move starts.
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldKey {
    pub at: Tick,
    pub offset: (i32, i32),
}

/// The moment the opponent is let go and launched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub at: Tick,
    /// Where the opponent is placed as they are released.
    pub offset: (i32, i32),
    pub hit: ReleaseHit,
}

/// Launch data of a release. Same meaning as the matching `HitBox` fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseHit {
    pub damage: i32,
    #[serde(default)]
    pub damage_in_combo: Option<i32>,
    pub hitstun: Tick,
    pub knockback: Knockback,
    #[serde(default)]
    pub scaling: Scaling,
    #[serde(default)]
    pub effects: Effects,
    #[serde(default)]
    pub reaction: Reaction,
    #[serde(default = "one")]
    pub di_modifier: Fx,
    #[serde(default = "mid")]
    pub height: Height,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub at: Tick,
    pub projectile: String,
    /// Offset from the character origin (+x forward).
    pub offset: (i32, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefenseWindow {
    pub window: Span,
    pub kind: DefenseKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefenseKind {
    Invulnerable,
    StrikeInvulnerable,
    ThrowInvulnerable,
    /// Absorbs this many hits without being interrupted.
    Armor(u8),
}

/// Leave the move early and start another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    /// Tags of moves this move may be cancelled into.
    pub into: Vec<String>,
    pub when: CancelWhen,
    pub window: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelWhen {
    OnHit,
    OnBlock,
    OnHitOrBlock,
    OnWhiff,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Cost {
    pub super_bars: u8,
    pub air_action: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Presentation {
    /// Logical animation name; the art itself is not part of this format.
    pub anim: Option<String>,
    pub sounds: Vec<SoundCue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundCue {
    pub at: Tick,
    pub id: String,
}

// ---------------------------------------------------------------------------------------------
// Hit data
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HitBox {
    pub id: String,
    /// Boxes of one group hit a victim once between them (the original's hitbox group).
    #[serde(default)]
    pub group: i32,
    pub rect: Rect,
    /// Frames on which the box can hit. Must lie inside the move's active span.
    pub window: Span,
    /// Keep the box live from `window.from` until the move ends (ignores `window.to`).
    #[serde(default)]
    pub until_end: bool,
    /// Turns on and off inside the window (multi-hit sweeps).
    #[serde(default)]
    pub repeat: Option<Repeat>,
    /// Move the attacker enters when this box hits.
    #[serde(default)]
    pub followup: Option<String>,
    /// For throws: the move the thrower enters when the grab connects.
    #[serde(default)]
    pub throw_into: Option<String>,
    /// Special behaviour of the hit; `Normal` for an ordinary strike.
    #[serde(default)]
    pub kind: HitKind,
    pub damage: i32,
    /// Damage when the victim is already in a combo (before scaling), if different.
    #[serde(default)]
    pub damage_in_combo: Option<i32>,
    /// Damage floor after combo scaling.
    #[serde(default)]
    pub min_damage: Option<i32>,
    pub hitstun: Tick,
    #[serde(default)]
    pub hitstun_in_combo: Option<Tick>,
    pub hitlag: Hitlag,
    pub knockback: Knockback,
    #[serde(default)]
    pub pushback: Fx,
    pub height: Height,
    /// Frame advantage when blocked. The game derives blockstun from this and the frames the
    /// move has left after contact, so authors never compute blockstun by hand.
    #[serde(default)]
    pub block_advantage: i32,
    #[serde(default)]
    pub scaling: Scaling,
    #[serde(default)]
    pub effects: Effects,
    #[serde(default)]
    pub reaction: Reaction,
    #[serde(default)]
    pub flags: HitFlags,
    /// The original's free-form tag for scripts (`hitbox.misc_data`), e.g. a hit meant to set
    /// off the fighter's own summon.
    #[serde(default)]
    pub misc_data: Option<String>,
}

/// Frames on and off, repeating from the start of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repeat {
    pub active: Tick,
    pub inactive: Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hitlag {
    pub attacker: u8,
    pub victim: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Knockback {
    /// Direction relative to facing; normalized at load. `(1, -1)` is forward and up.
    pub dir: Vec2,
    pub strength: Fx,
    /// Flip the horizontal direction depending on which side of the hitbox the victim is on.
    #[serde(default)]
    pub reversible: bool,
    /// Where the direction comes from. `dir` is used when this is `Direction`.
    #[serde(default)]
    pub from: KnockbackFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KnockbackFrom {
    #[default]
    Direction,
    /// Pulls the victim toward the hitbox centre.
    TowardHitbox,
    /// Pushes the victim away from the hitbox centre.
    AwayFromHitbox,
}

/// What a hit does to the victim's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Reaction {
    /// Reaction when the victim is on the ground.
    pub grounded: ReactionKind,
    /// Reaction when the victim is in the air.
    pub aerial: ReactionKind,
    /// A launched victim must spend this many frames in the launch before landing can end it
    /// (negative means no minimum).
    #[serde(default = "minus_one")]
    pub min_launch_frames: i16,
}

impl Default for Reaction {
    fn default() -> Self {
        Reaction { grounded: ReactionKind::Stumble, aerial: ReactionKind::Launch, min_launch_frames: -1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactionKind {
    /// Slides along the ground (the default for a grounded victim).
    Stumble,
    /// Sent flying, even from the ground (the default for an airborne victim).
    Launch,
    /// Held in place by a grab.
    Held,
    /// Stunned and unable to act.
    Dizzy,
}

/// Special hit behaviours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HitKind {
    #[default]
    Normal,
    /// Damages a victim who is being held, without launching or freeing them. Ignores invulnerability.
    ThrowDamage,
    /// Combo breaker pushing both fighters apart.
    Burst,
    /// A burst used on offense (a cancel into a push).
    OffensiveBurst,
    /// Damage only: no hitstun and no launch.
    NoHitstun,
    /// Turns the victim around and reverses their horizontal speed.
    Flip,
    /// Reports contact to the move's behavior; does no damage or stun.
    Detect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Height {
    High,
    Mid,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Scaling {
    /// Extra combo-scaling steps this hit adds beyond the normal one. Negative values mean
    /// the combo scales less (combo starters that keep damage high).
    pub proration: i8,
    pub scale_combo: bool,
    /// Whether the hit adds to the combo counter. Grabs do not.
    pub counts_as_hit: bool,
}

impl Default for Scaling {
    fn default() -> Self {
        Scaling { proration: 0, scale_combo: true, counts_as_hit: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Effects {
    pub knockdown: Knockdown,
    /// A knockdown keeps the victim stunned until they land. Turn off for a fixed stun time.
    pub stun_until_landing: bool,
    /// A grounded victim bounces once off the floor.
    pub ground_bounce: bool,
    /// An airborne victim bounces off the floor.
    pub air_ground_bounce: bool,
    pub wall_slam: bool,
    pub guard_break: bool,
    pub throw: bool,
}

impl Default for Effects {
    fn default() -> Self {
        Effects {
            knockdown: Knockdown::None,
            stun_until_landing: true,
            ground_bounce: false,
            air_ground_bounce: false,
            wall_slam: false,
            guard_break: false,
            throw: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Knockdown {
    #[default]
    None,
    Soft,
    Hard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HitFlags {
    pub can_clash: bool,
    pub beats_throw: bool,
    pub hits_projectiles: bool,
    pub hits_grounded: bool,
    pub hits_air: bool,
    pub hits_downed: bool,
}

impl Default for HitFlags {
    fn default() -> Self {
        HitFlags {
            can_clash: yes(),
            beats_throw: yes(),
            hits_projectiles: yes(),
            hits_grounded: yes(),
            hits_air: yes(),
            hits_downed: false,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Projectiles and provenance
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projectile {
    pub id: String,
    /// Initial velocity (+x forward).
    pub velocity: Vec2,
    /// A push applied on the first tick, after friction (+x forward, +y down).
    #[serde(default)]
    pub force: Vec2,
    /// Friction per tick on the horizontal and vertical speed.
    #[serde(default)]
    pub friction: Vec2,
    #[serde(default)]
    pub gravity: Fx,
    /// Damages the opponent directly (no hit) while they stand on the ground within reach.
    #[serde(default)]
    pub zone: Option<Zone>,
    /// Another projectile (by id) that appears where this one ends, however it ends.
    #[serde(default)]
    pub on_disable: Option<String>,
    /// Keeps flying after it hits (it still hits each hitbox once).
    #[serde(default)]
    pub pierces: bool,
    /// Things that happen on one of its own ticks (the original's `_frame_N` hooks).
    #[serde(default)]
    pub events: Vec<ProjEvent>,
    /// Frames before it expires; 0 means it lives until its behavior (or a hit) removes it.
    pub lifetime: Tick,
    pub hitboxes: Vec<HitBox>,
    /// Id of a registered Rust behavior for steering, sticking, exploding...
    #[serde(default)]
    pub behavior: Option<String>,
    /// Imported mods: the object's scene (`res://`-relative), its own script, and its states
    /// with their scripts and hitboxes. The interpreter runs these like a fighter's moves.
    #[serde(default)]
    pub scene: Option<String>,
    #[serde(default)]
    pub script: Option<StateScript>,
    #[serde(default)]
    pub states: Vec<ObjectState>,
}

/// A patch of ground that burns whoever stands in it: every `every` ticks, if the opponent
/// is grounded, not invulnerable and within `half_width` of the projectile, they lose `damage`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub half_width: Fx,
    pub damage: i32,
    pub every: Tick,
}

/// Something that happens to a projectile when its own tick counter reaches `at`
/// (0 is its first tick after it appeared).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjEvent {
    pub at: Tick,
    pub kind: ProjEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProjEventKind {
    /// Change the friction coefficients from now on.
    SetFriction(Vec2),
    /// Remove the projectile.
    Disable,
}

/// One state of a scripted object: its scene node, script (with the node's values) and hitboxes,
/// whose windows count from the state's own first frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectState {
    pub node: String,
    pub script: StateScript,
    #[serde(default)]
    pub hitboxes: Vec<HitBox>,
}

/// A script of the original and the values its scene node gives the script's variables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateScript {
    /// `res://`-relative path of the `.gd` file.
    pub path: String,
    /// The node's name in the scene (the state name scripts refer to it by).
    #[serde(default)]
    pub node: String,
    #[serde(default)]
    pub vars: Vec<(String, ScriptVar)>,
    /// What the move's option widgets give the script as `data` when nothing is chosen.
    #[serde(default)]
    pub data: Option<ScriptVar>,
    /// Where the stick's direction goes in `data`: the key ("" for `data` itself) and the scale
    /// (1 for an eight-way choice, 100 for a free aim).
    #[serde(default)]
    pub stick: Option<(String, i64)>,
}

/// A scene value for a script variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScriptVar {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Vec2(f64, f64),
    List(Vec<ScriptVar>),
    Map(Vec<(String, ScriptVar)>),
    Color(f64, f64, f64, f64),
    /// A resource by `res://`-relative path (a scene, a texture...).
    Res(String),
}

/// A scene's nodes: what `$Path` and `get_node` reach, with the values the scene gives them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDef {
    pub path: String,
    pub nodes: Vec<NodeDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDef {
    /// Path from the scene's root ("." for the root, "Flip/Sprite"...).
    pub path: String,
    #[serde(default)]
    pub ty: String,
    #[serde(default)]
    pub props: Vec<(String, ScriptVar)>,
    /// An animated sprite's animations.
    #[serde(default)]
    pub anims: Vec<AnimDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimDef {
    pub name: String,
    pub frames: u32,
    pub fps: f32,
    #[serde(default)]
    pub looped: bool,
}

/// Where a file came from and which choices are still guesses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Provenance {
    pub source: String,
    pub assumptions: Vec<String>,
    pub todo: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

/// Lowercase letters, digits and underscores; may start with a digit (e.g. `3_combo`).
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn span_ok(s: Span, max: Tick) -> bool {
    s.from >= 1 && s.from <= s.to && s.to <= max
}

impl CharacterFile {
    /// Returns every problem found (empty means valid).
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        if self.format_version != FORMAT_VERSION {
            errs.push(format!(
                "format_version {} is not supported (expected {})",
                self.format_version, FORMAT_VERSION
            ));
        }
        let c = &self.character;
        if !is_id(&c.id) {
            errs.push(format!("character id {:?} must be lowercase snake_case", c.id));
        }
        let b = &c.body;
        if b.max_health <= 0 {
            errs.push("body.max_health must be positive".into());
        }
        for (name, v) in [
            ("gravity", b.gravity.0),
            ("ground_friction", b.ground_friction.0),
            ("air_friction", b.air_friction.0),
            ("max_ground_speed", b.max_ground_speed.0),
            ("max_air_speed", b.max_air_speed.0),
            ("max_fall_speed", b.max_fall_speed.0),
        ] {
            if v < 0.0 || !v.is_finite() {
                errs.push(format!("body.{name} must be a non-negative number"));
            }
        }
        for (name, r) in [("collision_box", b.collision_box), ("hurtbox", b.hurtbox)] {
            if r.half_w <= 0 || r.half_h <= 0 {
                errs.push(format!("body.{name} needs positive half sizes"));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for m in &c.moves {
            if !seen.insert(m.id.as_str()) {
                errs.push(format!("duplicate move id {:?}", m.id));
            }
            for e in m.validate(&c.projectiles) {
                errs.push(format!("move {:?}: {e}", m.id));
            }
        }
        // Modes used by moves must be declared on the character.
        for md in &c.modes {
            if !is_id(md) {
                errs.push(format!("mode {md:?} must be lowercase snake_case"));
            }
        }
        for m in &c.moves {
            let used = m.modes.iter().chain(m.set_mode.iter());
            for u in used {
                if !c.modes.iter().any(|d| d == u) {
                    errs.push(format!("move {:?}: mode {u:?} is not declared in character.modes", m.id));
                }
            }
        }
        // Links between moves of this file must resolve.
        for m in &c.moves {
            for h in &m.hitboxes {
                for (what, target) in [("followup", &h.followup), ("throw_into", &h.throw_into)] {
                    if let Some(t) = target {
                        if !seen.contains(t.as_str()) {
                            errs.push(format!("move {:?}: hitbox {:?} {what} names unknown move {t:?}", m.id, h.id));
                        }
                    }
                }
            }
        }
        for p in &c.projectiles {
            for h in &p.hitboxes {
                if h.damage < 0 {
                    errs.push(format!("projectile {:?}: negative damage", p.id));
                }
            }
        }
        errs
    }
}

impl Move {
    pub fn validate(&self, projectiles: &[Projectile]) -> Vec<String> {
        let mut errs = Vec::new();
        let listed = self.timeline.total();
        // Open-ended moves keep running past the listed frames, so only the lower bound of
        // a frame number can be checked for them.
        let total = if self.timeline.is_open_ended() { Tick::MAX } else { listed };
        if !is_id(&self.id) {
            errs.push("id must be lowercase snake_case".into());
        }
        if listed == 0 && !self.timeline.is_open_ended() {
            errs.push("timeline has zero frames".into());
        }
        for t in &self.tags {
            if !is_id(t) {
                errs.push(format!("tag {t:?} must be lowercase snake_case"));
            }
        }
        if self.input.as_ref().is_some_and(|i| i.buffer > 30) {
            errs.push("input.buffer above 30 ticks is almost certainly a mistake".into());
        }
        if self.availability.combo_only && self.availability.neutral_only {
            errs.push("availability: combo_only and neutral_only exclude each other".into());
        }
        if self.timeline.is_open_ended() && self.timeline.end == TimelineEnd::Script && self.behavior.is_none() && self.script.is_none() {
            errs.push("timeline.end is Script but the move has no behavior to end it".into());
        }
        for w in &self.hurt {
            if !span_ok(w.window, total) {
                errs.push(format!("hurt window {}..{} is invalid", w.window.from, w.window.to));
            }
            if w.rect.half_w <= 0 || w.rect.half_h <= 0 {
                errs.push("hurt window needs positive half sizes".into());
            }
        }
        let mut pnames = std::collections::HashSet::new();
        for p in &self.params {
            if !is_id(&p.name) {
                errs.push(format!("param {:?} must be lowercase snake_case", p.name));
            }
            if !pnames.insert(p.name.as_str()) {
                errs.push(format!("duplicate param {:?}", p.name));
            }
            match &p.kind {
                ParamKind::Slider { min, max, default } => {
                    if !(min.0 <= default.0 && default.0 <= max.0) {
                        errs.push(format!("param {:?}: default is outside min..max", p.name));
                    }
                }
                ParamKind::Count { min, max, default } => {
                    if !(*min <= *default && *default <= *max) {
                        errs.push(format!("param {:?}: default is outside min..max", p.name));
                    }
                }
                ParamKind::Choice { options, default } => {
                    if options.is_empty() {
                        errs.push(format!("param {:?}: no options", p.name));
                    } else if !options.contains(default) {
                        errs.push(format!("param {:?}: default {default:?} is not one of the options", p.name));
                    }
                }
                ParamKind::Direction { steps, allowed, arc, .. } => {
                    if let Some(a) = arc {
                        if a.range_deg.0 <= 0.0 || a.range_deg.0 > 360.0 {
                            errs.push(format!("param {:?}: arc range must be in (0, 360]", p.name));
                        }
                    }
                    if !allowed.is_empty() && *steps != DirSteps::Eight {
                        errs.push(format!("param {:?}: `allowed` only applies to eight-way directions", p.name));
                    }
                }
                _ => {}
            }
            match p.binding {
                Some(Binding::Hold { max_ticks: 0 }) => {
                    errs.push(format!("param {:?}: Hold binding needs max_ticks above 0", p.name));
                }
                Some(Binding::Stick) if !matches!(p.kind, ParamKind::Direction { .. }) => {
                    errs.push(format!("param {:?}: only a direction can be bound to the stick", p.name));
                }
                _ => {}
            }
        }
        if let Some(t) = &self.throw {
            let mut last = 0;
            for k in &t.keyframes {
                if k.at < 1 || k.at > total {
                    errs.push(format!("throw keyframe at {} is outside 1..={listed}", k.at));
                }
                if k.at <= last {
                    errs.push("throw keyframes must be in increasing frame order".into());
                }
                last = k.at;
            }
            if let Some(r) = &t.release {
                if r.at < 1 || r.at > total {
                    errs.push(format!("throw release at {} is outside 1..={listed}", r.at));
                }
                if r.hit.damage < 0 {
                    errs.push("throw release has negative damage".into());
                }
                let k = &r.hit.knockback;
                if k.dir.0 .0 == 0.0 && k.dir.1 .0 == 0.0 && k.strength.0 != 0.0 && k.from == KnockbackFrom::Direction {
                    errs.push("throw release has knockback strength but a zero direction".into());
                }
            }
        }
        let ph = &self.physics;
        if let Some(g) = ph.gravity_override {
            if g.gravity.0 < 0.0 || g.fall_speed.0 < 0.0 {
                errs.push("physics.gravity_override needs non-negative values".into());
            }
        }
        if let Some(c) = ph.speed_caps {
            if c.ground.0 < 0.0 || c.air.0 < 0.0 {
                errs.push("physics.speed_caps needs non-negative values".into());
            }
        }
        for (n, v) in [("friction_x", ph.friction_x), ("friction_y", ph.friction_y)] {
            if v.is_some_and(|f| !f.0.is_finite()) {
                errs.push(format!("physics.{n} must be a finite number"));
            }
        }
        match (self.timeline.active_span(), self.hitboxes.is_empty()) {
            (Some(_), true) => errs.push("timeline.active > 0 but there are no hitboxes".into()),
            (None, false) => errs.push("hitboxes present but timeline.active is 0".into()),
            _ => {}
        }
        let mut ids = std::collections::HashSet::new();
        for h in &self.hitboxes {
            if !ids.insert(h.id.as_str()) {
                errs.push(format!("duplicate hitbox id {:?}", h.id));
            }
            if let Some(a) = self.timeline.active_span() {
                let upper_ok = h.until_end || h.window.to <= a.to;
                if !span_ok(h.window, total) || h.window.from < a.from || !upper_ok {
                    errs.push(format!(
                        "hitbox {:?} window {}..{} is outside the active span {}..{}",
                        h.id, h.window.from, h.window.to, a.from, a.to
                    ));
                }
            }
            if h.until_end && !self.timeline.is_open_ended() && h.window.to != listed {
                // A fixed move ends at its last frame, so "until the end" must say so.
                errs.push(format!("hitbox {:?}: until_end on a fixed move must reach its last frame", h.id));
            }
            if let Some(r) = h.repeat {
                if r.active == 0 || r.inactive == 0 {
                    errs.push(format!("hitbox {:?}: repeat needs non-zero active and inactive frames", h.id));
                }
            }
            if h.rect.half_w < 0 || h.rect.half_h < 0 {
                errs.push(format!("hitbox {:?} has a negative half size", h.id));
            } else if (h.rect.half_w == 0 || h.rect.half_h == 0) && self.behavior.is_none() {
                // A zero-size box is a placeholder that a behavior resizes at runtime.
                errs.push(format!("hitbox {:?} has a zero size and the move has no behavior to resize it", h.id));
            }
            if h.damage < 0 {
                errs.push(format!("hitbox {:?} has negative damage", h.id));
            }
            if h.knockback.dir.0 .0 == 0.0 && h.knockback.dir.1 .0 == 0.0 && h.knockback.strength.0 != 0.0 {
                errs.push(format!("hitbox {:?} has knockback strength but a zero direction", h.id));
            }
        }
        for ev in &self.motion {
            if ev.at < 1 || ev.at > total {
                errs.push(format!("motion event at {} is outside 1..={listed}", ev.at));
            }
        }
        for s in &self.spawns {
            if s.at < 1 || s.at > total {
                errs.push(format!("spawn at {} is outside 1..={listed}", s.at));
            }
            if !projectiles.iter().any(|p| p.id == s.projectile) {
                errs.push(format!("spawn references unknown projectile {:?}", s.projectile));
            }
        }
        for d in &self.defense {
            if !span_ok(d.window, total) {
                errs.push(format!("defense window {}..{} is outside 1..={listed}", d.window.from, d.window.to));
            }
        }
        for c in &self.cancels {
            if !span_ok(c.window, total) {
                errs.push(format!("cancel window {}..{} is outside 1..={listed}", c.window.from, c.window.to));
            }
            if c.into.is_empty() {
                errs.push("cancel with an empty `into` list".into());
            }
        }
        errs
    }
}

/// Parses a RON document. Files start with `#![enable(implicit_some)]` so optional fields can
/// be written bare (`damage_in_combo: 10` instead of `Some(10)`).
pub fn parse(text: &str) -> Result<CharacterFile, ron::error::SpannedError> {
    ron::from_str(text)
}
