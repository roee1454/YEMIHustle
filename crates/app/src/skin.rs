//! The menus' look: fonts, images, sounds and a few texts loaded at runtime from a local copy
//! of the original game's files (`reference/yomi_recovered`, the private decompile). Nothing
//! from the original is compiled in or committed; without that folder every asset is `None`
//! and the menus fall back to Bevy's default font, a text title and no icons.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;

/// Images the menus use, by short name -> path inside the original project.
const IMAGES: &[(&str, &str)] = &[
    ("updown", "ui/updown.png"),
    ("check_off", "ui/checkbutton1.png"),
    ("check_on", "ui/checkbutton2.png"),
    ("check_off_disabled", "ui/checkbutton3.png"),
    ("check_on_disabled", "ui/checkbutton4.png"),
    ("grabber", "ui/HorizSlider/grabber.png"),
    ("discord", "ui/discord.png"),
    ("twitter", "ui/twitter.png"),
    ("tiktok", "ui/tiktok.png"),
    ("event", "ui/steed.png"),
    ("portrait_ninja", "characters/stickman/sprites/idle.png"),
    ("portrait_robo", "characters/robo/sprites/idle.png"),
    ("portrait_wizard", "characters/wizard/sprites/idle.png"),
    ("portrait_mutant", "characters/mutant/sprites/idle4.png"),
    ("portrait_cowboy", "characters/swordandgun/sprites/idle.png"),
];

#[derive(Resource, Default)]
pub struct Skin {
    pub font: Option<Handle<Font>>,
    pub font_bold: Option<Handle<Font>>,
    images: HashMap<&'static str, Handle<Image>>,
    /// Images added at run time (imported fighters' portraits).
    extra: HashMap<String, Handle<Image>>,
    pub hover: Option<Handle<crate::fx::Pcm>>,
    pub select: Option<Handle<crate::fx::Pcm>>,
    /// The roadmap panel's three paragraphs (intro, complete, incomplete).
    pub roadmap: Vec<String>,
    pub source: Option<PathBuf>,
}

impl Skin {
    pub fn image(&self, name: &str) -> Option<Handle<Image>> {
        self.images.get(name).cloned().or_else(|| self.extra.get(name).cloned())
    }

    pub fn add_image(&mut self, name: String, image: Handle<Image>) {
        self.extra.insert(name, image);
    }

    pub fn load(dir: &Path, fonts: &mut Assets<Font>, images: &mut Assets<Image>, audio: &mut Assets<crate::fx::Pcm>) -> Self {
        let mut skin = Skin::default();
        if !dir.join("project.godot").exists() {
            return skin;
        }
        skin.source = Some(dir.to_path_buf());
        if let Ok(bytes) = std::fs::read(dir.join("ui/PixeloidSans.ttf")) {
            skin.font = Some(fonts.add(Font::from_bytes(bytes)));
        }
        if let Ok(bytes) = std::fs::read(dir.join("ui/PixeloidSans-Bold.ttf")) {
            skin.font_bold = Some(fonts.add(Font::from_bytes(bytes)));
        }
        for (name, rel) in IMAGES {
            let Ok(bytes) = std::fs::read(dir.join(rel)) else { continue };
            match Image::from_buffer(
                &bytes,
                ImageType::Extension(crate::gamefiles::sniff_ext(&bytes, rel)),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::nearest(),
                RenderAssetUsages::default(),
            ) {
                Ok(img) => {
                    skin.images.insert(name, images.add(img));
                }
                Err(e) => eprintln!("skin: could not decode {rel}: {e}"),
            }
        }
        for (field, rel) in [("hover", "sound/ui/button_hover3.wav"), ("select", "sound/ui/button_select6.wav")] {
            let Some(pcm) = std::fs::read(dir.join(rel)).ok().and_then(|b| crate::fx::decode_wav(&b)) else { continue };
            let h = audio.add(pcm);
            if field == "hover" {
                skin.hover = Some(h);
            } else {
                skin.select = Some(h);
            }
        }
        if let Ok(main) = std::fs::read_to_string(dir.join("Main.tscn")) {
            for node in ["Complete2", "Complete", "Incomplete"] {
                if let Some(t) = node_text(&main, node) {
                    skin.roadmap.push(t);
                }
            }
        }
        skin
    }
}

/// The `text = "..."` property of the first node with this name in a Godot scene.
fn node_text(scene: &str, node: &str) -> Option<String> {
    let start = scene.find(&format!("[node name=\"{node}\""))?;
    let rest = &scene[start..];
    let end = rest[1..].find("\n[").map(|i| i + 1).unwrap_or(rest.len());
    let block = &rest[..end];
    let t = block.find("\ntext = \"")? + "\ntext = \"".len();
    let body = &block[t..];
    let close = body.find("\"\n")?;
    Some(body[..close].replace("\\\"", "\"").trim_end().to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_multiline_node_text() {
        let scene = "[node name=\"A\" type=\"Label\"]\ntext = \"line one\nline two\"\nautowrap = true\n\n[node name=\"B\"]\n";
        assert_eq!(super::node_text(scene, "A").as_deref(), Some("line one\nline two"));
        assert_eq!(super::node_text(scene, "B"), None);
    }
}
