// SPDX-License-Identifier: AGPL-3.0-only

//! Surface nets over one brick's apron, visiting only the cells the surface
//! is in.
//!
//! The mesher was the `fast-surface-nets` crate, and it is a good one. What
//! it cannot know is what this field looks like: a brick's apron is 35,937
//! cells, and the surface passes through a sheet of them, a thousand or two.
//! The crate reads the eight corners of every cell to find that out, and for
//! a remesh that is most of the work: `stroke_profile` had it at 105
//! microseconds a brick on one core, bricks with no surface included.
//!
//! This finds the cells first. One pass over the samples makes a row of sign
//! bits for each of the apron's 34 by 34 lines, and a cell has the surface in
//! it exactly when its eight corners' bits are not all the same, which for a
//! whole line of cells is four rows and a few shifts. Only those cells are
//! then visited.
//!
//! **What it makes is what the crate makes, to the bit and in the same
//! order**: the same vertices, the same normals, the same triangles. The
//! arithmetic below is the crate's, written out a component at a time in the
//! order the crate's vector type does it, and a test holds the two to each
//! other on every kind of field there is a fixture for. That matters past
//! tidiness. A brick's mesh is compared, counted and exported all over this
//! codebase, and a mesher that differed in the last bit would move every one
//! of those numbers.

use crate::brick::{APRON_DIM, APRON_VOXELS};

/// One line of the apron, a bit a sample: set where the sample is negative.
type Row = u64;

const DIM: usize = APRON_DIM;
/// Cells along an axis: one fewer than samples.
const CELLS: usize = DIM - 1;
const STRIDE_Y: usize = DIM;
const STRIDE_Z: usize = DIM * DIM;

const _: () = assert!(DIM <= 64, "a line of the apron has to fit a row of bits");

/// Offsets of a cell's eight corners from its lowest, in the crate's order:
/// bit 0 is x, bit 1 is y, bit 2 is z.
const CORNERS: [usize; 8] = [
    0,
    1,
    STRIDE_Y,
    STRIDE_Y + 1,
    STRIDE_Z,
    STRIDE_Z + 1,
    STRIDE_Z + STRIDE_Y,
    STRIDE_Z + STRIDE_Y + 1,
];

/// What the mesher makes, and what it keeps between calls so that a remesh
/// allocates nothing once warm.
#[derive(Debug, Default, Clone)]
pub(crate) struct NetsBuffer {
    /// A vertex for each cell the surface is in, in apron coordinates.
    pub positions: Vec<[f32; 3]>,
    /// The field's gradient at each vertex. Not unit length.
    pub normals: Vec<[f32; 3]>,
    /// Three to a triangle, into `positions`.
    pub indices: Vec<u32>,
    /// The cell each vertex came from.
    pub surface_points: Vec<[u32; 3]>,
    /// The same cell as an index into the samples.
    surface_strides: Vec<u32>,
    /// From a cell's index to its vertex. Only the entries of cells with a
    /// vertex are ever read, so the rest are whatever they were.
    stride_to_index: Vec<u32>,
    rows: Vec<Row>,
}

/// Mesh an apron of samples. Negative is inside.
pub(crate) fn surface_nets(sdf: &[f32; APRON_VOXELS], out: &mut NetsBuffer) {
    out.positions.clear();
    out.normals.clear();
    out.indices.clear();
    out.surface_points.clear();
    out.surface_strides.clear();
    out.stride_to_index.resize(APRON_VOXELS, u32::MAX);
    out.rows.resize(DIM * DIM, 0);

    // Which samples are negative, a row of bits a line.
    let (mut any, mut all) = (0 as Row, !(0 as Row));
    let (lines, _) = sdf.as_chunks::<DIM>();
    for (row, line) in out.rows.iter_mut().zip(lines) {
        let bits = negatives(line);
        *row = bits;
        any |= bits;
        all &= bits;
    }
    let whole: Row = (1 << DIM) - 1;
    if any == 0 || all & whole == whole {
        // One sign throughout: no surface, and no cell to visit.
        return;
    }

    let cells: Row = (1 << CELLS) - 1;
    for z in 0..CELLS {
        for y in 0..CELLS {
            let row = y + z * DIM;
            let (a, b, c, d) =
                (out.rows[row], out.rows[row + 1], out.rows[row + DIM], out.rows[row + DIM + 1]);
            let some = a | b | c | d;
            let every = a & b & c & d;
            // A cell's corners are at x and x + 1 of each of the four lines.
            let mut mixed = (some | (some >> 1)) & !(every & (every >> 1)) & cells;
            while mixed != 0 {
                let x = mixed.trailing_zeros() as usize;
                mixed &= mixed - 1;
                let stride = x + y * STRIDE_Y + z * STRIDE_Z;
                vertex(sdf, [x, y, z], stride, out);
            }
        }
    }

    quads(sdf, out);
}

/// Which samples of a line are negative, a bit each, the first sample lowest.
///
/// This is the one pass over every sample, 39,304 of them a brick, so it is
/// worth four at a time where the machine can: a compare and a movemask, both
/// of which every x86-64 has. `-0.0` and a NaN are not negative here, as they
/// are not for `<`.
#[cfg(target_arch = "x86_64")]
#[inline]
fn negatives(line: &[f32; DIM]) -> Row {
    use std::arch::x86_64::{_mm_cmplt_ps, _mm_loadu_ps, _mm_movemask_ps, _mm_setzero_ps};
    let mut bits: Row = 0;
    let (fours, rest) = line.as_chunks::<4>();
    let mut at = 0;
    for four in fours {
        // SAFETY: SSE2 is part of x86-64, and `four` is four floats, which is
        // what an unaligned load reads.
        let mask =
            unsafe { _mm_movemask_ps(_mm_cmplt_ps(_mm_loadu_ps(four.as_ptr()), _mm_setzero_ps())) };
        bits |= (mask as Row) << at;
        at += 4;
    }
    for (index, value) in rest.iter().enumerate() {
        bits |= Row::from(*value < 0.0) << (at + index);
    }
    bits
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn negatives(line: &[f32; DIM]) -> Row {
    let mut bits: Row = 0;
    for (x, value) in line.iter().enumerate() {
        bits |= Row::from(*value < 0.0) << x;
    }
    bits
}

/// The vertex of one cell the surface is in: the mean of where the surface
/// crosses the cell's edges, and the field's gradient there.
#[inline]
fn vertex(sdf: &[f32; APRON_VOXELS], cell: [usize; 3], stride: usize, out: &mut NetsBuffer) {
    let corners = &sdf[stride..stride + CORNERS[7] + 1];
    let d = [
        corners[CORNERS[0]],
        corners[CORNERS[1]],
        corners[CORNERS[2]],
        corners[CORNERS[3]],
        corners[CORNERS[4]],
        corners[CORNERS[5]],
        corners[CORNERS[6]],
        corners[CORNERS[7]],
    ];

    // The mean of the edge crossings, an edge at a time.
    //
    // The crate has this as `interp2 * corner1 + interp1 * corner2` on
    // vectors, where the corners are the ends of the edge and every
    // coordinate of them is nought or one. Written out by coordinate that is
    // one of three things, and each is here as what it comes to:
    //
    // - along the edge, where the ends are nought and one: `interp2 * 0.0 +
    //   interp1`, kept as written, because nought times a number is not
    //   always the nought it looks like;
    // - across it where both ends are one: `interp2 + interp1`, which is not
    //   always one and so is added up as it is;
    // - across it where both ends are nought: nought, and adding it to a sum
    //   that began at nought and has only had numbers that are not negative
    //   added to it changes nothing, so it is left out.
    //
    // The edges are in the crate's order, because the sum is taken in it.
    let mut count = 0u32;
    let mut sum = [0.0f32; 3];
    macro_rules! edge {
        ($first:expr, $second:expr, along $along:expr, at one [$($one:expr),*]) => {
            let (d1, d2) = (d[$first], d[$second]);
            if (d1 < 0.0) != (d2 < 0.0) {
                count += 1;
                let interp1 = d1 / (d1 - d2);
                let interp2 = 1.0 - interp1;
                sum[$along] += interp2 * 0.0 + interp1;
                $( sum[$one] += interp2 + interp1; )*
            }
        };
    }
    edge!(0b000, 0b001, along 0, at one []);
    edge!(0b000, 0b010, along 1, at one []);
    edge!(0b000, 0b100, along 2, at one []);
    edge!(0b001, 0b011, along 1, at one [0]);
    edge!(0b001, 0b101, along 2, at one [0]);
    edge!(0b010, 0b011, along 0, at one [1]);
    edge!(0b010, 0b110, along 2, at one [1]);
    edge!(0b011, 0b111, along 2, at one [0, 1]);
    edge!(0b100, 0b101, along 0, at one [2]);
    edge!(0b100, 0b110, along 1, at one [2]);
    edge!(0b101, 0b111, along 1, at one [0, 2]);
    edge!(0b110, 0b111, along 0, at one [1, 2]);
    let count = count as f32;
    let s = [sum[0] / count, sum[1] / count, sum[2] / count];

    out.stride_to_index[stride] = out.positions.len() as u32;
    out.positions.push([cell[0] as f32 + s[0], cell[1] as f32 + s[1], cell[2] as f32 + s[2]]);
    out.normals.push(gradient(&d, s));
    out.surface_points.push([cell[0] as u32, cell[1] as u32, cell[2] as u32]);
    out.surface_strides.push(stride as u32);
}

/// The field's gradient at `s` inside a cell: along each axis, the four
/// edges that run that way, blended by where `s` is across the other two.
///
/// The crate's, a coordinate at a time. Its `yzx` and `zxy` are which of the
/// other two coordinates each line reads.
#[inline]
fn gradient(d: &[f32; 8], s: [f32; 3]) -> [f32; 3] {
    let neg = [1.0 - s[0], 1.0 - s[1], 1.0 - s[2]];
    // The four edge differences along one axis, and the two coordinates
    // across it, in the order the crate blends them.
    #[inline(always)]
    fn blend(edges: [f32; 4], neg: [f32; 3], s: [f32; 3], u: usize, v: usize) -> f32 {
        let [d00, d10, d01, d11] = edges;
        neg[u] * neg[v] * d00 + neg[u] * s[v] * d10 + s[u] * neg[v] * d01 + s[u] * s[v] * d11
    }
    [
        blend(
            [d[0b001] - d[0b000], d[0b101] - d[0b100], d[0b011] - d[0b010], d[0b111] - d[0b110]],
            neg,
            s,
            1,
            2,
        ),
        blend(
            [d[0b010] - d[0b000], d[0b011] - d[0b001], d[0b110] - d[0b100], d[0b111] - d[0b101]],
            neg,
            s,
            2,
            0,
        ),
        blend(
            [d[0b100] - d[0b000], d[0b110] - d[0b010], d[0b101] - d[0b001], d[0b111] - d[0b011]],
            neg,
            s,
            0,
            1,
        ),
    ]
}

/// A quad for every edge of the lattice the surface crosses, between the
/// vertices of the four cells round that edge.
fn quads(sdf: &[f32; APRON_VOXELS], out: &mut NetsBuffer) {
    let NetsBuffer { positions, indices, surface_points, surface_strides, stride_to_index, .. } =
        out;
    // No faces on the highest plane of cells: the brick beyond owns those,
    // which is what makes neighbours tile with no gap and no doubled quad.
    let last = (CELLS - 1) as u32;
    for (&[x, y, z], &stride) in surface_points.iter().zip(surface_strides.iter()) {
        let p = stride as usize;
        if y != 0 && z != 0 && x != last {
            quad(sdf, stride_to_index, positions, p, p + 1, STRIDE_Y, STRIDE_Z, indices);
        }
        if x != 0 && z != 0 && y != last {
            quad(sdf, stride_to_index, positions, p, p + STRIDE_Y, STRIDE_Z, 1, indices);
        }
        if x != 0 && y != 0 && z != last {
            quad(sdf, stride_to_index, positions, p, p + STRIDE_Z, 1, STRIDE_Y, indices);
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn quad(
    sdf: &[f32; APRON_VOXELS],
    stride_to_index: &[u32],
    positions: &[[f32; 3]],
    p1: usize,
    p2: usize,
    axis_b: usize,
    axis_c: usize,
    indices: &mut Vec<u32>,
) {
    let negative_face = match (sdf[p1] < 0.0, sdf[p2] < 0.0) {
        (true, false) => false,
        (false, true) => true,
        _ => return,
    };

    // Seen from the front:
    // v1 v3
    // v2 v4
    let v1 = stride_to_index[p1];
    let v2 = stride_to_index[p1 - axis_b];
    let v3 = stride_to_index[p1 - axis_c];
    let v4 = stride_to_index[p1 - axis_b - axis_c];
    let apart = |a: u32, b: u32| {
        let (a, b) = (positions[a as usize], positions[b as usize]);
        let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
        x * x + y * y + z * z
    };
    // Split along the shorter diagonal.
    let quad = if apart(v1, v4) < apart(v2, v3) {
        if negative_face { [v1, v4, v2, v1, v3, v4] } else { [v1, v2, v4, v1, v4, v3] }
    } else if negative_face {
        [v2, v3, v4, v2, v1, v3]
    } else {
        [v2, v4, v3, v2, v3, v1]
    };
    indices.extend_from_slice(&quad);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Noise;
    use fast_surface_nets::ndshape::ConstShape3u32;
    use fast_surface_nets::{SurfaceNetsBuffer, surface_nets as theirs};
    use proptest::prelude::*;

    type Shape = ConstShape3u32<34, 34, 34>;

    /// Hold this mesher to the crate's on one apron: every buffer, bit for
    /// bit and in order.
    fn same_as_the_crate(sdf: &[f32; APRON_VOXELS], what: &str) -> usize {
        let mut reference = SurfaceNetsBuffer::default();
        theirs(sdf.as_slice(), &Shape {}, [0; 3], [(APRON_DIM - 1) as u32; 3], &mut reference);
        let mut ours = NetsBuffer::default();
        // Twice into one buffer, so that what a call leaves behind is seen
        // not to reach the next.
        surface_nets(sdf, &mut ours);
        surface_nets(sdf, &mut ours);

        let bits = |list: &[[f32; 3]]| -> Vec<[u32; 3]> {
            list.iter().map(|v| [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()]).collect()
        };
        assert_eq!(ours.surface_points, reference.surface_points, "{what}: the cells");
        assert_eq!(bits(&ours.positions), bits(&reference.positions), "{what}: the vertices");
        assert_eq!(bits(&ours.normals), bits(&reference.normals), "{what}: the normals");
        assert_eq!(ours.indices, reference.indices, "{what}: the triangles");
        ours.positions.len()
    }

    fn apron(of: impl Fn(f32, f32, f32) -> f32) -> Box<[f32; APRON_VOXELS]> {
        let mut samples = vec![0.0f32; APRON_VOXELS];
        for (index, sample) in samples.iter_mut().enumerate() {
            let (x, y, z) = (index % DIM, index / DIM % DIM, index / (DIM * DIM));
            *sample = of(x as f32, y as f32, z as f32).clamp(-3.0, 3.0);
        }
        samples.into_boxed_slice().try_into().expect("an apron's worth")
    }

    #[test]
    fn a_ball_a_plane_and_a_corner_mesh_as_the_crate_meshes_them() {
        let ball = apron(|x, y, z| {
            ((x - 16.3).powi(2) + (y - 15.1).powi(2) + (z - 17.7).powi(2)).sqrt() - 11.4
        });
        assert!(same_as_the_crate(&ball, "a ball") > 500);
        // Through the lattice exactly: samples of nought, which are not
        // negative, and of minus nought, which is not either.
        let plane = apron(|x, _, _| x - 16.0);
        assert!(same_as_the_crate(&plane, "a plane on the lattice") > 500);
        let minus_nought = apron(|x, _, _| if x == 16.0 { -0.0 } else { x - 16.0 });
        same_as_the_crate(&minus_nought, "a plane of minus nought");
        // Against every face of the apron, the first cells and the last.
        let corner = apron(|x, y, z| (x - 0.5).min(y - 0.5).min(z - 0.5).min(32.5 - x));
        assert!(same_as_the_crate(&corner, "a corner against the apron's faces") > 500);
        let thin = apron(|x, y, _| ((x - 20.0).abs() - 0.4).max(y - 30.0));
        same_as_the_crate(&thin, "a sheet thinner than a cell");
    }

    /// The row of bits is `<` on every sample, whatever the samples: noughts
    /// of both signs, the smallest numbers there are, and things that are not
    /// numbers.
    #[test]
    fn a_row_of_bits_is_less_than_nought_sample_by_sample() {
        let odd = [0.0f32, -0.0, f32::MIN_POSITIVE, -f32::MIN_POSITIVE, f32::NAN, -f32::NAN];
        let mut random = Noise::seeded(7);
        for _ in 0..500 {
            let line: [f32; DIM] = std::array::from_fn(|_| match random.below(4) {
                0 => odd[random.below(odd.len())],
                1 => 3.0,
                2 => -3.0,
                _ => random.below(2001) as f32 / 1000.0 - 1.0,
            });
            let expected = line
                .iter()
                .enumerate()
                .fold(0 as Row, |bits, (x, value)| bits | (Row::from(*value < 0.0) << x));
            assert_eq!(negatives(&line), expected, "{line:?}");
        }
    }

    #[test]
    fn an_apron_of_one_sign_has_no_mesh() {
        for value in [3.0f32, -3.0, 0.0] {
            let flat = apron(|_, _, _| value);
            assert_eq!(same_as_the_crate(&flat, "one value throughout"), 0);
        }
        // And a buffer that held a mesh is empty after one.
        let mut buffer = NetsBuffer::default();
        surface_nets(&apron(|x, _, _| x - 16.5), &mut buffer);
        assert!(!buffer.indices.is_empty());
        surface_nets(&apron(|_, _, _| 3.0), &mut buffer);
        assert!(buffer.indices.is_empty() && buffer.positions.is_empty());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Noise, which is every case at once: cells with one corner in,
        /// with seven, surfaces through every face of the apron.
        #[test]
        fn noise_meshes_as_the_crate_meshes_it(seed in any::<u64>(), rough in 0.0f32..1.0) {
            let mut random = Noise::seeded(seed);
            let mut samples = vec![0.0f32; APRON_VOXELS];
            for (index, sample) in samples.iter_mut().enumerate() {
                let x = (index % DIM) as f32;
                let noise = (random.below(2001) as f32 / 1000.0 - 1.0) * 6.0 * rough;
                *sample = (x - 16.5 + noise).clamp(-3.0, 3.0);
            }
            let samples: Box<[f32; APRON_VOXELS]> =
                samples.into_boxed_slice().try_into().expect("an apron's worth");
            same_as_the_crate(&samples, "noise");
        }

        /// A tilted plane at any angle and offset: the smooth surfaces a
        /// sculpt is mostly made of.
        #[test]
        fn a_plane_at_any_angle_meshes_as_the_crate_meshes_it(
            nx in -1.0f32..1.0, ny in -1.0f32..1.0, nz in -1.0f32..1.0, offset in 0.0f32..33.0,
        ) {
            prop_assume!(nx.abs() + ny.abs() + nz.abs() > 0.1);
            let length = (nx * nx + ny * ny + nz * nz).sqrt();
            let plane = apron(|x, y, z| {
                ((x - 16.5) * nx + (y - 16.5) * ny + (z - 16.5) * nz) / length + offset - 16.5
            });
            same_as_the_crate(&plane, "a plane");
        }
    }
}
