// SPDX-License-Identifier: AGPL-3.0-only

//! A mask stroke that ends in the brush's own falloff.
//!
//! Found by the maintainer on 2026-11-27: the brushes "instead of a smooth
//! fade to 0% at the ring ... almost always has some kind of edge / border
//! when painting / masking".
//!
//! [`Brush::apply_mask`] blends each voxel toward its target by the stamp's
//! weight, once a stamp, and a stroke is a stamp every few voxels. So a
//! voxel the brush passes over is blended dozens of times, and with eight
//! bits rounded to nearest it settles where one more step is under half a
//! level: at `255 - 0.5 / weight`. Under the smooth falloff a weight of a
//! two hundredth is 96 percent of the way out, so a stroke was protected
//! nearly to the full at 96 percent of its radius and nothing beyond: the
//! ring the maintainer saw. `apply_mask`'s own comment measured it: 300
//! stamps at radius 8 left `[255 x 8, 0]`.
//!
//! A [`MaskStroke`] gives a voxel the most weight any stamp of the stroke
//! gave it, applied once to what the voxel held when the stroke first
//! reached it: `before + (target - before) * most`. A stroke's middle is
//! the brush's strength of the way to the target, and across it the
//! protection follows the falloff out to nothing at the radius, however
//! many stamps there are. A second stroke over the first goes the same
//! share again of what is left, as a second coat of paint does
//! (`paint/stroke.rs` lays its coats the same way).
//!
//! Blur has no target and is left to [`Brush::apply_mask`].

use std::cell::RefCell;
use std::collections::HashMap;

use glam::{IVec3, Vec3};

use crate::brick::{BRICK_VOXELS, BrickCoord, brick_index};
use crate::brush::{Brush, BrushScratch, MaskOp, Stamp, Symmetry};
use crate::mask::{PROTECTED, UNMASKED};
use crate::pattern::Prepared;
use crate::volume::Volume;

/// What one stroke has done to the voxels of one brick.
struct Held {
    /// What each voxel held when the stroke first reached it.
    before: Box<[u8; BRICK_VOXELS]>,
    /// The most weight any stamp of the stroke gave each voxel, in 255ths.
    /// Nought is a voxel the stroke has not reached.
    most: Box<[u8; BRICK_VOXELS]>,
}

impl Held {
    fn new() -> Self {
        let zeros =
            || vec![0_u8; BRICK_VOXELS].into_boxed_slice().try_into().expect("BRICK_VOXELS");
        Self { before: zeros(), most: zeros() }
    }
}

/// One stroke of the mask brush. See the module documentation.
#[derive(Default)]
pub struct MaskStroke {
    held: HashMap<BrickCoord, Held>,
}

impl MaskStroke {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new stroke: what the last one reached is forgotten.
    pub fn begin(&mut self) {
        self.held.clear();
    }

    /// Mask under one stamp of the stroke, and under its mirrors when
    /// symmetry is on, as [`Brush::apply_mask_symmetric`] does.
    #[allow(clippy::too_many_arguments)]
    pub fn stamp(
        &mut self,
        brush: &Brush,
        volume: &mut Volume,
        stamp: &Stamp,
        op: MaskOp,
        symmetry: Symmetry,
        centre: Vec3,
        scratch: &mut BrushScratch,
    ) {
        if op == MaskOp::Blur {
            brush.apply_mask_symmetric(volume, stamp, op, symmetry, centre, scratch);
            return;
        }
        self.one(brush, volume, stamp, op);
        if symmetry.is_off() {
            return;
        }
        let mut twins = [*stamp; Symmetry::MAX_MIRRORS];
        let count = symmetry.mirrors(stamp, centre, &mut twins);
        for twin in &twins[..count] {
            self.one(brush, volume, twin, op);
        }
    }

    fn one(&mut self, brush: &Brush, volume: &mut Volume, stamp: &Stamp, op: MaskOp) {
        if brush.radius <= 0.0 || brush.strength <= 0.0 || stamp.pressure <= 0.0 {
            return;
        }
        let voxel_size = volume.voxel_size();
        let inverse_radius = 1.0 / brush.radius;
        let gain = (brush.strength * stamp.pressure).clamp(0.0, 1.0);
        let extent = Vec3::splat(brush.radius);
        let (lo, hi) = volume.voxel_bounds(stamp.centre - extent, stamp.centre + extent);
        let pattern = if brush.pattern.is_off() {
            Prepared::OFF
        } else {
            brush.pattern.prepare(voxel_size, stamp.normal, stamp.tangent)
        };
        let falloff = brush.falloff;
        let middle = stamp.centre;
        let radius_squared = brush.radius * brush.radius;
        let target = if op == MaskOp::Raise { f32::from(PROTECTED) } else { f32::from(UNMASKED) };

        let held = RefCell::new(std::mem::take(&mut self.held));
        volume.edit_mask(lo, hi, |voxel: IVec3, position: Vec3, protection: u8| {
            let offset = position - middle;
            if offset.length_squared() >= radius_squared {
                return protection;
            }
            let weight =
                falloff.weight(offset.length() * inverse_radius) * gain * pattern.weight(position);
            let weight = (weight.clamp(0.0, 1.0) * 255.0).round() as u8;
            if weight == 0 {
                return protection;
            }
            let coord = BrickCoord::containing(voxel);
            let local = voxel - coord.origin();
            let index = brick_index(local.x as usize, local.y as usize, local.z as usize);
            let mut held = held.borrow_mut();
            let brick = held.entry(coord).or_insert_with(Held::new);
            if brick.most[index] == 0 {
                brick.before[index] = protection;
            } else if weight <= brick.most[index] {
                return protection;
            }
            brick.most[index] = brick.most[index].max(weight);
            let before = f32::from(brick.before[index]);
            let next = before + (target - before) * f32::from(brick.most[index]) / 255.0;
            // Toward the target where it is within a level, as `apply_mask`
            // does, so a full weight lands on it exactly and an eraser
            // leaves no residue: see there.
            let next = match op {
                MaskOp::Raise if target - next < 1.0 => next.ceil(),
                MaskOp::Lower => next.floor(),
                _ => next.round(),
            };
            next.clamp(f32::from(UNMASKED), f32::from(PROTECTED)) as u8
        });
        self.held = held.into_inner();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{BrushDirection, FalloffCurve};

    /// A flat floor at z = 0 in a box, at one voxel a millimetre.
    fn floor() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::new(0.0, 0.0, -400.0), 400.0);
        volume
    }

    /// The protection along a line across a straight stroke down the y
    /// axis, from its middle line outward, one voxel a step.
    fn across(volume: &Volume, out: i32) -> Vec<u8> {
        (0..=out).map(|x| volume.mask().at(IVec3::new(x, 0, 0))).collect()
    }

    /// A stroke down the y axis, a stamp every `spacing` millimetres.
    fn stroke(volume: &mut Volume, brush: &Brush, op: MaskOp, spacing: f32) {
        let mut stroke = MaskStroke::new();
        stroke.begin();
        let mut scratch = BrushScratch::new();
        let mut y = -30.0;
        while y <= 30.0 {
            let stamp = Stamp::new(Vec3::new(0.0, y, 0.0), Vec3::Z, BrushDirection::Add);
            stroke.stamp(brush, volume, &stamp, op, Symmetry::default(), Vec3::ZERO, &mut scratch);
            y += spacing;
        }
    }

    /// A stroke's protection across it is the falloff, from full in the
    /// middle to nothing at the radius, however close the stamps are: the
    /// ring the maintainer saw was a stroke settling nearly solid out to
    /// the rim.
    #[test]
    fn a_stroke_fades_to_nothing_at_its_radius_however_many_stamps() {
        let brush = Brush { radius: 10.0, strength: 1.0, ..Brush::default() };
        for spacing in [0.5_f32, 2.0, 5.0] {
            let mut volume = floor();
            stroke(&mut volume, &brush, MaskOp::Raise, spacing);
            let profile = across(&volume, 11);
            assert_eq!(profile[0], PROTECTED, "{spacing}: {profile:?}");
            for x in 0..=11 {
                let falloff = FalloffCurve::Smooth.weight(x as f32 / 10.0);
                let want = (falloff * 255.0).round();
                assert!(
                    (f32::from(profile[x]) - want).abs() <= 2.0,
                    "spacing {spacing}: {x} mm out is {} where the falloff is {want}: {profile:?}",
                    profile[x]
                );
            }
            assert!(profile[10] == 0 && profile[11] == 0);
        }

        // What `apply_mask` stamp by stamp left, for the record: solid to 7
        // millimetres of the 10, and gone by the 10th. Run on 2026-11-27 it
        // was [255 x 7, 247, 190, 59, 0, 0].
        let mut volume = floor();
        let mut scratch = BrushScratch::new();
        let mut y = -30.0;
        while y <= 30.0 {
            let stamp = Stamp::new(Vec3::new(0.0, y, 0.0), Vec3::Z, BrushDirection::Add);
            brush.apply_mask(&mut volume, &stamp, MaskOp::Raise, &mut scratch);
            y += 0.5;
        }
        let old = across(&volume, 11);
        assert!(old[..7].iter().all(|level| *level == PROTECTED), "{old:?}");
        assert!(old[8] > 150 && old[10] == 0, "{old:?}");
    }

    /// A stroke at half strength goes half way, and a second goes half of
    /// what is left; an eraser at full strength leaves nothing in its
    /// middle and a fade at its edge.
    #[test]
    fn strokes_go_their_strength_of_the_way_and_an_eraser_clears() {
        let half = Brush { radius: 8.0, strength: 0.5, ..Brush::default() };
        let mut volume = floor();
        stroke(&mut volume, &half, MaskOp::Raise, 1.0);
        assert_eq!(volume.mask().at(IVec3::ZERO), 128);
        stroke(&mut volume, &half, MaskOp::Raise, 1.0);
        assert_eq!(volume.mask().at(IVec3::ZERO), 192);

        let full = Brush { radius: 8.0, strength: 1.0, ..Brush::default() };
        stroke(&mut volume, &full, MaskOp::Raise, 1.0);
        assert_eq!(volume.mask().at(IVec3::ZERO), PROTECTED);
        let was = across(&volume, 8);
        let eraser = Brush { radius: 4.0, ..full };
        stroke(&mut volume, &eraser, MaskOp::Lower, 1.0);
        let profile = across(&volume, 8);
        assert_eq!(profile[0], UNMASKED, "{profile:?}");
        for x in 1..4 {
            // What was there, less the eraser's falloff of it.
            let left = 1.0 - FalloffCurve::Smooth.weight(x as f32 / 4.0);
            let want = f32::from(was[x]) * left;
            assert!((f32::from(profile[x]) - want).abs() <= 1.5, "{x}: {profile:?} from {was:?}");
        }
        assert_eq!(profile[4..], was[4..], "past the eraser's radius");
    }
}
