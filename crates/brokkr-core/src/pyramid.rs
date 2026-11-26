// SPDX-License-Identifier: AGPL-3.0-only

//! The form pyramid: the same solid at two, four, eight times the voxel size.
//!
//! The field knows the distance to the surface for three voxels either side of
//! it and nothing further ([`NARROW_BAND`]). That is exactly enough to mesh and
//! to sculpt, and it is the reason the engine cannot answer any question wider
//! than three voxels: is this point down a crevice, which way does the form
//! face here once the scales and pores are ignored, how far below the general
//! run of the surface is this.
//!
//! Those are the questions a wash, a detail preserving volume change and a
//! seam hidden in a fold all ask (`docs/STUDIO-PLAN.md`, sections 8 to 10).
//! This answers them without widening the band: each level is a complete
//! [`Volume`] of the same solid at twice the voxel size of the one below, with
//! its own three voxel band, which at level `k` reaches `3 * 2^k` voxels of
//! the original.
//!
//! # Each level is a distance field, and that is not what the plan first said
//!
//! The plan sketched a pyramid of coverage: one byte a cell holding the
//! fraction of it that is solid, sampled trilinearly. That was worked through
//! before any of it was written and it has a defect on the simplest input
//! there is. For a flat face, the fraction solid is a ramp across exactly one
//! cell, and a trilinear read of that ramp AT the face is not one half: it is
//! `0.5 - 0.5 f + f^2` or `1.5 f - f^2` depending on which half of the cell
//! the face crosses at, `f`, and that swings `0.0625` either side of one half
//! as the face moves across the lattice. Occlusion is twice the excess over
//! one half, so a sloping flat wall would have read as bands of occlusion up
//! to `0.125` deep, in step with the lattice. A crease reads about `0.5`. A
//! wash painted from that would have drawn the voxel grid on every incline.
//!
//! A distance field does not have the problem, for a reason that is one line
//! long: the distance to a plane is LINEAR, and trilinear interpolation
//! reproduces a linear function exactly. So a point on a flat face reads zero
//! at every level whatever the phase, and what is left over is only what is
//! actually there.
//!
//! # How a level is made
//!
//! [`Volume::coarsened`] takes each coarse voxel as the tent weighted mean of
//! the 27 fine voxels round it, weights `1/4, 1/2, 1/4` along each axis. Three
//! properties, each of which was the reason for a choice:
//!
//! * **Centred.** Coarse voxel `j` averages fine voxels `2j - 1`, `2j` and
//!   `2j + 1`, so it sits at the same world position as fine voxel `2j`. Every
//!   level therefore shares the document's lattice origin and
//!   [`Volume::sample_world`] works on any of them with the same point. A
//!   2x2x2 box would have sat half a fine voxel off, at every level, and every
//!   reader would have had to remember by how much.
//! * **Exact on a plane.** The mean of a linear function over a symmetric
//!   stencil is its value at the centre.
//! * **A low pass.** A groove one voxel wide that a point sample would hit or
//!   miss by phase is averaged into its walls instead, so the coarse surface
//!   bridges it. That bridging IS the signal: a point at the bottom of the
//!   groove then lies inside the coarse solid.
//!
//! The mean of clamped values is wrong near the edge of the fine band, where
//! some of the 27 are saturated, and it is right within a voxel or so of the
//! surface, where none are. So the mean is trusted only for where the surface
//! is, and [`Volume::redistance`] then rebuilds the coarse band outward from
//! it. That pass already exists, already refuses to move a sign, and is what
//! turns a field that was valid for 1.5 coarse voxels into one valid for 3.
//!
//! # What is read from it
//!
//! [`FormPyramid::relief`] at a point ON the fine surface is how far that
//! point lies from the coarser one, signed: **negative below it, in a pit or a
//! fold; positive proud of it, on a ridge or a peak; zero on a flat.** In
//! voxels of the level, so it is a depth relative to the scale being asked
//! about and means the same thing at every voxel size.
//!
//! [`FormPyramid::form_normal`] is the coarse field's gradient: the direction
//! the form faces with the detail smaller than that level averaged away, and
//! defined well off the surface, which a volume tool needs because it warps
//! space and not just the surface.
//!
//! # Derived, never saved, never undone
//!
//! A pyramid is a function of one volume at one moment. It is not in the
//! project file and not in an undo entry, and nothing here keeps it in step
//! with the volume it was built from: a caller that edits the volume builds
//! another. Rebuilding only what an edit touched is the obvious next step and
//! is deliberately not in this increment.
//!
//! # What it costs, measured
//!
//! `the_cost_of_a_pyramid_is_printed`, release build, a shared four core
//! container, six levels over a ball at a voxel of one:
//!
//! | ball radius | field | pyramid | of the field | built in |
//! |---|---|---|---|---|
//! | 60 | 77 bricks, 9.6 MB | 5.0 MB | 52% | 0.30 s |
//! | 120 | 299 bricks, 37.4 MB | 13.8 MB | 37% | 0.58 s |
//! | 240 | 1154 bricks, 144.3 MB | 51.4 MB | 36% | 1.71 s |
//!
//! **That is four times the 8% the plan estimated**, and the plan's figure was
//! for one byte a cell where this stores an `f32`. The first level alone is
//! 28% of the field: a quarter as many bricks, each the same size. The small
//! ball's 52% is a floor of a few bricks a level that a real model does not
//! notice.
//!
//! Two ways down from here, neither taken yet because neither is needed to
//! find out whether the signal is any good. A level could be stored as one
//! byte a voxel once built, which is the plan's 8% again at a resolution of a
//! fortieth of a voxel. And a consumer that only wants folds, not pores, can
//! skip the first level's storage after the second has been derived from it.

use glam::{IVec3, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashSet;

use crate::brick::{BRICK_DIM, Brick, BrickCoord, INSIDE, NARROW_BAND, OUTSIDE};
use crate::region::FieldRegion;
use crate::volume::Volume;

/// The most levels a pyramid is ever built with.
///
/// Level `k` has a voxel `2^k` times the original, so six levels reach 64
/// voxels: 16 mm at the 0.25 mm voxel the application ships. Past that a
/// "feature" is the model.
pub const MAX_LEVELS: usize = 6;

/// How far from a fine brick that carries surface the coarse band can reach,
/// in fine voxels.
///
/// The coarse band is [`NARROW_BAND`] coarse voxels, which is twice that in
/// fine ones, and the tent reads one fine voxel further. Every coarse brick
/// within this of a surface brick is built dense, because a coarse brick left
/// out reads as [`OUTSIDE`] and puts a cliff in the coarse field along its
/// faces: the same seam, at a larger scale, that the apron rule exists to
/// prevent.
const COARSE_REACH: i32 = 2 * NARROW_BAND as i32 + 1;

/// Tent weights along one axis, for offsets -1, 0 and +1.
const TENT: [f32; 3] = [0.25, 0.5, 0.25];

/// The voxels of one coarse brick whose tent mean had no saturated tap in it,
/// with the value the mean gave each.
///
/// Those values are exact to the order of the filter, because every number
/// that went into them was a measured distance. A list rather than a second
/// brick: they are the voxels within a voxel or so of the surface, a few
/// percent of a brick.
#[derive(Default)]
struct Trusted {
    values: Vec<(u32, f32)>,
}

impl Trusted {
    fn keep(&mut self, index: usize, value: f32) {
        self.values.push((index as u32, value));
    }

    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl Volume {
    /// The same solid at twice the voxel size.
    ///
    /// See the module documentation for the construction. The result is a
    /// distance field in its own right, valid across its own narrow band,
    /// sharing this volume's lattice origin. It carries no mask and no paint.
    ///
    /// Cost is proportional to the surface. Solid interior stays one value per
    /// brick and empty space stays absent.
    pub fn coarsened(&self) -> Volume {
        let mut coarse = Volume::new(self.voxel_size() * 2.0);
        let dim = BRICK_DIM as i32;

        // Which coarse bricks can hold any of the coarse band, and which are
        // plain interior. Sorted before use: iteration order of a hash map
        // must not be able to reach the result.
        let mut near_surface: FxHashSet<BrickCoord> = FxHashSet::default();
        let mut solid: FxHashSet<BrickCoord> = FxHashSet::default();
        for coord in self.brick_coords() {
            match self.brick(coord) {
                Some(Brick::Uniform(value)) if *value <= INSIDE => {
                    solid
                        .insert(BrickCoord::containing(coord.origin().div_euclid(IVec3::splat(2))));
                }
                Some(Brick::Uniform(value)) if *value >= OUTSIDE => {}
                Some(_) => {
                    let lo =
                        (coord.origin() - IVec3::splat(COARSE_REACH)).div_euclid(IVec3::splat(2));
                    let hi = (coord.max_voxel() + IVec3::splat(COARSE_REACH))
                        .div_euclid(IVec3::splat(2));
                    let (b_lo, b_hi) = (BrickCoord::containing(lo).0, BrickCoord::containing(hi).0);
                    for bz in b_lo.z..=b_hi.z {
                        for by in b_lo.y..=b_hi.y {
                            for bx in b_lo.x..=b_hi.x {
                                near_surface.insert(BrickCoord::new(bx, by, bz));
                            }
                        }
                    }
                }
                None => {}
            }
        }
        let mut near_surface: Vec<BrickCoord> = near_surface.into_iter().collect();
        near_surface.sort();

        let built: Vec<(BrickCoord, Brick, Option<Trusted>)> = near_surface
            .par_iter()
            .map_init(FieldRegion::new, |region, coord| {
                let origin = coord.origin();
                // Fine voxels this coarse brick's tents read: its own span
                // doubled, and one more either side.
                let lo = origin * 2 - IVec3::ONE;
                let hi = (origin + IVec3::splat(dim - 1)) * 2 + IVec3::ONE;
                self.snapshot(lo, hi, region);

                let mut brick = Brick::dense_filled(OUTSIDE);
                let data = brick.make_dense();
                let mut trusted = Trusted::default();
                for z in 0..dim {
                    for y in 0..dim {
                        for x in 0..dim {
                            let centre = (origin + IVec3::new(x, y, z)) * 2;
                            let mut sum = 0.0;
                            let mut every_tap_measured = true;
                            for (dz, wz) in TENT.iter().enumerate() {
                                for (dy, wy) in TENT.iter().enumerate() {
                                    for (dx, wx) in TENT.iter().enumerate() {
                                        let tap = centre
                                            + IVec3::new(
                                                dx as i32 - 1,
                                                dy as i32 - 1,
                                                dz as i32 - 1,
                                            );
                                        let fine = region.get(tap);
                                        every_tap_measured &= fine > INSIDE && fine < OUTSIDE;
                                        sum += wx * wy * wz * fine;
                                    }
                                }
                            }
                            // Distances are stored in voxels, and a coarse
                            // voxel is two fine ones.
                            let index = (x + y * dim + z * dim * dim) as usize;
                            let value = (sum * 0.5).clamp(INSIDE, OUTSIDE);
                            data[index] = value;
                            if every_tap_measured {
                                trusted.keep(index, value);
                            }
                        }
                    }
                }
                (*coord, brick, (!trusted.is_empty()).then_some(trusted))
            })
            .collect();

        let built_here: FxHashSet<BrickCoord> = built.iter().map(|(coord, ..)| *coord).collect();
        let mut exact: Vec<(BrickCoord, Trusted)> = Vec::new();
        for (coord, brick, trusted) in built {
            coarse.insert_brick(coord, brick);
            if let Some(trusted) = trusted {
                exact.push((coord, trusted));
            }
        }
        // Interior that no surface brick is near. Every fine brick under such
        // a coarse brick is solid or the model is malformed: there is no way
        // from inside to outside that does not pass through a brick carrying
        // surface, and a brick carrying surface would have put this one in
        // `near_surface`.
        let mut solid: Vec<BrickCoord> =
            solid.into_iter().filter(|coord| !built_here.contains(coord)).collect();
        solid.sort();
        for coord in solid {
            coarse.insert_brick(coord, Brick::Uniform(INSIDE));
        }

        // The tent mean of a saturated neighbourhood is half the band, which
        // is a plateau and not a distance. This solves the band outward from
        // where the mean can be trusted and flattens what is past it.
        coarse.redistance();

        // And then the voxels the mean got exactly right are put back.
        //
        // `redistance` re-derives every voxel beside the surface as its value
        // over its gradient, which is the right seed for a field whose values
        // are suspect. These are not suspect, but the gradient it divides by
        // is taken across neighbours a voxel further out, where the mean has
        // saturated taps in it and reads short. So the pass moved values that
        // were already exact, by up to 0.046 of a voxel on a face square to an
        // axis -- measured, and it showed as relief on a flat wall that slid
        // with the wall's phase against the lattice. Restoring them takes that
        // to the rounding error of the mean, and leaves the pass doing what it
        // is needed for: everything beyond the reach of the fine band.
        //
        // Sign cannot change here. The pass keeps the sign of what it was
        // given, and what it was given is what is being put back.
        for (coord, trusted) in exact {
            let Some(mut brick) = coarse.brick(coord).cloned() else {
                continue;
            };
            let data = brick.make_dense();
            for (index, value) in trusted.values {
                data[index as usize] = value;
            }
            coarse.insert_brick(coord, brick);
        }

        // Whatever that left wholly outside is empty space, and empty space is
        // an absent brick.
        let empty: Vec<BrickCoord> = coarse
            .brick_coords()
            .filter(
                |coord| matches!(coarse.brick(*coord), Some(Brick::Uniform(v)) if *v >= OUTSIDE),
            )
            .collect();
        for coord in empty {
            coarse.remove_brick_recording(coord);
        }
        // Nothing draws a level, so nothing should find it waiting to be
        // meshed.
        coarse.take_dirty(&mut Vec::new());
        coarse
    }
}

/// One volume's form at successively coarser scales.
///
/// `level` throughout is an index into the pyramid: level 0 has a voxel twice
/// the source's, level 1 four times, and so on.
pub struct FormPyramid {
    levels: Vec<Volume>,
}

impl FormPyramid {
    /// Build up to `levels` levels over `volume`, coarsest last.
    ///
    /// Stops early once a level carries no surface at all, which is what
    /// happens when the voxel has outgrown the model.
    pub fn build(volume: &Volume, levels: usize) -> Self {
        let mut built: Vec<Volume> = Vec::new();
        for _ in 0..levels.min(MAX_LEVELS) {
            let next = match built.last() {
                Some(previous) => previous.coarsened(),
                None => volume.coarsened(),
            };
            if next.stats().dense_bricks == 0 {
                break;
            }
            built.push(next);
        }
        Self { levels: built }
    }

    /// How many levels were built.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The volume at `level`. A distance field like any other.
    pub fn level(&self, level: usize) -> &Volume {
        &self.levels[level]
    }

    /// World size of one voxel at `level`.
    pub fn voxel_size(&self, level: usize) -> f32 {
        self.levels[level].voxel_size()
    }

    /// Signed distance from `point` to the form at `level`, in voxels of that
    /// level, saturating at the band.
    ///
    /// For a point on the source surface: negative in a recess narrower than
    /// the level resolves, positive on a protrusion, zero on anything flat at
    /// that scale.
    pub fn relief(&self, level: usize, point: Vec3) -> f32 {
        self.levels[level].sample_world(point)
    }

    /// [`FormPyramid::relief`] in world units.
    pub fn relief_mm(&self, level: usize, point: Vec3) -> f32 {
        self.relief(level, point) * self.voxel_size(level)
    }

    /// Outward direction of the form at `level`, near `point`.
    ///
    /// The detail finer than the level does not turn it. Meaningful within the
    /// level's band, which is `3 * voxel_size(level)` either side of the form.
    pub fn form_normal(&self, level: usize, point: Vec3) -> Vec3 {
        self.levels[level].gradient_world(point)
    }

    /// Bytes every level holds between them.
    pub fn resident_bytes(&self) -> usize {
        self.levels.iter().map(|level| level.stats().resident_bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{Brush, BrushDirection, BrushKind, BrushScratch, Stamp};
    use crate::clip::ClipPlane;
    use crate::pattern::{Pattern, PatternKind};

    /// A slab: solid on the negative side of a plane through `point`.
    ///
    /// Built with [`Volume::edit_box`] from the plane's own distance, so the
    /// field is an exact distance field at any tilt and any phase against the
    /// lattice, which is the whole point of the fixture.
    fn slab(normal: Vec3, point: Vec3) -> Volume {
        let mut volume = Volume::new(1.0);
        volume.edit_box(Vec3::splat(-40.0), Vec3::splat(104.0), |position, _| {
            ((position - point).dot(normal)).clamp(INSIDE, OUTSIDE)
        });
        volume.take_dirty(&mut Vec::new());
        volume
    }

    fn ball(radius: f32) -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::splat(64.0), radius);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// Points spread over a unit sphere, deterministic and off every axis.
    fn directions(count: usize) -> Vec<Vec3> {
        // A Fibonacci spiral. Even enough, and no two points share a lattice
        // row, so a phase artefact cannot hide by aligning with the samples.
        let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
        (0..count)
            .map(|i| {
                let y = 1.0 - 2.0 * (i as f32 + 0.5) / count as f32;
                let r = (1.0 - y * y).sqrt();
                let theta = golden * i as f32;
                Vec3::new(r * theta.cos(), y, r * theta.sin())
            })
            .collect()
    }

    #[test]
    fn a_flat_face_reads_zero_at_every_level_whatever_its_phase_and_tilt() {
        // The property the coverage pyramid did not have. Planes at awkward
        // tilts, each slid across the lattice in steps that are not a divisor
        // of any level's voxel, sampled at points on the plane itself.
        let tilts = [
            Vec3::X,
            Vec3::new(1.0, 0.37, 0.0).normalize(),
            Vec3::new(0.61, 0.52, 0.83).normalize(),
            Vec3::new(-0.2, 1.0, 0.45).normalize(),
        ];
        let mut worst: f32 = 0.0;
        let mut read = 0;
        for normal in tilts {
            for step in 0..5 {
                let point = Vec3::splat(32.0) + normal * (step as f32 * 0.37);
                let volume = slab(normal, point);
                let pyramid = FormPyramid::build(&volume, 3);
                assert_eq!(pyramid.level_count(), 3, "a slab this size carries three levels");

                // Two directions across the face, well inside the box.
                let across = normal.any_orthonormal_vector();
                let along = normal.cross(across);
                for a in -3..=3 {
                    for b in -3..=3 {
                        let on_face = point + across * (a as f32 * 5.3) + along * (b as f32 * 4.1);
                        for level in 0..pyramid.level_count() {
                            let relief = pyramid.relief(level, on_face);
                            worst = worst.max(relief.abs());
                            read += 1;
                        }
                    }
                }
            }
        }
        assert!(read > 2000, "the sweep read almost nothing");
        // In voxels of the level. A crease reads several tenths; this has to
        // be far enough under that for a flat wall to paint as flat.
        //
        // Measured at 0.020 worst over this sweep, and exactly zero at the
        // first level for a face square to an axis. It is not zero everywhere
        // because a trilinear read on a tilted face reaches voxels more than a
        // voxel from the surface, whose values are `redistance`'s first order
        // solution and not the mean; and because each level is built from the
        // band of the one below, which carries that. The coverage pyramid
        // this replaced would have read 0.125 here by construction.
        assert!(worst < 0.03, "a flat face read {worst} voxels of relief at its worst");
    }

    #[test]
    fn the_coarse_surface_is_where_the_fine_one_is_on_a_ball() {
        // A ball is convex, so its surface lies proud of the smoothed form
        // and the relief is positive: but it is second order in the level's
        // voxel over the radius, and it has to be the same all the way round.
        // The spread is the lattice noise a signal has to rise above.
        let radius = 40.0;
        let volume = ball(radius);
        let pyramid = FormPyramid::build(&volume, 3);
        for level in 0..pyramid.level_count() {
            let reliefs: Vec<f32> = directions(400)
                .into_iter()
                .map(|outward| pyramid.relief(level, Vec3::splat(64.0) + outward * radius))
                .collect();
            let mean = reliefs.iter().sum::<f32>() / reliefs.len() as f32;
            let spread = reliefs.iter().map(|r| (r - mean).abs()).fold(0.0, f32::max);
            assert!(
                (-0.02..0.15).contains(&mean),
                "level {level}: a ball's surface read a mean relief of {mean}"
            );
            assert!(spread < 0.06, "level {level}: relief varies by {spread} round a plain ball");
        }
    }

    #[test]
    fn a_groove_reads_as_below_the_form_once_the_level_is_wider_than_it() {
        // A slot four voxels wide and twelve deep, cut into a slab. At the
        // first level (two voxel cells) it is resolved, so its floor is still
        // surface there and reads zero. Two levels up the cells are eight
        // voxels and the form dips into the slot without following it down;
        // one more and it has all but closed over. Measured at the floor:
        // 0.0, -0.6, -3.6 and -9.6 mm at the four levels, against a slot
        // 12 mm deep.
        let mut volume = slab(Vec3::Y, Vec3::splat(40.0));
        // A plane's normal points at what goes, and what goes is where every
        // plane agrees: between x = 38 and x = 42, above y = 28.
        let slot = [
            ClipPlane { point: Vec3::new(38.0, 0.0, 0.0), normal: Vec3::X },
            ClipPlane { point: Vec3::new(42.0, 0.0, 0.0), normal: -Vec3::X },
            ClipPlane { point: Vec3::new(0.0, 28.0, 0.0), normal: Vec3::Y },
        ];
        volume.clip_convex(&slot);
        let pyramid = FormPyramid::build(&volume, 4);
        assert_eq!(pyramid.level_count(), 4);

        let floor = Vec3::new(40.0, 28.0, 40.0);
        let open_face = Vec3::new(70.0, 40.0, 40.0);
        assert!(volume.sample_world(floor).abs() < 0.2, "the fixture's floor is not at the floor");

        for level in 0..pyramid.level_count() {
            let on_the_face = pyramid.relief(level, open_face);
            assert!(on_the_face.abs() < 0.05, "level {level}: the open face read {on_the_face}");
        }
        assert!(
            pyramid.relief_mm(0, floor).abs() < 0.3,
            "a groove the first level resolves read {} mm at its floor",
            pyramid.relief_mm(0, floor)
        );
        // And it deepens with the level, which is what lets a wash tell a
        // pore from a fold: each level has to read clearly further below the
        // form than the one before.
        for level in 1..pyramid.level_count() {
            assert!(
                pyramid.relief_mm(level, floor) < pyramid.relief_mm(level - 1, floor) - 0.4,
                "the groove floor reads {} mm at level {} and {} at level {level}",
                pyramid.relief_mm(level - 1, floor),
                level - 1,
                pyramid.relief_mm(level, floor)
            );
        }
        assert!(
            pyramid.relief_mm(3, floor) < -8.0,
            "a 12 mm slot four levels up reads only {} mm below the form",
            pyramid.relief_mm(3, floor)
        );
    }

    #[test]
    fn the_form_normal_ignores_detail_finer_than_its_level() {
        // A ball roughened all over with a pattern a few voxels across. The
        // surface normal is thrown about by it; the form normal two levels up
        // should still point out of the ball.
        let radius = 40.0;
        let centre = Vec3::splat(64.0);
        let mut volume = ball(radius);
        let rough = Brush {
            kind: BrushKind::Draw,
            radius: 14.0,
            strength: 0.9,
            pattern: Pattern { kind: PatternKind::Noise, scale_mm: 4.0, depth: 1.0 },
            ..Brush::default()
        };
        let mut scratch = BrushScratch::new();
        for outward in directions(160) {
            let stamp = Stamp::new(centre + outward * radius, outward, BrushDirection::Add);
            for _ in 0..3 {
                rough.apply(&mut volume, &stamp, &mut scratch);
            }
        }
        let pyramid = FormPyramid::build(&volume, 3);

        let mut worst_surface: f32 = 1.0;
        let mut worst_form: f32 = 1.0;
        for outward in directions(300) {
            // Walk out to wherever the roughened surface now is.
            let mut at = centre + outward * radius;
            for _ in 0..8 {
                at -= outward * volume.sample_world(at);
            }
            worst_surface = worst_surface.min(volume.gradient_world(at).dot(outward));
            worst_form = worst_form.min(pyramid.form_normal(2, at).dot(outward));
        }
        // The control: the roughening really did turn the surface normal, or
        // the form normal passing proves nothing about ignoring it.
        assert!(
            worst_surface < 0.9,
            "the fixture is not rough: its worst surface normal is within {worst_surface} of radial"
        );
        assert!(
            worst_form > 0.97,
            "the form normal strays to {worst_form} of radial on a roughened ball"
        );
    }

    #[test]
    fn solid_interior_stays_one_value_per_brick() {
        // A ball large enough to have whole bricks of interior. They must come
        // through as tiles, or a level costs its volume instead of its surface.
        let volume = ball(56.0);
        let coarse = volume.coarsened();
        let stats = coarse.stats();
        assert!(stats.dense_bricks > 0);
        assert!(
            coarse.sample_world(Vec3::splat(64.0)) <= INSIDE,
            "the middle of the ball is not solid"
        );
        assert!(coarse.sample_world(Vec3::splat(-60.0)) >= OUTSIDE, "far outside is not empty");
        // Every stored brick is either surface or solid: nothing is left
        // stored that reads as empty.
        for coord in coarse.brick_coords() {
            assert!(
                !matches!(coarse.brick(coord), Some(Brick::Uniform(v)) if *v >= OUTSIDE),
                "an empty tile was left stored at {coord:?}"
            );
        }
        // And the surface is fewer bricks than it was, since each is twice as
        // wide.
        assert!(
            stats.dense_bricks < volume.stats().dense_bricks,
            "{} coarse bricks against {} fine",
            stats.dense_bricks,
            volume.stats().dense_bricks
        );
    }

    #[test]
    fn a_level_is_a_distance_field_in_its_own_right() {
        // Which is what lets every reader treat a level as a volume. The band
        // was rebuilt by `redistance`, so its own measure of itself applies.
        let volume = ball(40.0);
        let pyramid = FormPyramid::build(&volume, 3);
        for level in 0..pyramid.level_count() {
            let drift = pyramid.level(level).gradient_drift();
            assert!(drift < 0.25, "level {level} has drifted {drift} from a distance field");
        }
    }

    #[test]
    fn the_band_of_a_level_is_whole_right_up_to_its_brick_faces() {
        // A ball whose poles stop four voxels short of a coarse brick's face,
        // so the outer half of the first level's band lies in coarse bricks
        // that no surface carrying fine brick is under. Those have to be built
        // anyway. Left out they read as empty space, which is three voxels
        // where the truth is two and a half, and the level gets a step along
        // the face of every brick the surface comes near: mutating
        // `COARSE_REACH` to zero does exactly that and nothing else noticed.
        let radius = 60.0;
        let centre = Vec3::splat(64.0);
        let volume = ball(radius);
        let level = volume.coarsened();

        let mut outward: Vec<Vec3> = directions(300);
        outward.extend([Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]);
        let mut worst: f32 = 0.0;
        for direction in outward {
            for off in [-2.5_f32, -1.5, 1.5, 2.5] {
                let at = centre + direction * (radius + off * level.voxel_size());
                worst = worst.max((level.sample_world(at) - off).abs());
            }
        }
        // First order upwind differencing is good to a few tenths of a voxel
        // this far out, and a missing brick is wrong by a half or more.
        assert!(worst < 0.3, "the level's band is off by {worst} voxels somewhere round the ball");
    }

    #[test]
    fn building_twice_gives_the_same_levels() {
        let volume = ball(30.0);
        let one = FormPyramid::build(&volume, 2);
        let two = FormPyramid::build(&volume, 2);
        for level in 0..one.level_count() {
            crate::testing::assert_same_field(one.level(level), two.level(level), "two builds");
        }
    }

    #[test]
    fn a_pyramid_stops_when_the_voxel_outgrows_the_model() {
        // A ball four voxels across has nothing left to say a few levels up.
        let volume = ball(2.0);
        let pyramid = FormPyramid::build(&volume, MAX_LEVELS);
        assert!(
            pyramid.level_count() < MAX_LEVELS,
            "a four voxel ball still has surface at level {}",
            pyramid.level_count()
        );
    }

    /// Not a gate. Prints what a pyramid costs so the number in the plan can
    /// be a measured one.
    ///
    /// ```text
    /// cargo test -p brokkr-core --release --lib the_cost_of_a_pyramid -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "a measurement, not a check"]
    fn the_cost_of_a_pyramid_is_printed() {
        for radius in [60.0, 120.0, 240.0] {
            let mut volume = Volume::new(1.0);
            volume.seed_sphere(Vec3::splat(radius + 16.0), radius);
            let fine = volume.stats();
            let started = web_time::Instant::now();
            let pyramid = FormPyramid::build(&volume, MAX_LEVELS);
            let took = started.elapsed();
            println!(
                "ball r={radius}: field {} dense bricks, {:.1} MB; pyramid {} levels, {:.1} MB \
                 ({:.1}% of the field), built in {:.0} ms",
                fine.dense_bricks,
                fine.resident_bytes as f64 / 1048576.0,
                pyramid.level_count(),
                pyramid.resident_bytes() as f64 / 1048576.0,
                100.0 * pyramid.resident_bytes() as f64 / fine.resident_bytes as f64,
                took.as_secs_f64() * 1000.0
            );
            for level in 0..pyramid.level_count() {
                let stats = pyramid.level(level).stats();
                println!(
                    "    level {level}: voxel {}, {} dense, {} tiles",
                    pyramid.voxel_size(level),
                    stats.dense_bricks,
                    stats.uniform_bricks
                );
            }
        }
    }
}
