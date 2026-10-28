//! Character styles, as in the original's Customize screen: the color the sprite's white parts
//! take, an optional outline, the hit spark, and up to three particle auras. Styles are saved as
//! RON files next to the settings; a few presets are built in.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The original's palette of simple colors (its `simple_colors`), as RGB.
pub const PALETTE: [[u8; 3]; 12] = [
    [0x94, 0xe4, 0xff], [0xff, 0xc1, 0xa1], [0xec, 0xff, 0xa4], [0xfe, 0xc2, 0xff], [0x6e, 0x86, 0x96], [0xff, 0xea, 0x5d],
    [0x04, 0x57, 0x9a], [0x85, 0x00, 0x1f], [0x00, 0x85, 0x61], [0x9f, 0x42, 0xba], [0x34, 0x35, 0x37], [0xff, 0x94, 0x44],
];

/// Hit sparks a style can pick: name and the original's effect scene.
pub const HITSPARKS: [(&str, &str); 10] = [
    ("bash", "fx/HitEffect1.tscn"),
    ("bash2", "fx/hitsparks/HitEffect1Alt.tscn"),
    ("bash3", "fx/hitsparks/HitEffect2Alt.tscn"),
    ("fire", "fx/hitsparks/FireHitEffect.tscn"),
    ("hearts", "fx/hitsparks/HeartHitEffect.tscn"),
    ("petals", "fx/hitsparks/PetalHitEffect.tscn"),
    ("coins", "fx/hitsparks/CoinHitEffect.tscn"),
    ("dust", "fx/hitsparks/DustHitEffect.tscn"),
    ("acid", "fx/hitsparks/AcidHitEffect.tscn"),
    ("elec", "fx/hitsparks/ElectricHitEffect.tscn"),
];

pub fn hitspark_scene(name: &str) -> &'static str {
    HITSPARKS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s).unwrap_or(HITSPARKS[0].1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Shape {
    #[default]
    Circle,
    Square,
    Spark,
    Star,
    Ring,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Circle, Shape::Square, Shape::Spark, Shape::Star, Shape::Ring];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Circle => "circle",
            Shape::Square => "square",
            Shape::Spark => "spark",
            Shape::Star => "star",
            Shape::Ring => "ring",
        }
    }
}

/// Where on the fighter an aura's particles start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Attach {
    #[default]
    Body,
    Head,
    Feet,
    Front,
    Back,
}

impl Attach {
    pub const ALL: [Attach; 5] = [Attach::Body, Attach::Head, Attach::Feet, Attach::Front, Attach::Back];

    pub fn label(self) -> &'static str {
        match self {
            Attach::Body => "body",
            Attach::Head => "head",
            Attach::Feet => "feet",
            Attach::Front => "front",
            Attach::Back => "back",
        }
    }
}

/// A particle aura (the original's trail settings, simplified).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Aura {
    pub show: bool,
    pub shape: Shape,
    pub attach: Attach,
    /// RGBA at birth and at death.
    pub start: [u8; 4],
    pub end: [u8; 4],
    /// Particles per second.
    pub amount: f32,
    /// Ticks a particle lives.
    pub lifetime: f32,
    /// Size in game pixels at birth and at death.
    pub size: f32,
    pub end_size: f32,
    /// Speed in game pixels per tick, its direction in degrees (0 up, 90 forward), and the
    /// spread around it.
    pub speed: f32,
    pub direction: f32,
    pub spread: f32,
    /// Pull per tick (positive is down).
    pub gravity: f32,
    /// How much of the fighter's movement particles keep (0..1).
    pub inherit: f32,
    /// Drawn in front of the fighter instead of behind.
    pub front: bool,
}

impl Default for Aura {
    fn default() -> Self {
        Aura {
            show: false,
            shape: Shape::Circle,
            attach: Attach::Body,
            start: [255, 255, 255, 200],
            end: [255, 255, 255, 0],
            amount: 30.0,
            lifetime: 40.0,
            size: 3.0,
            end_size: 1.0,
            speed: 0.6,
            direction: 0.0,
            spread: 60.0,
            gravity: 0.0,
            inherit: 0.3,
            front: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub name: String,
    /// What the sprite's pure-white pixels become.
    pub color: [u8; 3],
    pub outline: Option<[u8; 3]>,
    pub hitspark: String,
    pub auras: [Aura; 3],
    /// Built into the game (not saved, not deletable).
    #[serde(skip)]
    pub preset: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style { name: "untitled".into(), color: P1_COLOR, outline: None, hitspark: "bash".into(), auras: Default::default(), preset: false }
    }
}

pub const P1_COLOR: [u8; 3] = [140, 209, 255];
pub const P2_COLOR: [u8; 3] = [255, 173, 128];

/// A player's style when none is picked: the fight's usual blue or orange.
pub fn default_for(player: usize) -> Style {
    Style { name: String::new(), color: if player == 0 { P1_COLOR } else { P2_COLOR }, preset: true, ..Style::default() }
}

fn aura(shape: Shape, attach: Attach, start: [u8; 4], end: [u8; 4], amount: f32, lifetime: f32, size: f32, speed: f32, direction: f32, spread: f32, gravity: f32) -> Aura {
    Aura { show: true, shape, attach, start, end, amount, lifetime, size, end_size: size * 0.25, speed, direction, spread, gravity, ..Aura::default() }
}

/// Styles that come with the game.
pub fn presets() -> Vec<Style> {
    let p = |name: &str, color: [u8; 3], outline: Option<[u8; 3]>, hitspark: &str, auras: [Aura; 3]| Style { name: name.into(), color, outline, hitspark: hitspark.into(), auras, preset: true };
    let off = Aura::default;
    vec![
        p("Inferno", [255, 148, 68], Some([133, 0, 31]), "fire", [
            aura(Shape::Circle, Attach::Body, [255, 210, 90, 230], [200, 30, 0, 0], 55.0, 32.0, 4.0, 0.9, 0.0, 40.0, -0.03),
            aura(Shape::Spark, Attach::Feet, [255, 240, 160, 255], [255, 90, 0, 0], 18.0, 20.0, 2.0, 1.6, 0.0, 70.0, 0.0),
            off(),
        ]),
        p("Frost", [148, 228, 255], Some([4, 87, 154]), "elec", [
            aura(Shape::Star, Attach::Body, [230, 250, 255, 240], [120, 200, 255, 0], 22.0, 50.0, 3.0, 0.3, 0.0, 180.0, -0.01),
            aura(Shape::Circle, Attach::Feet, [180, 235, 255, 140], [180, 235, 255, 0], 25.0, 30.0, 5.0, 0.4, 90.0, 120.0, 0.0),
            off(),
        ]),
        p("Void", [159, 66, 186], Some([20, 10, 30]), "acid", [
            aura(Shape::Ring, Attach::Body, [200, 120, 255, 200], [40, 0, 80, 0], 16.0, 45.0, 6.0, 0.2, 0.0, 360.0, 0.0),
            aura(Shape::Square, Attach::Back, [90, 30, 140, 220], [0, 0, 0, 0], 30.0, 26.0, 3.0, 1.0, -90.0, 30.0, 0.0),
            off(),
        ]),
        p("Golden", [255, 234, 93], Some([90, 60, 0]), "coins", [
            aura(Shape::Spark, Attach::Body, [255, 255, 200, 255], [255, 200, 0, 0], 26.0, 24.0, 2.5, 1.2, 0.0, 360.0, 0.04),
            off(),
            off(),
        ]),
        p("Sakura", [254, 194, 255], None, "petals", [
            aura(Shape::Circle, Attach::Head, [255, 200, 230, 230], [255, 140, 200, 0], 12.0, 70.0, 2.5, 0.5, 120.0, 60.0, 0.015),
            off(),
            off(),
        ]),
        p("Toxic", [236, 255, 164], Some([0, 133, 97]), "acid", [
            aura(Shape::Circle, Attach::Feet, [150, 255, 90, 200], [0, 120, 40, 0], 30.0, 40.0, 3.5, 0.5, 0.0, 50.0, -0.02),
            off(),
            off(),
        ]),
        p("Shadow", [52, 53, 55], Some([255, 255, 255]), "dust", [
            aura(Shape::Square, Attach::Body, [30, 30, 40, 200], [0, 0, 0, 0], 40.0, 22.0, 3.0, 0.4, 0.0, 360.0, -0.02),
            off(),
            off(),
        ]),
        p("Lover", [255, 193, 161], Some([133, 0, 31]), "hearts", [
            aura(Shape::Star, Attach::Head, [255, 120, 160, 255], [255, 60, 120, 0], 6.0, 50.0, 3.0, 0.4, 0.0, 50.0, -0.01),
            off(),
            off(),
        ]),
    ]
}

fn styles_dir() -> Option<PathBuf> {
    crate::settings::Settings::path().and_then(|p| p.parent().map(|d| d.join("styles")))
}

/// Every style the player can pick: the presets, then their own.
#[derive(bevy::prelude::Resource, Clone)]
pub struct Styles {
    pub list: Vec<Style>,
}

/// A name usable as a file name.
fn file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == ' ' { c } else { '_' }).collect();
    if s.trim().is_empty() { "untitled".into() } else { s.trim().to_string() }
}

impl Styles {
    pub fn load() -> Styles {
        let mut list = presets();
        if let Some(dir) = styles_dir() {
            let mut own: Vec<Style> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "ron"))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|t| ron::from_str::<Style>(&t).ok())
                .collect();
            own.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            list.extend(own);
        }
        Styles { list }
    }

    pub fn get(&self, name: &str) -> Option<&Style> {
        self.list.iter().find(|s| s.name == name)
    }

    /// The style a player fights in.
    pub fn for_player(&self, player: usize, name: &str) -> Style {
        self.get(name).cloned().unwrap_or_else(|| default_for(player))
    }

    /// Saves a player style (replacing one of the same name) and lists it.
    pub fn save(&mut self, style: &Style) -> Result<(), String> {
        let mut style = style.clone();
        style.preset = false;
        if self.list.iter().any(|s| s.preset && s.name == style.name) {
            style.name = format!("{} (mine)", style.name);
        }
        let dir = styles_dir().ok_or("no config folder")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = ron::ser::to_string_pretty(&style, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{}.ron", file_name(&style.name))), text).map_err(|e| e.to_string())?;
        match self.list.iter_mut().find(|s| s.name == style.name) {
            Some(slot) => *slot = style,
            None => self.list.push(style),
        }
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), String> {
        if self.get(name).is_none_or(|s| s.preset) {
            return Err("built-in styles cannot be deleted".into());
        }
        if let Some(dir) = styles_dir() {
            let _ = std::fs::remove_file(dir.join(format!("{}.ron", file_name(name))));
        }
        self.list.retain(|s| s.name != name);
        Ok(())
    }

    /// The style after (or before) `name` in the list, "" (the default) included.
    pub fn cycle(&self, name: &str, d: i32) -> String {
        let mut names: Vec<&str> = vec![""];
        names.extend(self.list.iter().map(|s| s.name.as_str()));
        let i = names.iter().position(|n| *n == name).unwrap_or(0) as i32;
        names[(i + d).rem_euclid(names.len() as i32) as usize].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_round_trip_and_cycle() {
        let s = presets().remove(0);
        let text = ron::ser::to_string_pretty(&s, ron::ser::PrettyConfig::default()).unwrap();
        let back: Style = ron::from_str(&text).unwrap();
        assert_eq!(back.name, s.name);
        assert_eq!(back.auras, s.auras);
        let all = Styles { list: presets() };
        assert_eq!(all.cycle("", 1), "Inferno");
        assert_eq!(all.cycle("Inferno", -1), "");
        assert_eq!(all.cycle("", -1), all.list.last().unwrap().name);
        assert_eq!(hitspark_scene("fire"), "fx/hitsparks/FireHitEffect.tscn");
        assert_eq!(hitspark_scene("nope"), "fx/HitEffect1.tscn");
    }
}
