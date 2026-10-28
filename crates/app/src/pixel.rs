//! A small toolkit for laying out menus the way the original does: a 640x360 canvas scaled to
//! the window (Godot's "viewport" stretch mode with "keep" aspect), a 9px pixel font with a
//! 1px black outline, and the original theme's flat style boxes.

use bevy::mesh::{Mesh2d, MeshBuilder};
use bevy::prelude::*;
use bevy::text::{FontSmoothing, LineHeight};
use bevy::window::PrimaryWindow;

use crate::skin::Skin;

pub const VW: f32 = 640.0;
pub const VH: f32 = 360.0;

pub const FONT: Color = Color::srgb(0.878, 0.878, 0.878);
/// Godot's disabled text (0.9 grey at 20% alpha) as it ends up on screen; Bevy blends alpha
/// in linear space, so the blended result is spelled out.
pub const FONT_DISABLED: Color = Color::srgb_u8(0x2d, 0x2d, 0x2d);
pub const RED: Color = Color::srgb(1.0, 0.2, 0.239);
/// The credit link under the logo (the theme's accent; red since the new logo).
pub const LINK_BLUE: Color = Color::srgb(1.0, 0.33, 0.33);
pub const CLEAR: Color = Color::srgb_u8(0x0a, 0x0b, 0x0e);
pub const BLACK: Color = Color::BLACK;

/// The menus' canvas; kept centred and scaled by `fit_canvas`.
#[derive(Component)]
pub struct Canvas;

/// Scales the UI so 640x360 fills the window, letterboxed, and centres every canvas.
pub fn fit_canvas(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut scale: ResMut<UiScale>,
    mut canvases: Query<&mut Node, With<Canvas>>,
) {
    let Ok(w) = windows.single() else { return };
    let (ww, wh) = (w.width(), w.height());
    if ww <= 0.0 || wh <= 0.0 {
        return;
    }
    let s = (ww / VW).min(wh / VH);
    if scale.0 != s {
        scale.0 = s;
    }
    let (left, top) = (px((ww / s - VW) / 2.0), px((wh / s - VH) / 2.0));
    for mut n in &mut canvases {
        if n.left != left || n.top != top {
            n.left = left;
            n.top = top;
        }
    }
}

pub fn canvas_node() -> Node {
    Node { position_type: PositionType::Absolute, width: px(VW), height: px(VH), ..default() }
}

/// An absolutely positioned box in canvas pixels.
pub fn at(x: f32, y: f32, w: f32, h: f32) -> Node {
    Node { position_type: PositionType::Absolute, left: px(x), top: px(y), width: px(w), height: px(h), ..default() }
}

// ---------------------------------------------------------------------------------------------
// Style boxes (the original theme's StyleBoxFlat values)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Sb {
    pub bg: Color,
    /// Border widths: left, top, right, bottom.
    pub b: [f32; 4],
    pub bc: Color,
}

const fn g(v: u8) -> Color {
    Color::srgb_u8(v, v, v)
}

pub const BTN_NORMAL: Sb = Sb { bg: g(20), b: [1.0, 0.0, 1.0, 1.0], bc: BLACK };
pub const BTN_HOVER: Sb = Sb { bg: g(42), b: [1.0, 0.0, 1.0, 1.0], bc: BLACK };
pub const BTN_PRESSED: Sb = Sb { bg: BLACK, b: [0.0, 1.0, 1.0, 1.0], bc: g(20) };
pub const BTN_DISABLED: Sb = Sb { bg: g(9), b: [1.0, 0.0, 1.0, 1.0], bc: BLACK };
pub const LINE_EDIT: Sb = Sb { bg: g(43), b: [1.0, 0.0, 1.0, 0.0], bc: BLACK };
pub const LINE_EDIT_FOCUS: Sb = Sb { bg: g(60), b: [1.0, 0.0, 1.0, 0.0], bc: BLACK };
pub const PANEL: Sb = Sb { bg: g(20), b: [0.0; 4], bc: BLACK };
pub const TAB_BG: Sb = Sb { bg: g(13), b: [2.0, 0.0, 0.0, 0.0], bc: Color::NONE };
pub const TAB_FG: Sb = Sb { bg: g(70), b: [2.0, 0.0, 0.0, 0.0], bc: Color::NONE };
pub const SCROLL: Sb = Sb { bg: g(20), b: [0.0; 4], bc: BLACK };
pub const GRABBER: Color = g(120);
pub const SEPARATOR: Color = Color::srgb(0.565, 0.604, 0.616);
/// The "format" option button on the character select uses the blue challenge style.
pub const BLUE_BUTTON: Sb = Sb { bg: Color::srgb(0.604, 0.0157, 0.07), b: [1.0, 0.0, 1.0, 1.0], bc: BLACK };

pub fn sb(s: Sb, node: Node) -> impl Bundle {
    let border = UiRect { left: px(s.b[0]), top: px(s.b[1]), right: px(s.b[2]), bottom: px(s.b[3]) };
    (Node { border, ..node }, BackgroundColor(s.bg), BorderColor::all(s.bc))
}

// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

pub fn font(skin: &Skin) -> TextFont {
    match &skin.font {
        Some(f) => TextFont {
            font: f.clone().into(),
            font_size: FontSize::Px(9.0),
            font_smoothing: FontSmoothing::None,
            ..default()
        },
        None => TextFont { font_size: FontSize::Px(8.0), ..default() },
    }
}

const OUTLINE: [(f32, f32); 8] = [(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, 1.0), (1.0, 1.0)];

/// Outlined text at another size of the pixel font (multiples of 9 stay crisp); `outline`
/// is the outline width in canvas pixels.
pub fn big_text(p: &mut ChildSpawnerCommands, skin: &Skin, s: &str, color: Color, size: f32, bold: bool, outline: f32) {
    let mut f = font(skin);
    if bold {
        if let Some(b) = &skin.font_bold {
            f.font = b.clone().into();
        }
    }
    f.font_size = FontSize::Px(size);
    p.spawn(Node { flex_shrink: 0.0, ..default() }).with_children(|w| {
        let steps = outline as i32;
        for dx in -steps..=steps {
            for dy in -steps..=steps {
                if dx != 0 || dy != 0 {
                    let n = Node { position_type: PositionType::Absolute, left: px(dx as f32), top: px(dy as f32), ..default() };
                    w.spawn((Text::new(s), f.clone(), TextColor(BLACK), n));
                }
            }
        }
        w.spawn((Text::new(s), f.clone(), TextColor(color)));
    });
}

/// How a label sits in its box.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
}

/// Outlined text in the normal flow of `p`. `wrap` gives a width to wrap at.
pub fn text(p: &mut ChildSpawnerCommands, skin: &Skin, s: &str, color: Color, wrap: Option<(f32, Justify)>) {
    let f = font(skin);
    let layout = TextLayout { justify: wrap.map(|w| w.1).unwrap_or_default(), ..default() };
    let outline = Color::BLACK.with_alpha(color.alpha());
    // A wrapping block is a fixed-width column so the text is measured at that width.
    let holder = match wrap {
        Some((w, _)) => Node { width: px(w), flex_direction: FlexDirection::Column, flex_shrink: 0.0, ..default() },
        None => Node { flex_shrink: 0.0, ..default() },
    };
    let copy = |dx: f32, dy: f32| match wrap {
        Some(_) => Node { position_type: PositionType::Absolute, left: px(dx), top: px(dy), width: percent(100), ..default() },
        None => Node { position_type: PositionType::Absolute, left: px(dx), top: px(dy), ..default() },
    };
    // Godot labels put 3px between lines (11px font + line_spacing 3)
    let lh = if wrap.is_some() { LineHeight::Px(14.0) } else { LineHeight::Px(11.0) };
    p.spawn(holder).with_children(|w| {
        for (dx, dy) in OUTLINE {
            w.spawn((Text::new(s), f.clone(), layout, lh, TextColor(outline), copy(dx, dy)));
        }
        w.spawn((Text::new(s), f.clone(), layout, lh, TextColor(color)));
    });
}

/// A label box: text aligned horizontally and centred vertically in (x, y, w, h).
pub fn label(p: &mut ChildSpawnerCommands, skin: &Skin, x: f32, y: f32, w: f32, h: f32, s: &str, color: Color, align: Align) {
    p.spawn(Node { justify_content: justify(align), align_items: AlignItems::Center, ..at(x, y, w, h) })
        .with_children(|c| text(c, skin, s, color, None));
}

pub fn justify(a: Align) -> JustifyContent {
    match a {
        Align::Left => JustifyContent::FlexStart,
        Align::Center => JustifyContent::Center,
    }
}

// ---------------------------------------------------------------------------------------------
// The main menu's animated background (BgCircle.gd)
// ---------------------------------------------------------------------------------------------

/// Three rings centred above the middle of the screen; their radii breathe on two sine waves.
#[derive(Component)]
pub struct Ring {
    /// Canvas y of the centre.
    y: f32,
    radius: f32,
    width: f32,
    variance: f32,
    period: f32,
}

#[derive(Resource)]
pub struct RingsShown(pub bool);

/// The logo on the main menu (canvas pixels), and the centre of the ring drawn in it.
pub const LOGO: (f32, f32, f32, f32) = (246.0, 2.0, 147.5, 80.0);
pub const LOGO_RING_Y: f32 = LOGO.1 + LOGO.3 * 0.5;

pub fn spawn_rings(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<ColorMaterial>>) {
    // the menu's theme: dark red bands, and a thin red ring pulsing around the logo's ring
    let dark = Color::srgb_u8(0x3a, 0x0a, 0x10);
    let red = Color::srgb_u8(0xc8, 0x08, 0x10);
    // drawn in the original's order: the big dark bands first, the thin ring on top
    for (i, (y, radius, width, variance, period, color)) in
        [(92.0, 350.0, 160.0, 8.0, 9.4, dark), (92.0, 300.0, 60.0, 8.0, 9.4, dark), (LOGO_RING_Y, 43.0, 1.5, 2.0, 8.4, red)].into_iter().enumerate()
    {
        commands.spawn((
            Ring { y, radius, width, variance, period },
            Mesh2d(meshes.add(Annulus::new(1.0, 2.0).mesh().resolution(64).build())),
            MeshMaterial2d(materials.add(color)),
            Transform::from_xyz(0.0, 0.0, -10.0 + i as f32),
            Visibility::Hidden,
        ));
    }
}

/// `Utils.wave(from, to, duration)`: a sine between `from` and `to` with the given period.
fn wave(from: f32, to: f32, period: f32, t: f32) -> f32 {
    let a = (to - from) * 0.5;
    from + a + (t / period * std::f32::consts::TAU).sin() * a
}

pub fn animate_rings(
    time: Res<Time<Real>>,
    shown: Res<RingsShown>,
    scale: Res<UiScale>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut rings: Query<(&Ring, &Mesh2d, &mut Transform, &mut Visibility)>,
) {
    let t = time.elapsed_secs();
    let s = scale.0;
    for (r, mesh, mut tf, mut vis) in &mut rings {
        let want = if shown.0 { Visibility::Visible } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if !shown.0 {
            continue;
        }
        // Godot's draw_arc centres the stroke on the radius
        let radius = r.radius + wave(-r.variance, r.variance, r.period, t);
        let (inner, outer) = ((radius - r.width / 2.0).max(0.0) * s, (radius + r.width / 2.0) * s);
        let _ = meshes.insert(&mesh.0, Annulus::new(inner, outer).mesh().resolution(64).build());
        // the ring's canvas point in world coordinates (origin at the window centre, y up)
        tf.translation.x = 0.0;
        tf.translation.y = (VH / 2.0 - r.y) * s;
    }
}
