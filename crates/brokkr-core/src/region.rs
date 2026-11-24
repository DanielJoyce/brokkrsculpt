// SPDX-License-Identifier: AGPL-3.0-only

//! A read only snapshot of the field over a small box of voxels.
//!
//! Several brushes are not per voxel functions. Smooth needs a neighbour
//! average, pinch needs the difference between a value and that average, and
//! draw needs the local gradient. Reading those straight from the volume while
//! writing to it would mean each voxel sees a mixture of old and new values,
//! and the result would depend on iteration order.
//!
//! So a stamp runs in two phases: copy the affected box out, then write back
//! using only the copy. The box is grown by one voxel on every side so a
//! gradient or neighbour average is available at every voxel of the core box
//! without a bounds check at the rim.

use glam::{IVec3, Vec3};

/// Most samples a region may hold, which is 256 MB of `f32`.
///
/// A guard rail rather than a tuning knob. Every intended caller asks for a box
/// around a brush or a brick, which is thousands of samples. Anything wildly
/// past that is a mistake in working out the bounds, and the resample operation
/// made exactly that mistake when asked to coarsen by a large factor.
pub const MAX_SAMPLES: i64 = 64 << 20;

/// Reusable storage for one stamp's worth of field values.
#[derive(Debug, Default, Clone)]
pub struct FieldRegion {
    /// Inclusive minimum voxel of the stored box, already grown by one.
    lo: IVec3,
    /// Inclusive maximum voxel of the stored box.
    hi: IVec3,
    size: IVec3,
    values: Vec<f32>,
}

impl FieldRegion {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resize to cover an inclusive voxel box, keeping the allocation when it
    /// is already big enough. Called once per stamp, so it must not allocate in
    /// the steady state of a stroke.
    ///
    /// The returned slice holds whatever the last caller left in it. Filling it
    /// with zeroes first would mean writing the whole box twice, which at a
    /// 20 mm brush is sixteen megabytes of memset before [`Volume::snapshot`]
    /// overwrites every byte of it. Snapshot does write every element -- the
    /// bricks it copies from tile the box exactly -- and
    /// `a_snapshot_reproduces_the_field_exactly` pins that by taking a
    /// snapshot into a region that already holds different values.
    ///
    /// [`Volume::snapshot`]: crate::Volume::snapshot
    pub(crate) fn resize(&mut self, lo: IVec3, hi: IVec3) -> &mut [f32] {
        self.lo = lo;
        self.hi = hi;
        self.size = hi - lo + IVec3::ONE;

        // Counted in 64 bits and checked. A caller asking for a box far larger
        // than intended is a bug worth stopping on: computing this in 32 bits
        // silently overflowed and then tried to allocate whatever fell out.
        let count = self.size.as_i64vec3().element_product();
        assert!(
            (0..=MAX_SAMPLES).contains(&count),
            "a field region of {:?} would hold {count} samples, past the {MAX_SAMPLES} limit",
            self.size
        );

        let count = count as usize;
        if self.values.len() < count {
            self.values.resize(count, 0.0);
        }
        &mut self.values[..count]
    }

    /// Inclusive voxel box the copy actually stores, padding included.
    ///
    /// A read outside it is answered by [`FieldRegion::get`]'s clamp rather
    /// than from the copy, so a caller reasoning about what a sample can have
    /// come from needs to know where that edge is rather than assume it.
    /// The values of one line of the region along X, from `from_x` to `to_x`
    /// inclusive, all of which must be inside it. For a pass over a box of
    /// the region that would otherwise ask [`FieldRegion::get`], with its
    /// clamp and its arithmetic, of every voxel.
    #[inline]
    pub(crate) fn row(&self, from_x: i32, to_x: i32, y: i32, z: i32) -> &[f32] {
        let local = IVec3::new(from_x, y, z) - self.lo;
        debug_assert!(
            local.cmpge(IVec3::ZERO).all()
                && y - self.lo.y < self.size.y
                && z - self.lo.z < self.size.z
                && to_x <= self.hi.x
                && from_x <= to_x,
            "a row outside the region"
        );
        let start =
            (local.x + local.y * self.size.x + local.z * self.size.x * self.size.y) as usize;
        &self.values[start..start + (to_x - from_x + 1) as usize]
    }

    /// The region laid bare: its lowest voxel, how many voxels it is each
    /// way, and its values, X fastest. For a reader that works out where a
    /// voxel's value is for itself, several voxels at a time, and has made
    /// sure of its own that each is inside.
    #[inline]
    pub(crate) fn layout(&self) -> (IVec3, IVec3, &[f32]) {
        (self.lo, self.size, &self.values)
    }

    pub(crate) fn bounds(&self) -> (IVec3, IVec3) {
        (self.lo, self.hi)
    }

    /// Value at a world voxel coordinate.
    ///
    /// Coordinates outside the stored box clamp to its edge rather than
    /// panicking. Callers stay inside the core box, so a clamp only ever
    /// happens at the outer rim of the padding.
    #[inline]
    pub fn get(&self, voxel: IVec3) -> f32 {
        let local = (voxel - self.lo).clamp(IVec3::ZERO, self.size - IVec3::ONE);
        let index = local.x + local.y * self.size.x + local.z * self.size.x * self.size.y;
        self.values[index as usize]
    }

    /// Mean of the six axis neighbours, which is the smoothing kernel.
    #[inline]
    pub fn neighbour_average(&self, voxel: IVec3) -> f32 {
        let local = voxel - self.lo;
        // Away from the rim all six neighbours are fixed strides from the
        // centre, which saves working each one out from its world coordinate.
        // See [`FieldRegion::cell`] for why that is worth a special case.
        if local.cmpgt(IVec3::ZERO).all() && (local + IVec3::ONE).cmplt(self.size).all() {
            let row = self.size.x as usize;
            let plane = (self.size.x * self.size.y) as usize;
            let index = local.x as usize + local.y as usize * row + local.z as usize * plane;
            let values = &self.values;
            let sum = values[index + 1]
                + values[index - 1]
                + values[index + row]
                + values[index - row]
                + values[index + plane]
                + values[index - plane];
            return sum / 6.0;
        }

        let sum = self.get(voxel + IVec3::X)
            + self.get(voxel - IVec3::X)
            + self.get(voxel + IVec3::Y)
            + self.get(voxel - IVec3::Y)
            + self.get(voxel + IVec3::Z)
            + self.get(voxel - IVec3::Z);
        sum / 6.0
    }

    /// The eight values of the cell whose minimum corner is `b`, ordered by
    /// the x, y then z bit of the offset.
    ///
    /// Normally the whole cell is inside the stored box, and then the eight are
    /// one index and three fixed strides apart. That case is worth writing out:
    /// every resampling brush lands here once per voxel, and deriving each
    /// corner from its world coordinate instead -- three subtractions, a clamp
    /// and two multiplies each, eight times over -- was the single most
    /// expensive thing a large brush did. Measured at a 20 mm radius it was
    /// three and a half milliseconds of a four millisecond budget.
    #[inline]
    pub(crate) fn cell(&self, b: IVec3) -> [f32; 8] {
        let local = b - self.lo;
        if local.cmpge(IVec3::ZERO).all() && (local + IVec3::ONE).cmplt(self.size).all() {
            let row = self.size.x as usize;
            let plane = (self.size.x * self.size.y) as usize;
            let index = local.x as usize + local.y as usize * row + local.z as usize * plane;
            // The condition above puts the far corner exactly at the last
            // element of the box, so this slice is in range and costs one
            // bounds check rather than eight.
            let cell = &self.values[index..index + plane + row + 2];
            return [
                cell[0],
                cell[1],
                cell[row],
                cell[row + 1],
                cell[plane],
                cell[plane + 1],
                cell[plane + row],
                cell[plane + row + 1],
            ];
        }

        // At the outer rim of the padding a corner really can fall outside, and
        // clamping it to the edge rather than panicking is the point of `get`.
        let corner = |dx: i32, dy: i32, dz: i32| self.get(b + IVec3::new(dx, dy, dz));
        [
            corner(0, 0, 0),
            corner(1, 0, 0),
            corner(0, 1, 0),
            corner(1, 1, 0),
            corner(0, 0, 1),
            corner(1, 0, 1),
            corner(0, 1, 1),
            corner(1, 1, 1),
        ]
    }

    /// Trilinearly interpolated value at a fractional voxel coordinate.
    ///
    /// Used by brushes that warp the field rather than adding to it. The
    /// coordinate must stay within one voxel of the core box, which is what the
    /// padding is for.
    #[inline]
    pub fn sample(&self, voxel: Vec3) -> f32 {
        // `voxel.floor()` and its cast, without the call: see `floor_parts`.
        let (x, base_x) = floor_parts(voxel.x);
        let (y, base_y) = floor_parts(voxel.y);
        let (z, base_z) = floor_parts(voxel.z);
        let frac = voxel - Vec3::new(base_x, base_y, base_z);
        let [c000, c100, c010, c110, c001, c101, c011, c111] = self.cell(IVec3::new(x, y, z));
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;

        let x00 = lerp(c000, c100, frac.x);
        let x10 = lerp(c010, c110, frac.x);
        let x01 = lerp(c001, c101, frac.x);
        let x11 = lerp(c011, c111, frac.x);

        lerp(lerp(x00, x10, frac.y), lerp(x01, x11, frac.y), frac.z)
    }
}

/// `value.floor()`, as the integer it casts to and as the float itself.
///
/// On x86-64 before SSE4.1, which is what a release is built for, `floor` is
/// not an instruction: it is a call into the compiler's own arithmetic
/// library, a loop of shifts and masks. A stroke of the Draw brush made three
/// of those calls for every voxel it wrote, and a profile of one
/// (`examples/stroke_profile.rs` under callgrind) had them at eleven parts in
/// a hundred of everything the stroke did.
///
/// A cast to an integer is an instruction everywhere. It rounds toward nought
/// where `floor` rounds down, so a negative number that was not whole is one
/// too high and is put down one. What is returned is `floor`'s answer to the
/// bit, the sign of a nought included, because the brushes that resample are
/// held to each other bit for bit and a fraction of `-0.0` is not a fraction
/// of `0.0`. Past what an `i32` holds, and for what is not a number, it is
/// `floor` itself.
#[inline(always)]
fn floor_parts(value: f32) -> (i32, f32) {
    // Asked this way round so that a NaN is not in range.
    let in_range = value.abs() < 2_147_483_648.0;
    if !in_range {
        let floored = value.floor();
        return (floored as i32, floored);
    }
    let truncated = value as i32;
    let back = truncated as f32;
    let (whole, floored) =
        if back > value { (truncated - 1, back - 1.0) } else { (truncated, back) };
    // `floor(-0.0)` is `-0.0`, and a cast there and back is `0.0`.
    let floored = if floored == 0.0 { 0.0f32.copysign(value) } else { floored };
    (whole, floored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Every case worth naming, and then any bits at all.
    #[test]
    fn floor_parts_is_floor_at_the_edges() {
        for value in [
            0.0f32,
            -0.0,
            0.5,
            -0.5,
            1.0,
            -1.0,
            2.5,
            -2.5,
            -3.0,
            7.999_999,
            -7.999_999,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            8_388_607.5,
            -8_388_607.5,
            16_777_216.0,
            2_147_483_520.0,
            -2_147_483_648.0,
            2_147_483_648.0,
            4.0e9,
            -4.0e9,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            let (whole, floored) = floor_parts(value);
            assert_eq!(floored.to_bits(), value.floor().to_bits(), "the float, for {value}");
            assert_eq!(whole, value.floor() as i32, "the integer, for {value}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20_000))]

        #[test]
        fn floor_parts_is_floor_for_any_bits(bits in any::<u32>()) {
            let value = f32::from_bits(bits);
            let (whole, floored) = floor_parts(value);
            prop_assert_eq!(floored.to_bits(), value.floor().to_bits());
            prop_assert_eq!(whole, value.floor() as i32);
        }

        /// And in the range a brush works in, where the bits above are thin.
        #[test]
        fn floor_parts_is_floor_near_nought(value in -5000.0f32..5000.0) {
            let (whole, floored) = floor_parts(value);
            prop_assert_eq!(floored.to_bits(), value.floor().to_bits());
            prop_assert_eq!(whole, value.floor() as i32);
        }
    }

    /// Build a region holding a linear ramp along X, whose gradient is known.
    fn ramp() -> FieldRegion {
        let mut region = FieldRegion::new();
        let lo = IVec3::splat(-2);
        let hi = IVec3::splat(2);
        let size = hi - lo + IVec3::ONE;
        let values = region.resize(lo, hi);
        for z in 0..size.z {
            for y in 0..size.y {
                for x in 0..size.x {
                    let index = x + y * size.x + z * size.x * size.y;
                    values[index as usize] = (lo.x + x) as f32;
                }
            }
        }
        region
    }

    #[test]
    fn values_read_back_at_their_world_coordinate() {
        let region = ramp();
        assert_eq!(region.get(IVec3::new(-2, 0, 0)), -2.0);
        assert_eq!(region.get(IVec3::new(0, 1, -1)), 0.0);
        assert_eq!(region.get(IVec3::new(2, 2, 2)), 2.0);
    }

    #[test]
    fn reads_outside_the_box_clamp_to_its_edge() {
        let region = ramp();
        assert_eq!(region.get(IVec3::new(99, 0, 0)), 2.0);
        assert_eq!(region.get(IVec3::new(-99, 0, 0)), -2.0);
    }

    #[test]
    fn a_flat_field_averages_to_itself() {
        let mut region = FieldRegion::new();
        region.resize(IVec3::splat(-1), IVec3::splat(1)).fill(7.0);
        assert_eq!(region.neighbour_average(IVec3::ZERO), 7.0);
        assert_eq!(region.sample(Vec3::new(0.25, -0.5, 0.75)), 7.0);
    }

    #[test]
    fn sampling_between_voxels_interpolates() {
        let region = ramp();
        assert!((region.sample(Vec3::new(0.5, 0.0, 0.0)) - 0.5).abs() < 1.0e-6);
        assert!((region.sample(Vec3::new(-1.25, 1.0, 1.0)) + 1.25).abs() < 1.0e-6);
        // Exactly on a voxel it agrees with the integer lookup.
        assert_eq!(region.sample(Vec3::new(1.0, 0.0, 0.0)), region.get(IVec3::new(1, 0, 0)));
    }

    #[test]
    fn the_neighbour_average_of_a_ramp_is_its_centre_value() {
        // A linear field is its own average, which is what makes smoothing a
        // no op on flat and evenly sloped surfaces.
        let region = ramp();
        assert!((region.neighbour_average(IVec3::ZERO) - 0.0).abs() < 1.0e-6);
        assert!((region.neighbour_average(IVec3::new(1, 0, 0)) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn resizing_keeps_the_allocation() {
        let mut region = FieldRegion::new();
        region.resize(IVec3::splat(-5), IVec3::splat(5));
        let capacity = region.values.capacity();
        region.resize(IVec3::splat(-3), IVec3::splat(3));
        assert_eq!(region.values.capacity(), capacity, "a smaller box should reuse the buffer");
    }
}
