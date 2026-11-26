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
