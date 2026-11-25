// SPDX-License-Identifier: AGPL-3.0-only

//! A picture of a paint: the paint on a slapchopped ball, as a brush's
//! button shows a stroke of the brush (`app/swatch.rs`).
//!
//! Asked for by the maintainer on 2026-11-25: paints should be shown the way
//! brushes are, on a textured ball, and the ball slapchopped (dark grey in
//! the recesses, near white on the highlights) so that what a paint does
//! over a figure prepared that way can be seen: a wash pools, a contrast
//! takes its light from what is under it, a metal reflects.
//!
//! # Made by the engine
//!
//! The ball is a real body: a ball with shallow bumps joined over its
//! face, so that it has recesses where they meet. It is looked at once, and
//! each pixel's surface kept: the way it faces, and how shut in it is
//! ([`brokkr_core::Volume::crease_at`], what a wash pools by). Its
//! undercoat is a slapchop of that: dark grey to near white by how much it
//! faces up and how far out of a recess it is. A paint's picture is then
//! each pixel's undercoat with the paint laid over it by its kind's blend,
//! by the stack's own arithmetic
//! ([`brokkr_core::paint::layers::over_undercoat`]), lit; a metal is lit as
//! the viewport lights one. That is a few thousand pixels of arithmetic, so
//! every paint on a rack has its picture, made the first time it is drawn
//! and kept.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use glam::{Vec2, Vec3};
use iced::widget::image::Handle;

use crate::paint_index::Kind;

/// The slapchop's two ends: a dark grey in the recesses, a near white on
/// the highlights.
const DARK: [u8; 3] = [52, 52, 56];
const LIGHT: [u8; 3] = [236, 234, 228];

/// How many bumps the ball has.
const BUMPS: usize = 22;

/// Where the light is, toward it, in the picture's frame: up, a little left
/// and toward the eye. The viewport's highlight is from the same side.
const LIGHT_FROM: Vec3 = Vec3::new(-0.4, 0.7, 0.6);

/// One pixel of the ball as it is looked at: which way it faces, how shut
/// in it is, and its undercoat. `None` off the ball.
#[derive(Debug, Clone, Copy)]
struct Seen {
    normal: Vec3,
    crease: f32,
    under: [u8; 3],
}

/// The ball, looked at `size` pixels across.
fn ball(size: usize) -> Vec<Option<Seen>> {
    use brokkr_core::solid::{Form, Placed, Solid};
    let voxel = 0.25;
    let radius = 4.0;
    // Shallow bumps over the side that faces the eye, laid on a spiral so
    // they are even: recesses for a wash to pool in, between surfaces that
    // still read as one ball. Eight big ones, the first try, read as a
    // cluster of berries at the rack's size.
    let bumps: Vec<(Vec3, f32)> = (0..BUMPS)
        .map(|at| {
            let height = 1.0 - 1.3 * (at as f32 + 0.5) / BUMPS as f32;
            let round = (1.0 - height * height).max(0.0).sqrt();
            let turn = at as f32 * std::f32::consts::PI * (3.0 - 5f32.sqrt());
            // The spiral runs from the eye outward: z is toward the eye.
            // Sizes varied, so it reads as a texture and not a pattern.
            let size = 0.7 + 0.45 * (at as f32 * 0.618).fract();
            (Vec3::new(round * turn.cos(), round * turn.sin(), height), size)
        })
        .collect();
    // The ball itself seeded whole, and the bumps joined on: a ball placed
    // as a shape is one set on a surface, and the first pictures had its
    // top cut flat.
    let mut body = brokkr_core::Volume::new(voxel);
    body.seed_sphere(Vec3::ZERO, radius);
    let mut join = brokkr_core::solid::Join::new();
    body.begin_stroke();
    for (way, size) in bumps {
        let out = way.normalize();
        if let Some(bump) = Placed::on(Form::Ball, out * radius * 0.97, out, Vec3::Y, size, 0.0) {
            join.add(&mut body, &bump as &dyn Solid, brokkr_core::solid::How::ADD);
        }
    }
    drop(body.end_stroke());

    // Room for the bumps, which stand out of the ball: at 1.3 the top
    // ones were cut off flat, seen on a strip of the pictures.
    let reach = radius * 1.5;
    let light = LIGHT_FROM.normalize();
    let mut seen = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let flat =
                (Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / size as f32 - 0.5) * 2.0 * reach;
            let mut at = Vec3::new(flat.x, -flat.y, reach * 1.5);
            let mut hit = None;
            for _ in 0..96 {
                let distance = body.sample_world(at) * voxel;
                if distance < voxel * 0.1 {
                    hit = Some(at);
                    break;
                }
                at.z -= distance.max(voxel * 0.25);
                if at.z < -reach * 1.5 {
                    break;
                }
            }
            seen.push(hit.map(|point| {
                let normal = body.gradient_world(point).normalize_or(Vec3::Z);
                let cell = (point / voxel).round().as_ivec3();
                let crease = body.crease_at(cell);
                // The slapchop: a dark coat everywhere, and the light dabbed
                // on where the surface faces up and stands out of a recess.
                let up = (normal.y + 1.0) * 0.5;
                let lit = (normal.dot(light) + 1.0) * 0.5;
                let open = 1.0 - crease.max(0.0);
                let weight = (0.45 * up + 0.25 * lit + 0.3 * open).clamp(0.0, 1.0);
                let t = ((weight - 0.3) / 0.5).clamp(0.0, 1.0);
                let t = t * t * (3.0 - 2.0 * t);
                let under = std::array::from_fn(|c| {
                    (f32::from(DARK[c]) + (f32::from(LIGHT[c]) - f32::from(DARK[c])) * t) as u8
                });
                Seen { normal, crease, under }
            }));
        }
    }
    seen
}

/// The ball as looked at, a pixel a place.
type Looked = std::sync::Arc<Vec<Option<Seen>>>;

/// The ball looked at `size` across, made once a size.
fn looked_at(size: usize) -> Looked {
    static KEPT: LazyLock<Mutex<HashMap<usize, Looked>>> = LazyLock::new(Mutex::default);
    let mut kept = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    kept.entry(size).or_insert_with(|| std::sync::Arc::new(ball(size))).clone()
}

/// The picture's pixels: red, green, blue and alpha, `size` by `size`.
pub(super) fn pixels(colour: [u8; 3], kind: Kind, size: usize) -> Vec<u8> {
    let seen = looked_at(size);
    let light = LIGHT_FROM.normalize();
    let toward_eye = Vec3::Z;
    let half_way = (light + toward_eye).normalize();
    let metal = kind == Kind::Metallic;
    let mut out = Vec::with_capacity(size * size * 4);
    for pixel in seen.iter() {
        let Some(seen) = pixel else {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        };
        let laid = brokkr_core::paint::layers::over_undercoat(
            seen.under,
            colour,
            kind.blend(),
            seen.crease,
        );
        let albedo = laid.map(|c| f32::from(c) / 255.0);
        let facing = seen.normal.dot(light).max(0.0);
        let shine = seen.normal.dot(half_way).max(0.0);
        let rgb: [f32; 3] = if metal {
            // As the viewport draws a metal: mostly what it reflects.
            let up = seen.normal.y * 0.5 + 0.5;
            let t = ((up - 0.3) / 0.65).clamp(0.0, 1.0);
            let reflected =
                (0.06 + 1.19 * t * t * (3.0 - 2.0 * t)) * (0.5 + 0.5 * seen.normal.z.max(0.0));
            let caught = shine.powf(40.0) * 0.65 * 2.2;
            std::array::from_fn(|c| albedo[c] * reflected * 1.35 + caught * (0.5 + 0.5 * albedo[c]))
        } else {
            let lit = 0.35 + 0.75 * facing;
            let caught = shine.powf(12.0) * 0.06;
            std::array::from_fn(|c| albedo[c] * lit + caught)
        };
        out.extend(rgb.iter().map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
        out.push(255);
    }
    out
}

/// A picture made: its colour, its kind and its size.
type Made = ([u8; 3], Kind, usize);

/// The picture of a paint, `size` pixels across, as something to draw:
/// made the first time it is asked for and kept.
pub(super) fn handle(colour: [u8; 3], kind: Kind, size: usize) -> Handle {
    static KEPT: LazyLock<Mutex<HashMap<Made, Handle>>> = LazyLock::new(Mutex::default);
    let mut kept = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    kept.entry((colour, kind, size))
        .or_insert_with(|| Handle::from_rgba(size as u32, size as u32, pixels(colour, kind, size)))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lightness(pixels: &[u8]) -> Vec<u32> {
        pixels
            .chunks(4)
            .filter(|pixel| pixel[3] > 0)
            .map(|pixel| u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2]))
            .collect()
    }

    /// The ball is a ball: about as much of the picture as a disc, and the
    /// undercoat is slapchopped, dark in places and near white in others.
    #[test]
    fn the_ball_is_slapchopped() {
        let seen = looked_at(48);
        let on = seen.iter().filter(|pixel| pixel.is_some()).count();
        let share = on as f32 / (48 * 48) as f32;
        assert!((0.35..0.75).contains(&share), "the ball covers {share} of its picture");
        let unders: Vec<u32> = seen
            .iter()
            .flatten()
            .map(|seen| seen.under.iter().map(|c| u32::from(*c)).sum())
            .collect();
        assert!(unders.iter().any(|value| *value < 250), "no dark recess");
        assert!(unders.iter().any(|value| *value > 600), "no near white highlight");
        assert!(seen.iter().flatten().any(|seen| seen.crease > 0.2), "nothing is shut in");
    }

    /// A paint covers; a wash leaves the highlights lighter than the
    /// recesses by more than a paint does; a metal spans more light than
    /// the same colour as paint.
    #[test]
    fn each_kind_is_drawn_its_own_way() {
        let spread = |values: &[u32]| {
            let mut sorted = values.to_vec();
            sorted.sort_unstable();
            sorted[sorted.len() * 98 / 100] - sorted[sorted.len() * 2 / 100]
        };
        let blue = [30, 60, 160];
        let paint = lightness(&pixels(blue, Kind::Paint, 48));
        let wash = lightness(&pixels(blue, Kind::Wash, 48));
        assert!(spread(&wash) > spread(&paint), "a wash did not keep the slapchop showing");
        let gold = [200, 160, 50];
        let flat = lightness(&pixels(gold, Kind::Paint, 48));
        let metal = lightness(&pixels(gold, Kind::Metallic, 48));
        assert!(spread(&metal) > spread(&flat), "a metal is no livelier than paint");
        assert_eq!(pixels(blue, Kind::Paint, 48).len(), 48 * 48 * 4);
    }

    /// A strip of every kind, written where `BROKKR_DUMP_FRAMES` says: to
    /// look at, not a claim.
    #[test]
    #[ignore = "a picture to look at, run by hand"]
    fn a_strip_of_every_kind() {
        let Ok(directory) = std::env::var("BROKKR_DUMP_FRAMES") else {
            return;
        };
        let size = 96;
        let kinds = [
            ([180, 30, 30], Kind::Paint),
            ([30, 25, 20], Kind::Wash),
            ([150, 20, 25], Kind::Contrast),
            ([60, 120, 40], Kind::Stain),
            ([200, 160, 50], Kind::Metallic),
        ];
        let mut strip = vec![0u8; size * size * 4 * kinds.len()];
        for (at, (colour, kind)) in kinds.iter().enumerate() {
            let one = pixels(*colour, *kind, size);
            for y in 0..size {
                let from = &one[y * size * 4..(y + 1) * size * 4];
                let to = (y * size * kinds.len() + at * size) * 4;
                strip[to..to + size * 4].copy_from_slice(from);
            }
        }
        image::save_buffer(
            std::path::Path::new(&directory).join("paint-previews.png"),
            &strip,
            (size * kinds.len()) as u32,
            size as u32,
            image::ColorType::Rgba8,
        )
        .expect("written");
    }
}
