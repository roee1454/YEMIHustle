//! Scripted objects and scene nodes: what mods' scripts spawn (projectiles, summons, domains),
//! and the nodes of a scene they reach (`$Sprite`, `get_node("/root/Main/UILayer/...")`).
//!
//! An object is a converted scene ([`charfmt::Projectile`] with `states`): the engine runs it the
//! way the original's `BaseObj` / `BaseProjectile` do (a state machine whose states are
//! `ObjectState` scripts with hitboxes; hitlag; physics flags; exported projectiles), and its own
//! scripts run in the interpreter on top. Nodes keep the values scripts give them (`visible`,
//! `frame`, `modulate`...), play their sprite animations and send `animation_finished`. Nodes a
//! script adds to the UI layer (cut-ins) and the objects' sprites are drawn by the app from
//! [`World::ui_nodes`] and [`World::object_nodes`].

use std::collections::HashMap;

use charfmt::{NodeDef, SceneDef, StateScript};
use gdscript::{Error, InstRef, ObjId, Value, Vm};

use crate::script::{arg, fighter_obj, fixed_of, text, var_value, xy, Ctx};
use crate::{Event, FVec, Fixed, Proj, World};

pub(crate) const OBJ_BASE: u32 = 2_000_000;
pub(crate) const OBJ_STATE_BASE: u32 = 3_000_000;
/// States per object in the id space of object states.
const STATES_PER_OBJ: u32 = 64;
pub(crate) const NODE_BASE: u32 = 20_000_000;
/// The game's UI layer (`/root/Main/UILayer`), where cut-ins go.
pub(crate) const UI_LAYER: ObjId = ObjId(70);
/// Objects end after this many ticks however their scripts run (a guard, 30 s).
const MAX_OBJECT_AGE: u16 = 1800;

pub(crate) fn obj_id(uid: u32) -> ObjId {
    ObjId(OBJ_BASE + uid)
}

pub(crate) fn obj_state_id(uid: u32, k: usize) -> ObjId {
    ObjId(OBJ_STATE_BASE + uid * STATES_PER_OBJ + k as u32)
}

/// (uid) for an object id, (uid, state) for an object state id, node index for a node id.
pub(crate) fn decode(o: ObjId) -> Option<ObjRef> {
    match o.0 {
        x if x >= NODE_BASE => Some(ObjRef::Node((x - NODE_BASE) as usize)),
        x if x >= OBJ_STATE_BASE => {
            let r = x - OBJ_STATE_BASE;
            Some(ObjRef::State(r / STATES_PER_OBJ, (r % STATES_PER_OBJ) as usize))
        }
        x if x >= OBJ_BASE => Some(ObjRef::Obj(x - OBJ_BASE)),
        _ => None,
    }
}

pub(crate) enum ObjRef {
    Obj(u32),
    State(u32, usize),
    Node(usize),
}

/// An object's script objects.
#[derive(Default)]
pub(crate) struct ObjInsts {
    pub root: Option<InstRef>,
    pub states: Vec<Option<InstRef>>,
    /// The state whose `_enter` ran.
    pub entered: Option<usize>,
    /// Values scripts gave the object that the engine has no field for (`hit_cancel_on_hit`...).
    pub props: HashMap<String, Value>,
    /// Hitboxes that already struck it: by whom, and which (group, repeat cycle).
    pub struck: Vec<(Striker, (String, u16))>,
}

/// What struck an object: a fighter's move, an object's state, or a plain projectile.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Striker {
    Fighter(usize, usize),
    Obj(u32, usize),
    Plain(usize),
}

/// What the original's `BaseProjectile` exports default to (an object scene's root values win).
fn obj_default(name: &str) -> Option<Value> {
    Some(match name {
        "immunity_susceptible" | "roll_immunity_susceptible" | "deletes_other_projectiles" | "movable" | "apply_hitlag_when_hit_by_melee" | "hookable" => Value::Bool(true),
        "fizzle_on_ceiling" | "can_be_hit_by_melee" | "hit_by_self_projectiles" | "hit_cancel_on_hit" | "free_cancel_on_hit" | "projectile_immune" | "got_parried"
        | "stopped" | "damages_own_team" | "invulnerable" | "projectile_invulnerable" | "throw_invulnerable" => Value::Bool(false),
        "hitlag_modifier" => Value::str("1.0"),
        "disabled_linger" => Value::Int(0),
        _ => return None,
    })
}

/// `hitbox.<k>` of a hitbox handed to a script (a dictionary).
pub(crate) fn key(v: &Value, k: &str) -> Value {
    match v {
        Value::Dict(d) => d.read().unwrap().iter().find(|(n, _)| n.to_text() == k).map(|(_, v)| v.clone()).unwrap_or(Value::Nil),
        _ => Value::Nil,
    }
}

/// A hitbox as scripts see it in `hit_by(hitbox)`.
pub(crate) fn hitbox_value(hb: &charfmt::HitBox, host: &str) -> Value {
    Value::dict(vec![
        (Value::str("name"), Value::str(&hb.id)),
        (Value::str("host"), Value::str(host)),
        (Value::str("damage"), Value::Int(hb.damage as i64)),
        (Value::str("damage_in_combo"), Value::Int(hb.damage_in_combo.unwrap_or(hb.damage) as i64)),
        (Value::str("hitlag_ticks"), Value::Int(hb.hitlag.attacker as i64)),
        (Value::str("victim_hitlag"), Value::Int(hb.hitlag.victim as i64)),
        (Value::str("throw"), Value::Bool(hb.effects.throw)),
        (Value::str("hitstun_ticks"), Value::Int(hb.hitstun as i64)),
        (Value::str("knockdown"), Value::Bool(hb.effects.knockdown != charfmt::Knockdown::None)),
        (Value::str("hard_knockdown"), Value::Bool(hb.effects.knockdown == charfmt::Knockdown::Hard)),
        (Value::str("hitbox_type"), Value::Int(if hb.kind == charfmt::HitKind::Detect { 5 } else { 0 })),
        (Value::str("misc_data"), Value::str(hb.misc_data.as_deref().unwrap_or(""))),
        (Value::str("rumble"), Value::Bool(false)),
        (Value::str("active"), Value::Bool(true)),
    ])
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum NodeOwner {
    Fighter(usize),
    Obj(u32),
}

/// A node scripts reached, and what they did to it.
pub(crate) struct NodeRec {
    pub owner: NodeOwner,
    /// Path in the owner's scene.
    pub path: String,
    pub props: HashMap<String, Value>,
    /// Animation time carried between ticks (in frames).
    acc: f32,
    /// `connect(signal, target, method)`.
    signals: Vec<(String, InstRef, String)>,
    /// Added to the UI layer under this name.
    pub ui: Option<String>,
    pub freed: bool,
}

/// What the app draws for a node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    /// The player the node belongs to (for colors).
    pub player: usize,
    /// The scene the node comes from and its path there (for its frames).
    pub scene: String,
    pub path: String,
    pub ty: String,
    pub anim: String,
    pub frame: u32,
    /// UI nodes: canvas position. Object nodes: offset from the object, in game pixels.
    pub pos: (f32, f32),
    /// The node hangs under a `Flip` node (mirrored with the owner's facing).
    pub flipped: bool,
    pub flip_h: bool,
    pub modulate: [f32; 4],
    pub scale: (f32, f32),
    pub z: i32,
    /// Clockwise, in radians (Godot's `rotation`).
    pub rotation: f32,
    /// A static sprite's texture.
    pub texture: Option<String>,
    pub centered: bool,
    pub offset: (f32, f32),
}

fn node_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `base` + `rel` as a scene path ("." is the root).
fn join(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if base == "." || base.is_empty() { vec![] } else { base.split('/').collect() };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() { ".".into() } else { parts.join("/") }
}

/// A node's `rotation` (radians) or `rotation_degrees`.
fn rotation_of(get: &dyn Fn(&str) -> Option<Value>) -> f32 {
    match (get("rotation").and_then(|v| v.as_f64()), get("rotation_degrees").and_then(|v| v.as_f64())) {
        (Some(r), _) => r as f32,
        (None, Some(d)) => (d as f32).to_radians(),
        _ => 0.0,
    }
}

/// How scripts left a fighter's own sprite (`host.sprite.frame = 3`, `$"%Sprite".visible`...),
/// as changes to what the engine would show.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteLook {
    /// False when the sprite or one of its parents (`Flip`) is hidden.
    pub visible: bool,
    /// The animation and frame a script picked this tick.
    pub anim: Option<String>,
    pub frame: Option<u32>,
    pub modulate: [f32; 4],
    /// Clockwise, in radians.
    pub rotation: f32,
    /// Moved from the scene's position and offset (game pixels).
    pub shift: (f32, f32),
    pub scale: (f32, f32),
    pub flip_h: bool,
}

/// The main sprite of a scene: `Flip/Sprite`, else `Sprite`.
fn main_sprite(scene: &SceneDef) -> Option<&'static str> {
    ["Flip/Sprite", "Sprite"].into_iter().find(|n| scene.nodes.iter().any(|d| d.path == *n))
}

fn color_of(v: &Value) -> [f32; 4] {
    match v {
        Value::Color(r, g, b, a) => [*r as f32, *g as f32, *b as f32, *a as f32],
        _ => [1.0; 4],
    }
}

impl Ctx<'_> {
    // ------------------------------------------------------------------------------------
    // Scenes and nodes
    // ------------------------------------------------------------------------------------

    pub(crate) fn proj(&self, uid: u32) -> Option<&Proj> {
        self.w.projs.iter().find(|p| p.uid == uid)
    }

    pub(crate) fn proj_mut(&mut self, uid: u32) -> Option<&mut Proj> {
        self.w.projs.iter_mut().find(|p| p.uid == uid)
    }

    fn scene_path(&self, owner: NodeOwner) -> Option<String> {
        match owner {
            NodeOwner::Fighter(i) => self.w.chars[i].file.character.scene.clone(),
            NodeOwner::Obj(uid) => {
                let p = self.proj(uid)?;
                self.w.chars[p.owner].file.character.projectiles.get(p.idx)?.scene.clone()
            }
        }
    }

    fn owner_player(&self, owner: NodeOwner) -> usize {
        match owner {
            NodeOwner::Fighter(i) => i,
            NodeOwner::Obj(uid) => self.proj(uid).map(|p| p.owner).unwrap_or(0),
        }
    }

    fn scene(&self, owner: NodeOwner) -> Option<&SceneDef> {
        let path = self.scene_path(owner)?;
        let i = self.owner_player(owner);
        self.w.chars[i].file.character.scenes.iter().find(|s| s.path == path)
    }

    fn node_def(&self, owner: NodeOwner, path: &str) -> Option<&NodeDef> {
        self.scene(owner)?.nodes.iter().find(|n| n.path == path)
    }

    /// The node record for a scene path (made on first use).
    fn node_rec(&mut self, owner: NodeOwner, path: &str) -> ObjId {
        if let Some(i) = self.t.nodes.iter().position(|n| n.owner == owner && n.path == path && n.ui.is_none() && !n.freed) {
            return ObjId(NODE_BASE + i as u32);
        }
        self.t.nodes.push(NodeRec { owner, path: path.to_string(), props: HashMap::new(), acc: 0.0, signals: vec![], ui: None, freed: false });
        ObjId(NODE_BASE + (self.t.nodes.len() - 1) as u32)
    }

    /// Where a script's own node sits: its owner and its path in the owner's scene.
    fn node_base(&self, from: ObjId) -> Option<(NodeOwner, String)> {
        match crate::script::target_of(from) {
            crate::script::Target::Fighter(i) => Some((NodeOwner::Fighter(i), ".".into())),
            crate::script::Target::State(i, idx) => {
                let node = self.w.chars[i].mv(idx).script.as_ref().map(|s| s.node.clone()).unwrap_or_default();
                Some((NodeOwner::Fighter(i), format!("StateMachine/{node}")))
            }
            crate::script::Target::Machine(i) => Some((NodeOwner::Fighter(i), "StateMachine".into())),
            _ => match decode(from)? {
                ObjRef::Obj(uid) => Some((NodeOwner::Obj(uid), ".".into())),
                ObjRef::State(uid, k) => {
                    let p = self.proj(uid)?;
                    let node = self.w.chars[p.owner].file.character.projectiles.get(p.idx)?.states.get(k)?.node.clone();
                    Some((NodeOwner::Obj(uid), format!("StateMachine/{node}")))
                }
                ObjRef::Node(i) => self.t.nodes.get(i).map(|n| (n.owner, n.path.clone())),
            },
        }
    }

    /// `get_node(path)` from `from`: a node, the UI layer, or None.
    pub(crate) fn find_node(&mut self, from: Option<ObjId>, path: &str) -> Option<Value> {
        let path = path.trim_matches('"');
        if let Some(rest) = path.strip_prefix("/root/Main/UILayer") {
            let name = rest.trim_start_matches('/');
            if name.is_empty() {
                return Some(Value::Obj(UI_LAYER));
            }
            let i = self.t.nodes.iter().position(|n| n.ui.as_deref() == Some(name) && !n.freed)?;
            return Some(Value::Obj(ObjId(NODE_BASE + i as u32)));
        }
        if path.starts_with('/') {
            return None;
        }
        let (owner, base) = self.node_base(from?)?;
        let full = if let Some(unique) = path.strip_prefix('%') {
            // a unique name: the node of that name anywhere in the scene
            let (head, tail) = unique.split_once('/').unwrap_or((unique, ""));
            let found = self.scene(owner)?.nodes.iter().find(|n| node_name(&n.path) == head)?.path.clone();
            if tail.is_empty() { found } else { join(&found, tail) }
        } else {
            join(&base, path)
        };
        if full != "." && self.node_def(owner, &full).is_none() {
            return None;
        }
        if full == "." {
            return Some(match owner {
                NodeOwner::Fighter(i) => Value::Obj(fighter_obj(i)),
                NodeOwner::Obj(uid) => Value::Obj(obj_id(uid)),
            });
        }
        Some(Value::Obj(self.node_rec(owner, &full)))
    }

    /// A node property: what a script set, else the scene's value, else Godot's default.
    fn node_prop(&self, i: usize, name: &str) -> Value {
        let n = &self.t.nodes[i];
        if let Some(v) = n.props.get(name) {
            return v.clone();
        }
        if let Some(v) = self.node_def(n.owner, &n.path).and_then(|d| d.props.iter().find(|(k, _)| k == name)).map(|(_, v)| var_value(v)) {
            return v;
        }
        match name {
            "visible" => Value::Bool(true),
            "playing" | "flip_h" | "flip_v" => Value::Bool(false),
            "frame" | "z_index" => Value::Int(0),
            "animation" => Value::str("default"),
            "position" | "offset" | "global_position" => Value::Vec2(0.0, 0.0),
            "scale" => Value::Vec2(1.0, 1.0),
            "modulate" | "self_modulate" => Value::Color(1.0, 1.0, 1.0, 1.0),
            "speed_scale" => Value::Float(1.0),
            "name" => Value::str(node_name(&n.path)),
            "text" => Value::str(""),
            // resources scripts pass around (materials, sprite frames): stand-ins
            "material" | "frames" | "texture" => Value::Obj(crate::script::SINK),
            _ => Value::Nil,
        }
    }

    /// A node's Godot type (`AnimatedSprite`, `Particles2D`...), as its scene says.
    pub(crate) fn node_type(&self, i: usize) -> Option<String> {
        let n = self.t.nodes.get(i)?;
        self.node_def(n.owner, &n.path).map(|d| d.ty.clone())
    }

    pub(crate) fn node_get(&mut self, i: usize, name: &str) -> Result<Value, Error> {
        if self.t.nodes.get(i).is_none() {
            return Ok(Value::Nil);
        }
        Ok(self.node_prop(i, name))
    }

    pub(crate) fn node_set(&mut self, i: usize, name: &str, v: Value) -> Result<(), Error> {
        if let Some(n) = self.t.nodes.get_mut(i) {
            if name == "animation" && n.props.get("animation").is_none_or(|a| a.to_text() != v.to_text()) {
                n.props.insert("frame".into(), Value::Int(0));
            }
            n.props.insert(name.to_string(), v);
        }
        Ok(())
    }

    pub(crate) fn node_call(&mut self, i: usize, method: &str, args: &[Value]) -> Result<Value, Error> {
        if self.t.nodes.get(i).is_none() {
            return Ok(Value::Nil);
        }
        let set = |s: &mut Self, k: &str, v: Value| {
            s.t.nodes[i].props.insert(k.to_string(), v);
        };
        Ok(match method {
            "play" => {
                if let Some(a) = args.first().filter(|a| !matches!(a, Value::Nil)) {
                    let a = a.to_text();
                    if !a.is_empty() {
                        set(self, "animation", Value::str(&a));
                        set(self, "frame", Value::Int(0));
                    }
                }
                set(self, "playing", Value::Bool(true));
                Value::Nil
            }
            "stop" => {
                set(self, "playing", Value::Bool(false));
                Value::Nil
            }
            "show" | "hide" => {
                set(self, "visible", Value::Bool(method == "show"));
                Value::Nil
            }
            "is_visible" | "is_visible_in_tree" => self.node_prop(i, "visible"),
            "is_playing" => self.node_prop(i, "playing"),
            "queue_free" | "free" => {
                self.t.nodes[i].freed = true;
                Value::Nil
            }
            "duplicate" => {
                let n = &self.t.nodes[i];
                let copy = NodeRec { owner: n.owner, path: n.path.clone(), props: n.props.clone(), acc: 0.0, signals: vec![], ui: None, freed: false };
                self.t.nodes.push(copy);
                Value::Obj(ObjId(NODE_BASE + (self.t.nodes.len() - 1) as u32))
            }
            "connect" => {
                if let (Some(sig), Some(Value::Inst(target)), Some(m)) = (args.first(), args.get(1), args.get(2)) {
                    self.t.nodes[i].signals.push((sig.to_text(), target.clone(), m.to_text()));
                }
                Value::Nil
            }
            "get" => self.node_prop(i, &arg(args, 0).to_text()),
            "set" => {
                set(self, &arg(args, 0).to_text(), arg(args, 1));
                Value::Nil
            }
            "get_name" => Value::str(node_name(&self.t.nodes[i].path)),
            // its children in the scene (an instanced sub-scene's own are not known)
            "get_children" | "get_child_count" => {
                let (owner, path) = (self.t.nodes[i].owner, self.t.nodes[i].path.clone());
                let prefix = if path == "." { String::new() } else { format!("{path}/") };
                let kids: Vec<String> = self.scene(owner).map(|s| s.nodes.iter().filter(|n| n.path.starts_with(&prefix) && !n.path[prefix.len()..].contains('/') && n.path != path).map(|n| n.path.clone()).collect()).unwrap_or_default();
                if method == "get_child_count" {
                    Value::Int(kids.len() as i64)
                } else {
                    Value::array(kids.iter().map(|k| Value::Obj(self.node_rec(owner, k))).collect())
                }
            }
            // a hitbox node's values, as the original hands them to a hurt state
            "to_data" => {
                let n = &self.t.nodes[i];
                let mut kv: Vec<(Value, Value)> = self.node_def(n.owner, &n.path).map(|d| d.props.iter().map(|(k, v)| (Value::str(k), var_value(v))).collect()).unwrap_or_default();
                for (k, v) in &self.t.nodes[i].props {
                    kv.retain(|(x, _)| x.to_text() != *k);
                    kv.push((Value::str(k), v.clone()));
                }
                Value::dict(kv)
            }
            "get_material" | "get_sprite_frames" => Value::Obj(crate::script::SINK),
            _ => Value::Nil,
        })
    }

    /// `UILayer.add_child(node)`.
    pub(crate) fn ui_add(&mut self, node: &Value) -> Value {
        if let Value::Obj(o) = node {
            if let Some(ObjRef::Node(i)) = decode(*o) {
                if let Some(n) = self.t.nodes.get_mut(i) {
                    n.ui = Some(node_name(&n.path).to_string());
                }
            }
        }
        Value::Nil
    }

    // ------------------------------------------------------------------------------------
    // Sounds and effects
    // ------------------------------------------------------------------------------------

    /// `play_sound(name)`: the owner scene's `Sounds/<name>` node.
    pub(crate) fn play_sound(&mut self, owner: NodeOwner, name: &str) {
        let Some(d) = self.node_def(owner, &format!("Sounds/{name}")) else { return };
        let path = d.props.iter().find(|(k, _)| k == "stream").map(|(_, v)| var_value(v));
        let vol = d.props.iter().find(|(k, _)| k == "volume_db").and_then(|(_, v)| var_value(v).as_f64()).unwrap_or(0.0);
        if let Some(Value::Res(p)) = path {
            self.w.events.push(Event::Sound { path: p.to_string(), volume: vol as f32 });
        }
    }

    /// `spawn_particle_effect(scene, pos, dir)`: an effect the app plays.
    pub(crate) fn effect(&mut self, args: &[Value], base: Option<FVec>) -> Value {
        let Some(Value::Res(scene)) = args.first() else { return Value::Nil };
        let (x, y) = match args.get(1) {
            Some(Value::Vec2(x, y)) => (*x, *y),
            _ => (0.0, 0.0),
        };
        let dir = match args.get(2) {
            Some(Value::Vec2(x, y)) => (*x as f32, *y as f32),
            _ => (1.0, 0.0),
        };
        let mut pos = FVec::new(Fixed::from_f64(x), Fixed::from_f64(y));
        if let Some(b) = base {
            pos = pos + b;
        }
        self.w.events.push(Event::Effect { scene: scene.to_string(), pos, dir });
        Value::Nil
    }

    // ------------------------------------------------------------------------------------
    // Objects
    // ------------------------------------------------------------------------------------

    /// Runs a script method with an interpreter of its own (scripts can be running already).
    pub(crate) fn run(&mut self, inst: &InstRef, name: &str, args: Vec<Value>) -> Option<Value> {
        let loader = self.t.loader.clone()?;
        let mut vm = Vm::shared(loader);
        if !vm.has_script_method(inst, name) {
            return None;
        }
        match vm.call(self, inst, name, args) {
            Ok(v) => Some(v),
            Err(e) => {
                let path = inst.read().unwrap().chain.last().map(|s| s.path.clone()).unwrap_or_default();
                World::note_problem(self.t, format!("{path} {name}: {e}"));
                None
            }
        }
    }

    fn instantiate(&mut self, s: &StateScript, obj: ObjId, host: Option<Value>) -> Option<InstRef> {
        let loader = self.t.loader.clone()?;
        let Some(script) = loader.by_path(&s.path) else {
            World::note_problem(self.t, format!("{}: script not found", s.path));
            return None;
        };
        let mut vm = Vm::shared(loader);
        let inst = match vm.instantiate(self, script, Some(obj)) {
            Ok(i) => i,
            Err(e) => {
                World::note_problem(self.t, format!("{}: {e}", s.path));
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
            if s.node != "." && !s.node.is_empty() {
                w.set("state_name", Value::str(&s.node));
            }
            if let Some(h) = host {
                w.set("host", h);
            }
            w.set("fixed", Value::Lib("fixed"));
        }
        if let Err(e) = vm.ready(self, &inst) {
            World::note_problem(self.t, format!("{} (onready): {e}", s.path));
        }
        Some(inst)
    }

    /// Spawns the object of scene `scene` for player `owner`, at `pos` facing `facing`.
    pub(crate) fn spawn_scene(&mut self, owner: usize, scene: &str, pos: FVec, facing: i64, from_move: usize, data: Value) -> Option<Value> {
        let ch = self.w.chars[owner].clone();
        let idx = ch.file.character.projectiles.iter().position(|d| d.scene.as_deref() == Some(scene) && !d.states.is_empty());
        let Some(idx) = idx else {
            World::note_problem(self.t, format!("spawn_object: {scene} is not an imported object"));
            return None;
        };
        Some(self.spawn_def(owner, idx, pos, facing, from_move, data))
    }

    /// Makes a scripted object from its definition and its script objects.
    pub(crate) fn spawn_def(&mut self, owner: usize, idx: usize, pos: FVec, facing: i64, from_move: usize, data: Value) -> Value {
        let ch = self.w.chars[owner].clone();
        let def = &ch.file.character.projectiles[idx];
        let uid = self.w.next_uid;
        self.w.next_uid += 1;
        self.w.projs.push(Proj::scripted(owner, idx, from_move, pos, facing, uid));
        let me = Value::Obj(obj_id(uid));
        let root = def.script.as_ref().and_then(|s| self.instantiate(s, obj_id(uid), None));
        let mut states = Vec::new();
        for (k, st) in def.states.iter().enumerate() {
            states.push(self.instantiate(&st.script, obj_state_id(uid, k), Some(me.clone())));
        }
        if let Some(r) = &root {
            r.write().unwrap().set("data", data.clone());
        }
        for s in states.iter().flatten() {
            s.write().unwrap().set("data", data.clone());
        }
        self.t.objs.insert(uid, ObjInsts { root: root.clone(), states, ..Default::default() });
        if let Some(r) = &root {
            for hook in ["_ready", "init"] {
                self.run(r, hook, vec![]);
            }
        }
        me
    }

    fn obj_state_inst(&self, uid: u32, k: usize) -> Option<InstRef> {
        self.t.objs.get(&uid)?.states.get(k).cloned().flatten()
    }

    fn state_index(&self, uid: u32, name: &str) -> Option<usize> {
        let p = self.proj(uid)?;
        self.w.chars[p.owner].file.character.projectiles.get(p.idx)?.states.iter().position(|s| s.node == name)
    }

    /// Switches an object to the state called `name` (unknown names are ignored, as in the
    /// original's state machine).
    pub(crate) fn obj_change_state(&mut self, uid: u32, name: &str) {
        let Some(k) = self.state_index(uid, name) else { return };
        let prev = self.proj(uid).map(|p| p.state);
        if let Some(old) = prev.and_then(|s| self.obj_state_inst(uid, s)) {
            self.run(&old, "_exit", vec![]);
            let dis = old.read().unwrap().get("disable_at_end").is_some_and(|v| v.truthy());
            if dis {
                if let Some(p) = self.proj_mut(uid) {
                    p.dead = true;
                }
            }
        }
        if let Some(p) = self.proj_mut(uid) {
            p.state = k;
            p.sframe = 0;
            p.connected.clear();
        }
        if let Some(o) = self.t.objs.get_mut(&uid) {
            o.entered = None;
        }
    }

    fn field(inst: &Option<InstRef>, name: &str) -> Option<Value> {
        inst.as_ref().and_then(|i| i.read().unwrap().get(name))
    }

    fn flag(inst: &Option<InstRef>, name: &str) -> bool {
        Self::field(inst, name).is_some_and(|v| v.truthy())
    }

    fn fixed_field(inst: &Option<InstRef>, name: &str, d: f64) -> Fixed {
        Self::field(inst, name).and_then(|v| fixed_of(&v).ok()).unwrap_or(Fixed::from_f64(d))
    }

    /// One tick of an object, as the original's `BaseObj.tick` (hitlag, else its state machine).
    pub(crate) fn native_tick(&mut self, uid: u32) {
        let Some(p) = self.proj(uid) else { return };
        if p.dead {
            return;
        }
        if p.hitlag > 0 {
            if let Some(p) = self.proj_mut(uid) {
                p.hitlag -= 1;
            }
            return;
        }
        let k = p.state;
        let st = self.obj_state_inst(uid, k);
        let root = self.t.objs.get(&uid).and_then(|o| o.root.clone());
        // entering the state
        if self.t.objs.get(&uid).is_some_and(|o| o.entered != Some(k)) {
            if let Some(o) = self.t.objs.get_mut(&uid) {
                o.entered = Some(k);
            }
            if let Some(s) = &st {
                let mut w = s.write().unwrap();
                w.set("current_tick", Value::Int(-1));
                w.set("active", Value::Bool(true));
            }
            if let Some(s) = st.clone() {
                if let Some(Value::Str(next)) = self.run(&s, "_enter", vec![]) {
                    self.obj_change_state(uid, &next);
                    return;
                }
            }
        }
        let anim_len = Self::field(&st, "anim_length").and_then(|v| v.as_i64()).unwrap_or(1).max(1) as u16;
        let endless = Self::flag(&st, "endless");
        let mut queued: Option<String> = None;
        let sframe = self.proj(uid).map(|p| p.sframe).unwrap_or(0);
        if sframe < anim_len || endless {
            let frame = sframe + 1;
            if let Some(p) = self.proj_mut(uid) {
                p.sframe = frame;
            }
            let tick = frame as i64 - 1;
            if let Some(s) = &st {
                let mut w = s.write().unwrap();
                w.set("current_tick", Value::Int(tick));
                w.set("current_real_tick", Value::Int(tick));
            }
            // ObjectState's own work at this tick: enter force, timed force, exported projectile
            let fac = Fixed::from_int(self.proj(uid).map(|p| p.facing).unwrap_or(1));
            let force_from = |sp: &str, dx: &str, dy: &str| -> Option<FVec> {
                let speed = Self::fixed_field(&st, sp, 0.0);
                (speed != Fixed::ZERO).then(|| FVec::new(Self::fixed_field(&st, dx, 0.0) * speed * fac, Self::fixed_field(&st, dy, 0.0) * speed))
            };
            let enter = if tick == 0 { force_from("enter_force_speed", "enter_force_dir_x", "enter_force_dir_y") } else { None };
            let timed = (Self::field(&st, "force_tick").and_then(|v| v.as_i64()) == Some(tick)).then(|| force_from("force_speed", "force_dir_x", "force_dir_y")).flatten();
            if let Some(p) = self.proj_mut(uid) {
                for f in [enter, timed].into_iter().flatten() {
                    p.force = p.force + f;
                }
            }
            if Self::field(&st, "projectile_tick").and_then(|v| v.as_i64()) == Some(tick) {
                if let Some(Value::Res(scene)) = Self::field(&st, "projectile_scene") {
                    let (px, py) = (Self::field(&st, "projectile_pos_x").and_then(|v| v.as_i64()).unwrap_or(0), Self::field(&st, "projectile_pos_y").and_then(|v| v.as_i64()).unwrap_or(0));
                    if let Some(p) = self.proj(uid).cloned() {
                        let at = FVec::new(p.pos.x + Fixed::from_int(px * p.facing), p.pos.y + Fixed::from_int(py));
                        self.spawn_scene(p.owner, &scene, at, p.facing, p.from_move, Value::Nil);
                    }
                }
            }
            if let Some(s) = st.clone() {
                for name in [format!("_frame_{tick}_shared"), format!("_frame_{tick}")] {
                    if let Some(Value::Str(next)) = self.run(&s, &name, vec![]) {
                        queued = Some(next.to_string());
                        break;
                    }
                }
            }
        }
        // physics flags of the state, the body's numbers from the object's own values
        if let Some(p) = self.proj(uid).cloned() {
            let mut p = p;
            let gravity = Self::fixed_field(&root, "gravity", 0.0);
            let max_fall = Self::fixed_field(&root, "max_fall_speed", 10.0);
            let fric = Self::fixed_field(&root, if p.grounded { "ground_friction" } else { "air_friction" }, 0.0);
            let max_speed = Self::fixed_field(&root, if p.grounded { "max_ground_speed" } else { "max_air_speed" }, 10.0);
            if Self::flag(&st, "apply_fric") && max_speed != Fixed::ZERO {
                p.vel.x = p.vel.x * (Fixed::ONE - fric / max_speed);
            }
            if Self::flag(&st, "apply_grav") && p.vel.y < max_fall {
                p.force.y += gravity;
            }
            if Self::flag(&st, "apply_custom_x_fric") {
                p.vel.x = p.vel.x * (Fixed::ONE - Self::fixed_field(&st, "custom_x_fric", 0.0));
            }
            if Self::flag(&st, "apply_custom_y_fric") {
                p.vel.y = p.vel.y * (Fixed::ONE - Self::fixed_field(&st, "custom_y_fric", 0.0));
            }
            if Self::flag(&st, "apply_custom_grav") {
                let (g, fall) = (Self::fixed_field(&st, "custom_grav", 0.0), Self::fixed_field(&st, "custom_grav_max_fall_speed", 0.0));
                if p.vel.y < fall {
                    p.vel.y = (p.vel.y + g).min(fall);
                }
            }
            if Self::flag(&st, "apply_forces") || Self::flag(&st, "apply_forces_no_limit") {
                p.vel = p.vel + p.force;
                p.force = FVec::ZERO;
                p.pos = p.pos + p.vel;
            }
            p.grounded = p.pos.y >= Fixed::ZERO;
            if p.pos.y > Fixed::ZERO {
                p.pos.y = Fixed::ZERO;
            }
            p.age = p.age.saturating_add(1);
            let half = Fixed::from_int(self.w.cfg.stage_half_width as i64 + 300);
            if p.age >= MAX_OBJECT_AGE || p.pos.x.abs() > half {
                p.dead = true;
            }
            // the sprite: the state's animation, one frame per `ticks_per_frame`
            let anim = Self::field(&st, "sprite_animation").map(|v| v.to_text()).filter(|a| !a.is_empty()).unwrap_or_else(|| {
                self.w.chars[p.owner].file.character.projectiles[p.idx].states.get(k).map(|s| s.node.clone()).unwrap_or_default()
            });
            let tpf = Self::field(&st, "ticks_per_frame").and_then(|v| v.as_i64()).unwrap_or(1).max(1) as u32;
            p.anim = anim;
            p.frame = p.sframe.saturating_sub(1) as u32 / tpf;
            if let Some(slot) = self.proj_mut(uid) {
                *slot = p;
            }
        }
        if queued.is_none() {
            if let Some(s) = st.clone() {
                for name in ["_tick_before", "_tick", "_tick_after"] {
                    if let Some(Value::Str(next)) = self.run(&s, name, vec![]) {
                        queued = Some(next.to_string());
                        break;
                    }
                }
            }
        }
        // a state that ran its length goes to its fallback
        if queued.is_none() && !endless && self.proj(uid).is_some_and(|p| p.sframe >= anim_len && p.state == k) {
            let fallback = Self::field(&st, "fallback_state").map(|v| v.to_text()).unwrap_or_default();
            if self.state_index(uid, &fallback).is_some() && fallback != self.proj(uid).map(|p| self.w.chars[p.owner].file.character.projectiles[p.idx].states[k].node.clone()).unwrap_or_default() {
                queued = Some(fallback);
            } else if Self::flag(&st, "disable_at_end") {
                if let Some(p) = self.proj_mut(uid) {
                    p.dead = true;
                }
            }
        }
        if let Some(n) = queued {
            self.obj_change_state(uid, &n);
        }
    }

    pub(crate) fn obj_get(&mut self, uid: u32, name: &str) -> Result<Value, Error> {
        let Some(p) = self.proj(uid).cloned() else { return Ok(Value::Nil) };
        let sprite = |s: &mut Self| {
            let path = ["Flip/Sprite", "Sprite"].into_iter().find(|n| s.node_def(NodeOwner::Obj(uid), n).is_some());
            path.map(|n| Value::Obj(s.node_rec(NodeOwner::Obj(uid), n))).unwrap_or(Value::Obj(crate::script::SINK))
        };
        Ok(match name {
            "obj_name" | "name" => Value::str(&p.obj_name),
            "creator" => Value::Obj(fighter_obj(p.owner)),
            "id" => Value::Int(p.owner as i64 + 1),
            "hitlag_ticks" => Value::Int(p.hitlag as i64),
            "disabled" => Value::Bool(p.dead),
            "current_tick" => Value::Int(p.age as i64),
            "is_ghost" => Value::Bool(false),
            "fixed" => Value::Lib("fixed"),
            "objs_map" => self.objs_map(),
            "sprite" => sprite(self),
            "state_machine" | "collision_box" => Value::Obj(crate::script::SINK),
            "hurtbox" => match self.node_def(NodeOwner::Obj(uid), "Hurtbox") {
                Some(_) => Value::Obj(self.node_rec(NodeOwner::Obj(uid), "Hurtbox")),
                None => Value::Obj(crate::script::SINK),
            },
            "position" => Value::Vec2(p.pos.x.to_f32() as f64, p.pos.y.to_f32() as f64),
            "obj_data" => crate::script::object_data(p.pos, p.vel),
            _ => {
                // what a script set, else the scene root's value, else BaseProjectile's default
                if let Some(v) = self.t.objs.get(&uid).and_then(|o| o.props.get(name)).cloned() {
                    return Ok(v);
                }
                if let Some(v) = self.node_def(NodeOwner::Obj(uid), ".").and_then(|d| d.props.iter().find(|(k, _)| k == name)).map(|(_, v)| var_value(v)) {
                    return Ok(v);
                }
                // what its scripts (the base classes' included) declare: `particles`, constants...
                let root = self.t.objs.get(&uid).and_then(|o| o.root.clone());
                if let (Some(r), Some(l)) = (root, self.t.loader.clone()) {
                    if let Some(v) = Vm::shared(l).script_member(self, &r, name).filter(|v| !matches!(v, Value::Nil)) {
                        return Ok(v);
                    }
                }
                return obj_default(name).ok_or_else(|| Error::Unsupported(format!("object.{name}")));
            }
        })
    }

    /// The object's hurtbox (its scene's `Hurtbox` node, as scripts left it): center and half
    /// size in world space, at whole-pixel positions like the original's.
    pub(crate) fn obj_hurtbox(&mut self, uid: u32) -> Option<(FVec, FVec)> {
        let (x, y, w, h) = self.hurtbox_numbers(uid)?;
        if w == 0 && h == 0 {
            return None;
        }
        let p = self.proj(uid)?;
        let c = FVec::new(Fixed::from_int(p.pos.x.round() + x * p.facing), Fixed::from_int(p.pos.y.round() + y));
        Some((c, FVec::new(Fixed::from_int(w), Fixed::from_int(h))))
    }

    /// The `Hurtbox` node's x, y, width and height (CollisionBox defaults where unset).
    fn hurtbox_numbers(&mut self, uid: u32) -> Option<(i64, i64, i64, i64)> {
        self.node_def(NodeOwner::Obj(uid), "Hurtbox")?;
        let ObjId(id) = self.node_rec(NodeOwner::Obj(uid), "Hurtbox");
        let i = (id - NODE_BASE) as usize;
        let g = |s: &Self, k: &str, d: i64| s.node_prop(i, k).as_i64().unwrap_or(d);
        Some((g(self, "x", 0), g(self, "y", 0), g(self, "width", 20), g(self, "height", 20)))
    }

    /// The original's `BaseProjectile.hit_by`: the object freezes for the hit's hitlag, and the
    /// fighter who struck it too; it may free the fighter to cancel.
    fn native_hit_by(&mut self, uid: u32, hb: &Value) {
        if key(hb, "throw").truthy() {
            return;
        }
        let get = |s: &mut Self, k: &str| s.obj_get(uid, k).unwrap_or(Value::Nil);
        let modifier = fixed_of(&get(self, "hitlag_modifier")).unwrap_or(Fixed::ONE);
        let scaled = |t: &Value| (modifier * Fixed::from_int(t.as_i64().unwrap_or(0))).round().clamp(0, u16::MAX as i64) as u16;
        let victim = scaled(&key(hb, "victim_hitlag"));
        if let Some(p) = self.proj_mut(uid) {
            p.hitlag = victim;
        }
        let host = key(hb, "host").to_text();
        let fighter = match host.as_str() {
            "P1" => Some(0),
            "P2" => Some(1),
            _ => None,
        };
        if let Some(a) = fighter {
            if get(self, "apply_hitlag_when_hit_by_melee").truthy() {
                let t = scaled(&key(hb, "hitlag_ticks"));
                let f = &mut self.w.f[a];
                f.hitlag = f.hitlag.max(t);
            }
            if get(self, "free_cancel_on_hit").truthy() || get(self, "hit_cancel_on_hit").truthy() {
                if let crate::Act::Move(run) = &mut self.w.f[a].act {
                    run.hit = true;
                }
            }
        }
    }

    /// The original's object hits (`apply_hitboxes`): the opponent's attacks reach an object
    /// that can be hit by melee, and the opponent's projectiles and objects reach one that is
    /// not projectile immune. The object's `hit_by` (its script's, else the engine's) takes it.
    fn hit_objects(&mut self) {
        // strikes are remembered while the move or state that made them lasts
        let moves: [Option<usize>; 2] = [0, 1].map(|i| match &self.w.f[i].act {
            crate::Act::Move(r) => Some(r.idx),
            _ => None,
        });
        let states: Vec<(u32, usize)> = self.w.projs.iter().filter(|p| p.is_object() && !p.dead).map(|p| (p.uid, p.state)).collect();
        for o in self.t.objs.values_mut() {
            o.struck.retain(|(s, _)| match s {
                Striker::Fighter(i, m) => moves[*i] == Some(*m),
                Striker::Obj(u, k) => states.contains(&(*u, *k)),
                Striker::Plain(_) => false,
            });
        }
        let mut hits: Vec<(u32, Striker, charfmt::HitBox, (String, u16), FVec)> = Vec::new();
        for (uid, _) in states.clone() {
            let Some(hurt) = self.obj_hurtbox(uid) else { continue };
            let Some(owner) = self.proj(uid).map(|p| p.owner) else { continue };
            let melee = self.obj_get(uid, "can_be_hit_by_melee").is_ok_and(|v| v.truthy());
            let immune = self.obj_get(uid, "projectile_immune").is_ok_and(|v| v.truthy());
            // the opponent's attacks, and the owner's own when the object takes its team's hits
            let own_team = self.obj_get(uid, "damages_own_team").is_ok_and(|v| v.truthy());
            let fighters = if own_team { vec![1 - owner, owner] } else { vec![1 - owner] };
            let (w, t) = (&*self.w, &*self.t);
            let done = |st: &Striker, k: &(String, u16)| t.objs.get(&uid).is_some_and(|o| o.struck.iter().any(|(a, b)| a == st && b == k));
            for a in fighters {
                let (Some(idx), true) = (moves[a], w.f[a].hitlag == 0) else { continue };
                for (c, h, hb, cycle) in w.hitboxes(a) {
                    let k = crate::hit_key(&hb, cycle);
                    let st = Striker::Fighter(a, idx);
                    if hb.kind == charfmt::HitKind::ThrowDamage || hb.effects.throw || !(melee || hb.kind == charfmt::HitKind::Detect) || done(&st, &k) {
                        continue;
                    }
                    if crate::overlap(&(c, h), &hurt) {
                        hits.push((uid, st, hb, k, crate::overlap_center(&(c, h), &hurt)));
                        break;
                    }
                }
            }
            if immune {
                continue;
            }
            for (pi, q) in w.projs.iter().enumerate() {
                if q.dead || q.owner == owner || q.hitlag > 0 {
                    continue;
                }
                let d = &w.chars[q.owner].file.character.projectiles[q.idx];
                let boxes = if q.is_object() { d.states.get(q.state).map(|s| &s.hitboxes[..]).unwrap_or(&[]) } else { &d.hitboxes[..] };
                let Some(frame) = (if q.is_object() { Some(q.sframe) } else { q.hit_frame(d.behavior.as_deref()) }) else { continue };
                for hb in boxes {
                    let Some(cycle) = crate::hitbox_live(hb, frame) else { continue };
                    let k = crate::hit_key(hb, cycle);
                    let st = if q.is_object() { Striker::Obj(q.uid, q.state) } else { Striker::Plain(pi) };
                    if hb.kind == charfmt::HitKind::ThrowDamage || hb.effects.throw || done(&st, &k) {
                        continue;
                    }
                    let whole = FVec::new(Fixed::from_int(q.pos.x.round()), Fixed::from_int(q.pos.y.round()));
                    let (c, h) = crate::rect_at(whole, q.facing, &hb.rect);
                    if crate::overlap(&(c, h), &hurt) {
                        hits.push((uid, st, hb.clone(), k, crate::overlap_center(&(c, h), &hurt)));
                        break;
                    }
                }
            }
        }
        for (uid, st, hb, k, spark) in hits {
            if let Some(o) = self.t.objs.get_mut(&uid) {
                o.struck.push((st.clone(), k));
            }
            let (host, attacker, mv) = match &st {
                Striker::Fighter(a, m) => (format!("P{}", a + 1), *a, Some(*m)),
                Striker::Obj(u, _) => match self.proj(*u) {
                    Some(q) => (q.obj_name.clone(), q.owner, None),
                    None => continue,
                },
                Striker::Plain(pi) => {
                    // a plain projectile is spent on the object, as on a fighter
                    let q = &mut self.w.projs[*pi];
                    let pierces = self.w.chars[q.owner].file.character.projectiles[q.idx].pierces;
                    q.dead |= !pierces;
                    (format!("P{}", q.owner + 1), q.owner, None)
                }
            };
            let dir = (if spark.x >= self.proj(uid).map(|p| p.pos.x).unwrap_or(Fixed::ZERO) { -1.0 } else { 1.0 }, 0.0);
            self.w.events.push(Event::ObjectHit { attacker, mv, hitbox: hb.id.clone(), pos: spark, dir, damage: hb.damage });
            let v = hitbox_value(&hb, &host);
            let root = self.t.objs.get(&uid).and_then(|o| o.root.clone());
            match root {
                Some(r) if sc_has(self, &r, "hit_by") => {
                    self.run(&r, "hit_by", vec![v]);
                }
                _ => self.native_hit_by(uid, &v),
            }
        }
    }

    pub(crate) fn obj_set(&mut self, uid: u32, name: &str, v: Value) -> Result<(), Error> {
        let Some(p) = self.proj_mut(uid) else { return Ok(()) };
        match name {
            "hitlag_ticks" => p.hitlag = v.as_i64().unwrap_or(0).clamp(0, u16::MAX as i64) as u16,
            "disabled" => p.dead = v.truthy(),
            _ => {
                if let Some(o) = self.t.objs.get_mut(&uid) {
                    o.props.insert(name.to_string(), v);
                }
            }
        }
        Ok(())
    }

    /// Every object by name, the fighters too (`objs_map`).
    pub(crate) fn objs_map(&self) -> Value {
        let mut m: Vec<(Value, Value)> = vec![(Value::str("P1"), Value::Obj(fighter_obj(0))), (Value::str("P2"), Value::Obj(fighter_obj(1)))];
        for p in self.w.projs.iter().filter(|p| p.is_object() && !p.dead) {
            m.push((Value::str(&p.obj_name), Value::Obj(obj_id(p.uid))));
        }
        Value::dict(m)
    }

    /// `obj_from_name(name)`.
    pub(crate) fn obj_from_name(&self, name: &str) -> Value {
        match name {
            "P1" => Value::Obj(fighter_obj(0)),
            "P2" => Value::Obj(fighter_obj(1)),
            _ => self.w.projs.iter().find(|p| p.obj_name == name && !p.dead).map(|p| Value::Obj(obj_id(p.uid))).unwrap_or(Value::Nil),
        }
    }

    /// A deterministic random whole number in `lo..=hi` (`randi_range`).
    pub(crate) fn randi_range(&mut self, lo: i64, hi: i64) -> Value {
        let r = &mut self.w.rng;
        *r ^= *r << 13;
        *r ^= *r >> 7;
        *r ^= *r << 17;
        if hi <= lo {
            return Value::Int(lo);
        }
        Value::Int(lo + (*r % (hi - lo + 1) as u64) as i64)
    }

    pub(crate) fn obj_call(&mut self, uid: u32, method: &str, args: &[Value]) -> Result<Value, Error> {
        let Some(p) = self.proj(uid).cloned() else { return Ok(Value::Nil) };
        let fac = Fixed::from_int(p.facing);
        let f2 = |a: &[Value]| -> Result<(Fixed, Fixed), Error> { Ok((fixed_of(&arg(a, 0))?, fixed_of(&arg(a, 1))?)) };
        let edit = |s: &mut Self, f: &dyn Fn(&mut Proj)| {
            if let Some(p) = s.proj_mut(uid) {
                f(p);
            }
        };
        Ok(match method {
            "tick" => {
                self.native_tick(uid);
                Value::Nil
            }
            "get_pos" => xy(Value::Int(p.pos.x.round()), Value::Int(p.pos.y.round())),
            "get_pos_visual" | "get_center_position_float" | "get_global_center" => Value::Vec2(p.pos.x.to_f32() as f64, p.pos.y.to_f32() as f64),
            "get_collision_box" => {
                let half = FVec::new(Fixed::from_int(10), Fixed::from_int(10));
                crate::script::collision_box(p.pos, half)
            }
            "get_hurtbox_center_float" | "get_hurtbox_center" => {
                let (x, y, _, _) = self.hurtbox_numbers(uid).unwrap_or_default();
                let (cx, cy) = (p.pos.x.round() + x * p.facing, p.pos.y.round() + y);
                if method == "get_hurtbox_center" { xy(Value::Int(cx), Value::Int(cy)) } else { Value::Vec2(cx as f64, cy as f64) }
            }
            "hurtbox_pos_relative_float" | "hurtbox_pos_relative" => {
                let (x, y, _, _) = self.hurtbox_numbers(uid).unwrap_or_default();
                if method == "hurtbox_pos_relative" { xy(Value::Int(x * p.facing), Value::Int(y)) } else { Value::Vec2((x * p.facing) as f64, y as f64) }
            }
            "hit_by" => {
                self.native_hit_by(uid, &arg(args, 0));
                Value::Nil
            }
            "obj_local_pos" => Value::Vec2(0.0, 0.0),
            "get_vel" => xy(text(p.vel.x), text(p.vel.y)),
            "set_pos" => {
                let (x, y) = f2(args)?;
                edit(self, &|p| p.pos = FVec::new(x, y));
                Value::Nil
            }
            "set_x" => {
                let x = fixed_of(&arg(args, 0))?;
                edit(self, &|p| p.pos.x = x);
                Value::Nil
            }
            "set_y" => {
                let y = fixed_of(&arg(args, 0))?;
                edit(self, &|p| p.pos.y = y);
                Value::Nil
            }
            "set_vel" => {
                let (x, y) = f2(args)?;
                edit(self, &|p| p.vel = FVec::new(x, y));
                Value::Nil
            }
            "apply_force" => {
                let (x, y) = f2(args)?;
                edit(self, &|p| p.force = p.force + FVec::new(x, y));
                Value::Nil
            }
            "apply_force_relative" => {
                let (x, y) = f2(args)?;
                edit(self, &|p| p.force = p.force + FVec::new(x * fac, y));
                Value::Nil
            }
            "apply_forces" | "apply_forces_no_limit" => {
                edit(self, &|p| {
                    p.vel = p.vel + p.force;
                    p.force = FVec::ZERO;
                    p.pos = p.pos + p.vel;
                });
                Value::Nil
            }
            "reset_momentum" => {
                edit(self, &|p| {
                    p.vel = FVec::ZERO;
                    p.force = FVec::ZERO;
                });
                Value::Nil
            }
            "move_directly" => {
                let (x, y) = f2(args)?;
                edit(self, &|p| p.pos = p.pos + FVec::new(x, y));
                Value::Nil
            }
            "move_directly_relative" => {
                let (x, y) = f2(args)?;
                edit(self, &|p| p.pos = p.pos + FVec::new(x * fac, y));
                Value::Nil
            }
            "get_facing_int" => Value::Int(p.facing),
            "get_facing" => Value::str(if p.facing < 0 { "Left" } else { "Right" }),
            "set_facing" => {
                let d = arg(args, 0).as_i64().unwrap_or(1);
                edit(self, &|p| p.facing = if d < 0 { -1 } else { 1 });
                Value::Nil
            }
            "is_grounded" => Value::Bool(p.grounded),
            "disable" | "queue_free" => {
                edit(self, &|p| p.dead = true);
                Value::Nil
            }
            "change_state" | "queue_state_change" => {
                self.obj_change_state(uid, &arg(args, 0).to_text());
                Value::Nil
            }
            "current_state" => Value::Obj(obj_state_id(uid, p.state)),
            "get_fighter" => Value::Obj(fighter_obj(p.owner)),
            "get_opponent" => Value::Obj(fighter_obj(1 - p.owner)),
            "obj_from_name" => self.obj_from_name(&arg(args, 0).to_text()),
            "is_in_group" => Value::Bool(false),
            "randi_range" => self.randi_range(arg(args, 0).as_i64().unwrap_or(0), arg(args, 1).as_i64().unwrap_or(0)),
            "play_sound" => {
                self.play_sound(NodeOwner::Obj(uid), &arg(args, 0).to_text());
                Value::Nil
            }
            "spawn_particle_effect" => self.effect(args, None),
            "spawn_particle_effect_relative" => self.effect(args, Some(p.pos)),
            "spawn_object" => {
                let scene = match args.first() {
                    Some(Value::Res(s)) => s.to_string(),
                    _ => return Ok(Value::Nil),
                };
                let (x, y) = (arg(args, 1).as_i64().unwrap_or(0), arg(args, 2).as_i64().unwrap_or(0));
                let relative = args.get(3).is_none_or(|v| v.truthy());
                let at = if relative { FVec::new(p.pos.x + Fixed::from_int(x * p.facing), p.pos.y + Fixed::from_int(y)) } else { FVec::new(Fixed::from_int(x), Fixed::from_int(y)) };
                self.spawn_scene(p.owner, &scene, at, p.facing, p.from_move, arg(args, 4)).unwrap_or(Value::Nil)
            }
            "get_active_hitboxes" => Value::array(vec![]),
            // presentation (objects' own invulnerability is their hurtbox, which scripts move)
            "stop_sound" | "screen_bump" | "update_data" | "stop_particles" | "rumble" | "set_material" | "update" | "emit_signal" | "connect"
            | "start_invulnerability" | "end_invulnerability" | "start_projectile_invulnerability" | "end_projectile_invulnerability"
            | "start_throw_invulnerability" | "end_throw_invulnerability" | "grab_camera_focus" | "release_camera_focus" | "set_grounded"
            | "update_grounded" | "create_speed_after_image" | "_create_speed_after_image" | "start_emitting" | "stop_emitting" => Value::Nil,
            _ => return Err(Error::Unsupported(format!("object.{method}()"))),
        })
    }

    /// An object state's own values the engine answers.
    pub(crate) fn obj_state_get(&mut self, uid: u32, k: usize, name: &str) -> Result<Value, Error> {
        let Some(p) = self.proj(uid) else { return Ok(Value::Nil) };
        let current = p.state == k;
        Ok(match name {
            "host" => Value::Obj(obj_id(uid)),
            "fixed" => Value::Lib("fixed"),
            "active" => Value::Bool(current),
            "current_tick" | "current_real_tick" if current => Value::Int(p.sframe as i64 - 1),
            "name" => Value::str(&self.w.chars[p.owner].file.character.projectiles[p.idx].states.get(k).map(|s| s.node.clone()).unwrap_or_default()),
            _ => return Err(Error::Unsupported(name.to_string())),
        })
    }

    pub(crate) fn obj_state_call(&mut self, uid: u32, _k: usize, method: &str, args: &[Value]) -> Result<Value, Error> {
        if crate::script::ENGINE_DONE.contains(&method) {
            return Ok(Value::Nil);
        }
        Ok(match method {
            "queue_state_change" => {
                self.obj_change_state(uid, &arg(args, 0).to_text());
                Value::Nil
            }
            _ => return Err(Error::Unsupported(method.to_string())),
        })
    }
}

impl World {
    /// The scripted objects' tick (after the plain projectiles'), and their nodes' animations.
    pub(crate) fn tick_objects(&mut self) {
        let Some(mut sc) = self.scripts.take() else { return };
        let uids: Vec<u32> = self.projs.iter().filter(|p| p.is_object() && !p.dead).map(|p| p.uid).collect();
        {
            let mut ctx = Ctx { w: self, t: &mut sc.t };
            for uid in uids {
                let root = ctx.t.objs.get(&uid).and_then(|o| o.root.clone());
                let scripted_tick = root.as_ref().is_some_and(|r| sc_has(&ctx, r, "tick"));
                match root {
                    Some(r) if scripted_tick => {
                        ctx.run(&r, "tick", vec![]);
                    }
                    _ => ctx.native_tick(uid),
                }
            }
        }
        self.scripts = Some(sc);
    }

    /// Attacks and projectiles striking scripted objects (after the fighters' hits).
    pub(crate) fn hit_objects(&mut self) {
        let Some(mut sc) = self.scripts.take() else { return };
        {
            let mut ctx = Ctx { w: self, t: &mut sc.t };
            ctx.hit_objects();
        }
        self.scripts = Some(sc);
    }

    /// Spawns a scripted object from data (a move's `spawns` naming one).
    pub(crate) fn spawn_object_def(&mut self, owner: usize, idx: usize, pos: FVec, facing: i64, from_move: usize) {
        let Some(mut sc) = self.scripts.take() else { return };
        {
            let mut ctx = Ctx { w: self, t: &mut sc.t };
            ctx.spawn_def(owner, idx, pos, facing, from_move, Value::Nil);
        }
        self.scripts = Some(sc);
    }

    /// Plays the nodes' sprite animations a tick and sends `animation_finished`.
    pub(crate) fn tick_nodes(&mut self) {
        let Some(mut sc) = self.scripts.take() else { return };
        {
            let mut ctx = Ctx { w: self, t: &mut sc.t };
            let mut finished: Vec<(InstRef, String)> = Vec::new();
            for i in 0..ctx.t.nodes.len() {
                if ctx.t.nodes[i].freed || !ctx.node_prop(i, "playing").truthy() {
                    continue;
                }
                let (owner, path) = (ctx.t.nodes[i].owner, ctx.t.nodes[i].path.clone());
                let anim = ctx.node_prop(i, "animation").to_text();
                let Some(a) = ctx.node_def(owner, &path).and_then(|d| d.anims.iter().find(|x| x.name == anim).or(d.anims.first())).cloned() else { continue };
                let speed = ctx.node_prop(i, "speed_scale").as_f64().unwrap_or(1.0) as f32;
                let n = &mut ctx.t.nodes[i];
                n.acc += a.fps * speed / 60.0;
                let mut frame = n.props.get("frame").and_then(Value::as_i64).unwrap_or(0) as u32;
                while n.acc >= 1.0 {
                    n.acc -= 1.0;
                    frame += 1;
                    if frame >= a.frames.max(1) {
                        // Godot sends `animation_finished` at the end of every loop too
                        finished.extend(n.signals.iter().filter(|s| s.0 == "animation_finished").map(|s| (s.1.clone(), s.2.clone())));
                        if a.looped {
                            frame = 0;
                        } else {
                            frame = a.frames.max(1) - 1;
                            n.props.insert("playing".into(), Value::Bool(false));
                            n.acc = 0.0;
                            break;
                        }
                    }
                }
                n.props.insert("frame".into(), Value::Int(frame as i64));
            }
            for (inst, m) in finished {
                ctx.run(&inst, &m, vec![]);
            }
        }
        self.scripts = Some(sc);
    }

    fn view(&self, t: &crate::script::Tables, owner: NodeOwner, path: &str, scene: &SceneDef, def: &NodeDef, rec: Option<&NodeRec>) -> NodeView {
        let get = |k: &str| -> Option<Value> { rec.and_then(|r| r.props.get(k).cloned()).or_else(|| def.props.iter().find(|(n, _)| n == k).map(|(_, v)| var_value(v))) };
        let v2 = |v: Option<Value>, d: (f32, f32)| match v {
            Some(Value::Vec2(x, y)) => (x as f32, y as f32),
            _ => d,
        };
        let _ = t;
        let player = match owner {
            NodeOwner::Fighter(i) => i,
            NodeOwner::Obj(uid) => self.projs.iter().find(|p| p.uid == uid).map(|p| p.owner).unwrap_or(0),
        };
        let mut m = color_of(&get("modulate").unwrap_or(Value::Color(1.0, 1.0, 1.0, 1.0)));
        let sm = color_of(&get("self_modulate").unwrap_or(Value::Color(1.0, 1.0, 1.0, 1.0)));
        for (a, b) in m.iter_mut().zip(sm) {
            *a *= b;
        }
        NodeView {
            player,
            scene: scene.path.clone(),
            path: path.to_string(),
            ty: def.ty.clone(),
            anim: get("animation").map(|v| v.to_text()).unwrap_or_else(|| "default".into()),
            frame: get("frame").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as u32,
            pos: v2(get("position"), (0.0, 0.0)),
            flipped: false,
            flip_h: get("flip_h").is_some_and(|v| v.truthy()),
            modulate: m,
            scale: v2(get("scale"), (1.0, 1.0)),
            z: get("z_index").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            rotation: rotation_of(&get),
            texture: match get("texture") {
                Some(Value::Res(p)) => Some(p.to_string()),
                _ => None,
            },
            centered: get("centered").is_none_or(|v| v.truthy()),
            offset: v2(get("offset"), (0.0, 0.0)),
        }
    }

    /// The nodes scripts put on the UI layer (cut-ins), visible ones, in the order added.
    pub fn ui_nodes(&self) -> Vec<NodeView> {
        let Some(sc) = &self.scripts else { return vec![] };
        let t = &sc.t;
        let mut out = Vec::new();
        for rec in t.nodes.iter().filter(|n| n.ui.is_some() && !n.freed) {
            let scene_path = match rec.owner {
                NodeOwner::Fighter(i) => self.chars[i].file.character.scene.clone(),
                NodeOwner::Obj(uid) => self.projs.iter().find(|p| p.uid == uid).and_then(|p| self.chars[p.owner].file.character.projectiles[p.idx].scene.clone()),
            };
            let i = match rec.owner {
                NodeOwner::Fighter(i) => i,
                NodeOwner::Obj(uid) => self.projs.iter().find(|p| p.uid == uid).map(|p| p.owner).unwrap_or(0),
            };
            let Some(scene) = scene_path.and_then(|sp| self.chars[i].file.character.scenes.iter().find(|s| s.path == sp)) else { continue };
            let Some(def) = scene.nodes.iter().find(|n| n.path == rec.path) else { continue };
            let v = self.view(t, rec.owner, &rec.path, scene, def, Some(rec));
            let visible = rec.props.get("visible").map(|v| v.truthy()).unwrap_or_else(|| def.props.iter().find(|(k, _)| k == "visible").is_none_or(|(_, v)| var_value(v).truthy()));
            if visible {
                out.push(v);
            }
        }
        out
    }

    /// An object's sprite nodes to draw: its main sprite showing its state's animation, then
    /// the other visible sprites of its scene, positioned from the object (game pixels).
    pub fn object_nodes(&self, uid: u32) -> Vec<NodeView> {
        let Some(p) = self.projs.iter().find(|p| p.uid == uid) else { return vec![] };
        let ch = &self.chars[p.owner].file.character;
        let Some(scene) = ch.projectiles[p.idx].scene.as_ref().and_then(|sp| ch.scenes.iter().find(|s| &s.path == sp)) else { return vec![] };
        let main = main_sprite(scene);
        let mut out = self.scene_nodes(NodeOwner::Obj(uid), scene, |_, _| true);
        for v in out.iter_mut().filter(|v| Some(v.path.as_str()) == main) {
            let set = self.scripts.as_ref().is_some_and(|sc| sc.t.nodes.iter().any(|n| n.owner == NodeOwner::Obj(uid) && n.path == v.path && n.ui.is_none() && !n.freed && n.props.contains_key("animation")));
            if !set {
                v.anim = p.anim.clone();
                v.frame = p.frame;
            }
        }
        out
    }

    /// The nodes of a fighter's scene that scripts showed or changed (marks, slashes, auras;
    /// sprites and `CPUParticles2D` emitters),
    /// apart from its own sprite, positioned from its feet (game pixels).
    pub fn fighter_nodes(&self, i: usize) -> Vec<NodeView> {
        let Some(sc) = &self.scripts else { return vec![] };
        let ch = &self.chars[i].file.character;
        let Some(scene) = ch.scene.as_ref().and_then(|sp| ch.scenes.iter().find(|s| &s.path == sp)) else { return vec![] };
        let main = main_sprite(scene);
        let owner = NodeOwner::Fighter(i);
        let touched = |path: &str| sc.t.nodes.iter().any(|n| n.owner == owner && n.path == path && n.ui.is_none() && !n.freed && !n.props.is_empty());
        self.scene_nodes(owner, scene, |path, _| Some(path) != main && touched(path))
    }

    /// How scripts left fighter `i`'s own sprite; None while they have not touched it.
    pub fn fighter_sprite(&self, i: usize) -> Option<SpriteLook> {
        let sc = self.scripts.as_ref()?;
        let ch = &self.chars[i].file.character;
        let scene = ch.scene.as_ref().and_then(|sp| ch.scenes.iter().find(|s| &s.path == sp))?;
        let main = main_sprite(scene)?;
        let owner = NodeOwner::Fighter(i);
        let rec_of = |path: &str| sc.t.nodes.iter().find(|n| n.owner == owner && n.path == path && n.ui.is_none() && !n.freed);
        let parents_touched = main.split('/').count() > 1 && rec_of("Flip").is_some_and(|r| !r.props.is_empty());
        let rec = rec_of(main).filter(|r| !r.props.is_empty());
        if rec.is_none() && !parents_touched {
            return None;
        }
        let def = scene.nodes.iter().find(|n| n.path == main)?;
        let set = |k: &str| rec.and_then(|r| r.props.get(k).cloned());
        let scene_v2 = |k: &str| match def.props.iter().find(|(n, _)| n == k).map(|(_, v)| var_value(v)) {
            Some(Value::Vec2(x, y)) => (x as f32, y as f32),
            _ => (0.0, 0.0),
        };
        let moved = |k: &str| match set(k) {
            Some(Value::Vec2(x, y)) => {
                let d = scene_v2(k);
                (x as f32 - d.0, y as f32 - d.1)
            }
            _ => (0.0, 0.0),
        };
        let (dp, doff) = (moved("position"), moved("offset"));
        let mut m = color_of(&set("modulate").unwrap_or(Value::Color(1.0, 1.0, 1.0, 1.0)));
        for (a, b) in m.iter_mut().zip(color_of(&set("self_modulate").unwrap_or(Value::Color(1.0, 1.0, 1.0, 1.0)))) {
            *a *= b;
        }
        let flip_visible = rec_of("Flip").and_then(|r| r.props.get("visible").cloned()).is_none_or(|v| v.truthy());
        Some(SpriteLook {
            visible: set("visible").is_none_or(|v| v.truthy()) && flip_visible,
            anim: set("animation").map(|v| v.to_text()),
            frame: set("frame").and_then(|v| v.as_i64()).map(|f| f.max(0) as u32),
            modulate: m,
            rotation: rotation_of(&set),
            shift: (dp.0 + doff.0, dp.1 + doff.1),
            scale: match set("scale") {
                Some(Value::Vec2(x, y)) => (x as f32, y as f32),
                _ => (1.0, 1.0),
            },
            flip_h: set("flip_h").is_some_and(|v| v.truthy()),
        })
    }

    /// A fighter's sprite shows the frame its state picks unless a script picks one: what a
    /// script picked lasts the tick it was set in.
    pub(crate) fn clear_sprite_picks(&mut self) {
        let Some(sc) = self.scripts.as_mut() else { return };
        for n in sc.t.nodes.iter_mut().filter(|n| matches!(n.owner, NodeOwner::Fighter(_)) && (n.path == "Flip/Sprite" || n.path == "Sprite")) {
            n.props.remove("frame");
            n.props.remove("animation");
        }
    }

    /// The visible sprite nodes of `scene` that `keep` accepts, with their parents' positions,
    /// visibility and z added up, relative to the owner (game pixels).
    fn scene_nodes(&self, owner: NodeOwner, scene: &SceneDef, keep: impl Fn(&str, &NodeDef) -> bool) -> Vec<NodeView> {
        let Some(sc) = &self.scripts else { return vec![] };
        let rec_of = |path: &str| sc.t.nodes.iter().find(|n| n.owner == owner && n.path == path && n.ui.is_none() && !n.freed);
        let visible = |path: &str| -> bool {
            let def = scene.nodes.iter().find(|n| n.path == path);
            rec_of(path).and_then(|r| r.props.get("visible").map(|v| v.truthy())).unwrap_or_else(|| def.and_then(|d| d.props.iter().find(|(k, _)| k == "visible")).is_none_or(|(_, v)| var_value(v).truthy()))
        };
        // z_index adds up along the parents (the root's included)
        let zi = |path: &str| -> i32 {
            let r = rec_of(path).and_then(|r| r.props.get("z_index").cloned());
            let d = scene.nodes.iter().find(|n| n.path == path).and_then(|d| d.props.iter().find(|(k, _)| k == "z_index")).map(|(_, v)| var_value(v));
            r.or(d).and_then(|v| v.as_i64()).unwrap_or(0) as i32
        };
        let mut out = Vec::new();
        for def in scene.nodes.iter().filter(|d| matches!(d.ty.as_str(), "AnimatedSprite" | "Sprite" | "CPUParticles2D") && !d.path.starts_with("StateMachine") && keep(&d.path, d)) {
            // every ancestor must be visible too
            let mut ok = visible(&def.path);
            let mut pos = (0.0f32, 0.0f32);
            let mut z = zi(".");
            let parts: Vec<&str> = def.path.split('/').collect();
            for k in 1..parts.len() {
                let anc = parts[..k].join("/");
                ok &= visible(&anc);
                z += zi(&anc);
                if let Some(ad) = scene.nodes.iter().find(|n| n.path == anc) {
                    let r = rec_of(&anc);
                    if let Some(Value::Vec2(x, y)) = r.and_then(|r| r.props.get("position").cloned()).or_else(|| ad.props.iter().find(|(n, _)| n == "position").map(|(_, v)| var_value(v))) {
                        pos.0 += x as f32;
                        pos.1 += y as f32;
                    }
                }
            }
            if !ok {
                continue;
            }
            let mut v = self.view(&sc.t, owner, &def.path, scene, def, rec_of(&def.path));
            v.pos = (v.pos.0 + pos.0, v.pos.1 + pos.1);
            v.z += z;
            v.flipped = def.path.starts_with("Flip/");
            out.push(v);
        }
        out
    }
}

fn sc_has(ctx: &Ctx, inst: &InstRef, name: &str) -> bool {
    let Some(loader) = ctx.t.loader.clone() else { return false };
    Vm::shared(loader).has_script_method(inst, name)
}
