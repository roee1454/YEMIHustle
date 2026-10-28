//! Reads a character's scenes and scripts into structured JSON. A port of
//! `tools/extract_character.py`; the output is identical to the original's.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::script::{self, enum_members, ScriptInfo, ENUMS, PROP_ENUM};
use crate::tscn::{self, Ext, Node, Scene};

/// An extraction run over one decompiled game folder.
pub struct Extractor {
    root: PathBuf,
    class_paths: HashMap<String, String>,
    scripts: RefCell<HashMap<String, Rc<ScriptInfo>>>,
}

type Res<T> = Result<T, String>;

fn rel_slash(p: &Path, root: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

impl Extractor {
    pub fn new(root: &Path) -> Self {
        let mut class_paths = HashMap::new();
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    if !p.to_string_lossy().contains("/.import") {
                        stack.push(p);
                    }
                } else if p.extension().is_some_and(|x| x == "gd") {
                    files.push(p);
                }
            }
        }
        files.sort();
        let re = Regex::new(r"(?m)^class_name\s+(\w+)").expect("valid");
        for p in files {
            let Ok(text) = tscn::read_text(&p) else { continue };
            let head: String = text.chars().take(4000).collect();
            if let Some(c) = re.captures(&head) {
                class_paths.insert(c[1].to_string(), rel_slash(&p, root));
            }
        }
        Extractor { root: root.to_path_buf(), class_paths, scripts: RefCell::new(HashMap::new()) }
    }

    fn full(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn read_scene(&self, rel: &str) -> Res<Scene> {
        let text = tscn::read_text(&self.full(rel)).map_err(|e| format!("{rel}: {e}"))?;
        Ok(tscn::parse(&text))
    }

    pub fn read(&self, rel: &str) -> String {
        tscn::read_text(&self.full(rel)).unwrap_or_default()
    }

    pub fn is_file(&self, rel: &str) -> bool {
        self.full(rel).is_file()
    }

    pub fn script_info(&self, rel: &str) -> Rc<ScriptInfo> {
        if let Some(i) = self.scripts.borrow().get(rel) {
            return i.clone();
        }
        let info = Rc::new(match tscn::read_text(&self.full(rel)) {
            Ok(t) if self.full(rel).is_file() => script::parse(&t),
            _ => ScriptInfo::default(),
        });
        self.scripts.borrow_mut().insert(rel.to_string(), info.clone());
        info
    }

    fn resolve_extends(&self, ext: Option<&str>) -> Option<String> {
        let ext = ext?;
        if ext.starts_with("res://") {
            return Some(ext.replace("res://", ""));
        }
        self.class_paths.get(ext).cloned()
    }

    pub fn class_chain(&self, rel: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = Some(rel.to_string());
        while let Some(c) = cur {
            if !seen.insert(c.clone()) {
                break;
            }
            let info = self.script_info(&c);
            chain.push(c);
            cur = self.resolve_extends(info.extends.as_deref());
        }
        chain
    }

    /// Merged export defaults, base class first.
    pub fn chain_defaults(&self, rel: &str) -> Map<String, Value> {
        let mut merged = Map::new();
        for p in self.class_chain(rel).iter().rev() {
            for (k, v) in &self.script_info(p).exports {
                merged.insert(k.clone(), v.clone());
            }
        }
        merged
    }

    // ------------------------------------------------------------------ scene walk
    /// Path -> merged node, with an instanced base scene merged underneath.
    pub fn scene_nodes(&self, scene: &Scene) -> Res<(Vec<(String, Node)>, Option<String>)> {
        let mut base_path = None;
        for n in &scene.nodes {
            if n.parent.is_none() {
                if let Some(inst) = &n.instance {
                    let id: u32 = Regex::new(r"ExtResource\(\s*(\d+)\s*\)").expect("valid").captures(inst).and_then(|c| c[1].parse().ok()).unwrap_or(0);
                    base_path = scene.ext.get(&id).map(|e| e.0.clone());
                }
            }
        }
        let mut base: Vec<(String, Node)> = Vec::new();
        if let Some(bp) = &base_path {
            let base_scene = self.read_scene(bp)?;
            base = self.scene_nodes(&base_scene)?.0;
            for (_, bn) in base.iter_mut() {
                bn.ext = base_scene.ext.clone();
            }
        }
        let mut nodes: Vec<(String, Node)> = Vec::new();
        let put = |nodes: &mut Vec<(String, Node)>, path: String, n: Node| match nodes.iter_mut().find(|(p, _)| *p == path) {
            Some(slot) => slot.1 = n,
            None => nodes.push((path, n)),
        };
        for n in &scene.nodes {
            let path = match n.parent.as_deref() {
                None => ".".to_string(),
                Some(".") => n.name.clone(),
                Some(p) => format!("{p}/{}", n.name),
            };
            let mut own = n.clone();
            own.ext = scene.ext.clone();
            let mut keys: Vec<String> = n.props.keys().cloned().collect();
            keys.sort();
            if let Some((_, b)) = base.iter().find(|(p, _)| *p == path) {
                let mut merged = b.clone();
                for (k, v) in &n.props {
                    merged.props.insert(k.clone(), v.clone());
                }
                if n.props.contains_key("script") {
                    merged.script_ext = Some(scene.ext.clone());
                }
                merged.ty = n.ty.clone().or_else(|| b.ty.clone());
                merged.origin = "base_override";
                merged.overrides = keys;
                put(&mut nodes, path, merged);
            } else {
                own.origin = "character";
                own.overrides = keys;
                put(&mut nodes, path, own);
            }
        }
        for (path, bn) in &base {
            if !nodes.iter().any(|(p, _)| p == path) {
                let mut b = bn.clone();
                b.origin = "base";
                b.overrides = vec![];
                nodes.push((path.clone(), b));
            }
        }
        Ok((nodes, base_path))
    }
}

impl Extractor {
    /// An object scene (a projectile, a summon, a domain...): its root script and properties, and
    /// its states (the `StateMachine` children) with their scripts, values and hitboxes.
    pub fn extract_object(&self, rel: &str, cache: &mut HashMap<String, Map<String, Value>>) -> Value {
        let psc = match self.read_scene(rel) {
            Ok(s) => s,
            Err(e) => return json!({"scene": rel, "error": e}),
        };
        let (pn, pbase) = match self.scene_nodes(&psc) {
            Ok(x) => x,
            Err(e) => return json!({"scene": rel, "error": e}),
        };
        let pfind = |p: &str| pn.iter().find(|(q, _)| q == p).map(|(_, n)| n);
        let proot = pfind(".");
        let rs = proot.and_then(|r| node_script(r, &psc.ext));
        let mut pstates = Vec::new();
        for (path, n) in &pn {
            if path.starts_with("StateMachine/") && path.matches('/').count() == 1 {
                let sscript = node_script(n, &psc.ext);
                let mut kids: Vec<&Node> = pn.iter().filter(|(p2, _)| p2.starts_with(&format!("{path}/")) && p2.matches('/').count() == 2).map(|(_, x)| x).collect();
                kids.sort_by_key(|x| index_of(x));
                let boxes: Vec<Value> = kids
                    .into_iter()
                    .filter(|c| node_script(c, &psc.ext).unwrap_or_default().contains("Hitbox"))
                    .map(|c| self.hitbox_record(c, cache))
                    .collect();
                pstates.push(json!({
                    "name": n.name,
                    "script": sscript,
                    "explicit": decode_enums(&clean_props(&n.props)),
                    "hitboxes": boxes,
                }));
            }
        }
        json!({
            "scene": rel,
            "inherits": pbase,
            "root_script": rs,
            "root_properties": proot.map(|r| Value::Object(decode_enums(&clean_props(&r.props)))).unwrap_or(json!({})),
            "states": pstates,
        })
    }

    /// Scenes the scripts and scenes under `dir` refer to (`preload("res://....tscn")`, exported
    /// scenes): the objects a fighter's scripts may spawn.
    pub fn referenced_scenes(&self, dir: &str) -> Vec<String> {
        let re = Regex::new(r#"res://([^"]+?\.tscn)"#).expect("valid");
        let mut out: Vec<String> = Vec::new();
        let mut stack = vec![self.full(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "gd" || x == "tscn") {
                    let text = std::fs::read_to_string(&p).unwrap_or_default();
                    for c in re.captures_iter(&text) {
                        let rel = c[1].to_string();
                        if !out.contains(&rel) && self.full(&rel).is_file() {
                            out.push(rel);
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// A scene's nodes with their values, and for animated sprites their animations.
    pub fn scene_def(&self, rel: &str) -> Option<Value> {
        let sc = self.read_scene(rel).ok()?;
        let (nodes, _) = self.scene_nodes(&sc).ok()?;
        let list: Vec<Value> = nodes
            .iter()
            .map(|(path, n)| {
                // hitboxes and other state children only need to be found, not described
                let under_state = path.starts_with("StateMachine/") && path.matches('/').count() >= 2;
                // (z_index too: it orders the sprites)
                let props: Map<String, Value> = if under_state { Map::new() } else { decode_enums(&n.props.iter().filter(|(k, _)| !matches!(k.as_str(), "script" | "__meta__")).map(|(k, v)| (k.clone(), v.clone())).collect()) };
                let anims: Vec<Value> = if n.props.contains_key("frames") {
                    self.node_anims(rel, path, 0).into_iter().map(|a| json!({"name": a.name, "frames": a.frames, "fps": a.fps, "looped": a.looped})).collect()
                } else {
                    vec![]
                };
                json!({"path": path, "ty": n.ty.clone().unwrap_or_default(), "props": props, "anims": anims})
            })
            .collect();
        Some(json!({"path": rel, "nodes": list}))
    }

    /// The animations of the `frames` a node gets in scene `rel` (or in the scene it inherits).
    fn node_anims(&self, rel: &str, path: &str, depth: usize) -> Vec<tscn::Anim> {
        let Ok(sc) = self.read_scene(rel) else { return vec![] };
        let own = sc.nodes.iter().find(|n| {
            let p = match n.parent.as_deref() {
                None => ".".to_string(),
                Some(".") => n.name.clone(),
                Some(p) => format!("{p}/{}", n.name),
            };
            p == path && n.props.contains_key("frames")
        });
        if let Some(n) = own {
            let v = &n.props["frames"];
            if let Some(id) = v.get("sub_id").and_then(Value::as_u64) {
                return sc.subs.get(&(id as u32)).map(|(_, t)| tscn::sprite_anims(t)).unwrap_or_default();
            }
            if let Some(r) = v.get("res").and_then(Value::as_str) {
                return tscn::sprite_anims(&self.read(r));
            }
            return vec![];
        }
        // inherited: the base scene gives the frames
        let base = sc.nodes.iter().find(|n| n.parent.is_none()).and_then(|r| r.instance.as_ref()).and_then(|i| {
            let id: u32 = Regex::new(r"ExtResource\(\s*(\d+)\s*\)").expect("valid").captures(i).and_then(|c| c[1].parse().ok())?;
            sc.ext.get(&id).map(|e| e.0.clone())
        });
        match base {
            Some(b) if depth < 6 => self.node_anims(&b, path, depth + 1),
            _ => vec![],
        }
    }
}

fn node_script(n: &Node, scene_ext: &Ext) -> Option<String> {
    let s = n.props.get("script")?;
    if let Some(r) = s.get("res").and_then(Value::as_str) {
        return Some(r.to_string());
    }
    let id = s.get("res_id").and_then(Value::as_u64)? as u32;
    let ext: &Ext = match (&n.script_ext, &n.ext) {
        (Some(e), _) => e,
        (None, e) => e,
    };
    let _ = scene_ext;
    ext.get(&id).map(|e| e.0.clone())
}

fn clean_props(props: &Map<String, Value>) -> Map<String, Value> {
    props.iter().filter(|(k, _)| !matches!(k.as_str(), "script" | "__meta__" | "z_index")).map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn decode_enums(props: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in props {
        let decoded = PROP_ENUM.iter().find(|(p, _)| p == k).and_then(|(_, en)| {
            let i = v.as_i64()?;
            enum_members(en).get(usize::try_from(i).ok()?).map(|m| Value::String(m.to_string()))
        });
        out.insert(k.clone(), decoded.unwrap_or_else(|| v.clone()));
    }
    out
}

fn multiline_list(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::Array(s.split('\n').filter(|x| !x.trim().is_empty()).map(|x| Value::String(x.to_string())).collect()),
        other => other.clone(),
    }
}

fn pick(map: &Map<String, Value>, fields: &[&str]) -> Map<String, Value> {
    fields.iter().filter_map(|k| map.get(*k).map(|v| (k.to_string(), v.clone()))).collect()
}

fn index_of(n: &Node) -> i64 {
    n.index.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Python numbers: ints stay ints, anything else is a float.
fn num_add(a: &Value, b: &Value) -> Value {
    match (a.as_i64(), b.as_i64()) {
        (Some(x), Some(y)) => json!(x + y),
        _ => json!(a.as_f64().unwrap_or(0.0) + b.as_f64().unwrap_or(0.0)),
    }
}

fn num_lt(a: &Value, b: &Value) -> bool {
    a.as_f64().unwrap_or(0.0) < b.as_f64().unwrap_or(0.0)
}

fn max_of(vals: &[Value]) -> Value {
    let mut best = vals[0].clone();
    for v in &vals[1..] {
        if num_lt(&best, v) {
            best = v.clone();
        }
    }
    best
}

fn min_of(vals: &[Value]) -> Value {
    let mut best = vals[0].clone();
    for v in &vals[1..] {
        if num_lt(v, &best) {
            best = v.clone();
        }
    }
    best
}

const HITBOX_FIELDS: &[&str] = &[
    "damage", "damage_in_combo", "minimum_damage", "start_tick", "active_ticks", "always_on",
    "hitstun_ticks", "combo_hitstun_ticks", "hitlag_ticks", "victim_hitlag", "combo_victim_hitlag",
    "knockback", "dir_x", "dir_y", "pushback_x", "hit_height", "hitbox_type",
    "grounded_hit_state", "aerial_hit_state", "knockdown", "hard_knockdown", "ground_bounce",
    "wall_slam", "guard_break", "plus_frames", "parriable", "block_cancel_allowed",
    "damage_proration", "scale_combo", "combo_scaling_amount", "cancellable", "can_clash",
    "beats_grab", "hits_otg", "hits_vs_aerial", "hits_vs_grounded", "hits_projectiles",
    "looping", "loop_active_ticks", "loop_inactive_ticks", "followup_state", "group", "priority",
    "x", "y", "width", "height", "block_pushback_modifier", "chip_damage_modifier",
    "meter_gain_modifier", "di_modifier", "sdi_modifier", "ignore_armor",
];

const STATE_FIELDS: &[&str] = &[
    "title", "type", "air_type", "anim_length", "sprite_animation", "iasa_at", "iasa_on_hit",
    "interrupt_frames", "fallback_state", "combo_only", "neutral_only", "tick_priority",
    "can_be_counterhit", "beats_backdash", "initiative_effect", "super_level_",
    "super_freeze_ticks_", "uses_air_movement", "land_cancel", "landing_recovery",
    "apply_forces", "apply_fric", "apply_grav", "reset_momentum", "force_tick",
    "enter_force_speed", "force_speed", "projectile_tick", "projectile_pos_x",
    "projectile_pos_y", "endless", "loop_animation", "disable_at_end", "extra_parry_hitlag", "throw_techable", "interruptible_on_opponent_turn",
    "end_feint", "can_feint_if_possible", "reversible", "burstable", "show_in_menu",
    "selectable", "allowed_stances_string",
];

impl Extractor {
    fn hitbox_record(&self, n: &Node, cache: &mut HashMap<String, Map<String, Value>>) -> Value {
        let script = node_script(n, &n.ext).unwrap_or_else(|| "mechanics/Hitbox.gd".to_string());
        let dflt = cache.entry(script.clone()).or_insert_with(|| self.chain_defaults(&script)).clone();
        let props = decode_enums(&clean_props(&n.props));
        let mut resolved = decode_enums(&dflt);
        for (k, v) in &props {
            resolved.insert(k.clone(), v.clone());
        }
        let key = pick(&resolved, HITBOX_FIELDS);
        json!({
            "name": n.name,
            "kind": if script.contains("ThrowBox") { "throwbox" } else { "hitbox" },
            "script": script,
            "key": key,
            "explicit": props,
        })
    }

    fn ui_params(&self, scene_path: Option<&str>) -> Value {
        let Some(sp) = scene_path else { return Value::Null };
        if !self.is_file(sp) {
            return Value::Null;
        }
        Value::Array(self.ui_params_in(sp, 0))
    }

    /// The option widgets of a UI scene: its root's children, after those of the scene it
    /// inherits (a child of the same name there is overridden, not added).
    fn ui_params_in(&self, sp: &str, depth: usize) -> Vec<Value> {
        let Ok(sc) = self.read_scene(sp) else { return Vec::new() };
        let inst_re = Regex::new(r"ExtResource\(\s*(\d+)\s*\)").expect("valid");
        let ext_of = |i: &str| inst_re.captures(i).and_then(|c| sc.ext.get(&c[1].parse::<u32>().unwrap_or(0)).map(|e| e.0.clone()));
        let mut params: Vec<Value> = Vec::new();
        if let Some(base) = sc.nodes.first().and_then(|r| r.instance.as_deref()).and_then(ext_of) {
            if depth < 8 && self.is_file(&base) {
                params = self.ui_params_in(&base, depth + 1);
            }
        }
        for n in &sc.nodes {
            if n.parent.as_deref() != Some(".") {
                continue;
            }
            let keep: Map<String, Value> = clean_props(&n.props)
                .into_iter()
                .filter(|(k, _)| !["margin_", "anchor_", "rect_", "size_flags", "mouse_", "theme", "focus"].iter().any(|p| k.starts_with(p)))
                .collect();
            if n.ty.is_none() && n.instance.is_none() {
                if let Some(old) = params.iter_mut().find(|p| p["name"].as_str() == Some(n.name.as_str())) {
                    if let Some(props) = old["props"].as_object_mut() {
                        props.extend(keep);
                    }
                    continue;
                }
            }
            let inst = n.instance.as_deref().and_then(ext_of);
            params.push(json!({"name": n.name, "type": n.ty, "instance": inst, "props": keep}));
        }
        params
    }
}

fn derive(resolved: &Map<String, Value>, hitboxes: &[Value], anim_length: i64) -> Value {
    let mut d = Map::new();
    d.insert("anim_length".into(), json!(anim_length));
    let mut iasa_tick: Option<i64> = None;
    if let Some(iasa) = resolved.get("iasa_at").map(|v| v.as_i64()).unwrap_or(Some(-1)) {
        let t = if iasa < 0 { anim_length + iasa } else { iasa };
        d.insert("iasa_tick".into(), json!(t));
        iasa_tick = Some(t);
    }
    if resolved.get("endless").is_some_and(|v| v.as_bool().unwrap_or(v.as_i64().unwrap_or(0) != 0)) {
        d.insert("endless".into(), json!(true));
        d.insert("note".into(), json!("endless state: runs until script ends it (e.g. landing); anim_length/iasa_tick are not the real duration"));
    }
    let kind = |h: &Value| h["kind"].as_str().unwrap_or("").to_string();
    let get = |h: &Value, k: &str| h["key"].get(k).cloned().unwrap_or(json!(0));
    let real: Vec<&Value> = hitboxes.iter().filter(|h| kind(h) == "hitbox").collect();
    let throws: Vec<&Value> = hitboxes.iter().filter(|h| kind(h) == "throwbox").collect();
    if !throws.is_empty() {
        d.insert("throwbox_count".into(), json!(throws.len()));
        d.insert("throw_start_ticks".into(), Value::Array(throws.iter().map(|h| get(h, "start_tick")).collect()));
    }
    if !real.is_empty() {
        let starts: Vec<Value> = real.iter().map(|h| get(h, "start_tick")).collect();
        let ends: Vec<Value> = real.iter().map(|h| num_add(&get(h, "start_tick"), &get(h, "active_ticks"))).collect();
        d.insert("first_hitbox_start_tick".into(), min_of(&starts));
        let last_end = max_of(&ends);
        d.insert("last_hitbox_end_tick".into(), last_end.clone());
        if let Some(t) = iasa_tick {
            d.insert("ticks_from_last_active_to_iasa".into(), json!(t - last_end.as_i64().unwrap_or(0)));
        }
        d.insert("hitbox_count".into(), json!(real.len()));
        d.insert("max_damage".into(), max_of(&real.iter().map(|h| get(h, "damage")).collect::<Vec<_>>()));
    }
    Value::Object(d)
}

/// The extracted character: scene, states with resolved values, projectiles.
pub fn extract(root: &Path, char_dir: &str, scene_file: &str) -> Res<Value> {
    let ex = Extractor::new(root);
    extract_with(&ex, char_dir, scene_file)
}

pub fn extract_with(ex: &Extractor, char_dir: &str, scene_file: &str) -> Res<Value> {
    let scene_rel = format!("{char_dir}/{scene_file}");
    let scene = ex.read_scene(&scene_rel)?;
    let (nodes, base_path) = ex.scene_nodes(&scene)?;
    let scene_ext = scene.ext.clone();
    let mut cache: HashMap<String, Map<String, Value>> = HashMap::new();
    let find = |p: &str| nodes.iter().find(|(q, _)| q == p).map(|(_, n)| n);

    let root = find(".").ok_or("the scene has no root node")?;
    let root_script = node_script(root, &scene_ext);
    let character = json!({
        "scene": scene_rel,
        "inherits": base_path,
        "root_script": root_script,
        "root_script_consts": root_script.as_ref().map(|s| Value::Object(ex.script_info(s).consts.clone())).unwrap_or(json!({})),
        "root_properties": decode_enums(&clean_props(&root.props)),
        "root_class_defaults": root_script.as_ref().map(|s| Value::Object(ex.chain_defaults(s))).unwrap_or(json!({})),
    });

    // states and their children
    let mut children: Vec<(String, Vec<&Node>)> = Vec::new();
    for (path, n) in &nodes {
        if path.starts_with("StateMachine/") && path.matches('/').count() >= 2 {
            let parent = path.rsplit_once('/').map(|x| x.0).unwrap_or("").to_string();
            match children.iter_mut().find(|(p, _)| *p == parent) {
                Some(slot) => slot.1.push(n),
                None => children.push((parent, vec![n])),
            }
        }
    }
    let mut ordered: Vec<&(String, Node)> = nodes.iter().collect();
    ordered.sort_by_key(|(_, n)| index_of(n));

    let mut states = Vec::new();
    for (path, n) in ordered {
        if !path.starts_with("StateMachine/") || path.matches('/').count() != 1 {
            continue;
        }
        let Some(script) = node_script(n, &scene_ext) else { continue };
        let dflt = ex.chain_defaults(&script);
        let explicit = decode_enums(&clean_props(&n.props));
        let mut resolved = decode_enums(&dflt);
        for (k, v) in &explicit {
            resolved.insert(k.clone(), v.clone());
        }
        let anim_length = resolved.get("anim_length").cloned().unwrap_or(json!(1));
        let (mut hitboxes, mut hurtboxes, mut commands, mut others) = (vec![], vec![], vec![], vec![]);
        let mut kids: Vec<&Node> = children.iter().find(|(p, _)| p == path).map(|(_, v)| v.clone()).unwrap_or_default();
        kids.sort_by_key(|c| index_of(c));
        for c in kids {
            let cs = node_script(c, &scene_ext).unwrap_or_default();
            if cs.ends_with("mechanics/Hitbox.gd") || cs.ends_with("ThrowBox.gd") || cs.contains("Hitbox") {
                hitboxes.push(ex.hitbox_record(c, &mut cache));
            } else if cs.contains("HurtboxState") || cs.contains("LimbHurtbox") {
                hurtboxes.push(json!({"name": c.name, "script": cs, "props": decode_enums(&clean_props(&c.props))}));
            } else if c.name.starts_with("HostCommand") || cs.contains("HostCommand") {
                commands.push(json!({"name": c.name, "script": cs, "props": clean_props(&c.props)}));
            } else {
                others.push(json!({"name": c.name, "script": cs, "props": clean_props(&c.props)}));
            }
        }
        let ui_path = resolved.get("data_ui_scene").and_then(|u| u.get("res")).and_then(Value::as_str).map(str::to_string);
        let sinfo = ex.script_info(&script);
        let chain = ex.class_chain(&script);
        let first_two = |f: fn(&ScriptInfo) -> bool| chain.iter().take(2).any(|p| f(&ex.script_info(p)));
        let mut key = pick(&resolved, STATE_FIELDS);
        for lk in ["interrupt_from_string", "interrupt_into_string", "hit_cancel_into_string", "interrupt_exceptions_string", "hit_cancel_exceptions_string"] {
            if let Some(v) = resolved.get(lk) {
                key.insert(lk.replace("_string", ""), multiline_list(v));
            }
        }
        if let Some(ps) = resolved.get("projectile_scene").filter(|p| p.is_object()) {
            key.insert("projectile_scene".into(), ps.get("res").cloned().unwrap_or(Value::Null));
        }
        let derived = derive(&resolved, &hitboxes, anim_length.as_i64().unwrap_or(1));
        states.push(json!({
            "name": n.name,
            "origin": n.origin,
            "script": script,
            "class_chain": chain,
            "key": key,
            "explicit_overrides": explicit,
            "hitboxes": hitboxes,
            "hurtbox_states": hurtboxes,
            "host_command_nodes": commands,
            "other_children": others,
            "player_parameters": ex.ui_params(ui_path.as_deref()),
            "ui_scene": ui_path,
            "script_info": {
                "sets_anim_length_in_code": first_two(|s| s.sets_anim_length),
                "sets_iasa_in_code": first_two(|s| s.sets_iasa),
                "sets_hitbox_fields_in_code": first_two(|s| s.sets_hitbox_fields),
                "applies_forces_in_code": first_two(|s| s.applies_forces),
                "consts": sinfo.consts,
                "frame_hooks": sinfo.frame_hooks,
                "funcs": sinfo.funcs.iter().filter(|f| f.starts_with('_') && !f.starts_with("_frame_")).collect::<Vec<_>>(),
            },
            "derived": derived,
        }));
    }

    // projectiles
    let mut projectiles = Vec::new();
    let pdir = ex.full(&format!("{char_dir}/projectiles"));
    let mut names: Vec<String> = std::fs::read_dir(&pdir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for fname in names.into_iter().filter(|f| f.ends_with(".tscn")) {
        let rel = format!("{char_dir}/projectiles/{fname}");
        projectiles.push(ex.extract_object(&rel, &mut cache));
    }

    let enums: Map<String, Value> = ENUMS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    Ok(json!({
        "meta": {
            "source": "Decompiled Your Only Move Is HUSTLE (Godot 3.5.1) - private reference data",
            "character_scene": character["scene"],
            "notes": [
                "'key' holds resolved values (script defaults merged with scene overrides); 'explicit_overrides' holds only what the scene sets.",
                "Fixed-point fields (knockback, dir_x, dir_y, forces) are strings in the original; kept as strings.",
                "Ticks are the game's fixed simulation ticks. start_tick/active_ticks are as authored; the engine activates a hitbox when the state's tick counter reaches start_tick-1 (see notes/01-02), so treat start_tick as the first hit-capable tick approximately.",
                "'iasa_tick' resolves negative iasa_at as anim_length + iasa_at, matching CharState.gd.",
                "Move logic that lives in GDScript (forces, spawns, conditional branches) is NOT in this file; see 'script_info' and the script path for what exists.",
                "derived.* values are computed here, not read from the game.",
            ],
        },
        "enums": enums,
        "character": character,
        "states": states,
        "projectiles": projectiles,
    }))
}
