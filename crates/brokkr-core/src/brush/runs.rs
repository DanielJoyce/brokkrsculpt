// SPDX-License-Identifier: AGPL-3.0-only

//! The Draw brush, a run of voxels at a time.
//!
//! A brush's arithmetic is a closure asked about one voxel, and with the
//! mesher and the library's `floor` out of the way it is what is left of a
//! stroke: about half of a Draw stroke's instructions, between the weight a
//! voxel is moved by and the trilinear read that moves it
//! (`docs/PERFORMANCE.md`).
//!
//! The weight is the same few operations for every voxel of a line, on
//! numbers that differ only in X: how far from the brush's middle, the
//! falloff of that, how far to read from. So for a run of voxels along X it
//! is worked out four at a time, with the operations every x86-64 has. So
//! is the read's own arithmetic, which cell and how far across it, and the
//! seven blends of its eight values. Only fetching the eight is done a voxel
//! at a time: they are wherever each voxel's own weight sends it.
//!
//! **What a run writes is what the closure writes, to the bit.** Each
//! operation below is the closure's, in the closure's order, done to four
//! numbers where the closure does it to one, and an addition, a
//! multiplication, a division and a square root give the same answer four at
//! a time as one at a time. Nothing is reordered and nothing is fused. The
//! tests that run every brush with its shortcuts on and off compare the two
//! through this, since with them off no run is offered.
//!
//! Only Draw, only with no pattern, and only on x86-64. The other brushes,
//! and Draw anywhere else, are the closure as they were.

use glam::{IVec3, Vec3};

use super::{BrushDirection, FalloffCurve};
use crate::brick::{INSIDE, OUTSIDE};
use crate::region::FieldRegion;
use crate::volume::VoxelEdit;

/// A brush's edit: the closure, and the run for the brushes that have one.
pub(super) struct Edit<'a, F> {
    pub voxel: F,
    pub draw: Option<Draw<'a>>,
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
        match &self.draw {
            Some(draw) => draw.run(first, values, voxel_size, free),
            None => (0, false),
        }
    }
}

/// What one stamp of the Draw brush holds constant: the names are the ones
/// `Brush::stamp` has them under.
pub(super) struct Draw<'a> {
    pub centre: Vec3,
    pub radius_squared: f32,
    pub inverse_radius: f32,
    pub falloff: FalloffCurve,
    pub gain: f32,
    pub displacement: f32,
    pub direction: BrushDirection,
    pub stroke_normal: Vec3,
    pub region: &'a FieldRegion,
}

impl Draw<'_> {
    #[cfg(not(target_arch = "x86_64"))]
    #[inline]
    fn run(&self, _: IVec3, _: &mut [f32], _: f32, _: f32) -> (usize, bool) {
        (0, false)
    }

    /// Four voxels at a time for as many fours as the run holds. The closure
    /// this follows is the `Draw` arm of `Brush::stamp`'s, with what comes
    /// before it for every brush, and each line here names the line there.
    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn run(&self, first: IVec3, values: &mut [f32], voxel_size: f32, free: f32) -> (usize, bool) {
        use std::arch::x86_64::*;

        let fours = values.len() / 4;
        // `if patterned <= 0.0 || free <= 0.0 { return value; }`
        if free <= 0.0 {
            return (fours * 4, false);
        }
        let mut changed = false;
        // `position = voxel.as_vec3() * voxel_size`, `offset = position -
        // centre`, for the two coordinates a line shares.
        let position_y = first.y as f32 * voxel_size;
        let position_z = first.z as f32 * voxel_size;
        let offset_y = position_y - self.centre.y;
        let offset_z = position_z - self.centre.z;
        let (squared_y, squared_z) = (offset_y * offset_y, offset_z * offset_z);
        let outward = self.direction.outward_sign();
        let (copy_low, copy_size, field) = self.region.layout();
        let (copy_low, copy_size) = (copy_low.to_array(), copy_size.to_array());
        let (row, plane) = (copy_size[0] as usize, (copy_size[0] * copy_size[1]) as usize);

        // SAFETY: every intrinsic here is SSE2, which is part of x86-64. The
        // only memory touched is `out`, which is the three arrays of four
        // floats declared below.
        unsafe {
            let size = _mm_set1_ps(voxel_size);
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            for four in 0..fours {
                let x = first.x + (four * 4) as i32;
                let position_x =
                    _mm_mul_ps(_mm_cvtepi32_ps(_mm_setr_epi32(x, x + 1, x + 2, x + 3)), size);
                let offset_x = _mm_sub_ps(position_x, _mm_set1_ps(self.centre.x));
                // `offset.length_squared()`: x squared and y squared, and
                // then z squared.
                let squared = _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(offset_x, offset_x), _mm_set1_ps(squared_y)),
                    _mm_set1_ps(squared_z),
                );
                // `if offset.length_squared() >= radius_squared { return value; }`
                let inside = _mm_cmplt_ps(squared, _mm_set1_ps(self.radius_squared));
                if _mm_movemask_ps(inside) == 0 {
                    continue;
                }
                // `distance = offset.length() * inverse_radius`
                let distance = _mm_mul_ps(_mm_sqrt_ps(squared), _mm_set1_ps(self.inverse_radius));
                // `FalloffCurve::weight`: `t = (1.0 - distance).clamp(0.0, 1.0)`,
                // a clamp being nought below nought and one above one.
                let raw = _mm_sub_ps(one, distance);
                let below = _mm_cmplt_ps(raw, zero);
                let above = _mm_cmpgt_ps(raw, one);
                let t = _mm_andnot_ps(below, raw);
                let t = _mm_or_ps(_mm_and_ps(above, one), _mm_andnot_ps(above, t));
                let weight = match self.falloff {
                    // `t * t * (3.0 - 2.0 * t)`
                    FalloffCurve::Smooth => _mm_mul_ps(
                        _mm_mul_ps(t, t),
                        _mm_sub_ps(_mm_set1_ps(3.0), _mm_mul_ps(_mm_set1_ps(2.0), t)),
                    ),
                    FalloffCurve::Linear => t,
                    // `t * t * t`
                    FalloffCurve::Sharp => _mm_mul_ps(_mm_mul_ps(t, t), t),
                    // `1.0 - inverse * inverse * inverse`
                    FalloffCurve::Wide => {
                        let inverse = _mm_sub_ps(one, t);
                        _mm_sub_ps(one, _mm_mul_ps(_mm_mul_ps(inverse, inverse), inverse))
                    }
                };
                // `shaped = falloff.weight(distance) * gain`, and
                // `if shaped <= 0.0 { return value; }`
                let shaped = _mm_mul_ps(weight, _mm_set1_ps(self.gain));
                let moved = _mm_movemask_ps(_mm_and_ps(inside, _mm_cmpgt_ps(shaped, zero)));
                if moved == 0 {
                    continue;
                }
                // `patterned = shaped * pattern.weight(position)`, which with
                // no pattern is times one, and `weight = patterned * free`.
                let weight = _mm_mul_ps(shaped, _mm_set1_ps(free));
                // `shift = stroke_normal * (weight * displacement * voxel_size
                // * direction.outward_sign())`
                let by = _mm_mul_ps(
                    _mm_mul_ps(_mm_mul_ps(weight, _mm_set1_ps(self.displacement)), size),
                    _mm_set1_ps(outward),
                );
                // `(position - shift) / voxel_size`
                let from = |position: __m128, normal: f32| {
                    _mm_div_ps(_mm_sub_ps(position, _mm_mul_ps(_mm_set1_ps(normal), by)), size)
                };
                let read = [
                    from(position_x, self.stroke_normal.x),
                    from(_mm_set1_ps(position_y), self.stroke_normal.y),
                    from(_mm_set1_ps(position_z), self.stroke_normal.z),
                ];

                // `region.sample(..)`. Its `floor_parts` first, for the
                // three coordinates of all four voxels: a cast toward
                // nought, one less where that came out above, and the sign
                // of a nought kept. Past what an `i32` holds `floor_parts`
                // asks the library, and so does this, a voxel at a time.
                let sign = _mm_set1_ps(-0.0);
                let limit = _mm_set1_ps(2_147_483_648.0);
                let mut in_range = _mm_set1_ps(f32::from_bits(u32::MAX));
                let mut whole = [[0i32; 4]; 3];
                let mut part = [zero; 3];
                for axis in 0..3 {
                    let value = read[axis];
                    in_range =
                        _mm_and_ps(in_range, _mm_cmplt_ps(_mm_andnot_ps(sign, value), limit));
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
                    _mm_storeu_si128(whole[axis].as_mut_ptr().cast(), floor);
                    // `frac = voxel - base`
                    part[axis] = _mm_sub_ps(value, floored);
                }
                // Which of the four have their whole cell inside the copy
                // of the field, away from its rim: `FieldRegion::cell`'s own
                // question, whose answer away from the rim is eight values
                // at fixed strides from the first.
                let mut inside_copy = in_range;
                let mut local = [[0i32; 4]; 3];
                for axis in 0..3 {
                    let floor = _mm_loadu_si128(whole[axis].as_ptr().cast());
                    let from_low = _mm_sub_epi32(floor, _mm_set1_epi32(copy_low[axis]));
                    let within = _mm_and_si128(
                        _mm_cmpgt_epi32(from_low, _mm_set1_epi32(-1)),
                        _mm_cmplt_epi32(from_low, _mm_set1_epi32(copy_size[axis] - 1)),
                    );
                    inside_copy = _mm_and_ps(inside_copy, _mm_castsi128_ps(within));
                    _mm_storeu_si128(local[axis].as_mut_ptr().cast(), from_low);
                }
                let mut new = [0.0f32; 4];
                if _mm_movemask_ps(inside_copy) & moved == moved {
                    // The eight corners of each voxel's cell are four pairs
                    // of neighbours along X. Each pair is read as one, the
                    // four voxels' pairs are dealt out into the near corners
                    // and the far, and then `sample`'s seven blends are made
                    // for the four together, in its order: along x, then y,
                    // then z.
                    let mut pairs = [[zero; 4]; 4];
                    for lane in 0..4 {
                        if moved >> lane & 1 == 0 {
                            continue;
                        }
                        let first = local[0][lane] as usize
                            + local[1][lane] as usize * row
                            + local[2][lane] as usize * plane;
                        for (pair, stride) in pairs.iter_mut().zip([0, row, plane, plane + row]) {
                            let two = &field[first + stride..first + stride + 2];
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
                    let sampled =
                        blend((blend((x00, x10), part[1]), blend((x01, x11), part[1])), part[2]);
                    _mm_storeu_ps(new.as_mut_ptr(), sampled);
                } else {
                    // At the rim of the copy, or past what an `i32` holds:
                    // the read as it is made a voxel at a time, clamps and
                    // all.
                    let mut out = [[0.0f32; 4]; 3];
                    for axis in 0..3 {
                        _mm_storeu_ps(out[axis].as_mut_ptr(), read[axis]);
                    }
                    for lane in 0..4 {
                        if moved >> lane & 1 != 0 {
                            new[lane] = self.region.sample(Vec3::new(
                                out[0][lane],
                                out[1][lane],
                                out[2][lane],
                            ));
                        }
                    }
                }
                // The clamp and the comparison the loop makes of whatever
                // an edit returns.
                for lane in 0..4 {
                    if moved >> lane & 1 == 0 {
                        continue;
                    }
                    let new = new[lane].clamp(INSIDE, OUTSIDE);
                    let value = &mut values[four * 4 + lane];
                    if new != *value {
                        *value = new;
                        changed = true;
                    }
                }
            }
        }
        (fours * 4, changed)
    }
}
