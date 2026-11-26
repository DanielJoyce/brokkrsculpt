// SPDX-License-Identifier: AGPL-3.0-only

//! Vector displacement: a map of displacements set down as a shape.
//!
//! `docs/STUDIO-PLAN.md` section 18, stage C10. A stamp image
//! ([`crate::brush::alpha`]) is a height at each point of a square: the
//! surface there goes straight out by so much. A VECTOR displacement map
//! holds three numbers at each point, and the surface there goes out,
//! across and up the square by so much: it can lean over itself, which
//! a height cannot say. A horn that curls, a mushroom, a scale with an
//! undercut.
//!
//! # A map is made a shape, once
//!
//! In a sculptor of meshes the map moves the vertices under the brush.
//! Here there are none. [`Relief::of`] instead builds the surface the map
//! describes, a grid of points each moved by its displacement, closes it
//! underneath with a skirt and a floor, and hands that mesh to the
//! voxeliser an imported model goes through ([`crate::voxelise`]). What
//! comes back is a body: the map's shape standing on a flat plate. It is
//! built at several sizes, and a stamp reads the one that is no finer
//! than itself, so that what it reads is a distance for as far as the
//! field wants one.
//!
//! # Set down on the surface as it is, and not on a flat
//!
//! The plate is flat and a model is not. Set down as it is, the plate's
//! edge stands proud of a surface that curves away from it by however
//! much the surface curves, and every stamp sits on a square plinth. So
//! the surface under the stamp is measured first ([`Heights::under`]: how
//! far it is from the stamp's own plane, on a grid over the square), and
//! [`Draped`] reads the shape at a point's height ABOVE THE SURFACE and
//! not above the plane. Where the map's displacement is nothing the
//! shape's top is then the surface itself, and the edge of the square
//! cannot be found afterwards.
//!
//! # Up and down, and a difference
//!
//! A map can dig as well as raise, and where it does nothing the body
//! has to be left exactly as it was. So a stamp is not the shape joined
//! to the body. It is what the shape DOES to the flat plate it stands
//! on, the difference between the shape's field and the plate's, added
//! to the body's own field ([`Join::stamp`]). Where the map does nothing
//! that difference is nothing.
//!
//! The shape is a [`Solid`] too, what is under its displaced surface,
//! for a stamp that is left as a body of its own.

use glam::{Mat3, Vec2, Vec3};

use super::{Join, Solid};
use crate::brick::{BRICK_DIM, INSIDE, OUTSIDE};
use crate::export::ExportMesh;
use crate::mask::PROTECTED;
use crate::volume::{BrickPreview, BrickVerdict, Volume};
use crate::voxelise::{VoxeliseOptions, voxelise};

/// The most texels along one side of a map. A stamp is a few dozen
/// voxels across on the model, as [`crate::brush::alpha::MAX_ALPHA_SIZE`]
/// has it for a stamp image.
pub const MAX_MAP_SIZE: usize = 512;

/// How many voxels across the square a map's shape is built at. A stamp
/// reads the largest of these that is no bigger than itself across.
const LEVELS: [usize; 3] = [32, 64, 128];

/// How many cells across the square the surface under a stamp is
/// measured on.
const HEIGHT_CELLS: usize = 24;

/// How far below the deepest the map digs the plate goes, and how far
/// over the highest it rises the air that is taken out goes, in
/// half-widths of the square.
const MARGIN: f32 = 0.08;

/// Why a set of displacements is not a map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// Fewer than three texels a side: no texel that is not on the rim.
    TooSmall,
    /// The number of displacements is not `width * height`.
    WrongCount { expected: usize, found: usize },
    /// A displacement is not a number.
    NotFinite,
    /// Every displacement is nothing, or it could not be made a shape.
    NoShape,
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapError::TooSmall => {
                f.write_str("a displacement map needs at least three pixels a side")
            }
            MapError::WrongCount { expected, found } => {
                write!(f, "a map of that size has {expected} displacements and {found} were given")
            }
            MapError::NotFinite => f.write_str("a map holds a displacement that is not a number"),
            MapError::NoShape => f.write_str("the map displaces nothing"),
        }
    }
}

/// A square of displacements. At each texel, how far the surface there
/// moves: `x` across the square, `y` up it, `z` out of it, in half-widths
/// of the square, so that a `z` of 1 is as high as the square is half
/// across.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorMap {
    size: usize,
    vectors: Vec<Vec3>,
}

impl VectorMap {
    /// A map from `width * height` displacements, row by row from the
    /// TOP of the picture, which is the top of the square. One that is
    /// not square, or is bigger than [`MAX_MAP_SIZE`], is read at the
    /// nearest square size that is.
    pub fn from_vectors(
        width: usize,
        height: usize,
        vectors: &[[f32; 3]],
    ) -> Result<Self, MapError> {
        if width < 3 || height < 3 {
            return Err(MapError::TooSmall);
        }
        if vectors.len() != width * height {
            return Err(MapError::WrongCount { expected: width * height, found: vectors.len() });
        }
        if vectors.iter().flatten().any(|part| !part.is_finite()) {
            return Err(MapError::NotFinite);
        }
        let size = width.max(height).min(MAX_MAP_SIZE);
        let read = |x: usize, y: usize| Vec3::from_array(vectors[x + y * width]);
        let mut held = Vec::with_capacity(size * size);
        for row in 0..size {
            // Row 0 of what is held is the BOTTOM of the square.
            let from_top = (size - 1 - row) as f32 / (size - 1) as f32 * (height - 1) as f32;
            let (y0, ty) = ((from_top as usize).min(height - 2), from_top.fract());
            let ty = if from_top as usize >= height - 1 { 1.0 } else { ty };
            for column in 0..size {
                let across = column as f32 / (size - 1) as f32 * (width - 1) as f32;
                let (x0, tx) = ((across as usize).min(width - 2), across.fract());
                let tx = if across as usize >= width - 1 { 1.0 } else { tx };
                let low = read(x0, y0).lerp(read(x0 + 1, y0), tx);
                let high = read(x0, y0 + 1).lerp(read(x0 + 1, y0 + 1), tx);
                held.push(low.lerp(high, ty));
            }
        }
        Ok(Self { size, vectors: held })
    }

    /// A map worked out from a formula: `displaced(u, v)` for each texel,
    /// `u` across the square and `v` up it, each from -1 to 1.
    pub fn from_fn(size: usize, displaced: impl Fn(f32, f32) -> Vec3) -> Self {
        let size = size.clamp(3, MAX_MAP_SIZE);
        let mut vectors = Vec::with_capacity(size * size);
        for row in 0..size {
            for column in 0..size {
                let at = |texel: usize| texel as f32 / (size - 1) as f32 * 2.0 - 1.0;
                let vector = displaced(at(column), at(row));
                vectors.push(if vector.is_finite() { vector } else { Vec3::ZERO });
            }
        }
        Self { size, vectors }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// The displacement at a point of the square, each axis from -1 to 1.
    /// Outside the square is nothing, and so is the outermost ring of
    /// texels, as it is for a stamp image and for its reason: a map
    /// whose edge is not nothing would end in a cliff the shape of the
    /// square.
    pub fn sample(&self, u: f32, v: f32) -> Vec3 {
        if !(u.abs() <= 1.0 && v.abs() <= 1.0) {
            return Vec3::ZERO;
        }
        let last = self.size - 1;
        let texel = |x: usize, y: usize| {
            if x == 0 || y == 0 || x == last || y == last {
                Vec3::ZERO
            } else {
                self.vectors[x + y * self.size]
            }
        };
        let (x, y) = ((u + 1.0) * 0.5 * last as f32, (v + 1.0) * 0.5 * last as f32);
        let (x0, y0) = ((x as usize).min(last - 1), (y as usize).min(last - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let low = texel(x0, y0).lerp(texel(x0 + 1, y0), tx);
        let high = texel(x0, y0 + 1).lerp(texel(x0 + 1, y0 + 1), tx);
        low.lerp(high, ty)
    }
}

/// The maps that come with the application, worked out from a formula.
/// Each leans over itself somewhere, which is what a map is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltInMap {
    /// A stem with a wider cap on it: a mushroom, a rivet with a neck.
    Stud,
    /// A horn that curls over to one side.
    Horn,
    /// A plate that leans over the surface behind it: a shingle, one
    /// scale of a hide.
    Shingle,
}

impl BuiltInMap {
    pub const ALL: [BuiltInMap; 3] = [BuiltInMap::Stud, BuiltInMap::Horn, BuiltInMap::Shingle];

    pub fn label(self) -> &'static str {
        match self {
            BuiltInMap::Stud => "Stud",
            BuiltInMap::Horn => "Horn",
            BuiltInMap::Shingle => "Shingle",
        }
    }

    pub fn map(self) -> VectorMap {
        VectorMap::from_fn(129, |u, v| self.displaced(u, v))
    }

    fn displaced(self, u: f32, v: f32) -> Vec3 {
        let flat = Vec2::new(u, v);
        let radius = flat.length();
        let around = if radius > 1.0e-6 { flat / radius } else { Vec2::X };
        match self {
            BuiltInMap::Stud => {
                // The ring of the square from 0.9 in to its middle, laid
                // along the outline of a stud from its foot to the top of
                // its cap: in to the stem, up it, OUT under the cap, and
                // over the cap to the middle.
                let outline = [
                    (0.9, 0.0),
                    (0.26, 0.0),
                    (0.22, 0.06),
                    (0.22, 0.30),
                    (0.50, 0.34),
                    (0.56, 0.42),
                    (0.50, 0.52),
                    (0.30, 0.60),
                    (0.0, 0.63),
                ];
                if radius >= 0.9 {
                    return Vec3::ZERO;
                }
                let (out, up) = along(&outline, 1.0 - radius / 0.9);
                (around * out - flat).extend(up)
            }
            BuiltInMap::Horn => {
                // A cone whose middle line bends over toward +u as it
                // rises: its tip is beyond its own foot.
                if radius >= 0.9 {
                    return Vec3::ZERO;
                }
                let foot = 0.34;
                if radius >= 0.5 {
                    // The ring outside the foot is drawn in to it.
                    let out = foot + (0.9 - foot) * (radius - 0.5) / 0.4;
                    return (around * out - flat).extend(0.0);
                }
                let rise = 1.0 - radius / 0.5;
                let turn = rise * 1.5;
                let long = 0.85;
                let middle = Vec3::new(1.0 - turn.cos(), 0.0, turn.sin()) * (long / 1.5);
                let thick = foot * (1.0 - rise).powf(0.8);
                let across = Vec3::new(turn.cos(), 0.0, -turn.sin());
                let point = middle + (across * around.x + Vec3::Y * around.y) * thick;
                point - flat.extend(0.0)
            }
            BuiltInMap::Shingle => {
                // A low dome pushed along +u by more than it slopes: its
                // far edge hangs over the surface behind it.
                let dome = (1.0 - (flat / Vec2::new(0.62, 0.5)).length_squared()).max(0.0);
                let high = 0.26 * dome.powf(0.6);
                Vec3::new(high * 1.6, 0.0, high)
            }
        }
    }
}

/// The point a share `along` of the way down an outline, by length.
fn along(outline: &[(f32, f32)], share: f32) -> (f32, f32) {
    let lengths: Vec<f32> = outline
        .windows(2)
        .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
        .collect();
    let mut left = share.clamp(0.0, 1.0) * lengths.iter().sum::<f32>();
    for (pair, length) in outline.windows(2).zip(&lengths) {
        if left <= *length {
            let part = if *length > 0.0 { left / length } else { 0.0 };
            return (
                pair[0].0 + (pair[1].0 - pair[0].0) * part,
                pair[0].1 + (pair[1].1 - pair[0].1) * part,
            );
        }
        left -= length;
    }
    outline[outline.len() - 1]
}

/// One size of a map's shape: a body with the square `across` voxels of
/// one unit a side, in the plane of X and Y about the origin, and the
/// map's `z` along Z.
struct Level {
    across: usize,
    body: Volume,
}

/// A map made a shape. See the module documentation.
pub struct Relief {
    levels: Vec<Level>,
    /// The deepest the map digs and the highest it rises, in half-widths
    /// of the square: at most nothing, and at least nothing.
    low: f32,
    high: f32,
    /// How far past the edge of the square the shape leans, in
    /// half-widths.
    lean: f32,
}

impl Relief {
    /// The shape of `map`, at each of the sizes a stamp reads.
    pub fn of(map: &VectorMap) -> Result<Self, MapError> {
        // What the map does, read on the finest grid a shape is built on.
        let finest = LEVELS[LEVELS.len() - 1];
        let (mut low, mut high, mut lean) = (0.0_f32, 0.0_f32, 0.0_f32);
        for row in 0..=finest {
            for column in 0..=finest {
                let (u, v) = (grid(column, finest), grid(row, finest));
                let moved = Vec3::new(u, v, 0.0) + map.sample(u, v);
                low = low.min(moved.z);
                high = high.max(moved.z);
                lean = lean.max(moved.x.abs().max(moved.y.abs()) - 1.0);
            }
        }
        if high - low < 1.0e-4 {
            return Err(MapError::NoShape);
        }
        let floor = floor_of(low);
        let levels: Vec<Level> = LEVELS
            .iter()
            .filter_map(|across| {
                let mesh = shape(map, *across, floor);
                let options = VoxeliseOptions {
                    voxel_size: 1.0,
                    centre: false,
                    refit_if_implausible: false,
                    fill_sealed_cavities: false,
                    repair_broken_scan_lines: true,
                    coarsen_to_fit: false,
                    refine_to_resolve: false,
                    already_reserved: 0.0,
                };
                let (body, _) = voxelise(&mesh, &options).ok()?;
                Some(Level { across: *across, body })
            })
            .collect();
        if levels.is_empty() {
            return Err(MapError::NoShape);
        }
        Ok(Self { levels, low, high, lean })
    }

    /// The level a stamp `across` voxels of the model reads: the finest
    /// that is no finer than the stamp, so that a voxel of the shape is
    /// at least a voxel of the model and its field is a distance for as
    /// far as the model's is.
    fn level(&self, across: f32) -> &Level {
        self.levels
            .iter()
            .rev()
            .find(|level| level.across as f32 <= across)
            .unwrap_or(&self.levels[0])
    }

    /// The highest it rises over the square and the deepest it digs
    /// under it, in half-widths of the square.
    pub fn span(&self) -> (f32, f32) {
        (self.low, self.high)
    }
}

/// Where a grid of `cells` puts its line `index`, from -1 to 1.
fn grid(index: usize, cells: usize) -> f32 {
    index as f32 / cells as f32 * 2.0 - 1.0
}

/// How far under the square the plate's floor is, in half-widths (a
/// negative height): twice as far as the map digs and a quarter of a
/// half-width more, so that the field down to the deepest the map digs
/// is the shape's and not the floor's. A quarter is four voxels of the
/// coarsest level, and a field is the distance to whatever is nearest:
/// with the floor nearer than that, the plate's own field stopped being
/// a height over it two and a half voxels down, and a stamp wrote the
/// difference into the body all over its square (found by the test of
/// the rim, 696 voxels of it).
fn floor_of(low: f32) -> f32 {
    2.0 * low - 2.0 * MARGIN - 0.25
}

/// The height, under the square, above which the shape's field says
/// whether a point is over or under the displaced surface, and below
/// which it may be saying that the point is under the floor.
fn waist_of(low: f32) -> f32 {
    low - MARGIN
}

/// How far past the square the plate a shape stands on goes, in voxels
/// of the level. The plate has a side, and within the band of it the
/// shape's field is the distance to that side and not to the plate's
/// top. With the side this far out, the field over the whole square is
/// the shape's and the plate's top's alone, and where the map does
/// nothing it is a point's height over the plate and nothing else.
const PLATE_PAST: usize = 6;

/// The map's shape as a closed mesh, with the square `across` units a
/// side: the displaced surface on a plate that goes [`PLATE_PAST`] past
/// the square each way, a skirt from the plate's rim down to `floor`
/// (in half-widths), and the floor.
fn shape(map: &VectorMap, across: usize, floor: f32) -> ExportMesh {
    let half = across as f32 * 0.5;
    let cells = across + 2 * PLATE_PAST;
    let side = cells + 1;
    // Where line `index` of the plate's grid is, in half-widths: line
    // `PLATE_PAST` is the edge of the square.
    let line = |index: usize| (index as f32 - PLATE_PAST as f32) / half - 1.0;
    let mut mesh = ExportMesh::default();
    for row in 0..side {
        for column in 0..side {
            let (u, v) = (line(column), line(row));
            mesh.positions.push((Vec3::new(u, v, 0.0) + map.sample(u, v)) * half);
        }
    }
    let top = |column: usize, row: usize| (column + row * side) as u32;
    for row in 0..cells {
        for column in 0..cells {
            let (a, b, c, d) = (
                top(column, row),
                top(column + 1, row),
                top(column + 1, row + 1),
                top(column, row + 1),
            );
            mesh.triangles.push([a, b, c]);
            mesh.triangles.push([a, c, d]);
        }
    }
    // The rim, once round with the plate on the left: along the bottom,
    // up the right, back along the top, down the left.
    let mut rim: Vec<(usize, usize)> = Vec::with_capacity(4 * cells);
    rim.extend((0..cells).map(|step| (step, 0)));
    rim.extend((0..cells).map(|step| (cells, step)));
    rim.extend((0..cells).map(|step| (cells - step, cells)));
    rim.extend((0..cells).map(|step| (0, cells - step)));
    let under = mesh.positions.len() as u32;
    for (column, row) in &rim {
        mesh.positions.push(Vec3::new(line(*column), line(*row), floor) * half);
    }
    let middle = mesh.positions.len() as u32;
    mesh.positions.push(Vec3::new(0.0, 0.0, floor) * half);
    for (at, (column, row)) in rim.iter().enumerate() {
        let next = (at + 1) % rim.len();
        let (here, there) = (top(*column, *row), top(rim[next].0, rim[next].1));
        let (below, beyond) = (under + at as u32, under + next as u32);
        // The skirt, facing out, and the floor, facing down.
        mesh.triangles.push([here, below, beyond]);
        mesh.triangles.push([here, beyond, there]);
        mesh.triangles.push([middle, beyond, below]);
    }
    mesh
}

/// How far the surface of a body is from a stamp's plane, over the
/// stamp's square: what a stamp is draped on.
#[derive(Debug, Clone, PartialEq)]
pub struct Heights {
    /// The grid covers the square and this much past it each way, in
    /// half-widths: a shape that leans past the edge is over surface too.
    reach: f32,
    /// `HEIGHT_CELLS + 1` a side, row by row from the bottom, in
    /// half-widths of the square.
    values: Vec<f32>,
}

impl Heights {
    /// A flat surface in the stamp's own plane: a stamp on nothing, as
    /// a body of its own is built.
    pub fn flat() -> Self {
        Self { reach: 1.0, values: vec![0.0; (HEIGHT_CELLS + 1) * (HEIGHT_CELLS + 1)] }
    }

    /// The surface of `volume` **as it was before the open stroke**,
    /// measured from the plane through `centre` that `frame` spans (its
    /// `x` across the square, its `y` up it, its `z` out of it), over a
    /// square of half-width `half` and as far past it as `relief` leans.
    ///
    /// From each point of the grid the surface is looked for along the
    /// plane's normal, the nearest to the plane, as far as the square is
    /// half across. Where there is none that near, off the edge of the
    /// model, the height is that far down.
    pub fn under(volume: &Volume, relief: &Relief, centre: Vec3, frame: Mat3, half: f32) -> Self {
        let reach = 1.0 + relief.lean.clamp(0.0, 1.0);
        let voxel = volume.voxel_size();
        let side = HEIGHT_CELLS + 1;
        let mut values = Vec::with_capacity(side * side);
        for row in 0..side {
            for column in 0..side {
                let (u, v) = (grid(column, HEIGHT_CELLS) * reach, grid(row, HEIGHT_CELLS) * reach);
                let foot = centre + (frame.x_axis * u + frame.y_axis * v) * half;
                let read = |along: f32| volume.sample_world_before(foot + frame.z_axis * along);
                let start = read(0.0);
                // Outside the body the surface is below, and inside it
                // above.
                let way = if start > 0.0 { -1.0 } else { 1.0 };
                let (mut along, mut last) = (0.0_f32, start);
                let mut found = way * half;
                while along.abs() < half {
                    // The field is in voxels and no more than three of
                    // them, so a step of most of what it says cannot pass
                    // the surface, and half a voxel is the least step.
                    let step = (last.abs() * 0.9).max(0.5) * voxel;
                    let next = along + way * step;
                    let value = read(next);
                    if (value > 0.0) != (start > 0.0) {
                        let part = last / (last - value);
                        found = along + (next - along) * if part.is_finite() { part } else { 0.5 };
                        break;
                    }
                    (along, last) = (next, value);
                }
                values.push(found / half);
            }
        }
        Self { reach, values }
    }

    /// The height at a point of the square, and how steep the surface is
    /// there: one over the cosine of its slope.
    fn at(&self, u: f32, v: f32) -> (f32, f32) {
        let side = HEIGHT_CELLS + 1;
        let cell = 2.0 * self.reach / HEIGHT_CELLS as f32;
        let (x, y) = (
            ((u + self.reach) / cell).clamp(0.0, HEIGHT_CELLS as f32),
            ((v + self.reach) / cell).clamp(0.0, HEIGHT_CELLS as f32),
        );
        let (x0, y0) = ((x as usize).min(HEIGHT_CELLS - 1), (y as usize).min(HEIGHT_CELLS - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let value = |x: usize, y: usize| self.values[x + y * side];
        let (a, b, c, d) =
            (value(x0, y0), value(x0 + 1, y0), value(x0, y0 + 1), value(x0 + 1, y0 + 1));
        let height = (a + (b - a) * tx) * (1.0 - ty) + (c + (d - c) * tx) * ty;
        let slope = Vec2::new(
            ((b - a) * (1.0 - ty) + (d - c) * ty) / cell,
            ((c - a) * (1.0 - tx) + (d - b) * tx) / cell,
        );
        (height, (1.0 + slope.length_squared()).sqrt())
    }
}

/// A map's shape set down on a surface.
///
/// It is two things. To [`Join::stamp`] it is a DISPLACEMENT of the body
/// it is set down on, up and down ([`Draped::displaced`]). And it is a
/// [`Solid`], what is under its displaced surface, which is what a stamp
/// left as a body of its own is built from.
pub struct Draped<'a> {
    relief: &'a Relief,
    level: &'a Level,
    heights: &'a Heights,
    centre: Vec3,
    frame: Mat3,
    /// Half the square across, in world units.
    half: f32,
    /// A voxel of the model, in world units.
    voxel_size: f32,
    /// Turned inside out: what the map raises is dug, and what it digs
    /// is raised.
    turned: bool,
}

/// A leaning part past the edge of the square is joined to the body only
/// where it is nearer than the body's own surface by this much, in
/// voxels: nearer by less is the same surface said twice.
const CLEAR: f32 = 0.25;

impl<'a> Draped<'a> {
    /// `relief` set down with its square of half-width `half` in the
    /// plane through `centre` that `frame` spans, on the surface
    /// `heights` measured from that same plane, in a body of
    /// `voxel_size`.
    pub fn on(
        relief: &'a Relief,
        heights: &'a Heights,
        centre: Vec3,
        frame: Mat3,
        half: f32,
        voxel_size: f32,
        turned: bool,
    ) -> Option<Self> {
        (half.is_finite() && half > 0.0 && voxel_size > 0.0 && centre.is_finite()).then(|| Self {
            relief,
            level: relief.level(2.0 * half / voxel_size),
            heights,
            centre,
            frame,
            half,
            voxel_size,
            turned,
        })
    }

    /// World units to a voxel of the level read.
    fn scale(&self) -> f32 {
        self.half / (self.level.across as f32 * 0.5)
    }

    /// How far from the middle of the square the shape reaches, in
    /// half-widths: the square, and as far past it as the shape leans.
    fn wide(&self) -> f32 {
        1.0 + self.relief.lean.max(0.0)
    }

    /// The heights the displaced surface is between, over the surface it
    /// is draped on, in half-widths.
    fn span(&self) -> (f32, f32) {
        let (low, high) = (self.relief.low, self.relief.high);
        if self.turned { (-high, -low) } else { (low, high) }
    }

    /// A point of the world in the stamp's own square: across, up, and
    /// its height over the stamp's plane, each in half-widths.
    ///
    /// The frame may be a mirror's, its axes the other way round: a
    /// stamp in a mirror is the stamp set down in the mirrored frame.
    fn local(&self, point: Vec3) -> Vec3 {
        self.frame.transpose() * (point - self.centre) / self.half
    }

    /// What the level's field says at a point of the square and a
    /// height over the draped surface, in voxels of the level: the
    /// shape's own field, read upside down when the stamp is turned.
    fn held(&self, local: Vec3, over: f32) -> f32 {
        let voxels = self.level.across as f32 * 0.5;
        let read = if self.turned { -over } else { over };
        self.level.body.sample_world(Vec3::new(local.x, local.y, read) * voxels)
    }

    /// How far outside the box the stamp can change anything in a point
    /// is, in world units: nothing inside it.
    fn outside(&self, point: Vec3) -> f32 {
        let local = self.local(point);
        let (surface, steep) = self.heights.at(local.x, local.y);
        let over = local.z - surface;
        let (low, high) = self.span();
        let wide = self.wide();
        let outside = Vec3::new(
            local.x.abs() - wide,
            local.y.abs() - wide,
            (low - MARGIN - over).max(over - high - MARGIN),
        );
        outside.max(Vec3::ZERO).length() * self.half / steep
    }

    /// What a voxel is to hold with the map's displacement in it, in
    /// voxels: `before` is what it held before the stroke and `value`
    /// what it holds now. `None` where the map changes nothing.
    ///
    /// **A difference, and not a shape.** The level's field is of the
    /// map's shape standing on a flat plate, and the field of the plate
    /// alone is a point's height over it. The difference of the two is
    /// what the map DOES to a flat surface, and that is added to the
    /// body's own field. Where the map does nothing the difference is
    /// nothing and the body is left exactly as it was, with no edge of a
    /// square to be found.
    ///
    /// The first build of this stage joined the shape to the body as a
    /// solid and took the air over it out as another. Where the map did
    /// nothing, the shape's top and the body's surface were the same
    /// surface said twice, by two fields that do not agree to the last
    /// fraction of a voxel, and every stamp left a faint dotted outline
    /// of its square: seen on a screen. Holding the solids back from the
    /// surface hid it and thinned every low part of every map.
    fn displaced(&self, point: Vec3, before: f32, value: f32) -> Option<f32> {
        let local = self.local(point);
        if local.x.abs() > self.wide() || local.y.abs() > self.wide() {
            return None;
        }
        let (surface, steep) = self.heights.at(local.x, local.y);
        let over = local.z - surface;
        let (low, high) = self.span();
        if over < low - MARGIN || over > high + MARGIN {
            return None;
        }
        let voxels = self.level.across as f32 * 0.5;
        // A voxel of the level, in voxels of the model, down this slope.
        let model = self.scale() / steep / self.voxel_size;
        let held = self.held(local, over);
        let known = held.abs() < OUTSIDE - 1.0e-3;
        let turn = if self.turned { -1.0 } else { 1.0 };

        // Past the edge of the square the map is nothing: only a part of
        // the shape that leans out over it, joined to the body where it
        // is clear of it.
        if local.x.abs() > 1.0 || local.y.abs() > 1.0 {
            let field = held * model;
            return (!self.turned && known && field < before - CLEAR).then(|| value.min(field));
        }
        // The displaced surface's own field, which turned inside out is
        // the other way up and the other way round.
        let field = turn * held;
        // The height over the plate, in voxels of the level, the way the
        // level is read.
        let plate = turn * over * voxels;
        if plate.abs() >= OUTSIDE {
            // Far from the plate there is only the shape. Over the
            // surface it adds, and where the level's field is a clamp
            // inside it, that is solid. Under the surface it digs, and a
            // clamp outside it is air.
            return if over > 0.0 {
                (field < OUTSIDE - 1.0e-3)
                    .then(|| value.min(if known { field * model } else { INSIDE }))
            } else {
                (field > INSIDE + 1.0e-3)
                    .then(|| value.max(if known { field * model } else { OUTSIDE }))
            };
        }
        let change = turn * (held - plate) * model;
        if change.abs() < 1.0e-4 {
            return None;
        }
        // The body's own field where it is a distance, and where it is a
        // clamp the height over the surface it was measured as.
        let base = if before.abs() < OUTSIDE - 1.0e-3 { before } else { over * voxels * model };
        let moved = base + change;
        Some(if change < 0.0 { value.min(moved) } else { value.max(moved) })
    }
}

impl Solid for Draped<'_> {
    /// What is under the displaced surface, down to the plate's floor:
    /// the shape itself, or turned inside out the air over it, kept to
    /// the square.
    fn distance(&self, point: Vec3) -> f32 {
        let local = self.local(point);
        let (surface, steep) = self.heights.at(local.x, local.y);
        let over = local.z - surface;
        let held = self.held(local, over);
        // A clamp is passed on as what it means, which is far, and not
        // as a distance: measured down a slope it is a little under
        // three voxels, and a join writes anything under three.
        let far = (OUTSIDE + 1.0) * self.voxel_size.max(self.scale());
        let shape = if held.abs() >= OUTSIDE - 1.0e-3 {
            far.copysign(held)
        } else {
            held * self.scale() / steep
        };
        let (low, high) = (self.relief.low, self.relief.high);
        let (own, wide, floor, top) = if self.turned {
            (-shape, 1.0, -(2.0 * high + 2.0 * MARGIN), -waist_of(low))
        } else {
            (shape, self.wide(), floor_of(low), high)
        };
        let outside =
            Vec3::new(local.x.abs() - wide, local.y.abs() - wide, (floor - over).max(over - top));
        let boxed =
            (outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0)) * self.half / steep;
        own.max(boxed)
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let (low, high) = (self.relief.low, self.relief.high);
        let (floor, top) = if self.turned {
            (-(2.0 * high + 2.0 * MARGIN), -waist_of(low))
        } else {
            (floor_of(low), high + MARGIN)
        };
        let (least, most) =
            self.heights.values.iter().fold((0.0_f32, 0.0_f32), |(least, most), value| {
                (least.min(*value), most.max(*value))
            });
        let wide = self.wide();
        let mut found = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for corner in 0..8 {
            let side = |bit: u8, low: f32, high: f32| if corner & bit == 0 { low } else { high };
            let local = Vec3::new(
                side(1, -wide, wide),
                side(2, -wide, wide),
                side(4, floor + least, top + most),
            );
            let world = self.centre + self.frame * (local * self.half);
            found = (found.0.min(world), found.1.max(world));
        }
        found
    }

    /// As far as the level read keeps its own field.
    fn sure(&self) -> f32 {
        (OUTSIDE - 0.5) * self.scale()
    }
}

impl Join {
    /// Stamp a map into the body: its displacement, up and down, of the
    /// surface as it was before the stroke. Part of the stroke as a
    /// solid joined is: [`Join::clear`] takes it away, and one undo.
    /// Under a mask, by as much as the mask lets through.
    #[contracts::debug_requires(volume.is_recording(), "a stamp displaces the body as it was before the stroke, which is the open stroke's undo record")]
    pub fn stamp(&mut self, volume: &mut Volume, stamp: &Draped<'_>) {
        let voxel_size = volume.voxel_size();
        let (low, high) = stamp.bounds();
        if !(low.is_finite() && high.is_finite()) {
            return;
        }
        let pad = Vec3::splat(2.0 * voxel_size);
        let (lo, hi) = volume.voxel_bounds(low - pad, high + pad);
        volume.snapshot_before(lo, hi, &mut self.region);
        let region = &self.region;
        let decide = |preview: &BrickPreview| {
            if preview.mask == Some(PROTECTED) {
                return BrickVerdict::Skip;
            }
            let middle = (preview.coord.origin().as_vec3()
                + Vec3::splat(BRICK_DIM as f32 * 0.5 - 0.5))
                * voxel_size;
            // Every voxel of the brick is at least this far outside
            // what the stamp can change.
            if stamp.outside(middle) / voxel_size > BRICK_DIM as f32 * 0.866_025_4 {
                BrickVerdict::Skip
            } else {
                BrickVerdict::Whole
            }
        };
        volume.edit_voxels_where(
            lo,
            hi,
            0,
            true,
            crate::Settled::NONE,
            decide,
            |voxel, position, value, free| {
                if free <= 0.0 {
                    return value;
                }
                match stamp.displaced(position, region.get(voxel), value) {
                    Some(wanted) => value + (wanted.clamp(INSIDE, OUTSIDE) - value) * free,
                    None => value,
                }
            },
        );
        self.boxes.push((lo, hi));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::IVec3;

    const CENTRE: Vec3 = Vec3::splat(64.0);
    const RADIUS: f32 = 40.0;
    /// Half the stamp across, in voxels: the middle level of a relief,
    /// read at its own size.
    const HALF: f32 = 32.0;

    fn ball() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(CENTRE, RADIUS);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// The frame of a stamp on top of the fixture ball: across along X,
    /// up the square along -Z, out along Y.
    fn frame() -> Mat3 {
        Mat3::from_cols(Vec3::X, -Vec3::Z, Vec3::Y)
    }

    /// The fixture ball stamped on top with `relief`, and the heights it
    /// was draped on.
    fn stamped(relief: &Relief, turned: bool) -> Volume {
        let mut volume = ball();
        let top = CENTRE + Vec3::Y * RADIUS;
        let mut join = Join::new();
        volume.begin_stroke();
        let heights = Heights::under(&volume, relief, top, frame(), HALF);
        let stamp = Draped::on(relief, &heights, top, frame(), HALF, 1.0, turned).expect("a stamp");
        join.stamp(&mut volume, &stamp);
        drop(volume.end_stroke());
        volume
    }

    /// How far from the ball's middle the surface is, looking DOWN the
    /// stamp's normal at a point of its square: the first solid met from
    /// well above.
    fn top_at(volume: &Volume, u: f32, v: f32) -> f32 {
        let foot = CENTRE + Vec3::Y * RADIUS + (frame().x_axis * u + frame().y_axis * v) * HALF;
        let mut along = 50.0;
        while along > -60.0 {
            if volume.sample_world(foot + Vec3::Y * along) < 0.0 {
                return along + 0.25;
            }
            along -= 0.25;
        }
        panic!("nothing under ({u}, {v})");
    }

    /// How far the ball's own surface is below the stamp's plane there.
    fn ball_at(u: f32, v: f32) -> f32 {
        let out = Vec2::new(u, v).length() * HALF;
        (RADIUS * RADIUS - out * out).sqrt() - RADIUS
    }

    /// A map is read the way a picture is laid: its first row is the top
    /// of the square, what is not square is read square, the rim is
    /// nothing, and what is not a map is refused.
    #[test]
    fn a_map_is_read_from_displacements() {
        // Four across and three down, each texel holding where it is.
        let vectors: Vec<[f32; 3]> = (0..3)
            .flat_map(|row| (0..4).map(move |column| [column as f32, row as f32, 1.0]))
            .collect();
        let map = VectorMap::from_vectors(4, 3, &vectors).expect("a map");
        assert_eq!(map.size(), 4);
        // Row 0 of the picture is the top of the square.
        assert!((map.vectors[3 * 4].y - 0.0).abs() < 1.0e-5, "the top row is not the first read");
        assert!((map.vectors[0].y - 2.0).abs() < 1.0e-5);
        assert!((map.vectors[1].x - 1.0).abs() < 1.0e-5);
        assert_eq!(map.sample(-1.0, 0.0), Vec3::ZERO, "the rim displaces something");
        assert_eq!(map.sample(1.2, 0.0), Vec3::ZERO);
        assert!(map.sample(0.0, 0.0).z > 0.5);
        assert_eq!(VectorMap::from_vectors(2, 9, &vectors), Err(MapError::TooSmall));
        assert_eq!(
            VectorMap::from_vectors(4, 4, &vectors),
            Err(MapError::WrongCount { expected: 16, found: 12 })
        );
        let mut broken = vectors.clone();
        broken[5][1] = f32::NAN;
        assert_eq!(VectorMap::from_vectors(4, 3, &broken), Err(MapError::NotFinite));
        let nothing = VectorMap::from_fn(16, |_, _| Vec3::ZERO);
        assert!(matches!(Relief::of(&nothing), Err(MapError::NoShape)));
    }

    /// Each map that comes with the application is a shape that leans
    /// over itself: somewhere there is air with the shape over it and
    /// the plate under it, which a height cannot say.
    #[test]
    fn the_maps_that_come_with_it_lean_over_themselves() {
        for kind in BuiltInMap::ALL {
            let relief = Relief::of(&kind.map()).expect("a shape");
            let (low, high) = relief.span();
            assert!(low > -0.01 && high > 0.2, "{kind:?} is between {low} and {high}");
            assert_eq!(relief.levels.len(), LEVELS.len());
            let level = &relief.levels[1];
            let half = level.across as i32 / 2;
            let mut hangs = 0;
            for y in -half..=half {
                for x in -half..=half {
                    // Up from the plate: solid, then air, then solid again.
                    let mut changes = Vec::new();
                    let mut last = true;
                    for z in -1..(high * half as f32) as i32 + 3 {
                        let solid = level.body.sample_voxel(IVec3::new(x, y, z)) < 0.0;
                        if solid != last {
                            changes.push(z);
                            last = solid;
                        }
                    }
                    if changes.len() >= 3 {
                        hangs += 1;
                    }
                }
            }
            assert!(hangs > 20, "{kind:?} hangs over itself at only {hangs} places");
        }
    }

    /// A stud stamped on a ball: it stands as high over the ball as the
    /// map says, its cap hangs over air, and at the rim of the square,
    /// where the map displaces nothing, the ball is where it was. No
    /// plinth, though the ball curves away under the square by several
    /// voxels.
    #[test]
    fn a_stamp_is_draped_on_the_surface() {
        let relief = Relief::of(&BuiltInMap::Stud.map()).expect("a shape");
        let volume = stamped(&relief, false);
        let (_, high) = relief.span();
        let top = top_at(&volume, 0.0, 0.0);
        assert!(
            (top - high * HALF).abs() < 1.5,
            "the stud stands {top} voxels over the ball and the map says {}",
            high * HALF
        );
        // The ball has curved away by this much at the rim of the square.
        assert!(ball_at(0.95, 0.0) < -10.0, "the fixture's ball is too flat to show a plinth");
        for (u, v) in
            [(0.95, 0.0), (-0.95, 0.0), (0.0, 0.95), (0.0, -0.95), (0.65, 0.65), (0.6, -0.6)]
        {
            let (found, wanted) = (top_at(&volume, u, v), ball_at(u, v));
            assert!(
                (found - wanted).abs() < 0.8,
                "at ({u}, {v}) the surface is at {found} and the ball was at {wanted}"
            );
        }
        // Where the map displaces nothing the body is exactly as it was:
        // past 0.93 of the way to the rim, every voxel, to the bit.
        let untouched = ball();
        let top_of_ball = CENTRE + Vec3::Y * RADIUS;
        let mut changed = 0;
        for z in 0..130 {
            for y in 60..130 {
                for x in 0..130 {
                    let voxel = IVec3::new(x, y, z);
                    let local = frame().transpose() * (voxel.as_vec3() - top_of_ball) / HALF;
                    let out = Vec2::new(local.x, local.y).length();
                    if out > 0.93
                        && volume.sample_voxel(voxel).to_bits()
                            != untouched.sample_voxel(voxel).to_bits()
                    {
                        changed += 1;
                    }
                }
            }
        }
        assert_eq!(changed, 0, "the body changed where the map displaces nothing");
        // Under the cap, outside the stem: air, with the cap over it.
        let under_cap = CENTRE
            + Vec3::Y * (RADIUS + ball_at(0.4, 0.0) + 0.18 * HALF)
            + frame().x_axis * (0.4 * HALF);
        assert!(volume.sample_world(under_cap) > 0.0, "there is no air under the stud's cap");
        assert!(
            volume.sample_world(under_cap + Vec3::Y * (0.22 * HALF)) < 0.0,
            "there is no cap over that air"
        );
    }

    /// A map that digs, digs, and turned inside out it raises by as
    /// much: a stamp is the displaced surface both ways.
    #[test]
    fn a_stamp_digs_as_well_as_raises() {
        let pit = VectorMap::from_fn(65, |u, v| {
            let dome = (1.0 - (u * u + v * v) / 0.36).max(0.0);
            Vec3::new(0.0, 0.0, -0.2 * dome)
        });
        let relief = Relief::of(&pit).expect("a shape");
        let (low, high) = relief.span();
        assert!((low + 0.2).abs() < 0.01 && high.abs() < 1.0e-4);
        let dug = top_at(&stamped(&relief, false), 0.0, 0.0);
        assert!(
            (dug + 0.2 * HALF).abs() < 1.0,
            "the pit is {dug} deep and the map says {}",
            0.2 * HALF
        );
        let raised = top_at(&stamped(&relief, true), 0.0, 0.0);
        assert!((raised - 0.2 * HALF).abs() < 1.0, "turned inside out it stands {raised} high");
        for volume in [stamped(&relief, false), stamped(&relief, true)] {
            let (found, wanted) = (top_at(&volume, 0.9, 0.0), ball_at(0.9, 0.0));
            assert!((found - wanted).abs() < 0.8, "the rim moved to {found} from {wanted}");
        }
    }

    /// A stamp is part of its stroke: cleared it leaves the ball as it
    /// was, and one undo takes it.
    #[test]
    fn a_stamp_is_one_stroke() {
        let relief = Relief::of(&BuiltInMap::Horn.map()).expect("a shape");
        let untouched = ball();
        let same = |one: &Volume, other: &Volume| {
            (0..130).all(|z| {
                (0..150).all(|y| {
                    (0..130).all(|x| {
                        let voxel = IVec3::new(x, y, z);
                        one.sample_voxel(voxel).to_bits() == other.sample_voxel(voxel).to_bits()
                    })
                })
            })
        };
        let mut volume = ball();
        let top = CENTRE + Vec3::Y * RADIUS;
        let mut join = Join::new();
        volume.begin_stroke();
        let heights = Heights::under(&volume, &relief, top, frame(), HALF);
        let stamp = Draped::on(&relief, &heights, top, frame(), HALF, 1.0, false).expect("a stamp");
        join.stamp(&mut volume, &stamp);
        assert!(!same(&volume, &untouched));
        join.clear(&mut volume);
        assert!(same(&volume, &untouched), "a cleared stamp left something");
        drop(volume.end_stroke());
        let mut again = stamped(&relief, false);
        assert!(top_at(&again, 0.0, 0.0) > 5.0, "the horn does not stand on the ball");
        again.begin_stroke();
        drop(again.end_stroke());
    }

    /// What it costs to make a map a shape and to set one down, printed.
    #[test]
    #[ignore = "a measurement, not a test"]
    fn what_a_stamp_costs() {
        for kind in BuiltInMap::ALL {
            let started = web_time::Instant::now();
            let map = kind.map();
            let read = started.elapsed();
            let relief = Relief::of(&map).expect("a shape");
            println!(
                "{kind:?}: the map in {read:?}, its shape at three sizes in {:?}",
                started.elapsed() - read
            );
            let started = web_time::Instant::now();
            drop(stamped(&relief, false));
            println!("{kind:?}: stamped 64 voxels across in {:?}", started.elapsed());
        }
    }
}
