// SPDX-License-Identifier: AGPL-3.0-only

//! Fewer triangles, within a stated distance of the sculpt.
//!
//! An export is one vertex per lattice cell the surface passes through. That
//! is the density the sculpt was edited at and far more than describing it
//! takes: a flat cut face is meshed as finely as an ear, and a ball 60 mm
//! across at a 0.055 mm voxel is eleven million triangles of which a slicer
//! needs a small fraction. This collapses edges until no more can go without
//! the surface leaving a tolerance, and the tolerance is in millimetres.
//!
//! # The field is the judge, and that is what a mesh tool does not have
//!
//! A decimator working on a mesh alone has nothing to measure against but the
//! mesh it is taking apart, so it accumulates an estimate (the quadric error
//! of Garland and Heckbert) and trusts it. Here the sculpt is still in hand as
//! a distance field, and "how far is this point from the surface" is one
//! trilinear read. So the estimate is used only for what an estimate is good
//! at, choosing where the merged vertex should go, and every collapse is then
//! accepted or refused by asking the field directly:
//!
//! * the new vertex, and the middle and edges of every triangle the collapse
//!   reshapes, must read within the tolerance of zero;
//! * each of those triangles must still face the way the field's gradient
//!   says the surface faces there.
//!
//! The second is not decoration. Distance alone cannot see a thin feature
//! being cut off: a triangle spanning the base of a narrow spike lies inside
//! the spike's own solid, a fraction of the spike's width from its wall, and
//! reads "close to the surface" however tall the spike is. But the gradient
//! there points sideways at that wall, not out through the triangle, and a
//! triangle that disagrees with the gradient by more than sixty degrees is
//! refused. `a_spike_is_not_sliced_off_at_its_base` is that case.
//!
//! The distance is the field's value over its gradient's length, for the
//! reason [`crate::generate`] gives: the brushes let the gradient drift from
//! one, and the raw value then under-reads.
//!
//! # Both directions, and the field can only judge one
//!
//! Everything above asks whether the new surface is near the sculpt. It
//! cannot ask whether the sculpt is still near the new surface, and the
//! difference is a corner. Slide the vertex at the corner of a cube along one
//! of its edges: it is still on the cube, every triangle it touches still
//! reads within a fraction of a voxel of a face, every normal still agrees,
//! and the tip of the corner is gone. The first version of this module did
//! exactly that. It took a cube to twelve triangles at a tolerance of 0.3 of
//! a voxel with one corner 0.84 from where it belonged, and the test that
//! caught it was the one written to celebrate the twelve.
//!
//! So each collapse is also held the other way round. Every vertex starts on
//! the sculpt and is a point of it; a collapse may go ahead only if the two
//! vertices it merges, where they stand, lie within the tolerance of the
//! triangles that replace them. And so that a tip cannot be walked back a
//! tolerance at a time, each vertex keeps a few WITNESSES: the points of the
//! sculpt that the collapses merged into it have left furthest out. The
//! witnesses of both ends are measured too, directly, against what replaces
//! the edge, and the furthest few are the merged vertex's witnesses from then
//! on. A collapse reshapes triangles that belong to its neighbours as well,
//! so the witnesses of the vertices round the edge are measured against
//! their own vertex's triangles as they will be.
//!
//! Five things were tried on the way to that and are worth naming, because
//! each looked finished.
//!
//! * **No reverse check.** The cube with a corner missing, as above.
//! * **The loss ADDED along the chain of merges**, which is the provable
//!   bound. The triangle inequality is charged at every step and the result
//!   was pessimistic by a factor of two that grew as the voxel shrank:
//!   28,408, 35,816 and 45,448 triangles for one ball at three voxel sizes,
//!   where the shape is the same. A witness is measured, not summed.
//! * **Witnesses, with the mesh left where the mesher put it.** Surface nets
//!   rounds a corner off, so the corner vertex was not on the sculpt to begin
//!   with, and every measurement taken from it was of that rounding. See
//!   `settle_onto_the_field`.
//! * **A triangle sampled at its middle and the middles of its edges.** The
//!   same thing as sampling it properly when it is a voxel across, and not
//!   when it is twenty: on a roughened ball the result passed over bumps and
//!   the sculpt ended 29% further from it than the tolerance. See
//!   `SAMPLE_SPACING_VOXELS`.
//! * **Taking the reverse check out again.** With the two fixes above in, it
//!   made no measurable difference on a cube, a ball or a roughened ball:
//!   the forward check done properly was holding both directions, and the
//!   witnesses looked like dead weight. A cone settled it. Lop the top off a
//!   sharp tip and the cut is a small disc, every point of it close to the
//!   cone's side, and without the witnesses the sculpt ended up to 156% of
//!   the tolerance from the result. The quadric had been saving the cube: it
//!   puts a vertex where three planes meet, and a cone's side is not planes.
//!
//! What the reverse direction still is not is a proof. A point absorbed
//! early and then displaced as witness by others further out is no longer
//! watched, so this direction is held by samples where the forward direction
//! is held by the field. The report's measured figure is therefore the
//! forward one. The tests measure the reverse by brute force, every vertex of
//! the full mesh against the result, and hold it to the tolerance and a
//! fiftieth on every fixture. The worst measured is 99%.
//!
//! # What stays exactly as it was
//!
//! **Topology.** A collapse is taken only when the two vertices have exactly
//! two neighbours in common (the link condition) and no vertex is left with
//! fewer than three triangles. Under those two rules a closed manifold stays a
//! closed manifold with the same number of handles, and
//! `a_through_hole_is_still_a_through_hole` counts them.
//!
//! **Paint boundaries.** A filament slot is an index: a boundary that moves
//! prints in the wrong filament. Every vertex that touches triangles of more
//! than one slot is locked. It is never moved and never removed, so a boundary
//! [`crate::mesh`] cut at half-triangle precision keeps every vertex it was
//! cut with, and triangles either side of it are simplified up to it.
//! Simplifying ALONG a boundary is not attempted; it costs a strip of dense
//! triangles the length of the boundary, which is small.
//!
//! **Printability.** Nothing here weakens the gate. The caller validates the
//! result exactly as it validates an undecimated mesh, and
//! [`Volume::export_mesh_decimated`] hands back the dense mesh, with the
//! reason, if the decimated one is not printable. That branch is believed
//! unreachable and is there because "believed" is not the standard exports
//! are held to.
//!
//! # The tolerance has a ceiling, and why
//!
//! [`MAX_TOLERANCE_VOXELS`], one voxel. The field knows distances for three
//! voxels either side of the surface and nothing beyond, so a looser tolerance
//! could not be checked; and two sheets of a thin wall can lie two voxels
//! apart, so surfaces allowed to wander a voxel each could meet. A coarser
//! mesh than that is a coarser sculpt, which is what the resample buttons are
//! for.
//!
//! # How it runs
//!
//! Sweeps over every edge, collapsing each that passes at the sweep's
//! threshold, with the threshold stepped up to the tolerance over the first
//! few sweeps so that the cheapest collapses are taken first. There is no
//! priority queue: a heap over the sixteen million edges of a large export is
//! several hundred megabytes holding an ordering that the field's verdict,
//! not the queue's, decides the result of.
//!
//! An edge that failed is not asked again until something beside it changes,
//! so once the threshold has stopped rising a sweep costs what moved in the
//! last one and not the whole mesh.
//!
//! # What it costs and what it saves, measured
//!
//! `the_cost_of_decimating_is_printed`, release build, one thread of a shared
//! four core container, a 60 mm ball:
//!
//! | voxel | full mesh | at 0.01 mm | at 0.03 mm | at 0.1 mm | time |
//! |---|---|---|---|---|---|
//! | 0.5 | 135,324 | 24,464 | 8,146 | 2,466 | 1.7 to 2.1 s |
//! | 0.25 | 542,556 | 24,830 | 8,520 | 2,658 | 6.5 to 7.2 s |
//! | 0.125 | 2,170,860 | 26,632 | 9,294 | 2,788 | 30 s |
//!
//! Read down a column: **the result barely depends on the voxel size.** A
//! ball needs about twenty five thousand triangles to stay within a hundredth
//! of a millimetre however finely it was sculpted, which is the statement
//! that the tolerance is a property of the shape and the print, and the voxel
//! is not. Read along the bottom row: two million triangles become nine
//! thousand at a tolerance finer than a resin printer's pixel.
//!
//! **It is not fast.** About 14 microseconds a triangle of input, so the
//! eleven million triangle export in the README would take two and a half
//! minutes. Nearly all of it is reading the field across every triangle a
//! collapse reshapes, at half voxel spacing, and that density was arrived at
//! by an independent measurement finding what coarser sampling missed (see
//! `SAMPLE_SPACING_VOXELS`). It was 3 microseconds when triangles were read at
//! four points each, and the results were wrong.
//!
//! It is an export and not a frame, it runs off the drawing thread, and it is
//! single threaded where it need not be: collapses in different parts of the
//! mesh do not touch each other, and bricks give an obvious way to hand them
//! to different cores. That is the next thing to do to this file.
//!
//! # It gives back some of what the mesher rounds off
//!
//! Not designed in. Surface nets puts a vertex at the mean of its cell's edge
//! crossings, which rounds a sharp corner: on a cube, no vertex of the full
//! mesh is within 1.16 voxels of a corner. The field holds the corner far
//! better than that. Because every vertex is first stood on the field, and
//! the quadric then places a merged vertex where the planes round it meet,
//! the cube's 18,252 triangles come back as 12 with every corner within 0.31
//! of a voxel of where it belongs. A decimated export of a cut model has
//! crisper edges than the full one.
//!
//! It is a side effect and not a meshing method. Recovering edges properly is
//! what dual contouring is for, and that is in the plan's list of things not
//! done.

use glam::{DMat3, DVec3, IVec3, Vec3};

use crate::brick::{BRICK_DIM, Brick, BrickCoord, NARROW_BAND};
use crate::export::{ExportMesh, MeshReport};
use crate::volume::Volume;

/// The loosest tolerance that can be verified, in voxels. See the module
/// documentation.
pub const MAX_TOLERANCE_VOXELS: f32 = 1.0;

/// How a triangle's normal must agree with the field's gradient at its
/// centre: the cosine of the largest angle allowed between them.
const MIN_AGREEMENT: f32 = 0.5;

/// How far a collapse may turn any triangle it reshapes, as the cosine of the
/// angle between its normal before and after.
const MIN_TURN: f32 = 0.2;

/// The worst shaped triangle a collapse may create, on a scale where an
/// equilateral triangle is 1 and a line is 0.
///
/// A sliver is still a closed surface and still prints. It is refused because
/// the build specification names degenerate triangles from decimation as the
/// thing that made SindriCAD's exports non-manifold, and a sliver is one
/// rounding error from degenerate. A collapse may leave a triangle worse than
/// this only if it was already worse and is not made worse still.
const MIN_QUALITY: f32 = 0.05;

/// How many points of the sculpt each vertex answers for.
///
/// Four, by measurement. Across a cube, a ball, a roughened ball and three
/// cones at five tolerances each, the furthest any point of the sculpt ended
/// from the result was 151% of the tolerance with no witnesses, 104% with
/// two, and 99% with four. On a crease or a tip the points a vertex has
/// absorbed lie to several sides of it, and each side needs its own.
///
/// Two was tried as a saving, at half the memory, and is what let three of
/// the other checks be deleted without any test noticing: the tests allowed
/// a twentieth over the tolerance and two witnesses used all of it. They now
/// allow a fiftieth.
///
/// It is 48 bytes a vertex for the length of an export.
const WITNESSES: usize = 4;

/// The share of the tolerance asked for that the pass works to.
///
/// Every check is made at points: a triangle's middle, the middles of its
/// edges, a handful of witnesses. A surface is not points, and the furthest
/// place on a triangle from a curved surface is not exactly at any of them.
/// Measured with the pass working to the whole tolerance, as the furthest
/// any vertex of the full mesh lay from the result: up to 7% over the
/// tolerance on a roughened ball, 2% on a cube, 1% on a plain ball. Working
/// to nine tenths leaves room for that, so that the number the user typed is
/// one the tests can hold the result to from both sides.
const WORKING_SHARE: f32 = 0.9;

/// How far apart a triangle is sampled, in voxels.
///
/// Measured, not reasoned. The first version looked at a triangle's middle
/// and the middles of its edges and nowhere else, which is sampling it
/// properly when it is a voxel across and not at all when it is twenty: on a
/// roughened ball the result passed over bumps and left the sculpt 29%
/// further out than the tolerance. Every two voxels, and then every voxel,
/// each closed most of that and not all: a reading of the result taken
/// independently at half a voxel still found triangles up to 16% past what
/// was allowed, because the trilinear field has a kink at every lattice
/// plane and a sample a voxel away does not see it. At half a voxel the
/// decimator's own reading and the independent one agree to three figures on
/// every fixture.
const SAMPLE_SPACING_VOXELS: f32 = 0.5;

/// The finest a triangle is ever sampled: this many steps along each edge.
/// A triangle longer than `SAMPLE_SPACING_VOXELS` times this is on a flat, or
/// the samples it does get would have refused it.
const MAX_SAMPLE_STEPS: usize = 16;

/// Newton steps each vertex is given to reach the field's surface.
const SETTLE_STEPS: usize = 4;

/// The fractions of the tolerance the first sweeps run at.
const RAMP: [f32; 3] = [0.25, 0.5, 1.0];

/// Sweeps after which the pass stops whatever is still moving.
const MAX_SWEEPS: usize = 64;

/// A sweep that removes less than one vertex in this many ends the pass.
const WORTH_ANOTHER_SWEEP: usize = 400;

const DEAD: u32 = u32::MAX;
const NONE: u32 = u32::MAX;

/// What to decimate to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecimateSpec {
    /// The furthest any part of the result may lie from the sculpt, in world
    /// units. Clamped to [`MAX_TOLERANCE_VOXELS`].
    pub tolerance_mm: f32,
    /// Stop once the mesh is down to this many triangles, if it gets there
    /// inside the tolerance. `None` takes everything the tolerance allows.
    pub target_triangles: Option<usize>,
}

impl DecimateSpec {
    /// Everything the tolerance allows.
    pub fn within(tolerance_mm: f32) -> Self {
        Self { tolerance_mm, target_triangles: None }
    }
}

/// What a decimation did, for the status line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DecimateReport {
    pub triangles_before: usize,
    pub triangles_after: usize,
    pub vertices_before: usize,
    pub vertices_after: usize,
    /// The tolerance actually used, after the clamp.
    pub tolerance_mm: f32,
    /// The furthest from the sculpt that any vertex, triangle centre or edge
    /// midpoint of the RESULT was measured to lie, after the fact.
    ///
    /// Never more than the larger of `tolerance_mm` and
    /// `dense_deviation_mm`: these are the points every collapse was checked
    /// at, so nothing the decimation did can exceed the tolerance, and what
    /// it did not touch is as far out as it was to begin with.
    pub max_deviation_mm: f32,
    /// The same measurement of the mesh BEFORE any collapse.
    ///
    /// Not zero, and worth knowing. A triangle is flat and the surface it
    /// spans is not, so even one vertex a voxel leaves the middle of a
    /// triangle a little off a curved surface. Where this is above the
    /// tolerance asked for, the tolerance is tighter than the full mesh
    /// itself achieves there and nothing in that region can be collapsed.
    pub dense_deviation_mm: f32,
    /// Triangles of the result that measurement covered. Every one, unless
    /// some lie off the field's band, which a mesh near the sculpt does not.
    pub measured_triangles: usize,
    pub sweeps: usize,
    /// Collapses refused because the field said the surface would leave the
    /// tolerance or stop facing the way the sculpt does.
    pub refused_by_field: usize,
    /// Collapses refused because they would have changed the topology.
    pub refused_by_topology: usize,
    /// Collapses refused because they would have flipped or flattened a
    /// triangle.
    pub refused_by_shape: usize,
    /// Collapses refused to keep a paint boundary where it is.
    pub refused_by_paint: usize,
    /// True when the decimated mesh failed validation and the dense one was
    /// returned in its place. See [`Volume::export_mesh_decimated`].
    pub fell_back: bool,
}

impl DecimateReport {
    /// One line for the status bar.
    pub fn summary(&self) -> String {
        if self.fell_back {
            return format!(
                "decimation gave a mesh that would not print, so the full {} triangles were kept",
                self.triangles_before
            );
        }
        if self.dense_deviation_mm > self.tolerance_mm {
            // Said outright, because otherwise the two numbers read as a
            // broken promise: the result is further out than the tolerance,
            // and it is so only where the full mesh already was.
            return format!(
                "{} to {} triangles, within {:.3} mm of the sculpt except where the full mesh \
                 is already further (up to {:.3} mm)",
                self.triangles_before,
                self.triangles_after,
                self.tolerance_mm,
                self.dense_deviation_mm
            );
        }
        format!(
            "{} to {} triangles, within {:.3} mm of the sculpt (measured {:.3} mm)",
            self.triangles_before, self.triangles_after, self.tolerance_mm, self.max_deviation_mm
        )
    }
}

impl Volume {
    /// [`Volume::export_mesh`], decimated to `spec`.
    ///
    /// The report from [`ExportMesh::validate`] is of the mesh returned. If
    /// the decimated mesh is not printable and the dense one is, the dense
    /// one is returned and [`DecimateReport::fell_back`] says so: an export
    /// never gets worse for having been asked to get smaller.
    pub fn export_mesh_decimated(
        &self,
        spec: DecimateSpec,
    ) -> (ExportMesh, MeshReport, DecimateReport) {
        let (dense, dense_report) = self.export_mesh();
        if !dense_report.is_printable() {
            // Nothing below is entitled to assume a manifold, and a mesh that
            // would not print is refused by the caller either way.
            let report = DecimateReport {
                triangles_before: dense.triangles.len(),
                triangles_after: dense.triangles.len(),
                vertices_before: dense.positions.len(),
                vertices_after: dense.positions.len(),
                ..DecimateReport::default()
            };
            return (dense, dense_report, report);
        }
        let (mesh, report) = dense.decimated(self, spec);
        keep_what_prints((dense, dense_report), mesh, report)
    }
}

impl crate::body::Document {
    /// [`crate::body::Document::export_bodies`], with every body decimated
    /// to `spec`, and what the decimation of each did.
    ///
    /// Per body, for the reason the weld is per body: each is checked
    /// against its own field. `visible` means what it means there.
    pub fn export_bodies_decimated(
        &self,
        visible: &[bool],
        spec: DecimateSpec,
    ) -> (Vec<crate::export::ExportedBody>, Vec<DecimateReport>) {
        assert_eq!(
            visible.len(),
            self.node_count(),
            "the visibility mask is indexed by node position and must cover every node"
        );
        let mut bodies = Vec::new();
        let mut reports = Vec::new();
        for (node, _) in self.nodes().iter().zip(visible).filter(|(_, shown)| **shown) {
            let Some(volume) = node.volume() else {
                continue;
            };
            let (mesh, mesh_report, report) = volume.export_mesh_decimated(spec);
            bodies.push((node.meta(), mesh, mesh_report));
            reports.push(report);
        }
        (bodies, reports)
    }
}

impl DecimateReport {
    /// Several bodies' reports as one, for a status line about a whole
    /// export: the counts added, the deviations at their worst.
    pub fn summed(reports: &[DecimateReport]) -> DecimateReport {
        let mut total = DecimateReport::default();
        for report in reports {
            total.triangles_before += report.triangles_before;
            total.triangles_after += report.triangles_after;
            total.vertices_before += report.vertices_before;
            total.vertices_after += report.vertices_after;
            total.tolerance_mm = total.tolerance_mm.max(report.tolerance_mm);
            total.max_deviation_mm = total.max_deviation_mm.max(report.max_deviation_mm);
            total.dense_deviation_mm = total.dense_deviation_mm.max(report.dense_deviation_mm);
            total.measured_triangles += report.measured_triangles;
            total.sweeps = total.sweeps.max(report.sweeps);
            total.refused_by_field += report.refused_by_field;
            total.refused_by_topology += report.refused_by_topology;
            total.refused_by_shape += report.refused_by_shape;
            total.refused_by_paint += report.refused_by_paint;
            total.fell_back |= report.fell_back;
        }
        total
    }
}

/// The decimated mesh if it is printable, and the dense one if it is not.
///
/// Its own function so that the branch can be tested. No volume this module
/// has been shown produces a decimated mesh that fails validation, which
/// means the only way to exercise "and if it does" is to hand it one.
fn keep_what_prints(
    (dense, dense_report): (ExportMesh, MeshReport),
    decimated: ExportMesh,
    mut report: DecimateReport,
) -> (ExportMesh, MeshReport, DecimateReport) {
    let mut decimated_report = decimated.validate();
    decimated_report.collapsed_triangles = dense_report.collapsed_triangles;
    if decimated_report.is_printable() {
        return (decimated, decimated_report, report);
    }
    report.fell_back = true;
    report.triangles_after = report.triangles_before;
    report.vertices_after = report.vertices_before;
    report.max_deviation_mm = report.dense_deviation_mm;
    (dense, dense_report, report)
}

impl ExportMesh {
    /// This mesh with as few triangles as `spec` allows, checked against the
    /// volume it was exported from.
    ///
    /// `self` must be a closed manifold, which is what
    /// [`Volume::export_mesh`] produces and [`ExportMesh::validate`] checks;
    /// [`Volume::export_mesh_decimated`] is the entry point that makes sure.
    pub fn decimated(&self, volume: &Volume, spec: DecimateSpec) -> (ExportMesh, DecimateReport) {
        let voxel = volume.voxel_size();
        // Asked of what was passed, before the clamp: `f32::min` returns the
        // other operand for a NaN, so clamping first would turn "no tolerance
        // at all" into the loosest one there is.
        let tolerance = if spec.tolerance_mm > 0.0 {
            spec.tolerance_mm.min(MAX_TOLERANCE_VOXELS * voxel)
        } else {
            0.0
        };
        let mut report = DecimateReport {
            triangles_before: self.triangles.len(),
            triangles_after: self.triangles.len(),
            vertices_before: self.positions.len(),
            vertices_after: self.positions.len(),
            tolerance_mm: tolerance,
            ..DecimateReport::default()
        };
        if tolerance <= 0.0 || self.triangles.is_empty() {
            return (self.clone(), report);
        }

        let mut work = Simplifier::new(self, volume, tolerance * WORKING_SHARE);
        work.settle_onto_the_field();
        report.dense_deviation_mm = work.measure().0;
        work.run(spec.target_triangles.unwrap_or(0), &mut report);
        let mesh = work.finish(&mut report);
        (mesh, report)
    }
}

/// Reads the field at a point without a hash lookup per corner.
///
/// A trilinear sample is eight voxels, and through [`Volume::sample_voxel`]
/// each is a lookup in the brick map. Nearly every sample a decimation takes
/// is beside the one before it, so the brick is remembered: when all eight
/// corners are in it, which is all but the samples straddling a brick face,
/// the read is eight array indexings.
struct Sampler<'a> {
    volume: &'a Volume,
    inverse_voxel: f32,
    cached: Option<(BrickCoord, Option<&'a Brick>)>,
}

/// One reading of the field.
#[derive(Clone, Copy)]
struct Reading {
    /// How far the point is from the surface, in world units, corrected for
    /// gradient drift. Infinite when the point is outside the band, where the
    /// field holds no distance.
    deviation: f32,
    /// Signed distance in world units, on the same terms.
    signed: f32,
    /// Unit outward direction of the field here, or zero where it has none.
    outward: Vec3,
}

impl<'a> Sampler<'a> {
    fn new(volume: &'a Volume) -> Self {
        Self { volume, inverse_voxel: 1.0 / volume.voxel_size(), cached: None }
    }

    fn corners(&mut self, base: IVec3) -> [f32; 8] {
        let coord = BrickCoord::containing(base);
        let local = base - coord.origin();
        let last = BRICK_DIM as i32 - 1;
        if local.cmplt(IVec3::splat(last)).all() {
            let brick = match self.cached {
                Some((cached, brick)) if cached == coord => brick,
                _ => {
                    let brick = self.volume.brick(coord);
                    self.cached = Some((coord, brick));
                    brick
                }
            };
            let Some(brick) = brick else {
                return [crate::brick::OUTSIDE; 8];
            };
            let (x, y, z) = (local.x as usize, local.y as usize, local.z as usize);
            return [
                brick.get(x, y, z),
                brick.get(x + 1, y, z),
                brick.get(x, y + 1, z),
                brick.get(x + 1, y + 1, z),
                brick.get(x, y, z + 1),
                brick.get(x + 1, y, z + 1),
                brick.get(x, y + 1, z + 1),
                brick.get(x + 1, y + 1, z + 1),
            ];
        }
        let at = |dx, dy, dz| self.volume.sample_voxel(base + IVec3::new(dx, dy, dz));
        [
            at(0, 0, 0),
            at(1, 0, 0),
            at(0, 1, 0),
            at(1, 1, 0),
            at(0, 0, 1),
            at(1, 0, 1),
            at(0, 1, 1),
            at(1, 1, 1),
        ]
    }

    fn read(&mut self, point: Vec3) -> Reading {
        const OUT_OF_BAND: Reading =
            Reading { deviation: f32::INFINITY, signed: f32::INFINITY, outward: Vec3::ZERO };

        let grid = point * self.inverse_voxel;
        let base = grid.floor();
        let f = grid - base;
        let c = self.corners(base.as_ivec3());
        // A saturated corner is not a distance, and a sample that leans on
        // one reads the edge of the band as if it were the surface.
        if c.iter().any(|value| !value.is_finite() || value.abs() >= NARROW_BAND) {
            return OUT_OF_BAND;
        }

        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x00 = lerp(c[0], c[1], f.x);
        let x10 = lerp(c[2], c[3], f.x);
        let x01 = lerp(c[4], c[5], f.x);
        let x11 = lerp(c[6], c[7], f.x);
        let y0 = lerp(x00, x10, f.y);
        let y1 = lerp(x01, x11, f.y);
        let value = lerp(y0, y1, f.z);

        // The gradient of the same trilinear patch, so it costs no reads.
        let gradient = Vec3::new(
            lerp(lerp(c[1] - c[0], c[3] - c[2], f.y), lerp(c[5] - c[4], c[7] - c[6], f.y), f.z),
            lerp(x10 - x00, x11 - x01, f.z),
            y1 - y0,
        );
        let length = gradient.length();
        // Corrected one way only. Where the brushes have FLATTENED the field
        // its value under-reads the distance, and dividing by the slope puts
        // that right, down to the floor `generate` uses against a plateau
        // turning rounding into distance. Where they have STEEPENED it the
        // value over-reads, and that is left alone: an over-read refuses a
        // collapse that might have been allowed, which is the safe direction,
        // and two ways of estimating a slope above one were found to disagree
        // by 60% at a kink. The first version divided both ways, and an
        // independent reading of its results found triangles 0.40 from the
        // field that it had measured at 0.23.
        let slope = length.clamp(0.5, 1.0);
        let signed = value / slope / self.inverse_voxel;
        Reading {
            deviation: signed.abs(),
            signed,
            outward: if length > 1.0e-6 { gradient / length } else { Vec3::ZERO },
        }
    }
}

/// Why an edge was not collapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    Field,
    Topology,
    Shape,
    Paint,
    /// Nothing to report: both ends locked, or the edge is no longer an edge.
    Quiet,
}

/// One collapse that passed every check.
struct Collapse {
    remove: u32,
    keep: u32,
    position: Vec3,
    /// What [`Simplifier::witness`] becomes for the kept vertex.
    witness: [Vec3; WITNESSES],
}

struct Simplifier<'a> {
    sampler: Sampler<'a>,
    tolerance: f32,
    /// World distance between the samples taken across a triangle.
    spacing: f32,
    positions: Vec<Vec3>,
    /// `[DEAD, _, _]` once a triangle has been collapsed away.
    triangles: Vec<[u32; 3]>,
    /// First corner of each vertex's list of corners, or [`NONE`]. Corner
    /// `3 t + k` is slot `k` of triangle `t`.
    head: Vec<u32>,
    /// The next corner of the same vertex, per corner.
    next: Vec<u32>,
    /// The sweep in which each vertex's neighbourhood last changed.
    changed: Vec<i32>,
    /// Per vertex slot, and per triangle slot, when the mesh is painted.
    vertex_slot: Vec<u8>,
    triangle_slot: Vec<u8>,
    /// Vertices that touch more than one slot, or whose own slot is not their
    /// triangles'. Never moved and never removed.
    locked: Vec<bool>,
    /// For each vertex, the points of the sculpt that the collapses merged
    /// into it have left furthest from the mesh. Each starts as the vertex's
    /// own position, which is on its own triangles and so costs nothing to
    /// hold. See the "both directions" section of the module documentation.
    witness: Vec<[Vec3; WITNESSES]>,
    alive_triangles: usize,
    alive_vertices: usize,
    // Reused between edges, so the inner loop does not allocate.
    around_remove: Vec<u32>,
    around_keep: Vec<u32>,
    ring: Vec<u32>,
}

impl<'a> Simplifier<'a> {
    fn new(mesh: &ExportMesh, volume: &'a Volume, tolerance: f32) -> Self {
        let vertices = mesh.positions.len();
        let painted = !mesh.slots.is_empty();
        let mut work = Self {
            sampler: Sampler::new(volume),
            tolerance,
            spacing: SAMPLE_SPACING_VOXELS * volume.voxel_size(),
            positions: mesh.positions.clone(),
            triangles: mesh.triangles.clone(),
            head: vec![NONE; vertices],
            next: vec![NONE; mesh.triangles.len() * 3],
            changed: vec![i32::MIN / 2; vertices],
            vertex_slot: if painted { mesh.slots.clone() } else { Vec::new() },
            triangle_slot: Vec::new(),
            locked: vec![false; vertices],
            witness: Vec::new(),
            alive_triangles: mesh.triangles.len(),
            alive_vertices: vertices,
            around_remove: Vec::new(),
            around_keep: Vec::new(),
            ring: Vec::new(),
        };
        work.link_corners();

        if painted {
            // The slot a triangle prints in is its FIRST vertex's: the rule
            // the shader and the 3MF writer both read, and the one
            // `split_paint_boundaries` arranges every triangle to satisfy.
            work.triangle_slot =
                mesh.triangles.iter().map(|triangle| mesh.slots[triangle[0] as usize]).collect();
            for (triangle, slot) in work.triangles.iter().zip(&work.triangle_slot) {
                for vertex in triangle {
                    if work.vertex_slot[*vertex as usize] != *slot {
                        work.locked[*vertex as usize] = true;
                    }
                }
            }
        }
        work
    }

    /// Rebuild every vertex's corner list from the triangles still alive.
    fn link_corners(&mut self) {
        self.head.fill(NONE);
        for (index, triangle) in self.triangles.iter().enumerate() {
            if triangle[0] == DEAD {
                continue;
            }
            for (slot, vertex) in triangle.iter().enumerate() {
                let corner = (index * 3 + slot) as u32;
                self.next[corner as usize] = self.head[*vertex as usize];
                self.head[*vertex as usize] = corner;
            }
        }
    }

    /// The live triangles round `vertex`, into `out`.
    fn around(&self, vertex: u32, out: &mut Vec<u32>) {
        out.clear();
        let mut corner = self.head[vertex as usize];
        while corner != NONE {
            let triangle = corner / 3;
            if self.triangles[triangle as usize][0] != DEAD {
                out.push(triangle);
            }
            corner = self.next[corner as usize];
        }
    }

    /// Move every free vertex onto the field's own surface.
    ///
    /// The mesher and the field disagree about where the surface is, by a
    /// fraction of a voxel. Surface nets puts a vertex at the mean of its
    /// cell's edge crossings, which lies a little inside a convex surface;
    /// the field's zero set, as [`Volume::sample_world`] and the raycast
    /// read it, is where the trilinear patch crosses zero. Neither is wrong.
    /// But every collapse below is judged against the second, and a vertex
    /// left where the first put it is already up to a third of a voxel from
    /// it: measured at 0.29 on a roughened ball, against a tolerance that may
    /// be a tenth of that. Nothing could be collapsed near such a vertex, and
    /// the report would have the result further from the sculpt than the
    /// tolerance before the pass had done anything.
    ///
    /// So the mesh is first put where the judge is looking. One Newton step
    /// along the gradient, and not taken where it would flip or flatten a
    /// triangle. Locked vertices stay: a paint boundary does not move.
    fn settle_onto_the_field(&mut self) {
        let mut around = std::mem::take(&mut self.around_keep);
        for vertex in 0..self.positions.len() as u32 {
            if self.locked[vertex as usize] {
                continue;
            }
            self.around(vertex, &mut around);
            // A few steps and not one. On a smooth surface the first lands;
            // at a corner the gradient turns within a voxel, and a single
            // step along where it pointed at the start stops short. Measured
            // on a cube: the corner vertices were left 0.18 of a voxel off the
            // field after one step, and every reverse measurement was then
            // being taken from a point that was not on the sculpt.
            for _ in 0..SETTLE_STEPS {
                let here = self.positions[vertex as usize];
                let reading = self.sampler.read(here);
                if !reading.signed.is_finite()
                    || reading.outward == Vec3::ZERO
                    || reading.deviation <= self.tolerance * 1.0e-3
                {
                    break;
                }
                let there = here - reading.outward * reading.signed;
                let harmless = around.iter().all(|triangle| {
                    let corners = self.triangles[*triangle as usize];
                    let old = corners.map(|corner| self.positions[corner as usize]);
                    let new = corners.map(|corner| {
                        if corner == vertex { there } else { self.positions[corner as usize] }
                    });
                    let old_normal = (old[1] - old[0]).cross(old[2] - old[0]);
                    let new_normal = (new[1] - new[0]).cross(new[2] - new[0]);
                    new_normal.length() > 1.0e-12
                        && old_normal.dot(new_normal)
                            > MIN_TURN * old_normal.length() * new_normal.length()
                });
                if !harmless {
                    break;
                }
                self.positions[vertex as usize] = there;
            }
        }
        self.around_keep = around;
        // Every vertex now stands on the sculpt, or as near as it is going
        // to, and so is the first point of the sculpt it answers for.
        self.witness = self.positions.iter().map(|position| [*position; WITNESSES]).collect();
    }

    /// The furthest from the field that any sample across a triangle reads,
    /// its three corners aside.
    ///
    /// The samples are a barycentric lattice fine enough to leave no two
    /// further apart than [`Simplifier::spacing`], up to
    /// [`MAX_SAMPLE_STEPS`] along an edge. At two steps, which is every
    /// triangle a voxel or two across, that is the middles of the three
    /// edges. Infinite if any sample is off the band, which no triangle near
    /// the sculpt has.
    ///
    /// Stops at the first sample past `limit` and returns it: a collapse
    /// that is going to be refused does not need to know by how much.
    fn furthest_sample(&mut self, points: [Vec3; 3], limit: f32) -> f32 {
        let longest = (points[1] - points[0])
            .length_squared()
            .max((points[2] - points[1]).length_squared())
            .max((points[0] - points[2]).length_squared())
            .sqrt();
        let steps = ((longest / self.spacing).ceil() as usize).clamp(2, MAX_SAMPLE_STEPS);
        let mut worst: f32 = 0.0;
        for i in 0..=steps {
            for j in 0..=(steps - i) {
                let k = steps - i - j;
                // The corners are vertices, which are asked about as
                // vertices.
                if i == steps || j == steps || k == steps {
                    continue;
                }
                let at = (points[0] * i as f32 + points[1] * j as f32 + points[2] * k as f32)
                    / steps as f32;
                worst = worst.max(self.sampler.read(at).deviation);
                if worst > limit {
                    return worst;
                }
            }
        }
        worst
    }

    /// The furthest any live triangle's samples, or any of its corners, reads
    /// from the field, and how many triangles that was.
    fn measure(&mut self) -> (f32, usize) {
        let mut worst: f32 = 0.0;
        let mut measured = 0;
        for index in 0..self.triangles.len() {
            let triangle = self.triangles[index];
            if triangle[0] == DEAD {
                continue;
            }
            let points = triangle.map(|vertex| self.positions[vertex as usize]);
            let mut here = self.furthest_sample(points, f32::INFINITY);
            for sample in
                [(points[0] + points[1] + points[2]) / 3.0, points[0], points[1], points[2]]
            {
                here = here.max(self.sampler.read(sample).deviation);
            }
            // A sample off the band has no distance to report. It is counted
            // as unmeasured and not as zero.
            if here.is_finite() {
                worst = worst.max(here);
                measured += 1;
            }
        }
        (worst, measured)
    }

    fn run(&mut self, target_triangles: usize, report: &mut DecimateReport) {
        let mut last_threshold = 0.0;
        for sweep in 0..MAX_SWEEPS {
            let threshold = self.tolerance * RAMP.get(sweep).copied().unwrap_or(1.0);
            let settled = threshold == last_threshold;
            last_threshold = threshold;
            let sweep = sweep as i32;
            let before = self.alive_vertices;

            'triangles: for index in 0..self.triangles.len() {
                for slot in 0..3 {
                    let triangle = self.triangles[index];
                    if triangle[0] == DEAD {
                        continue 'triangles;
                    }
                    let (a, b) = (triangle[slot], triangle[(slot + 1) % 3]);
                    // Each edge is two half edges, one in each of its
                    // triangles. Taking the one that runs low to high visits
                    // every edge once.
                    if a >= b {
                        continue;
                    }
                    // Asked last sweep, at this threshold, with nothing round
                    // either end changed since: the answer has not changed.
                    if settled
                        && self.changed[a as usize] < sweep - 1
                        && self.changed[b as usize] < sweep - 1
                    {
                        continue;
                    }
                    match self.consider(a, b, threshold) {
                        Ok(collapse) => {
                            self.apply(collapse, sweep);
                            if self.alive_triangles <= target_triangles {
                                report.sweeps = sweep as usize + 1;
                                return;
                            }
                        }
                        // Counted only at the full tolerance, or every edge
                        // the ramp turned away early would be counted as
                        // refused when it was only deferred.
                        Err(refusal) if threshold >= self.tolerance => match refusal {
                            Refusal::Field => report.refused_by_field += 1,
                            Refusal::Topology => report.refused_by_topology += 1,
                            Refusal::Shape => report.refused_by_shape += 1,
                            Refusal::Paint => report.refused_by_paint += 1,
                            Refusal::Quiet => {}
                        },
                        Err(_) => {}
                    }
                }
            }

            report.sweeps = sweep as usize + 1;
            // Dead triangles are skipped as lists are walked and never
            // unlinked, so the lists are rebuilt once a sweep to stop them
            // filling with the dead.
            self.link_corners();
            let removed = before - self.alive_vertices;
            if settled && removed * WORTH_ANOTHER_SWEEP < self.alive_vertices.max(1) {
                break;
            }
        }
    }

    /// Whether the edge `a`-`b` may be collapsed at `threshold`, and to where.
    fn consider(&mut self, a: u32, b: u32, threshold: f32) -> Result<Collapse, Refusal> {
        let painted = !self.vertex_slot.is_empty();
        // A locked vertex is never removed, so it is the one kept, and the
        // merged vertex goes exactly where it already is.
        let (remove, keep, fixed) = match (self.locked[a as usize], self.locked[b as usize]) {
            (true, true) => return Err(Refusal::Quiet),
            (true, false) => (b, a, true),
            (false, true) => (a, b, true),
            (false, false) => (a, b, false),
        };
        if painted && !fixed && self.vertex_slot[remove as usize] != self.vertex_slot[keep as usize]
        {
            // Two unlocked vertices of different slots cannot share an edge
            // in a mesh that obeys the first vertex rule, but nothing here
            // should be the thing that depends on it.
            return Err(Refusal::Paint);
        }

        let mut around_remove = std::mem::take(&mut self.around_remove);
        let mut around_keep = std::mem::take(&mut self.around_keep);
        let mut ring = std::mem::take(&mut self.ring);
        self.around(remove, &mut around_remove);
        self.around(keep, &mut around_keep);
        let verdict =
            self.judge(remove, keep, fixed, threshold, &around_remove, &around_keep, &mut ring);
        self.around_remove = around_remove;
        self.around_keep = around_keep;
        self.ring = ring;
        verdict
    }

    #[allow(clippy::too_many_arguments)]
    fn judge(
        &mut self,
        remove: u32,
        keep: u32,
        fixed: bool,
        threshold: f32,
        around_remove: &[u32],
        around_keep: &[u32],
        ring: &mut Vec<u32>,
    ) -> Result<Collapse, Refusal> {
        // ---- topology
        //
        // The triangles the edge lies in, which the collapse removes. A
        // closed manifold has exactly two.
        let shared = around_remove
            .iter()
            .filter(|triangle| self.triangles[**triangle as usize].contains(&keep))
            .count();
        if shared != 2 {
            return Err(if shared == 0 { Refusal::Quiet } else { Refusal::Topology });
        }
        // The link condition: the vertices adjacent to BOTH ends must be the
        // two opposite the edge and no others. A third means the collapse
        // would pinch the surface into two sheets meeting along an edge.
        ring.clear();
        for triangle in around_remove {
            for vertex in self.triangles[*triangle as usize] {
                if vertex != remove && vertex != keep && !ring.contains(&vertex) {
                    ring.push(vertex);
                }
            }
        }
        let mut common = 0;
        let mut seen = [NONE; 2];
        for triangle in around_keep {
            for vertex in self.triangles[*triangle as usize] {
                if vertex != remove && vertex != keep && ring.contains(&vertex) {
                    // Counted once: a common neighbour appears in two of
                    // `keep`'s triangles.
                    if !seen[..common.min(2)].contains(&vertex) {
                        if common < 2 {
                            seen[common] = vertex;
                        }
                        common += 1;
                    }
                }
            }
        }
        if common != 2 {
            return Err(Refusal::Topology);
        }
        // The kept vertex ends with both fans less the two triangles that
        // go, and three is the fewest a vertex of a closed surface can have.
        // This is the tetrahedron, which passes the link condition: every
        // edge of one has exactly two common neighbours and collapsing any
        // of them leaves two triangles back to back.
        //
        // The vertices OPPOSITE the edge each lose a triangle too, and are
        // not asked here, because the link condition has already answered
        // for them: one with only three triangles has a third neighbour that
        // is adjacent to both ends of the edge, which is a third common
        // neighbour.
        if around_remove.len() + around_keep.len() < 4 + 3 {
            return Err(Refusal::Topology);
        }

        // ---- where the merged vertex goes
        let position = if fixed {
            self.positions[keep as usize]
        } else {
            self.best_position(remove, keep, around_remove, around_keep)
        };
        if !fixed {
            let reading = self.sampler.read(position);
            if reading.deviation > threshold {
                return Err(Refusal::Field);
            }
        }

        // ---- every triangle the collapse reshapes: its shape, and its paint
        //
        // In this order on purpose. These two are arithmetic on a few
        // corners; the reverse check below is a handful of distances; the
        // field's verdict at the end reads the field across every reshaped
        // triangle at half voxel spacing, and is most of what a collapse
        // costs. So everything that can refuse cheaply refuses first.
        for (list, moved) in [(around_remove, remove), (around_keep, keep)] {
            // A kept vertex that does not move reshapes nothing of its own.
            if fixed && moved == keep {
                continue;
            }
            for triangle in list {
                let corners = self.triangles[*triangle as usize];
                if corners.contains(&remove) && corners.contains(&keep) {
                    continue;
                }
                let old = corners.map(|vertex| self.positions[vertex as usize]);
                let new = corners.map(|vertex| {
                    if vertex == moved { position } else { self.positions[vertex as usize] }
                });

                let old_normal = (old[1] - old[0]).cross(old[2] - old[0]);
                let new_normal = (new[1] - new[0]).cross(new[2] - new[0]);
                let new_length = new_normal.length();
                if new_length <= 1.0e-12
                    || old_normal.dot(new_normal) <= MIN_TURN * old_normal.length() * new_length
                {
                    return Err(Refusal::Shape);
                }
                let quality = quality(new, new_length);
                if quality < MIN_QUALITY && quality < quality_of(old, old_normal.length()) {
                    return Err(Refusal::Shape);
                }

                // The first vertex rule has to survive: after the collapse
                // this triangle must still have a vertex of its own slot to
                // lead with.
                if !self.vertex_slot.is_empty() {
                    let slot = self.triangle_slot[*triangle as usize];
                    let leads = corners.iter().any(|vertex| {
                        let vertex = if *vertex == remove { keep } else { *vertex };
                        self.vertex_slot[vertex as usize] == slot
                    });
                    if !leads {
                        return Err(Refusal::Paint);
                    }
                }
            }
        }

        // ---- the other direction
        //
        // Everything above asks whether the NEW surface is near the sculpt.
        // This asks whether the sculpt is still near the new surface. Some
        // points of it are in hand: where the two vertices stand now, and the
        // points each is already answering for. Every one must lie within the
        // threshold of what replaces the edge, and the few that end up
        // furthest out are what the merged vertex answers for from here on.
        let here = [self.positions[remove as usize], self.positions[keep as usize]];
        let mut kept = [(position, -1.0_f32); WITNESSES];
        let mut ask = |work: &Self, point: Vec3, of: usize| -> Result<(), Refusal> {
            // A witness that is still the vertex's own position was asked
            // about already, as that position.
            if of < 2 && point == here[of] && kept.iter().any(|(held, _)| *held == point) {
                return Ok(());
            }
            let lost =
                work.distance_to_patch(point, remove, keep, position, around_remove, around_keep);
            if lost > threshold {
                return Err(Refusal::Field);
            }
            // Keep the furthest few, without holding one point twice.
            if kept.iter().any(|(held, _)| *held == point) {
                return Ok(());
            }
            if let Some(nearest) =
                kept.iter_mut().min_by(|a, b| a.1.total_cmp(&b.1)).filter(|slot| slot.1 < lost)
            {
                *nearest = (point, lost);
            }
            Ok(())
        };
        ask(self, here[0], 0)?;
        for point in self.witness[remove as usize] {
            ask(self, point, 0)?;
        }
        // A kept vertex that does not move is still on its own triangles,
        // but what it answers for may have had a reshaped triangle nearest.
        if !fixed {
            ask(self, here[1], 1)?;
        }
        for point in self.witness[keep as usize] {
            ask(self, point, 1)?;
        }
        let witness = kept.map(|(point, _)| point);

        // And the neighbours' witnesses. A collapse reshapes triangles that
        // belong to the vertices round the edge as well, and each of those
        // vertices may be answering for a point that one of the reshaped
        // triangles was the nearest thing to. Leaving these unasked was
        // measured: the cube's corners came back 0.47 of a voxel out against
        // a tolerance of 0.3, lost not by the collapse that merged them but by
        // a later one next door.
        //
        // A witness that is still its vertex's own position is skipped. It is
        // one of that vertex's triangles' corners and stays one.
        for triangle in around_keep {
            for vertex in self.triangles[*triangle as usize] {
                if vertex != remove && vertex != keep && !ring.contains(&vertex) {
                    ring.push(vertex);
                }
            }
        }
        for neighbour in ring.iter() {
            for point in self.witness[*neighbour as usize] {
                if point == self.positions[*neighbour as usize] {
                    continue;
                }
                let mut nearest = f32::INFINITY;
                let mut corner = self.head[*neighbour as usize];
                while corner != NONE {
                    let corners = self.triangles[(corner / 3) as usize];
                    corner = self.next[corner as usize];
                    if corners[0] == DEAD || (corners.contains(&remove) && corners.contains(&keep))
                    {
                        continue;
                    }
                    let after = corners.map(|vertex| {
                        if vertex == remove || vertex == keep {
                            position
                        } else {
                            self.positions[vertex as usize]
                        }
                    });
                    nearest = nearest.min(distance_to_triangle(point, after));
                }
                if nearest > threshold {
                    return Err(Refusal::Field);
                }
            }
        }

        // ---- the field's verdict, last because it is the dear one
        //
        // Each reshaped triangle: its middle within the threshold and facing
        // the way the sculpt faces there, and every other sample across it
        // within the threshold too.
        for (list, moved) in [(around_remove, remove), (around_keep, keep)] {
            if fixed && moved == keep {
                continue;
            }
            for triangle in list {
                let corners = self.triangles[*triangle as usize];
                if corners.contains(&remove) && corners.contains(&keep) {
                    continue;
                }
                let new = corners.map(|vertex| {
                    if vertex == moved { position } else { self.positions[vertex as usize] }
                });
                let new_normal = (new[1] - new[0]).cross(new[2] - new[0]);
                let centre = self.sampler.read((new[0] + new[1] + new[2]) / 3.0);
                if centre.deviation > threshold
                    || centre.outward.dot(new_normal) < MIN_AGREEMENT * new_normal.length()
                {
                    return Err(Refusal::Field);
                }
                if self.furthest_sample(new, threshold) > threshold {
                    return Err(Refusal::Field);
                }
            }
        }

        Ok(Collapse { remove, keep, position, witness })
    }

    /// How far `point` is from the triangles round an edge as they will be
    /// once it has collapsed to `position`.
    fn distance_to_patch(
        &self,
        point: Vec3,
        remove: u32,
        keep: u32,
        position: Vec3,
        around_remove: &[u32],
        around_keep: &[u32],
    ) -> f32 {
        let mut nearest = f32::INFINITY;
        for triangle in around_remove.iter().chain(around_keep) {
            let corners = self.triangles[*triangle as usize];
            // The two on the edge itself are the ones that go.
            if corners.contains(&remove) && corners.contains(&keep) {
                continue;
            }
            let after = corners.map(|vertex| {
                if vertex == remove || vertex == keep {
                    position
                } else {
                    self.positions[vertex as usize]
                }
            });
            nearest = nearest.min(distance_to_triangle(point, after));
        }
        nearest
    }

    /// Where the vertex an edge collapses to should sit.
    ///
    /// The point that minimises the summed squared distance to the planes of
    /// every triangle round the edge, which is Garland and Heckbert's quadric
    /// built from the mesh as it is now and stored nowhere, then pulled onto
    /// the field's own surface by one Newton step.
    ///
    /// On a flat or a straight crease the minimiser is a whole plane or line
    /// of points and the system is singular. A small pull toward the edge's
    /// midpoint makes it well posed everywhere and, where the planes do pin
    /// the point down, moves it by a negligible amount.
    fn best_position(
        &mut self,
        remove: u32,
        keep: u32,
        around_remove: &[u32],
        around_keep: &[u32],
    ) -> Vec3 {
        let ends = [self.positions[remove as usize], self.positions[keep as usize]];
        let midpoint = (ends[0] + ends[1]) * 0.5;
        let origin = midpoint.as_dvec3();

        // Accumulated relative to the midpoint and in f64: the planes are
        // millimetres from the origin and the answer is wanted to microns.
        let mut a = DMat3::ZERO;
        let mut b = DVec3::ZERO;
        let mut weight = 0.0;
        for triangle in around_remove.iter().chain(around_keep) {
            let [p, q, r] = self.triangles[*triangle as usize]
                .map(|vertex| self.positions[vertex as usize].as_dvec3() - origin);
            let cross = (q - p).cross(r - p);
            let area = cross.length();
            if area <= 1.0e-18 {
                continue;
            }
            let normal = cross / area;
            let offset = normal.dot(p);
            // The two triangles on the edge are in both lists and counted
            // twice. They are the ones about to go, and the weight is a
            // weighting, so nothing is owed for that.
            a += DMat3::from_cols(normal * normal.x, normal * normal.y, normal * normal.z) * area;
            b += normal * (offset * area);
            weight += area;
        }
        if weight <= 0.0 {
            return midpoint;
        }
        let pull = weight * 1.0e-3;
        let system = a + DMat3::from_diagonal(DVec3::splat(pull));
        let determinant = system.determinant();
        let mut candidate = midpoint;
        if determinant.abs() > 1.0e-30 {
            let solved = system.inverse() * b;
            // A solution far from the edge it replaces is the system being
            // nearly singular in a direction the pull did not hold.
            let reach = f64::from(ends[0].distance(ends[1])) * 2.0;
            if solved.is_finite() && solved.length() <= reach {
                candidate = (origin + solved).as_vec3();
            }
        }

        // Onto the sculpt itself. The quadric knows the triangles; the field
        // knows the surface they were cut from.
        let reading = self.sampler.read(candidate);
        if reading.signed.is_finite() && reading.outward != Vec3::ZERO {
            candidate -= reading.outward * reading.signed;
        }
        candidate
    }

    fn apply(&mut self, collapse: Collapse, sweep: i32) {
        let Collapse { remove, keep, position, witness } = collapse;
        self.positions[keep as usize] = position;
        self.witness[keep as usize] = witness;

        let mut corner = self.head[remove as usize];
        self.head[remove as usize] = NONE;
        while corner != NONE {
            let following = self.next[corner as usize];
            let index = (corner / 3) as usize;
            let triangle = self.triangles[index];
            if triangle[0] != DEAD {
                for vertex in triangle {
                    self.changed[vertex as usize] = sweep;
                }
                if triangle.contains(&keep) {
                    self.triangles[index][0] = DEAD;
                    self.alive_triangles -= 1;
                } else {
                    self.triangles[index][(corner % 3) as usize] = keep;
                    self.next[corner as usize] = self.head[keep as usize];
                    self.head[keep as usize] = corner;
                }
            }
            corner = following;
        }
        // And everything that was already round the kept vertex has a
        // neighbour that moved.
        let mut corner = self.head[keep as usize];
        while corner != NONE {
            let triangle = self.triangles[(corner / 3) as usize];
            if triangle[0] != DEAD {
                for vertex in triangle {
                    self.changed[vertex as usize] = sweep;
                }
            }
            corner = self.next[corner as usize];
        }
        self.alive_vertices -= 1;
    }

    /// Pack what is left into a mesh, and measure it.
    fn finish(mut self, report: &mut DecimateReport) -> ExportMesh {
        let painted = !self.vertex_slot.is_empty();
        let mut remap = vec![NONE; self.positions.len()];
        let mut mesh = ExportMesh::default();
        let mut normal_sums: Vec<Vec3> = Vec::new();
        let (worst, measured) = self.measure();

        for (index, triangle) in self.triangles.iter().enumerate() {
            if triangle[0] == DEAD {
                continue;
            }
            let mut corners = *triangle;
            if painted {
                // Lead with a vertex of the triangle's own slot. `judge`
                // refused every collapse that would have left none.
                let slot = self.triangle_slot[index];
                if let Some(first) =
                    corners.iter().position(|vertex| self.vertex_slot[*vertex as usize] == slot)
                {
                    corners.rotate_left(first);
                }
            }
            let points = corners.map(|vertex| self.positions[vertex as usize]);
            let face = (points[1] - points[0]).cross(points[2] - points[0]);
            let packed = corners.map(|vertex| {
                if remap[vertex as usize] == NONE {
                    remap[vertex as usize] = mesh.positions.len() as u32;
                    mesh.positions.push(self.positions[vertex as usize]);
                    if painted {
                        mesh.slots.push(self.vertex_slot[vertex as usize]);
                    }
                    normal_sums.push(Vec3::ZERO);
                }
                remap[vertex as usize]
            });
            for vertex in packed {
                normal_sums[vertex as usize] += face;
            }
            mesh.triangles.push(packed);
        }

        // The field's own gradient is the normal where it has one: it is the
        // sculpt's, where the mean of a few large facets is the mesh's.
        mesh.normals = mesh
            .positions
            .iter()
            .zip(normal_sums)
            .map(|(position, sum)| {
                let reading = self.sampler.read(*position);
                if reading.outward != Vec3::ZERO {
                    reading.outward
                } else {
                    sum.try_normalize().unwrap_or(Vec3::Y)
                }
            })
            .collect();

        report.triangles_after = mesh.triangles.len();
        report.vertices_after = mesh.positions.len();
        report.max_deviation_mm = worst;
        report.measured_triangles = measured;
        mesh
    }
}

/// Distance from a point to a triangle: Ericson's closest point, which is a
/// page of cases.
fn distance_to_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> f32 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return p.distance(a);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return p.distance(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return p.distance(a + ab * (d1 / (d1 - d3)));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return p.distance(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return p.distance(a + ac * (d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return p.distance(b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denominator = 1.0 / (va + vb + vc);
    p.distance(a + ab * (vb * denominator) + ac * (vc * denominator))
}

/// Shape of a triangle from its corners and the length of its unnormalised
/// normal: 1 for equilateral, 0 for a line.
fn quality(points: [Vec3; 3], normal_length: f32) -> f32 {
    let edges = (points[1] - points[0]).length_squared()
        + (points[2] - points[1]).length_squared()
        + (points[0] - points[2]).length_squared();
    if edges <= 0.0 {
        return 0.0;
    }
    // Twice the area is the normal's length, and an equilateral triangle of
    // side `s` has area `sqrt(3) / 4 * s^2` against `3 s^2` of squared edge.
    2.0 * 3.0_f32.sqrt() * normal_length / edges
}

fn quality_of(points: [Vec3; 3], normal_length: f32) -> f32 {
    quality(points, normal_length)
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashMap;

    use super::*;
    use crate::brick::{INSIDE, OUTSIDE};
    use crate::brush::{Brush, BrushDirection, BrushKind, BrushScratch, Stamp};
    use crate::clip::ClipPlane;
    use crate::pattern::{Pattern, PatternKind};

    const CENTRE: Vec3 = Vec3::splat(40.0);

    fn ball(radius: f32, voxel: f32) -> Volume {
        let mut volume = Volume::new(voxel);
        volume.seed_sphere(CENTRE, radius);
        volume
    }

    /// An axis aligned cube, as an exact distance field.
    fn cube(half: f32, voxel: f32) -> Volume {
        let mut volume = Volume::new(voxel);
        let reach = Vec3::splat(half + NARROW_BAND * voxel * 2.0);
        volume.edit_box(CENTRE - reach, CENTRE + reach, move |position, _| {
            let outside = (position - CENTRE).abs() - Vec3::splat(half);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / voxel).clamp(INSIDE, OUTSIDE)
        });
        volume
    }

    /// A ball roughened with a pattern, so the surface has detail at the scale
    /// of a few voxels and a field the brushes have been through.
    fn rough_ball() -> Volume {
        let mut volume = ball(24.0, 1.0);
        let brush = Brush {
            kind: BrushKind::Draw,
            radius: 12.0,
            strength: 0.9,
            pattern: Pattern { kind: PatternKind::Scales, scale_mm: 6.0, depth: 1.0 },
            ..Brush::default()
        };
        let mut scratch = BrushScratch::new();
        for towards in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, Vec3::new(0.6, 0.6, 0.5)] {
            let towards = towards.normalize();
            let stamp = Stamp::new(CENTRE + towards * 24.0, towards, BrushDirection::Add);
            for _ in 0..4 {
                brush.apply(&mut volume, &stamp, &mut scratch);
            }
        }
        volume
    }

    /// The worst shaped triangle in a mesh, on [`quality`]'s scale.
    fn worst_quality(mesh: &ExportMesh) -> f32 {
        mesh.triangles
            .iter()
            .map(|triangle| {
                let points = triangle.map(|vertex| mesh.positions[vertex as usize]);
                let normal = (points[1] - points[0]).cross(points[2] - points[0]);
                quality(points, normal.length())
            })
            .fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn no_sliver_is_made_that_was_not_there() {
        // A needle is still a closed surface and still prints, and it is one
        // rounding error from a triangle of no area, which is the defect the
        // build specification names. Flat faces are where they get made: on
        // a plane nothing about the field objects to a triangle forty voxels
        // long and a hair wide.
        for (volume, what) in [(cube(20.0, 1.0), "a cube"), (cone(20.0).0, "a cone")] {
            let (dense, _) = volume.export_mesh();
            let floor = MIN_QUALITY.min(worst_quality(&dense));
            for tolerance in [0.1, 0.3, 1.0] {
                let (mesh, _) = dense.decimated(&volume, DecimateSpec::within(tolerance));
                let worst = worst_quality(&mesh);
                // A hundredth of slack: the full mesh's own worst triangle is
                // moved a hair when its vertices are stood on the field.
                assert!(
                    worst >= floor * 0.99,
                    "{what} at {tolerance}: the worst triangle is {worst}, below {floor}"
                );
                assert_eq!(mesh.validate().zero_area_triangles, 0);
            }
        }
    }

    /// Vertices minus edges plus faces, which is 2 for a ball and drops by 2
    /// for every handle.
    fn euler(mesh: &ExportMesh) -> i64 {
        let mut edges: FxHashMap<(u32, u32), ()> = FxHashMap::default();
        for [a, b, c] in &mesh.triangles {
            for (from, to) in [(a, b), (b, c), (c, a)] {
                edges.insert((*from.min(to), *from.max(to)), ());
            }
        }
        mesh.positions.len() as i64 - edges.len() as i64 + mesh.triangles.len() as i64
    }

    /// The furthest any of `points` lies from `mesh`, by brute force.
    ///
    /// The reverse of what the decimator checks. It measures the result
    /// against the field; this measures the ORIGINAL against the result, with
    /// no help from anything the decimator computed, and is what would see a
    /// feature that had been cut away.
    fn furthest_from(mesh: &ExportMesh, points: &[Vec3]) -> f32 {
        let corners: Vec<[Vec3; 3]> =
            mesh.triangles.iter().map(|t| t.map(|v| mesh.positions[v as usize])).collect();
        points
            .iter()
            .map(|point| {
                corners
                    .iter()
                    .map(|triangle| distance_to_triangle(*point, *triangle))
                    .fold(f32::INFINITY, f32::min)
            })
            .fold(0.0, f32::max)
    }

    /// The furthest any point of `mesh` reads from the field, sampled every
    /// half voxel across every triangle through [`Volume::sample_world`].
    ///
    /// The forward direction again, but not the decimator's own measurement
    /// of it: a finer lattice than it samples on, through the volume's public
    /// reader and not its cached one, and the field's RAW value with no
    /// correction for its slope. The decimator's own reading is never below
    /// the raw one, so a result it judged inside the tolerance has to be
    /// inside it here as well. A triangle the decimator sampled too coarsely
    /// to see a bump under is seen here.
    fn furthest_from_the_field(mesh: &ExportMesh, volume: &Volume) -> f32 {
        let voxel = volume.voxel_size();
        let step = voxel * 0.5;
        let mut worst: f32 = 0.0;
        for triangle in &mesh.triangles {
            let [a, b, c] = triangle.map(|vertex| mesh.positions[vertex as usize]);
            let longest = (b - a).length().max((c - b).length()).max((a - c).length());
            let steps = ((longest / step).ceil() as usize).clamp(1, 64);
            for i in 0..=steps {
                for j in 0..=(steps - i) {
                    let k = steps - i - j;
                    let at = (a * i as f32 + b * j as f32 + c * k as f32) / steps as f32;
                    worst = worst.max(volume.sample_world(at).abs() * voxel);
                }
            }
        }
        worst
    }

    /// Every `every`th vertex of the full mesh as the decimator sees it once
    /// it has stood each one on the field: the points of the sculpt that the
    /// result has to stay near.
    ///
    /// Not the mesher's own positions, which are up to a third of a voxel off
    /// the field (see `settle_onto_the_field`) and so would charge the
    /// decimator with a discrepancy it did not make.
    fn sculpt_points(dense: &ExportMesh, volume: &Volume, every: usize) -> Vec<Vec3> {
        let mut work = Simplifier::new(dense, volume, 1.0);
        work.settle_onto_the_field();
        work.positions.iter().step_by(every).copied().collect()
    }

    #[test]
    fn a_ball_keeps_its_shape_and_loses_most_of_its_triangles() {
        let volume = ball(24.0, 1.0);
        let (dense, dense_report) = volume.export_mesh();
        assert!(dense_report.is_printable());

        let tolerance = 0.2;
        let (mesh, mesh_report, report) =
            volume.export_mesh_decimated(DecimateSpec::within(tolerance));
        assert!(
            !report.fell_back,
            "the decimated ball did not validate: {}",
            mesh_report.summary()
        );
        assert!(mesh_report.is_printable(), "{}", mesh_report.summary());
        assert_eq!(report.triangles_after, mesh.triangles.len());
        assert!(
            mesh.triangles.len() * 4 < dense.triangles.len(),
            "{} triangles from {}: a ball at a fifth of a voxel should lose three quarters",
            mesh.triangles.len(),
            dense.triangles.len()
        );

        // Forward: the result against the sculpt, by the field and by the
        // ball's own equation, which the field's errors cannot flatter.
        assert_eq!(report.measured_triangles, mesh.triangles.len(), "not every triangle was read");
        assert!(
            report.max_deviation_mm <= tolerance,
            "measured {} against a tolerance of {tolerance}",
            report.max_deviation_mm
        );
        // And inside the share of it the pass works to, which is the room
        // left for what sampling at points cannot see. The full mesh of a
        // plain ball is nowhere near the tolerance, so this is the pass's own
        // doing.
        assert!(report.dense_deviation_mm < tolerance * 0.5);
        // The share is written out and not read from the constant, so that
        // changing the constant has to change this line as well.
        assert!(
            report.max_deviation_mm <= tolerance * 0.9 + 1.0e-4,
            "measured {}, which is past the nine tenths of {tolerance} the pass works to",
            report.max_deviation_mm
        );
        let independent = furthest_from_the_field(&mesh, &volume);
        assert!(independent <= tolerance, "an independent reading finds {independent}");
        for position in &mesh.positions {
            let off = (position.distance(CENTRE) - 24.0).abs();
            assert!(off <= tolerance + 0.05, "a vertex is {off} from the ball");
        }
        // Reverse: the sculpt against the result.
        let reverse = furthest_from(&mesh, &sculpt_points(&dense, &volume, 3));
        assert!(reverse <= tolerance * 1.02, "the sculpt is {reverse} from the result");

        assert_eq!(euler(&mesh), 2, "a ball came back with handles");
        assert_eq!(mesh.normals.len(), mesh.positions.len());
        for (position, normal) in mesh.positions.iter().zip(&mesh.normals) {
            let outward = (*position - CENTRE).normalize();
            assert!(
                normal.dot(outward) > 0.95,
                "a normal points {} off radial",
                normal.dot(outward)
            );
        }
    }

    #[test]
    fn a_cube_goes_down_to_a_cube_with_its_corners_on() {
        // The case the feature exists for: a cut face meshed one vertex a
        // voxel. A cube is six of them, and it is 18,252 triangles as it
        // leaves the mesher. Measured: 12 at a tolerance of 0.3 of a voxel.
        //
        // **It was 12 once before, and that 12 was wrong**: a corner 0.84 of
        // a voxel from where it belonged, with every check passing. The
        // module documentation has the history. What makes this 12 different
        // is the last third of this test.
        let volume = cube(20.0, 1.0);
        let (dense, _) = volume.export_mesh();
        let tolerance = 0.3;
        let (mesh, mesh_report, report) =
            volume.export_mesh_decimated(DecimateSpec::within(tolerance));
        assert!(mesh_report.is_printable(), "{}", mesh_report.summary());
        assert!(
            mesh.triangles.len() <= 24,
            "{} triangles from {}: the faces of a cube are flat",
            mesh.triangles.len(),
            dense.triangles.len()
        );
        assert!(report.max_deviation_mm <= tolerance, "measured {}", report.max_deviation_mm);
        assert_eq!(euler(&mesh), 2);

        // The reverse direction, which is the one that sees a corner cut
        // off: every vertex of the full mesh, stood on the field, against the
        // result.
        let reverse = furthest_from(&mesh, &sculpt_points(&dense, &volume, 1));
        assert!(reverse <= tolerance * 1.02, "the sculpt is {reverse} from the result");

        // And against the cube itself: how far the nearest vertex is from
        // each true corner, at its worst. The full mesh does not get close,
        // because surface nets rounds a corner off; the result has to.
        let worst_corner = |positions: &[Vec3]| {
            (0..8)
                .map(|corner| {
                    let sign = Vec3::new(
                        if corner & 1 == 0 { -1.0 } else { 1.0 },
                        if corner & 2 == 0 { -1.0 } else { 1.0 },
                        if corner & 4 == 0 { -1.0 } else { 1.0 },
                    );
                    let at = CENTRE + sign * 20.0;
                    positions.iter().map(|p| p.distance(at)).fold(f32::INFINITY, f32::min)
                })
                .fold(0.0, f32::max)
        };
        let full = worst_corner(&dense.positions);
        let result = worst_corner(&mesh.positions);
        assert!(full > 1.0, "the full mesh reaches within {full} of every corner");
        assert!(
            result < 0.5,
            "the result's worst corner is {result} of a voxel from the cube's (the full mesh: {full})"
        );
    }

    #[test]
    fn a_tight_tolerance_keeps_the_rounding_the_field_has_on_an_edge() {
        // The other half of the same fixture, and the reason the last test
        // does not ask for less than 0.3. The field does not hold a sharp
        // edge: it rounds one over about a voxel. A tolerance tighter than
        // that rounding is a request to reproduce it, and the decimator does:
        // the flats still go, and the twelve edges keep the triangles their
        // curve needs.
        let volume = cube(20.0, 1.0);
        let (dense, _) = volume.export_mesh();
        let (loose, _, _) = volume.export_mesh_decimated(DecimateSpec::within(0.3));
        let (tight, tight_report, report) = volume.export_mesh_decimated(DecimateSpec::within(0.1));
        assert!(tight_report.is_printable());
        assert!(
            tight.triangles.len() > loose.triangles.len() * 10,
            "at a third of the tolerance the cube kept only {} triangles against {}",
            tight.triangles.len(),
            loose.triangles.len()
        );
        assert!(tight.triangles.len() * 4 < dense.triangles.len(), "the flats did not go");
        // And this is the fixture where the full mesh is itself further from
        // the field than the tolerance asked for, on those edges.
        assert!(report.dense_deviation_mm > 0.1, "{}", report.dense_deviation_mm);
        assert!(report.max_deviation_mm <= report.dense_deviation_mm);
        assert!(report.summary().contains("already further"), "{}", report.summary());
    }

    #[test]
    fn a_sculpted_surface_stays_within_the_tolerance_both_ways() {
        let volume = rough_ball();
        let (dense, _) = volume.export_mesh();
        // What the full mesh itself reads, by the same independent reading
        // the results get below: the floor no tolerance can get under.
        let settled = ExportMesh { positions: sculpt_points(&dense, &volume, 1), ..dense.clone() };
        let full_mesh_independently = furthest_from_the_field(&settled, &volume);
        for tolerance in [0.05, 0.25, 1.0] {
            let (mesh, mesh_report, report) =
                volume.export_mesh_decimated(DecimateSpec::within(tolerance));
            assert!(mesh_report.is_printable(), "at {tolerance}: {}", mesh_report.summary());
            assert!(!report.fell_back);
            assert!(mesh.triangles.len() < dense.triangles.len(), "at {tolerance} nothing went");
            // A triangle a voxel across is already this far from a rough
            // surface in places, before anything is collapsed; the tightest
            // tolerance here is tighter than that, which is the case the
            // report's second number exists for.
            let bound = tolerance.max(report.dense_deviation_mm);
            assert!(
                report.max_deviation_mm <= bound,
                "at {tolerance} the result measured {} against a full mesh at {}",
                report.max_deviation_mm,
                report.dense_deviation_mm
            );
            // The same direction read independently, and finer than the
            // decimator samples. This is what a triangle laid over a bump it
            // was never sampled on fails.
            let independent = furthest_from_the_field(&mesh, &volume);
            // A hair over the full mesh's own figure: where nothing could be
            // collapsed the result IS the full mesh, but a triangle whose far
            // corner moved is sampled at slightly different points.
            let allowed = tolerance.max(full_mesh_independently * 1.01);
            assert!(
                independent <= allowed,
                "at {tolerance} an independent reading finds the result {independent} from the \
                 field, against {allowed}"
            );
            let reverse = furthest_from(&mesh, &sculpt_points(&dense, &volume, 3));
            // Held by witnesses and not by the field, so it is allowed a
            // fiftieth more. Measured at 91% to 93% across these.
            assert!(
                reverse <= tolerance * 1.02,
                "at {tolerance} the sculpt is {reverse} from the result"
            );
            assert_eq!(euler(&mesh), 2, "at {tolerance}");
        }
    }

    #[test]
    fn a_looser_tolerance_takes_more() {
        let volume = rough_ball();
        let mut last = usize::MAX;
        for tolerance in [0.02, 0.1, 0.4, 1.0] {
            let (mesh, _, _) = volume.export_mesh_decimated(DecimateSpec::within(tolerance));
            assert!(
                mesh.triangles.len() < last,
                "a tolerance of {tolerance} left {} triangles, no fewer than a tighter one",
                mesh.triangles.len()
            );
            last = mesh.triangles.len();
        }
    }

    #[test]
    fn the_tolerance_is_clamped_to_what_the_field_can_check() {
        let volume = ball(24.0, 0.5);
        let (_, _, report) = volume.export_mesh_decimated(DecimateSpec::within(50.0));
        assert_eq!(report.tolerance_mm, MAX_TOLERANCE_VOXELS * 0.5);
        assert!(report.max_deviation_mm <= report.tolerance_mm);
    }

    #[test]
    fn no_tolerance_is_no_change() {
        let volume = ball(12.0, 1.0);
        let (dense, _) = volume.export_mesh();
        for tolerance in [0.0, -1.0, f32::NAN] {
            let (mesh, report) = dense.decimated(&volume, DecimateSpec::within(tolerance));
            assert_eq!(mesh.triangles, dense.triangles);
            assert_eq!(mesh.positions, dense.positions);
            assert_eq!(report.triangles_after, report.triangles_before);
        }
    }

    #[test]
    fn a_target_stops_it_early() {
        let volume = ball(24.0, 1.0);
        let (dense, _) = volume.export_mesh();
        let target = dense.triangles.len() / 2;
        let (mesh, mesh_report, report) = volume.export_mesh_decimated(DecimateSpec {
            tolerance_mm: 1.0,
            target_triangles: Some(target),
        });
        assert!(mesh_report.is_printable());
        assert!(
            mesh.triangles.len() <= target,
            "{} against a target of {target}",
            mesh.triangles.len()
        );
        // It stops AT the target and does not run on to what the tolerance
        // would have allowed, which is far fewer.
        assert!(
            mesh.triangles.len() + 4 > target,
            "it ran {} past the target",
            target - mesh.triangles.len()
        );
        assert_eq!(report.triangles_after, mesh.triangles.len());
    }

    #[test]
    fn a_through_hole_is_still_a_through_hole() {
        // A ball with a square hole right through it has one handle. Closing
        // it, or pinching it, changes the count.
        let mut volume = ball(24.0, 1.0);
        let through = [
            ClipPlane { point: CENTRE - Vec3::X * 5.0, normal: Vec3::X },
            ClipPlane { point: CENTRE + Vec3::X * 5.0, normal: -Vec3::X },
            ClipPlane { point: CENTRE - Vec3::Y * 5.0, normal: Vec3::Y },
            ClipPlane { point: CENTRE + Vec3::Y * 5.0, normal: -Vec3::Y },
        ];
        volume.clip_convex(&through);
        let (dense, dense_report) = volume.export_mesh();
        assert!(dense_report.is_printable());
        assert_eq!(euler(&dense), 0, "the fixture is not a ball with one hole through it");

        let (mesh, mesh_report, report) = volume.export_mesh_decimated(DecimateSpec::within(0.5));
        assert!(mesh_report.is_printable(), "{}", mesh_report.summary());
        assert!(mesh.triangles.len() * 3 < dense.triangles.len());
        assert_eq!(euler(&mesh), 0, "the hole was closed or pinched");
        assert!(report.refused_by_topology + report.refused_by_field + report.refused_by_shape > 0);
    }

    /// A cone standing on a flat base, as an exact distance field, and where
    /// its tip is.
    fn cone(half_angle_degrees: f32) -> (Volume, Vec3) {
        // Off the lattice on every axis, so nothing about the tip is helped
        // by landing on a voxel.
        let apex = Vec3::new(40.3, 58.4, 40.6);
        let (sin, cos) = half_angle_degrees.to_radians().sin_cos();
        let mut volume = Volume::new(1.0);
        volume.edit_box(Vec3::splat(4.0), Vec3::splat(76.0), move |position, _| {
            let down = apex.y - position.y;
            let radial = Vec3::new(position.x - apex.x, 0.0, position.z - apex.z).length();
            // Nearest the slanted side, or nearest the tip itself.
            let side = if radial * sin + down * cos >= 0.0 {
                radial * cos - down * sin
            } else {
                position.distance(apex)
            };
            side.max(22.0 - position.y).clamp(INSIDE, OUTSIDE)
        });
        (volume, apex)
    }

    #[test]
    fn the_tip_of_a_cone_is_not_whittled_away() {
        // The fixture the reverse direction is for, and the one that showed
        // it was needed after a cube and two balls had said it was not.
        //
        // A sharp tip is where a surface can stay close to the sculpt
        // everywhere and still lose some of it: lop the top off a cone and
        // the cut is a small disc, every point of it a fraction of the
        // cone's width from the cone's side. Measured, as the furthest any
        // point of the sculpt lay from the result, over these nine cases:
        // at most 95% of the tolerance as written here, and up to 151% with
        // the witnesses not asked.
        for half_angle in [12.0, 20.0, 35.0] {
            let (volume, _) = cone(half_angle);
            let (dense, dense_report) = volume.export_mesh();
            assert!(
                dense_report.is_printable(),
                "the {half_angle} degree cone does not mesh closed"
            );
            let sculpt = sculpt_points(&dense, &volume, 1);
            for tolerance in [0.2, 0.5, 1.0] {
                let (mesh, report) = dense.decimated(&volume, DecimateSpec::within(tolerance));
                assert!(mesh.validate().is_printable());
                assert!(mesh.triangles.len() * 4 < dense.triangles.len());
                assert_eq!(euler(&mesh), 2);
                let reverse = furthest_from(&mesh, &sculpt);
                assert!(
                    reverse <= tolerance * 1.02,
                    "a {half_angle} degree cone at {tolerance}: the sculpt is {reverse} from the \
                     result"
                );
                assert!(report.max_deviation_mm <= tolerance.max(report.dense_deviation_mm));
            }
        }
    }

    /// A closed mesh with no field under it, for asking the topology rules a
    /// question directly. Every read of the field comes back off the band,
    /// so a collapse that gets PAST the topology rules is refused by the
    /// field, and one refused for its topology was refused by nothing else.
    fn judged_in_empty_space(
        positions: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
        edge: (u32, u32),
    ) -> Option<Refusal> {
        let mesh = ExportMesh {
            normals: vec![Vec3::Y; positions.len()],
            positions,
            triangles,
            slots: Vec::new(),
        };
        assert!(mesh.validate().is_printable(), "the fixture is not a closed manifold");
        let volume = Volume::new(1.0);
        let mut work = Simplifier::new(&mesh, &volume, 1.0);
        work.witness = work.positions.iter().map(|position| [*position; WITNESSES]).collect();
        work.consider(edge.0, edge.1, 1.0).err()
    }

    #[test]
    fn no_edge_of_a_tetrahedron_may_go() {
        // It passes the link condition, which is why it has a rule of its
        // own: every edge has exactly two common neighbours, and collapsing
        // one leaves two triangles back to back.
        let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
        let triangles = vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]];
        for edge in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
            let refusal = judged_in_empty_space(positions.clone(), triangles.clone(), edge);
            assert!(
                refusal == Some(Refusal::Topology),
                "the edge {edge:?} of a tetrahedron was not refused for its topology"
            );
        }
    }

    #[test]
    fn an_edge_round_a_waist_may_not_go() {
        // A three sided tube with a point on each end: two rings of three
        // vertices, joined by side walls. An edge of a ring has THREE
        // neighbours common to its ends, the third vertex of its own ring and
        // one on each side of it, and collapsing it pinches the tube to a
        // line there. This is the link condition and nothing else: every
        // vertex here has four triangles or more.
        let ring = |y: f32| {
            [0.0_f32, 120.0, 240.0].map(|degrees| {
                let (sin, cos) = degrees.to_radians().sin_cos();
                Vec3::new(cos, y, sin)
            })
        };
        let mut positions = vec![Vec3::new(0.0, 2.0, 0.0)];
        positions.extend(ring(1.0));
        positions.extend(ring(-1.0));
        positions.push(Vec3::new(0.0, -2.0, 0.0));
        // 0 is the top point, 1..=3 the upper ring, 4..=6 the lower, 7 the
        // bottom point.
        let mut triangles = Vec::new();
        for i in 0..3u32 {
            let (a, b) = (1 + i, 1 + (i + 1) % 3);
            let (c, d) = (4 + i, 4 + (i + 1) % 3);
            triangles.push([0, b, a]);
            triangles.push([a, b, d]);
            triangles.push([a, d, c]);
            triangles.push([7, c, d]);
        }
        let on_the_ring = judged_in_empty_space(positions.clone(), triangles.clone(), (1, 2));
        assert!(
            on_the_ring == Some(Refusal::Topology),
            "an edge round the waist of a tube was not refused for its topology"
        );
        // The control: an edge that runs ALONG the tube breaks no rule of
        // topology, so whatever refuses it here is the empty field.
        let along = judged_in_empty_space(positions, triangles, (1, 4));
        assert!(
            along == Some(Refusal::Field),
            "an edge along the tube should have got as far as the field and been refused there"
        );
    }

    #[test]
    fn a_thin_rod_is_not_pinched_shut() {
        // A rod little more than two voxels thick, at the loosest tolerance
        // there is, where the tolerance is most of its radius. Whatever is
        // refused and for whatever reason, what comes back has to be a closed
        // surface with no handles and no triangle of no area. The rules of
        // topology themselves are asked directly in the two tests above;
        // this is the whole pass on the shape most likely to break them.
        let mut refused_for_topology = 0;
        for radius in [1.1_f32, 1.3, 1.6] {
            let mut volume = Volume::new(1.0);
            let (from, to) = (Vec3::new(20.3, 40.2, 40.4), Vec3::new(60.1, 41.0, 39.7));
            volume.edit_box(Vec3::splat(8.0), Vec3::splat(72.0), |position, _| {
                let along = (to - from).normalize();
                let t = (position - from).dot(along).clamp(0.0, from.distance(to));
                (position.distance(from + along * t) - radius).clamp(INSIDE, OUTSIDE)
            });
            let (dense, dense_report) = volume.export_mesh();
            assert!(dense_report.is_printable(), "the rod of radius {radius} does not mesh closed");
            assert_eq!(euler(&dense), 2);

            let (mesh, report) = dense.decimated(&volume, DecimateSpec::within(1.0));
            let mesh_report = mesh.validate();
            assert!(
                mesh_report.is_printable(),
                "a rod of radius {radius} came back unprintable: {}",
                mesh_report.summary()
            );
            assert_eq!(euler(&mesh), 2, "a rod of radius {radius} changed its topology");
            assert_eq!(mesh_report.zero_area_triangles, 0);
            assert!(
                mesh.triangles.len() < dense.triangles.len(),
                "nothing of the rod of radius {radius} was decimated"
            );
            refused_for_topology += report.refused_by_topology;
        }
        // The control. If nothing on any of them was refused for its
        // topology, the field is doing all the work and these rods would
        // pass with the topology checks deleted.
        assert!(
            refused_for_topology > 0,
            "no collapse on any rod was refused for its topology, so this fixture is not testing \
             what it says"
        );
    }

    #[test]
    fn a_spike_is_not_sliced_off_at_its_base() {
        // A fin three voxels thick standing ten proud of a flat. Its base is
        // narrower than twice the tolerance, so a triangle laid across the
        // base is within the tolerance of the fin's own walls at every point
        // the distance check looks at. Only the agreement with the gradient
        // refuses it.
        let mut volume = Volume::new(1.0);
        volume.edit_box(Vec3::splat(4.0), Vec3::splat(76.0), |position, _| {
            let slab = position.y - 30.0;
            let across = (position.x - 40.0).abs() - 1.5;
            let along = (position.z - 40.0).abs() - 14.0;
            let up = position.y - 40.0;
            let fin = across.max(along).max(up);
            slab.min(fin).clamp(INSIDE, OUTSIDE)
        });
        // The slab's own rim, where the box ends, is not printable as it
        // stands, so the mesh is taken through `decimated` directly and the
        // claim is about the fin alone.
        let (dense, _) = volume.export_mesh();
        let (mesh, report) = dense.decimated(&volume, DecimateSpec::within(1.0));
        assert!(mesh.triangles.len() < dense.triangles.len(), "nothing was decimated at all");

        let tip = Vec3::new(40.0, 40.0, 40.0);
        let reverse = furthest_from(&mesh, &[tip]);
        assert!(
            reverse < 1.6,
            "the top of the fin is {reverse} from the decimated mesh: it was cut off \
             ({} field refusals)",
            report.refused_by_field
        );
    }

    #[test]
    fn a_paint_boundary_keeps_every_vertex_it_had() {
        use crate::colour::ColourField;

        let mut volume = ball(24.0, 1.0);
        // Half the ball in slot 2, written where the paint brush writes it.
        let (lo, hi) = volume.voxel_bounds(Vec3::splat(8.0), Vec3::splat(72.0));
        let mut region = crate::region::FieldRegion::new();
        volume.snapshot(lo, hi, &mut region);
        volume.edit_colour(lo, hi, |voxel, position, _, slot| {
            if position.x > CENTRE.x + 0.3 && ColourField::paintable(region.get(voxel)) {
                2
            } else {
                slot
            }
        });
        let (dense, dense_report) = volume.export_mesh();
        assert!(dense_report.is_printable());
        assert!(!dense.slots.is_empty(), "the fixture is not painted");

        let slot_of = |mesh: &ExportMesh, triangle: &[u32; 3]| mesh.slots[triangle[0] as usize];
        let area_by_slot = |mesh: &ExportMesh| {
            let mut areas = [0.0f64; 3];
            for triangle in &mesh.triangles {
                let [a, b, c] = triangle.map(|v| mesh.positions[v as usize]);
                areas[slot_of(mesh, triangle) as usize] +=
                    f64::from((b - a).cross(c - a).length()) * 0.5;
            }
            areas
        };
        // The vertices that sit on the boundary: those used by triangles of
        // both slots.
        let boundary = |mesh: &ExportMesh| {
            let mut seen: FxHashMap<u32, u8> = FxHashMap::default();
            let mut on: Vec<[u32; 3]> = Vec::new();
            for triangle in &mesh.triangles {
                let slot = slot_of(mesh, triangle);
                for vertex in triangle {
                    match seen.insert(*vertex, slot) {
                        Some(other) if other != slot => {
                            on.push(mesh.positions[*vertex as usize].to_array().map(f32::to_bits));
                        }
                        _ => {}
                    }
                }
            }
            on.sort();
            on.dedup();
            on
        };

        let (mesh, mesh_report, report) = volume.export_mesh_decimated(DecimateSpec::within(0.3));
        assert!(mesh_report.is_printable(), "{}", mesh_report.summary());
        assert!(
            mesh.triangles.len() * 2 < dense.triangles.len(),
            "painting stopped the decimation"
        );
        assert_eq!(mesh.slots.len(), mesh.positions.len());

        let before = boundary(&dense);
        assert!(before.len() > 50, "the fixture's boundary is {} vertices", before.len());
        assert_eq!(boundary(&mesh), before, "the paint boundary's vertices moved or went");

        // And each filament still covers what it covered, to within the
        // surface the tolerance lets the triangles either side flatten.
        let (was, now) = (area_by_slot(&dense), area_by_slot(&mesh));
        assert!(was[0] > 100.0 && was[2] > 100.0, "the fixture is not painted in two");
        // Each alone moves by a percent or so: flattening a ball's facets
        // loses area in proportion to the sag over the radius. What must not
        // move is the SHARE each has, which is where a boundary that had
        // drifted would show.
        for slot in [0, 2] {
            let change = (now[slot] - was[slot]).abs() / was[slot];
            assert!(change < 0.03, "slot {slot} covers {:.2}% more or less", change * 100.0);
        }
        let share = |areas: [f64; 3]| areas[2] / (areas[0] + areas[2]);
        assert!(
            (share(now) - share(was)).abs() < 0.002,
            "slot 2 had {:.3} of the surface and now has {:.3}",
            share(was),
            share(now)
        );
        assert_eq!(now[1], 0.0, "a slot nothing was painted in appeared");
        let _ = report;
    }

    #[test]
    fn a_document_decimates_each_visible_body_against_its_own_field() {
        use crate::body::Document;

        // Two bodies that are not the same shape, one of them hidden.
        let mut doc = Document::from_volume(ball(20.0, 1.0));
        let first = doc.active();
        let second = doc.add_body("cube", cube(12.0, 1.0));
        let spec = DecimateSpec::within(0.3);

        let all = vec![true; doc.node_count()];
        let (bodies, reports) = doc.export_bodies_decimated(&all, spec);
        assert_eq!(bodies.len(), 2);
        assert_eq!(reports.len(), 2);
        for ((meta, mesh, mesh_report), report) in bodies.iter().zip(&reports) {
            assert!(mesh_report.is_printable(), "{}: {}", meta.name, mesh_report.summary());
            assert_eq!(report.triangles_after, mesh.triangles.len());
            assert!(report.triangles_after * 4 < report.triangles_before, "{}", meta.name);
            assert!(report.max_deviation_mm <= 0.3);
        }
        // Each against its own field: the cube comes down to a cube, which
        // it could not if it had been judged against the ball's.
        let cube_triangles = bodies
            .iter()
            .find(|(meta, ..)| meta.id == second)
            .map(|(_, mesh, _)| mesh.triangles.len())
            .expect("the cube was exported");
        assert!(cube_triangles <= 24, "the cube came back as {cube_triangles} triangles");

        let total = DecimateReport::summed(&reports);
        assert_eq!(total.triangles_after, bodies.iter().map(|(_, m, _)| m.triangles.len()).sum());
        assert!(total.summary().contains("triangles"));

        // And a hidden body is left out, exactly as the plain export does.
        let only_first: Vec<bool> = doc.nodes().iter().map(|node| node.id == first).collect();
        let (bodies, reports) = doc.export_bodies_decimated(&only_first, spec);
        assert_eq!(bodies.len(), 1);
        assert_eq!(reports.len(), 1);
        assert_eq!(bodies[0].0.id, first);
    }

    #[test]
    fn decimating_twice_gives_the_same_mesh() {
        let volume = rough_ball();
        let (one, _, _) = volume.export_mesh_decimated(DecimateSpec::within(0.3));
        let (two, _, _) = volume.export_mesh_decimated(DecimateSpec::within(0.3));
        assert_eq!(one.triangles, two.triangles);
        assert_eq!(one.positions, two.positions);
    }

    #[test]
    fn a_decimated_mesh_that_would_not_print_is_not_the_one_returned() {
        // No volume here produces one, so the branch is handed one: a
        // tetrahedron as the dense mesh, and the same with a face missing as
        // what decimation is supposed to have made of it.
        let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
        let closed = ExportMesh {
            positions: positions.clone(),
            normals: vec![Vec3::Y; 4],
            triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
            slots: Vec::new(),
        };
        let open = ExportMesh { triangles: closed.triangles[..3].to_vec(), ..closed.clone() };
        let closed_report = closed.validate();
        assert!(closed_report.is_printable(), "the fixture's closed mesh is not closed");
        assert!(!open.validate().is_printable(), "the fixture's open mesh is not open");
        let claimed = DecimateReport {
            triangles_before: 4,
            triangles_after: 3,
            vertices_before: 4,
            vertices_after: 4,
            ..DecimateReport::default()
        };

        let (mesh, mesh_report, report) =
            keep_what_prints((closed.clone(), closed_report), open, claimed);
        assert_eq!(mesh.triangles, closed.triangles, "the mesh that would not print was returned");
        assert!(mesh_report.is_printable());
        assert!(report.fell_back);
        assert_eq!(report.triangles_after, 4, "the report still claims the decimated count");
        assert!(report.summary().contains("would not print"));

        // And the control: one that does print is the one returned.
        let (mesh, _, report) =
            keep_what_prints((closed.clone(), closed_report), closed.clone(), claimed);
        assert_eq!(mesh.triangles.len(), 4);
        assert!(!report.fell_back);
    }

    /// Not a gate. Prints what decimating costs and what it saves.
    ///
    /// ```text
    /// cargo test -p brokkr-core --release --lib the_cost_of_decimating -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "a measurement, not a check"]
    fn the_cost_of_decimating_is_printed() {
        for (radius, voxel) in [(30.0, 0.5), (30.0, 0.25), (30.0, 0.125)] {
            let volume = ball(radius, voxel);
            let started = web_time::Instant::now();
            let (dense, _) = volume.export_mesh();
            let export = started.elapsed();
            for tolerance in [0.01, 0.03, 0.1] {
                let started = web_time::Instant::now();
                let (mesh, report) = dense.decimated(&volume, DecimateSpec::within(tolerance));
                let took = started.elapsed();
                println!(
                    "ball r={radius} voxel={voxel}: {} triangles (export {:.0} ms) -> {} at {} mm \
                     ({:.1}%), measured {:.4} mm, {} sweeps, {:.0} ms, printable {}",
                    dense.triangles.len(),
                    export.as_secs_f64() * 1000.0,
                    mesh.triangles.len(),
                    report.tolerance_mm,
                    100.0 * mesh.triangles.len() as f64 / dense.triangles.len() as f64,
                    report.max_deviation_mm,
                    report.sweeps,
                    took.as_secs_f64() * 1000.0,
                    mesh.validate().is_printable()
                );
            }
        }
    }
}
