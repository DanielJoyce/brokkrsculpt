// SPDX-License-Identifier: AGPL-3.0-only

//! Severing a piece off a body with a disc: the engine of the ring cut.
//! The design is `docs/CUT-RING.md`.
//!
//! A plane cut is `max(field, plane)` through the whole body (`clip.rs`),
//! which is why a cut across an arm takes the torso beside it too. A disc is
//! a plane that stops at a radius, and it separates a piece only if it goes
//! all the way round the piece. So a disc is asked which piece it severs, if
//! any, before anything is cut.
//!
//! # The walk
//!
//! The solid is walked a voxel at a time, six neighbours a voxel, from both
//! faces of the disc at once, and never across it: a step from one voxel to
//! the next crosses the disc when the two are on opposite sides of its plane
//! and the point between them is within its radius. If the two walks meet,
//! the disc does not go all the way round and nothing is cut. Otherwise the
//! walk that runs out first is the piece, and it is the smaller side: the
//! arm, not the figure. The two take turns a few thousand voxels at a time,
//! so the walk costs about twice the smaller side. `d < 0.0` is solid, as it
//! is everywhere the field is classified (see `split.rs`).
//!
//! # The two bodies
//!
//! Each is a copy of the source ([`Volume::duplicated`], which carries the
//! paint and the layers) with the other side's solid raised to outside, as
//! `split.rs` makes its parts, and within a few voxels of the disc
//! `max(field, ±plane)`, so that each has a flat closed face exactly on the
//! plane. Solid the walk did not reach (a loose part of the body nowhere near
//! the disc) stays with the rest. A brick with nothing of a body's own near
//! it is dropped from that body.

use std::collections::VecDeque;

use glam::{IVec3, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::brick::{BRICK_DIM, BRICK_VOXELS, Brick, BrickCoord, NARROW_BAND, OUTSIDE, brick_index};

use crate::body::{Document, NodeId};
use crate::project::name_that_fits;
use crate::undo::{Change, Entry};
use crate::volume::Volume;

/// A cut that stops at a radius: a plane through `centre` square to
/// `normal`, within `radius` of `centre`. World units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Disc {
    pub centre: Vec3,
    pub normal: Vec3,
    pub radius: f32,
}

impl Disc {
    /// How far a point is in front of the plane, world units.
    pub fn side(&self, at: Vec3) -> f32 {
        (at - self.centre).dot(self.normal)
    }

    /// Whether a point is within the disc's radius, measured in its plane.
    pub fn within(&self, at: Vec3) -> bool {
        let offset = at - self.centre;
        let across = offset - self.normal * offset.dot(self.normal);
        across.length_squared() <= self.radius * self.radius
    }

    /// Two directions in the disc's plane, square to each other.
    pub fn axes(&self) -> (Vec3, Vec3) {
        self.normal.any_orthonormal_pair()
    }
}

/// What a disc does to a body.
pub enum Severed {
    /// It severs a piece: the piece, the smaller side, and the rest.
    Apart { piece: Box<Volume>, rest: Box<Volume> },
    /// It does not go all the way round: the two sides are joined past its
    /// rim.
    NotRound,
    /// It meets no solid on one side or the other.
    Misses,
}

/// How far either side of the plane the face is made exactly, in voxels:
/// the field's own band.
const FACE_BAND: f32 = NARROW_BAND;

/// How many voxels one walk takes before the other has its turn.
const TURN: usize = 4096;

/// Which walk reached a voxel.
const UNSEEN: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;

impl Volume {
    /// The disc a ring of `radius` (world units) round the point `at`, seen
    /// along `along`, proposes: square to the long axis of the solid under
    /// the ring, through the middle of it.
    ///
    /// The middle is half way between `at` and where the view's line leaves
    /// the solid behind it. The solid within `radius` of the middle is
    /// sampled on a grid of about forty a side, and its largest spread (the
    /// principal axis of its points) is the axis: an arm in a ring is a
    /// piece of cylinder, whose spread is along it. `None` when there is no
    /// solid there.
    pub fn disc_across(&self, at: Vec3, along: Vec3, radius: f32) -> Option<Disc> {
        let voxel = self.voxel_size();
        let along = along.normalize_or_zero();
        if along == Vec3::ZERO || radius <= 0.0 {
            return None;
        }
        // Through the solid along the view, to where it ends.
        let step = voxel * 0.5;
        let mut exit = at + along * step;
        let mut travelled = 0.0;
        while self.sample_world(exit) < 0.0 && travelled < radius * 4.0 {
            exit += along * step;
            travelled += step;
        }
        let middle = (at + exit) * 0.5;

        let pitch = (radius * 2.0 / 40.0).max(voxel);
        let reach = (radius / pitch).ceil() as i32;
        let mut points = Vec::new();
        for z in -reach..=reach {
            for y in -reach..=reach {
                for x in -reach..=reach {
                    let offset = Vec3::new(x as f32, y as f32, z as f32) * pitch;
                    if offset.length_squared() > radius * radius {
                        continue;
                    }
                    let point = middle + offset;
                    if self.sample_world(point) < 0.0 {
                        points.push(point);
                    }
                }
            }
        }
        if points.len() < 4 {
            return None;
        }
        let mean = points.iter().copied().sum::<Vec3>() / points.len() as f32;
        let mut covariance = glam::Mat3::ZERO;
        for point in &points {
            let d = *point - mean;
            covariance += glam::Mat3::from_cols(d * d.x, d * d.y, d * d.z);
        }
        let axis = principal_axis(covariance).unwrap_or(along);
        Some(Disc { centre: middle, normal: axis, radius })
    }

    /// Sever the piece `disc` cuts off, if it cuts one off. See the module
    /// documentation.
    pub fn sever(&self, disc: &Disc) -> Severed {
        let voxel = self.voxel_size();
        let disc = Disc { normal: disc.normal.normalize_or(Vec3::Z), ..*disc };
        let solid = |at: IVec3| self.sample_voxel(at) < 0.0;
        let world = |at: IVec3| at.as_vec3() * voxel;
        let front_of = |at: IVec3| disc.side(world(at)) >= 0.0;
        // A step crosses the disc where its ends are on opposite sides and
        // the point half way between them is within the radius.
        let crosses = |from: IVec3, to: IVec3| {
            front_of(from) != front_of(to) && disc.within((world(from) + world(to)) * 0.5)
        };

        // The seeds: solid voxels within a voxel of the disc, on each side.
        let mut labels = Labels::default();
        let mut queues = [VecDeque::new(), VecDeque::new()];
        let (u, v) = disc.axes();
        let span = (disc.radius / voxel).ceil() as i32;
        for j in -span..=span {
            for i in -span..=span {
                let on = disc.centre + (u * i as f32 + v * j as f32) * voxel;
                if !disc.within(on) {
                    continue;
                }
                // The voxels round the point within a voxel of the plane,
                // on both sides: the eight corners of one cell found only
                // one side where the plane lay exactly on a lattice plane.
                let base = (on / voxel).round().as_ivec3();
                for near in NEAR {
                    let cell = base + near;
                    if disc.side(world(cell)).abs() > voxel
                        || !solid(cell)
                        || labels.get(cell) != UNSEEN
                    {
                        continue;
                    }
                    let side = if front_of(cell) { FRONT } else { BACK };
                    labels.set(cell, side);
                    queues[usize::from(side - 1)].push_back(cell);
                }
            }
        }
        if queues.iter().any(VecDeque::is_empty) {
            return Severed::Misses;
        }

        // A brick solid throughout and away from the disc is walked whole:
        // one step for 32 768 voxels. The first version walked every voxel
        // and took 1.5 s to sever the ball the application opens with
        // through its middle, and 76 s at a quarter of the voxel.
        let interior = |coord: BrickCoord| {
            matches!(self.brick(coord), Some(Brick::Uniform(value)) if *value < 0.0)
                && !near_disc(&disc, coord, voxel)
        };
        // Step into a voxel from one already walked: whole, if its brick is
        // interior, or alone. `Err` when the other walk is there already.
        let enter = |at: IVec3,
                     side: u8,
                     labels: &mut Labels,
                     queue: &mut VecDeque<Step>|
         -> Result<(), ()> {
            let coord = BrickCoord::containing(at);
            if interior(coord) {
                match labels.whole.get(&coord) {
                    None => {
                        labels.whole.insert(coord, side);
                        queue.push_back(Step::Brick(coord));
                    }
                    Some(found) if *found != side => return Err(()),
                    Some(_) => {}
                }
                return Ok(());
            }
            if !solid(at) {
                return Ok(());
            }
            match labels.get(at) {
                UNSEEN => {
                    labels.set(at, side);
                    queue.push_back(Step::Voxel(at));
                }
                found if found != side => return Err(()),
                _ => {}
            }
            Ok(())
        };

        // The two walks, taking turns, until one runs out.
        let mut steps: [VecDeque<Step>; 2] = [
            queues[0].drain(..).map(Step::Voxel).collect(),
            queues[1].drain(..).map(Step::Voxel).collect(),
        ];
        let piece = 'walk: loop {
            for side in [FRONT, BACK] {
                let queue = &mut steps[usize::from(side - 1)];
                for _ in 0..TURN {
                    let Some(step) = queue.pop_front() else {
                        break 'walk side;
                    };
                    let met = match step {
                        // The brick the voxel is in, flooded from it through
                        // its own values with no lookups, and what steps out
                        // of its faces handed on. A voxel at a time through
                        // the map, the walk was 1.6 s of the 1.8 a ball's
                        // severing took.
                        Step::Voxel(at) => {
                            let mut out = Vec::new();
                            let coord = BrickCoord::containing(at);
                            match self.flood_brick(coord, at, side, &disc, &mut labels, &mut out) {
                                Err(()) => Err(()),
                                Ok(()) => out.into_iter().try_for_each(|(from, next)| {
                                    if crosses(from, next) {
                                        return Ok(());
                                    }
                                    enter(next, side, &mut labels, queue)
                                }),
                            }
                        }
                        Step::Brick(coord) => {
                            // Into each face's neighbour: whole if it is
                            // interior too, a face of voxels if not.
                            STEPS.iter().try_for_each(|step| {
                                let beside = BrickCoord(coord.0 + *step);
                                if interior(beside) {
                                    let corner = beside.origin();
                                    return enter(corner, side, &mut labels, queue);
                                }
                                if self.brick(beside).is_none() {
                                    return Ok(());
                                }
                                face(coord, *step).try_for_each(|from| {
                                    let next = from + *step;
                                    if crosses(from, next) {
                                        return Ok(());
                                    }
                                    enter(next, side, &mut labels, queue)
                                })
                            })
                        }
                    };
                    if met.is_err() {
                        return Severed::NotRound;
                    }
                }
                if queue.is_empty() {
                    break 'walk side;
                }
            }
        };
        // The other walk was stopped part way: what it had not reached of
        // its own side is the rest's anyway, as is any solid neither reached.
        let sign = if piece == FRONT { 1.0 } else { -1.0 };
        let piece_body = self.severed_side(&disc, &labels, piece, sign, true);
        let rest_body = self.severed_side(&disc, &labels, piece, -sign, false);
        Severed::Apart { piece: Box::new(piece_body), rest: Box::new(rest_body) }
    }

    /// Flood the solid of one brick from `start`, already labelled `side`,
    /// within the brick: through its values and its labels directly, the
    /// disc tested only where the brick is near it. Each step out of a face
    /// goes into `out`, from and to, for the walk to take on. `Err` where
    /// the other walk has been.
    fn flood_brick(
        &self,
        coord: BrickCoord,
        start: IVec3,
        side: u8,
        disc: &Disc,
        labels: &mut Labels,
        out: &mut Vec<(IVec3, IVec3)>,
    ) -> Result<(), ()> {
        let voxel = self.voxel_size();
        let Some(brick) = self.brick(coord) else {
            return Ok(());
        };
        let value = |index: usize| match brick {
            Brick::Uniform(value) => *value,
            Brick::Dense(data) => data[index],
        };
        let near = near_disc(disc, coord, voxel);
        let origin = coord.origin();
        let world = |at: IVec3| at.as_vec3() * voxel;
        let crosses = |from: IVec3, to: IVec3| {
            near && (disc.side(world(from)) >= 0.0) != (disc.side(world(to)) >= 0.0)
                && disc.within((world(from) + world(to)) * 0.5)
        };
        let marks = labels.bricks.entry(coord).or_insert_with(|| {
            vec![UNSEEN; BRICK_VOXELS].into_boxed_slice().try_into().expect("BRICK_VOXELS")
        });
        let last = BRICK_DIM as i32 - 1;
        let start = start - origin;
        let mut stack = vec![start];
        while let Some(local) = stack.pop() {
            for step in STEPS {
                let next = local + step;
                if next.min_element() < 0 || next.max_element() > last {
                    out.push((origin + local, origin + next));
                    continue;
                }
                let index = brick_index(next.x as usize, next.y as usize, next.z as usize);
                if value(index) >= 0.0 || crosses(origin + local, origin + next) {
                    continue;
                }
                match marks[index] {
                    UNSEEN => {
                        marks[index] = side;
                        stack.push(next);
                    }
                    found if found != side => return Err(()),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// One of the two bodies: the piece's (`own` true) or the rest's.
    /// `keep` is the sign of the side of the plane this body keeps.
    fn severed_side(
        &self,
        disc: &Disc,
        labels: &Labels,
        piece: u8,
        keep: f32,
        own: bool,
    ) -> Volume {
        let voxel = self.voxel_size();
        let mut body = self.duplicated(IVec3::ZERO);
        // Cleared, as a parting clears it: a mask belongs to the body that
        // was, and an inverted empty mask would read as Mask All.
        let unmasked = body.mask().cleared(false);
        drop(body.replace_mask(unmasked));

        // Which bricks hold solid of this body's own, for which are kept.
        let is_own = |coord: BrickCoord, index: usize, value: f32| -> bool {
            if value >= 0.0 {
                return false;
            }
            (labels.at(coord, index) == piece) == own
        };
        let mut holding: FxHashSet<BrickCoord> = FxHashSet::default();
        for coord in self.brick_coords() {
            let holds = match self.brick(coord) {
                None => false,
                Some(Brick::Uniform(value)) => {
                    // A uniform brick the walk went through is wholly one
                    // side's; one it did not is the rest's.
                    *value < 0.0 && (labels.at(coord, 0) == piece) == own
                }
                Some(Brick::Dense(data)) => {
                    (0..BRICK_VOXELS).any(|index| is_own(coord, index, data[index]))
                }
            };
            if holds {
                holding.insert(coord);
            }
        }
        let near_disc = |coord: BrickCoord| near_disc(disc, coord, voxel);
        let kept = |coord: BrickCoord| {
            near_disc(coord)
                || (-1..=1).any(|z| {
                    (-1..=1).any(|y| {
                        (-1..=1)
                            .any(|x| holding.contains(&BrickCoord(coord.0 + IVec3::new(x, y, z))))
                    })
                })
        };

        let coords: Vec<BrickCoord> = body.brick_coords().collect();
        for coord in coords {
            if !kept(coord) {
                drop(body.take_brick(coord));
                continue;
            }
            let Some(mut brick) = body.take_brick(coord) else {
                continue;
            };
            // A uniform brick away from the disc is kept or dropped whole:
            // made dense, a solid interior is 128 KB a brick for nothing.
            if let Brick::Uniform(value) = brick
                && !near_disc(coord)
            {
                if value >= 0.0 || (labels.at(coord, 0) == piece) == own {
                    body.insert_brick(coord, brick);
                }
                continue;
            }
            let data = brick.make_dense();
            let origin = coord.origin();
            let mut all_outside = true;
            for (index, value) in data.iter_mut().enumerate() {
                let local = IVec3::new(
                    (index % BRICK_DIM) as i32,
                    ((index / BRICK_DIM) % BRICK_DIM) as i32,
                    (index / (BRICK_DIM * BRICK_DIM)) as i32,
                );
                let at = (origin + local).as_vec3() * voxel;
                let original = *value;
                let side = disc.side(at) / voxel;
                // Within a few voxels of the disc, the face: exactly on the
                // plane, and what is past it outside. Elsewhere the other
                // body's solid is outside.
                let out = if side.abs() <= FACE_BAND && disc.within(at) {
                    original.max(-keep * side).min(OUTSIDE)
                } else if original < 0.0 && !is_own(coord, index, original) {
                    OUTSIDE
                } else {
                    original
                };
                *value = out;
                all_outside &= out >= OUTSIDE;
            }
            if !all_outside {
                body.insert_brick(coord, brick);
            }
        }
        body.mark_everything_dirty();
        body
    }
}

/// Why a body was not severed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsevered {
    /// No such body.
    NoBody,
    /// The disc meets no solid on one side or the other.
    Misses,
    /// The disc does not go all the way round.
    NotRound,
}

/// A body severed in two: the piece, the rest, and the one undo entry.
pub struct SeverOutcome {
    pub piece: NodeId,
    pub rest: NodeId,
    pub entry: Entry,
}

impl Document {
    /// Sever the piece a disc cuts off a body into a body of its own, as one
    /// gesture: the piece and the rest take the source's place in the list,
    /// the piece made active, and the source goes into the entry so that
    /// undoing puts back exactly the body that was there.
    ///
    /// Ask [`Document::split_guard`] first, as for every split.
    pub fn sever(&mut self, source: NodeId, disc: &Disc) -> Result<SeverOutcome, Unsevered> {
        let at = self.index_of(source).ok_or(Unsevered::NoBody)?;
        let depth = self.nodes()[at].depth();
        let name = self.node(source).ok_or(Unsevered::NoBody)?.name.clone();
        let (piece, rest) = match self.volume(source).ok_or(Unsevered::NoBody)?.sever(disc) {
            Severed::Apart { piece, rest } => (*piece, *rest),
            Severed::NotRound => return Err(Unsevered::NotRound),
            Severed::Misses => return Err(Unsevered::Misses),
        };
        let mut changes = Vec::with_capacity(3);
        let mut bodies = Vec::with_capacity(2);
        for (offset, (label, volume)) in
            [(format!("{name} A"), piece), (format!("{name} B"), rest)].into_iter().enumerate()
        {
            let position = at + 1 + offset;
            let id = self.insert_body(position, depth, name_that_fits(&label), volume);
            changes.push(Change::NodeAdded { at: position, id });
            bodies.push(id);
        }
        let node = self.remove(at);
        changes.push(Change::NodeRemoved { at, node: Box::new(node) });
        self.set_active(bodies[0]);
        Ok(SeverOutcome { piece: bodies[0], rest: bodies[1], entry: Entry::new(changes) })
    }
}

/// The largest eigenvector of a symmetric 3x3 matrix, by power iteration.
fn principal_axis(matrix: glam::Mat3) -> Option<Vec3> {
    let mut vector = Vec3::new(0.577, 0.577, 0.577);
    for _ in 0..64 {
        let next = matrix * vector;
        let length = next.length();
        if length <= f32::EPSILON {
            return None;
        }
        vector = next / length;
    }
    Some(vector)
}

/// The six neighbours of a voxel.
const STEPS: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

/// A voxel and the 26 round it.
const NEAR: [IVec3; 27] = {
    let mut near = [IVec3::ZERO; 27];
    let mut at = 0;
    while at < 27 {
        near[at] = IVec3::new((at % 3) as i32 - 1, ((at / 3) % 3) as i32 - 1, (at / 9) as i32 - 1);
        at += 1;
    }
    near
};

/// Which walk reached each voxel: a brick walked whole by one label, the
/// rest a voxel at a time.
#[derive(Default)]
struct Labels {
    whole: FxHashMap<BrickCoord, u8>,
    bricks: FxHashMap<BrickCoord, Box<[u8; BRICK_VOXELS]>>,
}

/// One step of a walk: a voxel, or a brick walked whole.
#[derive(Clone, Copy)]
enum Step {
    Voxel(IVec3),
    Brick(BrickCoord),
}

/// The voxels of a brick's face toward `toward`, one of [`STEPS`].
fn face(coord: BrickCoord, toward: IVec3) -> impl Iterator<Item = IVec3> {
    let last = BRICK_DIM as i32 - 1;
    let origin = coord.origin();
    let fixed = move |axis: i32| if axis > 0 { last } else { 0 };
    (0..BRICK_DIM as i32).flat_map(move |a| {
        (0..BRICK_DIM as i32).map(move |b| {
            let local = if toward.x != 0 {
                IVec3::new(fixed(toward.x), a, b)
            } else if toward.y != 0 {
                IVec3::new(a, fixed(toward.y), b)
            } else {
                IVec3::new(a, b, fixed(toward.z))
            };
            origin + local
        })
    })
}

/// Whether a brick is within a few voxels of the disc, where its voxels
/// are walked and written one at a time.
fn near_disc(disc: &Disc, coord: BrickCoord, voxel: f32) -> bool {
    let half = BRICK_DIM as f32 * 0.5 * voxel;
    let middle = (coord.origin().as_vec3() + Vec3::splat(BRICK_DIM as f32 * 0.5)) * voxel;
    let reach = half * 3.0_f32.sqrt() + FACE_BAND * voxel;
    disc.side(middle).abs() <= reach && {
        let offset = middle - disc.centre;
        let across = offset - disc.normal * offset.dot(disc.normal);
        across.length() <= disc.radius + reach
    }
}

impl Labels {
    fn get(&self, at: IVec3) -> u8 {
        let coord = BrickCoord::containing(at);
        if let Some(label) = self.whole.get(&coord) {
            return *label;
        }
        let local = at - coord.origin();
        self.bricks.get(&coord).map_or(UNSEEN, |brick| {
            brick[brick_index(local.x as usize, local.y as usize, local.z as usize)]
        })
    }

    /// The label of voxel `index` of a brick.
    fn at(&self, coord: BrickCoord, index: usize) -> u8 {
        if let Some(label) = self.whole.get(&coord) {
            return *label;
        }
        self.bricks.get(&coord).map_or(UNSEEN, |brick| brick[index])
    }

    fn set(&mut self, at: IVec3, label: u8) {
        let coord = BrickCoord::containing(at);
        let local = at - coord.origin();
        let brick = self.bricks.entry(coord).or_insert_with(|| {
            vec![UNSEEN; BRICK_VOXELS].into_boxed_slice().try_into().expect("BRICK_VOXELS")
        });
        brick[brick_index(local.x as usize, local.y as usize, local.z as usize)] = label;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body and an arm: a big ball, and a rod standing out of it along X
    /// with a smaller ball on its end.
    fn figure() -> Volume {
        let mut volume = Volume::new(0.25);
        volume.seed_sphere(Vec3::ZERO, 6.0);
        let mut arm = Volume::new(0.25);
        arm.edit_box(Vec3::new(3.0, -3.0, -3.0), Vec3::new(17.0, 3.0, 3.0), |position, _| {
            let along = (position.x - 10.0).abs() - 6.0;
            let round = Vec3::new(0.0, position.y, position.z).length() - 1.5;
            let rod = along.max(round);
            let end = (position - Vec3::new(16.0, 0.0, 0.0)).length() - 2.5;
            (rod.min(end) / 0.25).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        volume.begin_stroke();
        volume.union_from(&arm);
        drop(volume.end_stroke());
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// The figure as a document of one body, severed by a disc across its
    /// arm: two bodies in its place, the piece active, and one undo puts
    /// the figure back.
    #[test]
    fn a_body_severed_is_two_and_one_undo() {
        use crate::undo::History;
        let mut document = Document::from_volume(figure());
        let source = document.active();
        let disc = Disc { centre: Vec3::new(10.0, 0.0, 0.0), normal: Vec3::X, radius: 3.0 };
        let outcome = document.sever(source, &disc).expect("severed");
        assert_eq!(document.bodies().count(), 2);
        assert_eq!(document.active(), outcome.piece);
        let mut history = History::default();
        history.push(outcome.entry);
        let visible = vec![true; document.nodes().len()];
        let _ = history.undo(&mut document, &visible);
        assert_eq!(document.bodies().count(), 1);
        let small = Disc { radius: 1.0, ..disc };
        assert!(matches!(document.sever(source, &small).err(), Some(Unsevered::NotRound)));
    }

    fn solid_voxels(volume: &Volume) -> usize {
        let mut count = 0;
        for coord in volume.brick_coords() {
            match volume.brick(coord) {
                Some(Brick::Uniform(value)) if *value < 0.0 => count += BRICK_VOXELS,
                Some(Brick::Dense(data)) => {
                    count += data.iter().filter(|value| **value < 0.0).count()
                }
                _ => {}
            }
        }
        count
    }

    /// A disc across the arm severs the arm, the smaller side, and the two
    /// bodies together are the figure, a voxel's rounding at the face apart;
    /// each is printable.
    #[test]
    fn a_disc_across_the_arm_severs_the_arm() {
        let volume = figure();
        let disc = Disc { centre: Vec3::new(9.0, 0.0, 0.0), normal: Vec3::X, radius: 3.0 };
        let Severed::Apart { piece, rest } = volume.sever(&disc) else {
            panic!("the arm was not severed");
        };
        let (whole, arm, body) = (solid_voxels(&volume), solid_voxels(&piece), solid_voxels(&rest));
        assert!(arm < body, "the piece ({arm}) is not the smaller side ({body})");
        let face = (std::f32::consts::PI * 1.5 * 1.5 / 0.0625) as usize;
        assert!(
            (arm + body).abs_diff(whole) <= face,
            "{arm} and {body} do not add up to {whole} within a face of {face}"
        );
        assert!(
            piece.sample_world(Vec3::new(16.0, 0.0, 0.0)) < 0.0,
            "the hand is not on the piece"
        );
        assert!(piece.sample_world(Vec3::ZERO) > 0.0, "the body is on the piece");
        assert!(rest.sample_world(Vec3::ZERO) < 0.0, "the body is not on the rest");
        assert!(rest.sample_world(Vec3::new(12.0, 0.0, 0.0)) > 0.0, "the arm is on the rest");
        for (part, what) in [(&piece, "the piece"), (&rest, "the rest")] {
            let (_, report) = part.export_mesh();
            assert!(report.is_printable(), "{what} is not printable: {}", report.summary());
        }
    }

    /// A disc too small to go round the arm cuts nothing, and one off to
    /// the side, in empty space, misses.
    #[test]
    fn a_disc_that_does_not_go_round_cuts_nothing() {
        let volume = figure();
        let small = Disc { centre: Vec3::new(9.0, 0.0, 0.0), normal: Vec3::X, radius: 1.0 };
        assert!(matches!(volume.sever(&small), Severed::NotRound));
        let away = Disc { centre: Vec3::new(9.0, 20.0, 0.0), normal: Vec3::X, radius: 2.0 };
        assert!(matches!(volume.sever(&away), Severed::Misses));
    }

    /// A ring round the arm, seen from the front, proposes a disc square to
    /// the arm through its middle.
    #[test]
    fn a_ring_round_the_arm_proposes_a_disc_across_it() {
        let volume = figure();
        let at = Vec3::new(10.0, 0.0, 1.5);
        let disc = volume.disc_across(at, -Vec3::Z, 3.0).expect("solid under the ring");
        assert!(disc.normal.dot(Vec3::X).abs() > 0.95, "the disc faces {:?}", disc.normal);
        assert!(
            disc.centre.distance(Vec3::new(10.0, 0.0, 0.0)) < 0.6,
            "its middle is {:?}",
            disc.centre
        );
        assert!(matches!(volume.sever(&disc), Severed::Apart { .. }));
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(12))]

        /// For any disc across the arm, tilted up to a third of a right
        /// angle and anywhere along it: the arm is severed, it is the
        /// smaller side, and the two together are the figure, a face's
        /// rounding apart.
        #[test]
        fn any_disc_across_the_arm_severs_it_and_loses_nothing(
            x in 7.0f32..13.0,
            tilt in (-0.5f32..0.5, -0.5f32..0.5),
        ) {
            let volume = figure();
            let normal = Vec3::new(1.0, tilt.0, tilt.1).normalize();
            let disc = Disc { centre: Vec3::new(x, 0.0, 0.0), normal, radius: 3.5 };
            let Severed::Apart { piece, rest } = volume.sever(&disc) else {
                return Err(proptest::test_runner::TestCaseError::fail("not severed"));
            };
            let (whole, arm, body) = (solid_voxels(&volume), solid_voxels(&piece), solid_voxels(&rest));
            proptest::prop_assert!(arm < body);
            let face = (std::f32::consts::PI * 2.0 * 2.0 / 0.0625) as usize;
            proptest::prop_assert!((arm + body).abs_diff(whole) <= face, "{} + {} against {}", arm, body, whole);
        }
    }
}
