// SPDX-License-Identifier: AGPL-3.0-only

//! A join wider than the field knows distances for.
//!
//! `docs/STUDIO-PLAN.md` section 18, stage C9. A soft join is a smooth
//! minimum of two distances: the body's and the solid's. The body's field
//! is a distance for three voxels either side of its surface and a clamp
//! past that, so a join wider than [`NARROW`] voxels would round off
//! something that is not there, and until this stage a wider fillet was
//! "a pass of the Smooth brush afterwards", by hand.
//!
//! Here the body's distance is WORKED OUT further than it is kept, in a
//! box about where the solid meets it, and the join is then the same
//! smooth minimum at any width up to [`super::MOST_BLEND`].
//!
//! # The distance, further out
//!
//! [`widen`]. Each voxel within [`SEED`] voxels of the surface knows where
//! the surface is: itself, less its distance along its gradient. From
//! those a front moves out a voxel at a time, each voxel on it handing
//! its neighbours the point of the surface it holds, and a voxel's
//! distance is to the nearest point it was handed ("dead reckoning":
//! Grevera, 2004, here spread as a front and not swept). It is not exact,
//! since the nearest point of a voxel is not always one a neighbour
//! holds, and a test here holds it to 0.15 of a voxel on a ball.
//!
//! **Swept first, and dropped.** The first build swept the whole box
//! about the solid, from each of its eight corners to the opposite one.
//! Measured in the development build, the build the tests run in: a knob
//! of radius 10 voxels joined over 3 took 83 ms and over 8 took 333 ms,
//! and a hung cable of 24 runs joined over 4 took a second, on every
//! move of the pointer. Nearly all of such a box is far from one surface
//! or the other. The front starts at the body's surface where the solid
//! is near and stops where it has gone as far as the join can reach. The sign is the field's own, which a clamp still has.
//!
//! A solid that is a copy of a body ([`super::Lent`]) has the same
//! trouble with its own distance, and says so ([`super::Solid::sure`]):
//! its distance is widened the same way.
//!
//! # What changes, and how far
//!
//! The smooth minimum of `a` and `b` over a width `w` is less than their
//! minimum by `(w - |a - b|)^2 / 4w`, where the two are within `w` of
//! each other, and by no more than `w / 4`. The field keeps values below
//! three voxels. So nothing changes where either distance is more than
//! `3 + w + w/4` voxels ([`far`]), which is how far the distances are
//! worked out, and the box they are worked out in is the body's surface
//! within twice that of the solid's, and that far again about it.
//!
//! # Tried first, and dropped: a blur along the seam
//!
//! The first build of this stage blurred the field about the line where
//! the two surfaces cross. A blur is even-handed: it fills a crease and
//! wears down a corner alike, and the insert is all corners. Measured, in
//! the test fixture (a ball of radius 30 voxels with a knob of radius 10
//! on it), by where the surface was along the crease's bisector: 0.35
//! voxels with no blur, 1.30 at the best width (a half-width of 3), 0.70
//! at 6 and BELOW where the body had been, at -0.65, at 8, where the knob
//! itself had melted. A fillet adds in the crease and touches nothing
//! else, at any width.

use glam::{IVec3, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashSet;

use super::{Join, Solid, nearer, taken_out};
use crate::brick::{BRICK_DIM, BrickCoord, INSIDE, OUTSIDE};
use crate::mask::PROTECTED;
use crate::region::FieldRegion;
use crate::volume::{BrickPreview, BrickVerdict, Volume};

/// The widest join the field's own distances will do, in voxels: what
/// [`super::MOST_BLEND`] was before this stage.
pub(super) const NARROW: f32 = 2.0;

/// A voxel within this many voxels of the surface says where the surface
/// is. Well inside the three the field keeps, so that the gradient it is
/// found along is read from distances and not from a clamp.
const SEED: f32 = 1.5;

/// The most voxels a wide join is worked out over. Past it the join is
/// left at [`NARROW`]: a box this big is a solid most of the size of the
/// model joined at the widest there is, and working it out is a wait of
/// seconds on every move of the pointer.
const MOST_VOXELS: i64 = 12 << 20;

/// How far from either surface a join of `width` voxels can change the
/// field, in voxels. See the module documentation.
pub(super) fn far(width: f32) -> f32 {
    OUTSIDE + width * 1.25
}

/// How near the solid a part of the body's surface has to be for the
/// body's distance to be spread from it, in voxels.
///
/// A voxel that changes is within the join's `width` of being as far from
/// one surface as from the other, and the nearer of the two is within
/// `3 + width / 4` of it (the module documentation). The body's surface
/// nearest it is as far from the solid, at most, as the voxel is from
/// both together: less than twice the nearer and the width, which is
/// `6 + 1.5 * width` (arithmetic).
fn near_enough(width: f32) -> f32 {
    2.0 * OUTSIDE + width * 1.5
}

/// What [`widen`] works in, kept from one join to the next: a shape being
/// dragged out is widened on every move of the pointer, over a box of some
/// hundreds of thousands of voxels each time.
#[derive(Debug, Default)]
pub(super) struct Spread {
    /// The point of the surface each voxel holds. Read only for a voxel
    /// that has been given one, so what is left from the last time is never
    /// looked at.
    point: Vec<[f32; 3]>,
    /// Whether a voxel has offered the point it now holds to its
    /// neighbours.
    offered: Vec<u8>,
    /// The voxels with a point to offer, by how far from it they are: each
    /// its place in the box, and where that is in y and in z.
    fronts: Vec<Vec<[u32; 3]>>,
}

/// Distances further out than `raw` knows them: `raw` is a field over a
/// box of `size`, in voxels, right within `seed` of its surface and a
/// clamp or an underestimate past that, with its sign right everywhere.
///
/// It spreads from the voxels within `seed` of the surface that `from`
/// picks, as far as `most` voxels (inside the surface, and outside it)
/// and no further, and what comes back is the distance wherever the
/// nearest of the surface is a part it spread from. Everywhere it did
/// not reach is infinite, with the sign.
///
/// A voxel near a part of the surface it did NOT spread from is given
/// its distance to a part it did, which is too far. It is for the caller
/// to pick every part that matters to it: the front is not let run along
/// the surface to find the rest, because it would run along all of it.
pub(super) fn widen(
    raw: &[f32],
    size: IVec3,
    seed: f32,
    most: (f32, f32),
    from: impl Fn(usize) -> bool,
    spread: &mut Spread,
) -> Vec<f32> {
    let count = raw.len();
    debug_assert_eq!(count as i64, size.as_i64vec3().element_product());
    let (row, plane) = (size.x as usize, (size.x * size.y) as usize);
    let at = |x: i32, y: i32, z: i32| x as usize + y as usize * row + z as usize * plane;
    let place = |index: usize| {
        IVec3::new((index % row) as i32, ((index % plane) / row) as i32, (index / plane) as i32)
    };
    // Where the surface is, by a voxel that is within `seed` of it:
    // itself, less its distance along its gradient.
    let surface_by = |index: usize| {
        let IVec3 { x, y, z } = place(index);
        let read = |x: i32, y: i32, z: i32| {
            raw[at(x.clamp(0, size.x - 1), y.clamp(0, size.y - 1), z.clamp(0, size.z - 1))]
        };
        let slope = Vec3::new(
            read(x + 1, y, z) - read(x - 1, y, z),
            read(x, y + 1, z) - read(x, y - 1, z),
            read(x, y, z + 1) - read(x, y, z - 1),
        );
        let normal = slope.try_normalize()?;
        Some((Vec3::new(x as f32, y as f32, z as f32) - normal * raw[index]).to_array())
    };

    // A front that moves out from the surface: each voxel on it offers
    // the point of the surface it holds to its neighbours, and a
    // neighbour that is nearer to that point than to the one it has
    // takes it and joins the front. The front is kept in order of
    // distance, by half voxels, so that a voxel has nearly always got
    // its nearest point by the time it offers one on: counted on the
    // knob of the cost test below, a front taken a round at a time
    // offered from twice as many voxels as it reached.
    let mut near = vec![f32::INFINITY; count];
    let Spread { point, offered, fronts } = spread;
    if point.len() < count {
        point.resize(count, [0.0; 3]);
    }
    offered.clear();
    offered.resize(count, 0);
    let order = |distance: f32| (distance * 2.0) as usize;
    let last = order(most.0.max(most.1) + 1.0) + 1;
    fronts.iter_mut().for_each(Vec::clear);
    if fronts.len() < last + 1 {
        fronts.resize_with(last + 1, Vec::new);
    }
    // A voxel on a front goes with where it is in y and z, so that taking
    // it off again is not three divisions to find out.
    let mut index = 0;
    for z in 0..size.z as u32 {
        for y in 0..size.y as u32 {
            for _ in 0..size.x {
                if raw[index].abs() <= seed
                    && from(index)
                    && let Some(surface) = surface_by(index)
                {
                    near[index] = raw[index].abs();
                    point[index] = surface;
                    fronts[order(near[index]).min(last)].push([index as u32, y, z]);
                }
                index += 1;
            }
        }
    }
    // How far a point may be from a voxel that takes it: inside the
    // surface, and outside it.
    let limits = (most.0 + 1.0, most.1 + 1.0);
    for now in 0..=last {
        let mut taken = 0;
        while taken < fronts[now].len() {
            let [index, y, z] = fronts[now][taken];
            let (index, y, z) = (index as usize, y as i32, z as i32);
            taken += 1;
            // Since it was put here it has been handed a nearer point,
            // and was put on an earlier front for that.
            if order(near[index]).min(last) < now {
                continue;
            }
            // It has offered the point it holds already: it is on a front
            // once for each point it was ever handed, and when two of
            // those fall in the same half voxel it comes round again
            // holding what it held the first time. Offering that again
            // changes nothing, since no neighbour that refused it has got
            // further from it since. Counted on a ball dragged out of a
            // ball, this was half of all the offering there was.
            if offered[index] != 0 {
                continue;
            }
            offered[index] = 1;
            let x = (index - y as usize * row - z as usize * plane) as i32;
            let held = point[index];
            // A neighbour that takes the point: what it now holds, that it
            // has not offered it, and the front it joins.
            macro_rules! give {
                ($other:expr, $away:expr, $y:expr, $z:expr) => {{
                    let (other, away): (usize, f32) = ($other, $away);
                    near[other] = away;
                    point[other] = held;
                    offered[other] = 0;
                    fronts[order(away).min(last).max(now)].push([
                        other as u32,
                        $y as u32,
                        $z as u32,
                    ]);
                }};
            }
            // The neighbours a line of three at a time: the three along x
            // at each of the nine places about the voxel in y and z, in
            // the order they were taken one by one, which is the order
            // they join a front in.
            //
            // Away from the faces of the box, which is nearly every voxel,
            // each line is asked as four: the three and the one after
            // them, which is read and not asked.
            #[cfg(target_arch = "x86_64")]
            if x >= 1 && x + 2 < size.x && y >= 1 && y + 1 < size.y && z >= 1 && z + 1 < size.z {
                use std::arch::x86_64::*;
                // What is beside the voxel in y, and in z, and how far
                // each is from the point, squared.
                let beside = |at: i32, held: f32| {
                    [(at - 1) as f32 - held, at as f32 - held, (at + 1) as f32 - held]
                        .map(|off| off * off)
                };
                let (wide, deep) = (beside(y, held[1]), beside(z, held[2]));
                // The first of the four of the line one less in y and z.
                let corner = index - row - plane - 1;
                // SAFETY: every intrinsic here is SSE2, which is part of
                // x86-64. Each load is of four floats of `raw` or of
                // `near`, which are both `count` long, beginning at the
                // voxel one less in x of a neighbour in y and z. The test
                // above is that those four are in the box: the voxel is
                // not on its first line or plane or its last, and has one
                // before it on its line and two after. The store is to an
                // array of four.
                unsafe {
                    let ox = _mm_sub_ps(
                        _mm_setr_ps((x - 1) as f32, x as f32, (x + 1) as f32, (x + 2) as f32),
                        _mm_set1_ps(held[0]),
                    );
                    let across = _mm_mul_ps(ox, ox);
                    let (inner, outer) = (_mm_set1_ps(limits.0), _mm_set1_ps(limits.1));
                    // One line of the nine, written out for each and not
                    // gone round: going round an array of three inside an
                    // array of three cost as much as the asking did.
                    macro_rules! line {
                        ($dy:literal, $dz:literal) => {{
                            let first = corner + $dy * row + $dz * plane;
                            debug_assert!(first + 4 <= count);
                            let field = _mm_loadu_ps(raw.as_ptr().add(first));
                            let has = _mm_loadu_ps(near.as_ptr().add(first));
                            // `(ox * ox + oy * oy + oz * oz).sqrt()`
                            let away = _mm_sqrt_ps(_mm_add_ps(
                                _mm_add_ps(across, _mm_set1_ps(wide[$dy])),
                                _mm_set1_ps(deep[$dz]),
                            ));
                            // `if raw[other] < 0.0 { most.0 } else { most.1 }`,
                            // and one.
                            let inside = _mm_cmplt_ps(field, _mm_setzero_ps());
                            let limit =
                                _mm_or_ps(_mm_and_ps(inside, inner), _mm_andnot_ps(inside, outer));
                            // `away <= limit + 1.0 && away + 1.0e-4 <
                            // near[other]`
                            let takes = _mm_and_ps(
                                _mm_cmple_ps(away, limit),
                                _mm_cmplt_ps(_mm_add_ps(away, _mm_set1_ps(1.0e-4)), has),
                            );
                            // Not the voxel itself, and not the fourth.
                            let asked = if ($dy, $dz) == (1, 1) { 0b101 } else { 0b111 };
                            let taken_by = _mm_movemask_ps(takes) & asked;
                            if taken_by != 0 {
                                let mut far = [0.0_f32; 4];
                                _mm_storeu_ps(far.as_mut_ptr(), away);
                                for lane in 0..3 {
                                    if taken_by >> lane & 1 != 0 {
                                        give!(first + lane, far[lane], y + $dy - 1, z + $dz - 1);
                                    }
                                }
                            }
                        }};
                    }
                    line!(0, 0);
                    line!(1, 0);
                    line!(2, 0);
                    line!(0, 1);
                    line!(1, 1);
                    line!(2, 1);
                    line!(0, 2);
                    line!(1, 2);
                    line!(2, 2);
                }
                continue;
            }
            // At a face of the box, and anywhere on a machine that is not
            // x86-64: one neighbour at a time, those that are in the box.
            for dz in -1..=1_i32 {
                let nz = z + dz;
                if nz < 0 || nz >= size.z {
                    continue;
                }
                let oz = nz as f32 - held[2];
                for dy in -1..=1_i32 {
                    let ny = y + dy;
                    if ny < 0 || ny >= size.y {
                        continue;
                    }
                    let oy = ny as f32 - held[1];
                    let middle = (index as isize
                        + dy as isize * row as isize
                        + dz as isize * plane as isize) as usize;
                    for dx in -1..=1_i32 {
                        let nx = x + dx;
                        if nx < 0 || nx >= size.x || (dx, dy, dz) == (0, 0, 0) {
                            continue;
                        }
                        let other = (middle as isize + dx as isize) as usize;
                        let limit = if raw[other] < 0.0 { limits.0 } else { limits.1 };
                        let ox = nx as f32 - held[0];
                        let away = (ox * ox + oy * oy + oz * oz).sqrt();
                        if away <= limit && away + 1.0e-4 < near[other] {
                            give!(other, away, ny, nz);
                        }
                    }
                }
            }
        }
    }

    for (distance, value) in near.iter_mut().zip(raw) {
        if *value < 0.0 {
            *distance = -*distance;
        }
    }
    near
}

impl Join {
    /// The part of [`Join::add`] that a join wider than [`NARROW`] adds:
    /// the smooth minimum over `width` voxels, with the body's distance
    /// worked out as far as that needs. `solid` is joined at [`NARROW`]
    /// already, and this only ever adds to what that did (or, cutting,
    /// takes more). See the module documentation.
    pub(super) fn add_wide(
        &mut self,
        volume: &mut Volume,
        solid: &dyn Solid,
        cut: bool,
        width: f32,
    ) {
        let voxel_size = volume.voxel_size();
        let far = far(width);
        let reach = far.ceil() as i32;
        let (low, high) = solid.bounds();
        let pad = Vec3::splat((2.0 * far + 2.0) * voxel_size);
        let (lo, hi) = volume.voxel_bounds(low - pad, high + pad);
        if (hi - lo + IVec3::ONE).as_i64vec3().element_product() > MOST_VOXELS {
            return;
        }
        volume.snapshot_before(lo, hi, &mut self.region);
        let region = &self.region;

        // The body's surface, as it was, near the solid: the voxels the
        // body's distance is spread from. See `near_enough`.
        let near_enough = near_enough(width);
        let starts: Vec<IVec3> = (lo.z..=hi.z)
            .into_par_iter()
            .flat_map_iter(|z| {
                // A line of the copy at a time: all but a few voxels of the
                // box are off the body's surface, and asking the region for
                // each by its coordinate was a seventh of a shape's
                // set-down to find that out (`docs/PERFORMANCE.md`).
                let mut found = Vec::new();
                for y in lo.y..=hi.y {
                    // Sixteen at a time first: written so that it is one
                    // comparison of sixteen and not sixteen of one.
                    for (part, values) in region.row(lo.x, hi.x, y, z).chunks(16).enumerate() {
                        let any =
                            values.iter().fold(false, |any, value| any | (value.abs() <= SEED));
                        if !any {
                            continue;
                        }
                        for (step, value) in values.iter().enumerate() {
                            if value.abs() > SEED {
                                continue;
                            }
                            let voxel = IVec3::new(lo.x + (part * 16 + step) as i32, y, z);
                            let off = solid.distance(voxel.as_vec3() * voxel_size) / voxel_size;
                            if off.abs() <= near_enough {
                                found.push(voxel);
                            }
                        }
                    }
                }
                found
            })
            .collect();
        if starts.is_empty() {
            return;
        }
        let (found_lo, found_hi) =
            starts.iter().fold((IVec3::MAX, IVec3::MIN), |(low, high), voxel| {
                (low.min(*voxel), high.max(*voxel))
            });
        // A solid that is not sure of its own distance has it spread
        // from its own surface, which can be as far again from the body's.
        let sure = solid.sure() / voxel_size;
        let about = if sure < far { 2 * reach } else { reach };
        let work_lo = (found_lo - about).max(lo);
        let work_hi = (found_hi + about).min(hi);
        let size = work_hi - work_lo + IVec3::ONE;
        let (row, plane) = (size.x as usize, (size.x * size.y) as usize);
        let voxel_at = |index: usize| {
            work_lo
                + IVec3::new(
                    (index % row) as i32,
                    ((index % plane) / row) as i32,
                    (index / plane) as i32,
                )
        };
        let count = size.element_product() as usize;
        // Which voxels of the box the body's distance is spread from. It
        // was a set of their coordinates, asked of every voxel near the
        // surface.
        let mut picked = vec![false; count];
        for voxel in &starts {
            let local = *voxel - work_lo;
            picked[local.x as usize + local.y as usize * row + local.z as usize * plane] = true;
        }

        let mut raw = vec![0.0_f32; count];
        raw.par_chunks_mut(row).enumerate().for_each(|(line, values)| {
            let y = work_lo.y + (line % size.y as usize) as i32;
            let z = work_lo.z + (line / size.y as usize) as i32;
            values.copy_from_slice(region.row(work_lo.x, work_hi.x, y, z));
        });
        // A voxel the join adds to holds more than the least there is,
        // so it is in the band or outside the body, and one it takes
        // from is in the band or inside: the other side is spread into
        // band-deep and no deeper.
        let shallow = OUTSIDE + 0.5;
        let sides = if cut { (far, shallow) } else { (shallow, far) };
        let body = widen(&raw, size, SEED, sides, |index| picked[index], &mut self.spread);

        let lent = (sure < far).then(|| {
            raw.par_iter_mut().enumerate().for_each(|(index, value)| {
                *value = solid.distance(voxel_at(index).as_vec3() * voxel_size) / voxel_size;
            });
            widen(&raw, size, (sure * 0.8).min(SEED), (far, far), |_| true, &mut self.spread)
        });

        // What each voxel is to hold, worked out from the body as it now
        // stands and before any of it is written, and which bricks hold
        // one that changes: a brick with none is not read, made dense or
        // recorded.
        let mut now = FieldRegion::new();
        volume.snapshot(work_lo, work_hi, &mut now);
        // A line of the box at a time and not a voxel at a time: nearly
        // all of the box is past the first question, and asked through a
        // call a voxel it was a twentieth of a shape's set-down to hear
        // "no" (`docs/PERFORMANCE.md`).
        let mut wanted = raw;
        let changed: FxHashSet<BrickCoord> = wanted
            .par_chunks_mut(row)
            .enumerate()
            .flat_map_iter(|(line, wanted)| {
                let y = work_lo.y + (line % size.y as usize) as i32;
                let z = work_lo.z + (line / size.y as usize) as i32;
                let first = line * row;
                let body = &body[first..first + row];
                let now = now.row(work_lo.x, work_hi.x, y, z);
                let mut bricks: Vec<BrickCoord> = Vec::new();
                for (step, wanted) in wanted.iter_mut().enumerate() {
                    *wanted = f32::NAN;
                    let before = body[step];
                    if before.abs() >= far {
                        continue;
                    }
                    let voxel = IVec3::new(work_lo.x + step as i32, y, z);
                    let off = match &lent {
                        Some(lent) => lent[first + step],
                        None => solid.distance(voxel.as_vec3() * voxel_size) / voxel_size,
                    };
                    if off.abs() >= far || off.is_nan() {
                        continue;
                    }
                    let value = now[step];
                    let joined = if cut {
                        taken_out(value, before, off, width)
                    } else {
                        value.min(nearer(before, off, width))
                    }
                    .clamp(INSIDE, OUTSIDE);
                    if joined != value {
                        *wanted = joined;
                        let coord = BrickCoord::containing(voxel);
                        if bricks.last() != Some(&coord) {
                            bricks.push(coord);
                        }
                    }
                }
                bricks
            })
            .collect();
        if changed.is_empty() {
            return;
        }
        let decide = |preview: &BrickPreview| {
            if preview.mask == Some(PROTECTED) || !changed.contains(&preview.coord) {
                BrickVerdict::Skip
            } else {
                BrickVerdict::Whole
            }
        };
        // Which voxels of a brick are not to be visited: all that are to
        // hold what they hold. A brick the join changes has a few hundred
        // such voxels in its thirty-two thousand, and visiting them all to
        // find that out was a twentieth of a set-down.
        let leave = |coord: BrickCoord, lines: &mut [u32; BRICK_DIM * BRICK_DIM]| {
            let local = coord.origin() - work_lo;
            for (index, line) in lines.iter_mut().enumerate() {
                let y = local.y + (index % BRICK_DIM) as i32;
                let z = local.z + (index / BRICK_DIM) as i32;
                if y < 0 || y >= size.y || z < 0 || z >= size.z {
                    *line = u32::MAX;
                    continue;
                }
                let first = y as usize * row + z as usize * plane;
                let mut left = 0_u32;
                for across in 0..BRICK_DIM as i32 {
                    let x = local.x + across;
                    if x < 0 || x >= size.x || wanted[first + x as usize].is_nan() {
                        left |= 1 << across;
                    }
                }
                *line = left;
            }
        };
        volume.edit_voxels_leaving(
            work_lo,
            work_hi,
            true,
            decide,
            leave,
            |voxel, _, value, free| {
                let local = voxel - work_lo;
                let wanted =
                    wanted[local.x as usize + local.y as usize * row + local.z as usize * plane];
                if wanted.is_nan() { value } else { value + (wanted - value) * free }
            },
        );
        self.boxes.push((work_lo, work_hi));
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::{Form, How, Lent, MOST_BLEND, Placed, Step};
    use super::*;

    const CENTRE: Vec3 = Vec3::splat(48.0);
    const RADIUS: f32 = 30.0;

    fn ball() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(CENTRE, RADIUS);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    fn knob(half: f32) -> Placed {
        Placed::on(Form::Ball, CENTRE + Vec3::Y * RADIUS, Vec3::Y, Vec3::X, half, 0.0)
            .expect("a knob")
    }

    /// The way from the fixture ball's middle to where a knob of `half`
    /// meets it: the crease, in the plane of X and Y.
    fn to_the_crease(half: f32) -> Vec3 {
        let along = 1.0 - half * half / (2.0 * RADIUS * RADIUS);
        Vec3::new((1.0 - along * along).sqrt(), along, 0.0)
    }

    /// How far out of the crease between the fixture ball and a knob of
    /// `half` the surface is, along the line that halves the crease:
    /// nothing for a crease with nothing in it.
    fn fill(volume: &Volume, half: f32) -> f32 {
        let crease = CENTRE + to_the_crease(half) * RADIUS;
        let from_ball = to_the_crease(half);
        let from_knob = (crease - (CENTRE + Vec3::Y * RADIUS)).normalize();
        let between = (from_ball + from_knob).normalize();
        let mut along = -4.0;
        let mut last = volume.sample_world(crease + between * along);
        assert!(last < 0.0, "the walk out of the crease began outside the model");
        while along < 20.0 {
            along += 0.0625;
            let value = volume.sample_world(crease + between * along);
            if value >= 0.0 {
                return along - 0.0625 * value / (value - last);
            }
            last = value;
        }
        panic!("the crease has no surface over it");
    }

    /// Where the surface is along `towards` from the fixture ball's
    /// middle, in voxels from it.
    fn surface(volume: &Volume, towards: Vec3) -> f32 {
        let mut last = (5.0, volume.sample_world(CENTRE + towards * 5.0));
        assert!(last.1 < 0.0, "the walk began outside the model");
        let mut along = last.0;
        while along < 90.0 {
            along += 0.125;
            let value = volume.sample_world(CENTRE + towards * along);
            if value >= 0.0 {
                return last.0 + (along - last.0) * (last.1 / (last.1 - value));
            }
            last = (along, value);
        }
        panic!("no surface along {towards}");
    }

    /// Every voxel of the fixture's surroundings where two bodies differ.
    fn differing(one: &Volume, other: &Volume) -> Vec<IVec3> {
        let mut found = Vec::new();
        for z in 0..100 {
            for y in 0..110 {
                for x in 0..100 {
                    let voxel = IVec3::new(x, y, z);
                    if one.sample_voxel(voxel).to_bits() != other.sample_voxel(voxel).to_bits() {
                        found.push(voxel);
                    }
                }
            }
        }
        found
    }

    /// The fixture ball with `solid` joined to it.
    fn joined(solid: &dyn Solid, how: How) -> Volume {
        let mut volume = ball();
        let mut join = Join::new();
        volume.begin_stroke();
        join.add(&mut volume, solid, how);
        drop(volume.end_stroke());
        volume
    }

    /// A distance worked out further than a field keeps it is the
    /// distance: on a ball, whose distance has a formula, to 0.15 of a
    /// voxel as far as it was asked for, with the sign, and infinite
    /// where the surface is further off than that.
    #[test]
    fn a_widened_distance_is_the_distance() {
        let size = IVec3::splat(72);
        let middle = Vec3::splat(35.3);
        let radius = 17.4;
        let exact = |index: usize| {
            let voxel =
                Vec3::new((index % 72) as f32, ((index / 72) % 72) as f32, (index / 5184) as f32);
            voxel.distance(middle) - radius
        };
        let raw: Vec<f32> =
            (0..72 * 72 * 72).map(|index| exact(index).clamp(INSIDE, OUTSIDE)).collect();
        let most = 13.0;
        let wide = widen(&raw, size, SEED, (most, most), |_| true, &mut Spread::default());
        let mut worst = 0.0_f32;
        for (index, found) in wide.iter().enumerate() {
            let exact = exact(index);
            assert_eq!(found.is_sign_negative(), exact < 0.0, "the sign at {index}");
            if exact.abs() <= most {
                worst = worst.max((found - exact).abs());
            } else if exact.abs() > most + 3.0 {
                assert!(found.is_infinite(), "{exact} from the surface was given as {found}");
            }
        }
        assert!(worst < 0.15, "a widened distance was {worst} voxels out");
    }

    /// The wider the join, the more of the crease is filled, and the top
    /// of the knob and the far side of the ball are where they were at
    /// every width: a fillet, and not a blur.
    #[test]
    fn a_wide_join_fills_the_crease_and_nothing_else() {
        let half = 10.0;
        let knob = knob(half);
        let plain = joined(&knob, How::ADD);
        let mut last = fill(&plain, half);
        assert!(last.abs() < 0.5, "a plain join left the crease {last} full");
        for width in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0] {
            let soft = joined(&knob, How { cut: false, blend: width });
            let filled = fill(&soft, half);
            assert!(
                filled > last + width * 0.05,
                "a join of {width} filled the crease by {filled}, and the one before by {last}"
            );
            last = filled;
            assert!((surface(&soft, Vec3::Y) - surface(&plain, Vec3::Y)).abs() < 0.02);
            assert!((surface(&soft, -Vec3::Y) - surface(&plain, -Vec3::Y)).abs() < 0.02);
            // Only ever more than the plain join, and in the band.
            for voxel in differing(&plain, &soft) {
                let value = soft.sample_voxel(voxel);
                assert!((INSIDE..=OUTSIDE).contains(&value));
                assert!(value < plain.sample_voxel(voxel), "{voxel} lost material to a join");
            }
        }
        // The widest: a smooth minimum over 8 voxels fills a crease by
        // up to a quarter of that in field, which between two surfaces
        // at about a right angle is that times the root of two.
        assert!(last > 2.0 && last < 4.0, "the widest join filled the crease by {last}");
    }

    /// Where a narrow join ends and a wide one begins there is no step:
    /// a hair over [`NARROW`] is a hair more than [`NARROW`].
    #[test]
    fn a_wide_join_takes_up_where_a_narrow_one_leaves_off() {
        let half = 10.0;
        let narrow = fill(&joined(&knob(half), How { cut: false, blend: NARROW }), half);
        let wider = fill(&joined(&knob(half), How { cut: false, blend: NARROW + 0.05 }), half);
        assert!(
            (wider - narrow).abs() < 0.1,
            "the crease is {narrow} full at {NARROW} and {wider} just past it"
        );
    }

    /// A solid taken out with a wide join: the rim of the socket is
    /// rounded, and nothing is added anywhere.
    #[test]
    fn a_wide_join_rounds_the_rim_of_what_is_taken_out() {
        let half = 10.0;
        let plain = joined(&knob(half), How { cut: true, blend: 0.0 });
        let soft = joined(&knob(half), How { cut: true, blend: 6.0 });
        let crease = to_the_crease(half);
        let turn = crease.x.atan2(crease.y) + 1.0 / RADIUS;
        let beside = Vec3::new(turn.sin(), turn.cos(), 0.0);
        let (rim, rounded) = (surface(&plain, beside), surface(&soft, beside));
        assert!(rounded < rim - 0.4, "the rim was at {rim} and is at {rounded}");
        for voxel in differing(&plain, &soft) {
            assert!(soft.sample_voxel(voxel) > plain.sample_voxel(voxel));
        }
    }

    /// A wide join is part of the stroke: set again it is gone, and one
    /// undo takes the solid and its join together.
    #[test]
    fn a_wide_join_goes_with_what_it_joined() {
        let untouched = ball();
        let how = How { cut: false, blend: MOST_BLEND };
        let mut volume = ball();
        let mut join = Join::new();
        volume.begin_stroke();
        join.set(&mut volume, &[&knob(10.0)], how);
        join.clear(&mut volume);
        assert!(differing(&volume, &untouched).is_empty(), "a cleared join left something");
        join.set(&mut volume, &[&knob(12.0)], how);
        join.set(&mut volume, &[&knob(7.0)], how);
        assert!(
            differing(&volume, &joined(&knob(7.0), how)).is_empty(),
            "a shape set again was not the shape set once"
        );
        let edit = volume.end_stroke().expect("a stroke that joined something");
        volume.apply_edit(edit);
        assert!(differing(&volume, &untouched).is_empty(), "the undo left something");
    }

    /// The runs of a cable, each joined widely to the body as it was:
    /// the cable is one height along its length, with no rib where two
    /// runs overlap, and the crease beside it is filled evenly.
    ///
    /// A join wider than the cable is thick reaches over the top of it:
    /// a smooth minimum raises both surfaces wherever they are within
    /// its width of each other, by up to a quarter of the width. That is
    /// what a wide join IS on something thin, and it is the same along
    /// the whole cable.
    #[test]
    fn a_cable_joined_widely_has_no_ribs() {
        let mut volume = ball();
        let mut join = Join::new();
        volume.begin_stroke();
        let radius = 3.0;
        let how = How { cut: false, blend: 6.0 };
        // Round the ball's waist in the plane of X and Z, half sunk.
        let points: Vec<Vec3> = (0..=24)
            .map(|step| {
                let turn = step as f32 / 24.0 * std::f32::consts::PI;
                CENTRE + Vec3::new(turn.cos(), 0.0, turn.sin()) * RADIUS
            })
            .collect();
        for run in points.windows(2) {
            join.add(&mut volume, &Step { from: run[0], to: run[1], radius }, how);
        }
        drop(volume.end_stroke());
        let read = |lift: f32| {
            let heights: Vec<f32> = (20..160)
                .map(|degree| {
                    let turn = (degree as f32).to_radians();
                    surface(
                        &volume,
                        Vec3::new(turn.cos() * lift.cos(), lift.sin(), turn.sin() * lift.cos()),
                    )
                })
                .collect();
            heights.iter().fold((f32::MAX, f32::MIN), |(least, most), height| {
                (least.min(*height), most.max(*height))
            })
        };
        let (least, most) = read(0.0);
        assert!(
            least > RADIUS + radius - 0.1
                && most < RADIUS + radius + 6.0 / 4.0
                && most - least < 0.2,
            "the top of the cable is between {least} and {most}"
        );
        // Beside it, where the crease was: four voxels up the ball from
        // the cable's middle.
        let (least, most) = read(4.0 / RADIUS);
        assert!(least > RADIUS + 0.5, "the crease beside the cable was not filled: {least}");
        assert!(most - least < 0.3, "the crease beside the cable is between {least} and {most}");
    }

    /// Several solids set together are joined as they are one at a time:
    /// the wide part is worked out once for all of them, and it is the
    /// same join to within what working a distance out is good for.
    #[test]
    fn solids_set_together_are_joined_as_they_are_one_by_one() {
        let how = How { cut: false, blend: 5.0 };
        let knobs = [
            Placed::on(Form::Ball, CENTRE + Vec3::Y * RADIUS, Vec3::Y, Vec3::X, 7.0, 0.0)
                .expect("a knob"),
            Placed::on(Form::Box, CENTRE + Vec3::X * RADIUS, Vec3::X, Vec3::Y, 6.0, 0.3)
                .expect("a box"),
            Placed::on(Form::Cone, CENTRE - Vec3::Z * RADIUS, -Vec3::Z, Vec3::Y, 8.0, 0.0)
                .expect("a cone"),
        ];
        let solids: Vec<&dyn Solid> = knobs.iter().map(|knob| knob as &dyn Solid).collect();
        let mut together = ball();
        let mut join = Join::new();
        together.begin_stroke();
        join.set(&mut together, &solids, how);
        drop(together.end_stroke());
        let mut apart = ball();
        let mut join = Join::new();
        apart.begin_stroke();
        for solid in &solids {
            join.add(&mut apart, *solid, how);
        }
        drop(apart.end_stroke());
        let untouched = ball();
        assert!(differing(&apart, &untouched).len() > 1000);
        let worst = differing(&together, &apart)
            .into_iter()
            .map(|voxel| (together.sample_voxel(voxel) - apart.sample_voxel(voxel)).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 0.1, "set together and added one by one differ by {worst}");
    }

    /// A body lent as a shape keeps its own distance no further than its
    /// own field does, which made smaller is less than a voxel here. Its
    /// wide join is the join of the shape it is a copy of.
    #[test]
    fn a_lent_body_is_joined_as_widely_as_a_shape() {
        let source = super::super::body(
            1.0,
            &[&Placed::on(Form::Ball, Vec3::splat(40.0), Vec3::Y, Vec3::X, 20.0, 0.0)
                .expect("a ball")],
        );
        let extent = source.surface_bounds().expect("a body");
        let half = 10.0;
        // A ball of radius 20 fits a ball of radius 20 times the root of
        // three, and is lent to fit one of `half` times that.
        let lent = Lent::on(
            &source,
            extent,
            CENTRE + Vec3::Y * RADIUS,
            Vec3::Y,
            Vec3::X,
            half * 3.0_f32.sqrt(),
            0.0,
        )
        .expect("a body lent");
        assert!(lent.sure() < far(6.0), "the fixture is sure of its distance too far to test this");
        let how = How { cut: false, blend: 6.0 };
        let (as_lent, as_shape) = (joined(&lent, how), joined(&knob(half), how));
        let (lent_fill, shape_fill) = (fill(&as_lent, half), fill(&as_shape, half));
        assert!(
            (lent_fill - shape_fill).abs() < 0.5,
            "the crease is {lent_fill} full round a lent ball and {shape_fill} round a ball"
        );
        // And the ball away from the knob has not swollen, which is what
        // a join read from a clamp does.
        assert!((surface(&as_lent, Vec3::X) - RADIUS).abs() < 0.05);
        assert!((surface(&as_lent, -Vec3::Y) - RADIUS).abs() < 0.05);
    }

    /// [`widen`] as it was before it took its neighbours three at a time
    /// and stopped offering a point twice: one voxel, one neighbour, and a
    /// voxel offering again every time it comes round. What the one that
    /// is used has to give back, to the bit.
    fn widen_one_by_one(
        raw: &[f32],
        size: IVec3,
        seed: f32,
        most: (f32, f32),
        from: impl Fn(usize) -> bool,
    ) -> Vec<f32> {
        let count = raw.len();
        debug_assert_eq!(count as i64, size.as_i64vec3().element_product());
        let (row, plane) = (size.x as usize, (size.x * size.y) as usize);
        let at = |x: i32, y: i32, z: i32| x as usize + y as usize * row + z as usize * plane;
        let place = |index: usize| {
            IVec3::new((index % row) as i32, ((index % plane) / row) as i32, (index / plane) as i32)
        };
        // Where the surface is, by a voxel that is within `seed` of it:
        // itself, less its distance along its gradient.
        let surface_by = |index: usize| {
            let IVec3 { x, y, z } = place(index);
            let read = |x: i32, y: i32, z: i32| {
                raw[at(x.clamp(0, size.x - 1), y.clamp(0, size.y - 1), z.clamp(0, size.z - 1))]
            };
            let slope = Vec3::new(
                read(x + 1, y, z) - read(x - 1, y, z),
                read(x, y + 1, z) - read(x, y - 1, z),
                read(x, y, z + 1) - read(x, y, z - 1),
            );
            let normal = slope.try_normalize()?;
            Some((Vec3::new(x as f32, y as f32, z as f32) - normal * raw[index]).to_array())
        };

        // A front that moves out from the surface: each voxel on it offers
        // the point of the surface it holds to its neighbours, and a
        // neighbour that is nearer to that point than to the one it has
        // takes it and joins the front. The front is kept in order of
        // distance, by half voxels, so that a voxel has nearly always got
        // its nearest point by the time it offers one on: counted on the
        // knob of the cost test below, a front taken a round at a time
        // offered from twice as many voxels as it reached.
        let mut near = vec![f32::INFINITY; count];
        let mut point = vec![[0.0_f32; 3]; count];
        let order = |distance: f32| (distance * 2.0) as usize;
        let last = order(most.0.max(most.1) + 1.0) + 1;
        let mut fronts: Vec<Vec<u32>> = vec![Vec::new(); last + 1];
        for index in 0..count {
            if raw[index].abs() <= seed
                && from(index)
                && let Some(surface) = surface_by(index)
            {
                near[index] = raw[index].abs();
                point[index] = surface;
                fronts[order(near[index]).min(last)].push(index as u32);
            }
        }
        let mut beside = Vec::with_capacity(26);
        for dz in -1..=1_i32 {
            for dy in -1..=1_i32 {
                for dx in -1..=1_i32 {
                    if (dx, dy, dz) != (0, 0, 0) {
                        let step =
                            dx as isize + dy as isize * row as isize + dz as isize * plane as isize;
                        beside.push((dx, dy, dz, step));
                    }
                }
            }
        }
        for now in 0..=last {
            let mut taken = 0;
            while taken < fronts[now].len() {
                let index = fronts[now][taken] as usize;
                taken += 1;
                // Since it was put here it has been handed a nearer point,
                // and was put on an earlier front for that.
                if order(near[index]).min(last) < now {
                    continue;
                }
                let (x, y, z) =
                    ((index % row) as i32, ((index % plane) / row) as i32, (index / plane) as i32);
                let held = point[index];
                for &(dx, dy, dz, step) in &beside {
                    let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                    if nx < 0 || ny < 0 || nz < 0 || nx >= size.x || ny >= size.y || nz >= size.z {
                        continue;
                    }
                    let other = (index as isize + step) as usize;
                    let limit = if raw[other] < 0.0 { most.0 } else { most.1 };
                    let (ox, oy, oz) =
                        (nx as f32 - held[0], ny as f32 - held[1], nz as f32 - held[2]);
                    let away = (ox * ox + oy * oy + oz * oz).sqrt();
                    if away <= limit + 1.0 && away + 1.0e-4 < near[other] {
                        near[other] = away;
                        point[other] = held;
                        fronts[order(away).min(last).max(now)].push(other as u32);
                    }
                }
            }
        }

        for (distance, value) in near.iter_mut().zip(raw) {
            if *value < 0.0 {
                *distance = -*distance;
            }
        }
        near
    }

    /// A field over a box to widen: one or two balls and sometimes a flat
    /// floor, kept to the band as a body's is, from a seed.
    fn a_field(seed: u64, size: IVec3) -> Vec<f32> {
        let mut noise = crate::testing::Noise::seeded(seed);
        let mut unit = || noise.below(10_000) as f32 / 10_000.0;
        let extent = size.as_vec3();
        let balls: Vec<(Vec3, f32)> = (0..2)
            .map(|_| {
                let centre = Vec3::new(unit(), unit(), unit()) * extent;
                (centre, 1.0 + unit() * extent.min_element() * 0.6)
            })
            .collect();
        let floor = (unit() < 0.5).then(|| unit() * extent.z);
        let two = unit() < 0.6;
        let mut raw = Vec::with_capacity(size.element_product() as usize);
        for z in 0..size.z {
            for y in 0..size.y {
                for x in 0..size.x {
                    let at = Vec3::new(x as f32, y as f32, z as f32);
                    let mut distance = at.distance(balls[0].0) - balls[0].1;
                    if two {
                        distance = distance.min(at.distance(balls[1].0) - balls[1].1);
                    }
                    if let Some(floor) = floor {
                        distance = distance.min(at.z - floor);
                    }
                    raw.push(distance.clamp(INSIDE, OUTSIDE));
                }
            }
        }
        raw
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        /// Whatever the field, the box, how far it is spread and which of
        /// its surface it is spread from: three neighbours at a time and
        /// each point offered once, it is what it was one neighbour at a
        /// time, to the bit. Twice in the same working space, so that what
        /// one widening leaves there is seen not to reach the next.
        #[test]
        fn widening_is_what_it_was_one_neighbour_at_a_time(
            seed in any::<u64>(),
            size in (1..28_i32, 1..22_i32, 1..22_i32),
            most in (3.5_f32..12.0, 3.5_f32..12.0),
            near_seed in prop_oneof![Just(SEED), 0.4_f32..2.5],
            some in any::<bool>(),
        ) {
            let size = IVec3::new(size.0, size.1, size.2);
            let mut spread = Spread::default();
            for round in 0..2_u64 {
                let raw = a_field(seed.wrapping_add(round), size);
                // All of the surface, or all of it but a part.
                let from =
                    |index: usize| !some || !(index as u64 ^ seed ^ round).is_multiple_of(5);
                let one_by_one = widen_one_by_one(&raw, size, near_seed, most, from);
                let by_threes = widen(&raw, size, near_seed, most, from, &mut spread);
                prop_assert_eq!(one_by_one.len(), by_threes.len());
                for (index, (one, three)) in one_by_one.iter().zip(&by_threes).enumerate() {
                    prop_assert_eq!(
                        one.to_bits(),
                        three.to_bits(),
                        "voxel {} of a box of {}: {} one by one and {} by threes",
                        index, size, one, three
                    );
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// Whatever is joined, however widely: the field stays in its
        /// band, a join only adds (or, cutting, only takes), nothing
        /// changes further from the solid than a join of that width
        /// reaches, and clearing it leaves the ball as it was.
        #[test]
        fn a_wide_join_is_a_join(
            form in proptest::sample::select(Form::ALL.to_vec()),
            half in 3.0_f32..14.0,
            lift in -0.5_f32..1.0,
            width in 0.0_f32..10.0,
            cut in proptest::bool::ANY,
        ) {
            let solid = Placed::on(form, CENTRE + Vec3::Y * RADIUS, Vec3::Y, Vec3::X, half, lift)
                .expect("a shape");
            let how = How { cut, blend: width };
            let untouched = ball();
            let mut volume = ball();
            let mut join = Join::new();
            volume.begin_stroke();
            join.add(&mut volume, &solid, how);
            let reach = far(width.clamp(0.0, MOST_BLEND));
            for voxel in differing(&untouched, &volume) {
                let (value, before) = (volume.sample_voxel(voxel), untouched.sample_voxel(voxel));
                prop_assert!((INSIDE..=OUTSIDE).contains(&value));
                let more = if cut { value > before } else { value < before };
                prop_assert!(more, "a join went the wrong way at {voxel}");
                let off = solid.distance(voxel.as_vec3());
                prop_assert!(off < reach + 0.5, "{voxel} changed, {off} from the solid");
            }
            join.clear(&mut volume);
            prop_assert!(differing(&volume, &untouched).is_empty());
            drop(volume.end_stroke());
        }
    }
}
#[cfg(test)]
mod cost {
    use super::super::{Form, How, Placed, Step, hang};
    use super::*;

    /// What a wide join costs, printed: `cargo test --release -p
    /// brokkr-core --lib solid::wide::cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a test"]
    fn what_a_wide_join_costs() {
        let centre = Vec3::splat(48.0);
        let ball = || {
            let mut volume = Volume::new(1.0);
            volume.seed_sphere(centre, 30.0);
            volume
        };
        for half in [10.0_f32, 25.0] {
            let knob = Placed::on(Form::Ball, centre + Vec3::Y * 30.0, Vec3::Y, Vec3::X, half, 0.0)
                .expect("a knob");
            for width in [0.0_f32, 2.0, 3.0, 4.0, 6.0, 8.0] {
                let mut volume = ball();
                let mut join = Join::new();
                volume.begin_stroke();
                let started = web_time::Instant::now();
                join.add(&mut volume, &knob, How { cut: false, blend: width });
                println!("a knob of {half} joined over {width}: {:?}", started.elapsed());
            }
        }
        let points = hang(
            centre + Vec3::new(-20.0, 26.0, 0.0),
            centre + Vec3::new(20.0, 26.0, 0.0),
            0.15,
            -Vec3::Y,
            24,
        );
        for width in [0.0_f32, 2.0, 4.0, 8.0] {
            let runs: Vec<Step> = points
                .windows(2)
                .map(|run| Step { from: run[0], to: run[1], radius: 2.0 })
                .collect();
            let solids: Vec<&dyn Solid> = runs.iter().map(|run| run as &dyn Solid).collect();
            let how = How { cut: false, blend: width };
            let mut volume = ball();
            let mut join = Join::new();
            volume.begin_stroke();
            let started = web_time::Instant::now();
            for run in &runs {
                join.add(&mut volume, run, how);
            }
            let each = started.elapsed();
            let started = web_time::Instant::now();
            join.set(&mut volume, &solids, how);
            println!(
                "a hung cable of 24 runs joined over {width}: {each:?} a run at a time, {:?} together",
                started.elapsed()
            );
        }
    }
}
