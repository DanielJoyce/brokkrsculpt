// SPDX-License-Identifier: AGPL-3.0-only

//! Parting: one body into two that fit back together.
//!
//! A model too big for the bed, or one that would print better in two pieces
//! laid flat, has to be cut. The cut tool removes a side. Splitting keeps
//! both, and gives each a flush face with nothing to say how they go back
//! together. This keeps both AND gives them a way to register: a peg on one
//! face and a socket in the other, sized so that a real printer's parts
//! actually go together.
//!
//! # Why this is nearly free here
//!
//! Three properties of a distance field do all of it.
//!
//! * A plane cut is `max(field, plane)` and closes its own face, which is
//!   [`crate::clip`]'s whole argument. So the two halves are that, taken once
//!   with the plane and once with it reversed.
//! * Adding a solid is `min(field, solid)` and removing one is
//!   `max(field, -solid)`. A key is a closed form solid, a few lines of
//!   distance function, so a peg is a `min` and a socket is a `max`.
//! * **Subtracting a constant from a distance function gives the solid grown
//!   by that constant, exactly.** So the socket is not a second, slightly
//!   larger model of the peg. It is the peg's own distance function less the
//!   clearance, and the gap between them is the clearance everywhere: round
//!   the wall, under the tip, and round the rounded edge.
//!
//! A mesh modeller does the first two with boolean remeshing and the third by
//! modelling two parts and hoping.
//!
//! # Where the keys go
//!
//! [`Volume::part_plan`] looks at the cut face before anything is cut. The
//! face is whatever the plane's cross-section of the solid is, and that can
//! be several separate ISLANDS: a plane through a figure's shins is two.
//! Each island that is big enough gets a key at its widest point, which is
//! the centre of the largest circle that fits in it, found with a distance
//! transform of the face rasterised at voxel pitch.
//!
//! One round key stops two parts sliding and does not stop them turning, so
//! an island with room for a second gets a second. A body with two islands
//! does not need that: two keys anywhere fix an orientation.
//!
//! A key is checked against the solid it has to live in. The socket needs
//! wall round it and under it, so the solid is walked along the key's axis
//! and round its rim, and a key that would break out through a side or the
//! far face is made shallower, or left out and the island named. A key is
//! never placed that the plan does not also say is sound.
//!
//! # What the lattice can hold
//!
//! A flat or gently curved face is positioned to a fraction of a voxel, so a
//! clearance of a few tenths of a voxel is real. A sharp edge is not: it is
//! rounded over about a voxel. So every key has rounded edges by design, and
//! one narrower than [`MIN_KEY_VOXELS`] is refused with the voxel size that
//! would allow it, because below that the field cannot hold the shape at all
//! (`README.md` gives the same floor for a brush).
//!
//! # What this is not
//!
//! A cut by one plane. Freeform parting surfaces, dovetails, and proposing
//! where to cut are in the plan and not here.

use glam::{IVec3, Vec2, Vec3};

use crate::body::{Document, NodeId};
use crate::brick::{INSIDE, OUTSIDE};
use crate::clip::ClipPlane;
use crate::project::name_that_fits;
use crate::undo::{Change, Entry};
use crate::volume::Volume;

/// The narrowest key the lattice can hold, in voxels across.
pub const MIN_KEY_VOXELS: f32 = 3.0;

/// The most cells the cut face is rasterised into. Past this the pitch is
/// coarsened: the face is being measured for where a key of millimetres goes,
/// not meshed.
const MAX_FACE_CELLS: usize = 4_000_000;

/// What kind of key joins two parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// A tapered peg on one part and a socket in the other. Prints without
    /// support with the cut face down, and centres itself as it goes in.
    Peg,
    /// A hole in both, for a pin: a length of rod, or of filament.
    PinHole,
    /// A pocket in each face for a disc magnet, open at the face.
    MagnetPocket,
}

impl KeyKind {
    pub const ALL: [KeyKind; 3] = [KeyKind::Peg, KeyKind::PinHole, KeyKind::MagnetPocket];

    pub fn label(self) -> &'static str {
        match self {
            KeyKind::Peg => "Peg and socket",
            KeyKind::PinHole => "Pin hole",
            KeyKind::MagnetPocket => "Magnet pocket",
        }
    }
}

impl std::fmt::Display for KeyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The size and fit of a key.
///
/// All in millimetres. The clearance belongs to the printer and not to the
/// model, so the application keeps it with the machine's other settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeySpec {
    pub kind: KeyKind,
    /// Radius at the cut face. For a magnet pocket, the magnet's radius.
    pub radius_mm: f32,
    /// How far the key reaches into the second part. For a pin hole, into
    /// each part. For a magnet pocket, the magnet's thickness.
    pub depth_mm: f32,
    /// The gap left between a key and what it fits into, per side.
    pub clearance_mm: f32,
    /// The least solid left round and under a socket.
    pub wall_mm: f32,
}

impl KeySpec {
    /// A peg sized for a part of a few centimetres on a filament printer.
    pub fn peg() -> Self {
        Self { kind: KeyKind::Peg, radius_mm: 2.0, depth_mm: 3.0, clearance_mm: 0.2, wall_mm: 1.0 }
    }
}

/// How many places a second key is tried at before an island is left with
/// one.
const SECOND_KEY_TRIES: usize = 48;

/// How much narrower a peg is at its tip than at the face.
const PEG_TAPER: f32 = 0.8;

/// How far a peg is rooted into the part it grows from, in voxels, so that
/// the union is of two overlapping solids and not two that merely touch.
const PEG_ROOT_VOXELS: f32 = 2.0;

/// One key, placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    /// Where its axis meets the cut face.
    pub centre: Vec3,
    /// Its axis: the plane's normal, pointing from the first part into the
    /// second.
    pub axis: Vec3,
    pub kind: KeyKind,
    pub radius_mm: f32,
    pub depth_mm: f32,
    pub clearance_mm: f32,
}

/// Why an island of the cut face got no key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoKey {
    /// The largest circle that fits in it, less the wall, is narrower than a
    /// key can be made. Holds that circle's radius.
    TooNarrow { inscribed_mm: f32 },
    /// There is not enough solid behind the face for a socket and its wall.
    /// Holds how much there is.
    TooShallow { solid_mm: f32 },
}

/// One connected region of the cut face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Island {
    /// Its widest point: the centre of the largest circle that fits.
    pub widest: Vec3,
    /// The radius of that circle.
    pub inscribed_mm: f32,
    pub area_mm2: f32,
    /// How many keys it was given.
    pub keys: usize,
    /// Why not, when that is none.
    pub refused: Option<NoKey>,
}

/// What parting a body on a plane would do, worked out without doing it.
#[derive(Debug, Clone, PartialEq)]
pub struct PartPlan {
    pub plane: ClipPlane,
    pub islands: Vec<Island>,
    pub keys: Vec<Key>,
}

impl PartPlan {
    /// Whether the plane passes through the solid at all.
    pub fn cuts(&self) -> bool {
        !self.islands.is_empty()
    }

    /// One line for the status bar: what was found and what could not be
    /// keyed.
    pub fn summary(&self) -> String {
        if self.islands.is_empty() {
            return "the plane misses the model".to_string();
        }
        let unkeyed = self.islands.iter().filter(|island| island.keys == 0).count();
        let mut line = format!(
            "{} cut face{}, {} key{}",
            self.islands.len(),
            if self.islands.len() == 1 { "" } else { "s" },
            self.keys.len(),
            if self.keys.len() == 1 { "" } else { "s" },
        );
        if unkeyed > 0 {
            let narrowest = self
                .islands
                .iter()
                .filter(|island| island.keys == 0)
                .map(|island| island.inscribed_mm * 2.0)
                .fold(f32::INFINITY, f32::min);
            line += &format!(
                "; {unkeyed} too small to key (the narrowest is {narrowest:.1} mm across): \
                 glue only"
            );
        }
        line
    }
}

/// The two parts of a body, as nodes of the document.
pub struct PartOutcome {
    /// The part behind the plane, which carries the pegs.
    pub first: NodeId,
    /// The part in front of it, which carries the sockets.
    pub second: NodeId,
    /// One entry for the whole gesture, in [`crate::split`]'s order.
    pub entry: Entry,
}

/// A frame on a plane: a point of it and two unit vectors across it.
#[derive(Clone, Copy)]
struct Frame {
    origin: Vec3,
    u: Vec3,
    v: Vec3,
}

impl Frame {
    fn on(plane: ClipPlane, near: Vec3) -> Self {
        let normal = plane.normal;
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        // The point of the plane nearest the model's middle, so that the
        // raster is centred on what it is looking at.
        let origin = near - normal * plane.distance(near);
        Self { origin, u, v }
    }

    fn at(&self, on_face: Vec2) -> Vec3 {
        self.origin + self.u * on_face.x + self.v * on_face.y
    }
}

/// The cut face as a grid of cells: which are solid, and how far each is
/// from the nearest that is not.
struct Face {
    frame: Frame,
    pitch: f32,
    /// Cells across and down, and the cell coordinate of the frame's origin.
    size: (usize, usize),
    centre: (f32, f32),
    solid: Vec<bool>,
    /// Distance to the nearest cell that is not solid, in world units.
    inset: Vec<f32>,
}

impl Face {
    fn index(&self, x: usize, y: usize) -> usize {
        x + y * self.size.0
    }

    fn world(&self, x: usize, y: usize) -> Vec3 {
        self.frame.at(Vec2::new(
            (x as f32 - self.centre.0) * self.pitch,
            (y as f32 - self.centre.1) * self.pitch,
        ))
    }

    /// Rasterise the cross-section of `volume` on `plane`.
    fn of(volume: &Volume, plane: ClipPlane) -> Option<Self> {
        let (lo, hi) = volume.surface_bounds()?;
        let frame = Frame::on(plane, (lo + hi) * 0.5);
        // The face cannot reach further from the frame's origin than the
        // model's own corners do.
        let reach = (0..8)
            .map(|corner| {
                let at = Vec3::new(
                    if corner & 1 == 0 { lo.x } else { hi.x },
                    if corner & 2 == 0 { lo.y } else { hi.y },
                    if corner & 4 == 0 { lo.z } else { hi.z },
                );
                let offset = at - frame.origin;
                Vec2::new(offset.dot(frame.u), offset.dot(frame.v)).abs()
            })
            .fold(Vec2::ZERO, Vec2::max);

        let mut pitch = volume.voxel_size();
        let cells = |pitch: f32| {
            (
                (2.0 * reach.x / pitch).ceil() as usize + 5,
                (2.0 * reach.y / pitch).ceil() as usize + 5,
            )
        };
        while cells(pitch).0.saturating_mul(cells(pitch).1) > MAX_FACE_CELLS {
            pitch *= 2.0;
        }
        let size = cells(pitch);
        let centre = ((size.0 / 2) as f32, (size.1 / 2) as f32);

        let mut face = Self {
            frame,
            pitch,
            size,
            centre,
            solid: vec![false; size.0 * size.1],
            inset: Vec::new(),
        };
        let mut any = false;
        // The outermost ring of cells is left empty, so every island has an
        // edge inside the grid and the distance transform has something to
        // measure from.
        for y in 1..size.1 - 1 {
            for x in 1..size.0 - 1 {
                let inside = volume.sample_world(face.world(x, y)) < 0.0;
                let index = face.index(x, y);
                face.solid[index] = inside;
                any |= inside;
            }
        }
        if !any {
            return None;
        }
        face.inset = distance_transform(&face.solid, size.0, size.1)
            .into_iter()
            .map(|cells| cells * pitch)
            .collect();
        Some(face)
    }

    /// The islands of the face: connected regions of solid cells, each as its
    /// list of cells, largest first.
    fn islands(&self) -> Vec<Vec<usize>> {
        let mut label = vec![false; self.solid.len()];
        let mut islands = Vec::new();
        let mut stack = Vec::new();
        for start in 0..self.solid.len() {
            if !self.solid[start] || label[start] {
                continue;
            }
            let mut cells = Vec::new();
            label[start] = true;
            stack.push(start);
            while let Some(cell) = stack.pop() {
                cells.push(cell);
                let (x, y) = (cell % self.size.0, cell / self.size.0);
                // Four connected. The outer ring is never solid, so none of
                // these steps off the grid.
                for next in [
                    self.index(x - 1, y),
                    self.index(x + 1, y),
                    self.index(x, y - 1),
                    self.index(x, y + 1),
                ] {
                    if self.solid[next] && !label[next] {
                        label[next] = true;
                        stack.push(next);
                    }
                }
            }
            islands.push(cells);
        }
        // Largest first, and by first cell among equals, so the order does
        // not depend on anything but the face.
        islands.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
        islands
    }
}

/// Distance from every solid cell to the nearest that is not, in cells.
///
/// Felzenszwalb and Huttenlocher's exact Euclidean transform: a one
/// dimensional lower envelope of parabolas, run along the rows and then down
/// the columns. Linear in the number of cells.
fn distance_transform(solid: &[bool], width: usize, height: usize) -> Vec<f32> {
    const FAR: f32 = 1.0e20;
    let mut squared: Vec<f32> =
        solid.iter().map(|inside| if *inside { FAR } else { 0.0 }).collect();

    fn along(line: &mut [f32], scratch: &mut (Vec<usize>, Vec<f32>, Vec<f32>)) {
        let n = line.len();
        let (apex, boundary, out) = scratch;
        apex.clear();
        boundary.clear();
        out.clear();
        apex.push(0);
        boundary.push(f32::NEG_INFINITY);
        boundary.push(f32::INFINITY);
        for q in 1..n {
            loop {
                let p = *apex.last().expect("the envelope always holds one parabola");
                let meet = ((line[q] + (q * q) as f32) - (line[p] + (p * p) as f32))
                    / (2.0 * q as f32 - 2.0 * p as f32);
                if meet <= boundary[apex.len() - 1] {
                    apex.pop();
                    boundary.pop();
                    continue;
                }
                let k = apex.len();
                apex.push(q);
                boundary[k] = meet;
                boundary.push(f32::INFINITY);
                break;
            }
        }
        let mut k = 0;
        for q in 0..n {
            while boundary[k + 1] < q as f32 {
                k += 1;
            }
            let p = apex[k];
            let d = q as f32 - p as f32;
            out.push(d * d + line[p]);
        }
        line.copy_from_slice(out);
    }

    let mut scratch = (Vec::new(), Vec::new(), Vec::new());
    for row in squared.chunks_mut(width) {
        along(row, &mut scratch);
    }
    let mut column = vec![0.0; height];
    for x in 0..width {
        for (y, value) in column.iter_mut().enumerate() {
            *value = squared[x + y * width];
        }
        along(&mut column, &mut scratch);
        for (y, value) in column.iter().enumerate() {
            squared[x + y * width] = *value;
        }
    }
    squared.into_iter().map(f32::sqrt).collect()
}

/// Signed distance to a cone with its ends cut off, exactly.
///
/// `along` is the coordinate down its axis and `out` the distance from the
/// axis. It runs from `along = from` with radius `radius_from` to
/// `along = to` with radius `radius_to`. Inigo Quilez's formulation.
fn frustum(along: f32, out: f32, from: f32, to: f32, radius_from: f32, radius_to: f32) -> f32 {
    let half = (to - from) * 0.5;
    let q = Vec2::new(out, along - (from + to) * 0.5);
    let k1 = Vec2::new(radius_to, half);
    let k2 = Vec2::new(radius_to - radius_from, 2.0 * half);
    let ca =
        Vec2::new(q.x - q.x.min(if q.y < 0.0 { radius_from } else { radius_to }), q.y.abs() - half);
    let cb = q - k1 + k2 * ((k1 - q).dot(k2) / k2.length_squared()).clamp(0.0, 1.0);
    let sign = if cb.x < 0.0 && ca.y < 0.0 { -1.0 } else { 1.0 };
    sign * ca.length_squared().min(cb.length_squared()).sqrt()
}

impl Key {
    /// How round its edges are. A sharp edge is not something the lattice
    /// holds, and a rounded one goes into a socket more willingly.
    fn rounding(&self, voxel: f32) -> f32 {
        (0.5 * voxel).min(self.radius_mm * 0.25)
    }

    /// Signed distance to the key's own solid, in world units.
    ///
    /// For a peg: the peg, rooted a little way back into the first part. For
    /// the other two kinds: the hole or pocket, which runs both ways from the
    /// face.
    fn solid(&self, at: Vec3, voxel: f32) -> f32 {
        let offset = at - self.centre;
        let along = offset.dot(self.axis);
        let out = (offset - self.axis * along).length();
        let round = self.rounding(voxel);
        match self.kind {
            KeyKind::Peg => {
                let root = PEG_ROOT_VOXELS * voxel;
                let tip = self.radius_mm * PEG_TAPER;
                // The taper carried back through the root, so the peg's side
                // is one straight line through the face.
                let slope = (self.radius_mm - tip) / self.depth_mm;
                frustum(
                    along,
                    out,
                    -root,
                    self.depth_mm - round,
                    self.radius_mm + slope * root - round,
                    tip - round,
                ) - round
            }
            KeyKind::PinHole | KeyKind::MagnetPocket => {
                let radius = self.radius_mm - round;
                frustum(along, out, -self.depth_mm + round, self.depth_mm - round, radius, radius)
                    - round
            }
        }
    }

    /// The box of world space the key and its clearance can reach.
    fn bounds(&self, voxel: f32) -> (Vec3, Vec3) {
        let reach = self.radius_mm.max(self.depth_mm)
            + self.clearance_mm
            + (PEG_ROOT_VOXELS + 4.0) * voxel
            + self.depth_mm;
        (self.centre - Vec3::splat(reach), self.centre + Vec3::splat(reach))
    }
}

/// How far the solid runs from `from` along `direction`, up to `limit`.
fn solid_depth(volume: &Volume, from: Vec3, direction: Vec3, limit: f32) -> f32 {
    let step = volume.voxel_size() * 0.5;
    let mut travelled = 0.0;
    while travelled < limit {
        if volume.sample_world(from + direction * (travelled + step)) >= 0.0 {
            return travelled;
        }
        travelled += step;
    }
    limit
}

impl Volume {
    /// Union a closed form solid into this volume, over a world box.
    ///
    /// `solid` is a signed distance in world units, negative inside. The
    /// box must cover the solid and its narrow band.
    fn unite(&mut self, (lo, hi): (Vec3, Vec3), solid: impl Fn(Vec3) -> f32 + Sync) {
        let voxel = self.voxel_size();
        self.edit_box(lo, hi, move |position, value| {
            value.min((solid(position) / voxel).clamp(INSIDE, OUTSIDE))
        });
    }

    /// Remove a closed form solid from this volume, over a world box.
    fn carve(&mut self, (lo, hi): (Vec3, Vec3), solid: impl Fn(Vec3) -> f32 + Sync) {
        let voxel = self.voxel_size();
        self.edit_box(lo, hi, move |position, value| {
            value.max((-solid(position) / voxel).clamp(INSIDE, OUTSIDE))
        });
    }

    /// What parting this volume on `plane` would give, and where its keys
    /// would go. Reads only.
    ///
    /// The plane's normal points from the first part into the second. Pegs
    /// grow from the first; sockets are cut into the second.
    pub fn part_plan(&self, plane: ClipPlane, spec: KeySpec) -> PartPlan {
        let mut plan = PartPlan { plane, islands: Vec::new(), keys: Vec::new() };
        let Some(face) = Face::of(self, plane) else {
            return plan;
        };
        let voxel = self.voxel_size();
        // The narrowest a key may be: what the lattice can hold.
        let least_radius = MIN_KEY_VOXELS * voxel * 0.5;
        // A socket is the key grown by the clearance, and needs wall beyond
        // that.
        let margin = spec.clearance_mm + spec.wall_mm;

        for cells in face.islands() {
            let widest = *cells
                .iter()
                .max_by(|a, b| face.inset[**a].total_cmp(&face.inset[**b]).then(b.cmp(a)))
                .expect("an island has cells");
            let inscribed = face.inset[widest];
            let at = |cell: usize| face.world(cell % face.size.0, cell / face.size.0);
            let mut island = Island {
                widest: at(widest),
                inscribed_mm: inscribed,
                area_mm2: cells.len() as f32 * face.pitch * face.pitch,
                keys: 0,
                refused: None,
            };

            let radius = spec.radius_mm.min(inscribed - margin);
            if radius < least_radius {
                island.refused = Some(NoKey::TooNarrow { inscribed_mm: inscribed });
                plan.islands.push(island);
                continue;
            }
            match self.key_at(at(widest), plane.normal, radius, spec) {
                Ok(key) => {
                    plan.keys.push(key);
                    island.keys = 1;
                }
                Err(refused) => {
                    island.refused = Some(refused);
                    plan.islands.push(island);
                    continue;
                }
            }

            // A second, to stop the parts turning about the first, where the
            // island has room for one well clear of it: as far from the first
            // as a key of the same size still fits and still has solid behind
            // it. The furthest cell of all is on the island's rim, where the
            // face is at its narrowest and the solid behind it at its
            // thinnest, so candidates are tried from the furthest inward
            // until one is sound. A smaller second key is not offered: two
            // keys of different sizes are a part that can be assembled one
            // way round and is expected to go the other.
            let first = plan.keys[plan.keys.len() - 1];
            let apart = |cell: usize| at(cell).distance(first.centre);
            let mut candidates: Vec<usize> = cells
                .iter()
                .copied()
                .filter(|cell| face.inset[*cell] - margin >= first.radius_mm)
                .filter(|cell| apart(*cell) >= 2.0 * (first.radius_mm + margin))
                .collect();
            candidates.sort_by(|a, b| apart(*b).total_cmp(&apart(*a)).then(a.cmp(b)));
            // Not every cell: a face has tens of thousands, and neighbours
            // give the same answer.
            let stride = (candidates.len() / SECOND_KEY_TRIES).max(1);
            for cell in candidates.into_iter().step_by(stride) {
                if let Ok(key) = self.key_at(at(cell), plane.normal, first.radius_mm, spec)
                    && key.depth_mm == first.depth_mm
                {
                    plan.keys.push(key);
                    island.keys = 2;
                    break;
                }
            }
            plan.islands.push(island);
        }
        plan
    }

    /// A key of `radius` at `centre`, as deep as the solid allows, or why
    /// none can go there.
    fn key_at(&self, centre: Vec3, axis: Vec3, radius: f32, spec: KeySpec) -> Result<Key, NoKey> {
        let voxel = self.voxel_size();
        let u = axis.any_orthonormal_vector();
        let v = axis.cross(u);
        // The axis, and eight points round the rim of the socket's wall.
        let rim = radius + spec.clearance_mm + spec.wall_mm;
        let feelers: Vec<Vec3> = std::iter::once(Vec3::ZERO)
            .chain((0..8).map(|step| {
                let angle = step as f32 * std::f32::consts::FRAC_PI_4;
                (u * angle.cos() + v * angle.sin()) * rim
            }))
            .collect();
        let wanted = spec.depth_mm + spec.clearance_mm + spec.wall_mm;
        let depth_into = |direction: Vec3| {
            feelers
                .iter()
                .map(|offset| solid_depth(self, centre + *offset, direction, wanted))
                .fold(f32::INFINITY, f32::min)
        };

        // The second part has to hold the socket. For the two kinds that cut
        // into both, so does the first.
        let mut solid = depth_into(axis);
        if spec.kind != KeyKind::Peg {
            solid = solid.min(depth_into(-axis));
        }
        // Where the walk ran its whole length the key is as deep as asked,
        // and exactly: the subtraction below would hand back the same number
        // less a rounding error.
        let depth = if solid >= wanted {
            spec.depth_mm
        } else {
            spec.depth_mm.min(solid - spec.clearance_mm - spec.wall_mm)
        };
        // A key shallower than it is narrow at its narrowest does not locate
        // anything, and a magnet pocket shallower than the magnet is no use.
        let least = match spec.kind {
            KeyKind::MagnetPocket => spec.depth_mm,
            KeyKind::Peg | KeyKind::PinHole => (MIN_KEY_VOXELS * voxel).min(spec.depth_mm),
        };
        if depth < least {
            return Err(NoKey::TooShallow { solid_mm: solid });
        }
        Ok(Key {
            centre,
            axis,
            kind: spec.kind,
            radius_mm: radius,
            depth_mm: depth,
            clearance_mm: spec.clearance_mm,
        })
    }

    /// The two parts a plan describes.
    ///
    /// Both arrive unmasked, as the halves of every split do: a mask is a
    /// veto on change, and a part that kept one would have refused the cut
    /// that made it.
    pub fn parted(&self, plan: &PartPlan) -> (Volume, Volume) {
        let voxel = self.voxel_size();
        let reversed = ClipPlane { point: plan.plane.point, normal: -plan.plane.normal };
        let half = |plane: ClipPlane| {
            let mut volume = self.duplicated(IVec3::ZERO);
            // Cleared to NORMAL polarity, which `take_mask` does not do: it
            // leaves an empty mask of the polarity it found, and an empty
            // inverted mask is Mask All. A body parted under Mask All came
            // out of the first version of this as two whole copies of itself.
            let unmasked = volume.mask().cleared(false);
            drop(volume.replace_mask(unmasked));
            volume.clip(plane);
            volume
        };
        let mut first = half(plan.plane);
        let mut second = half(reversed);

        for key in &plan.keys {
            let bounds = key.bounds(voxel);
            let key = *key;
            let socket = move |at: Vec3| key.solid(at, voxel) - key.clearance_mm;
            match key.kind {
                KeyKind::Peg => {
                    first.unite(bounds, move |at| key.solid(at, voxel));
                    second.carve(bounds, socket);
                }
                KeyKind::PinHole | KeyKind::MagnetPocket => {
                    first.carve(bounds, socket);
                    second.carve(bounds, socket);
                }
            }
        }
        (first, second)
    }
}

impl Document {
    /// Part a body in two along a plan, as one gesture.
    ///
    /// The two parts take the source's place in the list, and the source goes
    /// into the entry, so undoing puts back exactly the body that was there.
    /// `None` when the id names no body or the plane misses it.
    ///
    /// Ask [`Document::split_guard`] first: like every split, this holds the
    /// source while it builds both parts.
    pub fn part(&mut self, source: NodeId, plan: &PartPlan) -> Option<PartOutcome> {
        if !plan.cuts() {
            return None;
        }
        let at = self.index_of(source)?;
        let depth = self.nodes()[at].depth();
        let name = self.node(source)?.name.clone();
        let (first, second) = self.volume(source)?.parted(plan);
        if first.brick_count() == 0 || second.brick_count() == 0 {
            return None;
        }

        let mut changes = Vec::with_capacity(3);
        let mut bodies = Vec::with_capacity(2);
        for (offset, (label, volume)) in
            [(format!("{name} A"), first), (format!("{name} B"), second)].into_iter().enumerate()
        {
            let position = at + 1 + offset;
            let id = self.insert_body(position, depth, name_that_fits(&label), volume);
            changes.push(Change::NodeAdded { at: position, id });
            bodies.push(id);
        }
        let node = self.remove(at);
        changes.push(Change::NodeRemoved { at, node: Box::new(node) });

        self.set_active(bodies[0]);
        Some(PartOutcome { first: bodies[0], second: bodies[1], entry: Entry::new(changes) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::assert_same_field;
    use crate::undo::{History, UndoOutcome};

    const CENTRE: Vec3 = Vec3::splat(40.0);

    fn ball(radius: f32, voxel: f32) -> Volume {
        let mut volume = Volume::new(voxel);
        volume.seed_sphere(CENTRE, radius);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// A plane through the fixtures' centre, tilted off every axis and off the
    /// lattice, so nothing about a result is owed to alignment.
    fn through_the_middle() -> ClipPlane {
        ClipPlane {
            point: CENTRE + Vec3::new(0.13, 0.31, -0.22),
            normal: Vec3::new(0.2, 1.0, 0.35).normalize(),
        }
    }

    /// Solid volume in cubic millimetres, from the fraction of each voxel the
    /// field says is inside.
    fn solid_mm3(volume: &Volume) -> f64 {
        let voxel = f64::from(volume.voxel_size());
        let (lo, hi) = volume.world_bounds().expect("the fixture has bricks");
        let (lo, hi) = volume.voxel_bounds(lo, hi);
        let mut cells = 0.0;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let d = volume.sample_voxel(IVec3::new(x, y, z));
                    cells += f64::from((0.5 - d).clamp(0.0, 1.0));
                }
            }
        }
        cells * voxel * voxel * voxel
    }

    #[test]
    fn a_ball_parts_into_two_printable_halves_that_add_up() {
        let volume = ball(20.0, 0.5);
        let plane = through_the_middle();
        let plan = volume.part_plan(plane, KeySpec::peg());
        assert!(plan.cuts());
        assert_eq!(plan.islands.len(), 1, "a ball's cross-section is one disc");
        let island = plan.islands[0];
        assert!(
            (island.inscribed_mm - 20.0).abs() < 0.8,
            "the disc through a 20 mm ball has an inscribed radius of {}",
            island.inscribed_mm
        );
        let disc = std::f32::consts::PI * 400.0;
        assert!((island.area_mm2 - disc).abs() < disc * 0.03, "its area is {}", island.area_mm2);
        assert!(island.widest.distance(plane.point) < 1.0, "its widest point is off centre");
        assert!(plane.distance(island.widest).abs() < 1.0e-3, "its widest point is off the plane");
        assert_eq!(island.keys, 2, "a disc this size has room for two keys");
        assert_eq!(plan.keys.len(), 2);
        // Two of a size, and well apart: far enough that each has its wall,
        // and far enough to be worth having as a stop against turning.
        let (one, other) = (plan.keys[0], plan.keys[1]);
        assert_eq!(one.radius_mm, other.radius_mm);
        assert_eq!(one.depth_mm, other.depth_mm);
        let spec = KeySpec::peg();
        let apart = one.centre.distance(other.centre);
        assert!(
            apart >= 2.0 * (one.radius_mm + spec.clearance_mm + spec.wall_mm),
            "the two keys are {apart} apart, which is inside each other's wall"
        );
        assert!(apart > 10.0, "the two keys are only {apart} apart on a 40 mm disc");
        assert!(plane.distance(other.centre).abs() < 1.0e-3, "the second key is off the plane");

        let (first, second) = volume.parted(&plan);
        for (part, what) in [(&first, "the first part"), (&second, "the second part")] {
            let (_, report) = part.export_mesh();
            assert!(report.is_printable(), "{what} is not printable: {}", report.summary());
        }

        // Nothing is lost and nothing is made but the keys: the two parts
        // hold the ball's volume, less the clearance round each socket.
        let whole = solid_mm3(&volume);
        let parts = solid_mm3(&first) + solid_mm3(&second);
        let pegs: f64 = plan
            .keys
            .iter()
            .map(|key| {
                // The shell of clearance round a peg: its side and its tip.
                let side = 2.0 * std::f64::consts::PI * f64::from(key.radius_mm * key.depth_mm);
                let tip = std::f64::consts::PI * f64::from(key.radius_mm * key.radius_mm);
                (side + tip) * f64::from(key.clearance_mm)
            })
            .sum();
        assert!(
            (whole - parts - pegs).abs() < pegs * 0.5 + whole * 0.001,
            "the ball is {whole:.0} mm3 and its parts {parts:.0}, with {pegs:.1} of clearance"
        );
    }

    #[test]
    fn the_peg_is_on_the_first_part_and_the_socket_in_the_second_with_the_clearance_between() {
        let voxel = 0.25;
        let volume = ball(16.0, voxel);
        let plane = through_the_middle();
        let spec = KeySpec { clearance_mm: 0.3, ..KeySpec::peg() };
        let plan = volume.part_plan(plane, spec);
        let (first, second) = volume.parted(&plan);
        let key = plan.keys[0];
        assert_eq!(key.axis, plane.normal);
        assert_eq!(key.radius_mm, spec.radius_mm);
        assert_eq!(key.depth_mm, spec.depth_mm);

        // Half way up the peg, on its axis: solid in the first part, which is
        // on the far side of the plane from where its own half lies.
        let mid = key.centre + key.axis * (key.depth_mm * 0.5);
        assert!(first.sample_world(mid) < 0.0, "there is no peg on the first part");
        assert!(second.sample_world(mid) > 0.0, "there is no socket in the second part");
        // Just past its tip there is clearance and then the second part again.
        let past = key.centre + key.axis * (key.depth_mm + spec.clearance_mm + 3.0 * voxel);
        assert!(first.sample_world(past) > 0.0, "the peg runs past its depth");
        assert!(second.sample_world(past) < 0.0, "the socket runs past the peg's tip and more");

        // The gap, measured: walk outward from the axis half way up the peg
        // and find where each part's surface is. The field's distances are in
        // voxels, so a zero crossing found by stepping is good to a fraction
        // of one.
        let across = key.axis.any_orthonormal_vector();
        let surface = |volume: &Volume, inside_first: bool| {
            let mut last = 0.0;
            let mut r = 0.0;
            while r < key.radius_mm + 2.0 {
                let d = volume.sample_world(mid + across * r);
                let prev = volume.sample_world(mid + across * last);
                if (prev < 0.0) != (d < 0.0) {
                    let _ = inside_first;
                    return last + (r - last) * prev.abs() / (prev.abs() + d.abs());
                }
                last = r;
                r += voxel * 0.25;
            }
            f32::NAN
        };
        let peg = surface(&first, true);
        let wall = surface(&second, false);
        let gap = wall - peg;
        assert!(
            (gap - spec.clearance_mm).abs() < 0.3 * voxel,
            "the peg's side is at {peg} and the socket's wall at {wall}: a gap of {gap}, \
             against a clearance of {}",
            spec.clearance_mm
        );
        // And the peg is tapered. Half way up it has lost half of what it
        // loses by the tip, which is a tenth of its radius.
        let expected = key.radius_mm * (1.0 + PEG_TAPER) * 0.5;
        assert!(
            (peg - expected).abs() < 0.3 * voxel,
            "the peg is {peg} wide half way up; a taper to {PEG_TAPER} of {} would make it \
             {expected}",
            key.radius_mm
        );
        assert!(
            expected < key.radius_mm - 0.5 * voxel,
            "the fixture's taper is too slight for this measurement to see"
        );
    }

    #[test]
    fn the_two_parts_never_occupy_the_same_space() {
        // If they did, they would not go together. Everywhere in a box round
        // the keys, at least one of the two is empty.
        let voxel = 0.5;
        let volume = ball(16.0, voxel);
        let plan = volume.part_plan(through_the_middle(), KeySpec::peg());
        let (first, second) = volume.parted(&plan);
        let mut both = 0;
        let mut looked = 0;
        for key in &plan.keys {
            let (lo, hi) = key.bounds(voxel);
            let (lo, hi) = volume.voxel_bounds(lo, hi);
            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        let at = IVec3::new(x, y, z);
                        looked += 1;
                        // Clear of the surface by a quarter voxel, so two
                        // faces that merely meet are not counted as overlap.
                        if first.sample_voxel(at) < -0.25 && second.sample_voxel(at) < -0.25 {
                            both += 1;
                        }
                    }
                }
            }
        }
        assert!(looked > 1000);
        assert_eq!(both, 0, "{both} voxels are solid in both parts");
    }

    #[test]
    fn each_island_of_the_cut_face_gets_its_own_key() {
        // Two balls in one body, apart, with a plane through both: the face
        // is two discs.
        let mut volume = Volume::new(0.5);
        let (left, right) = (CENTRE - Vec3::X * 14.0, CENTRE + Vec3::X * 14.0);
        volume.edit_box(Vec3::splat(4.0), Vec3::splat(76.0), |position, _| {
            let d = (position.distance(left) - 9.0).min(position.distance(right) - 9.0);
            (d / 0.5).clamp(INSIDE, OUTSIDE)
        });
        let plane = ClipPlane { point: CENTRE + Vec3::Y * 0.2, normal: Vec3::Y };
        let plan = volume.part_plan(plane, KeySpec::peg());
        assert_eq!(plan.islands.len(), 2, "{}", plan.summary());
        for island in &plan.islands {
            assert!(island.keys >= 1, "an island 18 mm across got no key: {island:?}");
            let nearest = island.widest.distance(left).min(island.widest.distance(right));
            assert!(
                nearest < 1.5,
                "an island's widest point is {nearest} from either ball's middle"
            );
        }
        // One key to each ball, at least.
        for ball in [left, right] {
            assert!(
                plan.keys.iter().any(|key| key.centre.distance(ball) < 9.0),
                "no key was placed on the ball at {ball}"
            );
        }
        let (first, second) = volume.parted(&plan);
        for part in [&first, &second] {
            assert!(part.export_mesh().1.is_printable());
        }
        assert!(plan.summary().contains("2 cut faces"), "{}", plan.summary());
    }

    #[test]
    fn an_island_too_narrow_for_a_key_is_named_and_left_alone() {
        // A rod 3 mm across. A 2 mm radius peg with a millimetre of wall does
        // not fit, and neither does the narrowest key the lattice holds.
        let mut volume = Volume::new(0.5);
        volume.edit_box(Vec3::splat(4.0), Vec3::splat(76.0), |position, _| {
            let radial = Vec2::new(position.x - 40.0, position.z - 40.0).length() - 1.5;
            let ends = (position.y - 40.0).abs() - 20.0;
            (radial.max(ends) / 0.5).clamp(INSIDE, OUTSIDE)
        });
        let plane = ClipPlane { point: CENTRE, normal: Vec3::Y };
        let plan = volume.part_plan(plane, KeySpec::peg());
        assert_eq!(plan.islands.len(), 1);
        assert!(plan.keys.is_empty(), "a key was put in a 3 mm rod: {:?}", plan.keys);
        let island = plan.islands[0];
        assert_eq!(island.keys, 0);
        match island.refused {
            Some(NoKey::TooNarrow { inscribed_mm }) => {
                assert!((inscribed_mm - 1.5).abs() < 0.5, "it measured the rod at {inscribed_mm}");
            }
            other => panic!("the rod was refused as {other:?}"),
        }
        assert!(plan.summary().contains("too small to key"), "{}", plan.summary());
        // It still parts: two rods, flush, to be glued.
        let (first, second) = volume.parted(&plan);
        assert!(first.export_mesh().1.is_printable());
        assert!(second.export_mesh().1.is_printable());
    }

    #[test]
    fn an_island_with_room_for_one_key_gets_one() {
        // A rod 9 mm across. A 2 mm peg and its wall fit in the middle of
        // it, and nowhere else far enough from the middle for two not to
        // share a wall. A second key that overlapped the first would be one
        // odd shaped key, and a socket with a breach in it.
        let mut volume = Volume::new(0.25);
        volume.edit_box(Vec3::splat(20.0), Vec3::splat(60.0), |position, _| {
            let radial = Vec2::new(position.x - 40.0, position.z - 40.0).length() - 4.5;
            let ends = (position.y - 40.0).abs() - 12.0;
            (radial.max(ends) / 0.25).clamp(INSIDE, OUTSIDE)
        });
        let plan = volume.part_plan(ClipPlane { point: CENTRE, normal: Vec3::Y }, KeySpec::peg());
        assert_eq!(plan.islands.len(), 1);
        assert_eq!(plan.keys.len(), 1, "a 9 mm rod was given {} keys", plan.keys.len());
        assert_eq!(plan.islands[0].keys, 1);
        assert!(plan.keys[0].centre.distance(CENTRE) < 0.5, "the one key is not in the middle");
        assert_eq!(plan.keys[0].radius_mm, 2.0);
    }

    #[test]
    fn a_key_is_made_shallower_where_the_second_part_is_thin_and_left_out_where_it_is_too_thin() {
        // A slab, parted near one face: the second part is a thin plate.
        let slab = |thickness_above: f32| {
            let mut volume = Volume::new(0.5);
            volume.edit_box(Vec3::splat(4.0), Vec3::splat(76.0), move |position, _| {
                let across = (Vec2::new(position.x - 40.0, position.z - 40.0).abs()
                    - Vec2::splat(15.0))
                .max_element();
                let up = (position.y - (40.0 + thickness_above)).max(20.0 - position.y);
                (across.max(up) / 0.5).clamp(INSIDE, OUTSIDE)
            });
            volume
        };
        let plane = ClipPlane { point: CENTRE, normal: Vec3::Y };
        let spec = KeySpec::peg();

        // Plenty above the plane: the key is as deep as asked.
        let deep = slab(10.0).part_plan(plane, spec);
        assert!(!deep.keys.is_empty());
        assert_eq!(deep.keys[0].depth_mm, spec.depth_mm);

        // 3 mm above: a 3 mm peg would leave no wall, so it is shortened to
        // what leaves the wall.
        let thin = slab(3.0);
        let plan = thin.part_plan(plane, spec);
        assert!(!plan.keys.is_empty(), "{:?}", plan.islands);
        let key = plan.keys[0];
        assert!(key.depth_mm < spec.depth_mm, "the key is still {} deep", key.depth_mm);
        let floor = 3.0 - key.depth_mm - spec.clearance_mm;
        assert!(
            floor >= spec.wall_mm - 0.5,
            "a key {} deep in a 3 mm plate leaves {floor} over it",
            key.depth_mm
        );
        let (_, second) = thin.parted(&plan);
        assert!(second.export_mesh().1.is_printable());
        // The socket does not break out through the plate's far face.
        let above = key.centre + Vec3::Y * 2.9;
        assert!(second.sample_world(above) < 0.0, "the socket broke through the plate");

        // 1.5 mm above: nothing fits, and the island says why.
        let plan = slab(1.5).part_plan(plane, spec);
        assert!(plan.keys.is_empty(), "a key was put in a 1.5 mm plate");
        assert!(
            matches!(plan.islands[0].refused, Some(NoKey::TooShallow { .. })),
            "{:?}",
            plan.islands[0].refused
        );
    }

    #[test]
    fn a_key_is_as_deep_as_its_rim_allows_and_not_as_its_middle_does() {
        // The cap of a ball. On the key's axis there is all the solid a full
        // depth socket and its wall need, 4.3 mm of it; round the rim of the
        // socket's wall, 3.2 mm out, the cap has curved down to under 4. A
        // key sized from its axis alone would leave less than the wall over
        // its shoulder.
        let voxel = 0.25;
        let volume = ball(16.0, voxel);
        let spec = KeySpec::peg();
        let plane = ClipPlane { point: CENTRE + Vec3::Y * 11.7, normal: Vec3::Y };
        let plan = volume.part_plan(plane, spec);
        assert!(!plan.keys.is_empty(), "{:?}", plan.islands);
        let key = plan.keys[0];
        assert!(
            key.centre.distance(plane.point) < 0.5,
            "the first key is not at the middle of the cap"
        );
        assert!(
            key.depth_mm < spec.depth_mm - 0.1,
            "the key is {} deep, as if the cap were as thick at its rim as on its axis",
            key.depth_mm
        );
        // What is left over the shoulder of the socket is the wall, near
        // enough.
        let (_, second) = volume.parted(&plan);
        let shoulder = key.centre
            + Vec3::Y * (key.depth_mm + spec.clearance_mm)
            + Vec3::X * (key.radius_mm + spec.clearance_mm);
        let over = solid_depth(&second, shoulder, Vec3::Y, 5.0);
        assert!(
            over >= spec.wall_mm - 2.0 * voxel,
            "there is {over} mm over the socket's shoulder, against a wall of {}",
            spec.wall_mm
        );
    }

    #[test]
    fn a_masked_body_is_still_cut_and_its_parts_arrive_unmasked() {
        // Mask everything, which is one bool. A cut that answered to the mask
        // would leave both halves whole.
        let mut volume = ball(14.0, 0.5);
        let _ = volume.flip_mask_polarity();
        assert!(volume.mask_fill() > 0.99, "the fixture is not masked");
        let plan = volume.part_plan(through_the_middle(), KeySpec::peg());
        let (first, second) = volume.parted(&plan);
        let whole = solid_mm3(&volume);
        for (part, what) in [(&first, "first"), (&second, "second")] {
            let share = solid_mm3(part) / whole;
            assert!(
                (0.4..0.6).contains(&share),
                "the {what} part holds {share:.2} of the ball: the mask stopped the cut"
            );
            assert_eq!(part.mask_fill(), 0.0, "the {what} part arrived masked");
        }
    }

    #[test]
    fn a_pin_hole_and_a_magnet_pocket_cut_into_both_parts() {
        let voxel = 0.25;
        let volume = ball(14.0, voxel);
        let plane = through_the_middle();
        for kind in [KeyKind::PinHole, KeyKind::MagnetPocket] {
            let spec = KeySpec { kind, radius_mm: 1.5, depth_mm: 2.0, ..KeySpec::peg() };
            let plan = volume.part_plan(plane, spec);
            assert!(!plan.keys.is_empty(), "{kind:?}: {:?}", plan.islands);
            let (first, second) = volume.parted(&plan);
            let key = plan.keys[0];
            // A millimetre either side of the face, on the axis, both are
            // empty; well past the depth, both are solid again.
            for (part, side, what) in [(&first, -1.0, "first"), (&second, 1.0, "second")] {
                let inside_the_hole = key.centre + key.axis * (side * 1.0);
                let beyond_it = key.centre + key.axis * (side * (key.depth_mm + 1.5));
                assert!(
                    part.sample_world(inside_the_hole) > 0.0,
                    "{kind:?}: there is no hole in the {what} part"
                );
                assert!(
                    part.sample_world(beyond_it) < 0.0,
                    "{kind:?}: the hole in the {what} part runs past its depth"
                );
                assert!(part.export_mesh().1.is_printable(), "{kind:?}: the {what} part");
            }
        }
    }

    #[test]
    fn a_plane_that_misses_the_model_parts_nothing() {
        let volume = ball(10.0, 0.5);
        let plane = ClipPlane { point: CENTRE + Vec3::Y * 30.0, normal: Vec3::Y };
        let plan = volume.part_plan(plane, KeySpec::peg());
        assert!(!plan.cuts());
        assert!(plan.keys.is_empty());
        assert_eq!(plan.summary(), "the plane misses the model");
        let mut doc = Document::from_volume(volume);
        let body = doc.active();
        assert!(doc.part(body, &plan).is_none(), "a plane that misses still parted the body");
        assert_eq!(doc.body_count(), 1);
    }

    #[test]
    fn a_key_narrower_than_the_lattice_holds_is_refused() {
        // At a 1 mm voxel the narrowest key is 3 mm across. Ask for one of
        // 1 mm radius on a face with all the room in the world.
        let volume = ball(20.0, 1.0);
        let spec = KeySpec { radius_mm: 1.0, ..KeySpec::peg() };
        let plan = volume.part_plan(through_the_middle(), spec);
        assert!(plan.keys.is_empty(), "a 2 mm key was placed on a 1 mm lattice");
        // The same request at a quarter of the voxel is granted.
        let finer = ball(20.0, 0.25).part_plan(through_the_middle(), spec);
        assert!(!finer.keys.is_empty());
        assert_eq!(finer.keys[0].radius_mm, 1.0);
    }

    #[test]
    fn the_distance_transform_is_the_distance() {
        // A filled disc: the transform at any cell is its distance to the rim.
        let (width, height) = (61, 53);
        let centre = (30.0_f32, 26.0_f32);
        let solid: Vec<bool> = (0..width * height)
            .map(|cell| {
                let (x, y) = ((cell % width) as f32, (cell / width) as f32);
                (x - centre.0).hypot(y - centre.1) <= 20.0
            })
            .collect();
        let inset = distance_transform(&solid, width, height);
        let mut checked = 0;
        for (cell, inside) in solid.iter().enumerate() {
            let (x, y) = ((cell % width) as f32, (cell / width) as f32);
            if !inside {
                assert_eq!(inset[cell], 0.0);
                continue;
            }
            let expected = 20.0 - (x - centre.0).hypot(y - centre.1);
            // To the nearest empty CELL, so it runs up to a cell and a bit
            // over the distance to the rim itself.
            assert!(
                inset[cell] >= expected && inset[cell] <= expected + 1.5,
                "at ({x}, {y}) the transform reads {} against {expected}",
                inset[cell]
            );
            checked += 1;
        }
        assert!(checked > 1000);
    }

    #[test]
    fn the_frustum_is_a_distance_function() {
        // Its value at a point is the distance to its nearest surface point:
        // zero on the surface, the right sign either side, and nowhere
        // changing faster than the point moves.
        let f = |along: f32, out: f32| frustum(along, out, -1.0, 3.0, 2.0, 1.2);
        assert!(f(1.0, 0.0) < 0.0, "the middle is not inside");
        assert!(f(5.0, 0.0) > 0.0 && f(1.0, 4.0) > 0.0, "outside is not outside");
        assert!((f(3.0, 0.5)).abs() < 1.0e-5, "the top face is not at zero");
        assert!((f(-1.0, 1.0)).abs() < 1.0e-5, "the bottom face is not at zero");
        assert!(
            (f(4.0, 0.0) - 1.0).abs() < 1.0e-5,
            "a unit above the top face reads {}",
            f(4.0, 0.0)
        );
        let mut worst: f32 = 0.0;
        for i in 0..40 {
            for j in 0..40 {
                let (a, o) = (-3.0 + i as f32 * 0.2, j as f32 * 0.12);
                let here = f(a, o);
                worst = worst.max((f(a + 0.01, o) - here).abs() / 0.01);
                worst = worst.max((f(a, o + 0.01) - here).abs() / 0.01);
            }
        }
        assert!(worst <= 1.0 + 1.0e-3, "it changes {worst} times as fast as the point moves");
    }

    #[test]
    fn parting_is_one_undo_entry_and_undoing_it_puts_the_body_back() {
        let original = ball(14.0, 0.5);
        let mut doc = Document::from_volume(ball(14.0, 0.5));
        let body = doc.active();
        let plan = doc.active_volume().part_plan(through_the_middle(), KeySpec::peg());
        let mut history = History::default();

        let outcome = doc.part(body, &plan).expect("the plane passes through the ball");
        assert_eq!(doc.body_count(), 2);
        assert!(doc.volume(body).is_none(), "the source is still in the document");
        assert_eq!(doc.active(), outcome.first);
        let names: Vec<String> = doc.nodes().iter().map(|node| node.name.clone()).collect();
        assert!(names[0].ends_with(" A") && names[1].ends_with(" B"), "{names:?}");
        history.push(outcome.entry);

        let visible = vec![true; doc.node_count()];
        let undone = history.undo(&mut doc, &visible);
        assert!(matches!(undone, UndoOutcome::Applied(_)), "there was nothing to undo");
        assert_eq!(doc.body_count(), 1, "undoing left {} bodies", doc.body_count());
        assert_same_field(
            doc.volume(body).expect("the source is back under its own id"),
            &original,
            "the body after undoing a part",
        );

        let visible = vec![true; doc.node_count()];
        let redone = history.redo(&mut doc, &visible);
        assert!(matches!(redone, UndoOutcome::Applied(_)), "there was nothing to redo");
        assert_eq!(doc.body_count(), 2);
        assert!(doc.volume(outcome.first).is_some() && doc.volume(outcome.second).is_some());
    }

    #[test]
    fn the_same_plan_twice_gives_the_same_parts() {
        let volume = ball(14.0, 0.5);
        let one = volume.part_plan(through_the_middle(), KeySpec::peg());
        let two = volume.part_plan(through_the_middle(), KeySpec::peg());
        assert_eq!(one, two);
        let (a, b) = volume.parted(&one);
        let (c, d) = volume.parted(&two);
        assert_same_field(&a, &c, "the first part");
        assert_same_field(&b, &d, "the second part");
    }
}
