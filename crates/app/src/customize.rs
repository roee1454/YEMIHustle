//! The Customize screen: make and save character styles (see `style.rs`). The left panel lists
//! styles, the right one edits the loaded style (color, outline, hit spark, three auras), and
//! a fighter in the middle shows it live, auras included.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::menu::{button, check_row, slider, Clickable, Effect, Id, Menu, Page, Ui};
use crate::pixel::*;
use crate::style::{Attach, Shape, Style, Styles, HITSPARKS, PALETTE};

/// The style being edited and the screen's own state.
#[derive(Clone)]
pub struct Edit {
    pub style: Style,
    /// The aura slot shown (0..3).
    pub slot: usize,
    /// The roster index of the fighter in the preview.
    pub preview: usize,
}

impl Default for Edit {
    fn default() -> Self {
        // start from a preset with auras, so there is something to look at and change
        let mut style = crate::style::presets().remove(0);
        style.preset = false;
        Edit { style, slot: 0, preview: 0 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CustId {
    Item(usize),
    New,
    Delete,
    Save,
    Name,
    Prev,
    Next,
    Color(u8),
    Outline(u8),
    NoOutline,
    Spark,
    Slot(u8),
    Show,
    Shape,
    Attach,
    Front,
    Start(u8),
    End(u8),
    Slider(u8),
}

/// The screen's sliders: label, range, and which value they edit.
const SLIDERS: [(&str, f32, f32); 13] = [
    ("hue", 0.0, 360.0),
    ("saturation", 0.0, 1.0),
    ("value", 0.0, 1.0),
    ("amount", 0.0, 120.0),
    ("lifetime", 5.0, 120.0),
    ("size", 0.5, 10.0),
    ("end size", 0.0, 10.0),
    ("speed", 0.0, 4.0),
    ("direction", -180.0, 180.0),
    ("spread", 0.0, 360.0),
    ("gravity", -0.1, 0.1),
    ("start alpha", 0.0, 255.0),
    ("end alpha", 0.0, 255.0),
];

fn hsv_of(c: [u8; 3]) -> (f32, f32, f32) {
    let h = Hsva::from(Color::srgb_u8(c[0], c[1], c[2]));
    (h.hue, h.saturation, h.value)
}

fn rgb_of(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = Color::hsv(h, s, v).to_srgba();
    [(c.red * 255.0).round() as u8, (c.green * 255.0).round() as u8, (c.blue * 255.0).round() as u8]
}

/// A slider's position (0..1).
fn slider_t(e: &Edit, k: usize) -> f32 {
    let a = &e.style.auras[e.slot];
    let (h, s, v) = hsv_of(e.style.color);
    let raw = match k {
        0 => h,
        1 => s,
        2 => v,
        3 => a.amount,
        4 => a.lifetime,
        5 => a.size,
        6 => a.end_size,
        7 => a.speed,
        8 => a.direction,
        9 => a.spread,
        10 => a.gravity,
        11 => a.start[3] as f32,
        _ => a.end[3] as f32,
    };
    let (_, lo, hi) = SLIDERS[k];
    ((raw - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn set_slider(e: &mut Edit, k: usize, t: f32) {
    let (_, lo, hi) = SLIDERS[k];
    let x = lo + t.clamp(0.0, 1.0) * (hi - lo);
    let (h, s, v) = hsv_of(e.style.color);
    let slot = e.slot;
    let a = &mut e.style.auras[slot];
    match k {
        0 => e.style.color = rgb_of(x, s.max(0.05), v.max(0.05)),
        1 => e.style.color = rgb_of(h, x, v.max(0.05)),
        2 => e.style.color = rgb_of(h, s, x),
        3 => a.amount = x.round(),
        4 => a.lifetime = x.round(),
        5 => a.size = (x * 2.0).round() / 2.0,
        6 => a.end_size = (x * 2.0).round() / 2.0,
        7 => a.speed = (x * 20.0).round() / 20.0,
        8 => a.direction = (x / 5.0).round() * 5.0,
        9 => a.spread = (x / 5.0).round() * 5.0,
        10 => a.gravity = (x * 1000.0).round() / 1000.0,
        11 => a.start[3] = x.round() as u8,
        _ => a.end[3] = x.round() as u8,
    }
}

fn slider_text(e: &Edit, k: usize) -> String {
    let (_, lo, hi) = SLIDERS[k];
    let x = lo + slider_t(e, k) * (hi - lo);
    if hi - lo <= 1.0 { format!("{x:.2}") } else if hi - lo <= 10.0 { format!("{x:.1}") } else { format!("{}", x.round()) }
}

/// Drags and clicks on the screen's sliders.
pub fn drag(menu: &mut Menu, k: u8, t: f32) {
    set_slider(&mut menu.cust, k as usize, t);
    menu.dirty = true;
}

pub fn wants_cursor(c: CustId) -> bool {
    matches!(c, CustId::Slider(_))
}

fn cycle<T: Copy + PartialEq>(all: &[T], cur: T, d: i32) -> T {
    let i = all.iter().position(|x| *x == cur).unwrap_or(0) as i32;
    all[(i + d).rem_euclid(all.len() as i32) as usize]
}

/// Acts on the screen's widgets.
pub fn activate(menu: &mut Menu, styles: &mut Styles, roster_len: usize, c: CustId, press: bool, step: i32, click: Option<f32>) -> Effect {
    let d = if press { 1 } else { step };
    let e = &mut menu.cust;
    let slot = e.slot;
    match c {
        CustId::Item(i) if press => {
            if let Some(s) = styles.list.get(i) {
                e.style = s.clone();
                e.style.preset = false;
            }
        }
        CustId::New if press => {
            e.style = Style { name: format!("style {}", styles.list.iter().filter(|s| !s.preset).count() + 1), ..Style::default() };
        }
        CustId::Delete if press => {
            let name = e.style.name.clone();
            menu.status = match styles.delete(&name) {
                Ok(()) => format!("deleted {name}"),
                Err(err) => err,
            };
        }
        CustId::Save if press => {
            if e.style.name.trim().is_empty() {
                e.style.name = "untitled".into();
            }
            let st = e.style.clone();
            menu.status = match styles.save(&st) {
                Ok(()) => format!("saved {}", st.name),
                Err(err) => format!("could not save: {err}"),
            };
        }
        CustId::Name if press => menu.editing = Some(Id::Cust(CustId::Name)),
        CustId::Prev if press => e.preview = (e.preview + roster_len.max(1) - 1) % roster_len.max(1),
        CustId::Next if press => e.preview = (e.preview + 1) % roster_len.max(1),
        CustId::Color(i) if press => e.style.color = PALETTE[i as usize],
        CustId::Outline(i) if press => e.style.outline = Some(PALETTE[i as usize]),
        CustId::NoOutline if press => e.style.outline = None,
        CustId::Spark => {
            let names: Vec<&str> = HITSPARKS.iter().map(|(n, _)| *n).collect();
            let cur = names.iter().position(|n| *n == e.style.hitspark).unwrap_or(0) as i32;
            e.style.hitspark = names[(cur + d).rem_euclid(names.len() as i32) as usize].to_string();
        }
        CustId::Slot(i) if press => e.slot = i as usize,
        CustId::Show => e.style.auras[slot].show = !e.style.auras[slot].show,
        CustId::Shape => e.style.auras[slot].shape = cycle(&Shape::ALL, e.style.auras[slot].shape, d),
        CustId::Attach => e.style.auras[slot].attach = cycle(&Attach::ALL, e.style.auras[slot].attach, d),
        CustId::Front => e.style.auras[slot].front = !e.style.auras[slot].front,
        CustId::Start(i) if press => {
            let c = PALETTE[i as usize];
            let a = &mut e.style.auras[slot];
            a.start = [c[0], c[1], c[2], a.start[3]];
        }
        CustId::End(i) if press => {
            let c = PALETTE[i as usize];
            let a = &mut e.style.auras[slot];
            a.end = [c[0], c[1], c[2], a.end[3]];
        }
        CustId::Slider(k) => match click {
            Some(t) => set_slider(e, k as usize, t),
            None => {
                let t = slider_t(e, k as usize) + step as f32 * 0.02;
                set_slider(e, k as usize, t);
            }
        },
        _ => {}
    }
    menu.dirty = true;
    Effect::None
}

fn swatch(p: &mut ChildSpawnerCommands, ui: &mut Ui, id: Id, x: f32, y: f32, c: [u8; 3], on: bool) {
    ui.order.push(id);
    let hov = ui.menu.hovered == Some(id) || ui.menu.focus == Some(id);
    if on || hov {
        p.spawn((at(x - 1.0, y - 1.0, 16.0, 12.0), BackgroundColor(if on { Color::WHITE } else { Color::srgb(0.6, 0.6, 0.6) })));
    }
    p.spawn((Button, Clickable(id), at(x, y, 14.0, 10.0), BackgroundColor(Color::srgb_u8(c[0], c[1], c[2]))));
}


pub fn build(p: &mut ChildSpawnerCommands, ui: &mut Ui, styles: &Styles, roster: &[(String, String)]) {
    let skin = ui.skin;
    let e = ui.menu.cust.clone();
    let gray = Color::srgb_u8(0x8c, 0x8c, 0x8c);

    // left: the styles
    p.spawn((at(8.0, 8.0, 150.0, 344.0), BackgroundColor(BLACK)));
    label(p, skin, 8.0, 9.0, 150.0, 11.0, "Styles", FONT, Align::Center);
    for (i, s) in styles.list.iter().enumerate().take(22) {
        let tag = if s.preset { "" } else { " *" };
        let on = s.name == e.style.name;
        let y = 22.0 + 12.5 * i as f32;
        p.spawn((at(12.0, y + 2.0, 8.0, 8.0), BackgroundColor(Color::srgb_u8(s.color[0], s.color[1], s.color[2]))));
        button(p, ui, Id::Cust(CustId::Item(i)), 22.0, y, 132.0, 12.0, &format!("{}{tag}", s.name), on, false);
    }
    button(p, ui, Id::Cust(CustId::New), 12.0, 312.0, 70.0, 12.0, "new", false, false);
    button(p, ui, Id::Cust(CustId::Delete), 84.0, 312.0, 70.0, 12.0, "delete", false, e.style.preset || styles.get(&e.style.name).is_none_or(|s| s.preset));
    button(p, ui, Id::Back, 12.0, 336.0, 142.0, 12.0, "back", false, false);

    // middle: name, save, the preview fighter (drawn in the world behind an open area)
    let editing = ui.menu.editing == Some(Id::Cust(CustId::Name));
    ui.order.push(Id::Cust(CustId::Name));
    let name = if editing { format!("{}|", e.style.name) } else if e.style.name.is_empty() { "style name".into() } else { e.style.name.clone() };
    p.spawn((Button, Clickable(Id::Cust(CustId::Name)), sb(if editing { LINE_EDIT_FOCUS } else { LINE_EDIT }, Node { align_items: AlignItems::Center, padding: UiRect::left(px(2)), ..at(170.0, 10.0, 180.0, 12.0) })))
        .with_children(|l| text(l, skin, &name, FONT, None));
    button(p, ui, Id::Cust(CustId::Save), 352.0, 10.0, 44.0, 12.0, "save", false, false);
    let who = roster.get(e.preview).map(|r| r.0.clone()).unwrap_or_default();
    button(p, ui, Id::Cust(CustId::Prev), 186.0, 320.0, 20.0, 12.0, "<", false, false);
    label(p, skin, 206.0, 321.0, 154.0, 11.0, &who, FONT, Align::Center);
    button(p, ui, Id::Cust(CustId::Next), 360.0, 320.0, 20.0, 12.0, ">", false, false);
    if !ui.menu.status.is_empty() {
        label(p, skin, 160.0, 338.0, 244.0, 11.0, &ui.menu.status, FONT, Align::Center);
    }

    // right: the editors
    let (x0, w) = (404.0, 228.0);
    p.spawn((at(x0, 8.0, w, 344.0), BackgroundColor(BLACK)));
    let x = x0 + 4.0;
    label(p, skin, x, 9.0, 60.0, 11.0, "color", gray, Align::Left);
    for (i, c) in PALETTE.iter().enumerate() {
        swatch(p, ui, Id::Cust(CustId::Color(i as u8)), x + 2.0 + 18.0 * i as f32, 22.0, *c, e.style.color == *c);
    }
    let slider_row = |p: &mut ChildSpawnerCommands, ui: &mut Ui, k: usize, y: f32| {
        let skin = ui.skin;
        label(p, skin, x, y, 56.0, 11.0, SLIDERS[k].0, FONT, Align::Left);
        slider(p, ui, Id::Cust(CustId::Slider(k as u8)), slider_t(&ui.menu.cust, k), at(x + 58.0, y, 132.0, 12.0));
        label(p, skin, x + 192.0, y, 30.0, 11.0, &slider_text(&ui.menu.cust, k), FONT, Align::Left);
    };
    for k in 0..3 {
        slider_row(p, ui, k, 35.0 + 12.5 * k as f32);
    }
    label(p, skin, x, 74.0, 60.0, 11.0, "outline", gray, Align::Left);
    button(p, ui, Id::Cust(CustId::NoOutline), x + 160.0, 74.0, 58.0, 11.0, "none", e.style.outline.is_none(), false);
    for (i, c) in PALETTE.iter().enumerate() {
        swatch(p, ui, Id::Cust(CustId::Outline(i as u8)), x + 2.0 + 18.0 * i as f32, 87.0, *c, e.style.outline == Some(*c));
    }
    label(p, skin, x, 101.0, 60.0, 11.0, "hit spark", gray, Align::Left);
    button(p, ui, Id::Cust(CustId::Spark), x + 100.0, 101.0, 118.0, 11.0, &e.style.hitspark, false, false);

    // aura slots
    label(p, skin, x, 117.0, 40.0, 11.0, "aura", gray, Align::Left);
    for i in 0..3u8 {
        let on = e.slot == i as usize;
        let shown = e.style.auras[i as usize].show;
        button(p, ui, Id::Cust(CustId::Slot(i)), x + 40.0 + 30.0 * i as f32, 117.0, 28.0, 11.0, &format!("{}{}", i + 1, if shown { "*" } else { "" }), on, false);
    }
    let a = e.style.auras[e.slot].clone();
    check_row(p, ui, Id::Cust(CustId::Show), x, 130.0, 106.0, "show", a.show, FONT);
    check_row(p, ui, Id::Cust(CustId::Front), x + 112.0, 130.0, 108.0, "in front", a.front, FONT);
    button(p, ui, Id::Cust(CustId::Shape), x, 143.0, 106.0, 11.0, &format!("shape: {}", a.shape.label()), false, false);
    button(p, ui, Id::Cust(CustId::Attach), x + 112.0, 143.0, 108.0, 11.0, &format!("from: {}", a.attach.label()), false, false);
    label(p, skin, x, 156.0, 80.0, 11.0, "start color", gray, Align::Left);
    for (i, c) in PALETTE.iter().enumerate() {
        swatch(p, ui, Id::Cust(CustId::Start(i as u8)), x + 2.0 + 18.0 * i as f32, 168.0, *c, a.start[..3] == c[..]);
    }
    label(p, skin, x, 180.0, 80.0, 11.0, "end color", gray, Align::Left);
    for (i, c) in PALETTE.iter().enumerate() {
        swatch(p, ui, Id::Cust(CustId::End(i as u8)), x + 2.0 + 18.0 * i as f32, 192.0, *c, a.end[..3] == c[..]);
    }
    for k in 3..SLIDERS.len() {
        slider_row(p, ui, k, 205.0 + 12.5 * (k - 3) as f32);
    }
}

/// The preview fighter's sprite.
#[derive(Component)]
pub struct PreviewSprite;

/// Shows the fighter of the Customize screen in the style being edited, where the screen
/// leaves room for it, and tells the auras where it stands.
#[allow(clippy::too_many_arguments)]
pub fn preview(
    mut commands: Commands,
    menu: Res<Menu>,
    roster: Res<crate::Roster>,
    windows: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut frame: ResMut<crate::aura::PreviewFrame>,
    mut sprites: Query<(Entity, &mut Sprite, &mut Transform), With<PreviewSprite>>,
    mut lib: ResMut<crate::fx::FxLibrary>,
    mut art: ResMut<crate::art::Art>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time>,
) {
    let open = menu.page() == Some(Page::Customize);
    frame.shown = open;
    if !open {
        for (e, ..) in &sprites {
            commands.entity(e).despawn();
        }
        return;
    }
    let Ok(_) = windows.single() else { return };
    let s = scale.0;
    // the fighter's feet at canvas (280, 296)
    frame.origin = Vec2::new((280.0 - VW / 2.0) * s, (VH / 2.0 - 296.0) * s);
    frame.zoom = s * 2.2;
    let all = crate::menu::choices(&roster);
    let Some((_, id)) = all.get(menu.cust.preview) else { return };
    let dirs = lib.dirs();
    let Some(ca) = art.character(&dirs, id) else { return };
    let offset = ca.offset;
    let tick = (time.elapsed_secs() * 15.0) as u32;
    let Some(path) = crate::art::idle_frame(ca, tick).map(str::to_string) else { return };
    let st = &menu.cust.style;
    let Some((image, _)) = lib.texture_styled(&path, st.color, st.outline, &mut images) else { return };
    let tf = Transform::from_translation((frame.origin + Vec2::new(offset.x, -offset.y) * frame.zoom).extend(0.5)).with_scale(Vec3::splat(frame.zoom));
    match sprites.single_mut() {
        Ok((_, mut sprite, mut t)) => {
            sprite.image = image;
            *t = tf;
        }
        Err(_) => {
            commands.spawn((PreviewSprite, Sprite { image, ..default() }, tf));
        }
    }
}

/// The preview fighter's auras (fixed tick).
pub fn preview_auras(mut commands: Commands, menu: Res<Menu>, shapes: Option<Res<crate::aura::AuraShapes>>, mut emitters: ResMut<crate::aura::Emitters>) {
    let Some(shapes) = shapes else { return };
    if menu.page() != Some(Page::Customize) {
        return;
    }
    let body = crate::aura::Body { feet: Vec2::ZERO, vel: Vec2::ZERO, facing: 1.0, center: Vec2::new(0.0, -17.0), half: Vec2::new(9.0, 17.0) };
    emitters.emit(&mut commands, &shapes, 2, &menu.cust.style, &body, crate::aura::Space::Preview, None);
}
