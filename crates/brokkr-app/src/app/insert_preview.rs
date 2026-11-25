// SPDX-License-Identifier: AGPL-3.0-only

//! What a shape being dragged out looks like before it is set down: a
//! coarse mesh of it, drawn over the model, and not the shape joined into
//! the field at every move.
//!
//! The maintainer asked whether a gesture could be tried against something
//! coarser and applied to the real thing. For an insert it can: a move of
//! the drag used to put the body back as it was and join the shape again,
//! fifty milliseconds a move for a ball 80 voxels across, measured by
//! `what_a_move_of_an_insert_costs` in release on the build machine, and
//! then the meshing of every brick it touched. Now a move samples the
//! shape's own distance on a grid of [`CELLS`] a side and meshes that,
//! which is a few thousand samples, and the join is done once, when the
//! button comes up. What the preview does not show is the soft join to
//! the body, which only the real join makes.

use brokkr_core::solid::Solid;
use glam::{IVec3, Vec3};

/// How many cells the preview's grid is across its longest side.
pub(super) const CELLS: usize = 40;

/// A coarse mesh of the union of `solids`, as triangles: surface nets on a
/// grid over the box that holds them.
pub(super) fn coarse_mesh(solids: &[&dyn Solid]) -> Vec<[Vec3; 3]> {
    let Some((low, high)) =
        solids.iter().map(|solid| solid.bounds()).reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    else {
        return Vec::new();
    };
    let span = (high - low).max_element();
    if !(span.is_finite() && span > 0.0) {
        return Vec::new();
    }
    let step = span / CELLS as f32;
    let low = low - Vec3::splat(step);
    let counts = (((high - low) / step).ceil().as_ivec3() + IVec3::splat(2)).max(IVec3::ONE);
    let (nx, ny, nz) = (counts.x as usize, counts.y as usize, counts.z as usize);
    let at = |x: usize, y: usize, z: usize| low + Vec3::new(x as f32, y as f32, z as f32) * step;
    let index = |x: usize, y: usize, z: usize| (z * ny + y) * nx + x;
    let mut values = vec![0.0_f32; nx * ny * nz];
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let point = at(x, y, z);
                values[index(x, y, z)] =
                    solids.iter().map(|solid| solid.distance(point)).fold(f32::MAX, f32::min);
            }
        }
    }
    let inside = |x: usize, y: usize, z: usize| values[index(x, y, z)] < 0.0;

    // A vertex in each cell the surface passes through: the mean of where
    // it crosses the cell's edges.
    let mut vertex: Vec<Option<Vec3>> = vec![None; nx * ny * nz];
    const EDGES: [([usize; 3], [usize; 3]); 12] = [
        ([0, 0, 0], [1, 0, 0]),
        ([0, 1, 0], [1, 1, 0]),
        ([0, 0, 1], [1, 0, 1]),
        ([0, 1, 1], [1, 1, 1]),
        ([0, 0, 0], [0, 1, 0]),
        ([1, 0, 0], [1, 1, 0]),
        ([0, 0, 1], [0, 1, 1]),
        ([1, 0, 1], [1, 1, 1]),
        ([0, 0, 0], [0, 0, 1]),
        ([1, 0, 0], [1, 0, 1]),
        ([0, 1, 0], [0, 1, 1]),
        ([1, 1, 0], [1, 1, 1]),
    ];
    for z in 0..nz - 1 {
        for y in 0..ny - 1 {
            for x in 0..nx - 1 {
                let mut sum = Vec3::ZERO;
                let mut crossings = 0;
                for (a, b) in EDGES {
                    let (pa, pb) = ((x + a[0], y + a[1], z + a[2]), (x + b[0], y + b[1], z + b[2]));
                    let (va, vb) =
                        (values[index(pa.0, pa.1, pa.2)], values[index(pb.0, pb.1, pb.2)]);
                    if (va < 0.0) != (vb < 0.0) {
                        let t = (va / (va - vb)).clamp(0.0, 1.0);
                        sum += at(pa.0, pa.1, pa.2).lerp(at(pb.0, pb.1, pb.2), t);
                        crossings += 1;
                    }
                }
                if crossings > 0 {
                    vertex[index(x, y, z)] = Some(sum / crossings as f32);
                }
            }
        }
    }

    // A quad across each grid edge the surface crosses, between the four
    // cells round it.
    let mut triangles = Vec::new();
    let mut quad = |cells: [(usize, usize, usize); 4]| {
        let corners: Option<Vec<Vec3>> =
            cells.iter().map(|(x, y, z)| vertex[index(*x, *y, *z)]).collect();
        if let Some(c) = corners {
            triangles.push([c[0], c[1], c[2]]);
            triangles.push([c[0], c[2], c[3]]);
        }
    };
    for z in 1..nz - 1 {
        for y in 1..ny - 1 {
            for x in 1..nx - 1 {
                let here = inside(x, y, z);
                if x + 1 < nx && here != inside(x + 1, y, z) {
                    quad([(x, y - 1, z - 1), (x, y, z - 1), (x, y, z), (x, y - 1, z)]);
                }
                if y + 1 < ny && here != inside(x, y + 1, z) {
                    quad([(x - 1, y, z - 1), (x, y, z - 1), (x, y, z), (x - 1, y, z)]);
                }
                if z + 1 < nz && here != inside(x, y, z + 1) {
                    quad([(x - 1, y - 1, z), (x, y - 1, z), (x, y, z), (x - 1, y, z)]);
                }
            }
        }
    }
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;
    use brokkr_core::solid::{Form, Placed};

    /// A ball's preview is a closed shell about the ball: every corner of
    /// every triangle is within a cell of its surface, and it has enough of
    /// them to be a ball and not a box.
    #[test]
    fn a_balls_preview_lies_on_the_ball() {
        let ball = Placed::on(Form::Ball, Vec3::ZERO, Vec3::Y, Vec3::X, 10.0, 0.0).expect("a ball");
        let triangles = coarse_mesh(&[&ball]);
        assert!(triangles.len() > 1000, "only {} triangles", triangles.len());
        let cell = 20.0 / CELLS as f32;
        for corner in triangles.iter().flatten() {
            assert!(ball.distance(*corner).abs() < cell, "{corner} is off the ball");
        }
    }

    #[test]
    fn nothing_is_no_mesh() {
        assert!(coarse_mesh(&[]).is_empty());
    }
}
