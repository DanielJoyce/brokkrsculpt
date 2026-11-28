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
//! ([`hani_core::Volume::crease_at`], what a wash pools by). Its
//! undercoat is a slapchop of that: dark grey to near white by how much it
//! faces up and how far out of a recess it is. A paint's picture is then
//! each pixel's undercoat with the paint laid over it by its kind's blend,
//! by the stack's own arithmetic
//! ([`hani_core::paint::layers::over_undercoat`]), lit; a metal, metallic or
//! chrome, is lit as the viewport lights one, by the viewport's own model
//! in Rust ([`hani_gpu::lights::shade`], which the offscreen tests hold to
//! the shader) under its default rig. That is a few thousand pixels of
//! arithmetic, so every paint on a rack has its picture, made the first
//! time it is drawn and kept. A metallic's flakes are not drawn here: the
//! picture is too small for them.

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
    /// How shut in it is, and how much of an edge: the crease is made of
    /// these by the pooling in hand.
    occlusion: f32,
    edge: f32,
    under: [u8; 3],
}

/// The ball, looked at `size` pixels across.
fn ball(size: usize) -> Vec<Option<Seen>> {
    use hani_core::solid::{Form, Placed, Solid};
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
    let mut body = hani_core::Volume::new(voxel);
    body.seed_sphere(Vec3::ZERO, radius);
    let mut join = hani_core::solid::Join::new();
    body.begin_stroke();
    for (way, size) in bumps {
        let out = way.normalize();
        if let Some(bump) = Placed::on(Form::Ball, out * radius * 0.97, out, Vec3::Y, size, 0.0) {
            join.add(&mut body, &bump as &dyn Solid, hani_core::solid::How::ADD);
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
                let (occlusion, edge) = (body.occlusion_at(cell), body.edge_at(cell));
                let crease = hani_core::paint::occlusion::crease_of(
                    occlusion,
                    edge,
                    hani_core::paint::occlusion::POOLING,
                );
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
                Seen { normal, occlusion, edge, under }
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

/// The picture's pixels: red, green, blue and alpha, `size` by `size`, the
/// paint pooled as strongly as `pooling` says. `chrome` draws a metallic as
/// chrome.
pub(super) fn pixels(
    colour: [u8; 3],
    kind: Kind,
    chrome: bool,
    size: usize,
    pooling: f32,
) -> Vec<u8> {
    let seen = looked_at(size);
    let light = LIGHT_FROM.normalize();
    let toward_eye = Vec3::Z;
    let half_way = (light + toward_eye).normalize();
    let metal = (kind == Kind::Metallic).then(|| {
        let finish = if chrome {
            hani_core::paint::Finish::Chrome
        } else {
            hani_core::paint::Finish::Metallic
        };
        finish.roughness_and_metal().0
    });
    let block = hani_gpu::LightBlock::default();
    let mut out = Vec::with_capacity(size * size * 4);
    for pixel in seen.iter() {
        let Some(seen) = pixel else {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        };
        let laid = hani_core::paint::layers::over_undercoat(
            seen.under,
            colour,
            kind.blend(),
            hani_core::paint::occlusion::crease_of(seen.occlusion, seen.edge, pooling),
        );
        let albedo = laid.map(|c| f32::from(c) / 255.0);
        let facing = seen.normal.dot(light).max(0.0);
        let shine = seen.normal.dot(half_way).max(0.0);
        if let Some(roughness) = metal {
            // As the viewport draws a metal: its colour is what it
            // reflects. The picture looks down the view's own axis with the
            // world upright, so the view is the identity.
            let surface =
                hani_gpu::Surface::metal(Vec3::from(hani_core::paint::linear(laid)), roughness);
            let lit = hani_gpu::lights::shade(
                &block,
                glam::Mat4::IDENTITY,
                &surface,
                seen.normal,
                seen.normal,
                seen.normal,
                0.0,
            );
            out.extend(hani_core::paint::srgb(lit.to_array()));
            out.push(255);
            continue;
        }
        let rgb: [f32; 3] = {
            let lit = 0.35 + 0.75 * facing;
            let caught = shine.powf(12.0) * 0.06;
            std::array::from_fn(|c| albedo[c] * lit + caught)
        };
        out.extend(rgb.iter().map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
        out.push(255);
    }
    out
}

/// A picture made: its colour, its kind, whether chrome, its size and the
/// pooling, by its bits.
type Made = ([u8; 3], Kind, bool, usize, u32);

/// The picture of a paint, `size` pixels across, as something to draw:
/// made the first time it is asked for and kept.
pub(super) fn handle(
    colour: [u8; 3],
    kind: Kind,
    chrome: bool,
    size: usize,
    pooling: f32,
) -> Handle {
    static KEPT: LazyLock<Mutex<HashMap<Made, Handle>>> = LazyLock::new(Mutex::default);
    // To a tenth: a drag of the slider would otherwise make a picture of
    // every paint at every step of it, and keep them all.
    let pooling = (pooling * 10.0).round() / 10.0;
    let mut kept = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    kept.entry((colour, kind, chrome, size, pooling.to_bits()))
        .or_insert_with(|| {
            Handle::from_rgba(size as u32, size as u32, pixels(colour, kind, chrome, size, pooling))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hani_core::paint::occlusion::POOLING;

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
        assert!(seen.iter().flatten().any(|seen| seen.occlusion > 0.1), "nothing is shut in");
    }

    /// A paint covers; a wash leaves the highlights lighter than the
    /// recesses by more than a paint does; a metal, with nothing of it
    /// diffuse, is darker than the same colour as paint wherever it is not
    /// toward a light (until 2026-11-27 this said it spanned more light,
    /// which the old formula's was and the viewport's model's is not, by 2
    /// levels in 324); chrome is a different picture with harder light.
    #[test]
    fn each_kind_is_drawn_its_own_way() {
        let spread = |values: &[u32]| {
            let mut sorted = values.to_vec();
            sorted.sort_unstable();
            sorted[sorted.len() * 98 / 100] - sorted[sorted.len() * 2 / 100]
        };
        let blue = [30, 60, 160];
        let paint = lightness(&pixels(blue, Kind::Paint, false, 48, POOLING));
        let wash = lightness(&pixels(blue, Kind::Wash, false, 48, POOLING));
        assert!(spread(&wash) > spread(&paint), "a wash did not keep the slapchop showing");
        let gold = [200, 160, 50];
        let flat = lightness(&pixels(gold, Kind::Paint, false, 48, POOLING));
        let metal = lightness(&pixels(gold, Kind::Metallic, false, 48, POOLING));
        let at = |values: &[u32], share: usize| {
            let mut sorted = values.to_vec();
            sorted.sort_unstable();
            sorted[(sorted.len() - 1) * share / 100]
        };
        assert!(
            at(&metal, 5) < at(&flat, 5) && at(&metal, 50) + 30 < at(&flat, 50),
            "a metal is lit as paint is: {} and {} against {} and {}",
            at(&metal, 5),
            at(&metal, 50),
            at(&flat, 5),
            at(&flat, 50)
        );
        // Chrome is the metallic nearly a mirror: a different picture, and
        // one with harder light in it, its brightest brighter.
        let chrome = lightness(&pixels(gold, Kind::Metallic, true, 48, POOLING));
        assert_ne!(chrome, metal, "chrome is drawn as the metallic is");
        let top = |values: &[u32]| values.iter().copied().max().unwrap_or(0);
        assert!(top(&chrome) > top(&metal), "chrome's brightest is no brighter");
        // The box means nothing on any other kind.
        assert_eq!(
            pixels(blue, Kind::Paint, true, 48, POOLING),
            pixels(blue, Kind::Paint, false, 48, POOLING)
        );
        assert_eq!(pixels(blue, Kind::Paint, false, 48, POOLING).len(), 48 * 48 * 4);
    }

    /// A strip of every kind, written where `HANI_DUMP_FRAMES` says: to
    /// look at, not a claim.
    #[test]
    #[ignore = "a picture to look at, run by hand"]
    fn a_strip_of_every_kind() {
        let Ok(directory) = std::env::var("HANI_DUMP_FRAMES") else {
            return;
        };
        let size = 96;
        let kinds = [
            ([180, 30, 30], Kind::Paint, false),
            ([30, 25, 20], Kind::Wash, false),
            ([150, 20, 25], Kind::Contrast, false),
            ([60, 120, 40], Kind::Stain, false),
            ([200, 160, 50], Kind::Metallic, false),
            ([200, 200, 205], Kind::Metallic, false),
            ([200, 200, 205], Kind::Metallic, true),
        ];
        let mut strip = vec![0u8; size * size * 4 * kinds.len()];
        for (at, (colour, kind, chrome)) in kinds.iter().enumerate() {
            let one = pixels(*colour, *kind, *chrome, size, POOLING);
            for y in 0..size {
                let from = &one[y * size * 4..(y + 1) * size * 4];
                let to = (y * size * kinds.len() + at * size) * 4;
                strip[to..to + size * 4].copy_from_slice(from);
            }
        }
        // A wash and a contrast at no pooling, the usual and the most.
        let poolings = [0.0, POOLING, 3.0];
        let pooled = [([30, 25, 20], Kind::Wash), ([150, 20, 25], Kind::Contrast)];
        let across = poolings.len() * pooled.len();
        let mut grid = vec![0u8; size * size * 4 * across];
        for (at, ((colour, kind), pooling)) in pooled
            .iter()
            .flat_map(|paint| poolings.iter().map(move |pooling| (paint, pooling)))
            .enumerate()
        {
            let one = pixels(*colour, *kind, false, size, *pooling);
            for y in 0..size {
                let to = (y * size * across + at * size) * 4;
                grid[to..to + size * 4].copy_from_slice(&one[y * size * 4..(y + 1) * size * 4]);
            }
        }
        image::save_buffer(
            std::path::Path::new(&directory).join("paint-pooling.png"),
            &grid,
            (size * across) as u32,
            size as u32,
            image::ColorType::Rgba8,
        )
        .expect("written");
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
