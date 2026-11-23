// SPDX-License-Identifier: AGPL-3.0-only

//! Changing a form and keeping what is on it.
//!
//! Inflate makes a model fatter by subtracting from the field. Every level
//! set then moves along ITS OWN normal, and the two walls of a fine groove
//! have normals that point at each other, so they meet and the groove is
//! gone. Scales, pores and wrinkles are all grooves. That is a dilation, and
//! no setting of it keeps detail.
//!
//! What keeps detail is a warp: `new(p) = old(p - u(p))`. Whatever was at
//! `p - u` arrives at `p` as it was, because nothing about it is worked out
//! again. `brush.rs` says so of Move, and it holds for any `u` that is
//! smooth compared with the detail.
//!
//! So the two tools here are one idea. The FORM is the model with its
//! detail smoothed away, which [`crate::pyramid`] already holds at every
//! scale. Its normal does not turn for a pore.
//!
//! * [`Volume::bulked`] moves every point along the form's normal by a set
//!   distance: `u = amount * form normal`. The mass gets thicker or thinner
//!   and the surface texture rides out with it.
//! * [`Volume::with_detail_gain`] leaves the form where it is and scales how
//!   far the surface stands off it: softer, or more pronounced. Detail that
//!   a 0.4 mm nozzle would erase can be exaggerated on purpose before a
//!   print.
//!
//! # What "detail" means is the caller's, in millimetres
//!
//! [`level_for`] turns a size in millimetres into a level of the pyramid.
//! Features smaller than that size ride along; features larger are the form
//! being changed. In millimetres so that it means the same thing at every
//! voxel size.
//!
//! # Limits, said before they are found
//!
//! * **Detail is carried, not made.** Where a surface grows, its detail
//!   stretches with it.
//! * **One warp is one trilinear read**, and that softens features near the
//!   size of a voxel. `a_ripple_survives_a_bulk` measures how much.
//! * **The amount is bounded by the scale.** The form is only known within
//!   a band of its own surface, three of ITS voxels, so a bulk larger than
//!   that band has nothing to steer by. [`FormError::TooFar`] says so and
//!   names the size that would do.
//! * **The mask scales the amount** and is not itself moved. Under a
//!   feathered mask the bulk fades out smoothly, which is the point of the
//!   feather; the mask stays where it was drawn.

use glam::{IVec3, Vec3};

use crate::brick::{INSIDE, NARROW_BAND, OUTSIDE};
use crate::colour::ColourField;
use crate::mask::PROTECTED;
use crate::pyramid::{FormPyramid, MAX_LEVELS};
use crate::volume::{BrickPreview, BrickVerdict, Volume};

/// Why a form could not be changed as asked. The volume is as it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FormError {
    /// The pyramid has no level at the scale asked for: the model is smaller
    /// than the detail size, or the pyramid was built too shallow.
    NoSuchLevel,
    /// The amount is more than the form at this scale can steer. `most_mm`
    /// is the most this scale allows.
    TooFar { most_mm: f32 },
    /// A number that is not one.
    NotFinite,
}

impl std::fmt::Display for FormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormError::NoSuchLevel => {
                f.write_str("the model has no form at that detail size: try a smaller one")
            }
            FormError::TooFar { most_mm } => write!(
                f,
                "that is further than this detail size can carry: {most_mm:.2} mm at most, or \
                 choose a larger detail size"
            ),
            FormError::NotFinite => f.write_str("that is not a number"),
        }
    }
}

/// The level of a pyramid over a volume of `voxel_size` whose voxel is
/// nearest `detail_mm`: level 0 is twice the voxel, level 1 four times.
///
/// Clamped to the levels a pyramid can have, so a detail size of nothing is
/// the finest form there is and not an error.
pub fn level_for(voxel_size: f32, detail_mm: f32) -> usize {
    if !(detail_mm > 0.0 && voxel_size > 0.0) {
        return 0;
    }
    let doublings = (detail_mm / voxel_size).log2().round();
    (doublings as i64 - 1).clamp(0, MAX_LEVELS as i64 - 1) as usize
}

/// How many of the pyramid's halvings the detail gain repeats on the form to
/// measure what they took off it. See [`Volume::with_detail_gain`].
const REDONE_HALVINGS: usize = 2;

/// The most a bulk at `level` can be, in voxels of the source volume.
///
/// The form's band is three of its own voxels either side. A point being
/// pulled back has to stay inside it the whole way, and the surface it is
/// near may stand a form voxel off the form, so what is left for the amount
/// is the band less one form voxel and less the source's own band.
fn most_voxels(level: usize) -> f32 {
    let form_voxel = (1_u32 << (level + 1)) as f32;
    ((NARROW_BAND - 1.25) * form_voxel - NARROW_BAND).max(0.0)
}

impl Volume {
    /// This volume with its mass thickened by `amount_mm` along the form's
    /// normal, or thinned by a negative amount, the detail finer than the
    /// form's scale carried with the surface.
    ///
    /// `pyramid` must have been built from this volume as it now is, and
    /// `level` is the scale: see [`level_for`]. Paint is carried with the
    /// surface. The mask scales the amount and is not moved.
    ///
    /// The result's gradient is not one everywhere, as after any warp; the
    /// caller runs [`Volume::redistance`] on it.
    pub fn bulked(
        &self,
        pyramid: &FormPyramid,
        level: usize,
        amount_mm: f32,
    ) -> Result<Volume, FormError> {
        if !amount_mm.is_finite() {
            return Err(FormError::NotFinite);
        }
        if level >= pyramid.level_count() {
            return Err(FormError::NoSuchLevel);
        }
        let voxel_size = self.voxel_size();
        let amount = amount_mm / voxel_size;
        let most = most_voxels(level);
        if amount.abs() > most {
            return Err(FormError::TooFar { most_mm: most * voxel_size });
        }
        let mut out = self.duplicated(IVec3::ZERO);
        let Some((low, high)) = self.world_bounds() else {
            return Ok(out);
        };
        if amount == 0.0 {
            return Ok(out);
        }

        let form_voxel = (1_u32 << (level + 1)) as f32;
        // Past this many form voxels from the form, a point is further from
        // the surface than the bulk and both bands together, and is solid or
        // empty whatever the bulk does.
        let far = (amount.abs() + NARROW_BAND) / form_voxel + 1.0;
        let reach = amount.abs().ceil() as i32 + 2;
        // One step back along the form's normal, however far. The plan drew
        // this as a walk in small steps, re-reading the normal at each. It
        // is not needed: the form is a distance field, the lines its
        // gradient follows are straight, and the normal a point starts with
        // is the normal all the way back. A mutant that took one step where
        // this took three survived every test, which is how that was
        // noticed.
        let pull_back = move |position: Vec3, free: f32| -> Option<Vec3> {
            let relief = pyramid.relief(level, position);
            if relief.abs() >= far {
                return None;
            }
            Some(position - pyramid.form_normal(level, position) * (amount_mm * free))
        };

        let margin = Vec3::splat((amount.abs() + NARROW_BAND * 2.0) * voxel_size);
        let (lo, hi) = self.voxel_bounds(low - margin, high + margin);
        let decide = |preview: &BrickPreview| {
            if preview.mask == Some(PROTECTED) {
                return BrickVerdict::Skip;
            }
            match preview.uniform {
                Some(value) if value == INSIDE || value == OUTSIDE => {
                    BrickVerdict::OnlyNearDifferentNeighbours
                }
                _ => BrickVerdict::Whole,
            }
        };
        out.edit_voxels_where(lo, hi, reach, true, decide, |_, position, value, free| {
            if free <= 0.0 {
                return value;
            }
            match pull_back(position, free) {
                Some(from) => self.sample_world(from),
                // Solid or empty, by which side of the form it is on.
                None if pyramid.relief(level, position) < 0.0 => INSIDE,
                None => OUTSIDE,
            }
        });

        // Paint goes with the surface: every voxel that can now be painted
        // takes the paint of the voxel it was pulled back to.
        *out.colour_mut() = ColourField::default();
        if !self.colour().is_empty() {
            let colour = self.colour();
            out.edit_colour(lo, hi, |_, position, _, protection| {
                let free = 1.0 - f32::from(protection) / f32::from(PROTECTED);
                match pull_back(position, free) {
                    Some(from) => colour.at((from / voxel_size).round().as_ivec3()),
                    None => 0,
                }
            });
        }
        // And the paint proper, the same way.
        *out.paint_mut() = crate::paint::field::PaintField::default();
        if !self.paint().is_empty() {
            let paint = self.paint();
            out.edit_paint(lo, hi, |_, position, _, protection| {
                let free = 1.0 - f32::from(protection) / f32::from(PROTECTED);
                match pull_back(position, free) {
                    Some(from) => paint.at((from / voxel_size).round().as_ivec3()),
                    None => crate::paint::field::BARE,
                }
            });
        }
        out.mark_everything_dirty();
        Ok(out)
    }

    /// This volume with its detail scaled about its form: `gain` of 1 is the
    /// volume as it is, 0 is the form alone, and more than 1 stands every
    /// feature further off the form than it was.
    ///
    /// `new = form + gain * (old - form)`, once. `brush.rs` forbids exactly
    /// this arithmetic inside a stamp, where it would be applied hundreds of
    /// times over and its rounding would compound into a crust. Done once
    /// from a field nobody is editing it is one linear step, clamped.
    ///
    /// The field only knows itself within its band, so a feature can be
    /// moved by about two voxels at most: more gain than that saturates and
    /// does not tear. For more, apply it twice.
    ///
    /// # The form of a ball is a smaller ball
    ///
    /// Smoothing pulls a surface toward its centre of curvature, so a level
    /// of the pyramid sits a little inside every bulge and a little outside
    /// every hollow. Scaling the detail about THAT would also change the
    /// model's size by `1 - gain` times the difference, and the difference
    /// is not small: see `what_a_bulk_costs` for the measurements.
    ///
    /// So the form is corrected by measuring what the smoothing does to this
    /// very shape. The form is brought back to a finer voxel and smoothed
    /// again by the same steps; how far that second pass moved it is, near
    /// enough, how far the first pass moved the surface, and it is added
    /// back.
    ///
    /// Measured on a ball 30 mm in radius at a quarter millimetre, taking
    /// ALL the detail off, over 300 directions (`what_a_bulk_costs`). The
    /// surface moves, in millimetres, on average and at worst:
    ///
    /// | Detail size | Form as it stands | Corrected |
    /// |---|---|---|
    /// | 0.5 mm | -0.002, -0.008 | +0.001, +0.010 |
    /// | 1 mm | -0.012, -0.028 | +0.000, +0.024 |
    /// | 2 mm | -0.053, -0.086 | -0.002, +0.052 |
    /// | 4 mm | -0.220, -0.298 | -0.013, -0.104 |
    /// | 8 mm | -0.710, -0.806 | -0.081, -0.378 |
    ///
    /// The average is the model's size, and that is kept. The worst is the
    /// pyramid's own grain, direction by direction, and the correction does
    /// not touch it: at a detail size a quarter of the model it is a third
    /// of a millimetre, which is the form tools saying that a quarter of
    /// the model is not detail.
    ///
    /// Estimating the shrinkage from the next COARSER level instead
    /// (Richardson's extrapolation) was tried first and is not here. The
    /// coarser level is the one least able to hold the model, and on a ball
    /// 30 mm in radius it made a detail size of 4 mm worse than no
    /// correction at all.
    pub fn with_detail_gain(
        &self,
        pyramid: &FormPyramid,
        level: usize,
        gain: f32,
    ) -> Result<Volume, FormError> {
        if !gain.is_finite() || gain < 0.0 {
            return Err(FormError::NotFinite);
        }
        if level >= pyramid.level_count() {
            return Err(FormError::NoSuchLevel);
        }
        let mut out = self.duplicated(IVec3::ZERO);
        let Some((low, high)) = self.world_bounds() else {
            return Ok(out);
        };
        if gain == 1.0 {
            return Ok(out);
        }
        let form_voxel = (1_u32 << (level + 1)) as f32;
        // The form of the form: see "The form of a ball is a smaller ball"
        // above. `None` when the form is too small to have one at this
        // scale, and then the gain is about the form as it stands.
        let again = {
            // Brought back two halvings and no further. Each halving smooths
            // four times as much as the one before, so the last two are
            // fifteen sixteenths of all of it, and bringing a coarse form
            // all the way back to the source's voxel was measured at 45 s
            // for a detail size of 8 mm where this is under one.
            let halvings = (level + 1).min(REDONE_HALVINGS);
            let finer = pyramid.voxel_size(level) / (1_u32 << halvings) as f32;
            let brought_back = pyramid.level(level).resampled(finer);
            let redone = FormPyramid::build(&brought_back, halvings);
            (redone.level_count() == halvings).then_some((redone, halvings - 1))
        };
        let (lo, hi) = self.voxel_bounds(low, high);
        let decide = |preview: &BrickPreview| {
            if preview.mask == Some(PROTECTED) || preview.uniform.is_some() {
                BrickVerdict::Skip
            } else {
                BrickVerdict::Whole
            }
        };
        out.edit_voxels_where(lo, hi, 0, true, decide, |_, position, value, free| {
            // Only where both fields are measuring and not saturated: at
            // the edge of either band the difference is the clamp's, and
            // scaling it would draw the band's edge onto the model.
            if free <= 0.0 || value.abs() >= NARROW_BAND - 0.01 {
                return value;
            }
            let relief = pyramid.relief(level, position);
            if relief.abs() >= NARROW_BAND - 0.01 {
                return value;
            }
            let mut form = relief * form_voxel;
            if let Some((again, last)) = &again {
                let twice = again.relief(*last, position);
                if twice.abs() < NARROW_BAND - 0.01 {
                    // What the smoothing took off this shape, put back.
                    form += (relief - twice) * form_voxel;
                }
            }
            let target = (form + gain * (value - form)).clamp(INSIDE, OUTSIDE);
            value + (target - value) * free
        });
        out.mark_everything_dirty();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTRE: Vec3 = Vec3::splat(64.0);
    const RADIUS: f32 = 40.0;
    /// How far the ripple stands either side of the ball's own surface, in
    /// voxels, and how many of them go round from pole to pole and back.
    const RIPPLE: f32 = 0.8;
    const CYCLES: f32 = 31.0;
    /// The scale the tests bulk at: a form voxel of eight, over a ripple
    /// eight voxels from crest to crest.
    const LEVEL: usize = 2;

    /// Every voxel of the fixture's box holds the same value in both, to
    /// the bit. Values and not bricks: a copy may keep a brick that is all
    /// one value as that value, which is the same field stored differently.
    #[track_caller]
    fn assert_same_values(one: &Volume, other: &Volume, what: &str) {
        for z in 0..128 {
            for y in 0..128 {
                for x in 0..128 {
                    let voxel = IVec3::new(x, y, z);
                    assert_eq!(
                        one.sample_voxel(voxel).to_bits(),
                        other.sample_voxel(voxel).to_bits(),
                        "{what}: {voxel} differs"
                    );
                }
            }
        }
    }

    fn latitude(direction: Vec3) -> f32 {
        // `normalize_or_zero`, for the one voxel at the very centre, where
        // there is no direction. The first version of this divided nothing
        // by nothing there and seeded the fixture with a NaN, which every
        // read near it then picked up; nothing noticed until a test looked
        // at the inside of the ball and not only at its surface.
        direction.normalize_or_zero().z.clamp(-1.0, 1.0).asin()
    }

    /// A ball with ripples running round it like the grooves of a screw
    /// thread, redistanced so that it is a distance field and not just a
    /// surface.
    fn rippled_ball() -> Volume {
        let mut volume = Volume::new(1.0);
        let reach = Vec3::splat(RADIUS + 8.0);
        volume.edit_box(CENTRE - reach, CENTRE + reach, |position, _| {
            let offset = position - CENTRE;
            let surface = RADIUS + RIPPLE * (CYCLES * latitude(offset)).cos();
            (offset.length() - surface).clamp(INSIDE, OUTSIDE)
        });
        volume.redistance();
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// How far out the surface is along a direction from the centre.
    fn radius_along(volume: &Volume, direction: Vec3) -> f32 {
        let direction = direction.normalize();
        let mut radius = RADIUS - 12.0;
        while radius < RADIUS + 20.0 && volume.sample_world(CENTRE + direction * radius) < 0.0 {
            radius += 0.25;
        }
        // Between the last inside sample and the first outside one.
        let (mut inside, mut outside) = (radius - 0.25, radius);
        for _ in 0..16 {
            let middle = (inside + outside) * 0.5;
            if volume.sample_world(CENTRE + direction * middle) < 0.0 {
                inside = middle;
            } else {
                outside = middle;
            }
        }
        (inside + outside) * 0.5
    }

    /// The mean radius and the ripple's amplitude, read off the surface
    /// along a fan of directions across the ripples, on the +x side.
    fn mean_and_ripple(volume: &Volume) -> (f32, f32) {
        let radii: Vec<f32> = (0..400)
            .map(|step| {
                // Over six whole cycles about the equator.
                let lat = (step as f32 / 400.0 - 0.5) * (6.0 * std::f32::consts::TAU / CYCLES);
                radius_along(volume, Vec3::new(lat.cos(), 0.0, lat.sin()))
            })
            .collect();
        let mean = radii.iter().sum::<f32>() / radii.len() as f32;
        // A sinusoid's amplitude is root two times its deviation.
        let variance =
            radii.iter().map(|r| (r - mean) * (r - mean)).sum::<f32>() / radii.len() as f32;
        (mean, (2.0 * variance).sqrt())
    }

    #[test]
    fn the_fixture_is_a_ball_with_a_ripple_on_it() {
        let ball = rippled_ball();
        for z in 0..128 {
            for y in 0..128 {
                for x in 0..128 {
                    let value = ball.sample_voxel(IVec3::new(x, y, z));
                    assert!(value.is_finite(), "the fixture holds {value} at {x} {y} {z}");
                }
            }
        }
        let (mean, ripple) = mean_and_ripple(&ball);
        assert!((mean - RADIUS).abs() < 0.1, "the ball's mean radius is {mean}");
        assert!((ripple - RIPPLE).abs() < 0.1, "the ripple stands {ripple} off the ball");
    }

    /// The measurement this module exists for, and the one the plan said to
    /// make first.
    ///
    /// A bulk moves the surface by what was asked and the ripple goes with
    /// it, by about the same however far: measured at 96% of what it was
    /// after 3 voxels and 98% after 9, the few percent being the one
    /// trilinear read. Offsetting the field, which is what Inflate does,
    /// moves the surface as far and fills the grooves as it goes: 80% left
    /// after 3 voxels, 61% after 6, 49% after 9.
    #[test]
    fn a_ripple_survives_a_bulk_and_not_an_inflation() {
        let ball = rippled_ball();
        let (_, before) = mean_and_ripple(&ball);
        let pyramid = FormPyramid::build(&ball, LEVEL + 1);
        assert_eq!(pyramid.level_count(), LEVEL + 1);

        for amount in [3.0_f32, -3.0, 6.0, -6.0, 9.0] {
            let mut bulked = ball.bulked(&pyramid, LEVEL, amount).expect("a bulk");
            bulked.redistance();
            let (mean, ripple) = mean_and_ripple(&bulked);
            assert!(
                (mean - (RADIUS + amount)).abs() < 0.05,
                "a bulk of {amount} put the surface at {mean}"
            );
            assert!(
                ripple > before * 0.92,
                "after a bulk of {amount} the ripple stands {ripple} where it stood {before}"
            );
        }

        // Far from the surface a bulk is solid or empty by which side of the
        // form it is on, without being worked out voxel by voxel. Both
        // sides, on a lattice through the whole box.
        let bulked = ball.bulked(&pyramid, LEVEL, 6.0).expect("a bulk");
        let (mut solid, mut empty) = (0, 0);
        for z in (0..128).step_by(3) {
            for y in (0..128).step_by(3) {
                for x in (0..128).step_by(3) {
                    let voxel = IVec3::new(x, y, z);
                    let from_surface = (voxel.as_vec3() - CENTRE).length() - (RADIUS + 6.0);
                    let value = bulked.sample_voxel(voxel);
                    if from_surface < -3.0 {
                        assert!(value < 0.0, "{voxel} is {from_surface} inside and reads {value}");
                        solid += 1;
                    } else if from_surface > 3.0 {
                        assert!(value > 0.0, "{voxel} is {from_surface} outside and reads {value}");
                        empty += 1;
                    }
                }
            }
        }
        assert!(solid > 5000 && empty > 5000, "{solid} solid and {empty} empty were checked");

        // The control: the same distances by offsetting the field, a voxel
        // at a time as a brush would do it, since one offset of several
        // would run off the end of the band.
        let mut inflated = ball.duplicated(IVec3::ZERO);
        let reach = Vec3::splat(RADIUS + 16.0);
        for (voxels, at_most) in [(6, 0.7), (9, 0.58)] {
            while {
                let (mean, _) = mean_and_ripple(&inflated);
                mean < RADIUS + voxels as f32 - 0.5
            } {
                inflated.edit_box(CENTRE - reach, CENTRE + reach, |_, value| {
                    (value - 1.0).clamp(INSIDE, OUTSIDE)
                });
                inflated.redistance();
            }
            let (_, ripple) = mean_and_ripple(&inflated);
            assert!(
                ripple < before * at_most,
                "inflating by {voxels} kept a ripple of {ripple} of {before}: the control shows \
                 nothing"
            );
        }
    }

    /// The mask scales the bulk: the free side grows by what was asked, the
    /// protected side does not move by a bit, and between them the surface
    /// is one surface.
    #[test]
    fn a_bulk_fades_out_under_the_mask() {
        let mut ball = rippled_ball();
        ball.edit_mask(IVec3::splat(0), IVec3::splat(128), |_, at, _| {
            // Protected for x below the centre, free above it, feathered
            // over sixteen voxels between.
            (((CENTRE.x + 8.0 - at.x) / 16.0).clamp(0.0, 1.0) * f32::from(PROTECTED)) as u8
        });
        let pyramid = FormPyramid::build(&ball, LEVEL + 1);
        let bulked = ball.bulked(&pyramid, LEVEL, 4.0).expect("a bulk");

        let grew = radius_along(&bulked, Vec3::X) - radius_along(&ball, Vec3::X);
        assert!((grew - 4.0).abs() < 0.3, "the free side grew by {grew}");
        let stayed = radius_along(&bulked, -Vec3::X) - radius_along(&ball, -Vec3::X);
        assert_eq!(stayed, 0.0, "the protected side moved");
        for z in 20..=108 {
            for y in 20..=108 {
                for x in 20..=50 {
                    let voxel = IVec3::new(x, y, z);
                    assert_eq!(
                        bulked.sample_voxel(voxel).to_bits(),
                        ball.sample_voxel(voxel).to_bits(),
                        "{voxel} is fully protected and changed"
                    );
                }
            }
        }
        // Half way across the feather it is half protected and has grown by
        // about half: the mask scales the amount, and is not a switch.
        let direction = Vec3::new(0.0, 1.0, 0.0);
        let half = radius_along(&bulked, direction) - radius_along(&ball, direction);
        assert!((1.2..2.8).contains(&half), "half way across the feather it grew by {half}");
        // Across the feather the growth runs from nothing to all of it and
        // never the wrong way: no step, and no dent.
        let mut last = 0.0_f32;
        for step in 0..=40 {
            let angle = std::f32::consts::PI * (1.0 - step as f32 / 40.0);
            let direction = Vec3::new(angle.cos(), angle.sin(), 0.0);
            let grown = radius_along(&bulked, direction) - radius_along(&ball, direction);
            assert!(
                grown > last - 0.35,
                "the bulk dips going round the feather: {grown} after {last}"
            );
            assert!((-0.2..=4.4).contains(&grown), "{grown} at step {step}");
            last = last.max(grown);
        }
    }

    /// Paint goes out with the surface it is on.
    #[test]
    fn a_bulk_carries_the_paint_with_the_surface() {
        let mut ball = rippled_ball();
        ball.edit_colour(
            IVec3::splat(0),
            IVec3::splat(128),
            |_, at, _, _| {
                if at.z > CENTRE.z { 3 } else { 0 }
            },
        );
        let pyramid = FormPyramid::build(&ball, LEVEL + 1);
        let bulked = ball.bulked(&pyramid, LEVEL, 5.0).expect("a bulk");
        let slot_on = |volume: &Volume, direction: Vec3| {
            let on_surface = CENTRE + direction.normalize() * radius_along(volume, direction);
            volume.colour().at(on_surface.round().as_ivec3())
        };
        for (direction, slot) in [
            (Vec3::new(1.0, 0.0, 0.5), 3),
            (Vec3::new(0.0, 1.0, 1.0), 3),
            (Vec3::new(-1.0, 0.3, 0.2), 3),
            (Vec3::Z, 3),
            (Vec3::new(1.0, 0.0, -0.5), 0),
            (Vec3::new(0.0, -1.0, -0.2), 0),
            (-Vec3::Z, 0),
        ] {
            assert_eq!(slot_on(&ball, direction), slot, "the fixture, along {direction}");
            assert_eq!(slot_on(&bulked, direction), slot, "after the bulk, along {direction}");
        }
        // The new surface is five voxels out, past the old band: paint that
        // stayed where it was would not be on it at all.
        assert!(radius_along(&bulked, Vec3::Z) > RADIUS + 4.0);

        // An unpainted body stays one, with nothing allocated for it.
        let plain = rippled_ball();
        let bulked = plain.bulked(&FormPyramid::build(&plain, LEVEL + 1), LEVEL, 5.0).unwrap();
        assert!(bulked.colour().is_empty(), "bulking an unpainted body painted it");
        assert!(bulked.paint().is_empty(), "bulking an unpainted body gave it paint");
    }

    /// And the paint proper ([`crate::paint::field`]) goes out with it the
    /// same way: a colour and its coverage, at the surface five voxels on.
    #[test]
    fn a_bulk_carries_the_rgba_paint_with_the_surface() {
        const RED: [u8; 4] = [200, 30, 20, 255];
        const BARE: [u8; 4] = crate::paint::field::BARE;
        let mut ball = rippled_ball();
        ball.edit_paint(IVec3::splat(0), IVec3::splat(128), |_, at, _, _| {
            if at.z > CENTRE.z { RED } else { BARE }
        });
        let pyramid = FormPyramid::build(&ball, LEVEL + 1);
        let bulked = ball.bulked(&pyramid, LEVEL, 5.0).expect("a bulk");
        let paint_on = |volume: &Volume, direction: Vec3| {
            let on_surface = CENTRE + direction.normalize() * radius_along(volume, direction);
            volume.paint().at(on_surface.round().as_ivec3())
        };
        for (direction, paint) in [
            (Vec3::new(1.0, 0.0, 0.5), RED),
            (Vec3::new(0.0, 1.0, 1.0), RED),
            (Vec3::Z, RED),
            (Vec3::new(1.0, 0.0, -0.5), BARE),
            (-Vec3::Z, BARE),
        ] {
            assert_eq!(paint_on(&ball, direction), paint, "the fixture, along {direction}");
            assert_eq!(paint_on(&bulked, direction), paint, "after the bulk, along {direction}");
        }
        assert!(radius_along(&bulked, Vec3::Z) > RADIUS + 4.0);
        // And none of it is left where the surface was: paint is only where
        // paint can be, which five voxels under the new surface is not.
        let mut painted = 0;
        for z in 0..128 {
            for y in 0..128 {
                for x in 0..128 {
                    let cell = IVec3::new(x, y, z);
                    if bulked.paint().at(cell) != BARE {
                        painted += 1;
                        assert!(
                            crate::paint::field::PaintField::paintable(bulked.sample_voxel(cell)),
                            "paint left behind at {cell}, {} under the surface",
                            -bulked.sample_voxel(cell)
                        );
                    }
                }
            }
        }
        assert!(painted > 10_000, "only {painted} voxels of the bulked ball are painted");

        // Under the mask nothing moves, and the paint on it does not either.
        // Painted on the inner half of the band only, so that paint pulled
        // from deeper in would be seen to be.
        let mut held = rippled_ball();
        let inner = rippled_ball();
        held.edit_paint(IVec3::splat(0), IVec3::splat(128), |cell, _, _, _| {
            if inner.sample_voxel(cell) < 0.0 { RED } else { BARE }
        });
        held.edit_mask(IVec3::splat(0), IVec3::splat(128), |_, at, _| {
            if at.x < CENTRE.x { PROTECTED } else { 0 }
        });
        let pyramid = FormPyramid::build(&held, LEVEL + 1);
        let bulked = held.bulked(&pyramid, LEVEL, 5.0).expect("a bulk");
        let mut compared = 0;
        for z in 16..112 {
            for y in 16..112 {
                for x in 16..(CENTRE.x as i32 - 2) {
                    let cell = IVec3::new(x, y, z);
                    assert_eq!(
                        bulked.paint().at(cell),
                        held.paint().at(cell),
                        "paint under the mask changed at {cell}"
                    );
                    compared += usize::from(held.paint().at(cell) == RED);
                }
            }
        }
        assert!(compared > 5_000, "only {compared} painted voxels were under the mask");
    }

    /// What is refused, and that nothing asked for is nothing done.
    #[test]
    fn a_bulk_the_form_cannot_steer_is_refused_and_none_at_all_changes_nothing() {
        let ball = rippled_ball();
        let pyramid = FormPyramid::build(&ball, LEVEL + 1);
        assert_same_values(&ball.bulked(&pyramid, LEVEL, 0.0).unwrap(), &ball, "a bulk of nothing");

        assert_eq!(ball.bulked(&pyramid, LEVEL + 1, 1.0).err(), Some(FormError::NoSuchLevel));
        assert_eq!(ball.bulked(&pyramid, LEVEL, f32::NAN).err(), Some(FormError::NotFinite));
        assert_eq!(ball.bulked(&pyramid, LEVEL, f32::INFINITY).err(), Some(FormError::NotFinite));
        // The most each scale carries: the form's band, less a form voxel
        // for the detail and less the source's own band.
        for (level, most) in [(0, 0.5), (1, 4.0), (2, 11.0)] {
            assert_eq!(most_voxels(level), most, "level {level}");
        }
        let refused = ball.bulked(&pyramid, 0, 2.0).err();
        assert_eq!(refused, Some(FormError::TooFar { most_mm: 0.5 }));
        assert!(refused.unwrap().to_string().contains("0.50 mm"));
        assert!(ball.bulked(&pyramid, 1, 4.0).is_ok(), "the most a scale allows was refused");
        assert!(ball.bulked(&pyramid, 1, -4.01).is_err());
        for error in [FormError::NoSuchLevel, FormError::NotFinite] {
            assert!(!error.to_string().is_empty());
        }

        // An empty volume is an empty volume.
        let empty = Volume::new(1.0);
        assert!(empty.bulked(&pyramid, LEVEL, 1.0).unwrap().world_bounds().is_none());
    }

    #[test]
    fn a_detail_size_in_millimetres_names_a_level() {
        // Level 0 is a form voxel of twice the voxel, level 1 four times.
        for (voxel, detail, level) in [
            (0.25, 0.5, 0),
            (0.25, 1.0, 1),
            (0.25, 2.0, 2),
            (0.25, 2.6, 2),
            (0.1, 1.6, 3),
            (1.0, 8.0, 2),
            // Smaller than a form can be is the finest form, and nonsense is
            // too, and larger than a pyramid goes is its coarsest level.
            (0.25, 0.1, 0),
            (0.25, 0.0, 0),
            (0.25, -3.0, 0),
            (0.25, f32::NAN, 0),
            (0.25, 1.0e9, MAX_LEVELS - 1),
        ] {
            assert_eq!(level_for(voxel, detail), level, "{detail} mm at a voxel of {voxel}");
        }
    }

    /// The detail gain: at nothing the ripple is mostly gone, at a half it
    /// is halved, at two it stands twice as far, at one nothing happens,
    /// and through all of it the ball under the ripple stays the size it
    /// was.
    #[test]
    fn the_detail_gain_scales_the_ripple_and_keeps_the_ball_its_size() {
        let ball = rippled_ball();
        let (mean, before) = mean_and_ripple(&ball);
        let pyramid = FormPyramid::build(&ball, LEVEL + 1);
        assert_same_values(
            &ball.with_detail_gain(&pyramid, LEVEL, 1.0).unwrap(),
            &ball,
            "a gain of 1",
        );

        let with = |gain: f32| {
            let mut out = ball.with_detail_gain(&pyramid, LEVEL, gain).expect("a gain");
            out.redistance();
            mean_and_ripple(&out)
        };
        for (gain, tolerance) in [(0.0, 0.3), (0.5, 0.15), (2.0, 0.35)] {
            let (moved_mean, ripple) = with(gain);
            assert!(
                (ripple / before - gain).abs() < tolerance,
                "at a gain of {gain} the ripple is {ripple} of {before}"
            );
            // To within what the correction leaves, which on this small
            // ball is a tenth of a voxel the other way at a gain of nothing
            // (measured: 40.106 from 39.998) against the 0.44 it corrects.
            assert!(
                (moved_mean - mean).abs() < 0.15 * (1.0 - gain).abs().max(0.3),
                "a gain of {gain} moved the ball from {mean} to {moved_mean}"
            );
        }
        // The control for "keeps the ball its size": the form as the pyramid
        // has it is 0.44 of a voxel inside this ball, which is how far a
        // gain of nothing would bring it in without the correction.
        let on_the_form = pyramid.relief(LEVEL, CENTRE + Vec3::new(0.6, 0.0, 0.8) * RADIUS) * 8.0;
        assert!(
            (0.25..0.7).contains(&on_the_form),
            "the uncorrected form is {on_the_form} voxels inside the ball"
        );

        assert_eq!(
            ball.with_detail_gain(&pyramid, pyramid.level_count(), 2.0).err(),
            Some(FormError::NoSuchLevel)
        );
        assert_eq!(ball.with_detail_gain(&pyramid, LEVEL, -1.0).err(), Some(FormError::NotFinite));
        assert_eq!(
            ball.with_detail_gain(&pyramid, LEVEL, f32::NAN).err(),
            Some(FormError::NotFinite)
        );
    }

    /// The mask holds the gain off, as it does the bulk.
    #[test]
    fn the_detail_gain_leaves_what_the_mask_protects() {
        let mut ball = rippled_ball();
        ball.edit_mask(IVec3::splat(0), IVec3::splat(128), |_, at, _| {
            if at.x < CENTRE.x { PROTECTED } else { 0 }
        });
        let pyramid = FormPyramid::build(&ball, LEVEL + 1);
        let flattened = ball.with_detail_gain(&pyramid, LEVEL, 0.0).expect("a gain");
        let mut changed = 0;
        for z in 16..112 {
            for y in 16..112 {
                for x in 16..112 {
                    let voxel = IVec3::new(x, y, z);
                    let same = flattened.sample_voxel(voxel).to_bits()
                        == ball.sample_voxel(voxel).to_bits();
                    if (x as f32) < CENTRE.x {
                        assert!(same, "{voxel} is protected and changed");
                    } else {
                        changed += usize::from(!same);
                    }
                }
            }
        }
        assert!(changed > 5000, "the free half changed at only {changed} voxels");

        // Half protected, half the change: the mask scales it.
        let free_result = rippled_ball().with_detail_gain(&pyramid, LEVEL, 0.0).expect("a gain");
        let mut half = rippled_ball();
        half.edit_mask(IVec3::splat(0), IVec3::splat(128), |_, _, _| 128);
        let half_result = half.with_detail_gain(&pyramid, LEVEL, 0.0).expect("a gain");
        let share = 1.0 - 128.0 / f32::from(PROTECTED);
        let mut compared = 0;
        for z in (16..112).step_by(2) {
            for y in (16..112).step_by(2) {
                for x in (16..112).step_by(2) {
                    let voxel = IVec3::new(x, y, z);
                    let was = ball.sample_voxel(voxel);
                    let whole = free_result.sample_voxel(voxel) - was;
                    if whole.abs() < 0.05 {
                        continue;
                    }
                    let part = half_result.sample_voxel(voxel) - was;
                    assert!(
                        (part - whole * share).abs() < 0.01,
                        "{voxel} moved {part} half protected and {whole} free"
                    );
                    compared += 1;
                }
            }
        }
        assert!(compared > 2000, "only {compared} voxels were compared");
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    #[test]
    #[ignore = "a measurement, not a claim: run by hand, in release"]
    fn what_a_bulk_costs() {
        let mut volume = Volume::new(0.25);
        volume.seed_sphere(Vec3::ZERO, 30.0);
        let level = level_for(0.25, 2.0);
        let started = std::time::Instant::now();
        let pyramid = FormPyramid::build(&volume, level + 2);
        let built = started.elapsed();
        let started = std::time::Instant::now();
        let mut bulked = volume.bulked(&pyramid, level, 1.0).expect("a bulk");
        let warped = started.elapsed();
        let started = std::time::Instant::now();
        bulked.redistance();
        let redistanced = started.elapsed();
        let started = std::time::Instant::now();
        let gained = volume.with_detail_gain(&pyramid, level, 1.5).expect("a gain");
        let gain = started.elapsed();
        // What taking all the detail off does to the size of a ball with
        // no detail on it: how far the form sits inside it, over a spread
        // of directions and not along one axis, where the lattice flatters
        // it.
        let directions: Vec<Vec3> = (0..300)
            .map(|index| {
                // A spiral over the sphere.
                let z = 1.0 - 2.0 * (index as f32 + 0.5) / 300.0;
                let turn = index as f32 * 2.399_963;
                let ring = (1.0 - z * z).sqrt();
                Vec3::new(ring * turn.cos(), ring * turn.sin(), z)
            })
            .collect();
        let radius = |volume: &Volume, direction: Vec3| {
            let mut at = 25.0;
            while at < 35.0 && volume.sample_world(direction * at) < 0.0 {
                at += 0.002;
            }
            at
        };
        for detail in [0.5_f32, 1.0, 2.0, 4.0, 8.0] {
            let level = level_for(0.25, detail);
            let pyramid = FormPyramid::build(&volume, level + 1);
            let mut form = volume.with_detail_gain(&pyramid, level, 0.0).expect("a gain");
            form.redistance();
            let moved: Vec<f32> = directions
                .iter()
                .map(|direction| radius(&form, *direction) - radius(&volume, *direction))
                .collect();
            let mean = moved.iter().sum::<f32>() / moved.len() as f32;
            let worst = moved
                .iter()
                .fold(0.0_f32, |worst, m| if m.abs() > worst.abs() { *m } else { worst });
            eprintln!(
                "detail {detail} mm (level {level}): the ball's surface moves {mean:+.3} mm on \
                 average and {worst:+.3} at worst"
            );
        }
        eprintln!(
            "{} bricks at level {level}: pyramid {built:?}, bulk of 1 mm {warped:?}, \
             redistance {redistanced:?}, detail gain {gain:?} ({} bricks after)",
            volume.stats().dense_bricks,
            gained.stats().dense_bricks
        );
    }
}
