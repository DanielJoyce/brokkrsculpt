// SPDX-License-Identifier: AGPL-3.0-only

//! Extract: the masked part of a surface thickened into a plate of its own.
//!
//! A user masks an area, a breastplate or a shoulder pad's footprint, and
//! asks for it as a separate piece of a given thickness. The plate follows
//! the surface: every point of it is within the thickness of the masked
//! area, measured along the surface's own normal.
//!
//! # The shape
//!
//! `s` is the signed distance to the whole surface, positive outside. The
//! plate spans `inner <= s <= outer`, which is the slab
//! `max(s - outer, inner - s)`:
//!
//! * `outer = offset + (1 - inward) * thickness`
//! * `inner = offset - inward * thickness`
//!
//! so `inward` is the share of the thickness that goes into the body, and
//! `offset` moves the whole plate out (or in) along the normal.
//!
//! The plate stops at the edge of the mask. A point is in the plate's
//! footprint when the nearest surface to it is masked: `lateral` is half
//! the distance to the nearest masked surface less the distance to the
//! nearest unmasked one, so it crosses zero, continuously, where the two
//! are equal, and the plate's side wall stands square to the surface.
//! The plate is `max(slab, lateral)`.
//!
//! Distances deeper than the band come from [`crate::reach`], spread twice:
//! once from the masked surface and once from the rest. Near the surface,
//! inside the band, `s` is the stored field itself, which is exact.
//!
//! A voxel is masked at half protection or more, so a feathered mask's edge
//! is the middle of its feather.
//!
//! # Not done
//!
//! * **The paint is not carried** onto the plate. The plate's surface is a
//!   new surface at another distance, and what was painted is on the body.
//! * **The side wall is square, not bevelled.** A chamfer would be a third
//!   term in the max.

use glam::Vec3;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::brick::{BRICK_DIM, Brick, BrickCoord, INSIDE, NARROW_BAND, OUTSIDE, brick_index};
use crate::reach::{Reach, limit_for, surface_seeds};
use crate::shell::{settled, voxel_of};
use crate::volume::Volume;
use contracts::debug_requires;

/// Half protection: at or over this a voxel is masked.
const MASKED: u8 = 128;

/// Feathered, any protection at all puts a voxel in the plate's footprint,
/// at a thickness that share of the whole.
const FEATHERED: u8 = 1;

/// Feathered, the thinnest a plate is let be, in voxels: where its share of
/// the thickness is less, it ends.
const MOST_FEATHER_EDGE: f32 = 1.0;

impl Volume {
    /// The mask's protection at a point in voxel units, 0..=1, between the
    /// eight voxels round it.
    fn mask_between(&self, point: Vec3) -> f32 {
        let base = point.floor();
        let f = point - base;
        let corner = base.as_ivec3();
        let mut sum = 0.0;
        for index in 0..8_i32 {
            let step = glam::IVec3::new(index & 1, (index >> 1) & 1, (index >> 2) & 1);
            let weight = (if step.x == 1 { f.x } else { 1.0 - f.x })
                * (if step.y == 1 { f.y } else { 1.0 - f.y })
                * (if step.z == 1 { f.z } else { 1.0 - f.z });
            sum += weight * f32::from(self.mask().at(corner + step));
        }
        sum / 255.0
    }
}

/// What to extract, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extraction {
    /// The plate's thickness.
    pub thickness_mm: f32,
    /// How much of the thickness goes into the body, nought to one.
    pub inward: f32,
    /// How far the whole plate is moved out along the normal. Negative moves
    /// it in.
    pub offset_mm: f32,
    /// Whether the plate's thickness follows the mask: full where the mask is
    /// full, none where it is nought, and that share of it between, so a
    /// feathered mask makes a plate that tapers to nothing at its edge.
    /// Otherwise the plate is the whole thickness wherever the mask is half
    /// or more, and stands square at its edge.
    pub feathered: bool,
}

impl Extraction {
    /// Where the plate's two faces stand from the surface, in voxels at
    /// `voxel_size`: `(inner, outer)`, positive outside.
    pub fn faces(&self, voxel_size: f32) -> (f32, f32) {
        let inward = self.inward.clamp(0.0, 1.0);
        let outer = self.offset_mm + (1.0 - inward) * self.thickness_mm;
        let inner = self.offset_mm - inward * self.thickness_mm;
        (inner / voxel_size, outer / voxel_size)
    }
}

/// Why nothing was extracted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtractError {
    /// Nothing on the surface is masked.
    NothingMasked,
    /// Thinner than a voxel; carries the thinnest in millimetres.
    TooThin(f32),
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractError::NothingMasked => write!(f, "nothing is masked"),
            ExtractError::TooThin(mm) => write!(f, "a plate thinner than {mm:.2} mm"),
        }
    }
}

impl Volume {
    /// The masked part of this body's surface thickened into a new body.
    /// This body is not changed.
    pub fn extracted(&self, how: Extraction) -> Result<Volume, ExtractError> {
        let voxel_size = self.voxel_size();
        if how.thickness_mm.is_nan() || how.thickness_mm < voxel_size {
            return Err(ExtractError::TooThin(voxel_size));
        }
        let (inner, outer) = how.faces(voxel_size);
        let mut masked = Vec::new();
        let mut rest = Vec::new();
        // Feathered, the plate reaches wherever the mask is anything at all,
        // and is as thick there as the mask is full.
        let threshold = if how.feathered { FEATHERED } else { MASKED };
        for seed in surface_seeds(self) {
            if self.mask().at(seed.voxel) >= threshold {
                masked.push(seed.point);
            } else {
                rest.push(seed.point);
            }
        }
        // How full the mask is at each masked seed's own point on the
        // surface, 0..=1, read between voxels so that it changes smoothly
        // from one seed to the next.
        let share: Vec<f32> = if how.feathered {
            masked.iter().map(|point| self.mask_between(*point)).collect()
        } else {
            Vec::new()
        };
        let offset = how.offset_mm / voxel_size;
        if masked.is_empty() {
            return Err(ExtractError::NothingMasked);
        }
        let limit = limit_for(inner.abs().max(outer.abs()));
        let near_masked = Reach::spread(masked, limit, |_| true);
        let near_rest = Reach::spread(rest, limit, |_| true);

        let coords: Vec<BrickCoord> = near_masked.bricks().collect();
        let built: Vec<(BrickCoord, Brick, Vec<bool>)> = coords
            .par_iter()
            .filter_map(|&coord| {
                let origin = coord.origin();
                let mut brick = Brick::dense_filled(OUTSIDE);
                let data = brick.make_dense();
                // Which voxels the front face decides, where the plate is the
                // body's own surface, exact, and is left as it is by the
                // smoothing below.
                let mut exact = vec![false; crate::brick::BRICK_VOXELS];
                for z in 0..BRICK_DIM {
                    for y in 0..BRICK_DIM {
                        for x in 0..BRICK_DIM {
                            let voxel = voxel_of(origin, x, y, z);
                            let Some((nearest, to_masked)) = near_masked.nearest(voxel) else {
                                continue;
                            };
                            // The faces, feathered: as far from the offset as
                            // the mask is full at the surface this voxel is
                            // nearest.
                            let (inner, outer, thin) = match share.get(nearest) {
                                Some(full) => (
                                    offset + (inner - offset) * full,
                                    offset + (outer - offset) * full,
                                    // Thinner than a voxel, a plate frays into
                                    // hair on the screen (seen 2026-11-28): it
                                    // ends there instead, where it is a voxel
                                    // thick, and positive past that.
                                    MOST_FEATHER_EDGE - (outer - inner) * full,
                                ),
                                None => (inner, outer, f32::NEG_INFINITY),
                            };
                            let to_rest = near_rest.distance(voxel).unwrap_or(f32::INFINITY);
                            let stored = self.sample_voxel(voxel);
                            // Exact within the band, the reach beyond it.
                            let s = if stored.abs() < NARROW_BAND - 0.5 {
                                stored
                            } else {
                                to_masked.min(to_rest).copysign(stored)
                            };
                            let slab = (s - outer).max(inner - s);
                            // Half the difference of the two distances: nought
                            // where they are equal, the wall square to the
                            // surface, and about the distance to that wall on
                            // either side. Until 2026-11-28 this was `-to_rest`
                            // on one side and `to_masked` on the other, which
                            // jumps from one to the other at the wall rather
                            // than crossing nought there, so the mesher put the
                            // wall wherever the voxels fell: a jagged rim,
                            // 0.066 mm root mean square round a cap where this
                            // is 0.022 (`what_the_faces_of_a_plate_measure`).
                            let to_rest = to_rest.min(limit);
                            let lateral = (to_masked - to_rest) * 0.5;
                            let front = s - outer;
                            exact[brick_index(x, y, z)] = stored.abs() < NARROW_BAND - 0.5
                                && front >= inner - s
                                && front >= lateral;
                            data[brick_index(x, y, z)] =
                                slab.max(lateral).max(thin).clamp(INSIDE, OUTSIDE);
                        }
                    }
                }
                settled(brick).map(|brick| (coord, brick, exact))
            })
            .collect();

        let mut out = Volume::new(voxel_size);
        let mut exact: FxHashMap<BrickCoord, Vec<bool>> = FxHashMap::default();
        for (coord, brick, kept) in built {
            out.insert_brick(coord, brick);
            exact.insert(coord, kept);
        }
        smooth_off_the_front(&mut out, &exact);
        out.mark_everything_dirty();
        Ok(out)
    }
}

/// How many times the faces a plate's reach built are smoothed.
const SMOOTHING_PASSES: usize = 2;

/// Smooth what of a plate the reach built, its back face and its side wall:
/// each voxel near the surface becomes half itself and half the mean of its
/// six neighbours, [`SMOOTHING_PASSES`] times. Not a voxel `exact` marks,
/// where the plate is the body's own surface.
///
/// Asked for by the maintainer on 2026-11-28: "when you do peel / and
/// possibly extract the back face is very rough and voxelized". The reach
/// measures a distance to the nearest of a cloud of surface points, each a
/// fraction of a voxel out, so what it builds is bumpy at the voxel's own
/// scale where the stored field is smooth. Measured with
/// `what_the_faces_of_a_plate_measure`: see the commit and
/// `docs/STUDIO-PLAN.md`.
fn smooth_off_the_front(plate: &mut Volume, exact: &FxHashMap<BrickCoord, Vec<bool>>) {
    for _ in 0..SMOOTHING_PASSES {
        let coords: Vec<BrickCoord> = exact.keys().copied().collect();
        let read = &*plate;
        let changed: Vec<(BrickCoord, Brick)> = coords
            .par_iter()
            .filter_map(|&coord| {
                let mut brick = read.brick(coord)?.clone();
                let kept = &exact[&coord];
                let origin = coord.origin();
                let data = brick.make_dense();
                let mut any = false;
                for z in 0..BRICK_DIM {
                    for y in 0..BRICK_DIM {
                        for x in 0..BRICK_DIM {
                            let index = brick_index(x, y, z);
                            let held = data[index];
                            if kept[index] || held.abs() >= NARROW_BAND {
                                continue;
                            }
                            let voxel = voxel_of(origin, x, y, z);
                            let around = [
                                glam::IVec3::X,
                                -glam::IVec3::X,
                                glam::IVec3::Y,
                                -glam::IVec3::Y,
                                glam::IVec3::Z,
                                -glam::IVec3::Z,
                            ]
                            .iter()
                            .map(|step| read.sample_voxel(voxel + *step))
                            .sum::<f32>()
                                / 6.0;
                            data[index] = (held + around) * 0.5;
                            any = true;
                        }
                    }
                }
                any.then_some((coord, brick))
            })
            .collect();
        for (coord, brick) in changed {
            plate.insert_brick(coord, brick);
        }
    }
}

/// How much bigger, in voxels, a plate is taken out of the body than it is:
/// a quarter of a voxel. Where the plate's front face is the body's own
/// surface (all of it inward), the two fields are nought there together,
/// and `max(body, -plate)` touched nought without crossing it: a sliver of
/// surface the mesher drew as sparkles round the rim of the cut, seen on a
/// screen on 2026-11-28. Grown this much, the cut crosses nought cleanly,
/// and the two parts stand a quarter of a voxel apart.
pub const PEEL_GROWTH: f32 = 0.25;

impl Volume {
    /// This body with `plate` taken out of it: the field
    /// `max(self, PEEL_GROWTH - plate)`, the difference of the two solids,
    /// the plate grown by [`PEEL_GROWTH`]. For an
    /// extract that peels the plate off the body rather than laying it over
    /// it, so the two fit together with no material in both.
    ///
    /// Only the plate's bricks can change, and of those only the ones this
    /// body has material near: where the plate is outside, `-plate` is
    /// inside and takes nothing, and where this body has no brick it is
    /// outside already. The plate must be on this body's lattice, as one
    /// [`Volume::extracted`] made from it is. The mask and the paint are
    /// kept; where the material went they are on nothing.
    #[debug_requires(plate.voxel_size() == self.voxel_size(), "one lattice")]
    pub fn without(&self, plate: &Volume) -> Volume {
        let mut rest = self.duplicated(glam::IVec3::ZERO);
        for coord in plate.brick_coords() {
            let Some(cut) = plate.brick(coord) else {
                continue;
            };
            if matches!(cut, Brick::Uniform(value) if *value >= OUTSIDE) {
                continue;
            }
            match rest.brick(coord) {
                None => continue,
                Some(Brick::Uniform(value)) if *value >= OUTSIDE => continue,
                Some(_) => {}
            }
            let Some(mut brick) = rest.take_brick(coord) else {
                continue;
            };
            let data = brick.make_dense();
            match cut {
                Brick::Uniform(value) => {
                    data.iter_mut().for_each(|held| *held = held.max(PEEL_GROWTH - value));
                }
                Brick::Dense(values) => {
                    for (held, value) in data.iter_mut().zip(values.iter()) {
                        *held = held.max(PEEL_GROWTH - value);
                    }
                }
            }
            // A brick taken out whole is released rather than kept as an
            // array of `OUTSIDE`.
            if !data.iter().all(|value| *value >= OUTSIDE) {
                rest.insert_brick(coord, brick);
            }
        }
        rest.mark_everything_dirty();
        rest
    }
}

/// Mask every voxel of `volume` whose world position `keep` accepts. For
/// tests here and in the application.
pub fn mask_where(volume: &mut Volume, keep: impl Fn(Vec3) -> bool) {
    let coords: Vec<BrickCoord> = volume.brick_coords().collect();
    for coord in coords {
        let lo = coord.origin();
        let hi = coord.max_voxel();
        volume.edit_mask(
            lo,
            hi,
            |_, world, byte| {
                if keep(world) { crate::mask::PROTECTED } else { byte }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capped_ball() -> Volume {
        let mut volume = Volume::new(0.25);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        mask_where(&mut volume, |world| world.y > 6.0);
        volume
    }

    /// [`capped_ball`] with the mask feathered, as a brush leaves it: from
    /// none at 5.25 mm up to full at 6.75, half at 6.
    fn feathered_ball() -> Volume {
        let mut volume = Volume::new(0.25);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        let coords: Vec<BrickCoord> = volume.brick_coords().collect();
        for coord in coords {
            volume.edit_mask(coord.origin(), coord.max_voxel(), |_, world, _| {
                (((world.y - 6.0) / 1.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8
            });
        }
        volume
    }

    fn solid(volume: &Volume, at: Vec3) -> bool {
        volume.sample_world(at) < 0.0
    }

    /// How bumpy a face of a ball-shaped solid is, between `low` and `high`
    /// millimetres from the middle, over a cap round +Y: the face's radius
    /// along a grid of directions, and the root mean square of each one
    /// against the mean of its four neighbours, in millimetres. A smooth
    /// sphere's is nought, near enough.
    pub(super) fn bumpiness(solid: &Volume, low: f32, high: f32) -> f32 {
        let (rows, columns) = (16, 48);
        let radius = |row: usize, column: usize| {
            let polar = (row as f32 + 2.0) / rows as f32 * 0.45;
            let azimuth = column as f32 / columns as f32 * std::f32::consts::TAU;
            let way =
                Vec3::new(polar.sin() * azimuth.cos(), polar.cos(), polar.sin() * azimuth.sin());
            let (mut inside, mut outside) = (low, high);
            let inside_is_solid = solid.sample_world(way * low) < 0.0;
            for _ in 0..30 {
                let middle = (inside + outside) * 0.5;
                if (solid.sample_world(way * middle) < 0.0) == inside_is_solid {
                    inside = middle;
                } else {
                    outside = middle;
                }
            }
            (inside + outside) * 0.5
        };
        let grid: Vec<Vec<f32>> =
            (0..rows).map(|row| (0..columns).map(|column| radius(row, column)).collect()).collect();
        let mut sum = 0.0;
        let mut count = 0;
        for row in 1..rows - 1 {
            for column in 0..columns {
                let left = grid[row][(column + columns - 1) % columns];
                let right = grid[row][(column + 1) % columns];
                let mean = (left + right + grid[row - 1][column] + grid[row + 1][column]) * 0.25;
                sum += (grid[row][column] - mean).powi(2);
                count += 1;
            }
        }
        (sum / count as f32).sqrt()
    }

    /// How jagged the side wall of a plate cut from [`capped_ball`] is: the
    /// polar angle where the plate ends, 9 mm out, round 96 directions, and
    /// the root mean square of each against the mean of its two neighbours,
    /// in millimetres along the surface.
    pub(super) fn wall_jaggedness(solid: &Volume) -> f32 {
        let count = 96;
        let at = |column: usize| {
            let azimuth = column as f32 / count as f32 * std::f32::consts::TAU;
            let point = |polar: f32| {
                Vec3::new(polar.sin() * azimuth.cos(), polar.cos(), polar.sin() * azimuth.sin())
                    * 9.0
            };
            let (mut inside, mut outside) = (0.6_f32, 1.2_f32);
            for _ in 0..30 {
                let middle = (inside + outside) * 0.5;
                if solid.sample_world(point(middle)) < 0.0 {
                    inside = middle;
                } else {
                    outside = middle;
                }
            }
            (inside + outside) * 0.5 * 9.0
        };
        let edge: Vec<f32> = (0..count).map(at).collect();
        let sum: f32 = (0..count)
            .map(|i| {
                let mean = (edge[(i + count - 1) % count] + edge[(i + 1) % count]) * 0.5;
                (edge[i] - mean).powi(2)
            })
            .sum();
        (sum / count as f32).sqrt()
    }

    /// Feathered, the plate is as thick as the mask is full: the whole of
    /// it where the mask is full, half where it is half, tapering to
    /// nothing through the feather and none past it.
    #[test]
    fn a_feathered_plate_is_as_thick_as_the_mask_is_full() {
        let volume = feathered_ball();
        let how = Extraction { thickness_mm: 2.0, inward: 0.0, offset_mm: 0.0, feathered: true };
        let plate = volume.extracted(how).expect("extracted");
        // How far out the plate reaches over the ball's surface at height `y`.
        let reach_at = |y: f32| {
            let way = Vec3::new((100.0 - y * y).max(0.0).sqrt(), y, 0.0).normalize();
            (0..=60)
                .map(|step| 10.0 + step as f32 * 0.05)
                .filter(|r| solid(&plate, way * *r))
                .fold(0.0_f32, |most, r| most.max(r - 10.0))
        };
        let full = reach_at(9.0);
        let half = reach_at(6.0);
        let faint = reach_at(5.45);
        assert!((full - 2.0).abs() < 0.2, "the full mask made {full} mm");
        assert!((half - 1.0).abs() < 0.25, "the half mask made {half} mm");
        assert!(faint < half && faint < 0.5, "the faint mask made {faint} mm");
        assert_eq!(reach_at(4.5), 0.0, "past the feather there is plate");
        // Square, as before, without the switch: the whole thickness at half.
        let square = volume.extracted(Extraction { feathered: false, ..how }).expect("extracted");
        let at_half = {
            let way = Vec3::new(8.0, 6.0, 0.0).normalize();
            (0..=60)
                .map(|step| 10.0 + step as f32 * 0.05)
                .filter(|r| solid(&square, way * *r))
                .fold(0.0_f32, |most, r| most.max(r - 10.0))
        };
        assert!(at_half > 1.7, "square at the mask's half it made {at_half} mm");
    }

    #[test]
    #[ignore]
    fn what_the_faces_of_a_plate_measure() {
        let volume = capped_ball();
        let plate = volume
            .extracted(Extraction {
                thickness_mm: 2.0,
                inward: 1.0,
                offset_mm: 0.0,
                feathered: false,
            })
            .expect("extracted");
        println!(
            "front {:.4} mm, back {:.4} mm",
            bumpiness(&plate, 9.0, 11.0),
            bumpiness(&plate, 7.0, 9.0)
        );
        let rest = volume.without(&plate);
        println!("the body's peeled face {:.4} mm", bumpiness(&rest, 6.5, 9.0));
        println!("the plate's wall {:.4} mm", wall_jaggedness(&plate));
        let feathered = feathered_ball();
        let soft = feathered
            .extracted(Extraction {
                thickness_mm: 2.0,
                inward: 1.0,
                offset_mm: 0.0,
                feathered: false,
            })
            .expect("extracted");
        println!("the plate's wall, from a feathered mask {:.4} mm", wall_jaggedness(&soft));
    }

    #[test]
    fn the_plate_straddles_the_surface_by_the_share_asked() {
        let volume = capped_ball();
        let how = Extraction { thickness_mm: 2.0, inward: 0.5, offset_mm: 0.0, feathered: false };
        let plate = volume.extracted(how).expect("extracted");
        assert!(solid(&plate, Vec3::Y * 9.2), "a millimetre in is plate");
        assert!(solid(&plate, Vec3::Y * 10.8), "a millimetre out is plate");
        assert!(!solid(&plate, Vec3::Y * 8.7), "past the inner face");
        assert!(!solid(&plate, Vec3::Y * 11.3), "past the outer face");
        assert!(!solid(&plate, -Vec3::Y * 10.0), "the unmasked side is not extracted");
        assert!(!solid(&plate, Vec3::X * 10.0), "the equator is unmasked");
        assert!(solid(&volume, Vec3::Y * 9.0), "the body is left as it was");
    }

    #[test]
    fn an_offset_lifts_the_plate_off_the_surface() {
        let volume = capped_ball();
        let how = Extraction { thickness_mm: 1.0, inward: 0.0, offset_mm: 1.0, feathered: false };
        let plate = volume.extracted(how).expect("extracted");
        assert!(!solid(&plate, Vec3::Y * 10.5), "a gap under the plate");
        assert!(solid(&plate, Vec3::Y * 11.5), "the plate");
        assert!(!solid(&plate, Vec3::Y * 12.3), "above it");
    }

    #[test]
    fn the_plate_stops_at_the_masks_edge() {
        let volume = capped_ball();
        let how = Extraction { thickness_mm: 1.0, inward: 0.5, offset_mm: 0.0, feathered: false };
        let plate = volume.extracted(how).expect("extracted");
        // The mask's edge on the surface is at y = 6, where the ball's
        // radius in x is 8.
        let at = |y: f32| Vec3::new((100.0 - y * y).sqrt(), y, 0.0);
        assert!(solid(&plate, at(6.8)), "inside the edge");
        assert!(!solid(&plate, at(5.2)), "outside the edge");
    }

    /// Peeled off, the plate's material is no longer in the body, the body
    /// is the same where the plate is not, and the two meet: no voxel is
    /// solid in both, and the inside of the plate was the body's.
    #[test]
    fn a_plate_peeled_off_leaves_no_material_in_both() {
        let volume = capped_ball();
        let how = Extraction { thickness_mm: 2.0, inward: 0.5, offset_mm: 0.0, feathered: false };
        let plate = volume.extracted(how).expect("extracted");
        let rest = volume.without(&plate);
        assert!(solid(&volume, Vec3::Y * 9.2) && !solid(&rest, Vec3::Y * 9.2), "not peeled");
        assert!(solid(&rest, Vec3::Y * 8.0), "below the plate is still the body");
        assert!(solid(&rest, -Vec3::Y * 9.5), "the far side is untouched");
        for coord in plate.brick_coords() {
            for voxel in voxels_of(coord) {
                let (kept, peeled) = (rest.sample_voxel(voxel), plate.sample_voxel(voxel));
                assert!(!(kept < 0.0 && peeled < 0.0), "solid in both at {voxel}");
                assert_eq!(
                    kept,
                    volume.sample_voxel(voxel).max(PEEL_GROWTH - peeled),
                    "at {voxel}"
                );
            }
        }
    }

    fn voxels_of(coord: BrickCoord) -> impl Iterator<Item = glam::IVec3> {
        let (lo, hi) = (coord.origin(), coord.max_voxel());
        (lo.z..=hi.z).flat_map(move |z| {
            (lo.y..=hi.y).flat_map(move |y| (lo.x..=hi.x).map(move |x| glam::IVec3::new(x, y, z)))
        })
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(12))]
        /// For any two balls on one lattice, the difference is
        /// `max(a, -b)` at every voxel of `b`'s bricks and `a` elsewhere.
        #[test]
        fn without_is_the_max_of_the_body_and_the_grown_plate_negated(
            x in -6.0f32..6.0, y in -6.0f32..6.0, r in 1.0f32..6.0,
        ) {
            let mut body = Volume::new(0.5);
            body.seed_sphere(Vec3::ZERO, 6.0);
            let mut plate = Volume::new(0.5);
            plate.seed_sphere(Vec3::new(x, y, 0.0), r);
            let rest = body.without(&plate);
            for coord in plate.brick_coords() {
                for voxel in voxels_of(coord) {
                    let wanted = body.sample_voxel(voxel).max(PEEL_GROWTH - plate.sample_voxel(voxel));
                    proptest::prop_assert_eq!(rest.sample_voxel(voxel), wanted);
                }
            }
            for coord in body.brick_coords().filter(|c| plate.brick(*c).is_none()) {
                let voxel = coord.origin();
                proptest::prop_assert_eq!(rest.sample_voxel(voxel), body.sample_voxel(voxel));
            }
        }
    }

    #[test]
    fn nothing_masked_or_too_thin_is_refused() {
        let mut volume = Volume::new(0.25);
        volume.seed_sphere(Vec3::ZERO, 5.0);
        let how = Extraction { thickness_mm: 1.0, inward: 0.5, offset_mm: 0.0, feathered: false };
        assert_eq!(volume.extracted(how).err(), Some(ExtractError::NothingMasked));
        let thin = Extraction { thickness_mm: 0.1, ..how };
        assert_eq!(volume.extracted(thin).err(), Some(ExtractError::TooThin(0.25)));
    }
}
