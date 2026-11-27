// SPDX-License-Identifier: AGPL-3.0-only

//! How far a voxel is from the surface, further than the band reaches.
//!
//! The stored field is a distance only within [`NARROW_BAND`] voxels of the
//! surface and a saturated sign beyond it. That is all a brush needs and not
//! enough for an operation whose answer lies deeper: a shell's inner wall
//! two millimetres in is eight voxels in at a quarter millimetre, and an
//! extracted plate can stand off the surface by as much. This finds the
//! distance there, sparsely, out to a limit the caller names.
//!
//! # The method
//!
//! Seeds first: every band voxel within one voxel of the surface gives the
//! point on the surface nearest it, `x - d * grad d / |grad d|^2`, which is
//! exact for a true distance field and close for a drifted one. Then each
//! seed's ownership spreads outward through the voxel lattice, nearest
//! first, a voxel keeping whichever seed is nearest of those its 26
//! neighbours offer. That is the propagated closest point transform; its
//! error comes from a voxel never being offered the seed that is truly
//! nearest, and on a lattice with 26 neighbours it is a small fraction of a
//! voxel, which `a_ball_reads_its_own_radius_deep_inside` measures.
//!
//! Nearest first is a bucket queue in quarter voxels rather than a heap: the
//! order only has to be good enough that a voxel is rarely improved after
//! it has spread, and when it is, it is queued again and spreads again.
//!
//! # What it costs
//!
//! Storage is a dense brick of seed numbers wherever the reach goes, four
//! bytes a voxel, so a reach of `r` voxels over a surface of `A` voxels is
//! about `4 * A * r` bytes rounded up to whole bricks, and the time is that
//! many voxels times 26 neighbours. It runs on one thread.

use glam::{IVec3, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::brick::{BRICK_DIM, BRICK_VOXELS, Brick, BrickCoord, NARROW_BAND, brick_index};
use crate::volume::Volume;

/// No seed has reached this voxel.
const UNSET: u32 = u32::MAX;

/// Buckets a voxel: a quarter of one.
const BUCKETS_PER_VOXEL: f32 = 4.0;

/// The 26 neighbours of a voxel.
fn neighbours() -> [IVec3; 26] {
    let mut out = [IVec3::ZERO; 26];
    let mut at = 0;
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                if (x, y, z) != (0, 0, 0) {
                    out[at] = IVec3::new(x, y, z);
                    at += 1;
                }
            }
        }
    }
    out
}

/// A point on the surface, in voxel units, and the voxel it was found from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Seed {
    pub point: Vec3,
    pub voxel: IVec3,
}

/// The surface of `volume` as points, one or more a surface voxel.
///
/// Every stored voxel within one voxel of the zero set gives one, so a
/// surface is covered at about the lattice's own spacing.
pub(crate) fn surface_seeds(volume: &Volume) -> Vec<Seed> {
    let coords: Vec<BrickCoord> = volume.brick_coords().collect();
    coords
        .par_iter()
        .flat_map_iter(|coord| {
            let mut found = Vec::new();
            let Some(Brick::Dense(data)) = volume.brick(*coord) else {
                return found.into_iter();
            };
            let origin = coord.origin();
            for z in 0..BRICK_DIM {
                for y in 0..BRICK_DIM {
                    for x in 0..BRICK_DIM {
                        let d = data[brick_index(x, y, z)];
                        if d.abs() >= 1.0 {
                            continue;
                        }
                        let voxel = origin + IVec3::new(x as i32, y as i32, z as i32);
                        let along = |axis: IVec3| {
                            (volume.sample_voxel(voxel + axis) - volume.sample_voxel(voxel - axis))
                                * 0.5
                        };
                        let gradient = Vec3::new(along(IVec3::X), along(IVec3::Y), along(IVec3::Z));
                        let length_squared = gradient.length_squared();
                        // A flat spot, a crease or a sheet thinner than a
                        // voxel: no direction to the surface worth having.
                        if length_squared < 0.0625 {
                            continue;
                        }
                        let point = voxel.as_vec3() - gradient * (d / length_squared);
                        found.push(Seed { point, voxel });
                    }
                }
            }
            found.into_iter()
        })
        .collect()
}

/// The nearest seed to every voxel within a limit of one.
pub(crate) struct Reach {
    seeds: Vec<Vec3>,
    slots: FxHashMap<BrickCoord, usize>,
    bricks: Vec<Box<[u32]>>,
    limit: f32,
}

impl Reach {
    /// Spread `seeds` outward to `limit` voxels, through the voxels `within`
    /// allows. A voxel `within` refuses is never reached and never passes a
    /// seed on.
    pub(crate) fn spread(seeds: Vec<Vec3>, limit: f32, within: impl Fn(IVec3) -> bool) -> Reach {
        let mut reach =
            Reach { seeds, slots: FxHashMap::default(), bricks: Vec::new(), limit: limit.max(0.0) };
        let buckets = (reach.limit * BUCKETS_PER_VOXEL) as usize + 2;
        let mut queue: Vec<Vec<(IVec3, u32)>> = vec![Vec::new(); buckets];
        for index in 0..reach.seeds.len() {
            let base = reach.seeds[index].floor().as_ivec3();
            for corner in 0..8 {
                let voxel = base + IVec3::new(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
                if within(voxel) {
                    reach.offer(voxel, index as u32, 0, &mut queue);
                }
            }
        }
        let around = neighbours();
        for bucket in 0..buckets {
            while let Some((voxel, seed)) = queue[bucket].pop() {
                // Bettered since it was queued: the better one spreads.
                if reach.seed_at(voxel) != seed {
                    continue;
                }
                for step in around {
                    let next = voxel + step;
                    if within(next) {
                        reach.offer(next, seed, bucket, &mut queue);
                    }
                }
            }
        }
        reach
    }

    /// Give `voxel` to `seed` if that seed is nearer than what it has.
    fn offer(&mut self, voxel: IVec3, seed: u32, now: usize, queue: &mut [Vec<(IVec3, u32)>]) {
        let distance = voxel.as_vec3().distance(self.seeds[seed as usize]);
        if distance > self.limit {
            return;
        }
        let current = self.seed_at(voxel);
        if current == seed {
            return;
        }
        if current != UNSET && voxel.as_vec3().distance(self.seeds[current as usize]) <= distance {
            return;
        }
        self.set(voxel, seed);
        // Never behind the bucket being emptied, or it would not be seen.
        let bucket = ((distance * BUCKETS_PER_VOXEL) as usize).max(now).min(queue.len() - 1);
        queue[bucket].push((voxel, seed));
    }

    fn seed_at(&self, voxel: IVec3) -> u32 {
        let coord = BrickCoord::containing(voxel);
        match self.slots.get(&coord) {
            None => UNSET,
            Some(&slot) => {
                let local = voxel - coord.origin();
                self.bricks[slot][brick_index(local.x as usize, local.y as usize, local.z as usize)]
            }
        }
    }

    fn set(&mut self, voxel: IVec3, seed: u32) {
        let coord = BrickCoord::containing(voxel);
        let slot = match self.slots.get(&coord) {
            Some(&slot) => slot,
            None => {
                self.bricks.push(vec![UNSET; BRICK_VOXELS].into_boxed_slice());
                self.slots.insert(coord, self.bricks.len() - 1);
                self.bricks.len() - 1
            }
        };
        let local = voxel - coord.origin();
        self.bricks[slot][brick_index(local.x as usize, local.y as usize, local.z as usize)] = seed;
    }

    /// Distance in voxels from `voxel` to the nearest seed, or `None` when
    /// that is further than the limit (or `within` kept the voxel out).
    pub(crate) fn distance(&self, voxel: IVec3) -> Option<f32> {
        match self.seed_at(voxel) {
            UNSET => None,
            seed => Some(voxel.as_vec3().distance(self.seeds[seed as usize])),
        }
    }

    /// Which seed is nearest `voxel`, by its place in the seeds the reach
    /// was spread from, and how far it is; `None` as [`Reach::distance`].
    pub(crate) fn nearest(&self, voxel: IVec3) -> Option<(usize, f32)> {
        match self.seed_at(voxel) {
            UNSET => None,
            seed => {
                let seed = seed as usize;
                Some((seed, voxel.as_vec3().distance(self.seeds[seed])))
            }
        }
    }

    /// Every brick the reach touched.
    pub(crate) fn bricks(&self) -> impl Iterator<Item = BrickCoord> + '_ {
        self.slots.keys().copied()
    }

    /// Whether anything at all was reached in this brick.
    pub(crate) fn touches(&self, coord: BrickCoord) -> bool {
        self.slots.contains_key(&coord)
    }
}

/// How far past the band a caller has to reach to see `depth` voxels in,
/// with one voxel spare for the mesher's apron.
pub(crate) fn limit_for(depth: f32) -> f32 {
    depth + NARROW_BAND + 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ball_reads_its_own_radius_deep_inside() {
        let voxel_size = 0.25;
        let radius = 10.0;
        let mut volume = Volume::new(voxel_size);
        volume.seed_sphere(Vec3::ZERO, radius);
        let seeds: Vec<Vec3> = surface_seeds(&volume).into_iter().map(|seed| seed.point).collect();
        assert!(!seeds.is_empty());
        let reach = Reach::spread(seeds, 16.0, |voxel| volume.sample_voxel(voxel) < 0.0);
        let mut worst: f32 = 0.0;
        for direction in
            [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ONE.normalize(), Vec3::new(1.0, -2.0, 0.5)]
        {
            let direction = direction.normalize();
            for depth_mm in [1.0, 2.0, 3.0] {
                let voxel = (direction * (radius - depth_mm) / voxel_size).round().as_ivec3();
                let truth = radius / voxel_size - voxel.as_vec3().length();
                let read = reach.distance(voxel).expect("reached");
                worst = worst.max((read - truth).abs());
            }
        }
        assert!(worst < 0.25, "worst error {worst} voxels");
        // The middle is forty voxels in, past the limit of sixteen.
        assert!(reach.distance(IVec3::ZERO).is_none(), "the middle is further than the limit");
    }
}
