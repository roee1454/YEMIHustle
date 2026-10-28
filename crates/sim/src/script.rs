//! Running the original's scripts. The simulation is the engine: it implements, once, the API
//! the original's own base classes offer scripts (`host.apply_force`, `host.opponent`,
//! `queue_state_change`...), and every script runs against it, whichever character or mod it
//! comes from. Scripts run in the `gdscript` interpreter; the engine's base classes
//! ([`NATIVE_SCRIPTS`]) are what this module implements natively, falling back to their original
//! code for anything it does not.
//!
//! Hooks the engine calls on a move's state script, in the original's order: `_enter` when the
//! move starts; each tick `_frame_N_shared` and `_frame_N` (N = the tick, from 0) before the
//! move's physics and `_tick` after; `_exit` when the fighter leaves the move however it leaves.
//! `on_got_hit`, `_on_hit_something` and `on_got_blocked` follow contact, and `is_usable` is
//! asked before the move starts. A hook that returns a state name, or calls
//! `queue_state_change`, switches state.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use charfmt::{ScriptVar, StateScript};
use gdscript::{Error, Host, InstRef, Loader, ObjId, Value, Vm};

use crate::{apply_fric, apply_grav, integrate, Act, Fixed, FVec, World};

/// The engine's base classes: the simulation answers for them.
pub const NATIVE_SCRIPTS: [&str; 6] = [
    "framework/FiniteStateMachine/StateInterface.gd",
    "ObjectState.gd",
    "characters/states/CharState.gd",
    "obj/BaseObj.gd",
    "characters/BaseChar.gd",
    "projectile/BaseProjectile.gd",
];

/// Methods of the base classes whose work the simulation does itself: a script calling them
/// (`._tick()`) gets nothing more done.
pub(crate) const ENGINE_DONE: [&str; 30] = [
    "_enter", "_exit", "_tick", "_tick_before", "_tick_after", "_enter_shared", "_exit_shared", "_tick_shared",
    "_frame_0_shared", "init", "_ready", "_on_hit_something", "on_got_hit", "on_got_blocked", "on_got_blocked_by",
    "process_hitboxes", "process_hurtboxes", "update_sprite_frame", "spawn_exported_projectile", "setup_hitboxes",
    "setup_hurtboxes", "setup_audio", "terminate_hitboxes", "activate_hitbox", "deactivate_hitbox", "emit_signal",
    "connect", "play_enter_sfx", "update_property_list", "enable_interrupt",
];

/// The base fighter's own lifecycle methods, which a fighter script's override calls on its way
/// (`.tick()`, `.hit_by(hitbox)`): the engine does their work itself.
const BASE_FIGHTER_DONE: [&str; 12] = [
    "tick", "_ready", "init", "hit_by", "on_got_hit", "_on_hit_something", "process_extra", "on_parried", "on_state_started",
    "on_state_ended", "_process", "block_hitbox",
];

/// Calls that only change what is seen or heard. The simulation has nothing to do for them.
const PRESENTATION: [&str; 26] = [
    "stop_particles", "hide_particles", "show_particles", "create_speed_after_image", "grab_camera_focus", "release_camera_focus", "emote",
    "set_vibration",
    "_create_speed_after_image", "screen_bump", "super_effect", "ex_effect", "emote_if_should", "unlock_achievement",
    "set_camera_zoom", "tween_camera_zoom", "stop_fly_fx", "add_penalty", "gain_super_meter", "use_super_bar",
    "update_data", "set_snap_to_ground", "reset_hurtbox", "stop_sound", "set_color", "set_material",
];

const FIGHTER_BASE: u32 = 1;
const STATE_BASE: u32 = 1000;
const STATE_STRIDE: u32 = 100_000;
/// Stand-in states for what the engine runs itself (idle, hurt...).
const ENGINE_STATE_BASE: u32 = 900;

/// Fighter `i` as scripts see it.
pub(crate) fn fighter_value(i: usize) -> Value {
    Value::Obj(fighter_obj(i))
}

pub(crate) fn fighter_obj(i: usize) -> ObjId {
    ObjId(FIGHTER_BASE + i as u32)
}

fn state_obj(i: usize, idx: usize) -> ObjId {
    ObjId(STATE_BASE + i as u32 * STATE_STRIDE + idx as u32)
}

/// Stands in for nodes the simulation does not model (sprites, sounds, effects): takes any
/// assignment and call, gives null back.
pub(crate) const SINK: ObjId = ObjId(50);
/// A fighter's state machine (`host.state_machine`).
const MACHINE_BASE: u32 = 60;
/// The match (`Global.current_game`, the original's `Game` node).
const GAME: ObjId = ObjId(71);
/// A fighter's hurtbox (`host.hurtbox`).
const HURTBOX_BASE: u32 = 80;
/// A hitbox of a fighter's current move (`get_active_hitboxes()`): base + fighter * stride + box.
const HITBOX_BASE: u32 = 1_000_000;
const HITBOX_STRIDE: u32 = 10_000;

pub(crate) enum Target {
    Sink,
    Game,
    Hurtbox(usize),
    UiLayer,
    Obj(u32),
    ObjState(u32, usize),
    Node(usize),
    Machine(usize),
    Fighter(usize),
    State(usize, usize),
    EngineState(usize),
    /// Fighter, index into its current move's hitboxes.
    Hitbox(usize, usize),
    Other,
}

pub(crate) fn target_of(o: ObjId) -> Target {
    target(o)
}

fn target(o: ObjId) -> Target {
    if o == crate::objects::UI_LAYER {
        return Target::UiLayer;
    }
    match crate::objects::decode(o) {
        Some(crate::objects::ObjRef::Obj(u)) => return Target::Obj(u),
        Some(crate::objects::ObjRef::State(u, k)) => return Target::ObjState(u, k),
        Some(crate::objects::ObjRef::Node(i)) => return Target::Node(i),
        None => {}
    }
    match o.0 {
        x if x == SINK.0 => Target::Sink,
        x if x == GAME.0 => Target::Game,
        x if (HURTBOX_BASE..HURTBOX_BASE + 2).contains(&x) => Target::Hurtbox((x - HURTBOX_BASE) as usize),
        x if (MACHINE_BASE..MACHINE_BASE + 2).contains(&x) => Target::Machine((x - MACHINE_BASE) as usize),
        x if (FIGHTER_BASE..FIGHTER_BASE + 2).contains(&x) => Target::Fighter((x - FIGHTER_BASE) as usize),
        x if (ENGINE_STATE_BASE..ENGINE_STATE_BASE + 2).contains(&x) => Target::EngineState((x - ENGINE_STATE_BASE) as usize),
        x if (HITBOX_BASE..HITBOX_BASE + 2 * HITBOX_STRIDE).contains(&x) => {
            let r = x - HITBOX_BASE;
            Target::Hitbox((r / HITBOX_STRIDE) as usize, (r % HITBOX_STRIDE) as usize)
        }
        x if x >= STATE_BASE && x < STATE_BASE + 2 * STATE_STRIDE => {
            let r = x - STATE_BASE;
            Target::State((r / STATE_STRIDE) as usize, (r % STATE_STRIDE) as usize)
        }
        _ => Target::Other,
    }
}

/// What the interpreter keeps between calls, apart from the interpreter itself.
#[derive(Default)]
pub(crate) struct Tables {
    /// Finds scripts (for objects made while scripts run).
    pub(crate) loader: Option<Arc<dyn Loader>>,
    /// Scripted objects' script objects, by object uid.
    pub(crate) objs: HashMap<u32, crate::objects::ObjInsts>,
    /// Scene nodes scripts reached.
    pub(crate) nodes: Vec<crate::objects::NodeRec>,
    /// Per fighter, per move: the move's state object.
    pub(crate) states: [Vec<Option<InstRef>>; 2],
    /// Per fighter: its own script (the fighter's variables and methods).
    pub(crate) roots: [Option<InstRef>; 2],
    /// Per fighter: state (node) name -> move index.
    by_node: [HashMap<String, usize>; 2],
    /// Fighter properties scripts set that the simulation keeps no field for.
    extra: [HashMap<String, Value>; 2],
    /// The move whose `_enter` ran and whose `_exit` has not.
    entered: [Option<usize>; 2],
    pub(crate) problems: BTreeMap<String, u32>,
    /// The "Start" states have not run yet (the round has not begun).
    start_pending: bool,
    /// The intro is playing: where each fighter stood (and faced) when it began.
    pub(crate) intro: Option<[(crate::FVec, i64); 2]>,
    /// Per fighter: its last two states, oldest first (the state machine's `states_stack`).
    history: [Vec<ObjId>; 2],
}

pub struct Scripts {
    vm: Vm,
    pub(crate) t: Tables,
}

impl Scripts {
    /// Unsupported calls and script errors met so far, with how often (for reports and tests).
    pub fn problems(&self) -> &BTreeMap<String, u32> {
        &self.t.problems
    }
}

/// The original's `object_data` of a body: whole-pixel position, velocity as fixed-point text.
pub(crate) fn object_data(pos: crate::FVec, vel: crate::FVec) -> Value {
    Value::dict(vec![
        (Value::str("position_x"), Value::Int(pos.x.round())),
        (Value::str("position_y"), Value::Int(pos.y.round())),
        (Value::str("vel_x"), text(vel.x)),
        (Value::str("vel_y"), text(vel.y)),
    ])
}

/// `get_collision_box()`: the box's edges in world space.
pub(crate) fn collision_box(c: crate::FVec, h: crate::FVec) -> Value {
    Value::dict(vec![
        (Value::str("x1"), Value::Int((c.x - h.x).round())),
        (Value::str("x2"), Value::Int((c.x + h.x).round())),
        (Value::str("y1"), Value::Int((c.y - h.y).round())),
        (Value::str("y2"), Value::Int((c.y + h.y).round())),
    ])
}

pub(crate) fn var_value(v: &ScriptVar) -> Value {
    match v {
        ScriptVar::Bool(b) => Value::Bool(*b),
        ScriptVar::Int(i) => Value::Int(*i),
        ScriptVar::Float(x) => Value::Float(*x),
        ScriptVar::Str(s) => Value::str(s),
        ScriptVar::Vec2(x, y) => Value::Vec2(*x, *y),
        ScriptVar::List(l) => Value::array(l.iter().map(var_value).collect()),
        ScriptVar::Map(m) => Value::dict(m.iter().map(|(k, v)| (Value::str(k), var_value(v))).collect()),
        ScriptVar::Color(r, g, b, a) => Value::Color(*r, *g, *b, *a),
        ScriptVar::Res(p) => Value::Res(std::sync::Arc::from(p.as_str())),
    }
}

pub(crate) fn fixed_of(v: &Value) -> Result<Fixed, Error> {
    gdscript::fixed::raw(v).map(Fixed)
}

pub(crate) fn text(x: Fixed) -> Value {
    gdscript::fixed::text(x.0)
}

pub(crate) fn xy(x: Value, y: Value) -> Value {
    Value::dict(vec![(Value::str("x"), x), (Value::str("y"), y)])
}

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// The original's name for what the engine is running itself.
fn engine_state_name(a: &Act) -> &'static str {
    match a {
        Act::Idle => "Wait",
        Act::Walk { back: false } => "WalkForward",
        Act::Walk { back: true } => "WalkBackward",
        Act::Crouch => "Crouch",
        Act::JumpSquat { .. } => "Jump",
        Act::Air => "Fall",
        Act::Landing { .. } => "Landing",
        Act::Dash { dir, .. } => {
            if *dir < 0 {
                "DashBackward"
            } else {
                "DashForward"
            }
        }
        Act::Block { .. } => "ParryHigh",
        Act::Hurt { launched: true, .. } => "HurtAerial",
        Act::Hurt { .. } => "HurtGrounded",
        Act::Knockdown { .. } => "Knockdown",
        Act::Getup { .. } => "GetUp",
        Act::Held => "Grabbed",
        Act::Move(_) => "",
        Act::Ko => "Knockdown",
    }
}

impl World {
    /// Makes the script objects of both fighters' moves (for characters that carry scripts) and
    /// runs them from now on. `loader` finds the scripts (a mod's folder, then the game's) and
    /// should mark [`NATIVE_SCRIPTS`] as native. Returns problems met while setting up.
    pub fn attach_scripts(&mut self, loader: Arc<dyn Loader>) -> Vec<String> {
        let mut sc = Scripts { vm: Vm::shared(loader.clone()), t: Tables::default() };
        sc.t.loader = Some(loader);
        let mut errs = Vec::new();
        for i in 0..2 {
            let ch = self.chars[i].clone();
            let file = &ch.file.character;
            let has_scripts = file.script.is_some() || file.moves.iter().any(|m| m.script.is_some());
            if !has_scripts {
                // no scripts of its own, but the base fighter's members and class still answer
                // other fighters' scripts (`opponent.MAX_SUPER_METER`, `is Fighter`)
                // (not readied: it has no scene whose nodes its variables could name)
                if let Some(script) = sc.vm.loader().by_path("characters/BaseChar.gd") {
                    let mut ctx = Ctx { w: self, t: &mut sc.t };
                    sc.t.roots[i] = sc.vm.instantiate(&mut ctx, script, Some(fighter_obj(i))).ok();
                }
                sc.t.states[i] = vec![None; file.moves.len()];
                continue;
            }
            let root = file.script.clone().unwrap_or(StateScript { path: "characters/BaseChar.gd".into(), node: String::new(), vars: vec![], data: None, stick: None });
            sc.t.roots[i] = self.make_instance(&mut sc, i, &root, fighter_obj(i), &mut errs);
            for (idx, mv) in file.moves.iter().enumerate() {
                let inst = mv.script.as_ref().and_then(|s| {
                    sc.t.by_node[i].insert(s.node.clone(), idx);
                    self.make_instance(&mut sc, i, s, state_obj(i, idx), &mut errs)
                });
                sc.t.states[i].push(inst);
            }
        }
        sc.t.start_pending = true;
        self.scripts = Some(Box::new(sc));
        for i in 0..2 {
            self.root_hook(i, "init", vec![]);
            self.root_hook(i, "_ready", vec![]);
            // Godot readies every node: the states too
            let n = self.scripts.as_ref().map_or(0, |sc| sc.t.states[i].len());
            for idx in 0..n {
                if self.has_state_script(i, idx) {
                    self.state_hook(i, idx, "_ready", vec![]);
                }
            }
        }
        errs
    }

    fn start_state(&self, i: usize) -> Option<usize> {
        self.scripts.as_ref()?.t.by_node[i].get("Start").copied()
    }

    /// The original starts every round in the fighter's "Start" state: its intro, where mods
    /// also set things up (cut-ins on the UI layer...). A round without the intro still runs
    /// those states' setup hooks, on its first tick.
    pub(crate) fn start_setup(&mut self) {
        let Some(sc) = self.scripts.as_mut().filter(|sc| sc.t.start_pending) else { return };
        sc.t.start_pending = false;
        for i in 0..2 {
            if let Some(idx) = self.start_state(i) {
                for hook in ["_enter", "_frame_0"] {
                    self.state_hook(i, idx, hook, vec![]);
                }
            }
        }
    }

    /// The round opens with its intro: fighters whose scripts have a "Start" state play it
    /// whole (its animation, sounds, effects and setup) through [`World::step_intro`]. Returns
    /// whether either does.
    pub fn begin_intro(&mut self) -> bool {
        let Some(sc) = self.scripts.as_mut().filter(|sc| sc.t.start_pending) else { return false };
        sc.t.start_pending = false;
        let from = [0, 1].map(|i| (self.f[i].pos, self.f[i].facing));
        let starts = [0, 1].map(|i| self.start_state(i));
        for (i, s) in starts.iter().enumerate() {
            if let Some(idx) = s {
                self.start_move(i, *idx);
            }
        }
        let any = starts.iter().any(Option::is_some);
        if any {
            self.scripts.as_mut().unwrap().t.intro = Some(from);
        }
        any
    }

    /// Whether the intro's "Start" states are playing.
    pub fn in_intro(&self) -> bool {
        self.scripts.as_ref().is_some_and(|sc| sc.t.intro.is_some())
    }

    /// One tick of the intro: the fighters play their "Start" states with no input; nothing
    /// hits and the clock does not run. Does nothing when no fighter has one.
    pub fn step_intro(&mut self) {
        if self.in_intro() {
            self.step([crate::Input::default(); 2]);
        }
    }

    /// The intro is over (or skipped): the fighters leave their "Start" states and stand
    /// where the round begins.
    pub fn end_intro(&mut self) {
        let Some(from) = self.scripts.as_mut().and_then(|sc| sc.t.intro.take()) else { return };
        for i in 0..2 {
            if matches!(self.f[i].act, crate::Act::Move(_)) {
                self.script_leave(i);
            }
            let f = &mut self.f[i];
            f.act = crate::Act::Idle;
            (f.pos, f.facing) = from[i];
            f.vel = crate::FVec::ZERO;
            f.force = crate::FVec::ZERO;
            f.grounded = true;
            f.hitlag = 0;
            f.invuln = 0;
            f.buffered = None;
            f.queued = None;
            f.chain.clear();
            f.mode = 0;
        }
    }

    fn make_instance(&mut self, sc: &mut Scripts, i: usize, s: &StateScript, obj: ObjId, errs: &mut Vec<String>) -> Option<InstRef> {
        let Some(script) = sc.vm.loader().by_path(&s.path) else {
            errs.push(format!("{}: script not found", s.path));
            return None;
        };
        let mut ctx = Ctx { w: self, t: &mut sc.t };
        let inst = match sc.vm.instantiate(&mut ctx, script, Some(obj)) {
            Ok(inst) => inst,
            Err(e) => {
                errs.push(format!("{}: {e}", s.path));
                return None;
            }
        };
        {
            let mut w = inst.write().unwrap();
            let declared = |name: &str| w.chain.iter().any(|sc| sc.members.iter().any(|m| m.name == name));
            let sets: Vec<(String, Value)> = s.vars.iter().filter(|(k, _)| declared(k)).map(|(k, v)| (k.clone(), var_value(v))).collect();
            for (k, v) in sets {
                w.set(&k, v);
            }
            if !s.node.is_empty() {
                w.set("state_name", Value::str(&s.node));
            }
            w.set("host", Value::Obj(fighter_obj(i)));
            w.set("fixed", Value::Lib("fixed"));
        }
        // a node lookup that fails leaves that variable null: noted, not fatal
        if let Err(e) = sc.vm.ready(&mut Ctx { w: self, t: &mut sc.t }, &inst) {
            Self::note_problem(&mut sc.t, format!("{} (onready): {e}", s.path));
        }
        Some(inst)
    }

    pub(crate) fn note_problem(t: &mut Tables, what: String) {
        *t.problems.entry(what).or_insert(0) += 1;
    }

    /// Calls `name` on fighter `i`'s state object for move `idx`, if its script defines it.
    pub(crate) fn state_hook(&mut self, i: usize, idx: usize, name: &str, args: Vec<Value>) -> Option<Value> {
        let mut sc = self.scripts.take()?;
        let inst = sc.t.states[i].get(idx).cloned().flatten();
        let out = match inst {
            Some(inst) if sc.vm.has_script_method(&inst, name) => {
                let r = sc.vm.call(&mut Ctx { w: self, t: &mut sc.t }, &inst, name, args);
                match r {
                    Ok(v) => Some(v),
                    Err(e) => {
                        let path = inst.read().unwrap().chain.last().map(|s| s.path.clone()).unwrap_or_default();
                        Self::note_problem(&mut sc.t, format!("{path} {name}: {e}"));
                        None
                    }
                }
            }
            _ => None,
        };
        self.scripts = Some(sc);
        out
    }

    /// Runs a method the fighter's own script defines (an override of the base fighter's:
    /// `tick`, `on_got_hit`, `hit_by`...). The base fighter's part is the engine's.
    pub(crate) fn root_hook(&mut self, i: usize, name: &str, args: Vec<Value>) -> Option<Value> {
        let mut sc = self.scripts.take()?;
        let inst = sc.t.roots[i].clone();
        let out = match inst {
            Some(inst) if sc.vm.has_script_method(&inst, name) => match sc.vm.call(&mut Ctx { w: self, t: &mut sc.t }, &inst, name, args) {
                Ok(v) => Some(v),
                Err(e) => {
                    Self::note_problem(&mut sc.t, format!("fighter {name}: {e}"));
                    None
                }
            },
            _ => None,
        };
        self.scripts = Some(sc);
        out
    }

    pub(crate) fn has_state_script(&self, i: usize, idx: usize) -> bool {
        self.scripts.as_ref().is_some_and(|sc| sc.t.states[i].get(idx).is_some_and(Option::is_some))
    }

    /// A field of a move's state object (`anim_length`...), as the script last left it.
    fn state_field(&self, i: usize, idx: usize, name: &str) -> Option<Value> {
        let sc = self.scripts.as_ref()?;
        let inst = sc.t.states[i].get(idx)?.as_ref()?;
        let v = inst.read().unwrap().get(name);
        v
    }

    /// For a scripted move: Some(Some(n)) if it ends after n frames, Some(None) if only its script
    /// ends it (`endless`), None if the move has no script.
    pub(crate) fn script_length(&self, i: usize, idx: usize) -> Option<Option<u16>> {
        if !self.has_state_script(i, idx) {
            return None;
        }
        if self.state_field(i, idx, "endless").is_some_and(|v| v.truthy()) {
            return Some(None);
        }
        Some(self.state_field(i, idx, "anim_length").and_then(|v| v.as_i64()).map(|n| n.clamp(1, u16::MAX as i64) as u16))
    }

    /// Asks the move's script whether it can start now (after the engine's own checks passed).
    pub(crate) fn script_usable(&mut self, i: usize, idx: usize) -> bool {
        self.state_hook(i, idx, "is_usable", vec![]).map_or(true, |v| v.truthy())
    }

    /// The move started: run `_enter`.
    pub(crate) fn script_enter(&mut self, i: usize, idx: usize) {
        self.script_leave(i);
        if !self.has_state_script(i, idx) {
            return;
        }
        let data = self.move_data(i, idx);
        if let Some(sc) = &mut self.scripts {
            sc.t.entered[i] = Some(idx);
            if let Some(inst) = &sc.t.states[i][idx] {
                let mut w = inst.write().unwrap();
                w.set("data", data);
                w.set("current_tick", Value::Int(-1));
                w.set("current_real_tick", Value::Int(-1));
                w.set("active", Value::Bool(true));
                w.set("was_blocked", Value::Bool(false));
            }
        }
        if let Some(Value::Str(next)) = self.state_hook(i, idx, "_enter", vec![]) {
            self.f[i].queued = Some(next.to_string());
        }
    }

    /// The `data` a move starts with: its widgets' defaults, with the direction held on the stick
    /// (in screen space, as the original's aim widgets give it) when the move takes one.
    fn move_data(&self, i: usize, idx: usize) -> Value {
        let Some(sc) = &self.chars[i].mv(idx).script else { return Value::Nil };
        let Some(d) = &sc.data else { return Value::Nil };
        let data = var_value(d);
        let inp = self.f[i].input;
        let (h, v) = (inp.right as i64 - inp.left as i64, inp.down as i64 - inp.up as i64);
        if let Some((key, scale)) = sc.stick.as_ref().filter(|_| h != 0 || v != 0) {
            // a free aim held diagonally points at 45 degrees, at full length
            let k = if *scale > 1 && h != 0 && v != 0 { scale * 71 / 100 } else { *scale };
            let target = if key.is_empty() {
                data.clone()
            } else {
                match &data {
                    Value::Dict(m) => m.read().unwrap().iter().find(|(kk, _)| kk.to_text() == *key).map(|(_, v)| v.clone()).unwrap_or(Value::Nil),
                    _ => Value::Nil,
                }
            };
            if let Value::Dict(m) = target {
                let mut m = m.write().unwrap();
                for (kk, vv) in m.iter_mut() {
                    match kk.to_text().as_str() {
                        "x" => *vv = Value::Int(h * k),
                        "y" => *vv = Value::Int(v * k),
                        _ => {}
                    }
                }
            }
        }
        data
    }

    /// The fighter left the move it entered last (if it did): run `_exit` and end what the
    /// original ends on every state change.
    pub(crate) fn script_leave(&mut self, i: usize) {
        let Some(idx) = self.scripts.as_mut().and_then(|sc| sc.t.entered[i].take()) else { return };
        self.state_hook(i, idx, "_exit", vec![]);
        if let Some(inst) = self.scripts.as_ref().and_then(|sc| sc.t.states[i][idx].clone()) {
            inst.write().unwrap().set("active", Value::Bool(false));
        }
        let f = &mut self.f[i];
        f.script_invuln = false;
        f.proj_invuln = false;
        f.throw_invuln = false;
        f.hyper_armor = false;
        f.proj_armor = false;
    }

    /// After a tick: leave the scripted move if the fighter is no longer in it.
    pub(crate) fn script_sync(&mut self, i: usize) {
        let now = match &self.f[i].act {
            Act::Move(run) => state_obj(i, run.idx),
            _ => ObjId(ENGINE_STATE_BASE + i as u32),
        };
        if let Some(sc) = self.scripts.as_mut() {
            let h = &mut sc.t.history[i];
            if h.last() != Some(&now) {
                h.push(now);
                if h.len() > 2 {
                    h.remove(0);
                }
            }
        }
        let Some(idx) = self.scripts.as_ref().and_then(|sc| sc.t.entered[i]) else { return };
        let still = matches!(&self.f[i].act, Act::Move(run) if run.idx == idx);
        if !still {
            self.script_leave(i);
        }
    }

    /// The frame hooks for the tick just started (`_frame_N_shared`, `_frame_N`).
    pub(crate) fn script_frame(&mut self, i: usize, idx: usize, frame: u16) {
        if !self.has_state_script(i, idx) {
            return;
        }
        let tick = frame as i64 - 1;
        if let Some(inst) = self.scripts.as_ref().and_then(|sc| sc.t.states[i][idx].clone()) {
            let mut w = inst.write().unwrap();
            w.set("current_tick", Value::Int(tick));
            w.set("current_real_tick", Value::Int(tick));
        }
        // the original's states pick their sprite's frame each tick; a script may do it itself
        // (`host.sprite.frame = ...`), which shows for this tick
        self.state_hook(i, idx, "update_sprite_frame", vec![]);
        for name in [format!("_frame_{tick}_shared"), format!("_frame_{tick}")] {
            if let Some(Value::Str(next)) = self.state_hook(i, idx, &name, vec![]) {
                self.f[i].queued = Some(next.to_string());
                return;
            }
        }
    }

    /// `_tick`, after the move's physics.
    pub(crate) fn script_tick(&mut self, i: usize, idx: usize) {
        for name in ["_tick_before", "_tick", "_tick_after"] {
            if let Some(Value::Str(next)) = self.state_hook(i, idx, name, vec![]) {
                self.f[i].queued = Some(next.to_string());
                return;
            }
        }
    }

    /// The move index a state name refers to, for this fighter.
    pub(crate) fn state_by_name(&self, i: usize, name: &str) -> Option<usize> {
        if let Some(idx) = self.scripts.as_ref().and_then(|sc| sc.t.by_node[i].get(name).copied()) {
            return Some(idx);
        }
        self.chars[i].find(&crate::state_id(name))
    }
}

/// The host scripts see: the world, plus the interpreter's tables.
pub(crate) struct Ctx<'a> {
    pub(crate) w: &'a mut World,
    pub(crate) t: &'a mut Tables,
}

impl Ctx<'_> {
    fn unsupported(&mut self, what: String) -> Error {
        World::note_problem(self.t, format!("unsupported: {what}"));
        Error::Unsupported(what)
    }

    fn current_state(&self, i: usize) -> Value {
        match &self.w.f[i].act {
            Act::Move(run) => Value::Obj(state_obj(i, run.idx)),
            _ => Value::Obj(ObjId(ENGINE_STATE_BASE + i as u32)),
        }
    }

    fn opp_of(&self, v: &Value) -> Option<usize> {
        match v {
            Value::Obj(o) => match target(*o) {
                Target::Fighter(j) => Some(j),
                _ => None,
            },
            _ => None,
        }
    }

    fn hurt_center(&self, i: usize) -> FVec {
        self.w.hurtboxes(i).first().map(|(c, _)| *c).unwrap_or(self.w.f[i].pos)
    }

    fn fighter_get(&mut self, i: usize, name: &str) -> Result<Value, Error> {
        let o = 1 - i;
        let f = &self.w.f[i];
        Ok(match name {
            "opponent" => Value::Obj(fighter_obj(o)),
            "id" => Value::Int(i as i64 + 1),
            "name" | "obj_name" => Value::str(&format!("P{}", i + 1)),
            "hp" => Value::Int(f.hp as i64),
            "MAX_HEALTH" | "max_hp" => Value::Int(f.max_hp as i64),
            "combo_count" => Value::Int(self.w.f[o].combo_hits as i64),
            "combo_damage" => Value::Int(self.w.f[o].combo_damage as i64),
            "is_ghost" | "initiative" | "feinting" | "reverse_state" | "disabled" | "state_interruptable" | "state_hit_cancellable" => {
                return self.extra_or(i, name, Value::Bool(false));
            }
            "gravity_enabled" => Value::Bool(true),
            "creator" => Value::Nil,
            "stage_width" => Value::Int(self.w.cfg.stage_half_width as i64),
            "current_tick" => Value::Int(self.w.frame as i64),
            "hitlag_ticks" => Value::Int(f.hitlag as i64),
            "air_movements_left" => Value::Int(f.air_actions as i64),
            "has_hyper_armor" => Value::Bool(f.hyper_armor),
            "has_projectile_armor" => Value::Bool(f.proj_armor),
            "invulnerable" => Value::Bool(f.script_invuln || f.invuln > 0),
            "projectile_invulnerable" => Value::Bool(f.proj_invuln),
            "throw_invulnerable" => Value::Bool(f.throw_invuln),
            "busy_interrupt" => Value::Bool(false),
            "colliding_with_opponent" => {
                let b = |k: usize| crate::world_rect(&self.w.f[k], &self.w.chars[k].file.character.body.collision_box);
                Value::Bool(crate::overlap(&b(i), &b(o)))
            }
            "stance" => Value::str(self.w.mode_name(i).unwrap_or("Normal")),
            "objs_map" => self.objs_map(),
            // the original's snapshot of the body (`obj_data.position_x`, `data.object_data`)
            "obj_data" => object_data(f.pos, f.vel),
            "data" => Value::dict(vec![(Value::str("object_data"), object_data(f.pos, f.vel))]),
            "state_machine" => Value::Obj(ObjId(MACHINE_BASE + i as u32)),
            "sprite" => self.find_node(Some(fighter_obj(i)), "Flip/Sprite").or_else(|| self.find_node(Some(fighter_obj(i)), "Sprite")).unwrap_or(Value::Obj(SINK)),
            // the values the original saves for rollback (intros copy the opponent's)
            "state_variables" => Value::array(vec![]),
            "hurtbox" => Value::Obj(ObjId(HURTBOX_BASE + i as u32)),
            "fixed" => Value::Lib("fixed"),
            _ => {
                if let Some(v) = self.t.extra[i].get(name) {
                    return Ok(v.clone());
                }
                // what the base fighter (or the fighter's own script) declares and the engine does
                // not model: its constants and the variables' values
                return self.root_member(i, name).ok_or_else(|| Error::Unsupported(format!("host.{name}")));
            }
        })
    }

    fn root_member(&mut self, i: usize, name: &str) -> Option<Value> {
        let inst = self.t.roots[i].clone()?;
        let loader = self.t.loader.clone()?;
        Vm::shared(loader).script_member(self, &inst, name)
    }

    /// Fighter `i`'s current move's hitboxes as script objects (live ones only).
    fn active_hitboxes(&self, i: usize) -> Value {
        let Act::Move(run) = &self.w.f[i].act else { return Value::array(vec![]) };
        let mv = self.w.chars[i].mv(run.idx);
        let live = mv.hitboxes.iter().enumerate().filter(|(_, hb)| crate::hitbox_live(hb, run.frame).is_some());
        Value::array(live.map(|(k, _)| Value::Obj(ObjId(HITBOX_BASE + i as u32 * HITBOX_STRIDE + k as u32))).collect())
    }

    /// A hitbox of fighter `i`'s current move, if it still has that move.
    fn hitbox_of(&self, i: usize, k: usize) -> Option<(charfmt::HitBox, u16)> {
        let Act::Move(run) = &self.w.f[i].act else { return None };
        let hb = self.w.chars[i].mv(run.idx).hitboxes.get(k)?.clone();
        let cycle = crate::hitbox_live(&hb, run.frame).unwrap_or(0);
        Some((hb, cycle))
    }

    /// Puts fighter `i` in the engine's hitstun (`change_state("HurtGrounded", {hitbox})` from
    /// a script): for `data.hitbox.hitstun_ticks` ticks, else a short stun.
    fn script_hurt(&mut self, i: usize, aerial: bool, data: &Value) {
        let hb = crate::objects::key(data, "hitbox");
        let stun = crate::objects::key(&hb, "hitstun_ticks").as_i64().unwrap_or(15).clamp(1, 600) as u16;
        let f = &mut self.w.f[i];
        let launched = aerial || !f.grounded;
        f.act = Act::Hurt { stun, launched, kd: charfmt::Knockdown::None, until_landing: false, min_air: 0, frame: 0 };
        f.buffered = None;
    }

    /// What `is <class>` answers for engine objects: the original's class tree.
    fn is_class(&mut self, obj: ObjId, class: &str) -> bool {
        let (base, hurt): (&[&str], bool) = match target(obj) {
            Target::Fighter(_) => (&["Fighter", "BaseChar", "BaseObj", "Node2D", "Node", "Object"], false),
            Target::State(..) => (&["CharacterState", "ObjectState", "StateInterface", "Node", "Object"], false),
            Target::EngineState(i) => (&["CharacterState", "ObjectState", "StateInterface", "Node", "Object"], matches!(self.w.f[i].act, Act::Hurt { .. } | Act::Knockdown { .. } | Act::Held)),
            Target::Obj(_) => (&["BaseProjectile", "BaseObj", "Node2D", "Node", "Object"], false),
            Target::ObjState(..) => (&["ObjectState", "StateInterface", "Node", "Object"], false),
            Target::Hitbox(..) => (&["Hitbox", "CollisionBox", "Node2D", "Node", "Object"], false),
            Target::Hurtbox(_) => (&["CollisionBox", "Node2D", "Node", "Object"], false),
            Target::Node(n) => {
                let ty = self.node_type(n);
                return ty.as_deref() == Some(class) || matches!(class, "Node" | "Node2D" | "CanvasItem" | "Object");
            }
            _ => (&["Node", "Object"], false),
        };
        if base.contains(&class) || (hurt && class == "CharacterHurtState") {
            return true;
        }
        // a script's own classes (`is MyState`)
        match (self.script_inner(obj), self.t.loader.clone()) {
            (Some(inst), Some(l)) => Vm::shared(l).is_a(&inst, class),
            _ => false,
        }
    }

    fn extra_or(&self, i: usize, name: &str, d: Value) -> Result<Value, Error> {
        Ok(self.t.extra[i].get(name).cloned().unwrap_or(d))
    }

    fn fighter_set(&mut self, i: usize, name: &str, v: Value) -> Result<(), Error> {
        let f = &mut self.w.f[i];
        match name {
            "hp" => f.hp = v.as_i64().unwrap_or(f.hp as i64) as i32,
            "has_hyper_armor" => f.hyper_armor = v.truthy(),
            "has_projectile_armor" => f.proj_armor = v.truthy(),
            "invulnerable" => f.script_invuln = v.truthy(),
            "projectile_invulnerable" => f.proj_invuln = v.truthy(),
            "throw_invulnerable" => f.throw_invuln = v.truthy(),
            "hitlag_ticks" => f.hitlag = v.as_i64().unwrap_or(0).clamp(0, u16::MAX as i64) as u16,
            "air_movements_left" => f.air_actions = v.as_i64().unwrap_or(0).clamp(0, u8::MAX as i64) as u8,
            "opponent" | "id" | "name" => return Err(Error::runtime(format!("host.{name} is read-only"))),
            // the engine keeps the clock
            "current_tick" => {}
            _ => {
                self.t.extra[i].insert(name.to_string(), v);
            }
        }
        Ok(())
    }

    fn fighter_call(&mut self, i: usize, name: &str, args: &[Value]) -> Result<Value, Error> {
        if PRESENTATION.contains(&name) || BASE_FIGHTER_DONE.contains(&name) {
            return Ok(Value::Nil);
        }
        match name {
            "play_sound" => {
                self.play_sound(crate::objects::NodeOwner::Fighter(i), &arg(args, 0).to_text());
                return Ok(Value::Nil);
            }
            "spawn_particle_effect" => return Ok(self.effect(args, None)),
            "spawn_particle_effect_relative" => {
                let at = self.w.f[i].pos;
                return Ok(self.effect(args, Some(at)));
            }
            "spawn_object" => {
                let scene = match args.first() {
                    Some(Value::Res(s)) => s.to_string(),
                    _ => return Err(self.unsupported("spawn_object of a scene that is not a resource".into())),
                };
                let f = &self.w.f[i];
                let (x, y) = (arg(args, 1).as_i64().unwrap_or(0), arg(args, 2).as_i64().unwrap_or(0));
                let relative = args.get(3).is_none_or(|v| v.truthy());
                let at = if relative { FVec::new(f.pos.x + Fixed::from_int(x * f.facing), f.pos.y + Fixed::from_int(y)) } else { FVec::new(Fixed::from_int(x), Fixed::from_int(y)) };
                let (facing, from) = (f.facing, match &f.act {
                    Act::Move(r) => r.idx,
                    _ => 0,
                });
                return Ok(self.spawn_scene(i, &scene, at, facing, from, arg(args, 4)).unwrap_or(Value::Nil));
            }
            "obj_from_name" => return Ok(self.obj_from_name(&arg(args, 0).to_text())),
            "randi_range" => return Ok(self.randi_range(arg(args, 0).as_i64().unwrap_or(0), arg(args, 1).as_i64().unwrap_or(0))),
            "get_active_hitboxes" => return Ok(self.active_hitboxes(i)),
            "get_hurtbox_center_float" => {
                let c = self.hurt_center(i);
                return Ok(Value::Vec2(c.x.to_f32() as f64, c.y.to_f32() as f64));
            }
            "get_hurtbox_center" => {
                let c = self.hurt_center(i);
                return Ok(xy(Value::Int(c.x.round()), Value::Int(c.y.round())));
            }
            "hurtbox_pos_relative_float" => {
                let c = self.hurt_center(i) - self.w.f[i].pos;
                return Ok(Value::Vec2((c.x.to_f32() * self.w.f[i].facing as f32) as f64, c.y.to_f32() as f64));
            }
            _ => {}
        }
        let o = 1 - i;
        let fac = self.w.f[i].facing;
        let body = self.w.chars[i].file.character.body.clone();
        let fx2 = |a: &[Value]| -> Result<(Fixed, Fixed), Error> { Ok((fixed_of(&arg(a, 0))?, fixed_of(&arg(a, 1))?)) };
        let f = &mut self.w.f[i];
        Ok(match name {
            "get_facing_int" => Value::Int(fac),
            "get_facing" => Value::str(if fac < 0 { "Left" } else { "Right" }),
            "set_facing" => {
                let d = arg(args, 0).as_i64().unwrap_or(1);
                f.facing = if d == -1 { -1 } else { 1 };
                Value::Nil
            }
            "turn_around" => {
                f.facing = -f.facing;
                Value::Nil
            }
            "get_pos" => xy(Value::Int(f.pos.x.round()), Value::Int(f.pos.y.round())),
            "get_collision_box" => {
                let (c, h) = crate::world_rect(&self.w.f[i], &self.w.chars[i].file.character.body.collision_box);
                collision_box(c, h)
            }
            "get_pos_visual" | "get_center_position_float" => Value::Vec2(f.pos.x.to_f32() as f64, f.pos.y.to_f32() as f64),
            "get_vel" => xy(text(f.vel.x), text(f.vel.y)),
            "set_pos" => {
                let (x, y) = fx2(args)?;
                f.pos = FVec::new(x, y);
                Value::Nil
            }
            "set_x" => {
                f.pos.x = fixed_of(&arg(args, 0))?;
                Value::Nil
            }
            "set_y" => {
                f.pos.y = fixed_of(&arg(args, 0))?;
                Value::Nil
            }
            "set_vel" => {
                let (x, y) = fx2(args)?;
                f.vel = FVec::new(x, y);
                Value::Nil
            }
            "apply_force" => {
                let (x, y) = fx2(args)?;
                f.force = f.force + FVec::new(x, y);
                Value::Nil
            }
            "apply_force_relative" => {
                let (x, y) = fx2(args)?;
                f.force = f.force + FVec::new(x * Fixed::from_int(fac), y);
                Value::Nil
            }
            "move_directly" => {
                let (x, y) = fx2(args)?;
                f.pos = f.pos + FVec::new(x, y);
                Value::Nil
            }
            "move_directly_relative" => {
                let (x, y) = fx2(args)?;
                f.pos = f.pos + FVec::new(x * Fixed::from_int(fac), y);
                Value::Nil
            }
            "reset_momentum" => {
                f.vel = FVec::ZERO;
                f.force = FVec::ZERO;
                Value::Nil
            }
            "apply_forces" => {
                integrate(f, Some(&body));
                Value::Nil
            }
            "apply_forces_no_limit" => {
                integrate(f, None);
                Value::Nil
            }
            "apply_grav" => {
                apply_grav(f, &body);
                Value::Nil
            }
            "apply_fric" => {
                apply_fric(f, &body);
                Value::Nil
            }
            "apply_x_fric" => {
                f.vel.x = f.vel.x * (Fixed::ONE - fixed_of(&arg(args, 0))?);
                Value::Nil
            }
            "apply_y_fric" => {
                f.vel.y = f.vel.y * (Fixed::ONE - fixed_of(&arg(args, 0))?);
                Value::Nil
            }
            "apply_grav_custom" => {
                let (g, fall) = fx2(args)?;
                if f.vel.y < fall {
                    f.vel.y = (f.vel.y + g).min(fall);
                }
                Value::Nil
            }
            "limit_x_speed" => {
                let l = fixed_of(&arg(args, 0))?;
                f.vel.x = f.vel.x.clamp(-l, l);
                Value::Nil
            }
            "is_grounded" => Value::Bool(f.grounded),
            "set_grounded" => {
                f.grounded = arg(args, 0).truthy();
                Value::Nil
            }
            "update_grounded" => {
                f.grounded = f.pos.y >= Fixed::ZERO;
                Value::Nil
            }
            "start_invulnerability" | "end_invulnerability" => {
                f.script_invuln = name.starts_with("start");
                Value::Nil
            }
            "start_projectile_invulnerability" | "end_projectile_invulnerability" => {
                f.proj_invuln = name.starts_with("start");
                Value::Nil
            }
            "start_throw_invulnerability" | "end_throw_invulnerability" => {
                f.throw_invuln = name.starts_with("start");
                Value::Nil
            }
            "end_roll_projectile_invulnerability" | "end_aerial_attack_invulnerability" | "end_grounded_attack_invulnerability" => Value::Nil,
            "global_hitlag" => {
                let n = arg(args, 0).as_i64().unwrap_or(0).clamp(0, u16::MAX as i64) as u16;
                for k in 0..2 {
                    self.w.f[k].hitlag = self.w.f[k].hitlag.max(n);
                }
                Value::Nil
            }
            "change_state" => {
                let name = arg(args, 0).to_text();
                match name.as_str() {
                    // the engine runs hurt states itself
                    "HurtGrounded" | "HurtAerial" => self.script_hurt(i, name == "HurtAerial", &arg(args, 1)),
                    _ => self.w.f[i].queued = Some(name),
                }
                Value::Nil
            }
            "change_stance_to" => {
                let m = crate::state_id(&arg(args, 0).to_text());
                if let Some(p) = self.w.chars[i].modes().iter().position(|x| *x == m) {
                    self.w.f[i].mode = p;
                }
                Value::Nil
            }
            "release_opponent" => {
                self.w.release_hold(i);
                Value::Nil
            }
            "update_facing" => {
                let dx = self.w.f[o].pos.x - self.w.f[i].pos.x;
                if dx.signum() != 0 {
                    self.w.f[i].facing = dx.signum();
                }
                Value::Nil
            }
            "current_state" => self.current_state(i),
            "get_opponent" | "get_fighter" => Value::Obj(fighter_obj(if name == "get_opponent" { o } else { i })),
            "get_opponent_dir" => {
                let d = (self.w.f[o].pos.x - self.w.f[i].pos.x).signum();
                Value::Int(if d == 0 { fac } else { d })
            }
            "get_object_dir" => {
                let j = self.opp_of(&arg(args, 0)).ok_or_else(|| Error::runtime("get_object_dir: not a fighter"))?;
                let d = (self.w.f[j].pos.x - self.w.f[i].pos.x).signum();
                Value::Int(if d == 0 { fac } else { d })
            }
            "obj_local_center" => {
                let j = self.opp_of(&arg(args, 0)).ok_or_else(|| Error::runtime("obj_local_center: not a fighter"))?;
                let (a, b) = (self.hurt_center(i), self.hurt_center(j));
                xy(Value::Int((b.x - a.x).round()), Value::Int((b.y - a.y).round()))
            }
            "distance_to" => {
                let j = self.opp_of(&arg(args, 0)).ok_or_else(|| Error::runtime("distance_to: not a fighter"))?;
                text((self.w.f[j].pos - self.w.f[i].pos).len())
            }
            "is_in_group" => Value::Bool(arg(args, 0).to_text() == "Fighter"),
            "xy_to_dir" => {
                let (mul, div) = (args.get(2).cloned().unwrap_or(Value::str("1.0")), args.get(3).cloned().unwrap_or(Value::str("100.0")));
                let (x, y) = (Value::str(&arg(args, 0).to_text()), Value::str(&arg(args, 1).to_text()));
                let d = gdscript::fixed::call("vec_div", &[x, y, div])?;
                let part = |k: &str| match &d {
                    Value::Dict(m) => m.read().unwrap().iter().find(|(key, _)| key.to_text() == k).map(|(_, v)| v.clone()).unwrap_or(Value::Nil),
                    _ => Value::Nil,
                };
                gdscript::fixed::call("vec_mul", &[part("x"), part("y"), mul])?
            }
            "is_class_name" => Value::Bool(matches!(arg(args, 0).to_text().as_str(), "Fighter" | "BaseObj")),
            _ => return Err(Error::Unsupported(format!("host.{name}()"))),
        })
    }

    fn state_get(&mut self, i: usize, idx: usize, name: &str) -> Result<Value, Error> {
        let current = matches!(&self.w.f[i].act, Act::Move(run) if run.idx == idx);
        let run = match &self.w.f[i].act {
            Act::Move(run) if current => Some(run.clone()),
            _ => None,
        };
        Ok(match (name, run) {
            ("host", _) => Value::Obj(fighter_obj(i)),
            ("fixed", _) => Value::Lib("fixed"),
            ("active", _) => Value::Bool(current),
            ("current_tick" | "current_real_tick", Some(r)) => Value::Int(r.frame as i64 - 1),
            ("hit_yet" | "hit_anything" | "hit_fighter", Some(r)) => Value::Bool(r.hit),
            ("was_blocked", Some(r)) => Value::Bool(r.blocked),
            ("name", _) => {
                let node = self.t.states[i].get(idx).cloned().flatten().and_then(|s| s.read().unwrap().get("state_name"));
                node.unwrap_or_else(|| Value::str(&self.w.chars[i].mv(idx).id))
            }
            _ => return Err(Error::Unsupported(name.to_string())),
        })
    }

    fn state_call(&mut self, i: usize, idx: usize, name: &str, args: &[Value]) -> Result<Value, Error> {
        if ENGINE_DONE.contains(&name) {
            return Ok(Value::Nil);
        }
        Ok(match name {
            "queue_state_change" => {
                self.w.f[i].queued = Some(arg(args, 0).to_text());
                Value::Nil
            }
            "is_usable" | "allowed_in_stance" | "is_usable_with_grounded_check" => Value::Bool(true),
            "get_name" | "get_animation" => self.state_get(i, idx, "name")?,
            _ => return Err(Error::Unsupported(name.to_string())),
        })
    }
}

/// `YEMI_HOST_TRACE=1`: print every host property read and call (debugging mod scripts).
fn trace(kind: &str, obj: ObjId, name: &str, r: &Result<Value, Error>) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("YEMI_HOST_TRACE").is_some()) {
        let out = match r {
            Ok(v) => v.type_name().to_string(),
            Err(e) => format!("ERR {e}"),
        };
        eprintln!("host {kind} {} .{name} -> {out}", obj.0);
    }
}

impl Host for Ctx<'_> {
    fn get(&mut self, obj: ObjId, name: &str) -> Result<Value, Error> {
        let r = self.get_inner(obj, name);
        trace("get", obj, name, &r);
        r
    }

    fn set(&mut self, obj: ObjId, name: &str, v: Value) -> Result<(), Error> {
        self.set_inner(obj, name, v)
    }

    fn call(&mut self, obj: ObjId, method: &str, args: &[Value]) -> Result<Value, Error> {
        let r = self.call_inner(obj, method, args);
        trace("call", obj, method, &r);
        r
    }

    fn script(&mut self, obj: ObjId) -> Option<InstRef> {
        self.script_inner(obj)
    }

    fn global(&mut self, name: &str) -> Option<Value> {
        self.global_inner(name)
    }

    fn node(&mut self, from: Option<ObjId>, path: &str) -> Result<Value, Error> {
        self.node_inner(from, path)
    }

    fn global_call(&mut self, name: &str, args: &[Value]) -> Result<Value, Error> {
        self.global_call_inner(name, args)
    }
}

impl Ctx<'_> {
    fn get_inner(&mut self, obj: ObjId, name: &str) -> Result<Value, Error> {
        match target(obj) {
            // `Global.current_game`: the match, whose clock scripts read (and nudge) every tick
            Target::Sink if name == "current_game" => Ok(Value::Obj(GAME)),
            Target::Game => Ok(match name {
                "current_tick" => Value::Int(self.w.frame as i64),
                "time" => Value::Int(self.w.frame as i64 + 3600),
                "p1" => Value::Obj(fighter_obj(0)),
                "p2" => Value::Obj(fighter_obj(1)),
                "objs_map" => self.objs_map(),
                "game_paused" | "game_finished" | "is_ghost" => Value::Bool(false),
                _ => Value::Nil,
            }),
            // `Main.match_data`: who is fighting, by the name of each fighter's scene (as the
            // original's mod loader names them, "Strongest" for `Strongest.tscn`)
            Target::Sink if name == "match_data" => {
                let who = |i: usize| {
                    let scene = self.w.chars[i].file.character.scene.clone().unwrap_or_else(|| self.w.chars[i].file.character.id.clone());
                    let stem = scene.rsplit('/').next().unwrap_or("").trim_end_matches(".tscn").to_string();
                    Value::dict(vec![(Value::str("name"), Value::str(&stem))])
                };
                let chars = Value::dict(vec![(Value::Int(1), who(0)), (Value::Int(2), who(1))]);
                Ok(Value::dict(vec![(Value::str("selected_characters"), chars)]))
            }
            // a stand-in node's own properties, so `obj.scale.x = 1` and the like work through
            Target::Sink => Ok(match name {
                "position" | "global_position" | "offset" => Value::Vec2(0.0, 0.0),
                "scale" => Value::Vec2(1.0, 1.0),
                "modulate" | "self_modulate" => Value::Color(1.0, 1.0, 1.0, 1.0),
                "rotation" | "rotation_degrees" => Value::Float(0.0),
                "visible" => Value::Bool(true),
                "z_index" | "frame" => Value::Int(0),
                "name" => Value::str(""),
                // its parts are stand-ins too (`effect.sprite.flip_h = ...`)
                _ => Value::Obj(SINK),
            }),
            Target::UiLayer => Ok(Value::Nil),
            Target::Hurtbox(i) => {
                let (c, h) = self.w.hurtboxes(i)[0];
                let f = &self.w.f[i];
                Ok(match name {
                    "x" => Value::Int(((c.x - f.pos.x) * Fixed::from_int(f.facing)).round()),
                    "y" => Value::Int((c.y - f.pos.y).round()),
                    "width" => Value::Int(h.x.round()),
                    "height" => Value::Int(h.y.round()),
                    "facing" => Value::str(if f.facing < 0 { "Left" } else { "Right" }),
                    _ => Value::Nil,
                })
            }
            Target::Obj(u) => self.obj_get(u, name),
            Target::ObjState(u, k) => self.obj_state_get(u, k, name),
            Target::Node(n) => self.node_get(n, name),
            Target::Machine(i) => match name {
                "state" => Ok(self.current_state(i)),
                "host" => Ok(Value::Obj(fighter_obj(i))),
                // the states it went through, the current one last (`_previous_state()`)
                "states_stack" => {
                    let mut h: Vec<Value> = self.t.history[i].iter().map(|o| Value::Obj(*o)).collect();
                    if h.is_empty() {
                        h.push(self.current_state(i));
                    }
                    Ok(Value::array(h))
                }
                "states_map" => Ok(Value::dict(vec![])),
                _ => Err(self.unsupported(format!("state_machine.{name}"))),
            },
            Target::Fighter(i) => self.fighter_get(i, name),
            Target::State(i, idx) => self.state_get(i, idx, name),
            Target::EngineState(i) => match (name, &self.w.f[i].act) {
                ("name" | "state_name", a) => Ok(Value::str(engine_state_name(a))),
                ("host", _) => Ok(Value::Obj(fighter_obj(i))),
                // the hurt states' stun, which scripts read and stretch (time stops)
                ("hitstun", Act::Hurt { stun, .. }) => Ok(Value::Int(*stun as i64)),
                ("current_tick", Act::Hurt { frame, .. }) => Ok(Value::Int(*frame as i64)),
                ("current_tick", _) => Ok(Value::Int(0)),
                // a state the engine runs has none of a script's variables
                _ => Ok(Value::Nil),
            },
            Target::Hitbox(i, k) => {
                let Some((hb, _)) = self.hitbox_of(i, k) else { return Ok(Value::Nil) };
                let v = crate::objects::hitbox_value(&hb, &format!("P{}", i + 1));
                Ok(crate::objects::key(&v, name))
            }
            Target::Other => Err(self.unsupported(format!("object .{name}"))),
        }
    }

    fn set_inner(&mut self, obj: ObjId, name: &str, v: Value) -> Result<(), Error> {
        match target(obj) {
            // the match's clock and camera are the app's
            Target::Sink | Target::UiLayer | Target::Game => Ok(()),
            Target::Obj(u) => self.obj_set(u, name, v),
            Target::ObjState(..) => Err(Error::Unsupported(name.to_string())),
            Target::EngineState(i) => {
                if let (true, Act::Hurt { stun, .. }) = (name == "hitstun", &mut self.w.f[i].act) {
                    *stun = v.as_i64().unwrap_or(0).clamp(0, 600) as u16;
                }
                // the rest (`iasa_at`, `interruptible_on_opponent_turn`...) the engine decides
                Ok(())
            }
            // a hitbox's tuning (`sdi_modifier`...) is the move's data
            Target::Hitbox(..) => Ok(()),
            Target::Node(n) => self.node_set(n, name, v),
            Target::Fighter(i) => self.fighter_set(i, name, v),
            Target::State(..) => Err(Error::Unsupported(name.to_string())),
            _ => Err(self.unsupported(format!("set .{name}"))),
        }
    }

 fn call_inner(&mut self, obj: ObjId, method: &str, args: &[Value]) -> Result<Value, Error> {
        match method {
            "get_node" | "find_node" | "get_node_or_null" | "has_node" => {
                let path = arg(args, 0).to_text();
                let found = self.find_node(Some(obj), &path);
                return Ok(match (method, found) {
                    ("has_node", f) => Value::Bool(f.is_some()),
                    (_, Some(v)) => v,
                    ("get_node_or_null", None) => Value::Nil,
                    (_, None) => {
                        World::note_problem(self.t, format!("node {path} (stand-in)"));
                        Value::Obj(SINK)
                    }
                });
            }
            // a fighter's state sits under its state machine
            "get_parent" if matches!(target(obj), Target::State(..) | Target::EngineState(_)) => {
                let i = match target(obj) {
                    Target::State(i, _) | Target::EngineState(i) => i,
                    _ => 0,
                };
                return Ok(Value::Obj(ObjId(MACHINE_BASE + i as u32)));
            }
            "get_parent" | "find_parent" | "get_tree" | "get_root" | "get_viewport" | "get_camera" => return Ok(Value::Obj(SINK)),
            "is_class_name" | "is_class" => {
                let c = arg(args, 0).to_text();
                return Ok(Value::Bool(self.is_class(obj, &c)));
            }
            // signals between nodes: what they would trigger, the engine does itself
            "emit_signal" | "disconnect" | "is_connected" | "connect" if !matches!(target(obj), Target::Node(_)) => return Ok(Value::Nil),
            // a node taken off its parent (an effect a script set up by hand)
            "remove_child" => return Ok(Value::Nil),
            // adding a stand-in (an effect scene instanced by hand) to anything
            "add_child" if matches!(args.first(), Some(Value::Obj(o)) if *o == SINK) => return Ok(Value::Nil),
            // `preload("X.tscn").instance()` of a scene that is not a node of the scripts' own
            // scenes (HUD layers, particle scenes added by hand)
            "instance_scene" => {
                World::note_problem(self.t, format!("scene {} instanced (stand-in)", arg(args, 0).to_text()));
                return Ok(Value::Obj(SINK));
            }
            "has_method" => {
                let name = arg(args, 0).to_text();
                let has = self.script_inner(obj).is_some_and(|i| i.read().unwrap().chain.iter().any(|s| s.funcs.contains_key(&name)));
                return Ok(Value::Bool(has));
            }
            _ => {}
        }
        match target(obj) {
            // offline there are no player names (`Network.pid_to_username`)
            Target::Sink if method == "pid_to_username" => Ok(Value::str("")),
            // a stand-in has no children to loop over
            Target::Sink if method == "get_children" => Ok(Value::array(vec![])),
            Target::Game => Ok(match method {
                "get_player" => Value::Obj(fighter_obj((arg(args, 0).as_i64().unwrap_or(1) - 1).clamp(0, 1) as usize)),
                "obj_from_name" => self.obj_from_name(&arg(args, 0).to_text()),
                // camera, slow motion, HUD: the app's
                _ => Value::Nil,
            }),
            // a group (the camera...) is one stand-in
            Target::Sink if method == "get_nodes_in_group" => Ok(Value::array(vec![Value::Obj(SINK)])),
            Target::Sink if method == "exists" => Ok(Value::Bool(false)),
            // tweens chain their calls (`create_tween().set_parallel().tween_property(...)`)
            Target::Sink if method.starts_with("tween_") || matches!(method, "new" | "create_tween" | "set_parallel" | "set_trans" | "set_ease" | "set_loops" | "chain" | "parallel") => Ok(Value::Obj(SINK)),
            Target::Sink => Ok(Value::Nil),
            Target::Hurtbox(i) => {
                let (c, _) = self.w.hurtboxes(i)[0];
                Ok(match method {
                    "get_center_float" => Value::Vec2(c.x.to_f32() as f64, c.y.to_f32() as f64),
                    "get_center" => xy(Value::Int(c.x.round()), Value::Int(c.y.round())),
                    _ => Value::Nil,
                })
            }
            Target::UiLayer => Ok(match method {
                "add_child" => self.ui_add(&arg(args, 0)),
                _ => Value::Nil,
            }),
            Target::Obj(u) => self.obj_call(u, method, args),
            Target::ObjState(u, k) => self.obj_state_call(u, k, method, args),
            Target::Node(n) => self.node_call(n, method, args),
            Target::Hitbox(i, k) => {
                let Some((hb, cycle)) = self.hitbox_of(i, k) else { return Ok(Value::Nil) };
                Ok(match method {
                    // it hits nothing more this time round
                    "deactivate" => {
                        if let Act::Move(run) = &mut self.w.f[i].act {
                            let key = crate::hit_key(&hb, cycle);
                            if !run.connected.contains(&key) {
                                run.connected.push(key);
                            }
                        }
                        Value::Nil
                    }
                    "overlaps" => {
                        let Act::Move(_) = &self.w.f[i].act else { return Ok(Value::Bool(false)) };
                        let mine = crate::world_rect(&self.w.f[i], &hb.rect);
                        let other = match arg(args, 0) {
                            Value::Obj(o) => match target(o) {
                                Target::Hurtbox(j) => self.w.hurtboxes(j),
                                Target::Fighter(j) => self.w.hurtboxes(j),
                                _ => vec![],
                            },
                            _ => vec![],
                        };
                        Value::Bool(other.iter().any(|r| crate::overlap(&mine, r)))
                    }
                    "to_data" => crate::objects::hitbox_value(&hb, &format!("P{}", i + 1)),
                    "get" => crate::objects::key(&crate::objects::hitbox_value(&hb, &format!("P{}", i + 1)), &arg(args, 0).to_text()),
                    _ => Value::Nil,
                })
            }
            Target::Machine(i) => match method {
                "queue_state" | "_change_state" => {
                    self.w.f[i].queued = Some(arg(args, 0).to_text());
                    Ok(Value::Nil)
                }
                _ => Err(self.unsupported(format!("state_machine.{method}()"))),
            },
            Target::Fighter(i) => self.fighter_call(i, method, args),
            Target::State(i, idx) => self.state_call(i, idx, method, args),
            Target::EngineState(i) => match method {
                "get_name" => Ok(Value::str(engine_state_name(&self.w.f[i].act))),
                "get" => Ok(Value::Nil),
                "queue_state_change" | "enable_interrupt" => Ok(Value::Nil),
                _ => Err(self.unsupported(format!("engine state .{method}()"))),
            },
            Target::Other => Err(self.unsupported(format!(".{method}()"))),
        }
    }

    fn script_inner(&mut self, obj: ObjId) -> Option<InstRef> {
        match target(obj) {
            Target::Fighter(i) => self.t.roots[i].clone(),
            Target::State(i, idx) => self.t.states[i].get(idx).cloned().flatten(),
            Target::Obj(u) => self.t.objs.get(&u).and_then(|o| o.root.clone()),
            Target::ObjState(u, k) => self.t.objs.get(&u).and_then(|o| o.states.get(k).cloned().flatten()),
            _ => None,
        }
    }

    fn global_inner(&mut self, name: &str) -> Option<Value> {
        match name {
            "ReplayManager" => Some(Value::dict(vec![(Value::str("resimulating"), Value::Bool(false)), (Value::str("playback"), Value::Bool(false))])),
            // the game's other singletons (offline, so `Network.multiplayer_active` is false):
            // read as nothing, calls do nothing
            "Network" | "Global" | "SteamYomi" | "SteamLobby" | "Custom" | "GameHooks" => Some(Value::Obj(SINK)),
            // Godot's servers and singletons: nothing a fight needs (`ResourceLoader.exists` is
            // false, so mods take their desktop paths)
            "ResourceLoader" | "AudioServer" | "OS" | "Engine" | "Input" | "Time" | "ProjectSettings" | "Performance" | "VisualServer"
            | "Physics2DServer" | "InputMap" | "TranslationServer" | "IP" | "JavaScript" | "Tween" => Some(Value::Obj(SINK)),
            _ => None,
        }
    }

    fn node_inner(&mut self, from: Option<ObjId>, path: &str) -> Result<Value, Error> {
        match self.find_node(from, path) {
            Some(v) => Ok(v),
            None => {
                World::note_problem(self.t, format!("node ${path} (stand-in)"));
                Ok(Value::Obj(SINK))
            }
        }
    }

    fn global_call_inner(&mut self, name: &str, args: &[Value]) -> Result<Value, Error> {
        // Godot's random numbers, from the match's seeded generator so a match replays the same
        let mut unit = |s: &mut Self| s.randi_range(0, 1 << 30).as_i64().unwrap_or(0) as f64 / (1u64 << 30) as f64;
        Ok(match name {
            "rand_range" => {
                let (lo, hi) = (arg(args, 0).as_f64().unwrap_or(0.0), arg(args, 1).as_f64().unwrap_or(0.0));
                Value::Float(lo + (hi - lo) * unit(self))
            }
            "randf" => Value::Float(unit(self)),
            "randi" => self.randi_range(0, i32::MAX as i64),
            "randomize" | "seed" => Value::Nil,
            _ => return Err(self.unsupported(format!("{name}()"))),
        })
    }
}
