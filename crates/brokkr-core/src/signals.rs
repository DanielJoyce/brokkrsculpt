// SPDX-License-Identifier: AGPL-3.0-only

//! Signals read off the geometry: where the creases are, and where the edges.
//!
//! A wash runs into recesses and off edges, a drybrush catches edges and
//! nothing else, and a sculptor judging a form wants the creases drawn a
//! little darker than the light alone draws them. All three want one number
//! per point of the surface: how concave it is there.
//!
//! [`crate::generate`] already computes that number, for one purpose, into a
//! mask. This is the same measurement made available per mesh vertex, so the
//! viewport can draw it and so a paint operation previewed in the shader and
//! applied on the CPU have one definition to agree on.
//!
//! # The measurement
//!
//! For a true distance field the Laplacian is the sum of the principal
//! curvatures, and dividing by the gradient's length keeps that true where the
//! brushes have let the gradient drift from one. [`crate::generate`] argues
//! both at length and they are not repeated here.
//!
//! What is different is WHERE it is taken. A mask wants the value at a voxel.
//! A mesh vertex is not at a voxel: surface nets puts it somewhere inside a
//! lattice cell. So this is the curvature at the centre of the vertex's cell,
//! which is the mean of the stencil over the cell's eight corners and comes
//! out as a compact form over the 4x4x4 block of voxels round the cell.
//!
//! # Why it is bit for bit the same on both sides of a brick seam
//!
//! A vertex on a seam is emitted twice, once by each brick, and both have to
//! shade it identically or the seam is drawn. The cell is the identity that
//! makes that possible: both bricks derive it from the same world coordinate
//! ([`BrickMesh::cells`]), so both evaluate the same expression over the same
//! 32 stored values in the same order. Where the block overhangs a brick's own
//! apron, by one voxel on the cells along its faces, the missing samples are
//! read from the volume, which is where the apron got the rest.
//!
//! The obvious cheaper version does not have the property. A seven point
//! stencil at one corner of the cell needs a neighbour on each side of that
//! corner, and for a cell on the low face of a brick's apron the one corner
//! with all its neighbours in the apron is not the corner the brick across the
//! seam would pick. Each brick would measure a voxel apart from the other.
//!
//! # Not free, so not on unless asked for
//!
//! 32 reads and a square root per vertex. That is small beside surface nets on
//! one brick and not small across eleven million vertices, so nothing computes
//! it unless a caller asks: [`Document::mesh_dirty_with_cavity`] sits beside
//! [`Document::mesh_dirty`] and the application chooses between them by
//! whether anything on screen is using the result. A mesh made without it
//! carries an empty [`BrickMesh::cavity`], which every reader takes as flat.

use glam::{IVec3, Vec3};
use rayon::prelude::*;

use crate::apron::ApronBuffer;
use crate::body::{Document, NodeId};
use crate::brick::{APRON_DIM, BrickCoord, NARROW_BAND};
use crate::mesh::{BrickMesh, MeshScratch};
use crate::volume::{PARALLEL_MESH_THRESHOLD, Volume};

/// The largest curvature the byte can carry, in 1/voxel.
///
/// A sphere of radius `R` voxels reads `2 / R`, so this is a feature two
/// voxels across, which is the finest thing the lattice can be said to hold
/// at all ([`crate::generate`] draws the same line for its slider).
pub const CAVITY_RANGE: f32 = 2.0;

/// The byte that means flat.
pub const CAVITY_FLAT: u8 = 128;

/// The byte that means "not measured", which is what a mesh made without the
/// signal uploads. Never produced by [`encode_cavity`].
pub const CAVITY_ABSENT: u8 = 0;

/// The gradient lengths the normalisation will believe. [`crate::generate`]'s
/// own, for its own reasons: a plateau would turn rounding into curvature.
const GRADIENT_FLOOR: f32 = 0.5;
const GRADIENT_CEILING: f32 = 2.0;

/// Curvature as a byte: 128 flat, above it concave, below it convex.
///
/// **A square root curve, not a straight line.** Most of a real surface is
/// nearly flat, and the differences worth drawing there are small: a 2 mm
/// fold on a 0.1 mm voxel reads 0.2 of a range of 2. On a straight line that
/// is thirteen levels from flat to full. On this curve it is forty.
///
/// Never returns [`CAVITY_ABSENT`]: the most convex value encodes as 1.
#[inline]
pub fn encode_cavity(cavity: f32) -> u8 {
    let scaled = (cavity.abs() / CAVITY_RANGE).min(1.0).sqrt() * 127.0;
    let signed = if cavity < 0.0 { -scaled } else { scaled };
    (128.0 + signed).round().clamp(1.0, 255.0) as u8
}

/// The inverse of [`encode_cavity`], in 1/voxel. Positive is concave.
///
/// [`CAVITY_ABSENT`] decodes as flat, which is what makes a mesh without the
/// signal draw exactly as it did before the signal existed.
///
/// `sculpt.wgsl` carries the same arithmetic, and
/// `the_shader_decodes_cavity_the_way_this_does` in `brokkr-gpu` reads it out
/// of the shader source to hold the two together.
#[inline]
pub fn decode_cavity(byte: u8) -> f32 {
    if byte == CAVITY_ABSENT {
        return 0.0;
    }
    let t = (f32::from(byte) - 128.0) / 127.0;
    CAVITY_RANGE * t * t.abs()
}

/// Curvature at the centre of a lattice cell, in 1/voxel, positive where the
/// surface is concave.
///
/// `sample` takes an offset from the cell's base corner, each component in
/// `-1..=2`, and returns the stored distance there. `None` when any of the 32
/// samples it reads is saturated or not finite: a stencil with a foot on the
/// edge of the band measures the band.
///
/// The order of every sum below is fixed and is part of the contract. Two
/// bricks evaluating this for one cell must get the same bits.
#[inline]
pub(crate) fn cell_cavity(sample: impl Fn(i32, i32, i32) -> f32) -> Option<f32> {
    let mut laplacian = 0.0;
    let mut gradient = Vec3::ZERO;
    let mut measured = true;
    let mut read = |x: i32, y: i32, z: i32| {
        let value = sample(x, y, z);
        measured &= value.is_finite() && value.abs() < NARROW_BAND;
        value
    };

    // Along each axis in turn, over the four lattice lines of the cell that
    // run that way. On one line the mean of the second differences at its two
    // corners is `(s[-1] - s[0] - s[1] + s[2]) / 2`, and the first difference
    // between them is the derivative at the cell's centre exactly.
    for a in 0..2 {
        for b in 0..2 {
            let (x0, x1) = (read(0, a, b), read(1, a, b));
            laplacian += read(-1, a, b) - x0 - x1 + read(2, a, b);
            gradient.x += x1 - x0;

            let (y0, y1) = (read(a, 0, b), read(a, 1, b));
            laplacian += read(a, -1, b) - y0 - y1 + read(a, 2, b);
            gradient.y += y1 - y0;

            let (z0, z1) = (read(a, b, 0), read(a, b, 1));
            laplacian += read(a, b, -1) - z0 - z1 + read(a, b, 2);
            gradient.z += z1 - z0;
        }
    }
    if !measured {
        return None;
    }

    // Four lines an axis, and the half from the mean of two second
    // differences.
    let laplacian = laplacian / 8.0;
    let slope = (gradient / 4.0).length().clamp(GRADIENT_FLOOR, GRADIENT_CEILING);
    // The field is negative inside, so the Laplacian is positive on a convex
    // surface. A cavity is the other sign.
    Some(-laplacian / slope)
}

impl Volume {
    /// Curvature at the centre of the cell whose base corner is the world
    /// voxel `cell`, read straight from the volume.
    ///
    /// The reference [`Volume::cavity_at_vertices`] is held to. It is a hash
    /// lookup per sample and is not for a mesh's worth of vertices.
    pub fn cavity_at_cell(&self, cell: IVec3) -> Option<f32> {
        cell_cavity(|x, y, z| self.sample_voxel(cell + IVec3::new(x, y, z)))
    }

    /// Fill `out.cavity` with one byte per vertex of a brick just meshed.
    ///
    /// `apron` must be the apron [`Volume::mesh_brick`] gathered for `coord`,
    /// which is what [`MeshScratch::apron`] holds straight after it returns a
    /// mesh that is not empty. An empty mesh gets an empty `cavity` and the
    /// apron is not looked at.
    ///
    /// A vertex surface nets made reads its own cell. A midpoint that
    /// [`crate::mesh`] added on a paint boundary sits between two cells and
    /// takes the mean of their bytes, which is the rule its mask byte follows.
    pub fn cavity_at_vertices(&self, coord: BrickCoord, apron: &ApronBuffer, out: &mut BrickMesh) {
        out.cavity.clear();
        if out.vertices.is_empty() {
            return;
        }
        debug_assert_eq!(apron.coord(), Some(coord), "the apron is some other brick's");

        // Apron local `l` is world voxel `l + origin - 1`.
        let apron_origin = coord.origin() - IVec3::ONE;
        let last = APRON_DIM as i32 - 1;
        let byte_at = |cell: IVec3| {
            let local = cell - apron_origin;
            // The block spans local - 1 through local + 2 on each axis.
            let inside = local.cmpge(IVec3::ONE).all() && local.cmple(IVec3::splat(last - 2)).all();
            let cavity = if inside {
                cell_cavity(|x, y, z| {
                    apron.get(
                        (local.x + x) as usize,
                        (local.y + y) as usize,
                        (local.z + z) as usize,
                    )
                })
            } else {
                // A cell along a face of the apron. The same expression over
                // the same stored values, so the brick across the seam, for
                // which this cell may be interior, gets the same bits.
                self.cavity_at_cell(cell)
            };
            cavity.map_or(CAVITY_FLAT, encode_cavity)
        };

        out.cavity.reserve(out.vertices.len());
        for (cell, partner) in out.cells.iter().zip(out.partners.iter()) {
            let here = byte_at(*cell);
            let byte = if partner == cell {
                here
            } else {
                ((u16::from(here) + u16::from(byte_at(*partner))) / 2) as u8
            };
            out.cavity.push(byte);
        }
    }
}

impl Document {
    /// [`Document::mesh_dirty`], with each mesh also carrying its cavity
    /// bytes.
    ///
    /// Beside that function and not a flag on it, so that a caller who does
    /// not want the signal runs exactly the code it ran before the signal
    /// existed. The batching rule is that function's and is repeated here for
    /// its reason: the choice between one core and all of them is made once,
    /// over the whole batch.
    pub fn mesh_dirty_with_cavity(&self, work: &[(NodeId, BrickCoord)], out: &mut [BrickMesh]) {
        assert_eq!(work.len(), out.len(), "one output mesh per dirty brick");

        let one =
            |body: NodeId, coord: BrickCoord, scratch: &mut MeshScratch, mesh: &mut BrickMesh| {
                match self.volume(body) {
                    Some(volume) => {
                        volume.mesh_brick(coord, scratch, mesh);
                        volume.cavity_at_vertices(coord, scratch.apron(), mesh);
                    }
                    None => mesh.clear(),
                }
            };

        if work.len() < PARALLEL_MESH_THRESHOLD {
            let mut scratch = MeshScratch::new();
            for ((body, coord), mesh) in work.iter().zip(out.iter_mut()) {
                one(*body, *coord, &mut scratch, mesh);
            }
            return;
        }

        out.par_iter_mut()
            .zip(work.par_iter())
            .for_each_init(MeshScratch::new, |scratch, (mesh, (body, coord))| {
                one(*body, *coord, scratch, mesh)
            });
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashMap;

    use super::*;
    use crate::brick::{INSIDE, OUTSIDE};
    use crate::brush::{Brush, BrushDirection, BrushKind, BrushScratch, Stamp};

    fn ball(radius: f32) -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::splat(48.0), radius);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// A solid block with a spherical void in the middle of it: the same
    /// surface as [`ball`], facing the other way.
    fn void(radius: f32) -> Volume {
        let mut volume = Volume::new(1.0);
        let centre = Vec3::splat(48.0);
        volume.edit_box(Vec3::splat(8.0), Vec3::splat(88.0), |position, _| {
            (radius - position.distance(centre)).clamp(INSIDE, OUTSIDE)
        });
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// Mesh every brick that could carry surface, with cavity, keyed by brick.
    fn meshed(volume: &Volume) -> Vec<(BrickCoord, BrickMesh)> {
        let mut coords: FxHashMap<BrickCoord, ()> = FxHashMap::default();
        for coord in volume.brick_coords() {
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        coords.insert(BrickCoord(coord.0 + IVec3::new(dx, dy, dz)), ());
                    }
                }
            }
        }
        let mut coords: Vec<BrickCoord> = coords.into_keys().collect();
        coords.sort();
        let mut scratch = MeshScratch::new();
        coords
            .into_iter()
            .filter_map(|coord| {
                let mut mesh = BrickMesh::default();
                volume.mesh_brick(coord, &mut scratch, &mut mesh);
                volume.cavity_at_vertices(coord, scratch.apron(), &mut mesh);
                (!mesh.is_empty()).then_some((coord, mesh))
            })
            .collect()
    }

    /// Mean decoded cavity over the vertices within `reach` of the fixtures'
    /// centre. The reach is what keeps [`void`]'s outer faces, where the
    /// block simply stops, out of a mean that is about its inner one.
    fn mean_cavity(meshes: &[(BrickCoord, BrickMesh)], reach: f32) -> (f32, usize) {
        let mut sum = 0.0;
        let mut count = 0;
        for (_, mesh) in meshes {
            assert_eq!(mesh.cavity.len(), mesh.vertices.len(), "one cavity byte per vertex");
            for (vertex, byte) in mesh.vertices.iter().zip(&mesh.cavity) {
                if Vec3::from_array(vertex.position).distance(Vec3::splat(48.0)) < reach {
                    sum += decode_cavity(*byte);
                    count += 1;
                }
            }
        }
        (sum / count as f32, count)
    }

    #[test]
    fn the_byte_round_trips_and_keeps_its_sign() {
        assert_eq!(encode_cavity(0.0), CAVITY_FLAT);
        assert_eq!(decode_cavity(CAVITY_FLAT), 0.0);
        assert_eq!(decode_cavity(CAVITY_ABSENT), 0.0, "an unmeasured vertex must draw as flat");
        let mut last = 0u8;
        for step in -40..=40 {
            let cavity = step as f32 * 0.06;
            let byte = encode_cavity(cavity);
            assert_ne!(byte, CAVITY_ABSENT, "{cavity} encoded as the byte that means absent");
            assert!(byte >= last, "the encoding is not monotonic at {cavity}");
            last = byte;
            let back = decode_cavity(byte);
            let clamped = cavity.clamp(-CAVITY_RANGE, CAVITY_RANGE);
            // The quantisation step grows with the value: fine near flat,
            // where it matters, and coarse at the ends.
            let step_here = 2.0 * CAVITY_RANGE * (clamped.abs() / CAVITY_RANGE).sqrt() / 127.0;
            assert!((back - clamped).abs() <= step_here + 1.0e-3, "{cavity} came back as {back}");
        }
        // Past the range it saturates and does not wrap.
        assert_eq!(encode_cavity(9.0), 255);
        assert_eq!(encode_cavity(-9.0), 1);
    }

    #[test]
    fn a_ball_reads_convex_and_a_void_reads_concave_by_two_over_the_radius() {
        // The two signs are one character apart and both produce a plausible
        // picture, which is why `generate` tests each and why this does.
        for radius in [8.0_f32, 16.0, 30.0] {
            let expected = 2.0 / radius;

            let (convex, count) = mean_cavity(&meshed(&ball(radius)), radius + 3.0);
            assert!(count > 500, "a ball of radius {radius} meshed to {count} vertices");
            assert!(
                (convex + expected).abs() < 0.2 * expected + 0.004,
                "a ball of radius {radius} read {convex}, expected {}",
                -expected
            );

            let (concave, hollow) = mean_cavity(&meshed(&void(radius)), radius + 3.0);
            assert!(hollow > 500, "a void of radius {radius} meshed to {hollow} vertices");
            assert!(
                (concave - expected).abs() < 0.2 * expected + 0.004,
                "a void of radius {radius} read {concave}, expected {expected}"
            );
        }
    }

    #[test]
    fn a_flat_face_reads_flat() {
        let mut volume = Volume::new(1.0);
        let normal = Vec3::new(0.3, 1.0, -0.45).normalize();
        volume.edit_box(Vec3::splat(8.0), Vec3::splat(88.0), |position, _| {
            ((position - Vec3::splat(48.3)).dot(normal)).clamp(INSIDE, OUTSIDE)
        });
        // Only the middle of the slab: its rim, where the box ends, is an
        // edge of the fixture and not a flat face.
        let mut seen = 0;
        for (_, mesh) in meshed(&volume) {
            for (vertex, byte) in mesh.vertices.iter().zip(&mesh.cavity) {
                let at = Vec3::from_array(vertex.position);
                if (at - Vec3::splat(48.0)).abs().max_element() < 24.0 {
                    seen += 1;
                    assert!(
                        (i16::from(*byte) - i16::from(CAVITY_FLAT)).abs() <= 1,
                        "a flat face read byte {byte} at {at}"
                    );
                }
            }
        }
        assert!(seen > 500, "only {seen} vertices of the face were looked at");
    }

    #[test]
    fn both_sides_of_a_brick_seam_read_the_same_byte() {
        // A sculpted ball, so the values along the seams are not the tidy
        // ones a seeded sphere has. Every vertex that two bricks both emit
        // must carry one byte, or the seam is drawn.
        let mut volume = ball(30.0);
        let brush =
            Brush { kind: BrushKind::Draw, radius: 12.0, strength: 0.8, ..Brush::default() };
        let mut scratch = BrushScratch::new();
        for towards in [Vec3::X, Vec3::new(0.5, 0.6, 0.62), Vec3::new(-0.3, 0.2, 0.93)] {
            let towards = towards.normalize();
            let stamp =
                Stamp::new(Vec3::splat(48.0) + towards * 30.0, towards, BrushDirection::Add);
            for _ in 0..4 {
                brush.apply(&mut volume, &stamp, &mut scratch);
            }
        }

        let mut seen: FxHashMap<(IVec3, IVec3), (u8, BrickCoord)> = FxHashMap::default();
        let mut shared = 0;
        let mut distinct: FxHashMap<u8, ()> = FxHashMap::default();
        for (coord, mesh) in meshed(&volume) {
            for index in 0..mesh.vertices.len() {
                let byte = mesh.cavity[index];
                distinct.insert(byte, ());
                if let Some((other, from)) = seen.insert(mesh.weld_key(index), (byte, coord)) {
                    shared += 1;
                    assert_eq!(
                        other, byte,
                        "a vertex shared by {from:?} and {coord:?} reads {other} in one and \
                         {byte} in the other"
                    );
                }
            }
        }
        // The controls: there were seams to disagree across, and the signal
        // varies, so agreeing was not the agreement of two constants.
        assert!(shared > 200, "only {shared} vertices were shared between bricks");
        assert!(distinct.len() > 20, "the sculpted ball reads only {} values", distinct.len());
    }

    #[test]
    fn the_apron_path_and_the_volume_path_agree() {
        // `cavity_at_vertices` takes the fast path through the apron for
        // every cell it can. That must be the same number the slow path
        // gives, at every vertex, because the slow path is what the brick
        // across a seam may be taking for the same cell.
        let volume = ball(22.0);
        let mut checked = 0;
        for (_, mesh) in meshed(&volume) {
            for (cell, byte) in mesh.cells.iter().zip(&mesh.cavity) {
                let reference = volume.cavity_at_cell(*cell).map_or(CAVITY_FLAT, encode_cavity);
                assert_eq!(*byte, reference, "cell {cell:?}");
                checked += 1;
            }
        }
        assert!(checked > 1000);
    }

    #[test]
    fn a_stencil_with_a_foot_on_the_band_edge_is_not_believed() {
        // A wall two voxels thick. Its two faces are inside each other's
        // stencils and the field saturates between samples the block reads,
        // so the honest answer is "not measured", which draws as flat.
        assert_eq!(cell_cavity(|x, _, _| if x < 2 { 0.5 } else { OUTSIDE }), None);
        assert_eq!(cell_cavity(|_, _, _| f32::NAN), None);
        // And the control: a clean linear field is measured, and is flat.
        let flat = cell_cavity(|x, y, z| 0.3 + 0.5 * x as f32 - 0.2 * y as f32 + 0.1 * z as f32);
        assert!(flat.is_some_and(|value| value.abs() < 1.0e-5), "{flat:?}");
    }

    #[test]
    fn meshing_with_cavity_changes_nothing_but_the_cavity() {
        // The whole promise of putting this beside `mesh_dirty` instead of
        // inside it.
        let doc = Document::from_volume(ball(26.0));
        let body = doc.active();
        let mut coords: Vec<BrickCoord> = doc.active_volume().brick_coords().collect();
        coords.sort();
        let work: Vec<(NodeId, BrickCoord)> = coords.iter().map(|coord| (body, *coord)).collect();

        let mut plain = vec![BrickMesh::default(); work.len()];
        let mut with = vec![BrickMesh::default(); work.len()];
        doc.mesh_dirty(&work, &mut plain);
        doc.mesh_dirty_with_cavity(&work, &mut with);

        let mut vertices = 0;
        for (a, b) in plain.iter().zip(&with) {
            assert_eq!(a.vertices, b.vertices);
            assert_eq!(a.indices, b.indices);
            assert_eq!(a.cells, b.cells);
            assert_eq!(a.mask, b.mask);
            assert_eq!(a.colour, b.colour);
            assert!(a.cavity.is_empty(), "the plain path grew a cavity");
            assert_eq!(b.cavity.len(), b.vertices.len());
            vertices += b.vertices.len();
        }
        assert!(vertices > 1000, "the fixture meshed to {vertices} vertices");

        // A buffer reused by the plain path after the other must not keep the
        // last tenant's bytes: the pool would upload them.
        doc.mesh_dirty(&work, &mut with);
        assert!(with.iter().all(|mesh| mesh.cavity.is_empty()), "a reused mesh kept stale cavity");
    }
}
