//! Particle auras: each shown aura of a fighter's style emits particles around its body. They
//! live in game units (so they stay put on the stage as the camera pans) and are drawn as small
//! sprites of a few shapes drawn here, no game files needed. The Customize screen's preview
//! uses the same emitters on a fighter standing in the preview box.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::style::{Attach, Aura, Shape, Style};

#[derive(Resource, Clone)]
pub struct AuraShapes(pub [Handle<Image>; 5]);

impl AuraShapes {
    fn of(&self, s: Shape) -> Handle<Image> {
        self.0[Shape::ALL.iter().position(|x| *x == s).unwrap_or(0)].clone()
    }
}

/// Draws the particle shapes (16x16, white, alpha shaped).
pub fn make_shapes(images: &mut Assets<Image>) -> AuraShapes {
    const N: u32 = 16;
    let mut draw = |f: &dyn Fn(f32, f32) -> f32| {
        let mut data = Vec::with_capacity((N * N * 4) as usize);
        for y in 0..N {
            for x in 0..N {
                let (u, v) = ((x as f32 + 0.5) / N as f32 * 2.0 - 1.0, (y as f32 + 0.5) / N as f32 * 2.0 - 1.0);
                let a = (f(u, v).clamp(0.0, 1.0) * 255.0) as u8;
                data.extend_from_slice(&[255, 255, 255, a]);
            }
        }
        let mut img = Image::new(Extent3d { width: N, height: N, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
        img.sampler = ImageSampler::nearest();
        images.add(img)
    };
    let r = |u: f32, v: f32| (u * u + v * v).sqrt();
    AuraShapes([
        draw(&|u, v| 1.0 - (r(u, v) - 0.55) * 4.0),
        draw(&|_, _| 1.0),
        draw(&|_, v| 1.0 - (v.abs() - 0.2) * 6.0),
        draw(&|u, v| {
            let (a, b) = (u.abs(), v.abs());
            1.0 - ((a * b).sqrt() * 2.6 + (a + b) * 0.35 - 0.35) * 3.0
        }),
        draw(&|u, v| 1.0 - ((r(u, v) - 0.7).abs() - 0.12) * 8.0),
    ])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Space {
    Fight,
    Preview,
}

#[derive(Component)]
pub struct Particle {
    pub space: Space,
    pos: Vec2,
    vel: Vec2,
    age: f32,
    life: f32,
    start: LinearRgba,
    end: LinearRgba,
    size0: f32,
    size1: f32,
    gravity: f32,
    front: bool,
    spark: bool,
}

/// What an aura needs to know about the fighter it surrounds (game units, y down).
pub struct Body {
    pub feet: Vec2,
    pub vel: Vec2,
    pub facing: f32,
    /// The hurtbox: centre and half size.
    pub center: Vec2,
    pub half: Vec2,
}

/// Fractional particles carried between ticks, per (fighter, aura slot).
#[derive(Resource, Default)]
pub struct Emitters {
    acc: [[f32; 3]; 3],
    rng: u64,
}

impl Emitters {
    fn rand(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x2545_F491_4F6C_DD1D;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// One tick of emission for slot `who` (0, 1 for the fighters, 2 for the preview).
    pub fn emit(&mut self, commands: &mut Commands, shapes: &AuraShapes, who: usize, style: &Style, body: &Body, space: Space, scoped: Option<DespawnOnExit<crate::AppState>>) {
        for (k, a) in style.auras.iter().enumerate() {
            if !a.show || a.amount <= 0.0 {
                continue;
            }
            self.acc[who][k] += a.amount / 60.0;
            while self.acc[who][k] >= 1.0 {
                self.acc[who][k] -= 1.0;
                let p = self.particle(a, body, space);
                let mut e = commands.spawn((
                    Sprite { image: shapes.of(a.shape), color: Color::NONE, custom_size: Some(Vec2::splat(1.0)), ..default() },
                    Transform::default(),
                    p,
                ));
                if let Some(s) = scoped.clone() {
                    e.insert(s);
                }
            }
        }
    }

    fn particle(&mut self, a: &Aura, b: &Body, space: Space) -> Particle {
        let (rx, ry) = (self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0);
        let at = match a.attach {
            Attach::Body => b.center + Vec2::new(rx * b.half.x, ry * b.half.y),
            Attach::Head => Vec2::new(b.center.x + rx * b.half.x * 0.6, b.center.y - b.half.y * 0.7 + ry * b.half.y * 0.3),
            Attach::Feet => Vec2::new(b.center.x + rx * b.half.x, b.feet.y - self.rand() * 3.0),
            Attach::Front => Vec2::new(b.center.x + b.half.x * b.facing, b.center.y + ry * b.half.y * 0.5),
            Attach::Back => Vec2::new(b.center.x - b.half.x * b.facing, b.center.y + ry * b.half.y * 0.5),
        };
        let deg = a.direction + (self.rand() - 0.5) * a.spread;
        let rad = deg.to_radians();
        let speed = a.speed * (0.6 + self.rand() * 0.8);
        let vel = Vec2::new(rad.sin() * speed * b.facing, -rad.cos() * speed) + b.vel * a.inherit;
        let rgba = |c: [u8; 4]| LinearRgba::from(Color::srgba_u8(c[0], c[1], c[2], c[3]));
        Particle {
            space,
            pos: at,
            vel,
            age: 0.0,
            life: a.lifetime.max(1.0) * (0.75 + self.rand() * 0.5),
            start: rgba(a.start),
            end: rgba(a.end),
            size0: a.size,
            size1: a.end_size,
            gravity: a.gravity,
            front: a.front,
            spark: a.shape == Shape::Spark,
        }
    }
}

/// Ages and moves particles (fixed tick) and removes the dead.
pub fn tick_particles(mut commands: Commands, mut q: Query<(Entity, &mut Particle)>, frozen: Option<Res<crate::fight::Game>>) {
    let fight_paused = frozen.is_some_and(|g| !g.stepped);
    for (e, mut p) in &mut q {
        if p.space == Space::Fight && fight_paused {
            continue;
        }
        p.age += 1.0;
        if p.age >= p.life {
            commands.entity(e).despawn();
            continue;
        }
        let g = p.gravity;
        p.vel.y += g;
        let v = p.vel;
        p.pos += v;
    }
}

/// Where a space's game units land on screen: the screen point of game (0, 0) and the zoom.
#[derive(Resource, Default, Clone, Copy)]
pub struct PreviewFrame {
    pub origin: Vec2,
    pub zoom: f32,
    pub shown: bool,
}

/// Places particle sprites (every frame).
pub fn draw_particles(
    mut commands: Commands,
    mut q: Query<(Entity, &Particle, &mut Sprite, &mut Transform, &mut Visibility)>,
    game: Option<Res<crate::fight::Game>>,
    shake: Res<crate::fx::Shake>,
    preview: Res<PreviewFrame>,
) {
    for (e, p, mut sprite, mut tf, mut vis) in &mut q {
        let (origin, zoom) = match p.space {
            Space::Fight => match &game {
                Some(g) => (Vec2::new((-g.cam_x + shake.offset.x) * crate::fight::ZOOM, crate::fight::GROUND_Y - shake.offset.y * crate::fight::ZOOM), crate::fight::ZOOM),
                None => {
                    commands.entity(e).despawn();
                    continue;
                }
            },
            Space::Preview => {
                if !preview.shown {
                    commands.entity(e).despawn();
                    continue;
                }
                (preview.origin, preview.zoom)
            }
        };
        let t = (p.age / p.life).clamp(0.0, 1.0);
        let c = p.start * (1.0 - t) + p.end * t;
        sprite.color = Color::LinearRgba(c);
        let size = (p.size0 + (p.size1 - p.size0) * t).max(0.2) * zoom;
        sprite.custom_size = Some(if p.spark { Vec2::new(size * 2.5, size * 0.6) } else { Vec2::splat(size) });
        tf.translation = (origin + Vec2::new(p.pos.x, -p.pos.y) * zoom).extend(if p.front { 0.75 } else { 0.35 });
        tf.rotation = if p.spark { Quat::from_rotation_z((-p.vel.y).atan2(p.vel.x)) } else { Quat::IDENTITY };
        *vis = Visibility::Inherited;
    }
}
