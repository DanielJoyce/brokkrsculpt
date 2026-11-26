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
//! footprint when the nearest surface to it is masked: `lateral` is the
//! distance to the nearest unmasked surface, negated, there, and the
//! distance to the nearest masked surface elsewhere, so it crosses zero at
//! the mask's edge and the plate's side wall stands square to the surface.
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

use crate::brick::{BRICK_DIM, Brick, BrickCoord, INSIDE, NARROW_BAND, OUTSIDE, brick_index};
use crate::reach::{Reach, limit_for, surface_seeds};
use crate::shell::{settled, voxel_of};
use crate::volume::Volume;
use contracts::debug_requires;

/// Half protection: at or over this a voxel is masked.
const MASKED: u8 = 128;

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
        for seed in surface_seeds(self) {
            if self.mask().at(seed.voxel) >= MASKED {
                masked.push(seed.point);
            } else {
                rest.push(seed.point);
            }
        }
        if masked.is_empty() {
            return Err(ExtractError::NothingMasked);
        }
        let limit = limit_for(inner.abs().max(outer.abs()));
        let near_masked = Reach::spread(masked, limit, |_| true);
        let near_rest = Reach::spread(rest, limit, |_| true);

        let coords: Vec<BrickCoord> = near_masked.bricks().collect();
        let built: Vec<(BrickCoord, Brick)> = coords
            .par_iter()
            .filter_map(|&coord| {
                let origin = coord.origin();
                let mut brick = Brick::dense_filled(OUTSIDE);
                let data = brick.make_dense();
                for z in 0..BRICK_DIM {
                    for y in 0..BRICK_DIM {
                        for x in 0..BRICK_DIM {
                            let voxel = voxel_of(origin, x, y, z);
                            let Some(to_masked) = near_masked.distance(voxel) else {
                                continue;
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
                            let lateral = if to_masked <= to_rest { -to_rest } else { to_masked };
                            data[brick_index(x, y, z)] = slab.max(lateral).clamp(INSIDE, OUTSIDE);
                        }
                    }
                }
                settled(brick).map(|brick| (coord, brick))
            })
            .collect();

        let mut out = Volume::new(voxel_size);
        for (coord, brick) in built {
            out.insert_brick(coord, brick);
        }
        out.mark_everything_dirty();
        Ok(out)
    }
}

impl Volume {
    /// This body with `plate` taken out of it: the field
    /// `max(self, -plate)`, the difference of the two solids. For an
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
                Brick::Uniform(value) => data.iter_mut().for_each(|held| *held = held.max(-value)),
                Brick::Dense(values) => {
                    for (held, value) in data.iter_mut().zip(values.iter()) {
                        *held = held.max(-value);
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

    fn solid(volume: &Volume, at: Vec3) -> bool {
        volume.sample_world(at) < 0.0
    }

    #[test]
    fn the_plate_straddles_the_surface_by_the_share_asked() {
        let volume = capped_ball();
        let how = Extraction { thickness_mm: 2.0, inward: 0.5, offset_mm: 0.0 };
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
        let how = Extraction { thickness_mm: 1.0, inward: 0.0, offset_mm: 1.0 };
        let plate = volume.extracted(how).expect("extracted");
        assert!(!solid(&plate, Vec3::Y * 10.5), "a gap under the plate");
        assert!(solid(&plate, Vec3::Y * 11.5), "the plate");
        assert!(!solid(&plate, Vec3::Y * 12.3), "above it");
    }

    #[test]
    fn the_plate_stops_at_the_masks_edge() {
        let volume = capped_ball();
        let how = Extraction { thickness_mm: 1.0, inward: 0.5, offset_mm: 0.0 };
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
        let how = Extraction { thickness_mm: 2.0, inward: 0.5, offset_mm: 0.0 };
        let plate = volume.extracted(how).expect("extracted");
        let rest = volume.without(&plate);
        assert!(solid(&volume, Vec3::Y * 9.2) && !solid(&rest, Vec3::Y * 9.2), "not peeled");
        assert!(solid(&rest, Vec3::Y * 8.0), "below the plate is still the body");
        assert!(solid(&rest, -Vec3::Y * 9.5), "the far side is untouched");
        for coord in plate.brick_coords() {
            for voxel in voxels_of(coord) {
                let (kept, peeled) = (rest.sample_voxel(voxel), plate.sample_voxel(voxel));
                assert!(!(kept < 0.0 && peeled < 0.0), "solid in both at {voxel}");
                assert_eq!(kept, volume.sample_voxel(voxel).max(-peeled), "at {voxel}");
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
        fn without_is_the_max_of_the_body_and_the_negated_plate(
            x in -6.0f32..6.0, y in -6.0f32..6.0, r in 1.0f32..6.0,
        ) {
            let mut body = Volume::new(0.5);
            body.seed_sphere(Vec3::ZERO, 6.0);
            let mut plate = Volume::new(0.5);
            plate.seed_sphere(Vec3::new(x, y, 0.0), r);
            let rest = body.without(&plate);
            for coord in plate.brick_coords() {
                for voxel in voxels_of(coord) {
                    let wanted = body.sample_voxel(voxel).max(-plate.sample_voxel(voxel));
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
        let how = Extraction { thickness_mm: 1.0, inward: 0.5, offset_mm: 0.0 };
        assert_eq!(volume.extracted(how).err(), Some(ExtractError::NothingMasked));
        let thin = Extraction { thickness_mm: 0.1, ..how };
        assert_eq!(volume.extracted(thin).err(), Some(ExtractError::TooThin(0.25)));
    }
}
