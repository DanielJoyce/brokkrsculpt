// SPDX-License-Identifier: AGPL-3.0-only

//! Hollowing a body: the surface stays where it is and becomes the outside
//! of a wall, and everything deeper than the wall is taken out.
//!
//! The wall is `max(d, depth - wall)`: solid where the point is inside the
//! body (`d < 0`) and nearer the surface than the wall's thickness. `d` is
//! the stored field, untouched, so the outer surface and everything on it,
//! paint and layers and mask, is exactly as it was. `depth` is how far in a
//! point is, which the band does not hold past three voxels, so it comes
//! from [`crate::reach`].
//!
//! The inner wall follows the outer one at a constant distance, the offset
//! surface, so a groove outside is a ridge inside, rounded, and a part
//! thinner than twice the wall stays solid. That is what a print wants: no
//! wall anywhere thinner than asked.
//!
//! # Not done
//!
//! * **No drain hole.** A resin print of a closed shell traps liquid inside.
//!   Cut one with the ring cut or a brush.
//! * **One thread.** The reach is serial; the write-back is parallel.

use glam::{IVec3, Vec3};
use rayon::prelude::*;

use crate::brick::{BRICK_DIM, Brick, BrickCoord, INSIDE, OUTSIDE, brick_index};
use crate::reach::{Reach, limit_for, surface_seeds};
use crate::volume::Volume;

/// The thinnest wall asked for, in voxels. Thinner than one cannot be meshed
/// as two surfaces.
pub const MIN_WALL_VOXELS: f32 = 1.0;

/// Why a body could not be hollowed. The volume is as it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShellError {
    /// The wall is thinner than a voxel; carries the thinnest in millimetres.
    TooThin(f32),
    /// The body has no surface.
    Empty,
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellError::TooThin(mm) => write!(f, "a wall thinner than {mm:.2} mm"),
            ShellError::Empty => write!(f, "nothing to hollow"),
        }
    }
}

/// A brick's worth of values made into what a volume keeps: `None` when it
/// is all outside and need not be stored.
pub(crate) fn settled(brick: Brick) -> Option<Brick> {
    match brick.is_collapsible() {
        Some(value) if value >= OUTSIDE => None,
        Some(value) => Some(Brick::Uniform(value)),
        None => Some(brick),
    }
}

/// The voxel at a flat index of a brick.
pub(crate) fn voxel_of(origin: IVec3, x: usize, y: usize, z: usize) -> IVec3 {
    origin + IVec3::new(x as i32, y as i32, z as i32)
}

impl Volume {
    /// This body hollowed to a wall `wall_mm` thick, its surface the outside
    /// of the wall.
    pub fn shelled(&self, wall_mm: f32) -> Result<Volume, ShellError> {
        let voxel_size = self.voxel_size();
        let wall = wall_mm / voxel_size;
        if wall.is_nan() || wall < MIN_WALL_VOXELS {
            return Err(ShellError::TooThin(MIN_WALL_VOXELS * voxel_size));
        }
        let seeds: Vec<Vec3> = surface_seeds(self).into_iter().map(|seed| seed.point).collect();
        if seeds.is_empty() {
            return Err(ShellError::Empty);
        }
        let reach = Reach::spread(seeds, limit_for(wall), |voxel| self.sample_voxel(voxel) < 0.0);

        // Only bricks with something inside can change.
        let coords: Vec<BrickCoord> =
            self.brick_coords().filter(|coord| self.brick_fill(*coord) != Some(OUTSIDE)).collect();
        let rebuilt: Vec<(BrickCoord, Option<Brick>)> = coords
            .par_iter()
            .map(|&coord| {
                if !reach.touches(coord) {
                    // Inside and out of reach is the hollow; a brick with
                    // surface in it is always reached.
                    return match self.brick_fill(coord) {
                        Some(value) if value <= INSIDE => (coord, None),
                        _ => (coord, self.brick(coord).cloned()),
                    };
                }
                let origin = coord.origin();
                let mut brick = Brick::dense_filled(OUTSIDE);
                let data = brick.make_dense();
                for z in 0..BRICK_DIM {
                    for y in 0..BRICK_DIM {
                        for x in 0..BRICK_DIM {
                            let voxel = voxel_of(origin, x, y, z);
                            let d = self.sample_voxel(voxel);
                            let value = if d >= 0.0 {
                                d
                            } else {
                                match reach.distance(voxel) {
                                    Some(depth) => d.max(depth - wall),
                                    None => OUTSIDE,
                                }
                            };
                            data[brick_index(x, y, z)] = value.clamp(INSIDE, OUTSIDE);
                        }
                    }
                }
                (coord, settled(brick))
            })
            .collect();

        let mut out = self.duplicated(IVec3::ZERO);
        for (coord, brick) in rebuilt {
            match brick {
                Some(brick) => out.insert_brick(coord, brick),
                None => drop(out.take_brick(coord)),
            }
        }
        out.mark_everything_dirty();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ball(voxel_size: f32, radius: f32) -> Volume {
        let mut volume = Volume::new(voxel_size);
        volume.seed_sphere(Vec3::ZERO, radius);
        volume
    }

    /// Where along a ray from the middle the field changes sign, going out.
    fn crossings(volume: &Volume, direction: Vec3, out_to: f32) -> Vec<f32> {
        let step = volume.voxel_size() * 0.05;
        let mut found = Vec::new();
        let mut was = volume.sample_world(Vec3::ZERO) < 0.0;
        let mut at = step;
        while at < out_to {
            let now = volume.sample_world(direction * at) < 0.0;
            if now != was {
                found.push(at);
                was = now;
            }
            at += step;
        }
        found
    }

    #[test]
    fn a_ball_becomes_a_wall_of_the_thickness_asked() {
        let volume = ball(0.25, 10.0);
        let shell = volume.shelled(2.0).expect("hollowed");
        assert!(shell.sample_world(Vec3::ZERO) > 0.0, "the middle is still solid");
        for direction in [Vec3::X, -Vec3::Y, Vec3::new(1.0, 1.0, -1.0).normalize()] {
            let found = crossings(&shell, direction, 12.0);
            assert_eq!(found.len(), 2, "two walls along {direction}: {found:?}");
            assert!((found[0] - 8.0).abs() < 0.1, "inner wall at {}", found[0]);
            assert!((found[1] - 10.0).abs() < 0.1, "outer wall at {}", found[1]);
        }
    }

    #[test]
    fn the_outer_surface_is_untouched() {
        let volume = ball(0.25, 10.0);
        let shell = volume.shelled(1.5).expect("hollowed");
        for voxel in [IVec3::new(40, 0, 0), IVec3::new(0, -39, 3), IVec3::new(23, 23, 22)] {
            let before = volume.sample_voxel(voxel);
            // Outside, and the first voxel in: the wall's own surface.
            if before > -1.0 {
                assert_eq!(shell.sample_voxel(voxel), before);
            }
        }
    }

    #[test]
    fn a_wall_thinner_than_a_voxel_is_refused() {
        let volume = ball(0.25, 10.0);
        assert_eq!(volume.shelled(0.2).err(), Some(ShellError::TooThin(0.25)));
        assert_eq!(Volume::new(0.25).shelled(1.0).err(), Some(ShellError::Empty));
    }

    #[test]
    fn a_part_thinner_than_two_walls_stays_solid() {
        let volume = ball(0.25, 3.0);
        let shell = volume.shelled(4.0).expect("hollowed");
        assert!(shell.sample_world(Vec3::ZERO) < 0.0, "a 3 mm ball under a 4 mm wall is solid");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(6))]
        #[test]
        fn the_inner_wall_is_the_wall_in(wall in 1.0f32..4.0) {
            let volume = ball(0.25, 9.0);
            let shell = volume.shelled(wall).expect("hollowed");
            let found = crossings(&shell, Vec3::new(0.3, 0.8, -0.5).normalize(), 11.0);
            prop_assert_eq!(found.len(), 2);
            prop_assert!((found[0] - (9.0 - wall)).abs() < 0.1, "inner at {} for {}", found[0], wall);
        }
    }
}
