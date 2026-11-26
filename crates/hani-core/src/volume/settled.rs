// SPDX-License-Identifier: AGPL-3.0-only

//! The voxels of a brick with detail in it that an edit cannot change.
//!
//! A stamp has always been spared the bricks that hold one value throughout:
//! the deep inside and the far outside, which a brush that resamples or
//! smooths gives back exactly as it found them. The planner narrows such a
//! brick to what is within the brush's reach of a neighbour that differs
//! ([`super::Volume::plan_visits`]).
//!
//! That is a claim about voxels, and it stopped at the brick. A brick the
//! surface passes through holds the surface's band, six voxels of its
//! thirty-two, and on either side of the band the same two values the
//! skipped bricks hold: the field is clamped to `INSIDE` and `OUTSIDE` three
//! voxels from the surface. So in the bricks a stamp does visit, most of the
//! voxels it worked out were ones it could not change. A profile of the Draw
//! brush had the working out at seven parts in ten of a stroke
//! (`examples/stroke_profile.rs` under callgrind, `docs/PERFORMANCE.md`).
//!
//! This carries the claim down to the voxel. A voxel is **settled** for an
//! edit when it and every voxel within the edit's reach of it hold the same
//! end of the band, and the edit leaves that end alone ([`Settled`], which is
//! the brush's own `leaves_constant_alone` asked of each end). A settled
//! voxel is not visited.
//!
//! How it is found, cheaply enough to be worth it: a brick is 1024 lines of
//! 32 voxels, and "holds `INSIDE`" is a bit for each voxel, so a line is a
//! `u32` ([`Lines`]). "And so does everything within reach" is that bit
//! pattern worn away by the reach along each axis in turn, which along a line
//! is shifts and ANDs of one word and across lines is ANDs of words. The 26
//! bricks round it supply the voxels past its faces. For a brick that is a
//! few thousand operations, against the thirty-two thousand voxels it can
//! spare.
//!
//! Nothing here changes what an edit writes. A voxel is skipped only where
//! the edit would have returned the value it was given, and the tests that
//! run every brush with skipping on and off and compare the fields bit for
//! bit hold this as they hold the brick-level skip.

use glam::{IVec3, Vec3};

use crate::brick::{BRICK_DIM, BRICK_VOXELS, INSIDE, OUTSIDE};

const DIM: usize = BRICK_DIM;
const _: () = assert!(DIM == 32, "a line of a brick is held in a u32");

/// The furthest reach the wearing away is done for: with the brick's own 32
/// voxels, this many either side still fit one `u64`.
pub(super) const MOST_REACH: i32 = 16;
/// A brick's lines and the furthest reach either side of them.
const MOST_SPAN: usize = DIM + 2 * MOST_REACH as usize;

/// Which ends of the band an edit leaves alone wherever everything it can
/// read already holds that end.
///
/// For a brush this is `leaves_constant_alone` asked of [`INSIDE`] and of
/// [`OUTSIDE`]. The default claims nothing, and an edit with nothing claimed
/// visits every voxel of every brick it is given, as it always did.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Settled {
    pub inside: bool,
    pub outside: bool,
    /// A ball the edit changes nothing outside of: its centre and its
    /// radius, in world units. Every brush is one, and its edit begins by
    /// asking whether the voxel is in it. Given the ball, the lines of a
    /// brick are narrowed to it before the edit is asked anything, and a
    /// line that misses it is not walked at all. See [`Ball`].
    pub within: Option<(Vec3, f32)>,
}

impl Settled {
    pub const NONE: Settled = Settled { inside: false, outside: false, within: None };

    /// The two ends of the band, by whether each is left alone.
    pub fn ends(inside: bool, outside: bool) -> Settled {
        Settled { inside, outside, within: None }
    }

    /// The same, for an edit that changes nothing outside a ball.
    pub fn inside_ball(self, centre: Vec3, radius: f32) -> Settled {
        Settled { within: Some((centre, radius)), ..self }
    }

    pub(super) fn any(self) -> bool {
        self.inside || self.outside
    }
}

#[cfg(test)]
thread_local! {
    /// Set by [`without`], on the thread that calls the edit.
    static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run something with no voxel treated as settled, whatever its edits claim:
/// what a test compares the same edits against.
#[cfg(test)]
pub(crate) fn without<R>(run: impl FnOnce() -> R) -> R {
    OFF.set(true);
    let result = run();
    OFF.set(false);
    result
}

#[cfg(test)]
pub(super) fn off() -> bool {
    OFF.get()
}

/// The ball an edit works inside, in voxels, for narrowing a line of voxels
/// to the part of it that can be in the ball.
///
/// **Wider than the ball by a voxel and a half, and worked out in `f64`.**
/// The edit's own test is in `f32` and of a position that is itself a
/// rounded product, and which side of the ball's skin a voxel falls is the
/// edit's to say and not this's. So this only ever says "not within a voxel
/// and a half of the ball", which no rounding of either reaches, and what it
/// keeps the edit still tests for itself. A cube round a ball is nearly half
/// outside it, and for the brushes that only read the voxel they write,
/// turning those voxels away was a third of the work.
#[derive(Debug, Clone, Copy)]
pub(super) struct Ball {
    centre: [f64; 3],
    /// The square of the radius and the margin, in voxels.
    reach_squared: f64,
}

impl Ball {
    pub(super) fn new(centre: Vec3, radius: f32, voxel_size: f32) -> Ball {
        let voxel = f64::from(voxel_size);
        let reach = f64::from(radius) / voxel + 1.5;
        Ball {
            centre: [
                f64::from(centre.x) / voxel,
                f64::from(centre.y) / voxel,
                f64::from(centre.z) / voxel,
            ],
            reach_squared: reach * reach,
        }
    }

    /// The voxels of the line at `y` and `z` that may be in the ball, as
    /// the first and the last of them along X, or `None` when none can be.
    #[inline]
    pub(super) fn span(&self, y: i32, z: i32) -> Option<(i32, i32)> {
        let (dy, dz) = (f64::from(y) - self.centre[1], f64::from(z) - self.centre[2]);
        let left = self.reach_squared - dy * dy - dz * dz;
        if left < 0.0 {
            return None;
        }
        let half = left.sqrt();
        // A cast rounds toward nought, which for a negative number is up:
        // one further out each way makes it a bound whatever the sign.
        Some(((self.centre[0] - half) as i32 - 1, (self.centre[0] + half) as i32 + 1))
    }
}

/// One brick as bits: for each of its lines, which voxels hold [`INSIDE`]
/// and which hold [`OUTSIDE`]. Lines are indexed `y + z * 32`.
#[derive(Clone)]
pub(super) struct Lines {
    pub inside: [u32; DIM * DIM],
    pub outside: [u32; DIM * DIM],
}

impl Lines {
    pub(super) const EMPTY: Lines = Lines { inside: [0; DIM * DIM], outside: [0; DIM * DIM] };

    /// Make these the lines of a brick's values.
    pub(super) fn read(&mut self, data: &[f32; BRICK_VOXELS]) {
        let (lines, _) = data.as_chunks::<DIM>();
        for (index, line) in lines.iter().enumerate() {
            let (inside, outside) = ends(line);
            self.inside[index] = inside;
            self.outside[index] = outside;
        }
    }

    #[cfg(test)]
    fn of(data: &[f32; BRICK_VOXELS]) -> Box<Lines> {
        let mut lines = Box::new(Lines::EMPTY);
        lines.read(data);
        lines
    }
}

/// Which voxels of one line hold each end of the band, a bit a voxel.
#[cfg(target_arch = "x86_64")]
#[inline]
fn ends(line: &[f32; DIM]) -> (u32, u32) {
    use std::arch::x86_64::{_mm_cmpeq_ps, _mm_loadu_ps, _mm_movemask_ps, _mm_set1_ps};
    let (mut inside, mut outside) = (0u32, 0u32);
    let (fours, _) = line.as_chunks::<4>();
    for (index, four) in fours.iter().enumerate() {
        // SAFETY: SSE2 is part of x86-64, and `four` is four floats, which is
        // what an unaligned load reads. A line is 32 floats, so there is no
        // remainder.
        unsafe {
            let values = _mm_loadu_ps(four.as_ptr());
            let at = index * 4;
            inside |= (_mm_movemask_ps(_mm_cmpeq_ps(values, _mm_set1_ps(INSIDE))) as u32) << at;
            outside |= (_mm_movemask_ps(_mm_cmpeq_ps(values, _mm_set1_ps(OUTSIDE))) as u32) << at;
        }
    }
    (inside, outside)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn ends(line: &[f32; DIM]) -> (u32, u32) {
    let (mut inside, mut outside) = (0u32, 0u32);
    for (x, value) in line.iter().enumerate() {
        inside |= u32::from(*value == INSIDE) << x;
        outside |= u32::from(*value == OUTSIDE) << x;
    }
    (inside, outside)
}

/// A brick with no lines of bits made for it.
pub(super) const NO_LINES: u32 = u32::MAX;

/// What an edit's planner worked out that the settled voxels of its bricks
/// are found from: every brick of the edit's box and a ring round it, as the
/// one value it holds or as its lines of bits.
pub(super) struct Surroundings<'a> {
    /// `Some(v)` for a brick that holds `v` throughout, X fastest from
    /// `grid_min`. See `EditScratch::fills`.
    pub fills: &'a [Option<f32>],
    /// For each brick, laid out as `fills` is, where in `store` its lines
    /// are, or [`NO_LINES`].
    pub lines: &'a [u32],
    pub store: &'a [Lines],
    pub grid_min: IVec3,
    pub grid_size: IVec3,
    pub reach: i32,
    pub claim: Settled,
}

impl Surroundings<'_> {
    /// The settled voxels of the brick at `brick`, which must be one of the
    /// edit's box and not of the ring.
    pub(super) fn settled(&self, brick: IVec3, out: &mut [u32; DIM * DIM]) {
        let local = brick - self.grid_min;
        debug_assert!(
            local.cmpge(IVec3::ONE).all() && local.cmplt(self.grid_size - IVec3::ONE).all(),
            "a brick of the ring has no ring of its own"
        );
        let around: [Around<'_>; 27] = std::array::from_fn(|index| {
            let offset = IVec3::new(index as i32 % 3, index as i32 / 3 % 3, index as i32 / 9);
            let at = local + offset - IVec3::ONE;
            let at = (at.x + at.y * self.grid_size.x + at.z * self.grid_size.x * self.grid_size.y)
                as usize;
            match (self.lines[at], self.fills[at]) {
                (NO_LINES, Some(value)) => Around::Fill(value),
                // Detail whose lines were not made: nothing is known of it,
                // which a value that is neither end of the band says.
                (NO_LINES, None) => Around::Fill(f32::NAN),
                (index, _) => Around::Detail(&self.store[index as usize]),
            }
        });
        settled_lines(&around, self.reach, self.claim, out);
    }
}

/// What is known of one brick of the 27 round a brick being edited.
#[derive(Clone, Copy)]
pub(super) enum Around<'a> {
    /// One value throughout, or not there, which reads as [`OUTSIDE`].
    Fill(f32),
    Detail(&'a Lines),
}

impl Around<'_> {
    #[inline]
    fn line(&self, inside: bool, index: usize) -> u32 {
        match self {
            Around::Fill(value) => {
                let end = if inside { INSIDE } else { OUTSIDE };
                if *value == end { u32::MAX } else { 0 }
            }
            Around::Detail(lines) => {
                if inside {
                    lines.inside[index]
                } else {
                    lines.outside[index]
                }
            }
        }
    }
}

/// The settled voxels of one brick: a bit for each, a `u32` a line, lines
/// indexed `y + z * 32`.
///
/// `around` is the brick and its 26 neighbours, `around[(dx + 1) + (dy + 1) *
/// 3 + (dz + 1) * 9]`. `reach` is how far the edit reads past the voxel it
/// writes, in voxels, and must be within [`MOST_REACH`].
pub(super) fn settled_lines(
    around: &[Around<'_>; 27],
    reach: i32,
    claim: Settled,
    out: &mut [u32; DIM * DIM],
) {
    debug_assert!((0..=MOST_REACH).contains(&reach));
    out.fill(0);
    if claim.inside {
        worn(around, reach as usize, true, out);
    }
    if claim.outside {
        worn(around, reach as usize, false, out);
    }
}

/// OR into `out` the voxels that hold one end of the band and have nothing
/// else within `reach` of them on any axis.
fn worn(around: &[Around<'_>; 27], reach: usize, inside: bool, out: &mut [u32; DIM * DIM]) {
    // The brick's own lines and `reach` more on every side, which the
    // neighbours supply.
    let span = DIM + 2 * reach;

    // Which neighbour a line past the brick's own belongs to, and which of
    // that neighbour's lines it is.
    let across = |at: usize| -> (usize, usize) {
        if at < reach {
            (0, DIM - reach + at)
        } else if at < reach + DIM {
            (1, at - reach)
        } else {
            (2, at - reach - DIM)
        }
    };

    // Along the line first. Three bricks' lines side by side in one word,
    // voxel `x` at bit `x + reach`, then the AND of every window of
    // `2 * reach + 1` bits, which leaves voxel `x`'s answer at bit `x`.
    let mut along = [0u32; MOST_SPAN * MOST_SPAN];
    for z in 0..span {
        let (brick_z, line_z) = across(z);
        for y in 0..span {
            let (brick_y, line_y) = across(y);
            let index = line_y + line_z * DIM;
            let row = brick_y * 3 + brick_z * 9;
            let left = u64::from(around[row].line(inside, index));
            let own = u64::from(around[row + 1].line(inside, index));
            let right = u64::from(around[row + 2].line(inside, index));
            let wide = if reach == 0 {
                own
            } else {
                (left >> (DIM - reach)) | (own << reach) | (right << (DIM + reach))
            };
            let mut all = wide;
            for shift in 1..=2 * reach {
                all &= wide >> shift;
            }
            along[y + z * span] = all as u32;
        }
    }

    // Then across the lines of a plane, for the brick's own lines only, and
    // then across the planes.
    let mut plane = [0u32; DIM * MOST_SPAN];
    for z in 0..span {
        for y in 0..DIM {
            let mut all = u32::MAX;
            for line in &along[y + z * span..y + z * span + 2 * reach + 1] {
                all &= *line;
            }
            plane[y + z * DIM] = all;
        }
    }
    for z in 0..DIM {
        for y in 0..DIM {
            let mut all = u32::MAX;
            for step in 0..=2 * reach {
                all &= plane[y + (z + step) * DIM];
            }
            out[y + z * DIM] |= all;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::brick_index;
    use crate::testing::Noise;
    use proptest::prelude::*;

    /// A block of three bricks a side: the brick in the middle and the 26
    /// round it, as values, to ask voxel by voxel.
    struct Block {
        values: Vec<f32>,
    }

    const WIDE: usize = 3 * DIM;

    impl Block {
        fn at(&self, voxel: IVec3) -> f32 {
            self.values[voxel.x as usize + voxel.y as usize * WIDE + voxel.z as usize * WIDE * WIDE]
        }

        /// The brick at `(bx, by, bz)` of the block as a brick's values.
        fn brick(&self, brick: IVec3) -> Box<[f32; BRICK_VOXELS]> {
            let mut data = vec![0.0f32; BRICK_VOXELS];
            for z in 0..DIM {
                for y in 0..DIM {
                    for x in 0..DIM {
                        data[brick_index(x, y, z)] =
                            self.at(brick * DIM as i32 + IVec3::new(x as i32, y as i32, z as i32));
                    }
                }
            }
            data.into_boxed_slice().try_into().expect("a brick's worth")
        }

        /// The answer the slow way: is every voxel within `reach` of this
        /// one of the middle brick the end asked for?
        fn settled(&self, voxel: IVec3, reach: i32, end: f32) -> bool {
            let centre = voxel + IVec3::splat(DIM as i32);
            for dz in -reach..=reach {
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        if self.at(centre + IVec3::new(dx, dy, dz)) != end {
                            return false;
                        }
                    }
                }
            }
            true
        }
    }

    /// A surface through the block with noise on it, so that the band, both
    /// ends and stray voxels of each are all there, on every side of the
    /// middle brick's faces.
    fn block(seed: u64, normal: [f32; 3], offset: f32, strays: usize) -> Block {
        let mut random = Noise::seeded(seed);
        let mut values = vec![0.0f32; WIDE * WIDE * WIDE];
        for (index, value) in values.iter_mut().enumerate() {
            let (x, y, z) = (index % WIDE, index / WIDE % WIDE, index / (WIDE * WIDE));
            let distance = (x as f32 - 48.0) * normal[0]
                + (y as f32 - 48.0) * normal[1]
                + (z as f32 - 48.0) * normal[2]
                + offset;
            *value = distance.clamp(INSIDE, OUTSIDE);
        }
        for _ in 0..strays {
            let at = random.below(values.len());
            values[at] = [INSIDE, OUTSIDE, 0.25, -1.5][random.below(4)];
        }
        Block { values }
    }

    fn lines_of(block: &Block, some_as_fills: bool) -> Vec<Option<Box<Lines>>> {
        (0..27)
            .map(|index| {
                let brick = IVec3::new(index % 3, index / 3 % 3, index / 9);
                let data = block.brick(brick);
                // A brick of one value is given as that value, as the
                // planner gives it.
                if some_as_fills && data.iter().all(|value| *value == data[0]) {
                    None
                } else {
                    Some(Lines::of(&data))
                }
            })
            .collect()
    }

    fn check(block: &Block, reach: i32, claim: Settled, some_as_fills: bool) -> usize {
        let lines = lines_of(block, some_as_fills);
        let around: [Around<'_>; 27] = std::array::from_fn(|index| match &lines[index] {
            Some(lines) => Around::Detail(lines),
            None => {
                let brick = IVec3::new(index as i32 % 3, index as i32 / 3 % 3, index as i32 / 9);
                Around::Fill(block.at(brick * DIM as i32))
            }
        });
        let mut out = [0u32; DIM * DIM];
        settled_lines(&around, reach, claim, &mut out);
        let mut settled = 0;
        for z in 0..DIM {
            for y in 0..DIM {
                for x in 0..DIM {
                    let voxel = IVec3::new(x as i32, y as i32, z as i32);
                    let expected = (claim.inside && block.settled(voxel, reach, INSIDE))
                        || (claim.outside && block.settled(voxel, reach, OUTSIDE));
                    let found = out[y + z * DIM] >> x & 1 == 1;
                    assert_eq!(found, expected, "voxel {voxel} at reach {reach}, {claim:?}");
                    settled += usize::from(found);
                }
            }
        }
        settled
    }

    #[test]
    fn a_line_of_bits_is_equality_voxel_by_voxel() {
        let mut random = Noise::seeded(3);
        for _ in 0..200 {
            let line: [f32; DIM] = std::array::from_fn(|_| {
                [INSIDE, OUTSIDE, 0.0, -0.0, 2.999_999_8, f32::NAN][random.below(6)]
            });
            let (inside, outside) = ends(&line);
            for (x, value) in line.iter().enumerate() {
                assert_eq!(inside >> x & 1 == 1, *value == INSIDE);
                assert_eq!(outside >> x & 1 == 1, *value == OUTSIDE);
            }
        }
    }

    /// A flat surface through the middle brick: either side of the band is
    /// settled, the band and what is within reach of it is not, and each
    /// claim is honoured on its own.
    #[test]
    fn either_side_of_the_band_is_settled_and_the_band_is_not() {
        let through = block(1, [0.0, 0.0, 1.0], 0.0, 0);
        let both = Settled::ends(true, true);
        // Z from 32 to 63 is the middle brick and the surface is at 48. The
        // field is an end of the band from three voxels off, so the planes
        // that are neither are 46 to 50, and with a reach of 2 the unsettled
        // ones are 44 to 52: nine of the thirty-two. Inside is 32 to 43,
        // twelve planes, and outside 53 to 63, eleven.
        assert_eq!(check(&through, 2, both, true), 23 * 32 * 32);
        assert_eq!(check(&through, 2, Settled::ends(true, false), true), 12 * 1024);
        assert_eq!(check(&through, 2, Settled::ends(false, true), true), 11 * 1024);
        assert_eq!(check(&through, 2, Settled::NONE, true), 0);
        assert_eq!(check(&through, 0, both, true), (32 - 5) * 32 * 32);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// Whatever the surface, the strays and the reach, a voxel is marked
        /// settled exactly when the slow way says it is, whether the bricks
        /// of one value come as bits or as that value.
        #[test]
        fn the_bits_are_the_slow_way_voxel_by_voxel(
            seed in any::<u64>(),
            nx in -1.0f32..1.0, ny in -1.0f32..1.0, nz in -1.0f32..1.0,
            offset in -40.0f32..40.0,
            strays in 0usize..60,
            reach in 0i32..=6,
            inside in any::<bool>(), outside in any::<bool>(),
            as_fills in any::<bool>(),
        ) {
            let block = block(seed, [nx, ny, nz], offset, strays);
            check(&block, reach, Settled::ends(inside, outside), as_fills);
        }
    }

    /// The span of a line never leaves out a voxel the edit's own test would
    /// let in: for balls of any size anywhere, at voxel sizes from the
    /// finest to the coarsest there are, every voxel that passes the test a
    /// brush makes, in the arithmetic a brush makes it in, is inside its
    /// line's span.
    #[test]
    fn a_span_holds_every_voxel_the_edit_would_let_in() {
        let mut random = Noise::seeded(5);
        let mut kept = 0usize;
        let mut walked = 0usize;
        for _ in 0..400 {
            let voxel_size = [0.03f32, 0.0625, 0.25, 0.3, 1.0, 2.0][random.below(6)];
            let radius = (1 + random.below(40)) as f32 * voxel_size * 0.77;
            let pick = |random: &mut Noise| (random.below(4001) as f32 - 2000.0) * 0.173;
            let centre = Vec3::new(pick(&mut random), pick(&mut random), pick(&mut random));
            let ball = Ball::new(centre, radius, voxel_size);
            let radius_squared = radius * radius;
            let middle = (centre / voxel_size).round().as_ivec3();
            let around = (radius / voxel_size).ceil() as i32 + 3;
            for z in middle.z - around..=middle.z + around {
                for y in middle.y - around..=middle.y + around {
                    let span = ball.span(y, z);
                    for x in middle.x - around..=middle.x + around {
                        // As `write_voxels` makes the position and a brush
                        // tests it.
                        let position = IVec3::new(x, y, z).as_vec3() * voxel_size;
                        let inside = (position - centre).length_squared() < radius_squared;
                        let in_span = span.is_some_and(|(from, to)| (from..=to).contains(&x));
                        assert!(
                            !inside || in_span,
                            "voxel {x} {y} {z} is in the ball and out of its line's span \
                             (voxel {voxel_size}, radius {radius}, centre {centre})"
                        );
                        kept += usize::from(in_span);
                        walked += 1;
                    }
                }
            }
        }
        assert!(kept * 10 < walked * 9, "the spans kept {kept} of {walked} voxels: nothing spared");
    }

    /// The furthest reach there is, which is where the three bricks' lines
    /// fill the word they are put side by side in.
    #[test]
    fn the_furthest_reach_is_right_too() {
        let tilted = block(9, [0.3, -0.2, 0.9], 4.0, 25);
        let both = Settled::ends(true, true);
        for reach in [MOST_REACH - 1, MOST_REACH] {
            check(&tilted, reach, both, true);
        }
    }
}
