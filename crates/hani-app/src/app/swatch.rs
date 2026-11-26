// SPDX-License-Identifier: AGPL-3.0-only

//! A picture of a brush: what a stroke of it does, on its button.
//!
//! `docs/STUDIO-PLAN.md` section 18, stage C7. Thirty-six names on as many
//! buttons say what a brush is called. What it DOES is a stroke, and a
//! picture of one says more than a name: that Band is flat on top and Bead
//! is round, that Scrape takes the tops off, that Stitches is bars a
//! width apart.
//!
//! # The picture is made by the brush
//!
//! Not drawn by hand, and not a diagram of the settings. A small slab is
//! made, the brush is run along it by the same engine that runs it on a
//! model ([`hani_core::brush::compound`], the stamp, [`hani_core::solid`]),
//! and the slab is looked at from above with a light on it. So a brush a
//! user makes and keeps has a picture the moment it is kept, a brush that
//! is changed has a picture that changes, and a picture cannot say
//! something the brush does not do.
//!
//! Three things are not as the brush is set, so that the picture can be
//! read:
//!
//! * **It is drawn harder than it is set**, at no less than
//!   [`LEAST_STRENGTH`]. A stroke at upstream's 0.15 raises the surface
//!   half a voxel, which at this size is not a picture of anything. The
//!   picture is of the shape of the stroke and not of how tall it is.
//! * **Its size is the picture's**, a little bigger or smaller for a brush
//!   that says how big it is, and not its millimetres.
//! * **A brush with nothing to do on a flat slab is given something.** One
//!   that only smooths or levels is drawn on a rippled slab, and one that
//!   only pinches or spreads on a slab with a ridge along the stroke's
//!   line, for the pinch to work on. The ripples are low and wide:
//!   seen on a screen as first made, tall and close, they were all that
//!   could be seen, and seven brushes had one picture of stripes.
//!
//! # Made once
//!
//! A picture is a few thousand voxels and a few thousand rays, and is
//! made the first time its brush is drawn and kept by the brush's
//! settings ([`Preset::key`]) for as long as the application runs. iced
//! gives every image handle an identity of its own, so making one afresh
//! on every draw would also upload it afresh on every draw.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use glam::{IVec3, Vec3};
use hani_core::brush::alpha::{Alpha, BuiltIn};
use hani_core::brush::compound::{Actions, Compound, Layer};
use hani_core::brush::stamp::{Placement, StampGesture};
use hani_core::region::FieldRegion;
use hani_core::solid::{self, Draped, Heights, How, Join, Placed, Solid, Step};
use hani_core::{BrushDirection, BrushScratch, Stamp, Symmetry, Volume};
use iced::widget::image::Handle;

use super::adding::Kind;
use super::presets::{Preset, StampOf};

/// How big a picture is, in pixels, which is also the slab's size in
/// voxels: a pixel a voxel.
pub(super) const WIDTH: usize = 72;
pub(super) const HEIGHT: usize = 28;

/// How far the slab goes up and down from its surface, in voxels: room
/// for a shape set down on it.
const DEEP: i32 = 14;

/// How far the slab goes past the picture's edges, so that no edge of it
/// is in the picture.
const MARGIN: i32 = 14;

/// The least strength a brush is drawn at. See the module documentation.
pub(super) const LEAST_STRENGTH: f32 = 0.5;

/// The radius a brush is drawn at when it does not say how big it is, in
/// voxels, and the size in millimetres that stands for.
const RADIUS: f32 = 6.0;
const USUAL_MM: f32 = 3.0;

/// How tall the ripples of the rippled slab are, in voxels, and how many
/// voxels there are to a radian of them.
const RIPPLE: f32 = 0.8;
const RIPPLE_PITCH: f32 = 1.7;

/// What a brush is run along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ground {
    Flat,
    /// Low ripples across the stroke's way, for a brush that smooths or
    /// levels.
    Rippled,
    /// A ridge along the stroke's line, for a brush that pinches or
    /// spreads.
    Ridged,
}

/// A slab for a brush to be run along.
fn slab(ground: Ground) -> Volume {
    let mut volume = Volume::new(1.0);
    let low = IVec3::new(-MARGIN, -MARGIN, -DEEP);
    let high = IVec3::new(WIDTH as i32 + MARGIN, HEIGHT as i32 + MARGIN, DEEP);
    let rippled = ground == Ground::Rippled;
    // How far above the ground a point is. Not quite its distance from a
    // rippled one, and near enough for a picture.
    volume.edit_voxels(low, high, move |_, at, _| {
        let up = if rippled { RIPPLE * ((at.x + at.y * 0.5) / RIPPLE_PITCH).sin() } else { 0.0 };
        at.z - up
    });
    if ground == Ground::Ridged {
        // A ridge is a stroke of Draw, gone over.
        let draw = Compound {
            actions: Actions::DRAW,
            radius: RADIUS,
            strength: 0.6,
            profile: Default::default(),
            pattern: hani_core::Pattern::default(),
        };
        let mut scratch = BrushScratch::new();
        for _ in 0..3 {
            for (at, along) in every(&line(), RADIUS * 0.25) {
                let stamp = Stamp::new(at, Vec3::Z, BrushDirection::Add).with_tangent(along);
                draw.apply(&mut volume, &stamp, None, &mut scratch);
            }
        }
    }
    volume
}

/// How high the slab's surface is over a point of the picture.
fn surface(volume: &Volume, at: Vec3) -> f32 {
    let mut z = (DEEP - 2) as f32;
    while z > -(DEEP as f32) && volume.sample_world(Vec3::new(at.x, at.y, z)) > 0.0 {
        z -= 0.1;
    }
    z
}

/// The line a stroke is drawn along: a shallow S from one side of the
/// picture to the other, which shows which way a brush faces as a
/// straight line would not.
fn line() -> Vec<Vec3> {
    let (from, to) = (9.0_f32, WIDTH as f32 - 9.0);
    (0..=120)
        .map(|step| {
            let along = step as f32 / 120.0;
            let swing = (along * std::f32::consts::TAU).sin() * 4.5;
            Vec3::new(from + (to - from) * along, HEIGHT as f32 * 0.5 + swing, 0.0)
        })
        .collect()
}

/// Points along a line every `apart` of its length, and the way the line
/// is going at each.
fn every(line: &[Vec3], apart: f32) -> Vec<(Vec3, Vec3)> {
    let mut found = Vec::new();
    let mut owed = 0.0;
    for step in line.windows(2) {
        let (run, length) = (step[1] - step[0], step[0].distance(step[1]));
        if length <= 0.0 {
            continue;
        }
        while owed <= length {
            found.push((step[0] + run * (owed / length), run / length));
            owed += apart.max(0.25);
        }
        owed -= length;
    }
    found
}

/// The radius a brush is drawn at: the picture's own, a little more or
/// less for a brush that says how big it is.
fn radius_of(preset: &Preset) -> f32 {
    let share = preset.size_mm.map_or(1.0, |size| (size / USUAL_MM).sqrt());
    (RADIUS * share).clamp(3.0, 8.0)
}

/// The picture a preset's tip or stamp names. The picture in hand is the
/// application's and not a preset's, so a dome stands for it.
fn alpha_of(picture: StampOf) -> Alpha {
    Alpha::built_in(match picture {
        StampOf::BuiltIn(kind) => kind,
        StampOf::InHand => BuiltIn::Dome,
    })
}

/// What a brush is run along: the flat, unless it has nothing to do
/// there. One that neither pushes nor swells, and levels to nothing held
/// proud of the surface, is given ripples to smooth or level, or, if all
/// it does is pinch or spread, a ridge to gather or open.
fn ground_for(actions: &Actions) -> Ground {
    let builds = actions.push != 0.0
        || actions.inflate != 0.0
        || (actions.level > 0.0 && actions.plane > 0.0);
    if builds {
        Ground::Flat
    } else if (actions.pinch != 0.0 || actions.gather != 0.0)
        && actions.smooth == 0.0
        && actions.level == 0.0
    {
        Ground::Ridged
    } else {
        Ground::Rippled
    }
}

/// The slab after a stroke of the brush.
fn stroked(preset: &Preset) -> Volume {
    let radius = radius_of(preset);
    if let Some(Kind::Map(map)) = preset.add.map(|setup| setup.what) {
        // A map, stamped in the middle of flat ground. One whose file is
        // gone has the bare ground for its picture.
        let mut volume = slab(Ground::Flat);
        let Ok(relief) = super::maps::relief(map) else {
            return volume;
        };
        let middle = Vec3::new(WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5, 0.0);
        let heights = Heights::flat();
        let mut join = Join::new();
        volume.begin_stroke();
        // Bigger than the picture is high: the shape is in the middle
        // of its square, and the rest of the square is ground that looks
        // like the ground.
        let stamp = Draped::on(
            &relief,
            &heights,
            middle,
            glam::Mat3::IDENTITY,
            HEIGHT as f32 * 0.8,
            1.0,
            false,
        );
        if let Some(stamp) = stamp {
            join.stamp(&mut volume, &stamp);
        }
        drop(volume.end_stroke());
        return volume;
    }
    if let Some(setup) = preset.add {
        let mut volume = slab(Ground::Flat);
        let mut join = Join::new();
        let how = How { cut: false, blend: setup.blend };
        let thick = radius * 0.5;
        // Off the ground by its lift, as the application holds it.
        let lifted = |at: Vec3, by: f32| at + Vec3::Z * (by * setup.lift);
        let mut pieces: Vec<Box<dyn Solid>> = Vec::new();
        let cable = |points: &[Vec3], pieces: &mut Vec<Box<dyn Solid>>| {
            for run in points.windows(2) {
                pieces.push(Box::new(Step { from: run[0], to: run[1], radius: thick }));
            }
            let last = points.len() - 1;
            let ends = [
                (setup.start, points[0], points[0] - points[1]),
                (setup.end, points[last], points[last] - points[last - 1]),
            ];
            for (end, at, along) in ends {
                if let Some(piece) = end.piece(at, along, thick) {
                    pieces.push(Box::new(piece));
                }
            }
        };
        match setup.what {
            Kind::Shape(form) => {
                let middle = Vec3::new(WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5, 0.0);
                let half = HEIGHT as f32 * 0.36;
                if let Some(placed) = Placed::on(form, middle, Vec3::Z, Vec3::X, half, setup.lift) {
                    pieces.push(Box::new(placed));
                }
            }
            Kind::Laid | Kind::Body | Kind::Map(_) => {
                let points: Vec<Vec3> =
                    every(&line(), thick).into_iter().map(|(at, _)| lifted(at, thick)).collect();
                cable(&points, &mut pieces);
            }
            Kind::Hung => {
                // Seen from above there is no down to hang toward, so it
                // hangs down the picture.
                let (from, to) =
                    (Vec3::new(10.0, 8.0, 0.0), Vec3::new(WIDTH as f32 - 10.0, 8.0, 0.0));
                let points =
                    solid::hang(lifted(from, thick), lifted(to, thick), setup.sag, Vec3::Y, 24);
                cable(&points, &mut pieces);
            }
        }
        volume.begin_stroke();
        for piece in &pieces {
            join.add(&mut volume, piece.as_ref(), how);
        }
        drop(volume.end_stroke());
        return volume;
    }

    if let Some((picture, height)) = preset.stamp {
        let mut volume = slab(Ground::Flat);
        let alpha = alpha_of(picture);
        let mut gesture = StampGesture::new();
        // One stamp sized by a drag, or a row of them at the brush's size.
        let places: Vec<(Vec3, f32)> = if preset.along > 0.0 {
            // Big enough to be made out as what they are.
            let radius = radius.max(4.5);
            let apart = radius * 2.0 * preset.along;
            let count = ((WIDTH as f32 - 16.0) / apart).floor().max(1.0) as usize;
            (0..=count)
                .map(|index| {
                    (Vec3::new(9.0 + index as f32 * apart, HEIGHT as f32 * 0.5, 0.0), radius)
                })
                .filter(|(at, radius)| at.x + radius < WIDTH as f32)
                .collect()
        } else {
            vec![(Vec3::new(WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5, 0.0), HEIGHT as f32 * 0.42)]
        };
        for (centre, radius) in places {
            let placement = Placement {
                centre,
                normal: Vec3::Z,
                up: Vec3::X,
                radius,
                height: radius * height.max(0.3),
            };
            gesture.begin();
            // A stamp that will not press is a stamp not in the picture.
            let _ = gesture.apply(&mut volume, &alpha, &placement, Symmetry::OFF, Vec3::ZERO);
            gesture.end();
        }
        return volume;
    }

    let Some(actions) = preset.custom.or_else(|| Actions::of(preset.kind)) else {
        // Move and Paint: no stroke on the field to draw.
        return slab(Ground::Flat);
    };
    let ground = ground_for(&actions);
    let mut volume = slab(ground);
    // A brush that works on what is there is given more of a go at it,
    // and a wider one: smoothing is its own repetition.
    let (radius, passes) = if ground == Ground::Flat { (radius, 2) } else { (radius * 1.3, 8) };
    let strength = preset.strength.unwrap_or_else(|| preset.kind.default_strength());
    let compound = Compound {
        actions,
        radius,
        strength: strength.max(LEAST_STRENGTH),
        profile: preset.profile,
        pattern: hani_core::Pattern::default(),
    };
    let tip = preset.tip.map(alpha_of);
    // Dabs big enough to be made out, each on the ground as it is there.
    let radius = if preset.dabs > 0.0 { radius.max(5.0) } else { radius };
    let compound = Compound { radius, ..compound };
    let apart = if preset.dabs > 0.0 { radius * 2.0 * preset.dabs } else { radius * 0.25 };
    let stamps: Vec<Stamp> = every(&line(), apart)
        .into_iter()
        .map(|(at, along)| {
            let on_the_ground = Vec3::new(at.x, at.y, surface(&volume, at));
            Stamp::new(on_the_ground, Vec3::Z, BrushDirection::Add).with_tangent(along)
        })
        .collect();
    let mut scratch = BrushScratch::new();
    volume.begin_stroke();
    if preset.build_up {
        // More than once, as a hand goes over a stroke: a brush that
        // builds up is one that is gone over.
        for _ in 0..passes {
            for stamp in &stamps {
                compound.apply(&mut volume, stamp, tip.as_ref(), &mut scratch);
            }
        }
    } else {
        let mut layer = Layer::new();
        layer.begin(preset.dabs <= 0.0);
        for stamp in &stamps {
            compound.lay(&mut volume, stamp, tip.as_ref(), &mut layer, &mut scratch);
        }
    }
    drop(volume.end_stroke());
    volume
}

/// A slab seen from above with a light on it: `WIDTH` by `HEIGHT` pixels
/// of red, green, blue and alpha.
fn seen(volume: &Volume) -> Vec<u8> {
    let mut region = FieldRegion::new();
    let low = IVec3::new(-2, -2, -DEEP);
    let high = IVec3::new(WIDTH as i32 + 2, HEIGHT as i32 + 2, DEEP);
    volume.snapshot(low, high, &mut region);
    // From over the top left, as the viewport's own light is.
    let light = Vec3::new(-0.45, -0.6, 0.66).normalize();
    let clay = Vec3::new(0.80, 0.78, 0.76);
    let top = (DEEP - 2) as f32;
    let mut pixels = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for row in 0..HEIGHT {
        for column in 0..WIDTH {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            // Down from the top until under the surface, by as far as the
            // field says the surface is, which it never is nearer than.
            let mut z = top;
            let mut value = region.sample(Vec3::new(x, y, z));
            let mut above = (z, value);
            while value > 0.0 && z > -top {
                above = (z, value);
                z -= value.max(0.2) * 0.9;
                value = region.sample(Vec3::new(x, y, z));
            }
            // Between the last point above and the first below.
            let hit = if value < above.1 && above.1 > 0.0 {
                above.0 + (z - above.0) * (above.1 / (above.1 - value))
            } else {
                z
            };
            let at = Vec3::new(x, y, hit);
            let across = |axis: Vec3| region.sample(at + axis) - region.sample(at - axis);
            let facing = Vec3::new(across(Vec3::X), across(Vec3::Y), across(Vec3::Z))
                .try_normalize()
                .unwrap_or(Vec3::Z);
            // What is nearer the eye is a little lighter, so that a flat
            // top and the flat ground round it are told apart.
            let lit = 0.30 + 0.72 * facing.dot(light).max(0.0) + 0.025 * hit.clamp(-6.0, 8.0);
            let colour = (clay * lit).clamp(Vec3::ZERO, Vec3::ONE) * 255.0;
            pixels.extend([colour.x as u8, colour.y as u8, colour.z as u8, 255]);
        }
    }
    pixels
}

/// The picture of a brush: `WIDTH` by `HEIGHT` pixels of red, green, blue
/// and alpha.
pub(super) fn pixels(preset: &Preset) -> Vec<u8> {
    seen(&stroked(preset))
}

/// The picture of a brush, as something to draw: made the first time it
/// is asked for and kept. See the module documentation.
pub(super) fn handle(preset: &Preset) -> Handle {
    static KEPT: LazyLock<Mutex<HashMap<String, Handle>>> = LazyLock::new(Mutex::default);
    let key = preset.key();
    let mut kept = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(found) = kept.get(&key) {
        return found.clone();
    }
    let made = Handle::from_rgba(WIDTH as u32, HEIGHT as u32, pixels(preset));
    kept.insert(key, made.clone());
    made
}

#[cfg(test)]
mod tests {
    use super::super::presets::{Presets, core};
    use super::*;

    /// How many pixels of two pictures differ by more than a shade.
    fn apart(one: &[u8], other: &[u8]) -> usize {
        one.chunks(4)
            .zip(other.chunks(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 6))
            .count()
    }

    /// Every brush that comes with the application has a picture, the
    /// size a picture is, with something in it: it is not the slab the
    /// brush was run along, flat or rippled. And no two brushes have the
    /// same picture, which is the least a picture is for.
    #[test]
    fn every_brush_has_a_picture_of_its_own() {
        let grounds =
            [Ground::Flat, Ground::Rippled, Ground::Ridged].map(|ground| seen(&slab(ground)));
        let [flat, rippled, ridged] = &grounds;
        assert_eq!(flat.len(), WIDTH * HEIGHT * 4);
        assert!(apart(flat, rippled) > WIDTH * HEIGHT / 3, "the rippled slab is not rippled");
        assert!(apart(flat, ridged) > 150, "the ridged slab has no ridge");
        let pictures: Vec<(&str, Vec<u8>)> =
            core().iter().map(|preset| (preset.name.as_str(), pixels(preset))).collect();
        for (name, picture) in &pictures {
            assert_eq!(picture.len(), WIDTH * HEIGHT * 4, "{name}");
            assert!(picture.chunks(4).all(|pixel| pixel[3] == 255), "{name} has holes in it");
            let changed = grounds.iter().map(|ground| apart(picture, ground)).min().unwrap_or(0);
            assert!(changed > 40, "{name} did nothing to its picture: {changed} pixels differ");
        }
        for (index, (name, picture)) in pictures.iter().enumerate() {
            for (other, earlier) in &pictures[..index] {
                let differ = apart(picture, earlier);
                assert!(differ > 12, "{name} and {other} have one picture: {differ} pixels differ");
            }
        }
    }

    /// What a picture shows is what the brush does: Band is flat across
    /// its middle where Draw is not, Crease is a groove, a row of rivets
    /// is separate rivets, and a brush that smooths is drawn on ripples
    /// and takes them out.
    #[test]
    fn a_picture_shows_what_its_brush_does() {
        let named =
            |name: &str| core().iter().find(|preset| preset.name == name).expect(name).clone();
        // How high the slab is at a point of the picture after the brush.
        let height = |volume: &Volume, x: f32, y: f32| {
            let mut z = (DEEP - 2) as f32;
            while z > -(DEEP as f32) && volume.sample_world(Vec3::new(x, y, z)) > 0.0 {
                z -= 0.05;
            }
            z
        };
        // The stroke's line crosses the middle of the picture half way.
        let (mid_x, mid_y) = (WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5);
        let (draw, band) = (stroked(&named("Draw")), stroked(&named("Band")));
        let (top, shoulder) = (height(&draw, mid_x, mid_y), height(&draw, mid_x, mid_y + 3.0));
        assert!(top > 1.0, "Draw raised its picture by only {top} voxels");
        let (flat_top, flat_shoulder) =
            (height(&band, mid_x, mid_y), height(&band, mid_x, mid_y + 3.0));
        // Half way out, Band has kept more of its height than Draw has.
        // Not all of it: a stroke this tall for its width is rounded off
        // by the brush's reach being a ball, flat middle or no.
        let (kept, flat_kept) = (shoulder / top, flat_shoulder / flat_top);
        assert!(
            flat_kept > kept + 0.1,
            "half way out Band keeps {flat_kept} of its height and Draw {kept}"
        );

        assert!(height(&stroked(&named("Crease")), mid_x, mid_y) < -0.5, "Crease is no groove");

        // Rivets in a row: up at one, down between two.
        let row = stroked(&named("Rivet row"));
        let along: Vec<f32> = (9..WIDTH - 9).map(|x| height(&row, x as f32, mid_y)).collect();
        let rivets = along.windows(2).filter(|pair| pair[0] <= 0.3 && pair[1] > 0.3).count();
        assert!(rivets >= 3, "a row of {rivets} rivets");

        // Smooth is drawn on ripples and takes them out where it went.
        let smooth = named("Smooth");
        let does =
            |preset: &Preset| preset.custom.or_else(|| Actions::of(preset.kind)).expect("a brush");
        assert_eq!(ground_for(&does(&smooth)), Ground::Rippled);
        let (before, after) = (slab(Ground::Rippled), stroked(&smooth));
        let rough = |volume: &Volume, y: f32| {
            let heights: Vec<f32> = (26..46).map(|x| height(volume, x as f32, y)).collect();
            heights.iter().copied().fold(f32::MIN, f32::max)
                - heights.iter().copied().fold(f32::MAX, f32::min)
        };
        assert!(rough(&before, mid_y) > 1.2, "the fixture's ripples are {}", rough(&before, mid_y));
        assert!(
            rough(&after, mid_y) < rough(&before, mid_y) * 0.6,
            "Smooth left ripples of {} where there were {}",
            rough(&after, mid_y),
            rough(&before, mid_y)
        );
        // The brushes that work on a ridge are drawn on one, and each
        // picture shows what its brush does to it. Pinch is a true pinch:
        // the ridge is narrower and as tall. Widen is wider and as tall.
        // Wear down is upstream's pinch turned round: it reads from
        // further off the brush's axis and the ridge is worn to a quarter
        // of its height. (Upstream's pinch the right way round left a
        // ridge 7.7 voxels wide at half its height over 12 wide, broader
        // with steeper sides, and came off the palette for it.) Held as
        // found, so that a change to any of them shows.
        for name in ["Pinch", "Widen", "Wear down"] {
            assert_eq!(ground_for(&does(&named(name))), Ground::Ridged, "{name}");
        }
        let wide = |volume: &Volume| {
            let top = height(volume, mid_x, mid_y);
            let across = (0..120).map(|step| mid_y - 6.0 + step as f32 * 0.1);
            across.filter(|y| height(volume, mid_x, *y) > top * 0.5).count() as f32 * 0.1
        };
        let top = |volume: &Volume| height(volume, mid_x, mid_y);
        let ridge = slab(Ground::Ridged);
        assert!(top(&ridge) > 2.5, "the fixture's ridge is {} voxels tall", top(&ridge));
        let after = |name: &str| {
            let volume = stroked(&named(name));
            (wide(&volume), top(&volume))
        };
        let (was_wide, was_tall) = (wide(&ridge), top(&ridge));
        let (pinched, pinched_top) = after("Pinch");
        assert!(pinched < was_wide * 0.8, "Pinch left the ridge {pinched} wide of {was_wide}");
        assert!(pinched_top > was_tall * 0.85, "and {pinched_top} tall of {was_tall}");
        let (widened, widened_top) = after("Widen");
        assert!(widened > was_wide * 1.15, "Widen left the ridge {widened} wide of {was_wide}");
        assert!(widened_top > was_tall * 0.85, "and {widened_top} tall of {was_tall}");
        let (_, worn_top) = after("Wear down");
        assert!(worn_top < was_tall * 0.5, "Wear down left the ridge {worn_top} tall");
        // And Draw and Clay are drawn on the flat, where they have
        // something to show.
        assert_eq!(ground_for(&Actions::DRAW), Ground::Flat);
        assert_eq!(ground_for(&Actions::CLAY), Ground::Flat);
    }

    /// A picture is made once for a brush and is the same picture however
    /// often it is asked for, and a brush that is changed is another
    /// brush with another.
    #[test]
    fn a_picture_is_made_once_and_follows_its_brush() {
        let draw = core().iter().find(|preset| preset.name == "Draw").expect("Draw").clone();
        assert_eq!(pixels(&draw), pixels(&draw), "a picture came out two ways");
        assert_eq!(handle(&draw).id(), handle(&draw).id(), "the picture was made twice");
        // The same brush under another name is the same picture.
        let renamed = Preset { name: "Mine".to_string(), ..draw.clone() };
        assert_eq!(handle(&renamed).id(), handle(&draw).id());
        let changed = Presets::parse("Mine = brush:Draw, plateau:0.8").list.remove(0);
        assert_ne!(handle(&changed).id(), handle(&draw).id());
        assert!(apart(&pixels(&changed), &pixels(&draw)) > 20);
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(48))]

            /// Whatever a brush file says, the brush it makes has a
            /// picture: the right size, with no hole in it, and made
            /// without a panic. A user's file is anybody's.
            #[test]
            fn any_brush_a_file_can_make_has_a_picture(
                parts in prop::collection::vec(
                    prop::sample::select(vec![
                        "brush:Clay", "brush:Smooth", "brush:Pinch", "brush:Paint", "brush:Move",
                        "push:-1", "push:0.4", "inflate:1", "inflate:-0.6", "pinch:-1", "smooth:1",
                        "level:1", "plane:-0.5", "plane:0.9", "side:Cut", "side:Fill",
                        "falloff:Round", "falloff:Sharp", "plateau:0.95", "size:0.1", "size:100",
                        "strength:0.01", "strength:0.8", "tip:Star", "tip:picture", "buildup:off",
                        "dabs:0.5", "dabs:4", "stamp:Bolt", "stamp:picture", "height:1", "along:0.5",
                        "along:4", "add:Ball", "add:Ring", "add:Cone", "add:Cable", "add:Hung cable",
                        "as:body", "join:2", "lift:-1", "lift:1", "start:Flange", "end:Ball", "sag:1",
                    ]),
                    0..7,
                ),
            ) {
                let line = format!("A brush = {}", parts.join(", "));
                let preset = Presets::parse(&line).list.remove(0);
                let picture = pixels(&preset);
                prop_assert_eq!(picture.len(), WIDTH * HEIGHT * 4, "{}", line);
                prop_assert!(picture.chunks(4).all(|pixel| pixel[3] == 255), "{}", line);
            }
        }
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// What the pictures of every brush that comes with the application
    /// cost to make, which is what the first draw of the palette waits
    /// for. Run by hand:
    /// `cargo test -p hani-app -- what_the_pictures_cost --ignored --nocapture`
    #[test]
    #[ignore = "a measurement, not a claim: run by hand"]
    fn what_the_pictures_cost() {
        let started = web_time::Instant::now();
        let brushes = super::super::presets::core();
        let mut slowest = (0.0_f64, "");
        for preset in brushes {
            let one = web_time::Instant::now();
            std::hint::black_box(pixels(preset));
            let took = one.elapsed().as_secs_f64() * 1000.0;
            if took > slowest.0 {
                slowest = (took, preset.name.as_str());
            }
        }
        println!(
            "{} pictures in {:.0} ms, the slowest {} at {:.1} ms",
            brushes.len(),
            started.elapsed().as_secs_f64() * 1000.0,
            slowest.1,
            slowest.0
        );
    }
}
