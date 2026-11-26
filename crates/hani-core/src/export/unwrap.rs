// SPDX-License-Identifier: AGPL-3.0-only

//! Unwrapping: an export mesh laid out flat, so that a picture can be put
//! on it.
//!
//! Nothing in this application works in texture coordinates. Brushes,
//! patterns and paint are all in the model's own space, and
//! `docs/STUDIO-PLAN.md` decision D1 is that this stays so: an unwrap is
//! made for one export, from the mesh that export writes, and thrown away.
//! It exists because the files a full colour printer or another program
//! takes want a picture and a place on it for every triangle.
//!
//! # How a mesh is cut up
//!
//! Into CHARTS, each a patch of surface that faces more or less one way,
//! laid flat by looking straight at it along that way.
//!
//! 1. A triangle nobody has taken starts a chart, and the way it faces is
//!    the chart's axis.
//! 2. The chart grows across shared edges, taking every triangle that faces
//!    within [`UnwrapSpec::widest_turn`] of the axis.
//! 3. Each triangle is flattened by dropping its distance along the axis.
//!
//! That is the plainest unwrap there is, and it is chosen for what it
//! guarantees and not for how few charts it makes:
//!
//! * **No triangle is flipped or folded.** Every one faces the axis, so
//!   seen along it they are all the right way round.
//! * **Stretch is bounded.** A triangle turned 60 degrees from the axis is
//!   drawn at half its width in one direction, and that is the worst. A
//!   texture is at most twice as coarse there as on a triangle seen square
//!   on.
//! * **Nothing overlaps.** Facing the axis is not quite enough for that: a
//!   ramp that winds round like a spiral stair faces up everywhere and lies
//!   over itself seen from above. So a triangle is only taken if it does
//!   not land on a part of the chart already laid down, and
//!   `no_two_triangles_of_a_chart_overlap` checks the result exactly.
//!
//! What it does not do is hide its seams or keep the charts few. A smooth
//! ball comes out as two or three dozen. For a picture that is BAKED from the
//! model, with the same colour either side of every seam and a margin round
//! every chart, a seam is not seen; for a picture somebody will paint on in
//! another program, fewer and better placed charts would matter, and that
//! is not this.
//!
//! # Packing
//!
//! The charts are laid in rows on a square atlas, tallest first, with a
//! margin between them, at the largest scale at which they all fit. The
//! scale is the same for every chart, so a texel is the same size on the
//! model everywhere, give or take the stretch above.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use super::ExportMesh;

/// How an unwrap is asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnwrapSpec {
    /// The atlas is this many texels on a side.
    pub size: u32,
    /// Texels left clear round every chart, so that a texture's edge
    /// colours can be spread into them and a blurred lookup does not pick
    /// up the chart next door.
    pub margin: u32,
    /// The furthest a triangle may face from its chart's axis, in radians.
    pub widest_turn: f32,
}

impl Default for UnwrapSpec {
    fn default() -> Self {
        Self { size: 2048, margin: 4, widest_turn: 60.0_f32.to_radians() }
    }
}

/// One patch of the surface, laid flat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chart {
    /// The way the patch faces: it is laid flat by looking along this.
    pub axis: Vec3,
    /// Where it sits on the atlas, in texels: its top left corner and its
    /// size, not counting the margin.
    pub origin: Vec2,
    pub size: Vec2,
    /// How many triangles it holds.
    pub triangles: usize,
}

/// A mesh's place on an atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct Unwrap {
    /// Points on the atlas, in texels, `x` to the right and `y` DOWN from
    /// its top left corner. A vertex of the mesh that is on the edge of two
    /// charts has one of these in each.
    pub points: Vec<Vec2>,
    /// For each triangle of the mesh, in the mesh's order, the three points
    /// its corners are at, in the order of its corners.
    pub corners: Vec<[u32; 3]>,
    /// For each triangle, which chart it is in.
    pub chart_of: Vec<u32>,
    pub charts: Vec<Chart>,
    /// The atlas's side, in texels.
    pub size: u32,
    /// How many texels there are to a world unit, the same everywhere.
    pub texels_per_unit: f32,
}

impl ExportMesh {
    /// Lay this mesh out on an atlas. See the module documentation.
    ///
    /// A mesh with no triangles unwraps to nothing. A triangle of no area
    /// is given a chart with the rest, at a single point, and bakes to
    /// nothing.
    pub fn unwrapped(&self, spec: UnwrapSpec) -> Unwrap {
        let size = spec.size.max(16);
        let margin = spec.margin.min(size / 8) as f32;
        let count = self.triangles.len();
        let mut unwrap = Unwrap {
            points: Vec::new(),
            corners: vec![[0; 3]; count],
            chart_of: vec![u32::MAX; count],
            charts: Vec::new(),
            size,
            texels_per_unit: 0.0,
        };
        if count == 0 {
            return unwrap;
        }

        let normals: Vec<Vec3> = self
            .triangles
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (
                    self.positions[*a as usize],
                    self.positions[*b as usize],
                    self.positions[*c as usize],
                );
                (b - a).cross(c - a).normalize_or_zero()
            })
            .collect();
        let neighbours = neighbours(&self.triangles);
        let reach = spec.widest_turn.clamp(0.1, 1.4).cos();

        // Each chart as it is grown: its axis, its triangles, and where each
        // of its vertices lands in the chart's own plane, in world units.
        struct Grown {
            axis: Vec3,
            triangles: Vec<u32>,
            /// Mesh vertex to index into `flat`.
            place: HashMap<u32, u32>,
            flat: Vec<Vec2>,
            low: Vec2,
            high: Vec2,
        }
        let mut grown: Vec<Grown> = Vec::new();
        let mut frontier: Vec<u32> = Vec::new();
        // Flattest first. A chart's axis is the way its seed faces, and a
        // seed on the rounded edge between two faces of a cube faces half
        // way between them: its chart took a strip of both, each drawn at
        // seven tenths of its size. A triangle that faces as its neighbours
        // do is in the middle of something, and that is the place to start.
        let flatness = |triangle: usize| {
            neighbours[triangle]
                .iter()
                .filter(|next| **next != u32::MAX)
                .map(|next| normals[triangle].dot(normals[*next as usize]))
                .fold(1.0_f32, f32::min)
        };
        let mut seeds: Vec<(f32, usize)> = (0..count).map(|t| (flatness(t), t)).collect();
        seeds.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, seed) in seeds {
            if unwrap.chart_of[seed] != u32::MAX {
                continue;
            }
            let chart = grown.len() as u32;
            // A triangle of no area faces nowhere. Any axis does.
            let axis = if normals[seed] == Vec3::ZERO { Vec3::Z } else { normals[seed] };
            let across = axis.any_orthonormal_vector();
            let up = axis.cross(across);
            let flatten = |point: Vec3| Vec2::new(point.dot(across), point.dot(up));
            let mut laid = Laid::default();
            let mut triangles = Vec::new();

            frontier.clear();
            frontier.push(seed as u32);
            unwrap.chart_of[seed] = chart;
            while let Some(triangle) = frontier.pop() {
                let flat =
                    self.triangles[triangle as usize].map(|v| flatten(self.positions[v as usize]));
                // The seed is always taken: a chart of one triangle cannot
                // overlap itself.
                if triangle as usize != seed && laid.would_overlap(flat) {
                    unwrap.chart_of[triangle as usize] = u32::MAX;
                    continue;
                }
                laid.add(flat);
                triangles.push(triangle);
                for next in neighbours[triangle as usize] {
                    if next == u32::MAX || unwrap.chart_of[next as usize] != u32::MAX {
                        continue;
                    }
                    if normals[next as usize].dot(axis) >= reach {
                        unwrap.chart_of[next as usize] = chart;
                        frontier.push(next);
                    }
                }
            }

            let mut place = HashMap::new();
            let mut flat = Vec::new();
            let (mut low, mut high) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
            for triangle in &triangles {
                for vertex in self.triangles[*triangle as usize] {
                    place.entry(vertex).or_insert_with(|| {
                        let at = flatten(self.positions[vertex as usize]);
                        low = low.min(at);
                        high = high.max(at);
                        flat.push(at);
                        flat.len() as u32 - 1
                    });
                }
            }
            grown.push(Grown { axis, triangles, place, flat, low, high });
        }

        // The largest scale at which every chart, with its margin, fits.
        // Found by halving: packing at a scale either fits or does not, and
        // a smaller scale always fits if a larger one does.
        let extents: Vec<Vec2> = grown.iter().map(|chart| chart.high - chart.low).collect();
        let longest = extents.iter().map(|extent| extent.max_element()).fold(0.0_f32, f32::max);
        // At this scale the longest chart alone spans the atlas, so nothing
        // larger can fit.
        let mut too_large = (size as f32 - 2.0 * margin) / longest.max(f32::MIN_POSITIVE);
        let mut fits = 0.0_f32;
        let mut placed = pack(&extents, 0.0, margin, size as f32).expect("charts of no size fit");
        for _ in 0..32 {
            let scale = (fits + too_large) * 0.5;
            match pack(&extents, scale, margin, size as f32) {
                Some(origins) => {
                    fits = scale;
                    placed = origins;
                }
                None => too_large = scale,
            }
        }
        unwrap.texels_per_unit = fits;

        for (index, chart) in grown.iter().enumerate() {
            let origin = placed[index];
            let first = unwrap.points.len() as u32;
            unwrap.points.extend(chart.flat.iter().map(|at| {
                let local = (*at - chart.low) * fits;
                // Up the chart's own plane is up the atlas, which is toward
                // smaller `y`.
                origin + Vec2::new(local.x, (chart.high.y - chart.low.y) * fits - local.y)
            }));
            for triangle in &chart.triangles {
                unwrap.corners[*triangle as usize] =
                    self.triangles[*triangle as usize].map(|vertex| first + chart.place[&vertex]);
            }
            unwrap.charts.push(Chart {
                axis: chart.axis,
                origin,
                size: extents[index] * fits,
                triangles: chart.triangles.len(),
            });
        }
        unwrap
    }
}

/// For each triangle, the triangle across each of its three edges, or
/// `u32::MAX` where there is none or more than one.
fn neighbours(triangles: &[[u32; 3]]) -> Vec<[u32; 3]> {
    let mut across: HashMap<(u32, u32), (u32, u32)> = HashMap::with_capacity(triangles.len() * 2);
    for (index, triangle) in triangles.iter().enumerate() {
        for edge in 0..3 {
            let (a, b) = (triangle[edge], triangle[(edge + 1) % 3]);
            let key = (a.min(b), a.max(b));
            let entry = across.entry(key).or_insert((u32::MAX, u32::MAX));
            if entry.0 == u32::MAX {
                entry.0 = index as u32;
            } else if entry.1 == u32::MAX {
                entry.1 = index as u32;
            } else {
                // A third triangle on one edge: nobody is anybody's
                // neighbour across it.
                *entry = (u32::MAX - 1, u32::MAX - 1);
            }
        }
    }
    triangles
        .iter()
        .enumerate()
        .map(|(index, triangle)| {
            let mut out = [u32::MAX; 3];
            for edge in 0..3 {
                let (a, b) = (triangle[edge], triangle[(edge + 1) % 3]);
                let (one, other) = across[&(a.min(b), a.max(b))];
                let next = if one == index as u32 { other } else { one };
                if next < u32::MAX - 1 {
                    out[edge] = next;
                }
            }
            out
        })
        .collect()
}

/// The triangles a chart has laid down so far, in its own plane, for asking
/// whether another would land on them.
#[derive(Default)]
struct Laid {
    triangles: Vec<[Vec2; 3]>,
    /// Which triangles touch each cell of a grid over the plane.
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// The grid's cell, set by the first triangle: about twice its size.
    cell: f32,
}

impl Laid {
    fn cells_of(&self, triangle: [Vec2; 3]) -> impl Iterator<Item = (i32, i32)> + use<> {
        let low = triangle[0].min(triangle[1]).min(triangle[2]) / self.cell;
        let high = triangle[0].max(triangle[1]).max(triangle[2]) / self.cell;
        let (x0, y0) = (low.x.floor() as i32, low.y.floor() as i32);
        let (x1, y1) = (high.x.floor() as i32, high.y.floor() as i32);
        // A triangle far larger than the first would touch a great many
        // cells. Capped: past the cap it is listed in fewer than it
        // touches, and the exact test below is what the result rests on.
        let (x1, y1) = (x1.min(x0 + 15), y1.min(y0 + 15));
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
    }

    fn add(&mut self, triangle: [Vec2; 3]) {
        if self.triangles.is_empty() {
            let extent = (triangle[0].max(triangle[1]).max(triangle[2])
                - triangle[0].min(triangle[1]).min(triangle[2]))
            .max_element();
            self.cell = (extent * 2.0).max(1.0e-6);
        }
        let index = self.triangles.len() as u32;
        self.triangles.push(triangle);
        for cell in self.cells_of(triangle) {
            self.cells.entry(cell).or_default().push(index);
        }
    }

    /// Whether a triangle would lie over one already laid, other than along
    /// an edge or at a corner they share.
    fn would_overlap(&self, triangle: [Vec2; 3]) -> bool {
        if self.triangles.is_empty() {
            return false;
        }
        for cell in self.cells_of(triangle) {
            let Some(listed) = self.cells.get(&cell) else {
                continue;
            };
            if listed.iter().any(|other| overlap(triangle, self.triangles[*other as usize])) {
                return true;
            }
        }
        false
    }
}

/// Twice the signed area of a triangle in the plane.
fn doubled_area(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

/// Whether a point is strictly inside a triangle, by a margin of a
/// thousandth of the triangle's own size, so that a point on a shared edge
/// or corner is not inside. What the tests count coverage with.
#[cfg(test)]
pub(crate) fn strictly_inside(point: Vec2, triangle: [Vec2; 3]) -> bool {
    let area = doubled_area(triangle[0], triangle[1], triangle[2]);
    if area.abs() <= f32::MIN_POSITIVE {
        return false;
    }
    let margin = area.abs() * 1.0e-3;
    (0..3).all(|edge| {
        let (a, b) = (triangle[edge], triangle[(edge + 1) % 3]);
        doubled_area(a, b, point) * area.signum() > margin
    })
}

/// Whether two triangles in the plane overlap by more than a shared edge or
/// corner.
///
/// By separating axes, which for two triangles is exact: they are apart if
/// and only if, looked at along one of their six edges, one is wholly to
/// one side of the other. Touching counts as apart, with a thousandth of
/// their size to spare, so that neighbours sharing an edge do not overlap
/// and neither do two triangles whose shared corner has rounded apart.
///
/// The first version tested four points of each triangle against the other
/// and let 64 texels of a spiral ramp be drawn twice: two triangles can
/// cross with none of those points inside either.
fn overlap(one: [Vec2; 3], other: [Vec2; 3]) -> bool {
    let span = |triangle: [Vec2; 3], axis: Vec2| {
        let along = triangle.map(|corner| corner.dot(axis));
        (along[0].min(along[1]).min(along[2]), along[0].max(along[1]).max(along[2]))
    };
    for triangle in [one, other] {
        // A triangle of no area lies on nothing.
        if doubled_area(triangle[0], triangle[1], triangle[2]).abs() <= f32::MIN_POSITIVE {
            return false;
        }
        for edge in 0..3 {
            let axis = (triangle[(edge + 1) % 3] - triangle[edge]).perp();
            let (one_low, one_high) = span(one, axis);
            let (other_low, other_high) = span(other, axis);
            let spare = 1.0e-3 * (one_high - one_low).max(other_high - other_low);
            if one_high <= other_low + spare || other_high <= one_low + spare {
                return false;
            }
        }
    }
    true
}

/// Lay charts of the given extents in rows on a square atlas, at a scale,
/// tallest first. The top left corner of each, in texels, in the order
/// given, or `None` when they do not fit.
fn pack(extents: &[Vec2], scale: f32, margin: f32, size: f32) -> Option<Vec<Vec2>> {
    let mut order: Vec<usize> = (0..extents.len()).collect();
    // Tallest first, and by index among equals so that the layout does not
    // depend on how a sort breaks ties.
    order.sort_by(|a, b| extents[*b].y.total_cmp(&extents[*a].y).then(a.cmp(b)));
    let mut origins = vec![Vec2::ZERO; extents.len()];
    let (mut x, mut y, mut row) = (margin, margin, 0.0_f32);
    for index in order {
        // A whole texel at least, so that a chart of no size still has
        // somewhere to be.
        let extent = (extents[index] * scale).max(Vec2::ONE);
        if extent.x + 2.0 * margin > size {
            return None;
        }
        if x + extent.x + margin > size {
            x = margin;
            y += row + margin;
            row = 0.0;
        }
        if y + extent.y + margin > size {
            return None;
        }
        origins[index] = Vec2::new(x, y);
        x += extent.x + margin;
        row = row.max(extent.y);
    }
    Some(origins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::{INSIDE, NARROW_BAND, OUTSIDE};
    use crate::volume::Volume;

    fn ball() -> ExportMesh {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 12.0);
        let (mesh, report) = volume.export_mesh();
        assert!(report.is_printable());
        mesh
    }

    fn cube() -> ExportMesh {
        let mut volume = Volume::new(0.5);
        let reach = Vec3::splat(10.0 + NARROW_BAND);
        volume.edit_box(-reach, reach, |position, _| {
            let outside = position.abs() - Vec3::splat(10.0);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / 0.5).clamp(INSIDE, OUTSIDE)
        });
        volume.export_mesh().0
    }

    /// A ramp that winds twice round an axis, like a spiral stair with no
    /// steps. Every triangle of it faces nearly straight up, and seen from
    /// above the second turn lies exactly over the first.
    fn spiral_ramp() -> ExportMesh {
        let (turns, steps) = (2.0_f32, 96);
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        for step in 0..=steps {
            let along = step as f32 / steps as f32;
            let angle = along * turns * std::f32::consts::TAU;
            let rise = along * turns * 2.0;
            for radius in [4.0_f32, 10.0] {
                positions.push(Vec3::new(radius * angle.cos(), radius * angle.sin(), rise));
            }
        }
        for step in 0..steps as u32 {
            let (a, b, c, d) = (step * 2, step * 2 + 1, step * 2 + 2, step * 2 + 3);
            triangles.push([a, b, c]);
            triangles.push([b, d, c]);
        }
        ExportMesh { normals: Vec::new(), slots: Vec::new(), positions, triangles }
    }

    fn area(mesh: &ExportMesh, triangle: usize) -> f32 {
        let [a, b, c] = mesh.triangles[triangle].map(|v| mesh.positions[v as usize]);
        (b - a).cross(c - a).length() * 0.5
    }

    fn atlas_triangle(unwrap: &Unwrap, triangle: usize) -> [Vec2; 3] {
        unwrap.corners[triangle].map(|point| unwrap.points[point as usize])
    }

    /// How many texel centres of the atlas are under more than one
    /// triangle, and under at least one: an exact count by rasterising,
    /// which is what the sampled test inside the unwrapper is held to.
    fn doubly_covered(unwrap: &Unwrap) -> (usize, usize) {
        let size = unwrap.size as usize;
        let mut covered = vec![0_u8; size * size];
        for triangle in 0..unwrap.corners.len() {
            let corners = atlas_triangle(unwrap, triangle);
            let low = corners[0].min(corners[1]).min(corners[2]).floor().max(Vec2::ZERO);
            let high = corners[0].max(corners[1]).max(corners[2]).ceil();
            for y in low.y as usize..(high.y as usize).min(size) {
                for x in low.x as usize..(high.x as usize).min(size) {
                    let centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    if strictly_inside(centre, corners) {
                        covered[y * size + x] = covered[y * size + x].saturating_add(1);
                    }
                }
            }
        }
        (
            covered.iter().filter(|count| **count > 1).count(),
            covered.iter().filter(|count| **count > 0).count(),
        )
    }

    /// Everything an unwrap has to be, checked on one mesh.
    fn check(mesh: &ExportMesh, spec: UnwrapSpec, what: &str) -> Unwrap {
        let unwrap = mesh.unwrapped(spec);
        let count = mesh.triangles.len();
        assert_eq!(unwrap.corners.len(), count, "{what}");
        assert_eq!(unwrap.chart_of.len(), count, "{what}");
        assert_eq!(unwrap.size, spec.size);
        assert!(unwrap.texels_per_unit > 0.0, "{what}: the scale is {}", unwrap.texels_per_unit);
        assert_eq!(
            unwrap.charts.iter().map(|chart| chart.triangles).sum::<usize>(),
            count,
            "{what}: the charts do not hold every triangle once"
        );

        let size = spec.size as f32;
        let margin = spec.margin as f32;
        for triangle in 0..count {
            let chart = unwrap.chart_of[triangle] as usize;
            assert!(chart < unwrap.charts.len(), "{what}: triangle {triangle} has no chart");
            let chart = unwrap.charts[chart];
            let corners = atlas_triangle(&unwrap, triangle);
            for corner in corners {
                assert!(
                    corner.cmpge(chart.origin - 0.01).all()
                        && corner.cmple(chart.origin + chart.size + 0.01).all(),
                    "{what}: triangle {triangle} is at {corner}, outside its chart at {} of {}",
                    chart.origin,
                    chart.size
                );
            }
            // Not flipped: seen from outside a triangle runs anticlockwise,
            // and with `y` down the atlas that is a negative doubled area.
            let flat = doubled_area(corners[0], corners[1], corners[2]);
            let solid = area(mesh, triangle);
            if solid > 1.0e-6 {
                assert!(flat < 0.0, "{what}: triangle {triangle} is flipped or flat ({flat})");
                // And drawn at between half its size and all of it: the
                // widest turn from the axis is 60 degrees, whose cosine is
                // a half.
                let drawn = (-flat * 0.5) / (solid * unwrap.texels_per_unit.powi(2));
                assert!(
                    (spec.widest_turn.cos() - 0.02..=1.001).contains(&drawn),
                    "{what}: triangle {triangle} is drawn at {drawn} of its size"
                );
            }
        }

        // Every chart is on the atlas with its margin, and no two charts'
        // margins overlap.
        for (index, chart) in unwrap.charts.iter().enumerate() {
            assert!(
                chart.origin.cmpge(Vec2::splat(margin - 0.01)).all()
                    && (chart.origin + chart.size).cmple(Vec2::splat(size - margin + 0.01)).all(),
                "{what}: chart {index} at {} of {} is off the atlas",
                chart.origin,
                chart.size
            );
            for other in &unwrap.charts[index + 1..] {
                let apart = (chart.origin.x + chart.size.x + margin <= other.origin.x + 0.01)
                    || (other.origin.x + other.size.x + margin <= chart.origin.x + 0.01)
                    || (chart.origin.y + chart.size.y + margin <= other.origin.y + 0.01)
                    || (other.origin.y + other.size.y + margin <= chart.origin.y + 0.01);
                assert!(apart, "{what}: two charts are closer than the margin");
            }
        }
        unwrap
    }

    #[test]
    fn a_ball_unwraps_into_charts_that_face_one_way_each_and_fit_the_atlas() {
        let mesh = ball();
        let spec = UnwrapSpec { size: 512, ..UnwrapSpec::default() };
        let unwrap = check(&mesh, spec, "ball");
        // A cone of 60 degrees covers a quarter of a sphere, so four charts
        // is the fewest there could be. Greedy growth makes more.
        assert!(
            (4..=40).contains(&unwrap.charts.len()),
            "a ball came out as {} charts",
            unwrap.charts.len()
        );
        let (twice, once) = doubly_covered(&unwrap);
        assert_eq!(twice, 0, "{twice} texels of the atlas are under two triangles");
        // The atlas is used: more than a fifth of it is under the mesh.
        let used = once as f32 / (512.0 * 512.0);
        assert!(used > 0.2, "only {used} of the atlas is used");

        // Twice the atlas is twice the scale, to within the margins.
        let larger = check(&mesh, UnwrapSpec { size: 1024, ..spec }, "ball at 1024");
        let ratio = larger.texels_per_unit / unwrap.texels_per_unit;
        assert!((ratio - 2.0).abs() < 0.1, "twice the atlas gave {ratio} times the scale");
        // And the same mesh unwraps the same way twice.
        assert_eq!(mesh.unwrapped(spec), unwrap);
    }

    #[test]
    fn a_cube_unwraps_to_its_faces() {
        let mesh = cube();
        let unwrap = check(&mesh, UnwrapSpec { size: 512, ..UnwrapSpec::default() }, "cube");
        // Six faces, and the rounding the mesher puts on the edges may give
        // an edge or a corner a chart of its own.
        assert!(
            (6..=30).contains(&unwrap.charts.len()),
            "a cube came out as {} charts",
            unwrap.charts.len()
        );
        // The six largest charts are the faces: each close to a square 20
        // on a side, drawn square on.
        let mut sizes: Vec<Vec2> = unwrap.charts.iter().map(|chart| chart.size).collect();
        sizes.sort_by(|a, b| (b.x * b.y).total_cmp(&(a.x * a.y)));
        for size in &sizes[..6] {
            let side = *size / unwrap.texels_per_unit;
            assert!(
                (side.x - 20.0).abs() < 1.5 && (side.y - 20.0).abs() < 1.5,
                "a face of the cube is drawn {side} across"
            );
        }
        assert_eq!(doubly_covered(&unwrap).0, 0);
    }

    /// The case facing one way is not enough for. Without the overlap test
    /// the whole ramp is one chart, since all of it faces up, and its
    /// second turn is drawn on top of its first.
    #[test]
    fn no_two_triangles_of_a_chart_overlap_even_on_a_spiral() {
        let mesh = spiral_ramp();
        let spec = UnwrapSpec { size: 512, ..UnwrapSpec::default() };
        let unwrap = check(&mesh, spec, "spiral ramp");
        assert!(unwrap.charts.len() >= 2, "a ramp of two turns is one chart");
        let (twice, once) = doubly_covered(&unwrap);
        assert!(once > 2000, "the ramp covers only {once} texels");
        assert_eq!(twice, 0, "{twice} texels of the ramp are drawn twice");
    }

    #[test]
    fn nothing_and_next_to_nothing_unwrap_without_fuss() {
        let empty = ExportMesh {
            positions: Vec::new(),
            normals: Vec::new(),
            triangles: Vec::new(),
            slots: Vec::new(),
        };
        let unwrap = empty.unwrapped(UnwrapSpec::default());
        assert!(unwrap.points.is_empty() && unwrap.charts.is_empty() && unwrap.corners.is_empty());

        // One triangle, and one with no area beside it.
        let mesh = ExportMesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::X * 2.0],
            normals: Vec::new(),
            triangles: vec![[0, 1, 2], [1, 3, 1]],
            slots: Vec::new(),
        };
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 64, margin: 2, ..UnwrapSpec::default() });
        assert_eq!(unwrap.corners.len(), 2);
        assert!(unwrap.chart_of.iter().all(|chart| (*chart as usize) < unwrap.charts.len()));
        assert!(unwrap.points.iter().all(|point| point.is_finite()));
        assert!(unwrap.texels_per_unit > 10.0, "one small triangle did not fill the atlas");
        // A nonsense spec is brought into range and does not panic.
        let odd = mesh.unwrapped(UnwrapSpec { size: 0, margin: 900, widest_turn: f32::NAN });
        assert!(odd.size >= 16 && odd.points.iter().all(|point| point.is_finite()));
    }
}
