//! The fighters' own sprites, read at runtime from the original game's character scenes in the
//! private library. Each character scene holds one `AnimatedSprite` (`Flip/Sprite`) whose
//! animations list one image per game tick; a state plays the animation named after it, one
//! frame per tick. Here the engine's action picks the animation and the tick picks the frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use sim::{Act, Fighter, World};

use crate::godot::{self, prop, V};

/// The first of `dirs` that holds `rel` (mods' folders come before the game's).
pub fn find_in(dirs: &[PathBuf], rel: &str) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(rel)).find(|p| p.exists())
}

/// Where each official character's main scene lives inside the game folder.
fn scene_of(id: &str) -> Option<&'static str> {
    Some(match id {
        "ninja" => "characters/stickman/NinjaGuy.tscn",
        "robo" => "characters/robo/Robot.tscn",
        "wizard" => "characters/wizard/Wizard.tscn",
        "mutant" => "characters/mutant/Mutant.tscn",
        "cowboy" => "characters/swordandgun/SwordGuy.tscn",
        _ => return None,
    })
}

/// The sprite offset of a base scene, which scenes without their own inherit.
fn base_offset(dirs: &[PathBuf], base: &str, fallback: Vec2) -> Vec2 {
    let doc = find_in(dirs, base).and_then(|p| std::fs::read_to_string(p).ok()).map(|t| godot::parse(&t)).unwrap_or_default();
    doc.nodes
        .iter()
        .find(|n| n.name == "Sprite" && n.parent.as_deref() == Some("Flip"))
        .and_then(|n| prop(&n.props, "offset").and_then(V::vec2))
        .map(|(x, y)| Vec2::new(x, y))
        .unwrap_or(fallback)
}

/// An extra `AnimatedSprite` under `Flip` that scripts show during certain moves.
pub struct Overlay {
    frames: Vec<String>,
    pub pos: Vec2,
    pub offset: Vec2,
    pub z: f32,
}

pub struct CharArt {
    /// Animation name -> image paths, one per tick.
    anims: HashMap<String, Vec<String>>,
    /// The converter's `snake_case` animation ids -> original names.
    by_snake: HashMap<String, String>,
    /// The sprite's offset from the fighter's feet, in game pixels (y down).
    pub offset: Vec2,
    pub overlays: HashMap<String, Overlay>,
}

/// A `frames` value: an inline sub-resource or a `.tres` file of its own (mods usually ship the
/// latter). Returns the document its image references resolve against and its properties.
fn resolve_frames(dirs: &[PathBuf], doc: &godot::Doc, v: &V) -> Option<(godot::Doc, godot::Props)> {
    if let Some((_, props)) = doc.sub_res(v) {
        return Some((doc.clone(), props.clone()));
    }
    let path = doc.ext_path(v)?;
    let text = std::fs::read_to_string(find_in(dirs, path)?).ok()?;
    let d = godot::parse(&text);
    let props = d.resource.clone();
    Some((d, props))
}

/// One frame's image: a file, or a region of a sprite sheet (`AtlasTexture`), written as
/// `sheet.png#atlas:x,y,w,h` for the image loader to cut out.
fn frame_image(doc: &godot::Doc, v: &V) -> Option<String> {
    if let Some(p) = doc.ext_path(v) {
        return Some(p.to_string());
    }
    let (ty, props) = doc.sub_res(v)?;
    if ty != "AtlasTexture" {
        return None;
    }
    let sheet = doc.ext_path(prop(props, "atlas")?)?;
    let r = prop(props, "region")?.numbers();
    let [x, y, w, h] = r[..] else { return None };
    Some(format!("{sheet}#atlas:{x},{y},{w},{h}"))
}

fn animations(doc: &godot::Doc, frames: &godot::Props) -> (HashMap<String, Vec<String>>, HashMap<String, String>) {
    let mut anims = HashMap::new();
    let mut by_snake = HashMap::new();
    for a in prop(frames, "animations").map(|a| a.items()).unwrap_or_default() {
        let Some(name) = a.get("name").and_then(V::str) else { continue };
        let paths: Vec<String> = a.get("frames").map(|f| f.items().iter().filter_map(|v| frame_image(doc, v)).collect()).unwrap_or_default();
        if paths.is_empty() {
            continue;
        }
        by_snake.entry(convert::ron::snake(name)).or_insert_with(|| name.to_string());
        anims.insert(name.to_string(), paths);
    }
    (anims, by_snake)
}

impl CharArt {
    fn load(dirs: &[PathBuf], scene: &str) -> Option<Self> {
        Self::load_scene(dirs, scene, "characters/BaseChar.tscn", Vec2::new(0.0, -18.0), 0)
    }

    /// The sprite of a scene, following the scenes it inherits until one sets `frames`.
    fn load_scene(dirs: &[PathBuf], scene: &str, base: &str, base_default: Vec2, depth: u32) -> Option<Self> {
        let text = std::fs::read_to_string(find_in(dirs, scene)?).ok()?;
        let doc = godot::parse(&text);
        let mut overlays = HashMap::new();
        for n in doc.nodes.iter().filter(|n| n.ty.as_deref() == Some("AnimatedSprite") && n.parent.as_deref() == Some("Flip")) {
            let Some((fdoc, f)) = prop(&n.props, "frames").and_then(|v| resolve_frames(dirs, &doc, v)) else { continue };
            let (anims, _) = animations(&fdoc, &f);
            let frames = anims.get("default").or_else(|| anims.get("Default")).or_else(|| anims.values().next()).cloned().unwrap_or_default();
            let v2 = |k: &str| prop(&n.props, k).and_then(V::vec2).map(|(x, y)| Vec2::new(x, y)).unwrap_or(Vec2::ZERO);
            overlays.insert(n.name.clone(), Overlay { frames, pos: v2("position"), offset: v2("offset"), z: prop(&n.props, "z_index").and_then(V::num).unwrap_or(0.0) });
        }
        let node = doc.nodes.iter().find(|n| n.name == "Sprite" && n.parent.as_deref() == Some("Flip") && prop(&n.props, "frames").is_some());
        let Some(node) = node else {
            // inherits another scene: the root node is an instance of it
            let parent = doc.nodes.first().and_then(|r| r.instance).and_then(|id| doc.ext.get(&id)).map(|(p, _)| p.clone())?;
            return (depth < 4).then(|| Self::load_scene(dirs, &parent, base, base_default, depth + 1)).flatten();
        };
        let (fdoc, frames) = resolve_frames(dirs, &doc, prop(&node.props, "frames")?)?;
        let offset = prop(&node.props, "offset").and_then(V::vec2).map(|(x, y)| Vec2::new(x, y)).unwrap_or_else(|| base_offset(dirs, base, base_default));
        let (anims, by_snake) = animations(&fdoc, &frames);
        Some(CharArt { anims, by_snake, offset, overlays })
    }

/// The first of these animations the character has (original names, or converter ids).
    /// The first image of the idle animation, for portraits.
    pub fn idle_path(&self) -> Option<&str> {
        self.find(&["Wait", "idle"]).and_then(|f| f.first()).map(String::as_str)
    }

    fn find(&self, names: &[&str]) -> Option<&Vec<String>> {
        names.iter().find_map(|n| self.anims.get(*n).or_else(|| self.by_snake.get(*n).and_then(|o| self.anims.get(o))))
    }
}

#[derive(Resource, Default)]
pub struct Art {
    chars: HashMap<String, Option<CharArt>>,
    projectiles: HashMap<String, Option<CharArt>>,
    /// A scene node's animations (scene, node path) -> animation -> images.
    nodes: HashMap<(String, String), Option<HashMap<String, Vec<String>>>>,
    /// Scenes of imported fighters (id -> `res://` path without the prefix).
    pub scenes: HashMap<String, String>,
}

/// The animations a node of `scene` shows (following the scenes it inherits).
fn node_frames(dirs: &[PathBuf], scene: &str, path: &str, depth: u32) -> Option<HashMap<String, Vec<String>>> {
    let text = std::fs::read_to_string(find_in(dirs, scene)?).ok()?;
    let doc = godot::parse(&text);
    let path_of = |n: &godot::Node| match n.parent.as_deref() {
        None => ".".to_string(),
        Some(".") => n.name.clone(),
        Some(p) => format!("{p}/{}", n.name),
    };
    if let Some(n) = doc.nodes.iter().find(|n| path_of(n) == path && prop(&n.props, "frames").is_some()) {
        let (fdoc, frames) = resolve_frames(dirs, &doc, prop(&n.props, "frames")?)?;
        return Some(animations(&fdoc, &frames).0);
    }
    let parent = doc.nodes.first().and_then(|r| r.instance).and_then(|id| doc.ext.get(&id)).map(|(p, _)| p.clone())?;
    (depth < 6).then(|| node_frames(dirs, &parent, path, depth + 1)).flatten()
}

impl Art {
    /// The images of an animated sprite node, by animation (cut-ins, objects' sprites).
    pub fn node(&mut self, dirs: &[PathBuf], scene: &str, path: &str) -> Option<&HashMap<String, Vec<String>>> {
        self.nodes.entry((scene.to_string(), path.to_string())).or_insert_with(|| node_frames(dirs, scene, path, 0)).as_ref()
    }

    pub fn character(&mut self, dirs: &[PathBuf], id: &str) -> Option<&CharArt> {
        let scene = self.scenes.get(id).cloned().or_else(|| scene_of(id).map(str::to_string));
        self.chars.entry(id.to_string()).or_insert_with(|| CharArt::load(dirs, &scene?)).as_ref()
    }

    /// The sprite of one of a character's projectiles (`id` is the converter's projectile id).
    pub fn projectile(&mut self, dirs: &[PathBuf], char_id: &str, id: &str) -> Option<&CharArt> {
        let scene_file = self.scenes.get(char_id).cloned().or_else(|| scene_of(char_id).map(str::to_string));
        self.projectiles
            .entry(format!("{char_id}/{id}"))
            .or_insert_with(|| {
                let dir = Path::new(&scene_file?).parent()?.join("projectiles");
                let scene = dirs.iter().find_map(|d| {
                    std::fs::read_dir(d.join(&dir)).ok()?.filter_map(Result::ok).find_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let stem = name.strip_suffix(".tscn")?;
                        (convert::ron::snake(stem) == id).then(|| dir.join(&name).to_string_lossy().into_owned())
                    })
                })?;
                CharArt::load_scene(dirs, &scene, "projectile/BaseProjectile.tscn", Vec2::ZERO, 0)
            })
            .as_ref()
    }
}

/// The image path to show for this fighter right now.
pub fn frame_path<'a>(art: &'a CharArt, w: &World, i: usize) -> Option<&'a str> {
    let f: &Fighter = &w.f[i];
    let ch = &w.chars[i];
    let (names, frame, looped): (Vec<&str>, usize, bool) = match &f.act {
        Act::Move(m) => {
            let mv = ch.file.character.moves.get(m.idx);
            let anim = mv.and_then(|mv| mv.presentation.anim.as_deref()).unwrap_or("idle");
            // scripted (imported) moves may hold each image for several ticks, or loop
            let var = |k: &str| mv.and_then(|mv| mv.script.as_ref()).and_then(|s| s.vars.iter().find(|(n, _)| n == k)).map(|(_, v)| v.clone());
            let tpf = match var("ticks_per_frame") {
                Some(charfmt::ScriptVar::Int(n)) if n > 1 => n as usize,
                _ => 1,
            };
            let looped = matches!(var("loop_animation"), Some(charfmt::ScriptVar::Bool(true)));
            let tick = m.frame.saturating_sub(1) as usize / tpf;
            let frames = art.find(&[anim, "Wait"])?;
            let k = if looped { tick % frames.len() } else { tick.min(frames.len() - 1) };
            return frames.get(k).map(String::as_str);
        }
        Act::Idle => (vec!["Wait"], 0, true),
        Act::Walk { back } => (if *back { vec!["WalkBackward", "Wait"] } else { vec!["WalkForward", "Wait"] }, 0, true),
        Act::Crouch => (vec!["Crouch", "Wait"], 0, true),
        Act::JumpSquat { frame, .. } => (vec!["JumpTransition", "Wait"], *frame as usize, false),
        Act::Air => (if f.vel.y.to_f32() < 0.0 { vec!["Jump", "Fall", "Wait"] } else { vec!["Fall", "Jump", "Wait"] }, 0, true),
        Act::Landing { frame } => (vec!["Landing", "Wait"], *frame as usize, false),
        Act::Dash { frame, dir, .. } => (if *dir * f.facing >= 0 { vec!["DashForward", "Wait"] } else { vec!["DashBackward", "Wait"] }, *frame as usize, false),
        Act::Block { low, .. } => (if *low { vec!["ParryLow", "ParryHigh", "Wait"] } else { vec!["ParryHigh", "ParryLow", "Wait"] }, 0, false),
        Act::Hurt { launched, frame, .. } => {
            (if *launched || !f.grounded { vec!["HurtAerial", "HurtGroundedMid", "Wait"] } else { vec!["HurtGroundedMid", "HurtGroundedHigh", "HurtGroundedLow", "HurtAerial", "Wait"] }, *frame as usize, false)
        }
        Act::Knockdown { frame } => (vec!["Knockdown", "Wait"], *frame as usize, false),
        Act::Getup { frame } => (vec!["Getup", "Wait"], *frame as usize, false),
        Act::Held => (vec!["Grabbed", "HurtAerial", "Wait"], 0, false),
        Act::Ko => (vec!["Knockdown", "Wait"], usize::MAX, false),
    };
    let frames = art.find(&names)?;
    let n = frames.len();
    let k = if looped { (w.frame as usize / 4) % n } else { frame.min(n - 1) };
    frames.get(k).map(String::as_str)
}

/// The image of a frame a fighter's script picked (`host.sprite.frame = 3`), in the animation it
/// picked or else the one its move shows.
pub fn picked_frame<'a>(art: &'a CharArt, w: &World, i: usize, anim: Option<&str>, frame: Option<u32>) -> Option<&'a str> {
    let current = match &w.f[i].act {
        Act::Move(m) => w.chars[i].file.character.moves.get(m.idx).and_then(|mv| mv.presentation.anim.as_deref()),
        _ => None,
    };
    let frames = art.find(&[anim.or(current).unwrap_or("Wait"), "Wait"])?;
    let k = frame.map(|f| f as usize).unwrap_or(0).min(frames.len() - 1);
    frames.get(k).map(String::as_str)
}

/// The idle animation, looping (`tick` counts frames).
pub fn idle_frame(art: &CharArt, tick: u32) -> Option<&str> {
    let frames = art.find(&["Wait", "idle"])?;
    frames.get(tick as usize % frames.len()).map(String::as_str)
}

/// A fighter's battle-intro pose `tick` ticks in: its intro animation, else its taunt, else idle.
pub fn intro_frame(art: &CharArt, tick: u32) -> Option<&str> {
    let frames = art.find(&["Intro", "intro", "CrossIntro", "Start", "start", "Taunt", "taunt", "Wait"])?;
    let k = (tick as usize / 4).min(frames.len() - 1);
    frames.get(k).map(String::as_str)
}

/// The image for a projectile `age` frames after it spawned: its default animation, looping.
pub fn projectile_frame(art: &CharArt, age: u16) -> Option<&str> {
    let frames = art.find(&["Default", "default"]).or_else(|| art.anims.values().next())?;
    frames.get(age as usize % frames.len()).map(String::as_str)
}

/// An overlay sprite a move shows (the original's scripts show these during the move).
pub struct OverlayUse {
    pub node: &'static str,
    /// Godot rotation in radians (clockwise on screen).
    pub rotation: f32,
    pub frame: usize,
}

pub fn overlay_for(char_id: &str, move_id: &str, frame: u16) -> Option<OverlayUse> {
    use std::f32::consts::TAU;
    let tick = frame as usize;
    match (char_id, move_id) {
        ("robo", "chainsaw") => Some(OverlayUse { node: "ChainsawArm", rotation: 0.0, frame: tick }),
        ("robo", "drive_jump") => Some(OverlayUse { node: "DriveJumpSprite", rotation: tick as f32 * TAU / 8.0, frame: 0 }),
        ("wizard", "liftoff") => Some(OverlayUse { node: "LiftoffSprite", rotation: TAU / 4.0, frame: tick }),
        ("mutant", "twist_attack") => Some(OverlayUse { node: "TwistAttackSprite", rotation: 0.0, frame: tick }),
        ("cowboy", m) if m.starts_with("shoot") => Some(OverlayUse { node: "ShootingArm", rotation: 0.0, frame: 0 }),
        _ => None,
    }
}

impl CharArt {
    pub fn overlay_path(&self, node: &str, frame: usize) -> Option<(&Overlay, &str)> {
        let o = self.overlays.get(node)?;
        o.frames.get(frame % o.frames.len().max(1)).map(|p| (o, p.as_str()))
    }
}
