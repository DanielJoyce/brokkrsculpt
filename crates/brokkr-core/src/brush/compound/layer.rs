// SPDX-License-Identifier: AGPL-3.0-only

//! A stroke that lays one layer: a brush with build-up off.
//!
//! `docs/STUDIO-PLAN.md` section 16.3. A brush builds up: every stamp works
//! on what the last one left, so a stroke that dwells, or crosses itself,
//! is taller there. That is what clay does and it is the default. With
//! build-up off a stroke lays **one layer**, as tall where it crossed
//! itself three times as where it passed once, and how tall is set by the
//! strength and not by how long the hand stayed.
//!
//! # How
//!
//! The way the paint brush lays its coat ([`crate::paint::stroke`]), and by
//! the same two means:
//!
//! * **Every stamp works from the field as it was before the stroke**, and
//!   not from what the stamps before it left. That field costs nothing to
//!   keep: it is the stroke's own undo record, read back
//!   ([`Volume::snapshot_before`]).
//! * **Each voxel keeps the most any stamp has given it**, and a stamp
//!   writes a voxel only when it gives it more. So a voxel holds what the
//!   one stamp that covered it best made of the field before the stroke,
//!   and a second pass over the same ground, which gives nothing more,
//!   writes nothing.
//!
//! One thing the paint brush did not need. A stamp is aimed at a surface,
//! and under a stroke that has already raised it the surface as it is NOW
//! is not the one the stroke was aimed at. A brush that levels to a plane
//! held off the surface would climb its own work, clay on clay, and each
//! stamp's plane would tilt with whatever the last one left. Seen on a
//! screen before this was understood, three passes of clay with build-up
//! off were a strip of rubble.
//!
//! **So the stamps of a layer are aimed at the surface as it was before
//! the stroke, and that is the caller's to do**: the centre where the
//! pointer's ray meets that surface ([`Volume::sample_world_before`]), and
//! the normal the way that surface faces
//! ([`Volume::gradient_world_before`]). The application does both. A
//! centre that is off that surface by some voxels all the same is put
//! back on it along the stamp's normal ([`settled`]), which is right for a
//! stamp that arrives square on its own work and is all this module can do
//! without knowing where the pointer is. (Working out the way the surface
//! used to face from the stamp, here, was built first. It had to treat any
//! difference between the stamp's normal and the surface as it is now as a
//! pen's lean, and so a second pass over a stroke was steered by the
//! first.)
//!
//! # A line is one footprint, swept
//!
//! As the paint brush found (section 15.4 of the plan): keep the most any
//! stamp gives each voxel, and the middle of a line gets all of a stamp
//! where a stamp landed and a little less half way to the next, which is
//! ribs along the stroke. Seen on a screen here too. So the stamps of a
//! line are joined: each one's footprint is the brush swept from where the
//! last was put down to where this one is, and a voxel is weighed by how
//! far it is from that step of the stroke's line and not from either end
//! of it. A brush set down in separate dabs ([`Layer::begin`]) is not
//! joined, which is what makes them separate.
//!
//! **What "one layer" can and cannot promise of a line.** A brush that
//! dwells, and a brush set down in dabs over the same ground in any order,
//! leave the field the same to the bit: each voxel is what the one stamp
//! that covered it best made of it. A line walked back over itself is one
//! layer too, no taller anywhere, but not to the bit: a step of the line
//! is measured across the way its LATER end faces, and walked the other
//! way that is the other end. On a surface that curves the two differ by
//! the turn of the surface between two stamps, and a few voxels at the
//! flanks take their value from the other stamp.
//!
//! # A layer's footprint is measured across the surface
//!
//! A brush that builds up weighs a voxel by how far it is from the stamp's
//! centre, in any direction: its footprint is a ball. For a stamp that
//! moves the surface a voxel that is the same as measuring across the
//! surface. A layer may be half a radius tall, and the top of it is then
//! half a radius from the centre however near the axis it is: weighed by
//! the ball, the layer falls short of what its strength says, by an eighth
//! at a strength of a half (2.19 voxels for 2.5, which is what the first
//! test of this found). So a layer weighs a voxel by how far it is from the
//! stamp's AXIS, and its footprint is a cylinder, cut off by a ball big
//! enough to hold the layer ([`REACH`]).
//!
//! # How tall a layer is
//!
//! A stamp of a brush that builds up moves the surface a voxel at most,
//! because stamps overlap and what each does compounds
//! ([`super::super::MAX_STAMP_VOXELS`]). A layer is laid once, from a clean
//! field, so that limit is not its limit:
//!
//! * a **push** moves the surface [`LAYER_DEPTH`] of the radius at full
//!   strength, in the middle of the stroke;
//! * a blend, toward the **plane** or the **mean**, and a **pinch**, are
//!   what [`LAYER_STAMPS`] stamps of the brush would have come to, which is
//!   how many a stroke lays across any one point at upstream's spacing. So
//!   clay with build-up off reaches its plane at a strength of a quarter
//!   and lays a strip of one thickness from there up;
//! * an **inflation** is an offset of the level set, and the field is only
//!   kept three voxels either side of the surface, so it is the push's
//!   depth and no more than [`MOST_OFFSET_VOXELS`].

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use contracts::debug_requires;
use glam::{IVec3, Vec3};
use rustc_hash::FxHashMap;

use super::super::alpha::Alpha;
use super::super::{
    BrushDirection, BrushScratch, PINCH_PULL_VOXELS, Stamp, Symmetry, admit,
    blend_toward_plane_clamps_back,
};
use super::{Compound, Frame, Side};
use crate::brick::{BRICK_DIM, BRICK_VOXELS, BrickCoord, INSIDE, OUTSIDE, brick_index};
use crate::mask::PROTECTED;
use crate::pattern::Prepared;
use crate::volume::{BrickPreview, BrickVerdict, Volume};

/// How far a push at full strength moves the surface in the middle of a
/// layer, as a share of the brush's radius.
///
/// Half: a layer as tall as its brush is wide would be a wall, and the
/// brush's reach is a ball of its radius, so the top of anything taller
/// than this is outside what the brush can write.
pub const LAYER_DEPTH: f32 = 0.5;

/// How many stamps of a brush that builds up a layer's blends are worth.
///
/// A stroke lays a stamp every quarter of a radius ([`super::super::Brush::spacing`]),
/// and with upstream's smooth falloff the weights of the stamps that reach
/// any one point on its line come to four: 1 at the point, and 0.844, 0.5
/// and 0.156 from the three either side. So this is one pass of the brush.
pub const LAYER_STAMPS: f32 = 4.0;

/// The most an inflation may offset the level set in one layer, in voxels.
///
/// The field is kept to three voxels either side of the surface
/// ([`OUTSIDE`]). An offset of more than that turns every clamped value
/// the brush can reach to the other sign, which is a ball of solid the
/// size of the brush and not an inflation.
pub const MOST_OFFSET_VOXELS: f32 = 2.0;

/// How far from a stamp's centre a layer's stamp may write, in radii: a
/// ball that holds a cylinder of the brush's radius and a layer either way
/// of its middle, with room for a curved surface to fall away under it.
const REACH: f32 = std::f32::consts::SQRT_2;

/// By how much a stamp has to give a voxel more than it has been given
/// before it writes it, as a share of all a stamp can give.
///
/// Not nothing, because a second pass over the same ground does not give
/// exactly what the first did: the same step of the line walked the other
/// way finds each voxel's foot on it a rounding apart. Without this a
/// pass that gives nothing more rewrote a scatter of voxels by the last
/// bit of their values, which is no harm to the surface and a whole
/// undo record's worth of bricks marked changed.
const MORE: f32 = 1.0e-4;

/// How near the axis of the brush a pinch may read from, as a share of how
/// far from it the voxel is. Less than all of it, so that a pinch gathers
/// the surface toward the axis and never reads from across it.
const MOST_PULL: f32 = 0.75;

/// What one stroke of a layer brush has given each voxel so far.
///
/// Kept for the length of a stroke: [`Layer::begin`] when it starts.
#[derive(Debug, Default)]
pub struct Layer {
    /// The most any stamp of the stroke has given each voxel, as the bits
    /// of an `f32`, by brick. Atoms because a stamp is laid across every
    /// core: each voxel is visited by one of them, so there is nothing to
    /// order, only something to be allowed to write through a shared
    /// reference.
    coverage: FxHashMap<BrickCoord, Box<[AtomicU32]>>,
    /// Whether the stroke is a line, its stamps joined, and not separate
    /// dabs.
    line: bool,
    /// Where the last stamp of a line was put down: the stroke's own, and
    /// each of its mirrors'.
    last: [Option<Vec3>; 1 + Symmetry::MAX_MIRRORS],
}

impl Layer {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new stroke: nothing covered yet. `line` for a stroke whose stamps
    /// are near enough to read as one line, and are joined into one; not
    /// for a brush set down in separate dabs.
    pub fn begin(&mut self, line: bool) {
        self.coverage.clear();
        self.line = line;
        self.last = [None; 1 + Symmetry::MAX_MIRRORS];
    }

    /// How many bricks the stroke has covered any of.
    pub fn bricks(&self) -> usize {
        self.coverage.len()
    }

    /// The most any stamp of the stroke has given a voxel.
    pub fn covered(&self, voxel: IVec3) -> f32 {
        let coord = BrickCoord::containing(voxel);
        self.coverage
            .get(&coord)
            .map_or(0.0, |brick| f32::from_bits(brick[local_index(voxel, coord)].load(RELAXED)))
    }
}

/// Each voxel is read and written by the one thread that visits it.
const RELAXED: Ordering = Ordering::Relaxed;

/// Where a voxel is in its brick's array.
#[inline]
fn local_index(voxel: IVec3, coord: BrickCoord) -> usize {
    let local = voxel - coord.origin();
    brick_index(local.x as usize, local.y as usize, local.z as usize)
}

/// A brick of coverage with nothing covered.
fn blank() -> Box<[AtomicU32]> {
    (0..BRICK_VOXELS).map(|_| AtomicU32::new(0)).collect()
}

/// The bricks of coverage one stamp can reach, laid out as a box so that a
/// voxel finds its own without a hash, and made as they are first written
/// so that a brick the stamp gives nothing to costs nothing.
struct Reach {
    lo: IVec3,
    size: IVec3,
    bricks: Vec<OnceLock<Box<[AtomicU32]>>>,
    /// Which of them the stroke had covered before this stamp.
    covered: Vec<bool>,
}

impl Reach {
    /// Take the coverage of the bricks in a voxel box out of the layer.
    fn take(layer: &mut Layer, lo: IVec3, hi: IVec3) -> Self {
        let lo = BrickCoord::containing(lo).0;
        let size = BrickCoord::containing(hi).0 - lo + IVec3::ONE;
        let count = (size.x * size.y * size.z) as usize;
        let mut reach = Self { lo, size, bricks: Vec::with_capacity(count), covered: Vec::new() };
        for index in 0..count {
            let cell = OnceLock::new();
            if let Some(held) = layer.coverage.remove(&reach.coord(index)) {
                // A cell just made holds nothing, so this cannot fail.
                let _ = cell.set(held);
            }
            reach.bricks.push(cell);
        }
        reach.covered = reach.bricks.iter().map(|cell| cell.get().is_some()).collect();
        reach
    }

    /// Hand the coverage back, with whatever the stamp added.
    fn give_back(self, layer: &mut Layer) {
        let coords: Vec<BrickCoord> =
            (0..self.bricks.len()).map(|index| self.coord(index)).collect();
        for (cell, coord) in self.bricks.into_iter().zip(coords) {
            if let Some(held) = cell.into_inner() {
                layer.coverage.insert(coord, held);
            }
        }
    }

    fn coord(&self, index: usize) -> BrickCoord {
        let index = index as i32;
        let (x, rest) = (index % self.size.x, index / self.size.x);
        let (y, z) = (rest % self.size.y, rest / self.size.y);
        BrickCoord(self.lo + IVec3::new(x, y, z))
    }

    #[inline]
    fn index(&self, coord: BrickCoord) -> usize {
        let at = coord.0 - self.lo;
        (at.x + self.size.x * (at.y + self.size.y * at.z)) as usize
    }

    /// Whether the stroke had covered any of a brick before this stamp.
    fn was_covered(&self, coord: BrickCoord) -> bool {
        self.covered[self.index(coord)]
    }

    /// Where the most a voxel has been given is kept.
    #[inline]
    fn slot(&self, voxel: IVec3) -> &AtomicU32 {
        let coord = BrickCoord(IVec3::new(
            voxel.x.div_euclid(BRICK_DIM as i32),
            voxel.y.div_euclid(BRICK_DIM as i32),
            voxel.z.div_euclid(BRICK_DIM as i32),
        ));
        &self.bricks[self.index(coord)].get_or_init(blank)[local_index(voxel, coord)]
    }
}

/// A stamp's centre put back along its normal onto the surface as it was
/// before the open stroke.
///
/// The field says how far off its surface a point is, up to three voxels,
/// so each step lands on the surface when it is that near and three voxels
/// nearer when it is not. `rise` is how far the stroke can have moved the
/// surface, in voxels, and sets how many steps are worth taking.
fn settled(volume: &Volume, centre: Vec3, normal: Vec3, rise: f32) -> Vec3 {
    let voxel_size = volume.voxel_size();
    let steps = (rise / OUTSIDE).ceil() as usize + 3;
    let mut at = centre;
    for _ in 0..steps {
        let off = volume.sample_world_before(at);
        if off.abs() < 0.01 {
            break;
        }
        at -= normal * (off * voxel_size);
    }
    // A point that never came within the band (a stamp given a centre far
    // off any surface) is left where it was given.
    if volume.sample_world_before(at).abs() < 1.0 { at } else { centre }
}

/// The point of the step from `from` to `to` that is nearest `point`.
#[inline]
fn nearest_on(from: Vec3, to: Vec3, point: Vec3) -> Vec3 {
    let step = to - from;
    let along = ((point - from).dot(step) / step.length_squared()).clamp(0.0, 1.0);
    from + step * along
}

impl Compound {
    /// Lay one stamp of a stroke that lays one layer, plus its mirrors when
    /// symmetry is on. See the module documentation.
    #[allow(clippy::too_many_arguments)]
    pub fn lay_symmetric(
        &self,
        volume: &mut Volume,
        stamp: &Stamp,
        tip: Option<&Alpha>,
        symmetry: Symmetry,
        centre: Vec3,
        layer: &mut Layer,
        scratch: &mut BrushScratch,
    ) {
        self.lay_one(volume, stamp, tip, layer, 0, scratch);
        if symmetry.is_off() {
            return;
        }
        let mut twins = [*stamp; Symmetry::MAX_MIRRORS];
        let count = symmetry.mirrors(stamp, centre, &mut twins);
        for (index, twin) in twins[..count].iter().enumerate() {
            self.lay_one(volume, twin, tip, layer, 1 + index, scratch);
        }
    }

    /// Lay one stamp of a stroke that lays one layer.
    pub fn lay(
        &self,
        volume: &mut Volume,
        stamp: &Stamp,
        tip: Option<&Alpha>,
        layer: &mut Layer,
        scratch: &mut BrushScratch,
    ) {
        self.lay_one(volume, stamp, tip, layer, 0, scratch);
    }

    /// Lay one stamp of the stroke, or of one of its mirrors: `which` says
    /// whose line it carries on.
    ///
    /// Not one loop for each set of actions, as [`Compound::apply`] is: a
    /// layer's stamp also reads the field before the stroke and keeps what
    /// each voxel has been given, and what asking about each action at
    /// each voxel costs beside those is in `what_a_layer_costs`.
    #[debug_requires(self.actions.is_legal(), "a brush's amounts are within what a brush can be")]
    #[debug_requires(
        volume.is_recording(),
        "a layer is laid over the field as it was before the stroke, which is the open stroke's \
         undo record: with no stroke open every stamp would build on the last"
    )]
    fn lay_one(
        &self,
        volume: &mut Volume,
        stamp: &Stamp,
        tip: Option<&Alpha>,
        layer: &mut Layer,
        which: usize,
        scratch: &mut BrushScratch,
    ) {
        let actions = self.actions;
        if self.radius <= 0.0
            || self.strength <= 0.0
            || stamp.pressure <= 0.0
            || actions.is_nothing()
        {
            return;
        }

        let voxel_size = volume.voxel_size();
        let radius = self.radius;
        let inverse_radius = 1.0 / radius;
        // No more than all of a layer, however hard the brush is set.
        let gain = (self.strength * stamp.pressure).min(1.0);
        // How far a push moves the surface at full weight, in voxels, and
        // an inflation.
        let depth = radius / voxel_size * LAYER_DEPTH;
        let offset = depth.min(MOST_OFFSET_VOXELS);
        let direction = stamp.direction;
        let normal = stamp.normal;

        // The stamp, back on the surface it was aimed at.
        let levels = actions.level != 0.0;
        let rise = gain * depth * actions.push.abs()
            + gain * offset * actions.inflate.abs()
            + if levels { actions.plane.abs() * radius / voxel_size } else { 0.0 };
        let centre = settled(volume, stamp.centre, normal, rise);
        // Where the line came from, if this stamp carries one on. A stamp
        // a long way from the last is not carrying on a line: the pointer
        // left the model and came back to it somewhere else.
        let came_from = layer.last[which]
            .filter(|_| layer.line)
            .filter(|last| last.distance(centre) < radius * 2.0 && *last != centre);
        layer.last[which] = Some(centre);

        let pinch_alone = actions.push == 0.0
            && actions.inflate == 0.0
            && actions.smooth == 0.0
            && actions.level == 0.0;
        let pinch_turn = match direction {
            BrushDirection::Subtract if pinch_alone => -1.0,
            _ => 1.0,
        };
        let plane_point = if actions.plane != 0.0 {
            centre + normal * (radius * actions.plane * direction.outward_sign())
        } else {
            centre
        };

        let pull = PINCH_PULL_VOXELS * LAYER_STAMPS;
        let reach_voxels = gain
            * (actions.push.abs() * depth + (actions.pinch.abs() + actions.gather.abs()) * pull);
        let reach = reach_voxels * voxel_size;
        let within = radius * REACH;
        let extent = Vec3::splat(within);
        // The box of the whole step, from where the line came from to here.
        let start = came_from.unwrap_or(centre);
        let (box_lo, box_hi) = (start.min(centre), start.max(centre));
        let stepped = start.distance(centre);
        let (lo, hi) = volume.voxel_bounds(box_lo - extent, box_hi + extent);
        let read_extent = extent + Vec3::splat(reach);
        let (read_lo, read_hi) = volume.voxel_bounds(box_lo - read_extent, box_hi + read_extent);
        volume.snapshot_before(read_lo, read_hi, &mut scratch.region);
        let region = &scratch.region;

        let pattern = if self.pattern.is_off() {
            Prepared::OFF
        } else {
            self.pattern.prepare(voxel_size, normal, stamp.tangent)
        };
        let slope = self.profile.slope();
        let frame = Frame::of(stamp);
        let read_voxels = (reach / voxel_size).ceil() as i32 + 1;

        let taken = Reach::take(layer, lo, hi);
        let covering = &taken;

        // Which bricks of the box can change at all: the generic brush's
        // own answer, and one thing more. That answer is given of the
        // brick as it is NOW, and a layer is made from the brick as it was
        // BEFORE the stroke. For a brick the stroke has not covered those
        // are the same brick. One it has covered is done in full.
        let level_alone = actions.push == 0.0
            && actions.inflate == 0.0
            && actions.pinch == 0.0
            && actions.gather == 0.0
            && actions.smooth == 0.0;
        let inflating = match direction {
            BrushDirection::Add => actions.inflate,
            BrushDirection::Subtract => -actions.inflate,
        };
        let leaves_saturated_alone = move |value: f32| {
            actions.level == 0.0
                && (inflating == 0.0
                    || (inflating > 0.0 && value == INSIDE)
                    || (inflating < 0.0 && value == OUTSIDE))
        };
        let decide = |preview: &BrickPreview| {
            if preview.mask == Some(PROTECTED) {
                return BrickVerdict::Skip;
            }
            let slack = Vec3::splat(voxel_size);
            let box_min = preview.lo.as_vec3() * voxel_size - slack;
            let box_max = preview.hi.as_vec3() * voxel_size + slack;
            // Further from both ends of the step than the brush reaches
            // from anywhere on it.
            let off = |end: Vec3| end.clamp(box_min, box_max).distance(end);
            if off(centre).min(off(start)) >= within + stepped {
                return BrickVerdict::Skip;
            }
            if covering.was_covered(preview.coord) {
                return BrickVerdict::Whole;
            }
            match preview.uniform {
                Some(value)
                    if (value == INSIDE || value == OUTSIDE) && leaves_saturated_alone(value) =>
                {
                    BrickVerdict::OnlyNearDifferentNeighbours
                }
                Some(value)
                    if level_alone
                        && blend_toward_plane_clamps_back(
                            value,
                            box_min,
                            box_max,
                            plane_point,
                            normal,
                            voxel_size,
                        ) =>
                {
                    BrickVerdict::Skip
                }
                _ => BrickVerdict::Whole,
            }
        };

        let within_squared = within * within;
        volume.edit_voxels_where(
            lo,
            hi,
            read_voxels,
            true,
            decide,
            move |voxel, position, value, free| {
                // The foot of the voxel on the stroke's line: the nearest
                // point of this step of it, or for a dab, the stamp.
                let foot = match came_from {
                    Some(last) => nearest_on(last, centre, position),
                    None => centre,
                };
                let from_foot = position - foot;
                if from_foot.length_squared() >= within_squared {
                    return value;
                }
                let from_centre = position - centre;
                // Across the surface, and not from the centre: see the
                // module documentation.
                let across = from_foot - normal * from_foot.dot(normal);
                let distance = across.length() * inverse_radius;
                if distance >= 1.0 {
                    return value;
                }
                let shaped = slope.weight(distance) * gain;
                if shaped <= 0.0 {
                    return value;
                }
                let mut patterned = shaped * pattern.weight(position);
                if let Some(image) = tip {
                    patterned *= frame.height(image, from_centre, inverse_radius);
                }
                if patterned <= 0.0 || free <= 0.0 {
                    return value;
                }
                let weight = patterned * free;

                // Only a stamp that gives the voxel more than any before it
                // writes it, and what it writes is made from the field as
                // it was before the stroke.
                let most = covering.slot(voxel);
                if weight <= f32::from_bits(most.load(RELAXED)) + MORE {
                    return value;
                }
                most.store(weight.to_bits(), RELAXED);
                let mut held = region.get(voxel);

                if actions.push != 0.0 || actions.pinch != 0.0 || actions.gather != 0.0 {
                    let mut from = position;
                    if actions.push != 0.0 {
                        from -= normal
                            * (weight
                                * depth
                                * voxel_size
                                * direction.outward_sign()
                                * actions.push);
                    }
                    if actions.pinch != 0.0 {
                        let lateral = -across;
                        let off_axis = lateral.length();
                        if let Some(to_axis) = lateral.try_normalize() {
                            let far = (weight * pull * voxel_size * actions.pinch.abs())
                                .min(off_axis * MOST_PULL);
                            from += to_axis * (far * pinch_turn * actions.pinch.signum());
                        }
                    }
                    if actions.gather != 0.0 {
                        // From further off the line by a share of how far
                        // off it the voxel is: `super::GATHER`. A layer's
                        // foot is on the stroke's line already, so `across`
                        // is the way off it.
                        let share =
                            (super::GATHER * LAYER_STAMPS * weight * pinch_turn * actions.gather)
                                .clamp(-super::MOST_GATHER, super::MOST_GATHER);
                        from += (across * share).clamp_length_max(pull * voxel_size);
                    }
                    held = region.sample(from / voxel_size);
                }
                if actions.inflate != 0.0 {
                    held += direction.field_sign() * weight * offset * actions.inflate;
                }
                if actions.smooth != 0.0 {
                    let average = region.neighbour_average(voxel);
                    held += (average - held) * (weight * actions.smooth * LAYER_STAMPS).min(1.0);
                }
                if levels {
                    let plane = (position - plane_point).dot(normal) / voxel_size;
                    let share = (patterned * actions.level * LAYER_STAMPS).min(1.0);
                    let blended = held + (plane - held) * share;
                    let adds = match direction {
                        BrushDirection::Add => true,
                        BrushDirection::Subtract => false,
                    };
                    let kept = match (actions.side, adds) {
                        (Side::Both, _) => blended,
                        (Side::Fill, true) | (Side::Cut, false) => blended.min(held),
                        (Side::Fill, false) | (Side::Cut, true) => blended.max(held),
                    };
                    held = admit(held, kept.clamp(INSIDE, OUTSIDE), free);
                }
                held
            },
        );
        taken.give_back(layer);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Actions, Profile, Shape};
    use super::*;
    use crate::pattern::Pattern;
    use crate::testing::assert_same_field;

    const CENTRE: Vec3 = Vec3::splat(48.0);
    const RADIUS: f32 = 30.0;

    /// The generic brush's fixture: a ball of thirty voxels, a voxel a
    /// unit, far enough from the origin to straddle brick boundaries.
    fn ball() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(CENTRE, RADIUS);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    fn brush(actions: Actions, radius: f32, strength: f32) -> Compound {
        Compound {
            actions,
            radius,
            strength,
            profile: Profile::default(),
            pattern: Pattern::default(),
        }
    }

    /// Directions from the ball's centre along a stroke over its top, a
    /// quarter of `radius` apart, which is upstream's spacing.
    fn path(radius: f32, stamps: usize) -> Vec<Vec3> {
        let step = radius * 0.25 / RADIUS;
        (0..stamps)
            .map(|index| {
                let angle = (index as f32 - (stamps - 1) as f32 / 2.0) * step;
                Vec3::new(angle.sin(), angle.cos(), 0.0)
            })
            .collect()
    }

    /// A stamp on the fresh ball's surface, out along `towards`.
    fn stamp(towards: Vec3) -> Stamp {
        Stamp::new(CENTRE + towards * RADIUS, towards, BrushDirection::Add).with_tangent(Vec3::X)
    }

    /// How far out along `towards` the surface is from where the fresh
    /// ball has it, in voxels: found by walking out from well inside until
    /// the field changes sign.
    fn rise(volume: &Volume, towards: Vec3) -> f32 {
        let mut last = (RADIUS - 12.0, volume.sample_world(CENTRE + towards * (RADIUS - 12.0)));
        assert!(last.1 < 0.0, "the walk began outside the model");
        let mut along = last.0;
        while along < RADIUS + 20.0 {
            along += 0.125;
            let value = volume.sample_world(CENTRE + towards * along);
            if value >= 0.0 {
                let share = last.1 / (last.1 - value);
                return last.0 + (along - last.0) * share - RADIUS;
            }
            last = (along, value);
        }
        panic!("no surface along {towards}");
    }

    /// One stroke of `passes` passes over `stamps`, laid as a line of one
    /// layer.
    fn laid(compound: &Compound, stamps: &[Stamp], passes: usize) -> Volume {
        laid_as(true, compound, stamps, passes)
    }

    /// One stroke of `passes` passes over `stamps`, there and back as a
    /// hand scrubs, laid as one layer: a line, or separate dabs.
    fn laid_as(line: bool, compound: &Compound, stamps: &[Stamp], passes: usize) -> Volume {
        let mut volume = ball();
        let (mut layer, mut scratch) = (Layer::new(), BrushScratch::new());
        volume.begin_stroke();
        layer.begin(line);
        for pass in 0..passes {
            // There and back, as a hand scrubs.
            let mut there: Vec<&Stamp> = stamps.iter().collect();
            if pass % 2 == 1 {
                there.reverse();
            }
            for stamp in there {
                compound.lay(&mut volume, stamp, None, &mut layer, &mut scratch);
            }
        }
        volume.end_stroke();
        volume
    }

    /// The same, with build-up on.
    fn built(compound: &Compound, stamps: &[Stamp], passes: usize) -> Volume {
        let mut volume = ball();
        let mut scratch = BrushScratch::new();
        volume.begin_stroke();
        for _ in 0..passes {
            for stamp in stamps {
                compound.apply(&mut volume, stamp, None, &mut scratch);
            }
        }
        volume.end_stroke();
        volume
    }

    /// The most two fields differ by anywhere near the stroke, in voxels.
    fn most_apart(one: &Volume, other: &Volume) -> f32 {
        let mut most = 0.0_f32;
        for z in 10..90 {
            for y in 40..100 {
                for x in 10..90 {
                    let voxel = IVec3::new(x, y, z);
                    most = most.max((one.sample_voxel(voxel) - other.sample_voxel(voxel)).abs());
                }
            }
        }
        most
    }

    /// A layer is as tall as the strength says, in the middle of a stamp
    /// and of a stroke, and a brush that dwells lays no more of it. With
    /// build-up on the same dwelling is taller for every stamp.
    #[test]
    fn a_layer_is_as_tall_as_its_strength_says_however_long_the_brush_dwells() {
        let (radius, strength) = (10.0, 0.5);
        let draw = brush(Actions::DRAW, radius, strength);
        let top = Vec3::Y;
        let expected = radius * LAYER_DEPTH * strength;

        let once = laid(&draw, &[stamp(top)], 1);
        let tall = rise(&once, top);
        assert!((tall - expected).abs() < 0.2, "one stamp rose {tall} voxels, not {expected}");
        let dwelt = laid(&draw, &[stamp(top)], 30);
        assert_same_field(&once, &dwelt, "thirty stamps in one place and one");

        // Along a stroke the middle of it is the same height, and so is
        // the middle of one scrubbed there and back and there again. Not
        // to the bit, for a line: see the module documentation. The same
        // stamps set down as dabs are, in whatever order.
        let stamps: Vec<Stamp> = path(radius, 9).into_iter().map(stamp).collect();
        let stroke = laid(&draw, &stamps, 1);
        let along = rise(&stroke, top);
        assert!((along - expected).abs() < 0.2, "a stroke rose {along} voxels, not {expected}");
        let scrubbed = laid(&draw, &stamps, 3);
        let (again, apart) = (rise(&scrubbed, top), most_apart(&stroke, &scrubbed));
        assert!((again - along).abs() < 0.02, "scrubbed, it is {again} voxels and not {along}");
        assert!(apart < 0.1, "scrubbing a line moved the field by {apart} voxels somewhere");
        assert_same_field(
            &laid_as(false, &draw, &stamps, 1),
            &laid_as(false, &draw, &stamps, 3),
            "dabs set down three times and once",
        );
        // A line has no ribs: half way between two stamps it is as tall as
        // it is at a stamp. The same stamps as dabs are not joined, and
        // are lower there.
        let towards = path(radius, 9);
        let half_way = (towards[4] + towards[5]).normalize();
        let (at, between) = (rise(&stroke, towards[4]), rise(&stroke, half_way));
        assert!((at - between).abs() < 0.02, "{at} voxels at a stamp and {between} between two");
        let dabbed = laid_as(false, &draw, &stamps, 1);
        let dip = rise(&dabbed, towards[4]) - rise(&dabbed, half_way);
        assert!(dip > 0.03, "dabs a quarter of a radius apart dip only {dip} voxels between");

        // Control: build-up on, the same dwelling is taller each time.
        let (one, thirty) = (built(&draw, &[stamp(top)], 1), built(&draw, &[stamp(top)], 30));
        assert!(rise(&thirty, top) > rise(&one, top) + 2.0, "build-up did not build up");

        // Half the strength is half the layer, and the invert key cuts
        // one as deep as it would have been tall.
        let half = laid(&brush(Actions::DRAW, radius, strength / 2.0), &[stamp(top)], 1);
        assert!((rise(&half, top) - expected / 2.0).abs() < 0.2, "{}", rise(&half, top));
        let cut = Stamp { direction: BrushDirection::Subtract, ..stamp(top) };
        let sunk = rise(&laid(&draw, &[cut], 4), top);
        assert!((sunk + expected).abs() < 0.2, "a cut went {sunk} voxels, not {}", -expected);
    }

    /// A stamp that arrives on what the stroke has already built is put
    /// back on the surface as it was, so the stroke does not climb its own
    /// work.
    #[test]
    fn a_stamp_is_put_back_on_the_surface_as_it_was_before_the_stroke() {
        let (radius, strength) = (10.0, 0.5);
        let draw = brush(Actions::DRAW, radius, strength);
        let top = Vec3::Y;
        let mut volume = ball();
        let (mut layer, mut scratch) = (Layer::new(), BrushScratch::new());
        volume.begin_stroke();
        layer.begin(true);
        draw.lay(&mut volume, &stamp(top), None, &mut layer, &mut scratch);
        // Where a ray from the camera would now land: on top of the layer.
        let raised = rise(&volume, top);
        assert!(raised > 2.0, "the fixture's layer is only {raised} voxels tall");
        let on_the_layer = CENTRE + top * (RADIUS + raised);
        let back = settled(&volume, on_the_layer, top, raised + 1.0);
        let off = (back - (CENTRE + top * RADIUS)).length();
        assert!(off < 0.05, "put back {off} voxels from the surface as it was");
        // From under it too, which is where a cut leaves the pointer.
        let under = settled(&volume, CENTRE + top * (RADIUS - 5.0), top, 6.0);
        assert!((under - (CENTRE + top * RADIUS)).length() < 0.05);
        // And a point nowhere near any surface is left where it was.
        let nowhere = CENTRE + top * (RADIUS + 40.0);
        assert_eq!(settled(&volume, nowhere, top, 2.0), nowhere);
        // The surface as it was still faces the way it did, under the
        // layer: on the shoulder of it, where the surface as it is now
        // leans away.
        let aside = Vec3::new(0.2, 0.98, 0.0).normalize();
        let shoulder = CENTRE + aside * (RADIUS + rise(&volume, aside));
        let leaning = volume.gradient_world(shoulder);
        assert!(leaning.dot(aside) < 0.99, "the fixture's shoulder does not lean: {leaning}");
        let faced = volume.gradient_world_before(CENTRE + aside * RADIUS);
        assert!(faced.dot(aside) > 0.995, "faced {faced}, and the ball there faces {aside}");
        volume.end_stroke();

        // Clay is the brush that shows why. Its plane is held off the
        // surface and faces the way the stamp does, so a stamp aimed at
        // the top of its own strip would hold the plane that much higher
        // again, tilted with whatever the strip's top is.
        // A smaller brush than the one above, for a ball of thirty voxels:
        // each stamp's plane touches the ball at its own centre, and with
        // a brush a third of the ball across the planes of two stamps a
        // quarter of a radius apart are most of a voxel apart at the rim.
        let radius = 5.0;
        let clay = brush(Actions::CLAY, radius, strength);
        let ridden = |layered: bool, passes: usize| {
            let mut volume = ball();
            let (mut layer, mut scratch) = (Layer::new(), BrushScratch::new());
            volume.begin_stroke();
            layer.begin(true);
            for pass in 0..passes {
                let mut there = path(radius, 9);
                if pass % 2 == 1 {
                    there.reverse();
                }
                for towards in there {
                    // Where the application aims a stamp. With build-up on,
                    // at the surface as it is now, facing the way it faces
                    // now. For a layer, along the same ray, at the surface
                    // as it was and facing the way that faces.
                    let stamp = if layered {
                        let at = CENTRE + towards * RADIUS;
                        Stamp::new(at, volume.gradient_world_before(at), BrushDirection::Add)
                    } else {
                        let at = CENTRE + towards * (RADIUS + rise(&volume, towards));
                        Stamp::new(at, volume.gradient_world(at), BrushDirection::Add)
                    };
                    if layered {
                        clay.lay(&mut volume, &stamp, None, &mut layer, &mut scratch);
                    } else {
                        clay.apply(&mut volume, &stamp, None, &mut scratch);
                    }
                }
            }
            volume.end_stroke();
            volume
        };
        // At this strength a layer of clay reaches its plane: a strip as
        // thick as the plane is held proud.
        let strip = ridden(true, 1);
        let thick = rise(&strip, top);
        let plane = radius * 0.35;
        assert!((thick - plane).abs() < 0.3, "a strip of clay is {thick} voxels, not {plane}");
        let again = ridden(true, 3);
        let (thick_again, apart) = (rise(&again, top), most_apart(&strip, &again));
        assert!(
            (thick_again - thick).abs() < 0.05 && apart < 0.25,
            "three passes of clay with build-up off are {thick_again} voxels thick and not \
             {thick}, and differ from one pass by {apart} voxels somewhere"
        );
        // Control: with build-up on, three passes are taller than one.
        let (one, three) = (ridden(false, 1), ridden(false, 3));
        assert!(rise(&three, top) > rise(&one, top) + 0.5, "clay did not build up");
    }

    /// A layer is one undo, a mask is honoured, and a mirrored stroke is
    /// one layer on each side.
    #[test]
    fn a_layer_is_one_undo_under_a_mask_and_mirrored() {
        let draw = brush(Actions::DRAW, 8.0, 0.6);
        let stamps: Vec<Stamp> = path(8.0, 7).into_iter().map(stamp).collect();
        let mut volume = ball();
        let (mut layer, mut scratch) = (Layer::new(), BrushScratch::new());
        volume.begin_stroke();
        layer.begin(true);
        for stamp in &stamps {
            draw.lay(&mut volume, stamp, None, &mut layer, &mut scratch);
        }
        assert!(layer.bricks() > 0);
        assert!(layer.covered((CENTRE + Vec3::Y * RADIUS).as_ivec3()) > 0.5);
        assert_eq!(layer.covered(CENTRE.as_ivec3()), 0.0, "the middle of the ball was covered");
        let edit = volume.end_stroke().expect("the stroke changed the field");
        assert!(rise(&volume, Vec3::Y) > 1.0);
        volume.apply_edit(edit);
        assert_same_field(&volume, &ball(), "a layer undone");
        // A new stroke begins with nothing covered.
        layer.begin(true);
        assert_eq!(layer.bricks(), 0);

        let mut masked = ball();
        masked.edit_mask(IVec3::splat(0), IVec3::splat(100), |_, _, _| PROTECTED);
        masked.begin_stroke();
        for stamp in &stamps {
            draw.lay(&mut masked, stamp, None, &mut layer, &mut scratch);
        }
        assert!(masked.end_stroke().is_none(), "a layer got under the mask");

        // Mirrored across X through the ball's centre: the same on both
        // sides, and walked twice the same as once.
        let symmetry = Symmetry::OFF.with_axis(crate::brush::MirrorAxis::X, true);
        let side = Vec3::new(0.6, 0.8, 0.0);
        let mirrored = |passes: usize| {
            let mut volume = ball();
            let (mut layer, mut scratch) = (Layer::new(), BrushScratch::new());
            volume.begin_stroke();
            for _ in 0..passes {
                draw.lay_symmetric(
                    &mut volume,
                    &stamp(side),
                    None,
                    symmetry,
                    CENTRE,
                    &mut layer,
                    &mut scratch,
                );
            }
            volume.end_stroke();
            volume
        };
        let once = mirrored(1);
        let (here, there) = (rise(&once, side), rise(&once, Vec3::new(-0.6, 0.8, 0.0)));
        assert!(
            here > 1.0 && (here - there).abs() < 0.05,
            "{here} on one side, {there} on the other"
        );
        assert_same_field(&once, &mirrored(2), "a mirrored layer laid twice and once");
    }

    /// A layer needs the stroke open: what the field was before the stroke
    /// is the stroke's undo record, and with none every stamp would build
    /// on the last and say nothing about it.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a layer is laid over the field as it was before the stroke")]
    fn a_layer_laid_with_no_stroke_open_is_refused() {
        let mut volume = ball();
        brush(Actions::DRAW, 8.0, 0.5).lay(
            &mut volume,
            &stamp(Vec3::Y),
            None,
            &mut Layer::new(),
            &mut BrushScratch::new(),
        );
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn actions() -> impl Strategy<Value = Actions> {
            let amount = || -1.5_f32..1.5;
            let side = prop::sample::select(Side::ALL.to_vec());
            (amount(), amount(), amount(), amount(), amount(), amount(), side, amount()).prop_map(
                |(push, inflate, pinch, smooth, level, plane, side, gather)| {
                    Actions { push, inflate, pinch, gather, smooth, level, plane, side }.legal()
                },
            )
        }

        proptest! {
            // Each case lays strokes on a ball: fewer of them.
            #![proptest_config(ProptestConfig::with_cases(24))]

            /// Whatever the brush is made of: its dabs set down twice, the
            /// second time in the other order, are its dabs set down
            /// once, to the bit; what it leaves is still a field, as dabs
            /// and as a line; and it changes nothing further from its
            /// stamps than the brush reaches.
            #[test]
            fn any_layer_is_laid_once_and_stays_in_reach(
                actions in actions(),
                shape in prop::sample::select(Shape::ALL.to_vec()),
                plateau in 0.0_f32..0.9,
                radius in 4.0_f32..12.0,
                strength in 0.05_f32..=1.0,
                subtract in any::<bool>(),
                stamps in 1_usize..5,
            ) {
                let compound = Compound {
                    profile: Profile { shape, plateau },
                    ..brush(actions, radius, strength)
                };
                let direction =
                    if subtract { BrushDirection::Subtract } else { BrushDirection::Add };
                let stamps: Vec<Stamp> = path(radius, stamps)
                    .into_iter()
                    .map(|towards| Stamp { direction, ..stamp(towards) })
                    .collect();
                let once = laid_as(false, &compound, &stamps, 1);
                let twice = laid_as(false, &compound, &stamps, 2);
                // And as a line: what it leaves is still a field.
                let line = laid(&compound, &stamps, 2);
                let fresh = ball();
                for z in 10..90 {
                    for y in 10..100 {
                        for x in 10..90 {
                            let voxel = IVec3::new(x, y, z);
                            let value = once.sample_voxel(voxel);
                            prop_assert_eq!(
                                value.to_bits(),
                                twice.sample_voxel(voxel).to_bits(),
                                "a second pass changed {}",
                                voxel
                            );
                            prop_assert!((INSIDE..=OUTSIDE).contains(&value), "{} at {}", value, voxel);
                            prop_assert!((INSIDE..=OUTSIDE).contains(&line.sample_voxel(voxel)));
                            let near = stamps.iter().any(|stamp| {
                                (voxel.as_vec3() - stamp.centre).length() < radius * REACH + 1.0
                            });
                            if !near {
                                prop_assert_eq!(
                                    value.to_bits(),
                                    fresh.sample_voxel(voxel).to_bits(),
                                    "changed out of reach at {}",
                                    voxel
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod timing {
    use super::super::{Actions, Profile};
    use super::*;
    use crate::pattern::Pattern;
    use std::time::Instant;

    /// What a stamp of a layer costs beside the same brush with build-up
    /// on. Run by hand, in release:
    /// `cargo test --release -p brokkr-core --lib what_a_layer_costs -- --ignored --nocapture`
    #[test]
    #[ignore = "a measurement, not a claim: run by hand, in release"]
    fn what_a_layer_costs() {
        const CENTRE: Vec3 = Vec3::splat(96.0);
        const RADIUS: f32 = 60.0;
        for (name, actions) in [("Draw", Actions::DRAW), ("Clay", Actions::CLAY)] {
            for radius in [8.0_f32, 24.0] {
                let compound = Compound {
                    actions,
                    radius,
                    strength: 0.3,
                    profile: Profile::default(),
                    pattern: Pattern::default(),
                };
                let stamps: Vec<Stamp> = (0..40)
                    .map(|index| {
                        let angle = (index as f32 - 20.0) * radius * 0.25 / RADIUS;
                        let towards = Vec3::new(angle.sin(), angle.cos(), 0.0);
                        Stamp::new(CENTRE + towards * RADIUS, towards, BrushDirection::Add)
                    })
                    .collect();
                let timed = |layered: bool| {
                    let mut best = f64::MAX;
                    for _ in 0..5 {
                        let mut volume = Volume::new(1.0);
                        volume.seed_sphere(CENTRE, RADIUS);
                        let (mut layer, mut scratch) = (Layer::new(), BrushScratch::new());
                        volume.begin_stroke();
                        let started = Instant::now();
                        for stamp in &stamps {
                            if layered {
                                compound.lay(&mut volume, stamp, None, &mut layer, &mut scratch);
                            } else {
                                compound.apply(&mut volume, stamp, None, &mut scratch);
                            }
                        }
                        best = best.min(started.elapsed().as_secs_f64() * 1000.0 / 40.0);
                        volume.end_stroke();
                    }
                    best
                };
                let (on, off) = (timed(false), timed(true));
                println!(
                    "{name} at {radius} voxels: build-up on {on:.3} ms a stamp, off {off:.3} ms, \
                     {:+.0}%",
                    (off / on - 1.0) * 100.0
                );
            }
        }
    }
}
