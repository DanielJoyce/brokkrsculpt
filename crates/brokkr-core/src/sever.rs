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

/// A cut that stops: a plane through `centre` square to `normal`, within
/// `radius` of `centre`, or, fitted to what it crosses
/// ([`Volume::fit_to_islands`]), within the cross-sections of the pieces it
/// is to cut and no more. World units.
#[derive(Debug, Clone, PartialEq)]
pub struct Disc {
    pub centre: Vec3,
    pub normal: Vec3,
    pub radius: f32,
    /// The cells of the plane the cut is made in, when it is fitted.
    pub region: Option<std::sync::Arc<Region>>,
    /// How the cut is bent away from its plane, when it is: stage 4 of the
    /// ring cut, a collar's line. `None` is flat.
    pub bend: Option<std::sync::Arc<Bend>>,
}

/// A cut bent away from its plane by a height at each of a few angles round
/// its middle, the line a shirt's collar makes round a neck.
///
/// The height at a point of the plane is the heights round the middle,
/// interpolated smoothly and periodically by angle (Catmull-Rom), eased in
/// from their mean at the middle to their full value at `reach` and beyond.
/// The ease is what keeps the surface smooth where every angle meets: at the
/// middle it is flat, at their mean height. World units.
#[derive(Debug, Clone, PartialEq)]
pub struct Bend {
    /// The angle 0 direction in the plane, and the quarter turn on from it.
    pub u: Vec3,
    pub v: Vec3,
    /// A height along the normal at each of `heights.len()` even angles,
    /// the first at `u`.
    pub heights: Vec<f32>,
    /// How far from the middle the heights take their full value.
    pub reach: f32,
}

impl Bend {
    /// The direction in the plane of the `k`th height.
    pub fn direction(&self, k: usize) -> Vec3 {
        let angle = std::f32::consts::TAU * k as f32 / self.heights.len().max(1) as f32;
        self.u * angle.cos() + self.v * angle.sin()
    }

    /// The heights' mean, which is the height at the middle.
    pub fn mean(&self) -> f32 {
        if self.heights.is_empty() {
            return 0.0;
        }
        self.heights.iter().sum::<f32>() / self.heights.len() as f32
    }

    /// The height round the middle at an angle, between the heights.
    fn around(&self, angle: f32) -> f32 {
        let count = self.heights.len();
        if count == 0 {
            return 0.0;
        }
        let at = angle.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * count as f32;
        let first = at.floor() as usize % count;
        let t = at - at.floor();
        let height = |offset: isize| {
            self.heights[(first as isize + offset).rem_euclid(count as isize) as usize]
        };
        let (p0, p1, p2, p3) = (height(-1), height(0), height(1), height(2));
        0.5 * (2.0 * p1
            + (p2 - p0) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
            + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
    }

    /// The height at a point of the plane, given as its offset from the
    /// middle within the plane.
    pub fn lift(&self, across: Vec3) -> f32 {
        let (x, y) = (across.dot(self.u), across.dot(self.v));
        let r = (x * x + y * y).sqrt();
        let mean = self.mean();
        if r <= f32::EPSILON {
            return mean;
        }
        let t = (r / self.reach.max(f32::EPSILON)).min(1.0);
        let ease = t * t * (3.0 - 2.0 * t);
        mean + (self.around(y.atan2(x)) - mean) * ease
    }

    /// The most any point of the cut is off the plane through the middle at
    /// the mean height, an upper bound: Catmull-Rom can overshoot the
    /// heights by up to an eighth of a step between them.
    fn span(&self) -> f32 {
        let mean = self.mean();
        let most = self.heights.iter().map(|h| (h - mean).abs()).fold(0.0, f32::max);
        most * 1.25
    }
}

/// The part of a disc's plane a fitted cut is made in: a grid square to
/// the normal round the centre, a cell a voxel, true where it cuts.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    u: Vec3,
    v: Vec3,
    pitch: f32,
    /// Cells from the centre to the grid's edge, each way.
    half: i32,
    cells: Vec<bool>,
}

impl Region {
    /// How many cells the cut is made in.
    fn area(&self) -> usize {
        self.cells.iter().filter(|cell| **cell).count()
    }

    fn index(&self, i: i32, j: i32) -> Option<usize> {
        let side = 2 * self.half + 1;
        (i.abs() <= self.half && j.abs() <= self.half)
            .then(|| ((j + self.half) * side + (i + self.half)) as usize)
    }

    fn at(&self, i: i32, j: i32) -> bool {
        self.index(i, j).is_some_and(|index| self.cells[index])
    }
}

impl Disc {
    /// How far a point is in front of the cut, world units: of its plane,
    /// less the bend's height there.
    pub fn side(&self, at: Vec3) -> f32 {
        (at - self.centre).dot(self.normal) - self.lift(at)
    }

    /// How far the cut is off its plane over a point: the bend's height
    /// there, zero when it is flat.
    pub fn lift(&self, at: Vec3) -> f32 {
        match &self.bend {
            None => 0.0,
            Some(bend) => {
                let offset = at - self.centre;
                bend.lift(offset - self.normal * offset.dot(self.normal))
            }
        }
    }

    /// The point of the cut over a point: moved along the normal onto it.
    pub fn on_cut(&self, at: Vec3) -> Vec3 {
        let offset = at - self.centre;
        let flat = at - self.normal * offset.dot(self.normal);
        flat + self.normal * self.lift(flat)
    }

    /// How far the cut can be off its plane anywhere: the reach a test of
    /// nearness to the plane is widened by.
    fn bent_by(&self) -> f32 {
        self.bend.as_ref().map_or(0.0, |bend| bend.mean().abs() + bend.span())
    }

    /// A disc of `radius` and no more.
    pub fn round(centre: Vec3, normal: Vec3, radius: f32) -> Self {
        Self { centre, normal, radius, region: None, bend: None }
    }

    /// The same cut, bent.
    pub fn bent(self, bend: Option<Bend>) -> Self {
        Self { bend: bend.map(std::sync::Arc::new), ..self }
    }

    /// Whether a point is within the cut, measured in its plane: within the
    /// radius, and in the fitted region when there is one.
    pub fn within(&self, at: Vec3) -> bool {
        let offset = at - self.centre;
        let across = offset - self.normal * offset.dot(self.normal);
        if across.length_squared() > self.radius * self.radius {
            return false;
        }
        match &self.region {
            None => true,
            Some(region) => region.at(
                (offset.dot(region.u) / region.pitch).round() as i32,
                (offset.dot(region.v) / region.pitch).round() as i32,
            ),
        }
    }

    /// Two directions in the disc's plane, square to each other.
    pub fn axes(&self) -> (Vec3, Vec3) {
        self.normal.any_orthonormal_pair()
    }
}

/// What fitting a disc to the islands under it found.
enum Fit {
    Fitted(Disc),
    /// No solid within the ring at all.
    Empty,
    /// Solid, but no island mostly within the ring.
    TooBig,
}

/// What a disc does to a body.
pub enum Severed {
    /// It severs a piece: the piece, the smaller side, and the rest.
    Apart {
        piece: Box<Volume>,
        rest: Box<Volume>,
        /// Whether the piece is on the side of the plane its normal points
        /// to.
        piece_in_front: bool,
    },
    /// It does not go all the way round: the two sides are joined past its
    /// rim.
    NotRound,
    /// It meets no solid on one side or the other.
    Misses,
}

/// How far either side of the plane the face is made exactly, in voxels:
/// the field's own band.
const FACE_BAND: f32 = NARROW_BAND;

/// How many steps a cut is slid along its normal each way, a radius in all.
const STEPS_EACH_WAY: i32 = 4;

/// How far an island is followed, in ring radii, before it is taken to be
/// more than a piece the ring is round.
const ISLAND_REACH: i32 = 3;

/// How much of an island must lie within the ring for it to be cut.
const ISLAND_SHARE: f32 = 0.4;

/// How many voxels round an island the fitted cut reaches.
const ISLAND_MARGIN: i32 = 2;

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
        // No further than the ring is wide: a knob seen with the body behind
        // it is not as deep as the body, and the middle belongs in the knob.
        while self.sample_world(exit) < 0.0 && travelled < radius * 2.0 {
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
        Some(Disc::round(middle, axis, radius))
    }

    /// The cut a ring round `at`, seen along `along`, proposes: the
    /// narrowest fitted cut ([`Volume::fit_to_islands`]) through what the
    /// ring is round.
    ///
    /// The directions tried are the solid's three axes under the ring
    /// ([`Volume::disc_across`]), the surface's normal where the ring was
    /// pressed, and the view; each is slid along itself a quarter of the
    /// radius at a time up to the radius either way. Of the cuts that fit
    /// and are necks (no wider than the slices beside them, not a graze off
    /// a flank, and round the ring's middle), the one with the smallest
    /// cross-section is chosen, a little against sliding far: on a limb
    /// that is square across it, and on a knob it is round its neck. A
    /// ball on its own has no neck and is round no part: `None`. The long axis alone, the first proposal, put a
    /// disc diagonally over the top of a knob and into the ball under it,
    /// which fitted nothing (seen on the screen, 2026-11-26).
    pub fn propose_cut(&self, at: Vec3, along: Vec3, facing: Vec3, radius: f32) -> Option<Disc> {
        let first = self.disc_across(at, along, radius)?;
        let axes = self.axes_under(first.centre, radius);
        let outward = self.outward_from(first.centre, radius);
        let mut directions: Vec<Vec3> = axes.into_iter().flatten().collect();
        directions.extend([facing, along].into_iter().filter_map(|way| way.try_normalize()));
        directions.extend(outward);
        // Each direction slid along itself: the cross-section at each step,
        // `None` where there is no solid within the ring and infinite where
        // the plane runs off past it.
        let mut necks: Option<(f32, Disc)> = None;
        for normal in directions {
            let slices: Vec<(f32, Option<f32>, Option<Disc>)> = (-STEPS_EACH_WAY..=STEPS_EACH_WAY)
                .map(|step| {
                    let offset = step as f32 / STEPS_EACH_WAY as f32 * radius;
                    let trial = Disc::round(first.centre + normal * offset, normal, radius);
                    match self.fit(&trial) {
                        Fit::Fitted(disc) => {
                            let area = disc.region.as_ref().map_or(0, |region| region.area());
                            (offset, Some(area as f32), Some(disc))
                        }
                        Fit::TooBig => (offset, Some(f32::INFINITY), None),
                        Fit::Empty => (offset, None, None),
                    }
                })
                .collect();
            if std::env::var("PROBE").is_ok() {
                eprintln!(
                    "dir {:?}: {:?}",
                    normal,
                    slices.iter().map(|(o, a, _)| (*o, *a)).collect::<Vec<_>>()
                );
            }
            for (at, (offset, area, disc)) in slices.iter().enumerate() {
                let (Some(area), Some(disc)) = (area, disc) else {
                    continue;
                };
                // Against cuts slanted to the way the piece grows out of
                // the body, where there is one: a dome on a ball had a slice
                // across its upper side, parallel to the screen, before this.
                let slant = outward.map_or(0.0, |out| 1.0 - normal.dot(out).abs());
                let score = area * (1.0 + 0.15 * offset.abs() / radius) * (1.0 + slant);
                // A neck: no wider than the slices either side of it. Past
                // the end of the solid is no slice at all, so the tip of a
                // knob, narrowing to nothing, is not a neck; past the range
                // tried counts as wider.
                let beside = |at: Option<usize>| match at.and_then(|at| slices.get(at)) {
                    None => f32::INFINITY,
                    Some((_, None, _)) => 0.0,
                    Some((_, Some(area), _)) => *area,
                };
                // And not a graze: a slice with the body past it on both
                // sides takes a cap off a flank, not the piece the ring is
                // round. Measured on a knob: its neck's slice, 329 cells,
                // lost to a slice off its side, 149, before this.
                let (before, after) = (beside(at.checked_sub(1)), beside(Some(at + 1)));
                // And round what the ring is round: through its middle, or
                // with the middle on the piece's side, the side where the
                // slices run out to nothing before they run into the body.
                // A slice down the flank of a leg, a narrow strip with the
                // body past it, beat the cut across both legs before this.
                let runs_out = |way: Vec<&(f32, Option<f32>, Option<Disc>)>| {
                    way.into_iter()
                        .find_map(|(_, area, _)| match area {
                            None => Some(true),
                            Some(area) if area.is_infinite() => Some(false),
                            Some(_) => None,
                        })
                        .unwrap_or(false)
                };
                let through = offset.abs() <= radius / STEPS_EACH_WAY as f32 + f32::EPSILON;
                let held = if *offset < 0.0 {
                    runs_out(slices[at + 1..].iter().collect())
                } else {
                    runs_out(slices[..at].iter().rev().collect())
                };
                // A neck, or a base: the last slice that fits before the body,
                // with the slices the other way running out to a tip. A spur
                // widens all the way to where it meets the body (a horn, a
                // nose, a ball blended on as a dome) and has no neck; its
                // base is where it comes off. Seen on the screen: a dome on
                // a ball was round no part before this.
                let graze = before.is_infinite() && after.is_infinite();
                let neck = *area <= before && *area <= after;
                let base = if before.is_infinite() {
                    runs_out(slices[at + 1..].iter().collect())
                } else if after.is_infinite() {
                    runs_out(slices[..at].iter().rev().collect())
                } else {
                    false
                };
                let narrow = !graze && (neck || base) && (through || held);
                if !narrow {
                    continue;
                }
                // How much piece the cut takes off: the slices on the piece's
                // side summed, to where they run out or into the body. The
                // smallest cut was a sliver shaved off a dome's flank, seen
                // on the screen; the cut that takes the most piece through
                // the least section is the dome's base.
                let toward_after = if before.is_infinite() {
                    true
                } else if after.is_infinite() {
                    false
                } else {
                    let sum = |way: Vec<&(f32, Option<f32>, Option<Disc>)>| {
                        way.into_iter()
                            .map_while(|(_, area, _)| area.filter(|area| area.is_finite()))
                            .sum::<f32>()
                    };
                    sum(slices[at + 1..].iter().collect())
                        >= sum(slices[..at].iter().rev().collect())
                };
                let piece: f32 = area * 0.5
                    + if toward_after {
                        slices[at + 1..]
                            .iter()
                            .map_while(|(_, area, _)| area.filter(|area| area.is_finite()))
                            .sum::<f32>()
                    } else {
                        slices[..at]
                            .iter()
                            .rev()
                            .map_while(|(_, area, _)| area.filter(|area| area.is_finite()))
                            .sum::<f32>()
                    };
                let score = score / piece.max(1.0);
                if necks.as_ref().is_none_or(|(best, _)| score < *best) {
                    necks = Some((score, disc.clone()));
                }
            }
        }
        // No neck is no part: the smallest slice anywhere was a cap off the
        // front of a ball the ring was drawn in the middle of.
        necks.map(|(_, disc)| disc)
    }

    /// The way a piece grows out of the body round `middle`, if there is
    /// one: from the middle of the solid within three radii of it toward
    /// `middle`. A spur or a dome on a body has the body's solid to one
    /// side; partway along a limb the solid is all round and there is none.
    fn outward_from(&self, middle: Vec3, radius: f32) -> Option<Vec3> {
        let voxel = self.voxel_size();
        let reach = radius * 3.0;
        let pitch = (reach * 2.0 / 30.0).max(voxel);
        let steps = (reach / pitch).ceil() as i32;
        let (mut sum, mut count) = (Vec3::ZERO, 0_usize);
        for z in -steps..=steps {
            for y in -steps..=steps {
                for x in -steps..=steps {
                    let offset = Vec3::new(x as f32, y as f32, z as f32) * pitch;
                    if offset.length_squared() <= reach * reach
                        && self.sample_world(middle + offset) < 0.0
                    {
                        sum += offset;
                        count += 1;
                    }
                }
            }
        }
        if count == 0 {
            return None;
        }
        let towards = -(sum / count as f32);
        (towards.length() > radius * 0.3).then(|| towards.normalize())
    }

    /// The three axes of the solid within `radius` of `middle`, the
    /// largest spread first.
    fn axes_under(&self, middle: Vec3, radius: f32) -> [Option<Vec3>; 3] {
        let voxel = self.voxel_size();
        let pitch = (radius * 2.0 / 40.0).max(voxel);
        let reach = (radius / pitch).ceil() as i32;
        let mut points = Vec::new();
        for z in -reach..=reach {
            for y in -reach..=reach {
                for x in -reach..=reach {
                    let offset = Vec3::new(x as f32, y as f32, z as f32) * pitch;
                    if offset.length_squared() <= radius * radius
                        && self.sample_world(middle + offset) < 0.0
                    {
                        points.push(middle + offset);
                    }
                }
            }
        }
        if points.len() < 4 {
            return [None; 3];
        }
        let mean = points.iter().copied().sum::<Vec3>() / points.len() as f32;
        let mut covariance = glam::Mat3::ZERO;
        for point in &points {
            let d = *point - mean;
            covariance += glam::Mat3::from_cols(d * d.x, d * d.y, d * d.z);
        }
        let Some(first) = principal_axis(covariance) else {
            return [None; 3];
        };
        // The next by taking the first out, and the last square to both.
        let without = covariance
            - glam::Mat3::from_cols(first * first.x, first * first.y, first * first.z)
                * first.dot(covariance * first);
        let second = principal_axis(without).map(|axis| {
            (axis - first * axis.dot(first)).normalize_or(first.any_orthogonal_vector())
        });
        let third = second.map(|second| first.cross(second).normalize_or_zero());
        [Some(first), second, third]
    }

    /// The keys a cut is to have: a parting's plan on the cut's plane
    /// ([`Volume::part_plan`]), and of its keys those within the cut, so
    /// that a cross-section the plane makes elsewhere (a torso beside the
    /// arm) gets none.
    ///
    /// A bent cut is planned on the plane at its mean height, and each key
    /// then moved along the normal onto the cut where it stands.
    pub fn plan_keys(&self, disc: &Disc, spec: crate::part::KeySpec) -> Vec<crate::part::Key> {
        let mean = disc.bend.as_ref().map_or(0.0, |bend| bend.mean());
        let point = disc.centre + disc.normal * mean;
        let plane = crate::clip::ClipPlane { point, normal: disc.normal };
        let plan = self.part_plan(plane, spec);
        plan.keys
            .into_iter()
            .filter(|key| disc.within(key.centre))
            .map(|key| crate::part::Key { centre: disc.on_cut(key.centre), ..key })
            .collect()
    }

    /// The disc fitted to the pieces it is drawn round: the cross-sections
    /// its plane makes of the solid (its islands) that lie mostly within
    /// its radius, each whole, a few voxels round, and nothing else.
    ///
    /// Asked for by the maintainer on 2026-11-26: the cut "should be the
    /// size that allows it to cut" and "only cut one leg", and a ring over
    /// both legs splits both. A leg a little wider than the ring drawn is
    /// mostly within it and is cut whole; the torso, which the same plane
    /// may cross beside it, is mostly outside it and is left alone; two
    /// legs in one ring are two islands, and both are cut. An island is
    /// followed at most three radii out: one that runs further (a plane
    /// along a torso) is not a piece the ring is round. `None` when no
    /// island qualifies.
    pub fn fit_to_islands(&self, disc: &Disc) -> Option<Disc> {
        match self.fit(disc) {
            Fit::Fitted(disc) => Some(disc),
            Fit::Empty | Fit::TooBig => None,
        }
    }

    /// [`Volume::fit_to_islands`], saying why there is no fit.
    fn fit(&self, disc: &Disc) -> Fit {
        let voxel = self.voxel_size();
        let normal = disc.normal.normalize_or(Vec3::Z);
        let (u, v) = normal.any_orthonormal_pair();
        let pitch = voxel;
        let ring = (disc.radius / pitch).ceil() as i32;
        let half = ring * ISLAND_REACH;
        let side = (2 * half + 1) as usize;
        let mut seen = vec![false; side * side];
        let index = |i: i32, j: i32| ((j + half) as usize) * side + (i + half) as usize;
        let solid = |i: i32, j: i32| {
            self.sample_world(disc.on_cut(disc.centre + (u * i as f32 + v * j as f32) * pitch))
                < 0.0
        };
        let in_ring = |i: i32, j: i32| i * i + j * j <= ring * ring;
        let mut cells = vec![false; side * side];
        let mut any = false;
        let mut met = false;
        for j in -ring..=ring {
            for i in -ring..=ring {
                if !in_ring(i, j) || seen[index(i, j)] || !solid(i, j) {
                    continue;
                }
                met = true;
                // One island, followed out from here.
                let mut island = Vec::new();
                let mut inside = 0_usize;
                let mut escaped = false;
                let mut stack = vec![(i, j)];
                seen[index(i, j)] = true;
                while let Some((a, b)) = stack.pop() {
                    island.push((a, b));
                    inside += usize::from(in_ring(a, b));
                    for (da, db) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (na, nb) = (a + da, b + db);
                        if na.abs() > half || nb.abs() > half {
                            escaped = true;
                            continue;
                        }
                        if seen[index(na, nb)] || !solid(na, nb) {
                            continue;
                        }
                        seen[index(na, nb)] = true;
                        stack.push((na, nb));
                    }
                }
                if escaped || (inside as f32) < island.len() as f32 * ISLAND_SHARE {
                    continue;
                }
                any = true;
                for (a, b) in island {
                    cells[index(a, b)] = true;
                }
            }
        }
        if !any {
            return if met { Fit::TooBig } else { Fit::Empty };
        }
        // A few voxels round each island, so that the face is cut cleanly
        // to the surface and past it.
        let mut grown = cells.clone();
        let mut furthest = 0.0_f32;
        for j in -half..=half {
            for i in -half..=half {
                if !cells[index(i, j)] {
                    continue;
                }
                for dj in -ISLAND_MARGIN..=ISLAND_MARGIN {
                    for di in -ISLAND_MARGIN..=ISLAND_MARGIN {
                        let (a, b) = (i + di, j + dj);
                        if a.abs() <= half && b.abs() <= half {
                            grown[index(a, b)] = true;
                            furthest = furthest.max(((a * a + b * b) as f32).sqrt());
                        }
                    }
                }
            }
        }
        Fit::Fitted(Disc {
            centre: disc.centre,
            normal,
            radius: (furthest + 1.0) * pitch,
            region: Some(std::sync::Arc::new(Region { u, v, pitch, half, cells: grown })),
            bend: disc.bend.clone(),
        })
    }

    /// Sever the piece `disc` cuts off, if it cuts one off. See the module
    /// documentation.
    pub fn sever(&self, disc: &Disc) -> Severed {
        let voxel = self.voxel_size();
        let disc = Disc { normal: disc.normal.normalize_or(Vec3::Z), ..disc.clone() };
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
                let on = disc.on_cut(on);
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
        Severed::Apart {
            piece: Box::new(piece_body),
            rest: Box::new(rest_body),
            piece_in_front: piece == FRONT,
        }
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
                    if original < 0.0 && !is_own(coord, index, original) {
                        // The other's solid: past the face, at its distance
                        // from the plane; on this side, a sliver of something
                        // the walk did not reach, which kept was a loose speck
                        // (seen as a fragments body on the screen).
                        let past = -keep * side;
                        if past > 0.0 { past.min(OUTSIDE) } else { OUTSIDE }
                    } else {
                        original.max(-keep * side).min(OUTSIDE)
                    }
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
    /// How many pieces were severed: two for a ring round two legs.
    pub pieces: usize,
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
        self.sever_keyed(source, disc, &[])
    }

    /// [`Document::sever`], and the two bodies keyed to each other by
    /// `keys` ([`Volume::plan_keys`], or moved from there): a peg on the
    /// body behind the cut's plane, its socket in the one in front, as a
    /// parting keys its halves (`part.rs`).
    pub fn sever_keyed(
        &mut self,
        source: NodeId,
        disc: &Disc,
        keys: &[crate::part::Key],
    ) -> Result<SeverOutcome, Unsevered> {
        let at = self.index_of(source).ok_or(Unsevered::NoBody)?;
        let depth = self.nodes()[at].depth();
        let name = self.node(source).ok_or(Unsevered::NoBody)?.name.clone();
        let voxel = self.voxel_size();
        let (piece, rest) = match self.volume(source).ok_or(Unsevered::NoBody)?.sever(disc) {
            Severed::Apart { piece, rest, piece_in_front } => {
                let (mut piece, mut rest) = (*piece, *rest);
                if !keys.is_empty() {
                    let (first, second) = if piece_in_front {
                        (&mut rest, &mut piece)
                    } else {
                        (&mut piece, &mut rest)
                    };
                    crate::part::put_keys(first, second, keys, voxel);
                }
                (piece, rest)
            }
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
        let mut entry = Entry::new(changes);
        // A ring round two legs severs both, one piece in two parts: each is
        // made a body of its own, in the same entry. See `split.rs`.
        let mut pieces = 1;
        // Only for two real parts or more: a speck is no leg, and its own
        // "fragments" body would only be clutter.
        if let Some(plan) = self.split_plan(bodies[0])
            && plan.bodies() - usize::from(plan.has_fragments()) >= 2
            && let Some(outcome) = self.split(plan)
        {
            pieces = outcome.bodies.len();
            entry = entry.followed_by(outcome.entry);
        }
        Ok(SeverOutcome { piece: self.active(), rest: bodies[1], pieces, entry })
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
    // A bent cut's height changes across a brick: widened by all of it.
    (disc.side(middle).abs() <= reach + disc.bent_by()) && {
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
        let disc = Disc::round(Vec3::new(10.0, 0.0, 0.0), Vec3::X, 3.0);
        let outcome = document.sever(source, &disc).expect("severed");
        assert_eq!(document.bodies().count(), 2);
        assert_eq!(document.active(), outcome.piece);
        let mut history = History::default();
        history.push(outcome.entry);
        let visible = vec![true; document.nodes().len()];
        let _ = history.undo(&mut document, &visible);
        assert_eq!(document.bodies().count(), 1);
        let small = Disc { radius: 1.0, ..disc.clone() };
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
        let disc = Disc::round(Vec3::new(9.0, 0.0, 0.0), Vec3::X, 3.0);
        let Severed::Apart { piece, rest, .. } = volume.sever(&disc) else {
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

    /// A bent cut: across the arm at x = 9, its +y side raised 2 along the
    /// arm. Near the arm's +y surface just past the plane is behind the cut
    /// and stays with the rest; at -y, where the bend is level, the same
    /// distance past the plane goes with the piece. Both are closed.
    #[test]
    fn a_bent_cut_severs_along_its_bend() {
        let volume = figure();
        let bend = Bend {
            u: Vec3::Y,
            v: Vec3::Z,
            heights: vec![2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            reach: 1.0,
        };
        let disc = Disc::round(Vec3::new(9.0, 0.0, 0.0), Vec3::X, 3.0).bent(Some(bend));
        let Severed::Apart { piece, rest, .. } = volume.sever(&disc) else {
            panic!("the arm was not severed by the bent cut");
        };
        let (raised, level) = (Vec3::new(10.0, 1.2, 0.0), Vec3::new(10.0, -1.2, 0.0));
        assert!(rest.sample_world(raised) < 0.0, "the raised side went with the piece");
        assert!(piece.sample_world(raised) > 0.0, "the raised side is on the piece");
        assert!(piece.sample_world(level) < 0.0, "the level side is not on the piece");
        assert!(rest.sample_world(level) > 0.0, "the level side stayed with the rest");
        for (part, what) in [(&piece, "the piece"), (&rest, "the rest")] {
            let (_, report) = part.export_mesh();
            assert!(report.is_printable(), "{what} is not printable: {}", report.summary());
        }
    }

    /// Keys planned on a bent cut stand on it: each key's middle is on the
    /// cut, not on the flat plane through the cut's middle.
    #[test]
    fn keys_on_a_bent_cut_stand_on_it() {
        use crate::part::KeySpec;
        let volume = figure();
        let bend = Bend {
            u: Vec3::Y,
            v: Vec3::Z,
            heights: vec![1.0, 0.5, 0.0, -0.5, -1.0, -0.5, 0.0, 0.5],
            reach: 0.5,
        };
        let disc = volume
            .fit_to_islands(&Disc::round(Vec3::new(10.0, 0.0, 0.0), Vec3::X, 3.0).bent(Some(bend)))
            .expect("the arm's island");
        assert!(disc.bend.is_some(), "the fit dropped the bend");
        let spec = KeySpec {
            radius_mm: 0.4,
            depth_mm: 0.75,
            clearance_mm: 0.1,
            wall_mm: 0.3,
            ..KeySpec::peg()
        };
        let keys = volume.plan_keys(&disc, spec);
        assert!(!keys.is_empty(), "no key on the bent face");
        for key in keys {
            assert!(disc.side(key.centre).abs() < 1e-3, "a key off the cut at {}", key.centre);
        }
    }

    /// A disc too small to go round the arm cuts nothing, and one off to
    /// the side, in empty space, misses.
    #[test]
    fn a_disc_that_does_not_go_round_cuts_nothing() {
        let volume = figure();
        let small = Disc::round(Vec3::new(9.0, 0.0, 0.0), Vec3::X, 1.0);
        assert!(matches!(volume.sever(&small), Severed::NotRound));
        let away = Disc::round(Vec3::new(9.0, 20.0, 0.0), Vec3::X, 2.0);
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

    /// A body with two legs: a ball, and two rods down out of it, apart.
    fn two_legs() -> Volume {
        let voxel = 0.25;
        let mut volume = Volume::new(voxel);
        volume.edit_box(Vec3::new(-9.0, -18.0, -7.0), Vec3::new(9.0, 7.0, 7.0), move |at, _| {
            let body = at.length() - 6.0;
            let leg = |x: f32| {
                let along = (at.y + 10.0).abs() - 6.0;
                let round = Vec3::new(at.x - x, 0.0, at.z).length() - 1.5;
                along.max(round)
            };
            (body.min(leg(3.0)).min(leg(-3.0)) / voxel)
                .clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// A ring round both legs cuts both off, each a body of its own, and
    /// the figure keeps its body; one undo puts it back.
    #[test]
    fn a_ring_round_both_legs_severs_both() {
        use crate::undo::History;
        let volume = two_legs();
        let ring = Disc::round(Vec3::new(0.0, -10.0, 0.0), Vec3::Y, 5.0);
        let fitted = volume.fit_to_islands(&ring).expect("two islands in the ring");
        let mut document = Document::from_volume(volume);
        let source = document.active();
        let outcome = document.sever(source, &fitted).expect("severed");
        assert_eq!(outcome.pieces, 2, "the two legs are not two bodies");
        assert_eq!(document.bodies().count(), 3);
        let mut history = History::default();
        history.push(outcome.entry);
        let visible = vec![true; document.nodes().len()];
        let _ = history.undo(&mut document, &visible);
        assert_eq!(document.bodies().count(), 1);
    }

    /// A ring smaller than a leg still cuts that leg, whole, and only it:
    /// the cut is the leg's cross-section and not the ring's circle.
    #[test]
    fn a_ring_smaller_than_the_leg_cuts_that_leg_whole() {
        let volume = two_legs();
        let ring = Disc::round(Vec3::new(3.0, -10.0, 0.0), Vec3::Y, 1.0);
        assert!(matches!(volume.sever(&ring), Severed::NotRound), "the circle alone went round");
        let fitted = volume.fit_to_islands(&ring).expect("the leg's island");
        assert!(fitted.within(Vec3::new(4.3, -10.0, 0.0)), "the leg's edge is not in the cut");
        assert!(!fitted.within(Vec3::new(-3.0, -10.0, 0.0)), "the other leg is in the cut");
        let Severed::Apart { piece, rest, .. } = volume.sever(&fitted) else {
            panic!("the leg was not severed");
        };
        assert!(piece.sample_world(Vec3::new(3.0, -15.0, 0.0)) < 0.0, "the leg is not the piece");
        assert!(rest.sample_world(Vec3::new(-3.0, -15.0, 0.0)) < 0.0, "the other leg went too");
    }

    /// An island the ring is not round, a slab the same plane crosses beside
    /// the leg, reaching a little way into the ring and far past it, is not
    /// in the cut; the rod the ring is round is.
    #[test]
    fn a_big_island_beside_the_ring_is_left_out() {
        let voxel = 0.25;
        let mut volume = Volume::new(voxel);
        volume.edit_box(Vec3::new(-2.0, -12.0, -3.0), Vec3::new(22.0, 12.0, 6.0), move |at, _| {
            let rod = Vec3::new(0.0, at.y, at.z).length() - 1.5;
            let slab = (at - Vec3::new(10.0, 0.0, 3.2)).abs() - Vec3::new(10.0, 9.0, 1.0);
            let slab = slab.max(Vec3::ZERO).length() + slab.max_element().min(0.0);
            (rod.min(slab) / voxel).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        let ring = Disc::round(Vec3::new(10.0, 0.0, 0.0), Vec3::X, 2.5);
        assert!(ring.within(Vec3::new(10.0, 0.0, 2.4)), "the slab does not reach into the ring");
        assert!(volume.sample_world(Vec3::new(10.0, 0.0, 2.4)) < 0.0, "the slab is not there");
        let fitted = volume.fit_to_islands(&ring).expect("the rod's island");
        assert!(fitted.within(Vec3::new(10.0, 1.4, 0.0)), "the rod is not in the cut");
        assert!(!fitted.within(Vec3::new(10.0, 0.0, 2.6)), "the slab is in the cut");
        assert!(!fitted.within(Vec3::new(10.0, 5.0, 3.2)), "the slab is in the cut");
    }

    /// A knob on a ball, rung from the front: the cut proposed goes round
    /// its neck and severs the knob, the smaller side.
    #[test]
    fn a_ring_round_a_knob_cuts_round_its_neck() {
        let voxel = 0.25;
        let mut volume = Volume::new(voxel);
        volume.edit_box(Vec3::splat(-10.0), Vec3::new(14.0, 10.0, 10.0), move |at, _| {
            let ball = at.length() - 8.0;
            let knob = (at - Vec3::new(9.5, 0.0, 0.0)).length() - 2.5;
            (ball.min(knob) / voxel).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        let at = Vec3::new(9.5, 0.0, 2.5);
        let disc = volume.propose_cut(at, -Vec3::Z, Vec3::Z, 3.0).expect("a cut round the knob");
        assert!(disc.normal.dot(Vec3::X).abs() > 0.8, "the cut faces {:?}", disc.normal);
        let Severed::Apart { piece, rest, .. } = volume.sever(&disc) else {
            panic!("the knob was not severed");
        };
        assert!(piece.sample_world(Vec3::new(11.0, 0.0, 0.0)) < 0.0, "the knob is not the piece");
        assert!(rest.sample_world(Vec3::ZERO) < 0.0, "the ball is not the rest");
    }

    /// A ring pressed on one leg and drawn wide enough to take in the other
    /// proposes a cut through both.
    #[test]
    fn a_wide_ring_over_two_legs_proposes_both() {
        let volume = two_legs();
        let at = Vec3::new(3.0, -10.0, 1.5);
        let disc = volume.propose_cut(at, -Vec3::Z, Vec3::Z, 7.5).expect("a cut");
        assert!(
            disc.normal.dot(Vec3::Y).abs() > 0.8,
            "the cut faces {:?} at {:?}",
            disc.normal,
            disc.centre
        );
        let Severed::Apart { piece, .. } = volume.sever(&disc) else {
            panic!("not severed");
        };
        assert!(piece.sample_world(Vec3::new(3.0, -15.0, 0.0)) < 0.0, "the right leg is not cut");
        assert!(piece.sample_world(Vec3::new(-3.0, -15.0, 0.0)) < 0.0, "the left leg is not cut");
    }

    /// A dome blended onto a ball, which widens all the way to the ball and
    /// has no neck: the ring round it cuts it off at its base.
    #[test]
    fn a_ring_round_a_dome_cuts_it_at_its_base() {
        let voxel = 0.25;
        let mut volume = Volume::new(voxel);
        volume.edit_box(Vec3::splat(-10.0), Vec3::new(14.0, 10.0, 10.0), move |at, _| {
            let ball = at.length() - 8.0;
            let dome = (at - Vec3::new(8.5, 0.0, 0.0)).length() - 3.0;
            // A smooth union, so the dome flares into the ball.
            let k = 1.5;
            let h = (0.5 + 0.5 * (dome - ball) / k).clamp(0.0, 1.0);
            let joined = dome + (ball - dome) * h - k * h * (1.0 - h);
            (joined / voxel).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        let at = Vec3::new(9.5, 0.0, 2.6);
        let disc = volume.propose_cut(at, -Vec3::Z, Vec3::Z, 4.0).expect("a cut round the dome");
        let Severed::Apart { piece, rest, .. } = volume.sever(&disc) else {
            panic!("the dome was not severed");
        };
        assert!(piece.sample_world(Vec3::new(10.5, 0.0, 0.0)) < 0.0, "the dome is not the piece");
        assert!(rest.sample_world(Vec3::ZERO) < 0.0, "the ball is not the rest");
        assert!(piece.sample_world(Vec3::ZERO) > 0.0, "the ball went with the dome");
    }

    /// A ring cut keyed: the arm and the body, a peg on one and a socket
    /// in the other, sized so they fit, in one undo; keys are planned only
    /// within the cut.
    #[test]
    fn a_ring_cut_parts_the_arm_with_keys() {
        use crate::part::{KeyKind, KeySpec};
        let volume = figure();
        let disc = volume
            .fit_to_islands(&Disc::round(Vec3::new(10.0, 0.0, 0.0), Vec3::X, 3.0))
            .expect("the arm's island");
        let spec = KeySpec {
            radius_mm: 0.5,
            depth_mm: 0.75,
            clearance_mm: 0.1,
            wall_mm: 0.4,
            ..KeySpec::peg()
        };
        let keys = volume.plan_keys(&disc, spec);
        assert!(!keys.is_empty(), "no key on the arm's face");
        assert!(keys.iter().all(|key| disc.within(key.centre) && key.kind == KeyKind::Peg));
        let mut document = Document::from_volume(volume);
        let source = document.active();
        let outcome = document.sever_keyed(source, &disc, &keys).expect("severed");
        let key = keys[0];
        let peg_tip = key.centre + key.axis * (key.depth_mm * 0.5);
        let behind = key.centre - key.axis * (key.depth_mm * 0.5);
        let (piece, rest) = (
            document.volume(outcome.piece).expect("the piece"),
            document.volume(outcome.rest).expect("the rest"),
        );
        // The arm is in front of the plane (+X), so the body behind it has
        // the peg, standing out into where the arm's socket is.
        assert!(rest.sample_world(peg_tip) < 0.0, "the body has no peg");
        assert!(piece.sample_world(peg_tip) > 0.0, "the arm has no socket");
        assert!(rest.sample_world(behind) < 0.0, "the body's peg is not rooted");
    }

    proptest::proptest! {
        /// A bend passes through its heights: past its reach, in the
        /// direction of each, the height there is that height.
        #[test]
        fn a_bend_passes_through_its_heights(
            heights in proptest::collection::vec(-5.0_f32..5.0, 3..12),
            reach in 0.5_f32..4.0,
        ) {
            let bend = Bend { u: Vec3::X, v: Vec3::Y, heights: heights.clone(), reach };
            for (k, height) in heights.iter().enumerate() {
                let at = bend.direction(k) * reach * 1.5;
                proptest::prop_assert!((bend.lift(at) - height).abs() < 1e-3);
            }
        }

        /// A bend of one height everywhere is the plane moved along its
        /// normal by that height: the same side for any point.
        #[test]
        fn a_level_bend_is_the_plane_moved(
            height in -5.0_f32..5.0,
            count in 3_usize..12,
            point in proptest::array::uniform3(-10.0_f32..10.0),
        ) {
            let normal = Vec3::new(0.3, 0.8, -0.2).normalize();
            let (u, v) = normal.any_orthonormal_pair();
            let bend = Bend { u, v, heights: vec![height; count], reach: 2.0 };
            let centre = Vec3::new(1.0, -2.0, 0.5);
            let bent = Disc::round(centre, normal, 5.0).bent(Some(bend));
            let moved = Disc::round(centre + normal * height, normal, 5.0);
            let at = Vec3::from(point);
            proptest::prop_assert!((bent.side(at) - moved.side(at)).abs() < 1e-3);
        }
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
            let disc = Disc::round(Vec3::new(x, 0.0, 0.0), normal, 3.5);
            let Severed::Apart { piece, rest, .. } = volume.sever(&disc) else {
                return Err(proptest::test_runner::TestCaseError::fail("not severed"));
            };
            let (whole, arm, body) = (solid_voxels(&volume), solid_voxels(&piece), solid_voxels(&rest));
            proptest::prop_assert!(arm < body);
            let face = (std::f32::consts::PI * 2.0 * 2.0 / 0.0625) as usize;
            proptest::prop_assert!((arm + body).abs_diff(whole) <= face, "{} + {} against {}", arm, body, whole);
        }
    }
}
