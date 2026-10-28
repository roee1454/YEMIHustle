//! Combat effects and sounds from the original game, loaded at runtime from the private copy of
//! its files (see `skin.rs`): hit sparks, block flashes, dust, smoke and every move's sounds.
//!
//! Effect scenes are Godot `ParticleEffect`s: frame-by-frame sprites (one frame per game tick),
//! `CPUParticles2D` emitters (simulated here the way Godot 3 does), and sounds played on the
//! first tick. Which effect plays when comes from the converted characters' `<id>_fx.ron` and
//! from the engine events below.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::audio::{Decodable, Volume};
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::math::{Affine2, Mat2};
use bevy::prelude::*;
use serde::Deserialize;

use crate::godot::{self, prop, Curve, Doc, Gradient, V};
use crate::settings::Settings;

// ---------------------------------------------------------------------------------------------
// Sound: the original's WAV files, decoded here (Bevy's own WAV support is not built in)
// ---------------------------------------------------------------------------------------------

#[derive(Asset, TypePath, Clone)]
pub struct Pcm {
    channels: u16,
    rate: u32,
    samples: Arc<[f32]>,
}

impl Decodable for Pcm {
    type Decoder = rodio::buffer::SamplesBuffer;

    fn decoder(&self) -> Self::Decoder {
        rodio::buffer::SamplesBuffer::new(
            std::num::NonZero::new(self.channels).expect("channels"),
            std::num::NonZero::new(self.rate).expect("rate"),
            self.samples.to_vec(),
        )
    }
}

/// PCM WAV (8/16/24/32-bit integer or 32-bit float).
pub fn decode_wav(b: &[u8]) -> Option<Pcm> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
        let body = &b[i + 8..(i + 8 + len).min(b.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + len + (len & 1);
    }
    let (fmt, data) = (fmt?, data?);
    let format = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]).max(1);
    let rate = u32::from_le_bytes(fmt[4..8].try_into().ok()?).max(1);
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    let samples: Vec<f32> = match (format, bits) {
        (3, 32) => data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        (_, 8) => data.iter().map(|v| (*v as f32 - 128.0) / 128.0).collect(),
        (_, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
        (_, 24) => data.chunks_exact(3).map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0).collect(),
        (_, 32) => data.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0).collect(),
        _ => return None,
    };
    Some(Pcm { channels, rate, samples: samples.into() })
}

// ---------------------------------------------------------------------------------------------
// Per-character data (written by tools/convert_character.py)
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
pub struct Sound {
    pub path: String,
    pub volume: f32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TimedSound {
    pub path: String,
    pub tick: u16,
    pub volume: f32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Particle {
    pub scene: String,
    pub pos: (f32, f32),
    pub tick: u16,
}

#[derive(Deserialize, Clone, Debug)]
pub struct HitboxFx {
    pub hit_sound: Option<Sound>,
    pub whiff_sound: Option<Sound>,
    pub bass_sound: Option<Sound>,
    pub bass_on_whiff: bool,
    pub hit_particle: Option<String>,
    pub replace_hit_particle: bool,
    pub whiff_particle: Option<String>,
    pub screenshake: i32,
    pub screenshake_frames: i32,
    pub bump_dir: (f32, f32),
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct MoveFx {
    pub enter_sfx: Option<Sound>,
    pub sfx: Option<TimedSound>,
    pub release_sfx: Option<Sound>,
    pub particle: Option<Particle>,
    pub timed_particle: Option<Particle>,
    pub hitboxes: HashMap<String, HitboxFx>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct CharacterFx {
    pub moves: HashMap<String, MoveFx>,
}

// ---------------------------------------------------------------------------------------------
// Effect scenes
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Tex {
    image: Handle<Image>,
    size: Vec2,
}

struct Emitter {
    emitting: bool,
    amount: usize,
    lifetime: f32,
    one_shot: bool,
    preprocess: f32,
    speed_scale: f32,
    explosiveness: f32,
    lifetime_randomness: f32,
    local_coords: bool,
    texture: Option<Tex>,
    frames: (u32, u32),
    anim_loop: bool,
    shape: i32,
    sphere_radius: f32,
    rect_extents: Vec2,
    direction: Vec2,
    spread: f32,
    gravity: Vec2,
    velocity: (f32, f32),
    angular_velocity: (f32, f32),
    linear_accel: (f32, f32, Option<Curve>),
    radial_accel: (f32, f32),
    damping: (f32, f32, Option<Curve>),
    angle: (f32, f32),
    scale: (f32, f32, Option<Curve>),
    color: [f32; 4],
    color_ramp: Option<Gradient>,
    anim_speed: (f32, f32),
    anim_offset: (f32, f32),
    align_y: bool,
}

enum PartKind {
    /// AnimatedSprite: frame `tick` is shown on game tick `tick`; removed after the last.
    Anim { frames: Vec<Tex>, offset: Vec2, centered: bool, flip_h: bool },
    Emitter(Box<Emitter>),
    Sound { path: String, volume: f32, pitch: f32 },
}

struct Part {
    /// Transform from the effect's root, in game units (Godot space: y down).
    xf: Affine2,
    visible: bool,
    modulate: [f32; 4],
    z: f32,
    kind: PartKind,
}

pub struct EffectDef {
    lifetime: f32,
    free: bool,
    z: f32,
    parts: Vec<Part>,
}

/// A node with its properties from every scene in the inheritance chain.
struct Flat {
    path: String,
    ty: String,
    props: Vec<(String, V, Arc<Doc>)>,
}

impl Flat {
    fn get(&self, k: &str) -> Option<(&V, &Arc<Doc>)> {
        self.props.iter().rev().find(|p| p.0 == k).map(|p| (&p.1, &p.2))
    }

    fn num(&self, k: &str, d: f32) -> f32 {
        self.get(k).and_then(|v| v.0.num()).unwrap_or(d)
    }

    fn vec(&self, k: &str, d: Vec2) -> Vec2 {
        self.get(k).and_then(|v| v.0.vec2()).map(|(x, y)| Vec2::new(x, y)).unwrap_or(d)
    }

    fn flag(&self, k: &str, d: bool) -> bool {
        self.get(k).and_then(|v| v.0.bool()).unwrap_or(d)
    }

    fn color(&self, k: &str) -> [f32; 4] {
        match self.get(k) {
            Some((V::Color(c), _)) => *c,
            _ => [1.0; 4],
        }
    }

    fn sub(&self, k: &str) -> Option<(String, godot::Props)> {
        let (v, doc) = self.get(k)?;
        doc.sub_res(v).cloned()
    }
}

// ---------------------------------------------------------------------------------------------
// The library: loads and caches scenes, images and sounds
// ---------------------------------------------------------------------------------------------

#[derive(Resource)]
pub struct FxLibrary {
    dir: Option<PathBuf>,
    /// Folders of imported mods, searched before `dir`.
    extra: Vec<PathBuf>,
    effects: HashMap<String, Option<Arc<EffectDef>>>,
    images: HashMap<String, Option<Tex>>,
    /// Decoded sprite sheets that atlas frames are cut from.
    sheets: HashMap<String, Option<Image>>,
    sounds: HashMap<String, Option<Handle<Pcm>>>,
    /// The base character's named sounds (Block2, Landing...): path, volume dB, pitch.
    base_sounds: HashMap<String, (String, f32, f32)>,
    pub chars: HashMap<String, CharacterFx>,
    rng: u64,
}

impl FxLibrary {
    pub fn new(dir: Option<PathBuf>, data_dir: &Path) -> Self {
        let mut chars = HashMap::new();
        let mut ids: Vec<String> = crate::ROSTER.iter().map(|s| s.to_string()).collect();
        for e in std::fs::read_dir(data_dir).into_iter().flatten().flatten() {
            if let Some(id) = e.file_name().to_string_lossy().strip_suffix("_fx.ron") {
                if !ids.iter().any(|i| i == id) {
                    ids.push(id.to_string());
                }
            }
        }
        for id in &ids {
            let p = data_dir.join(format!("{id}_fx.ron"));
            if let Ok(t) = std::fs::read_to_string(&p) {
                match ron::from_str::<CharacterFx>(&t) {
                    Ok(c) => {
                        chars.insert(id.to_string(), c);
                    }
                    Err(e) => eprintln!("fx: could not read {}: {e}", p.display()),
                }
            }
        }
        let mut base_sounds = HashMap::new();
        if let Some(d) = &dir {
            if let Ok(t) = std::fs::read_to_string(d.join("characters/BaseChar.tscn")) {
                let doc = godot::parse(&t);
                for n in doc.nodes.iter().filter(|n| n.parent.as_deref() == Some("Sounds")) {
                    if let Some(path) = prop(&n.props, "stream").and_then(|v| doc.ext_path(v)) {
                        let vol = prop(&n.props, "volume_db").and_then(V::num).unwrap_or(0.0);
                        let pitch = prop(&n.props, "pitch_scale").and_then(V::num).unwrap_or(1.0);
                        base_sounds.insert(n.name.clone(), (path.to_string(), vol, pitch));
                    }
                }
            }
        }
        FxLibrary { dir, extra: Vec::new(), effects: HashMap::new(), images: HashMap::new(), sheets: HashMap::new(), sounds: HashMap::new(), base_sounds, chars, rng: 0x9e37_79b9_7f4a_7c15 }
    }

    pub fn available(&self) -> bool {
        self.dir.is_some()
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Adds the folders of imported mods (their `res` roots).
    pub fn set_mod_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.extra = dirs;
    }

    /// Where to look for a game file, most specific first.
    pub fn dirs(&self) -> Vec<PathBuf> {
        self.extra.iter().cloned().chain(self.dir.clone()).collect()
    }

    fn find(&self, rel: &str) -> Option<PathBuf> {
        crate::art::find_in(&self.dirs(), rel)
    }

    /// A game image (path inside the library's game folder) and its size in pixels.
    pub fn texture(&mut self, path: &str, images: &mut Assets<Image>) -> Option<(Handle<Image>, Vec2)> {
        self.image(path, images).map(|t| (t.image, t.size))
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Like `texture`, with the sprite's pure-white pixels recolored: the original's shader
    /// paints those in the player's color and leaves every other pixel alone.
    pub fn texture_tinted(&mut self, path: &str, tint: [u8; 3], images: &mut Assets<Image>) -> Option<(Handle<Image>, Vec2)> {
        self.texture_styled(path, tint, None, images)
    }

    /// Like `texture_tinted`, and with an outline: transparent pixels next to the sprite take
    /// `outline` (a style's outline, like the original's outline shader).
    pub fn texture_styled(&mut self, path: &str, tint: [u8; 3], outline: Option<[u8; 3]>, images: &mut Assets<Image>) -> Option<(Handle<Image>, Vec2)> {
        let mut key = format!("{path}#{:02x}{:02x}{:02x}", tint[0], tint[1], tint[2]);
        if let Some(o) = outline {
            key.push_str(&format!("/{:02x}{:02x}{:02x}", o[0], o[1], o[2]));
        }
        if let Some(t) = self.images.get(&key) {
            return t.clone().map(|t| (t.image, t.size));
        }
        let tex = self.decode(path).and_then(|mut img| {
            let size = img.size().as_vec2();
            let (w, h) = (img.width() as usize, img.height() as usize);
            if img.texture_descriptor.format.block_copy_size(None) == Some(4) {
                if let Some(d) = img.data.as_mut() {
                    for px in d.chunks_exact_mut(4) {
                        if px[0] >= 253 && px[1] >= 253 && px[2] >= 253 {
                            px[..3].copy_from_slice(&tint);
                        }
                    }
                    if let Some(o) = outline {
                        let alpha: Vec<u8> = d.chunks_exact(4).map(|p| p[3]).collect();
                        let solid = |x: usize, y: usize| alpha[y * w + x] >= 128;
                        for y in 0..h {
                            for x in 0..w {
                                if alpha[y * w + x] >= 16 {
                                    continue;
                                }
                                let near = (x > 0 && solid(x - 1, y)) || (x + 1 < w && solid(x + 1, y)) || (y > 0 && solid(x, y - 1)) || (y + 1 < h && solid(x, y + 1));
                                if near {
                                    let i = (y * w + x) * 4;
                                    d[i..i + 4].copy_from_slice(&[o[0], o[1], o[2], 255]);
                                }
                            }
                        }
                    }
                }
            }
            Some(Tex { image: images.add(img), size })
        });
        self.images.insert(key, tex.clone());
        tex.map(|t| (t.image, t.size))
    }

    fn decode_file(&self, path: &str) -> Option<Image> {
        let bytes = std::fs::read(self.find(path)?).ok()?;
        Image::from_buffer(&bytes, ImageType::Extension(crate::gamefiles::sniff_ext(&bytes, path)), CompressedImageFormats::NONE, true, ImageSampler::nearest(), RenderAssetUsages::default()).ok()
    }

    /// An image by path; `sheet.png#atlas:x,y,w,h` is that region of a sprite sheet (the
    /// original's `AtlasTexture` frames, see `art.rs`).
    fn decode(&mut self, path: &str) -> Option<Image> {
        let Some((sheet, rect)) = path.split_once("#atlas:") else { return self.decode_file(path) };
        let r: Vec<u32> = rect.split(',').filter_map(|n| n.trim().parse::<f32>().ok()).map(|n| n.max(0.0) as u32).collect();
        let [x, y, w, h] = r[..] else { return None };
        if !self.sheets.contains_key(sheet) {
            let img = self.decode_file(sheet).and_then(|i| i.convert(bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb));
            self.sheets.insert(sheet.to_string(), img);
        }
        let src = self.sheets.get(sheet)?.as_ref()?;
        let (sw, sh) = (src.width(), src.height());
        let data = src.data.as_ref()?;
        let (w, h) = (w.min(sw.saturating_sub(x)), h.min(sh.saturating_sub(y)));
        if w == 0 || h == 0 {
            return None;
        }
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let start = ((row * sw + x) * 4) as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        let mut img = Image::new(
            bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            out,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        img.sampler = ImageSampler::nearest();
        Some(img)
    }

    fn image(&mut self, path: &str, images: &mut Assets<Image>) -> Option<Tex> {
        if let Some(t) = self.images.get(path) {
            return t.clone();
        }
        let tex = self.decode(path).and_then(|img| {
            let size = img.size().as_vec2();
            Some(Tex { image: images.add(img), size })
        });
        self.images.insert(path.to_string(), tex.clone());
        tex
    }

    pub fn sound(&mut self, path: &str, pcm: &mut Assets<Pcm>) -> Option<Handle<Pcm>> {
        if let Some(s) = self.sounds.get(path) {
            return s.clone();
        }
        let h = self.find(path).and_then(|p| std::fs::read(p).ok()).and_then(|b| decode_wav(&b)).map(|p| pcm.add(p));
        if h.is_none() && self.dir.is_some() {
            eprintln!("fx: no sound at {path}");
        }
        self.sounds.insert(path.to_string(), h.clone());
        h
    }

    fn flatten(&self, path: &str, depth: u32) -> Vec<Flat> {
        if depth > 8 {
            return vec![];
        }
        let Some(file) = self.find(path) else { return vec![] };
        let Ok(text) = std::fs::read_to_string(file) else { return vec![] };
        let doc = Arc::new(godot::parse(&text));
        let mut out: Vec<Flat> = Vec::new();
        for (k, n) in doc.nodes.iter().enumerate() {
            let at = doc.path_of(n);
            let own: Vec<(String, V, Arc<Doc>)> = n.props.iter().map(|(a, b)| (a.clone(), b.clone(), doc.clone())).collect();
            if let Some(inst) = n.instance {
                // an instanced scene: graft its nodes under this one
                let base = doc.ext.get(&inst).map(|e| e.0.clone()).unwrap_or_default();
                for mut f in self.flatten(&base, depth + 1) {
                    f.path = match (k, f.path.as_str()) {
                        (0, p) => p.to_string(),
                        (_, ".") => at.clone(),
                        (_, p) => format!("{at}/{p}"),
                    };
                    out.push(f);
                }
                if let Some(f) = out.iter_mut().find(|f| f.path == at) {
                    f.props.extend(own);
                }
                continue;
            }
            match out.iter_mut().find(|f| f.path == at) {
                Some(f) => f.props.extend(own),
                None => out.push(Flat { path: at, ty: n.ty.clone().unwrap_or_default(), props: own }),
            }
        }
        out
    }

    pub fn effect(&mut self, path: &str, images: &mut Assets<Image>) -> Option<Arc<EffectDef>> {
        if let Some(e) = self.effects.get(path) {
            return e.clone();
        }
        let nodes = self.flatten(path, 0);
        let def = (!nodes.is_empty()).then(|| Arc::new(self.build(&nodes, images, false)));
        if def.is_none() && self.dir.is_some() {
            eprintln!("fx: could not load effect {path}");
        }
        self.effects.insert(path.to_string(), def.clone());
        def
    }

    /// One node of a scene (and what hangs under it) as a lasting effect: a fighter's or
    /// object's particle node that its scripts show (`$"Flip/void1".show()`). Placed at the
    /// node itself: its own position is left out.
    pub fn node_effect(&mut self, scene: &str, node: &str, images: &mut Assets<Image>) -> Option<Arc<EffectDef>> {
        let key = format!("{scene}#{node}");
        if let Some(e) = self.effects.get(&key) {
            return e.clone();
        }
        let leaf = node.rsplit('/').next().unwrap_or(node).to_string();
        let prefix = format!("{node}/");
        let mut nodes: Vec<Flat> = self
            .flatten(scene, 0)
            .into_iter()
            .filter(|n| n.path == node || n.path.starts_with(&prefix))
            .map(|mut n| {
                if n.path == node {
                    n.props.retain(|p| p.0 != "position" && p.0 != "visible");
                    n.path = leaf.clone();
                } else {
                    n.path = format!("{leaf}/{}", &n.path[prefix.len()..]);
                }
                n
            })
            .collect();
        let def = (!nodes.is_empty()).then(|| {
            nodes.push(Flat { path: ".".into(), ty: "Node2D".into(), props: vec![] });
            Arc::new(self.build(&nodes, images, true))
        });
        self.effects.insert(key, def.clone());
        def
    }

    /// `persistent`: emitters keep emitting and the effect lasts until it is taken away.
    fn build(&mut self, nodes: &[Flat], images: &mut Assets<Image>, persistent: bool) -> EffectDef {
        let root = nodes.iter().find(|n| n.path == ".");
        let get = |k: &str, d: f32| root.map(|r| r.num(k, d)).unwrap_or(d);
        let one_shot = !persistent && root.map(|r| r.flag("one_shot", true)).unwrap_or(true);
        let mut parts = Vec::new();
        for n in nodes.iter().filter(|n| n.path != ".") {
            // compose transforms down the node path
            let mut xf = Affine2::IDENTITY;
            let mut visible = n.flag("visible", true);
            let mut modulate = mul4(n.color("modulate"), n.color("self_modulate"));
            let mut prefix = String::new();
            for seg in n.path.split('/') {
                prefix = if prefix.is_empty() { seg.to_string() } else { format!("{prefix}/{seg}") };
                if let Some(a) = nodes.iter().find(|m| m.path == prefix) {
                    xf *= local_xf(a);
                    if a.path != n.path {
                        visible &= a.flag("visible", true);
                        modulate = mul4(modulate, a.color("modulate"));
                    }
                }
            }
            let z = n.num("z_index", 0.0);
            let kind = match n.ty.as_str() {
                "AnimatedSprite" | "Sprite" => {
                    let mut frames = Vec::new();
                    if n.ty == "Sprite" {
                        if let Some(p) = n.get("texture").and_then(|(v, d)| d.ext_path(v).map(str::to_string)) {
                            frames.extend(self.image(&p, images));
                        }
                    } else if let Some((_, sf)) = n.sub("frames") {
                        let doc = n.get("frames").map(|v| v.1.clone()).expect("frames");
                        if let Some(anim) = prop(&sf, "animations").and_then(|a| a.items().first().cloned()) {
                            for f in anim.get("frames").map(|f| f.items().to_vec()).unwrap_or_default() {
                                if let Some(p) = doc.ext_path(&f) {
                                    frames.extend(self.image(p, images));
                                }
                            }
                        }
                    }
                    if frames.is_empty() {
                        continue;
                    }
                    PartKind::Anim { frames, offset: n.vec("offset", Vec2::ZERO), centered: n.flag("centered", true), flip_h: n.flag("flip_h", false) }
                }
                "CPUParticles2D" => {
                    let mut em = self.emitter(n, one_shot, images);
                    // shown by a script: it emits
                    em.emitting |= persistent;
                    PartKind::Emitter(Box::new(em))
                }
                "AudioStreamPlayer2D" | "AudioStreamPlayer" => {
                    let Some(path) = n.get("stream").and_then(|(v, d)| d.ext_path(v).map(str::to_string)) else { continue };
                    PartKind::Sound { path, volume: n.num("volume_db", 0.0), pitch: n.num("pitch_scale", 1.0) }
                }
                _ => continue,
            };
            parts.push(Part { xf, visible, modulate, z, kind });
        }
        EffectDef { lifetime: get("lifetime", 1.0), free: !persistent && root.map(|r| r.flag("free", true)).unwrap_or(true), z: get("z_index", 0.0), parts }
    }

    fn emitter(&mut self, n: &Flat, one_shot: bool, images: &mut Assets<Image>) -> Emitter {
        let curve = |k: &str| n.sub(k).map(|(_, p)| Curve::from_props(&p));
        let texture = n.get("texture").and_then(|(v, d)| d.ext_path(v).map(str::to_string)).and_then(|p| self.image(&p, images));
        let (mut frames, mut anim_loop) = ((1, 1), false);
        if let Some((ty, m)) = n.sub("material") {
            if ty == "CanvasItemMaterial" && prop(&m, "particles_animation").and_then(V::bool).unwrap_or(false) {
                let g = |k: &str| prop(&m, k).and_then(V::num).unwrap_or(1.0).max(1.0) as u32;
                frames = (g("particles_anim_h_frames"), g("particles_anim_v_frames"));
                anim_loop = prop(&m, "particles_anim_loop").and_then(V::bool).unwrap_or(false);
            }
        }
        let pair = |k: &str, d: f32| (n.num(k, d), n.num(&format!("{k}_random"), 0.0));
        Emitter {
            emitting: n.flag("emitting", true),
            amount: n.num("amount", 8.0).max(1.0) as usize,
            lifetime: n.num("lifetime", 1.0).max(0.001),
            // ParticleEffect sets every emitter's one_shot to its own
            one_shot,
            preprocess: n.num("preprocess", 0.0),
            speed_scale: n.num("speed_scale", 1.0),
            explosiveness: n.num("explosiveness", 0.0),
            lifetime_randomness: n.num("lifetime_randomness", 0.0),
            local_coords: n.flag("local_coords", true),
            texture,
            frames,
            anim_loop,
            shape: n.num("emission_shape", 0.0) as i32,
            sphere_radius: n.num("emission_sphere_radius", 1.0),
            rect_extents: n.vec("emission_rect_extents", Vec2::ONE),
            direction: n.vec("direction", Vec2::X),
            spread: n.num("spread", 45.0),
            gravity: n.vec("gravity", Vec2::new(0.0, 98.0)),
            velocity: pair("initial_velocity", 0.0),
            angular_velocity: pair("angular_velocity", 0.0),
            linear_accel: (n.num("linear_accel", 0.0), n.num("linear_accel_random", 0.0), curve("linear_accel_curve")),
            radial_accel: pair("radial_accel", 0.0),
            damping: (n.num("damping", 0.0), n.num("damping_random", 0.0), curve("damping_curve")),
            angle: pair("angle", 0.0),
            scale: (n.num("scale_amount", 1.0), n.num("scale_amount_random", 0.0), curve("scale_amount_curve")),
            color: n.color("color"),
            color_ramp: n.sub("color_ramp").map(|(_, p)| Gradient::from_props(&p)),
            anim_speed: pair("anim_speed", 0.0),
            anim_offset: pair("anim_offset", 0.0),
            align_y: n.flag("flag_align_y", false),
        }
    }

    /// A named sound of the base character (`Block2`, `Landing`...).
    pub fn base_sound(&self, name: &str) -> Option<(String, f32, f32)> {
        self.base_sounds.get(name).cloned()
    }
}

fn mul4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| a[i] * b[i])
}

/// A Node2D's own transform: translate, rotate (radians), scale.
fn local_xf(n: &Flat) -> Affine2 {
    let pos = n.vec("position", Vec2::ZERO);
    let rot = n.num("rotation", 0.0);
    let rot = if n.get("rotation").is_none() { n.num("rotation_degrees", 0.0).to_radians() } else { rot };
    let scale = n.vec("scale", Vec2::ONE);
    Affine2::from_scale_angle_translation(scale, rot, pos)
}

// ---------------------------------------------------------------------------------------------
// Running effects
// ---------------------------------------------------------------------------------------------

struct P {
    pos: Vec2,
    vel: Vec2,
    rot: f32,
    scale: f32,
    color: [f32; 4],
    frame: u32,
    time: f32,
    life: f32,
    active: bool,
    spawned: bool,
    /// per-particle random numbers: angle, scale, anim offset, accel, radial, damping, angular velocity, anim speed
    r: [f32; 8],
}

struct EmitterRun {
    time: f32,
    particles: Vec<P>,
    /// Gravity mirrored for effects facing left (ParticleEffect.on_update_timer_timeout).
    gravity: Vec2,
}

/// An effect playing in the world. Lives on an entity whose children draw its parts.
#[derive(Component)]
pub struct ActiveFx {
    def: Arc<EffectDef>,
    /// Effect root in game units (Godot space).
    root: Affine2,
    tick: u32,
    emitters: Vec<Option<EmitterRun>>,
    sprites: Vec<Vec<Entity>>,
}

/// Screen shake from hits (BetterCamera.bump): random offsets that fade out.
#[derive(Resource, Default)]
pub struct Shake {
    /// amount, elapsed, duration (seconds), direction (zero: a random direction each frame)
    bumps: Vec<(f32, f32, f32, Vec2)>,
    rng: u64,
    pub offset: Vec2,
}

impl Shake {
    pub fn bump(&mut self, amount: f32, frames: f32, dir: Vec2) {
        let amount = amount.min(50.0) * 1.5;
        let time = frames / 60.0 * 1.5;
        if amount > 0.0 && time > 0.0 {
            self.bumps.push((amount, 0.0, time, dir));
        }
    }

    pub fn update(&mut self, dt: f32) {
        let mut sum = Vec2::ZERO;
        for b in &mut self.bumps {
            b.1 += dt;
            // tween from the amount down to zero with a circular ease-out
            let t = (b.1 / b.2).clamp(0.0, 1.0);
            let ease = (1.0 - (t - 1.0) * (t - 1.0)).sqrt();
            let a = b.0 * (1.0 - ease);
            if b.3 != Vec2::ZERO {
                sum += b.3 * a;
            } else {
                self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let ang = (self.rng >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;
                sum += Vec2::new(ang.cos(), ang.sin()) * a;
            }
        }
        self.bumps.retain(|b| b.1 < b.2);
        self.offset = sum;
    }
}

/// Starts an effect at `pos` (game units) pointing along `dir`.
pub fn spawn_effect(commands: &mut Commands, lib: &mut FxLibrary, images: &mut Assets<Image>, scene: &str, pos: Vec2, dir: Vec2) -> bool {
    let Some(def) = lib.effect(scene, images) else { return false };
    spawn_def(commands, lib, def, pos, dir);
    true
}

/// A scene node shown by scripts, as a lasting effect at `pos` (see [`FxLibrary::node_effect`]);
/// move it with [`place_effect`] and despawn it when the node is hidden.
pub fn spawn_node_effect(commands: &mut Commands, lib: &mut FxLibrary, images: &mut Assets<Image>, scene: &str, node: &str, pos: Vec2, facing: f32) -> Option<Entity> {
    let def = lib.node_effect(scene, node, images)?;
    Some(spawn_def(commands, lib, def, pos, Vec2::new(facing, 0.0)))
}

/// Moves a lasting effect (a shown scene node) with its owner.
pub fn place_effect(fx: &mut ActiveFx, pos: Vec2, facing: f32) {
    fx.root = Affine2::from_scale_angle_translation(Vec2::new(if facing < 0.0 { -1.0 } else { 1.0 }, 1.0), 0.0, pos);
}

fn spawn_def(commands: &mut Commands, lib: &mut FxLibrary, def: Arc<EffectDef>, pos: Vec2, dir: Vec2) -> Entity {
    let facing = if dir.x < 0.0 { -1.0 } else { 1.0 };
    let rot = if facing < 0.0 { (dir * Vec2::new(-1.0, -1.0)).to_angle() } else { dir.to_angle() };
    let root = Affine2::from_scale_angle_translation(Vec2::new(facing, 1.0), rot, pos);
    let mut emitters = Vec::new();
    let mut sprites = Vec::new();
    let e = commands.spawn((Transform::default(), Visibility::default())).id();
    for part in &def.parts {
        match &part.kind {
            PartKind::Emitter(em) => {
                let mut gravity = em.gravity;
                if facing < 0.0 {
                    gravity.x = -gravity.x;
                }
                let mut run = EmitterRun { time: 0.0, particles: Vec::with_capacity(em.amount), gravity };
                for _ in 0..em.amount {
                    let r: [f32; 8] = std::array::from_fn(|_| lib.rand());
                    run.particles.push(P { pos: Vec2::ZERO, vel: Vec2::ZERO, rot: 0.0, scale: 1.0, color: [1.0; 4], frame: 0, time: 0.0, life: 1.0, active: false, spawned: false, r });
                }
                let n = em.amount;
                let children = (0..n).map(|_| commands.spawn((Sprite::default(), Transform::default(), Visibility::Hidden, ChildOf(e))).id()).collect();
                // preprocess: run the emitter ahead before showing it
                let world = root * part.xf;
                let mut steps = (em.preprocess * 60.0).round() as i32;
                while steps > 0 {
                    step_emitter(em, &mut run, world, 1.0 / 60.0, lib);
                    steps -= 1;
                }
                emitters.push(Some(run));
                sprites.push(children);
            }
            PartKind::Anim { .. } => {
                emitters.push(None);
                sprites.push(vec![commands.spawn((Sprite::default(), Transform::default(), Visibility::Hidden, ChildOf(e))).id()]);
            }
            PartKind::Sound { .. } => {
                emitters.push(None);
                sprites.push(vec![]);
            }
        }
    }
    commands.entity(e).insert(ActiveFx { def, root, tick: 0, emitters, sprites });
    e
}

/// One update of a CPUParticles2D, following Godot 3.5's `_particles_process`.
fn step_emitter(em: &Emitter, run: &mut EmitterRun, world: Affine2, dt: f32, lib: &mut FxLibrary) {
    let dt = dt * em.speed_scale;
    let prev = run.time;
    run.time += dt;
    let n = em.particles_len();
    for (i, p) in run.particles.iter_mut().enumerate() {
        // when this particle (re)starts within the emission cycle
        let restart = i as f32 / n as f32 * em.lifetime * (1.0 - em.explosiveness);
        let cycle = |t: f32| (t / em.lifetime).floor();
        let phase = |t: f32| t - cycle(t) * em.lifetime;
        let crossed = if em.one_shot {
            !p.spawned && run.time >= restart
        } else {
            (cycle(run.time) > cycle(prev) && phase(run.time) >= restart) || (cycle(run.time) == cycle(prev) && phase(prev) < restart && phase(run.time) >= restart) || (prev == 0.0 && restart == 0.0)
        };
        if crossed && em.emitting && !(em.one_shot && p.spawned) {
            p.spawned = true;
            p.active = true;
            p.time = 0.0;
            p.life = em.lifetime * (1.0 - lib_rand(&mut p.r, 0) * em.lifetime_randomness);
            let ang = em.direction.y.atan2(em.direction.x) + ((p.r[1] * 2.0 - 1.0) * em.spread).to_radians();
            let speed = em.velocity.0 * lerp(1.0, p.r[2], em.velocity.1);
            let mut pos = match em.shape {
                1 => {
                    let (s, t) = (p.r[3], std::f32::consts::TAU * p.r[4]);
                    Vec2::new(t.cos(), t.sin()) * em.sphere_radius * (1.0 - s * s).sqrt()
                }
                2 => Vec2::new(p.r[3] * 2.0 - 1.0, p.r[4] * 2.0 - 1.0) * em.rect_extents,
                _ => Vec2::ZERO,
            };
            let mut vel = Vec2::new(ang.cos(), ang.sin()) * speed;
            if !em.local_coords {
                vel = world.matrix2 * vel;
                pos = world.transform_point2(pos);
            }
            p.pos = pos;
            p.vel = vel;
            // fresh randoms for the next life
            for k in 0..8 {
                p.r[k] = lib.rand();
            }
        }
        if !p.active {
            continue;
        }
        p.time += dt;
        if p.time > p.life {
            p.active = false;
            continue;
        }
        let tv = p.time / p.life;
        let org = if em.local_coords { Vec2::ZERO } else { world.translation };
        let mut force = run.gravity;
        let lin = em.linear_accel.0 + em.linear_accel.2.as_ref().map(|c| c.sample(tv)).unwrap_or(0.0);
        if p.vel.length() > 0.0 {
            force += p.vel.normalize() * lin * lerp(1.0, p.r[3], em.linear_accel.1);
        }
        let diff = p.pos - org;
        if diff.length() > 0.0 {
            force += diff.normalize() * em.radial_accel.0 * lerp(1.0, p.r[4], em.radial_accel.1);
        }
        p.vel += force * dt;
        let damp = em.damping.0 + em.damping.2.as_ref().map(|c| c.sample(tv)).unwrap_or(0.0);
        if damp > 0.0 {
            let v = p.vel.length() - damp * lerp(1.0, p.r[5], em.damping.1) * dt;
            p.vel = if v < 0.0 { Vec2::ZERO } else { p.vel.normalize_or_zero() * v };
        }
        let custom1 = p.time / em.lifetime;
        let mut angle = em.angle.0 * lerp(1.0, p.r[0], em.angle.1);
        angle += custom1 * em.lifetime * em.angular_velocity.0 * lerp(1.0, p.r[6] * 2.0 - 1.0, em.angular_velocity.1);
        p.rot = angle.to_radians();
        if em.align_y && p.vel.length() > 0.0 {
            p.rot = p.vel.to_angle() - std::f32::consts::FRAC_PI_2;
        }
        let tex_scale = em.scale.2.as_ref().map(|c| c.sample(tv)).unwrap_or(1.0);
        p.scale = (tex_scale * lerp(em.scale.0, 1.0, p.r[1] * em.scale.1)).max(0.000001);
        let ramp = em.color_ramp.as_ref().map(|g| g.sample(tv)).unwrap_or([1.0; 4]);
        p.color = mul4(em.color, ramp);
        let phase = em.anim_offset.0 * lerp(1.0, p.r[2], em.anim_offset.1) + custom1 * em.anim_speed.0 * lerp(1.0, p.r[7], em.anim_speed.1);
        let total = (em.frames.0 * em.frames.1).max(1) as i64;
        let f = (total as f32 * phase) as i64;
        p.frame = if em.anim_loop { f.rem_euclid(total) } else { f.clamp(0, total - 1) } as u32;
        p.pos += p.vel * dt;
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lib_rand(r: &mut [f32; 8], k: usize) -> f32 {
    r[k]
}

impl Emitter {
    fn particles_len(&self) -> usize {
        self.amount
    }
}

/// Advances every effect by one game tick: sprite frames, particles, sounds, expiry.
pub fn tick_effects(
    mut commands: Commands,
    mut lib: ResMut<FxLibrary>,
    mut fxs: Query<(Entity, &mut ActiveFx)>,
    mut pcm: ResMut<Assets<Pcm>>,
    settings: Res<Settings>,
) {
    for (e, mut fx) in &mut fxs {
        fx.tick += 1;
        let def = fx.def.clone();
        let root = fx.root;
        let tick = fx.tick;
        for (k, part) in def.parts.iter().enumerate() {
            match &part.kind {
                PartKind::Emitter(em) => {
                    if let Some(run) = fx.emitters[k].as_mut() {
                        step_emitter(em, run, root * part.xf, 1.0 / 60.0, &mut lib);
                    }
                }
                PartKind::Sound { path, volume, pitch } if tick == 1 => {
                    play(&mut commands, &mut lib, &mut pcm, &settings, path, *volume, *pitch, 0.0);
                }
                _ => {}
            }
        }
        if def.free && tick as f32 / 60.0 >= def.lifetime {
            commands.entity(e).despawn();
        }
    }
}

/// Draws effects: called after the fight camera is placed. `to_screen` maps game units to
/// window pixels; `zoom` is its scale.
pub fn draw_effects(
    fxs: Query<&ActiveFx>,
    mut sprites: Query<(&mut Sprite, &mut Transform, &mut Visibility)>,
    game: Option<Res<crate::Game>>,
    shake: Res<Shake>,
) {
    let Some(game) = game else { return };
    let (zoom, ground) = (crate::fight::ZOOM, crate::fight::GROUND_Y);
    // game units (y down) -> window (y up), including the camera and its shake
    let screen = Affine2::from_cols(
        Vec2::new(zoom, 0.0),
        Vec2::new(0.0, -zoom),
        Vec2::new((-game.cam_x + shake.offset.x) * zoom, ground - shake.offset.y * zoom),
    );
    for fx in &fxs {
        let def = &fx.def;
        for (k, part) in def.parts.iter().enumerate() {
            let ents = &fx.sprites[k];
            let base_z = 20.0 + def.z * 0.01 + part.z * 0.001 + k as f32 * 0.0001;
            match &part.kind {
                PartKind::Anim { frames, offset, centered, flip_h } => {
                    let Ok((mut s, mut tf, mut vis)) = sprites.get_mut(ents[0]) else { continue };
                    let idx = fx.tick as usize;
                    if !part.visible || idx >= frames.len() {
                        *vis = Visibility::Hidden;
                        continue;
                    }
                    let tex = &frames[idx];
                    let corner = if *centered { Vec2::ZERO } else { tex.size / 2.0 };
                    let m = screen * fx.root * part.xf * Affine2::from_translation(*offset + corner);
                    set_sprite(&mut s, &mut tf, tex, None, m, part.modulate, base_z, *flip_h);
                    *vis = Visibility::Visible;
                }
                PartKind::Emitter(em) => {
                    let Some(run) = &fx.emitters[k] else { continue };
                    let world = fx.root * part.xf;
                    for (i, p) in run.particles.iter().enumerate() {
                        let Ok((mut s, mut tf, mut vis)) = sprites.get_mut(ents[i]) else { continue };
                        if !p.active || !part.visible {
                            *vis = Visibility::Hidden;
                            continue;
                        }
                        let local = Affine2::from_scale_angle_translation(Vec2::splat(p.scale), p.rot, p.pos);
                        let m = if em.local_coords { screen * world * local } else { screen * local };
                        let color = mul4(p.color, part.modulate);
                        let z = base_z + i as f32 * 0.000001;
                        match &em.texture {
                            Some(tex) => {
                                let (h, v) = em.frames;
                                let fs = Vec2::new(tex.size.x / h as f32, tex.size.y / v as f32);
                                let (fx_, fy) = ((p.frame % h) as f32, (p.frame / h) as f32);
                                let rect = Rect::new(fx_ * fs.x, fy * fs.y, (fx_ + 1.0) * fs.x, (fy + 1.0) * fs.y);
                                set_sprite(&mut s, &mut tf, tex, Some((rect, fs)), m, color, z, false);
                            }
                            None => {
                                s.image = Handle::default();
                                s.custom_size = Some(Vec2::ONE);
                                apply_affine(&mut tf, m, z);
                                s.color = Color::srgba(color[0], color[1], color[2], color[3]);
                            }
                        }
                        *vis = Visibility::Visible;
                    }
                }
                PartKind::Sound { .. } => {}
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn set_sprite(s: &mut Sprite, tf: &mut Transform, tex: &Tex, region: Option<(Rect, Vec2)>, m: Affine2, color: [f32; 4], z: f32, flip: bool) {
    if s.image != tex.image {
        s.image = tex.image.clone();
    }
    let (rect, size) = region.map(|(r, s)| (Some(r), s)).unwrap_or((None, tex.size));
    s.rect = rect;
    s.custom_size = Some(size);
    s.flip_x = flip;
    s.color = Color::srgba(color[0], color[1], color[2], color[3]);
    apply_affine(tf, m, z);
}

/// A 2D affine without shear as a Bevy transform.
fn apply_affine(tf: &mut Transform, m: Affine2, z: f32) {
    let a: Mat2 = m.matrix2;
    let sx = a.x_axis.length();
    let det = a.determinant();
    let sy = if sx > 0.0 { det / sx } else { 0.0 };
    let rot = a.x_axis.y.atan2(a.x_axis.x);
    tf.translation = m.translation.extend(z);
    tf.rotation = Quat::from_rotation_z(rot);
    tf.scale = Vec3::new(sx, sy, 1.0);
}

/// Plays a sound file from the original with a volume in dB, a pitch, and a random pitch
/// variation (+- `vary`), on the game's effects volume.
#[allow(clippy::too_many_arguments)]
pub fn play(commands: &mut Commands, lib: &mut FxLibrary, pcm: &mut Assets<Pcm>, settings: &Settings, path: &str, db: f32, pitch: f32, vary: f32) {
    let Some(h) = lib.sound(path, pcm) else { return };
    let vol = |k: &str| crate::settings::OPTIONS.iter().find(|f| f.key() == Some(k)).map(|f| settings.options.get(f)).unwrap_or(1.0) as f32;
    let linear = 10f32.powf(db / 20.0) * vol("fx_volume") * vol("master_volume");
    let speed = pitch * (1.0 + (lib.rand() * 2.0 - 1.0) * vary);
    commands.spawn((AudioPlayer(h), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(linear)).with_speed(speed)));
}

pub fn update_shake(time: Res<Time>, mut shake: ResMut<Shake>) {
    shake.update(time.delta_secs());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_16_bit_wav() {
        let mut b = Vec::new();
        let data: [i16; 4] = [0, 16384, -16384, 32767];
        b.extend(b"RIFF");
        b.extend(&(36u32 + 8).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(&16u32.to_le_bytes());
        b.extend(&1u16.to_le_bytes()); // PCM
        b.extend(&1u16.to_le_bytes()); // mono
        b.extend(&44100u32.to_le_bytes());
        b.extend(&(44100u32 * 2).to_le_bytes());
        b.extend(&2u16.to_le_bytes());
        b.extend(&16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(&8u32.to_le_bytes());
        for s in data {
            b.extend(&s.to_le_bytes());
        }
        let p = decode_wav(&b).unwrap();
        assert_eq!((p.channels, p.rate, p.samples.len()), (1, 44100, 4));
        assert!((p.samples[1] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn shake_fades_out() {
        let mut s = Shake::default();
        s.bump(4.0, 10.0, Vec2::ZERO);
        s.update(0.01);
        assert!(s.offset.length() > 0.0);
        for _ in 0..100 {
            s.update(0.01);
        }
        assert_eq!(s.offset, Vec2::ZERO);
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;

    fn lib() -> Option<FxLibrary> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = root.join("reference/yomi_recovered");
        dir.join("project.godot").exists().then(|| FxLibrary::new(Some(dir), &root.join("data/converted")))
    }

    /// Every effect scene and sound the converted characters name loads (needs the private copy).
    #[test]
    fn every_referenced_effect_and_sound_loads() {
        let Some(mut lib) = lib() else { return };
        let mut images = Assets::<Image>::default();
        let mut pcm = Assets::<Pcm>::default();
        let (mut scenes, mut sounds) = (vec!["fx/HitEffect1.tscn".to_string(), "fx/FlawedParryEffect.tscn".into(), "fx/DashParticle.tscn".into(), "fx/KnockbackSmoke.tscn".into()], vec![]);
        for c in lib.chars.values() {
            for m in c.moves.values() {
                scenes.extend(m.particle.iter().chain(&m.timed_particle).map(|p| p.scene.clone()));
                sounds.extend(m.enter_sfx.iter().chain(&m.release_sfx).map(|s| s.path.clone()));
                sounds.extend(m.sfx.iter().map(|s| s.path.clone()));
                for h in m.hitboxes.values() {
                    scenes.extend(h.hit_particle.iter().chain(&h.whiff_particle).cloned());
                    sounds.extend(h.hit_sound.iter().chain(&h.whiff_sound).chain(&h.bass_sound).map(|s| s.path.clone()));
                }
            }
        }
        scenes.sort();
        scenes.dedup();
        sounds.sort();
        sounds.dedup();
        let bad_scenes: Vec<_> = scenes.iter().filter(|s| lib.effect(s, &mut images).is_none_or(|d| d.parts.is_empty())).cloned().collect();
        let bad_sounds: Vec<_> = sounds.iter().filter(|s| lib.sound(s, &mut pcm).is_none()).cloned().collect();
        assert!(bad_scenes.is_empty(), "effects with nothing to show: {bad_scenes:?}");
        assert!(bad_sounds.is_empty(), "sounds that do not decode: {bad_sounds:?}");
        assert!(lib.base_sound("Block2").is_some());
    }

    /// Every ParticleEffect scene in the original loads into something drawable or audible.
    #[test]
    fn all_original_effect_scenes_load() {
        let Some(mut lib) = lib() else { return };
        let dir = lib.dir.clone().expect("dir");
        let mut images = Assets::<Image>::default();
        let mut stack = vec![dir.clone()];
        let (mut total, mut empty) = (0, vec![]);
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "tscn") {
                    let text = std::fs::read_to_string(&p).unwrap_or_default();
                    if !text.contains("res://fx/ParticleEffect.tscn") {
                        continue;
                    }
                    let rel = p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
                    total += 1;
                    if lib.effect(&rel, &mut images).is_none_or(|d| d.parts.is_empty()) {
                        empty.push(rel);
                    }
                }
            }
        }
        eprintln!("loaded {total} effect scenes, {} images", images.len());
        assert!(total >= 100, "found {total}");
        assert!(empty.len() * 20 < total, "too many effects with no parts: {empty:?}");
    }

    /// The default hit spark: an 8-frame sprite and particle bursts that fade out.
    #[test]
    fn hit_spark_simulates() {
        let Some(mut lib) = lib() else { return };
        let mut images = Assets::<Image>::default();
        let def = lib.effect("fx/HitEffect1.tscn", &mut images).expect("hit effect");
        let anim = def.parts.iter().find_map(|p| match &p.kind {
            PartKind::Anim { frames, .. } => Some(frames.len()),
            _ => None,
        });
        assert_eq!(anim, Some(9));
        let (k, em) = def.parts.iter().enumerate().find_map(|(k, p)| match &p.kind {
            PartKind::Emitter(e) => Some((k, e)),
            _ => None,
        }).expect("an emitter");
        assert_eq!(em.amount, 32);
        let mut run = EmitterRun { time: 0.0, particles: (0..em.amount).map(|_| P { pos: Vec2::ZERO, vel: Vec2::ZERO, rot: 0.0, scale: 1.0, color: [1.0; 4], frame: 0, time: 0.0, life: 1.0, active: false, spawned: false, r: [0.5; 8] }).collect(), gravity: em.gravity };
        let world = def.parts[k].xf;
        step_emitter(em, &mut run, world, 1.0 / 60.0, &mut lib);
        assert!(run.particles.iter().all(|p| p.active), "explosiveness 1 emits everything at once");
        for _ in 0..60 {
            step_emitter(em, &mut run, world, 1.0 / 60.0, &mut lib);
        }
        assert!(run.particles.iter().all(|p| !p.active), "one-shot particles are gone after their lifetime");
    }
}
