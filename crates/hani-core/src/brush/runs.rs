// SPDX-License-Identifier: AGPL-3.0-only

//! A brush's arithmetic, a run of voxels at a time.
//!
//! A brush's arithmetic is a closure asked about one voxel, and with the
//! mesher and the library's `floor` out of the way it is what is left of a
//! stroke: from a third of a stamp's instructions to a half, between the
//! weight a voxel is moved by and what moves it (`docs/PERFORMANCE.md`).
//!
//! The weight is the same few operations for every voxel of a line, on
//! numbers that differ only in X: how far from the brush's middle, the
//! falloff of that, how far to read from. So for a run of voxels along X it
//! is worked out four at a time, with the operations every x86-64 has. So
//! is everything an action does with it: where a push, a pinch and a gather
//! read from, the trilinear read's own arithmetic and the seven blends of
//! its eight values, the six neighbours a smooth takes the mean of, the
//! blend toward a plane. Only fetching the eight values of a read is done a
//! voxel at a time: they are wherever each voxel's own weight sends it.
//!
//! One kernel, [`Kernel`], for every brush on the field: upstream's six are
//! each one action of it (`compound.rs` holds those to the classic brushes
//! bit for bit, and has since before this file), and the studio's are
//! several at once.
//!
//! **What a run writes is what the closure writes, to the bit.** Each
//! operation below is the closure's, in the closure's order, done to four
//! numbers where the closure does it to one, and an addition, a
//! multiplication, a division and a square root give the same answer four at
//! a time as one at a time. Nothing is reordered and nothing is fused. The
//! tests that run every brush with its shortcuts on and off compare the two
//! through this, since with them off no run is offered.
//!
//! One thing is not held to the bit, because the language does not hold
//! it: which of two noughts of opposite sign `f32::min` and `f32::max`
//! return. Levelling to one side takes the lesser or the greater of a
//! voxel's value and its blend, and where one is nought and the other minus
//! nought either may come back, from the closure as from here. The two are
//! equal, and a voxel is written only when what it would hold is not equal
//! to what it holds.
//!
//! Only with no pattern and no picture across the tip, only under a mask
//! that is one value across a brick, and only on x86-64. Anything else is
//! the closure as it was.

use glam::{IVec3, Vec3};

use super::BrushDirection;
use super::compound::{Actions, Slope};
// Read only by the x86-64 run; elsewhere there is no run to read them.
#[cfg(target_arch = "x86_64")]
use super::compound::{Shape, Side};
#[cfg(target_arch = "x86_64")]
use crate::brick::{INSIDE, OUTSIDE};
use crate::region::FieldRegion;
use crate::volume::VoxelEdit;

/// A brush's edit: the closure, and the run where the brush can take one.
pub(super) struct Edit<'a, F> {
    pub voxel: F,
    pub run: Option<Kernel<'a>>,
}

impl<F> VoxelEdit for Edit<'_, F>
where
    F: Fn(IVec3, Vec3, f32, f32) -> f32 + Sync,
{
    #[inline(always)]
    fn voxel(&self, voxel: IVec3, position: Vec3, value: f32, free: f32) -> f32 {
        (self.voxel)(voxel, position, value, free)
    }

    #[inline]
    fn run(&self, first: IVec3, values: &mut [f32], voxel_size: f32, free: f32) -> (usize, bool) {
        match &self.run {
            Some(kernel) => kernel.run(first, values, voxel_size, free),
            None => (0, false),
        }
    }
}

/// What one stamp holds constant: the names are the ones
/// `Compound::run` has them under, which are `Brush::stamp`'s.
///
/// Filled on every target and read only by the x86-64 run: elsewhere the run
/// is the empty one above it, and the brush goes voxel by voxel.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub(super) struct Kernel<'a> {
    pub centre: Vec3,
    pub radius_squared: f32,
    pub inverse_radius: f32,
    pub slope: Slope,
    pub gain: f32,
    pub displacement: f32,
    pub direction: BrushDirection,
    pub normal: Vec3,
    /// How much of each action. Its `plane` is not read: `plane_point` is
    /// where the plane has been put.
    pub actions: Actions,
    /// One, or minus one for a pinch that the invert key has turned round.
    pub pinch_turn: f32,
    /// Which way the stroke is going along the surface, for a gather.
    pub along: Option<Vec3>,
    pub plane_point: Vec3,
    pub region: &'a FieldRegion,
}

#[cfg(test)]
thread_local! {
    /// How many voxels this thread has done by runs, for a test to see
    /// that the runs it means to compare were taken at all.
    pub(super) static DONE_BY_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Kernel<'_> {
    #[cfg(not(target_arch = "x86_64"))]
    #[inline]
    fn run(&self, _: IVec3, _: &mut [f32], _: f32, _: f32) -> (usize, bool) {
        (0, false)
    }

    /// Four voxels at a time for as many fours as the run holds. The closure
    /// this follows is `Compound::run`'s, and each line here names the line
    /// there.
    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn run(&self, first: IVec3, values: &mut [f32], voxel_size: f32, free: f32) -> (usize, bool) {
        use std::arch::x86_64::*;

        use super::PINCH_PULL_VOXELS;
        use super::compound::{GATHER, MOST_GATHER};

        let (fours, _) = values.as_chunks_mut::<4>();
        let count = fours.len() * 4;
        #[cfg(test)]
        DONE_BY_RUNS.set(DONE_BY_RUNS.get() + count);
        // `if patterned <= 0.0 || free <= 0.0 { return value; }`
        if free <= 0.0 {
            return (count, false);
        }
        let actions = self.actions;
        let (pushes, pinches, gathers) =
            (actions.push != 0.0, actions.pinch != 0.0, actions.gather != 0.0);
        let (inflates, smooths, levels) =
            (actions.inflate != 0.0, actions.smooth != 0.0, actions.level != 0.0);
        let adds = matches!(self.direction, BrushDirection::Add);
        let outward = self.direction.outward_sign();
        let field_sign = self.direction.field_sign();
        let (centre, normal, plane_point) =
            (self.centre.to_array(), self.normal.to_array(), self.plane_point.to_array());
        // `position = voxel.as_vec3() * voxel_size`, `offset = position -
        // centre`, for the two coordinates a line shares.
        let position_y = first.y as f32 * voxel_size;
        let position_z = first.z as f32 * voxel_size;
        let offset_y = position_y - centre[1];
        let offset_z = position_z - centre[2];
        let (squared_y, squared_z) = (offset_y * offset_y, offset_z * offset_z);
        let copy = Source::of(self.region);
        let mut changed = false;

        // SAFETY: every intrinsic here is SSE2, which is part of x86-64.
        // What is loaded and stored through a pointer is four floats of a
        // slice or an array four long or longer, each named where it is.
        unsafe {
            let size = _mm_set1_ps(voxel_size);
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            let all = _mm_castsi128_ps(_mm_set1_epi32(-1));
            // `mask ? yes : no`
            let choose = |mask: __m128, yes: __m128, no: __m128| {
                _mm_or_ps(_mm_and_ps(mask, yes), _mm_andnot_ps(mask, no))
            };
            // `f32::clamp`: the low end below it, the high end above it and
            // itself otherwise.
            let clamp = |value: __m128, low: f32, high: f32| {
                let (low, high) = (_mm_set1_ps(low), _mm_set1_ps(high));
                let below = _mm_cmplt_ps(value, low);
                let above = _mm_cmpgt_ps(value, high);
                choose(below, low, choose(above, high, value))
            };
            // `Vec3::dot`: x and y, and then z.
            let dot = |a: &[__m128; 3], b: [f32; 3]| {
                _mm_add_ps(
                    _mm_add_ps(
                        _mm_mul_ps(a[0], _mm_set1_ps(b[0])),
                        _mm_mul_ps(a[1], _mm_set1_ps(b[1])),
                    ),
                    _mm_mul_ps(a[2], _mm_set1_ps(b[2])),
                )
            };
            // A vector's own length squared, the same way.
            let squared = |a: &[__m128; 3]| {
                _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(a[0], a[0]), _mm_mul_ps(a[1], a[1])),
                    _mm_mul_ps(a[2], a[2]),
                )
            };

            for (four, values) in fours.iter_mut().enumerate() {
                let x = first.x + (four * 4) as i32;
                let position = [
                    _mm_mul_ps(_mm_cvtepi32_ps(_mm_setr_epi32(x, x + 1, x + 2, x + 3)), size),
                    _mm_set1_ps(position_y),
                    _mm_set1_ps(position_z),
                ];
                let offset = [
                    _mm_sub_ps(position[0], _mm_set1_ps(centre[0])),
                    _mm_set1_ps(offset_y),
                    _mm_set1_ps(offset_z),
                ];
                // `offset.length_squared()`: x squared and y squared, and
                // then z squared.
                let far = _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(offset[0], offset[0]), _mm_set1_ps(squared_y)),
                    _mm_set1_ps(squared_z),
                );
                // `if offset.length_squared() >= radius_squared { return value; }`
                let inside = _mm_cmplt_ps(far, _mm_set1_ps(self.radius_squared));
                if _mm_movemask_ps(inside) == 0 {
                    continue;
                }
                // `distance = offset.length() * inverse_radius`
                let distance = _mm_mul_ps(_mm_sqrt_ps(far), _mm_set1_ps(self.inverse_radius));
                // `Slope::weight`: `along = ((distance - plateau) *
                // steepness).max(0.0)`. Which nought a nought comes out as
                // makes no difference to any of the curves.
                let along = _mm_max_ps(
                    _mm_mul_ps(
                        _mm_sub_ps(distance, _mm_set1_ps(self.slope.plateau)),
                        _mm_set1_ps(self.slope.steepness),
                    ),
                    zero,
                );
                let curved = match self.slope.shape {
                    // `(1.0 - along * along).sqrt()`, of `along.min(1.0)`.
                    Shape::Round => {
                        let along = _mm_min_ps(along, one);
                        _mm_sqrt_ps(_mm_sub_ps(one, _mm_mul_ps(along, along)))
                    }
                    shape => {
                        // `FalloffCurve::weight`: `t = (1.0 -
                        // distance).clamp(0.0, 1.0)`.
                        let t = clamp(_mm_sub_ps(one, along), 0.0, 1.0);
                        match shape {
                            // `t * t * (3.0 - 2.0 * t)`
                            Shape::Smooth => _mm_mul_ps(
                                _mm_mul_ps(t, t),
                                _mm_sub_ps(_mm_set1_ps(3.0), _mm_mul_ps(_mm_set1_ps(2.0), t)),
                            ),
                            Shape::Linear => t,
                            // `t * t * t`
                            Shape::Sharp => _mm_mul_ps(_mm_mul_ps(t, t), t),
                            // `1.0 - inverse * inverse * inverse`
                            Shape::Wide | Shape::Round => {
                                let inverse = _mm_sub_ps(one, t);
                                _mm_sub_ps(one, _mm_mul_ps(_mm_mul_ps(inverse, inverse), inverse))
                            }
                        }
                    }
                };
                // `shaped = slope.weight(distance) * gain`, and
                // `if shaped <= 0.0 { return value; }`
                let shaped = _mm_mul_ps(curved, _mm_set1_ps(self.gain));
                let moving = _mm_and_ps(inside, _mm_cmpgt_ps(shaped, zero));
                let moved = _mm_movemask_ps(moving);
                if moved == 0 {
                    continue;
                }
                // `patterned = shaped * pattern.weight(position)`, which with
                // no pattern is times one, and `weight = patterned * free`.
                let patterned = shaped;
                let weight = _mm_mul_ps(patterned, _mm_set1_ps(free));
                let old = _mm_loadu_ps(values.as_ptr());
                // `held = value`
                let mut held = old;

                // The reads from somewhere else, as one read.
                if pushes || pinches || gathers {
                    // `from = position`
                    let mut from = position;
                    // Which of the four read at all: `read`.
                    let mut reads = if pushes || gathers { all } else { zero };
                    if pushes {
                        // `shift = normal * (weight * displacement *
                        // voxel_size * direction.outward_sign() *
                        // actions.push)`, and `from -= shift`.
                        let by = _mm_mul_ps(
                            _mm_mul_ps(
                                _mm_mul_ps(
                                    _mm_mul_ps(weight, _mm_set1_ps(self.displacement)),
                                    size,
                                ),
                                _mm_set1_ps(outward),
                            ),
                            _mm_set1_ps(actions.push),
                        );
                        for axis in 0..3 {
                            from[axis] =
                                _mm_sub_ps(from[axis], _mm_mul_ps(_mm_set1_ps(normal[axis]), by));
                        }
                    }
                    if pinches {
                        // `to_centre = centre - position`
                        let to_centre = [
                            _mm_sub_ps(_mm_set1_ps(centre[0]), position[0]),
                            _mm_sub_ps(_mm_set1_ps(centre[1]), position[1]),
                            _mm_sub_ps(_mm_set1_ps(centre[2]), position[2]),
                        ];
                        // `lateral = to_centre - normal * to_centre.dot(normal)`
                        let across = dot(&to_centre, normal);
                        let mut lateral = to_centre;
                        for axis in 0..3 {
                            lateral[axis] = _mm_sub_ps(
                                lateral[axis],
                                _mm_mul_ps(_mm_set1_ps(normal[axis]), across),
                            );
                        }
                        // `lateral.try_normalize()`: one over its length,
                        // and none where that is not a number above nought.
                        let recip = _mm_div_ps(one, _mm_sqrt_ps(squared(&lateral)));
                        let finite = _mm_cmplt_ps(
                            _mm_andnot_ps(_mm_set1_ps(-0.0), recip),
                            _mm_set1_ps(f32::INFINITY),
                        );
                        let drawn = _mm_and_ps(finite, _mm_cmpgt_ps(recip, zero));
                        // `pull = to_axis * (weight * PINCH_PULL_VOXELS *
                        // voxel_size * pinch_turn * actions.pinch)`, and
                        // `from += pull`.
                        let by = _mm_mul_ps(
                            _mm_mul_ps(
                                _mm_mul_ps(
                                    _mm_mul_ps(weight, _mm_set1_ps(PINCH_PULL_VOXELS)),
                                    size,
                                ),
                                _mm_set1_ps(self.pinch_turn),
                            ),
                            _mm_set1_ps(actions.pinch),
                        );
                        for axis in 0..3 {
                            let to_axis = _mm_mul_ps(lateral[axis], recip);
                            let pulled = _mm_add_ps(from[axis], _mm_mul_ps(to_axis, by));
                            from[axis] = choose(drawn, pulled, from[axis]);
                        }
                        reads = _mm_or_ps(reads, drawn);
                    }
                    if gathers {
                        // `from_centre = position - centre`, which is
                        // `offset`, and `off = from_centre - normal *
                        // from_centre.dot(normal)`.
                        let out = dot(&offset, normal);
                        let mut off = offset;
                        for axis in 0..3 {
                            off[axis] =
                                _mm_sub_ps(off[axis], _mm_mul_ps(_mm_set1_ps(normal[axis]), out));
                        }
                        // `off -= along * off.dot(along)`
                        if let Some(along) = self.along {
                            let along = along.to_array();
                            let on = dot(&off, along);
                            for axis in 0..3 {
                                off[axis] =
                                    _mm_sub_ps(off[axis], _mm_mul_ps(_mm_set1_ps(along[axis]), on));
                            }
                        }
                        // `share = (GATHER * weight * pinch_turn *
                        // actions.gather).clamp(-MOST_GATHER, MOST_GATHER)`
                        let share = clamp(
                            _mm_mul_ps(
                                _mm_mul_ps(
                                    _mm_mul_ps(_mm_set1_ps(GATHER), weight),
                                    _mm_set1_ps(self.pinch_turn),
                                ),
                                _mm_set1_ps(actions.gather),
                            ),
                            -MOST_GATHER,
                            MOST_GATHER,
                        );
                        // `step = off * share`
                        let step = [
                            _mm_mul_ps(off[0], share),
                            _mm_mul_ps(off[1], share),
                            _mm_mul_ps(off[2], share),
                        ];
                        // `from += step.clamp_length_max(most)`, which is
                        // `most * (step / length)` where the step is longer
                        // than `most` and the step itself where it is not.
                        let most = PINCH_PULL_VOXELS * voxel_size;
                        let long = squared(&step);
                        let longer = _mm_cmpgt_ps(long, _mm_set1_ps(most * most));
                        let length = _mm_sqrt_ps(long);
                        for axis in 0..3 {
                            let shortened =
                                _mm_mul_ps(_mm_set1_ps(most), _mm_div_ps(step[axis], length));
                            from[axis] =
                                _mm_add_ps(from[axis], choose(longer, shortened, step[axis]));
                        }
                    }
                    // `if read { held = region.sample(from / voxel_size); }`
                    let reading = _mm_movemask_ps(reads) & moved;
                    if reading != 0 {
                        let at = [
                            _mm_div_ps(from[0], size),
                            _mm_div_ps(from[1], size),
                            _mm_div_ps(from[2], size),
                        ];
                        let sampled = copy.sample(self.region, at, reading);
                        held = choose(reads, sampled, held);
                    }
                }

                // `held += direction.field_sign() * weight * displacement *
                // actions.inflate`
                if inflates {
                    held = _mm_add_ps(
                        held,
                        _mm_mul_ps(
                            _mm_mul_ps(
                                _mm_mul_ps(_mm_set1_ps(field_sign), weight),
                                _mm_set1_ps(self.displacement),
                            ),
                            _mm_set1_ps(actions.inflate),
                        ),
                    );
                }

                // `average = region.neighbour_average(voxel)`, and
                // `held += (average - held) * (weight * actions.smooth)`.
                if smooths {
                    let average =
                        copy.neighbours(self.region, IVec3::new(x, first.y, first.z), moved);
                    held = _mm_add_ps(
                        held,
                        _mm_mul_ps(
                            _mm_sub_ps(average, held),
                            _mm_mul_ps(weight, _mm_set1_ps(actions.smooth)),
                        ),
                    );
                }

                if levels {
                    // `plane = (position - plane_point).dot(normal) /
                    // voxel_size`
                    let from_plane = [
                        _mm_sub_ps(position[0], _mm_set1_ps(plane_point[0])),
                        _mm_sub_ps(position[1], _mm_set1_ps(plane_point[1])),
                        _mm_sub_ps(position[2], _mm_set1_ps(plane_point[2])),
                    ];
                    let plane = _mm_div_ps(dot(&from_plane, normal), size);
                    // `blended = held + (plane - held) * (patterned *
                    // actions.level)`
                    let blended = _mm_add_ps(
                        held,
                        _mm_mul_ps(
                            _mm_sub_ps(plane, held),
                            _mm_mul_ps(patterned, _mm_set1_ps(actions.level)),
                        ),
                    );
                    let kept = match (actions.side, adds) {
                        (Side::Both, _) => blended,
                        // `blended.min(held)`
                        (Side::Fill, true) | (Side::Cut, false) => _mm_min_ps(blended, held),
                        // `blended.max(held)`
                        (Side::Fill, false) | (Side::Cut, true) => _mm_max_ps(blended, held),
                    };
                    // `held = admit(held, kept.clamp(INSIDE, OUTSIDE), free)`
                    let target = clamp(kept, INSIDE, OUTSIDE);
                    held = if free >= 1.0 {
                        target
                    } else {
                        _mm_add_ps(held, _mm_mul_ps(_mm_sub_ps(target, held), _mm_set1_ps(free)))
                    };
                }

                // The clamp and the comparison the loop makes of whatever
                // an edit returns: `new = edit(..).clamp(INSIDE, OUTSIDE)`,
                // and `if new != old`.
                let new = clamp(held, INSIDE, OUTSIDE);
                let differs = _mm_and_ps(moving, _mm_cmpneq_ps(new, old));
                if _mm_movemask_ps(differs) != 0 {
                    _mm_storeu_ps(values.as_mut_ptr(), choose(differs, new, old));
                    changed = true;
                }
            }
        }
        (count, changed)
    }
}

/// The copy of the field a stamp reads, as its numbers: where it begins,
/// how big it is and how far apart a row and a plane of it are.
#[cfg(target_arch = "x86_64")]
struct Source<'a> {
    low: [i32; 3],
    size: [i32; 3],
    row: usize,
    plane: usize,
    field: &'a [f32],
}

#[cfg(target_arch = "x86_64")]
impl<'a> Source<'a> {
    #[inline(always)]
    fn of(region: &'a FieldRegion) -> Self {
        let (low, size, field) = region.layout();
        let (low, size) = (low.to_array(), size.to_array());
        Self { low, size, row: size[0] as usize, plane: (size[0] * size[1]) as usize, field }
    }

    /// `region.sample(at)` for the four voxels whose bit is set in `lanes`.
    /// What comes back for the others is not to be used.
    ///
    /// # Safety
    ///
    /// None of its own: SSE2 is part of x86-64, and every load is of a slice
    /// that was bounds checked.
    #[inline(always)]
    unsafe fn sample(
        &self,
        region: &FieldRegion,
        at: [std::arch::x86_64::__m128; 3],
        lanes: i32,
    ) -> std::arch::x86_64::__m128 {
        use std::arch::x86_64::*;
        // SAFETY: as the function's own note says.
        unsafe {
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            // `floor_parts` first, for the three coordinates of all four
            // voxels: a cast toward nought, one less where that came out
            // above, and the sign of a nought kept. Past what an `i32`
            // holds `floor_parts` asks the library, and so does this, a
            // voxel at a time.
            let sign = _mm_set1_ps(-0.0);
            let limit = _mm_set1_ps(2_147_483_648.0);
            let mut inside_copy = _mm_castsi128_ps(_mm_set1_epi32(-1));
            let mut local = [[0i32; 4]; 3];
            let mut part = [zero; 3];
            for axis in 0..3 {
                let value = at[axis];
                let in_range = _mm_cmplt_ps(_mm_andnot_ps(sign, value), limit);
                let truncated = _mm_cvttps_epi32(value);
                let back = _mm_cvtepi32_ps(truncated);
                let above = _mm_cmpgt_ps(back, value);
                // A mask is minus one where it is set.
                let floor = _mm_add_epi32(truncated, _mm_castps_si128(above));
                let floored = _mm_sub_ps(back, _mm_and_ps(above, one));
                let nought = _mm_cmpeq_ps(floored, zero);
                let floored = _mm_or_ps(
                    _mm_and_ps(nought, _mm_and_ps(value, sign)),
                    _mm_andnot_ps(nought, floored),
                );
                // `frac = voxel - base`
                part[axis] = _mm_sub_ps(value, floored);
                // Which of the four have their whole cell inside the copy
                // of the field, away from its rim: `FieldRegion::cell`'s
                // own question, whose answer away from the rim is eight
                // values at fixed strides from the first.
                let from_low = _mm_sub_epi32(floor, _mm_set1_epi32(self.low[axis]));
                let within = _mm_and_si128(
                    _mm_cmpgt_epi32(from_low, _mm_set1_epi32(-1)),
                    _mm_cmplt_epi32(from_low, _mm_set1_epi32(self.size[axis] - 1)),
                );
                inside_copy =
                    _mm_and_ps(inside_copy, _mm_and_ps(in_range, _mm_castsi128_ps(within)));
                _mm_storeu_si128(local[axis].as_mut_ptr().cast(), from_low);
            }
            if _mm_movemask_ps(inside_copy) & lanes == lanes {
                // The eight corners of each voxel's cell are four pairs of
                // neighbours along X. Each pair is read as one, the four
                // voxels' pairs are dealt out into the near corners and the
                // far, and then `sample`'s seven blends are made for the
                // four together, in its order: along x, then y, then z.
                let (row, plane) = (self.row, self.plane);
                let mut pairs = [[zero; 4]; 4];
                for lane in 0..4 {
                    if lanes >> lane & 1 == 0 {
                        continue;
                    }
                    let first = local[0][lane] as usize
                        + local[1][lane] as usize * row
                        + local[2][lane] as usize * plane;
                    for (pair, stride) in pairs.iter_mut().zip([0, row, plane, plane + row]) {
                        let two = &self.field[first + stride..first + stride + 2];
                        pair[lane] = _mm_castpd_ps(_mm_load_sd(two.as_ptr().cast()));
                    }
                }
                let corners = |pair: &[__m128; 4]| {
                    let low = _mm_movelh_ps(pair[0], pair[1]);
                    let high = _mm_movelh_ps(pair[2], pair[3]);
                    (
                        _mm_shuffle_ps::<0b10_00_10_00>(low, high),
                        _mm_shuffle_ps::<0b11_01_11_01>(low, high),
                    )
                };
                // `a + (b - a) * t`
                let blend = |(a, b): (__m128, __m128), t: __m128| {
                    _mm_add_ps(a, _mm_mul_ps(_mm_sub_ps(b, a), t))
                };
                let x00 = blend(corners(&pairs[0]), part[0]);
                let x10 = blend(corners(&pairs[1]), part[0]);
                let x01 = blend(corners(&pairs[2]), part[0]);
                let x11 = blend(corners(&pairs[3]), part[0]);
                blend((blend((x00, x10), part[1]), blend((x01, x11), part[1])), part[2])
            } else {
                // At the rim of the copy, or past what an `i32` holds: the
                // read as it is made a voxel at a time, clamps and all.
                let mut out = [[0.0f32; 4]; 3];
                for axis in 0..3 {
                    _mm_storeu_ps(out[axis].as_mut_ptr(), at[axis]);
                }
                let mut new = [0.0f32; 4];
                for lane in 0..4 {
                    if lanes >> lane & 1 != 0 {
                        new[lane] =
                            region.sample(Vec3::new(out[0][lane], out[1][lane], out[2][lane]));
                    }
                }
                _mm_loadu_ps(new.as_ptr())
            }
        }
    }

    /// `region.neighbour_average(voxel)` for four voxels along X from
    /// `first`, those whose bit is set in `lanes`. What comes back for the
    /// others is not to be used.
    ///
    /// # Safety
    ///
    /// None of its own: SSE2 is part of x86-64, and every load is of a slice
    /// that was bounds checked.
    #[inline(always)]
    unsafe fn neighbours(
        &self,
        region: &FieldRegion,
        first: IVec3,
        lanes: i32,
    ) -> std::arch::x86_64::__m128 {
        use std::arch::x86_64::*;
        let local = [first.x - self.low[0], first.y - self.low[1], first.z - self.low[2]];
        // Away from the rim, for all four: `neighbour_average`'s own
        // question, asked of the first of them and the last.
        let clear = local[0] > 0
            && local[0] + 4 < self.size[0]
            && local[1] > 0
            && local[1] + 1 < self.size[1]
            && local[2] > 0
            && local[2] + 1 < self.size[2];
        // SAFETY: as the function's own note says.
        unsafe {
            if clear {
                let (row, plane) = (self.row, self.plane);
                let index = local[0] as usize + local[1] as usize * row + local[2] as usize * plane;
                let four = |from: usize| _mm_loadu_ps(self.field[from..from + 4].as_ptr());
                // In its order: the two along x, the two along y, the two
                // along z, each added to what came before.
                let sum = _mm_add_ps(four(index + 1), four(index - 1));
                let sum = _mm_add_ps(sum, four(index + row));
                let sum = _mm_add_ps(sum, four(index - row));
                let sum = _mm_add_ps(sum, four(index + plane));
                let sum = _mm_add_ps(sum, four(index - plane));
                _mm_div_ps(sum, _mm_set1_ps(6.0))
            } else {
                let mut average = [0.0f32; 4];
                for (lane, average) in average.iter_mut().enumerate() {
                    if lanes >> lane & 1 != 0 {
                        *average = region.neighbour_average(first + IVec3::X * lane as i32);
                    }
                }
                _mm_loadu_ps(average.as_ptr())
            }
        }
    }
}
