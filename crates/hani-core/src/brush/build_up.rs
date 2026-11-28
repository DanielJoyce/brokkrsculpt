// SPDX-License-Identifier: AGPL-3.0-only

//! Where a stroke that builds up holds its plane: how fast clay builds.
//!
//! Issue 179 of this fork. A brush that levels to a plane held off the
//! surface (Clay, Scrape, and any brush of the studio's with a `plane`)
//! blends each voxel toward that plane by the stamp's weight. A stroke
//! that BUILDS UP sets every stamp down on the surface as the stamps before
//! it left it, so what one stamp lifts the middle of the surface by is what
//! the stroke adds there, again and again: the blend times the plane's
//! height, in voxels.
//!
//! Until this the plane was held [`super::CLAY_OFFSET`] of the radius up
//! whatever the strength, and nothing capped what a stamp lifted. With a
//! 3 mm brush at 0.25 mm voxels that is 4.2 voxels, so a stamp at a weight
//! of a half lifted the middle 2.1 voxels (arithmetic), where Draw is held to
//! [`super::MAX_STAMP_VOXELS`] a stamp "so one stamp can never saturate the
//! narrow band". Measured with `measure_what_strength_does_to_a_pass`
//! (`hani-app`, `app/pen_strokes.rs`): a pass of clay was 4.4 times as tall
//! as Draw's at the same strength, and from a strength of about a half up
//! it ran away, 28 mm off a 30 mm ball from one pass at 0.75. Said by the
//! maintainer on 2026-11-27: "strength should affect build up".
//!
//! # The rule
//!
//! The plane is held `radius * plane * gain` off the stamp's centre, where
//! `plane` is [`super::CLAY_OFFSET`] for the classic Clay or the brush's own
//! `plane`, and `gain` is the strength times the pen's pressure, as every
//! kernel already has it. And it is never held further off than
//! [`super::MAX_STAMP_VOXELS`], which is Draw's own cap read the same way:
//! Draw moves the surface `weight * min(radius share, cap)` a stamp, and
//! clay now lifts the middle by `weight * min(plane's height, cap)`. A
//! weight is at most one, so no stamp of a stroke that builds up lifts the
//! middle further than a stamp of Draw can move it.
//!
//! **Tried first and dropped**: capping the lift (the weight times the
//! plane's height) at the cap, rather than the plane's height. A plane held
//! `cap / gain` off is the same rule wherever the cap does not bite, and
//! from a strength of about a half up, with the 3 mm brush, every stamp
//! lifts the middle a whole voxel: one pass measured 1.04 mm at 0.5 and
//! 1.13 at 1.0, which is the slider doing nothing over its top half. With
//! the rule as it is, 0.54 and 1.13. See the module documentation of
//! `app/pen_strokes.rs` for both sets of numbers.
//!
//! # What it does not touch
//!
//! * A stroke that lays **one layer** (`compound/layer.rs`) holds its plane
//!   where it always has: a layer is laid once, from the field before the
//!   stroke, and its plane is how thick the layer is.
//! * Flatten, and every brush whose plane is nothing: nothing times the gain
//!   is nothing.
//! * The sign. Scrape's plane is under the surface, and it is held under it
//!   by the same rule.

use super::MAX_STAMP_VOXELS;

/// How far off a stamp's centre a stroke that builds up holds its plane,
/// along the stamp's normal and in world units, signed as `plane` is: the
/// caller turns it to the side the brush builds toward.
///
/// `plane` is in radii of the brush, `gain` is the strength times the
/// pressure. The three kernels that build up (the classic one in
/// `brush.rs`, the engine's Clay and the studio's compound brush) each
/// call this with the same numbers and nothing else, which is what keeps
/// them the same to the bit.
#[inline]
pub(super) fn plane_height(radius: f32, plane: f32, gain: f32, voxel_size: f32) -> f32 {
    // Over a gain of one the blend's own weight is over one too, and the
    // cap is shared out so the middle still moves no further than it.
    // A division by one is exact, so up to a gain of one this is the cap.
    let most = MAX_STAMP_VOXELS * voxel_size / gain.max(1.0);
    (radius * plane * gain).clamp(-most, most)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use proptest::prelude::*;

    use super::*;
    use crate::brush::compound::{Actions, Compound, Profile, Side};
    use crate::brush::{Brush, BrushDirection, BrushKind, BrushScratch, CLAY_OFFSET, Stamp};
    use crate::volume::Volume;

    #[test]
    fn at_full_gain_the_plane_is_where_it_was_until_it_is_capped() {
        // A brush too small for the cap to bite: two voxels across, at a
        // voxel of one. 2 * 0.35 = 0.7 voxels, under the cap.
        assert_eq!(plane_height(2.0, CLAY_OFFSET, 1.0, 1.0), 2.0 * CLAY_OFFSET);
        // The 3 mm brush the fault was measured with, at 0.25 mm voxels:
        // 4.2 voxels before, one now.
        assert_eq!(plane_height(3.0, CLAY_OFFSET, 1.0, 0.25), 0.25);
        // And at a gain that keeps it under the cap, in proportion to it.
        let low = plane_height(3.0, CLAY_OFFSET, 0.1, 0.25);
        assert!((low - 3.0 * CLAY_OFFSET * 0.1).abs() < 1e-6, "{low}");
    }

    #[test]
    fn a_plane_of_nothing_is_nothing_and_a_plane_under_the_surface_stays_under() {
        assert_eq!(plane_height(3.0, 0.0, 0.7, 0.25), 0.0);
        let scrape = plane_height(3.0, -0.15, 0.3, 0.25);
        assert!(scrape < 0.0, "{scrape}");
        assert_eq!(scrape, -plane_height(3.0, 0.15, 0.3, 0.25));
        assert_eq!(plane_height(3.0, -1.0, 1.0, 0.25), -0.25);
    }

    proptest! {
        /// The plane's height, as the rule says it: in proportion to the
        /// gain, never past the cap, and never further for less gain.
        #[test]
        fn the_height_follows_the_gain_and_keeps_to_the_cap(
            radius in 0.05_f32..40.0,
            plane in -1.0_f32..=1.0,
            gain in 0.0_f32..=1.0,
            less in 0.0_f32..=1.0,
            voxel_size in 0.02_f32..2.0,
        ) {
            let height = plane_height(radius, plane, gain, voxel_size);
            prop_assert!(height.abs() <= MAX_STAMP_VOXELS * voxel_size);
            prop_assert!(height * plane >= 0.0, "the plane changed side");
            let lower = plane_height(radius, plane, gain * less, voxel_size);
            prop_assert!(lower.abs() <= height.abs());
        }
    }

    /// A ball 32 voxels in radius at `voxel_size`, its top at the origin,
    /// so that the middle of a stamp there is a voxel on the surface.
    fn slab(voxel_size: f32) -> Volume {
        let mut volume = Volume::new(voxel_size);
        let radius = 32.0 * voxel_size;
        volume.seed_sphere(Vec3::new(0.0, 0.0, -radius), radius);
        volume
    }

    /// How far the surface stands over z = 0 at the origin, in voxels.
    fn lift(volume: &Volume) -> f32 {
        -volume.sample_world(Vec3::ZERO)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// What the rule is for: no one stamp of a brush that builds up
        /// lifts the middle of the surface further than Draw's cap, for any
        /// size of brush, strength, pressure or voxel, the classic Clay and
        /// a compound brush with a plane of any height alike. Before this
        /// change a stamp of the 3 mm brush at full strength lifted it 4.2
        /// voxels.
        #[test]
        fn no_stamp_lifts_the_middle_past_the_cap(
            voxels_across in 2.0_f32..24.0,
            strength in 0.01_f32..=1.0,
            pressure in 0.01_f32..=1.0,
            voxel_size in 0.1_f32..1.0,
            plane in 0.05_f32..=1.0,
        ) {
            let radius = voxels_across * voxel_size * 0.5;
            let stamp = Stamp::new(Vec3::ZERO, Vec3::Z, BrushDirection::Add).with_pressure(pressure);
            let mut scratch = BrushScratch::new();

            let mut volume = slab(voxel_size);
            let before = lift(&volume);
            let clay = Brush { kind: BrushKind::Clay, radius, strength, ..Brush::default() };
            clay.apply(&mut volume, &stamp, &mut scratch);
            let classic = lift(&volume) - before;
            prop_assert!(classic <= MAX_STAMP_VOXELS + 1e-4, "Clay lifted the middle {classic} voxels");

            let mut volume = slab(voxel_size);
            let held_off = Compound {
                actions: Actions { plane, ..Actions::CLAY },
                radius,
                strength,
                profile: Profile::default(),
                pattern: crate::pattern::Pattern::default(),
            };
            held_off.apply(&mut volume, &stamp, None, &mut scratch);
            let compound = lift(&volume) - before;
            prop_assert!(compound <= MAX_STAMP_VOXELS + 1e-4, "plane {plane} lifted the middle {compound} voxels");
        }
    }

    /// And the weight sets how far, as it does for Draw: half the pressure
    /// lifts the middle less, and a plane under the surface cuts it.
    #[test]
    fn half_the_pressure_lifts_less_and_a_plane_under_the_surface_cuts() {
        let voxel_size = 0.25;
        let mut scratch = BrushScratch::new();
        let clay = Brush { kind: BrushKind::Clay, radius: 3.0, strength: 0.3, ..Brush::default() };
        let by = |pressure: f32, scratch: &mut BrushScratch| {
            let mut volume = slab(voxel_size);
            let stamp =
                Stamp::new(Vec3::ZERO, Vec3::Z, BrushDirection::Add).with_pressure(pressure);
            clay.apply(&mut volume, &stamp, scratch);
            lift(&volume)
        };
        let (full, half) = (by(1.0, &mut scratch), by(0.5, &mut scratch));
        assert!(0.0 < half && half < full, "half the pressure lifted {half}, full {full}");

        let mut volume = slab(voxel_size);
        let scrape = Compound {
            actions: Actions { level: 1.0, plane: -0.15, side: Side::Cut, ..Actions::CLAY },
            radius: 3.0,
            strength: 0.3,
            profile: Profile::default(),
            pattern: crate::pattern::Pattern::default(),
        };
        scrape.apply(
            &mut volume,
            &Stamp::new(Vec3::ZERO, Vec3::Z, BrushDirection::Add),
            None,
            &mut scratch,
        );
        let cut = lift(&volume);
        assert!(
            (-MAX_STAMP_VOXELS - 1e-4..0.0).contains(&cut),
            "Scrape moved the middle {cut} voxels"
        );
    }
}
