//! Visual atmosphere: procedural speed lines / storm layer, screen darkening on supers,
//! impact frames on lethal hits, and dynamic hitstop zoom.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use crate::fight::{Game, GROUND_Y, ZOOM};
use crate::settings::Settings;

const NUM_LINES: usize = 240;
const LINE_MIN_SIZE: f32 = 30.0;
const LINE_MAX_SIZE: f32 = 280.0;
const SUSTAINED_TARGET_THRESHOLD: f32 = 0.35;
const SUSTAINED_TIME_THRESHOLD: f32 = 0.08;
const CHAR_FADE_RADIUS_X: f32 = 140.0;
const CHAR_FADE_RADIUS_Y: f32 = 95.0;
const LOW_SPEED_VISIBILITY: f32 = 0.25;

/// Deterministic 2D value noise for smooth procedural motion without heap allocations.
fn hash2(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393).wrapping_add((y as u32).wrapping_mul(668265263));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}

fn smooth_noise2(x: f32, y: f32) -> f32 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let wx = fx * fx * (3.0 - 2.0 * fx);
    let wy = fy * fy * (3.0 - 2.0 * fy);
    let s00 = hash2(ix, iy);
    let s10 = hash2(ix + 1, iy);
    let s01 = hash2(ix, iy + 1);
    let s11 = hash2(ix + 1, iy + 1);
    let s0 = s00 + (s10 - s00) * wx;
    let s1 = s01 + (s11 - s01) * wx;
    s0 + (s1 - s0) * wy
}

/// Tracks dynamic visual intensity, storm rain, speedlines, and hitstop punch.
#[derive(Resource)]
pub struct Atmosphere {
    // --- Speed lines / Storm ---
    pub speed_lines_on: bool,
    pub dir: Vec2,
    pub speed: f32,
    pub intensity: f32,
    pub smoothed_target: f32,
    pub sustained_above: f32,
    pub tick: u64,
    /// Whether the persistent rain storm atmosphere is active
    pub storm_weather: bool,

    // --- Screen Darken / Super Dim ---
    pub dim_opacity: f32,
    pub dim_target: f32,

    // --- Impact Frames ---
    pub impact_frames: u32,
    pub impact_color: Color,

    // --- Dynamic Zoom Kick ---
    pub zoom_kick: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            speed_lines_on: true,
            dir: Vec2::new(-0.75, -0.66).normalize(),
            speed: 1.0,
            intensity: 0.0,
            smoothed_target: 0.0,
            sustained_above: 0.0,
            tick: 0,
            storm_weather: false,

            dim_opacity: 0.0,
            dim_target: 0.0,

            impact_frames: 0,
            impact_color: Color::WHITE,

            zoom_kick: 0.0,
        }
    }
}

impl Atmosphere {
    /// Trigger a speed line burst along a trajectory with controlled intensity
    pub fn burst_lines(&mut self, dir: Vec2, speed: f32, max_intensity: f32) {
        if dir.length_squared() > 0.001 {
            self.dir = dir.normalize();
        }
        self.speed = speed.clamp(1.0, 3.0);
        let target = max_intensity.clamp(0.0, 0.85);
        self.smoothed_target = target;
        self.intensity = target * target;
        self.sustained_above = SUSTAINED_TIME_THRESHOLD;
    }

    /// Trigger super dimming for cinematic focus
    pub fn trigger_super_dim(&mut self, target: f32) {
        self.dim_target = target.clamp(0.0, 0.50);
    }

    /// Trigger high-contrast impact frames (1-2 black & white or flash frames)
    pub fn trigger_impact(&mut self, frames: u32, color: Color) {
        self.impact_frames = frames.max(self.impact_frames);
        self.impact_color = color;
    }

    /// Trigger dynamic camera zoom punch (e.g. +0.05 on heavy hit, +0.20 on K.O.)
    pub fn trigger_zoom_kick(&mut self, amount: f32) {
        self.zoom_kick = (self.zoom_kick + amount).min(0.35);
    }
}

/// Updates the atmosphere intensity and decays transient effects every frame.
pub fn update_atmosphere(
    time: Res<Time>,
    mut atmo: ResMut<Atmosphere>,
    game: Option<Res<Game>>,
    settings: Res<Settings>,
) {
    let dt = time.delta_secs();
    atmo.tick = atmo.tick.wrapping_add(1);

    // Decay impact zoom kick snappily
    if atmo.zoom_kick > 0.0 {
        atmo.zoom_kick = (atmo.zoom_kick - dt * 3.5).max(0.0);
    }

    // Interpolate super dimming
    let dim_target = atmo.dim_target;
    atmo.dim_opacity += (dim_target - atmo.dim_opacity) * (dt * 18.0).min(1.0);

    let Some(game) = game else {
        atmo.smoothed_target = 0.0;
        atmo.intensity = 0.0;
        return;
    };

    let w = &game.world;

    let in_freeze = w.f[0].hitlag > 0 || w.f[1].hitlag > 0;

    // Check fighters' velocity and combat states
    let v0 = Vec2::new(w.f[0].vel.x.to_f32(), -w.f[0].vel.y.to_f32());
    let v1 = Vec2::new(w.f[1].vel.x.to_f32(), -w.f[1].vel.y.to_f32());
    let (spd0, spd1) = (v0.length(), v1.length());

    // In reference (SpeedLines.gd), speed lines are suppressed during freeze ticks.
    // They only fire when a player is launched fast through the air (spd > 6.0).
    let launched0 = !in_freeze && matches!(w.f[0].act, sim::Act::Hurt { launched: true, .. }) && spd0 > 6.0;
    let launched1 = !in_freeze && matches!(w.f[1].act, sim::Act::Hurt { launched: true, .. }) && spd1 > 6.0;

    let storm_on = atmo.storm_weather || settings.flag("storm_weather");

    let mut target = 0.0;
    if launched0 {
        target = (spd0 / 18.0).clamp(0.15, 0.80);
        atmo.dir = -v0.normalize();
        atmo.speed = (spd0 / 9.0).clamp(1.0, 2.5);
    } else if launched1 {
        target = (spd1 / 18.0).clamp(0.15, 0.80);
        atmo.dir = -v1.normalize();
        atmo.speed = (spd1 / 9.0).clamp(1.0, 2.5);
    } else if storm_on {
        target = 0.35;
        atmo.dir = Vec2::new(-0.75, -0.66).normalize();
        atmo.speed = 1.4;
    }

    // Check if either fighter is currently in a Super move
    let mut super_move_active = false;
    for i in 0..2 {
        if let sim::Act::Move(run) = &w.f[i].act {
            let ch = &w.chars[i].file.character;
            if let Some(mv) = ch.moves.get(run.idx) {
                if mv.cost.super_bars > 0 {
                    super_move_active = true;
                }
            }
        }
    }

    // Reference (Main.tscn SuperDim): Color(0, 0, 0, 0.32549) strictly during super_active.
    // Standard heavy hits and launchers DO NOT dim the screen.
    if super_move_active {
        atmo.dim_target = 0.33;
    } else if atmo.dim_target > 0.0 && !in_freeze {
        atmo.dim_target = (atmo.dim_target - dt * 2.0).max(0.0);
    }

    // Smooth intensity interpolation (mimicking SpeedLines.gd INTENSITY_LERP_AT_60HZ)
    let lerp_t = 1.0 - (1.0 - 0.92_f32).powf(dt * 60.0);
    atmo.smoothed_target += (target - atmo.smoothed_target) * lerp_t;
    atmo.intensity = atmo.smoothed_target * atmo.smoothed_target;

    if target >= SUSTAINED_TARGET_THRESHOLD {
        atmo.sustained_above = (atmo.sustained_above + dt).min(SUSTAINED_TIME_THRESHOLD * 2.0);
    } else {
        atmo.sustained_above = (atmo.sustained_above - dt).max(0.0);
    }
}

/// Renders the atmospheric visuals: super dim, impact frames, speedlines, and storm weather.
pub fn draw_atmosphere(
    game: Res<Game>,
    mut atmo: ResMut<Atmosphere>,
    mut gizmos: Gizmos,
    shake: Res<crate::fx::Shake>,
    settings: Res<Settings>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let w = &game.world;
    let cam = game.cam_x;
    let sh = shake.offset;
    let effective_zoom = ZOOM * (1.0 + atmo.zoom_kick);
    let to_screen = |x: f32, y: f32| Vec2::new((x - cam + sh.x) * effective_zoom, GROUND_Y - (y + sh.y) * effective_zoom);

    // Dynamic viewport extents covering the entire screen window + margins
    let (span_x, span_y) = if let Ok(win) = windows.single() {
        (win.width() * 0.5 + 240.0, win.height() * 0.5 + 240.0)
    } else {
        (1000.0, 600.0)
    };
    let total_w = span_x * 2.0;
    let total_h = span_y * 2.0;

    // 1. Fullscreen Super Dimming
    if atmo.dim_opacity > 0.01 {
        gizmos.rect_2d(
            Vec2::ZERO,
            Vec2::new(total_w * 1.5, total_h * 1.5),
            Color::srgba(0.02, 0.02, 0.05, atmo.dim_opacity),
        );
    }

    // 2. High-Contrast Impact Frames
    if atmo.impact_frames > 0 {
        gizmos.rect_2d(
            Vec2::ZERO,
            Vec2::new(total_w * 1.5, total_h * 1.5),
            atmo.impact_color,
        );
        atmo.impact_frames -= 1;
        return;
    }

    // 3. Procedural Speed Lines & Storm Overlay
    let show_speed_lines = atmo.speed_lines_on && settings.flag("speed_lines");
    let storm_on = atmo.storm_weather || settings.flag("storm_weather");
    if show_speed_lines && (atmo.intensity > 0.01 || storm_on) {
        let p1_screen = to_screen(w.f[0].pos.x.to_f32(), w.f[0].pos.y.to_f32() - 15.0);
        let p2_screen = to_screen(w.f[1].pos.x.to_f32(), w.f[1].pos.y.to_f32() - 15.0);
        let ground_screen_y = to_screen(0.0, 0.0).y;

        let sustain_factor = (atmo.sustained_above / SUSTAINED_TIME_THRESHOLD).clamp(0.0, 1.0);
        let sustain = LOW_SPEED_VISIBILITY + (1.0 - LOW_SPEED_VISIBILITY) * sustain_factor;

        let num_lines = if storm_on {
            NUM_LINES
        } else {
            ((NUM_LINES as f32) * atmo.intensity).clamp(10.0, NUM_LINES as f32) as usize
        };

        for i in 0..num_lines {
            let t = (atmo.tick as f32) * 0.025 * atmo.speed.max(0.4);
            let seed_x = smooth_noise2(i as f32 * 2.71, t * 0.5) * 6400.0;
            let seed_y = smooth_noise2(i as f32 * 3.83 + 73.1, t * 0.4) * 3600.0;
            let variation = smooth_noise2(i as f32 * 4.19 + 17.0, 0.0);

            // Flow with velocity direction
            let flow_dist = (atmo.tick as f32) * 12.0 * atmo.speed;
            let mut px = seed_x + atmo.dir.x * flow_dist;
            let mut py = seed_y + atmo.dir.y * flow_dist;

            // Viewport wrap covering the entire screen window
            px = ((px % total_w) + total_w) % total_w - span_x;
            py = ((py % total_h) + total_h) % total_h - span_y;

            // Character fade bubble (keep players readable!)
            let dx1 = (px - p1_screen.x).abs() / CHAR_FADE_RADIUS_X;
            let dy1 = (py - p1_screen.y).abs() / CHAR_FADE_RADIUS_Y;
            let s1 = (dx1 * dx1 + dy1 * dy1).sqrt();

            let dx2 = (px - p2_screen.x).abs() / CHAR_FADE_RADIUS_X;
            let dy2 = (py - p2_screen.y).abs() / CHAR_FADE_RADIUS_Y;
            let s2 = (dx2 * dx2 + dy2 * dy2).sqrt();

            let clearance = if storm_on {
                // In storm rain, rain falls everywhere with subtle character highlight
                0.35 + 0.65 * s1.min(s2).clamp(0.0, 1.0)
            } else {
                s1.min(s2).clamp(0.0, 1.0)
            };

            let line_intensity = (atmo.intensity.max(if storm_on { 0.22 } else { 0.0 })
                * variation
                * clearance
                * sustain)
                .clamp(0.0, 1.0);

            if line_intensity < 0.02 {
                continue;
            }

            let line_len = LINE_MIN_SIZE + (LINE_MAX_SIZE - LINE_MIN_SIZE) * line_intensity;
            let half = atmo.dir * (line_len * 0.5);
            let start = Vec2::new(px, py) - half;
            let end = Vec2::new(px, py) + half;

            let color = if storm_on {
                Color::srgba(0.80, 0.88, 0.98, (line_intensity * 0.65).min(0.75))
            } else {
                Color::srgba(1.0, 1.0, 1.0, (line_intensity * 0.85).min(0.90))
            };

            gizmos.line_2d(start, end, color);

            // Floor splash ripple during storm weather
            if storm_on && (py - ground_screen_y).abs() < 14.0 {
                let splash_w = 4.0 + variation * 8.0;
                gizmos.line_2d(
                    Vec2::new(px - splash_w, ground_screen_y),
                    Vec2::new(px + splash_w, ground_screen_y),
                    Color::srgba(0.85, 0.94, 1.0, line_intensity * 0.55),
                );
            }
        }
    }
}
