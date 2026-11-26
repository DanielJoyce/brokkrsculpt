// SPDX-License-Identifier: AGPL-3.0-only

//! Deformers: a body twisted, bent or tapered along one of its axes.
//!
//! Each is a warp, `new(p) = old(inverse(p))`, the same idea as
//! [`crate::form`]: whatever was at the source point arrives at `p` as it
//! was, so surface detail rides along. What differs from a move or a turn
//! is that the map is not rigid, so the gradient's length drifts off one
//! where the warp stretches or squeezes; the caller runs
//! [`Volume::redistance`] after, as the form tools do.
//!
//! # The frame
//!
//! The body's surface bounds give a centre and, along the chosen axis, a
//! base and a length `L`. `t` runs from nought at the base to one at the far
//! end. Everything is measured from that, so a deformer means the same on a
//! body wherever it stands.
//!
//! * **Twist** turns each slice about the axis by `amount * t` degrees.
//! * **Bend** curls the axis into an arc of `amount` degrees over `L`,
//!   toward the next axis round (X for an upright body, Y otherwise), the
//!   base staying put. A slice stays square to the arc.
//! * **Taper** scales each slice across the axis by `1 + (amount - 1) * t`,
//!   so `amount` is the far end's size and the base keeps its own.
//!
//! # Not done
//!
//! * **The mask is not carried.** Paint and layers are, voxel by voxel.
//! * **No live preview**: each is applied by a button, and undone by undo.
//! * **A bend is at most half a turn.** Past that the arc's inverse is not
//!   one to one with the angle `atan2` gives.

use glam::{IVec3, Quat, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashSet;

use crate::brick::{BRICK_DIM, Brick, BrickCoord, INSIDE, OUTSIDE, brick_index};
use crate::shell::{settled, voxel_of};
use crate::volume::Volume;

/// Which deformer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeformKind {
    #[default]
    Twist,
    Bend,
    Taper,
}

impl DeformKind {
    pub const ALL: [DeformKind; 3] = [DeformKind::Twist, DeformKind::Bend, DeformKind::Taper];

    pub fn label(self) -> &'static str {
        match self {
            DeformKind::Twist => "Twist",
            DeformKind::Bend => "Bend",
            DeformKind::Taper => "Taper",
        }
    }

    /// The amount that changes nothing.
    pub fn identity(self) -> f32 {
        match self {
            DeformKind::Twist | DeformKind::Bend => 0.0,
            DeformKind::Taper => 1.0,
        }
    }
}

impl std::fmt::Display for DeformKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One of the three axes of the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    X,
    #[default]
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    /// The direction a bend along this axis curls toward.
    pub fn toward(self) -> Vec3 {
        match self {
            Axis::Y | Axis::Z => Vec3::X,
            Axis::X => Vec3::Y,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }
}

impl std::fmt::Display for Axis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A deformer and how much of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Deformation {
    pub kind: DeformKind,
    pub axis: Axis,
    /// Degrees for a twist or a bend; the far end's scale for a taper.
    pub amount: f32,
}

/// The most a bend may curl, in degrees.
pub const MAX_BEND_DEGREES: f32 = 180.0;
/// The smallest a taper may make the far end.
pub const MIN_TAPER: f32 = 0.05;

/// A deformation fixed to one body's frame, both ways round.
#[derive(Debug, Clone, Copy)]
struct Frame {
    how: Deformation,
    centre: Vec3,
    axis: Vec3,
    toward: Vec3,
    base: f32,
    length: f32,
}

impl Frame {
    fn new(how: Deformation, low: Vec3, high: Vec3) -> Self {
        let axis = how.axis.unit();
        let base = low.dot(axis);
        let length = (high.dot(axis) - base).max(1e-3);
        Frame { how, centre: (low + high) * 0.5, axis, toward: how.axis.toward(), base, length }
    }

    /// Where along the axis a point is, nought at the base and one at the end.
    fn along(&self, p: Vec3) -> f32 {
        (p.dot(self.axis) - self.base) / self.length
    }

    /// The bend's radius, signed; `None` when it is too slight to be one.
    fn radius(&self) -> Option<f32> {
        let angle = self.how.amount.clamp(-MAX_BEND_DEGREES, MAX_BEND_DEGREES).to_radians();
        (angle.abs() > 1e-4).then(|| self.length / angle)
    }

    fn taper(&self, t: f32) -> f32 {
        (1.0 + (self.how.amount.max(MIN_TAPER) - 1.0) * t).max(MIN_TAPER)
    }

    /// Split a point into (across, along the axis from the base, the rest),
    /// `across` measured along `toward` from the centre line.
    fn bend_parts(&self, p: Vec3) -> (f32, f32, Vec3) {
        let q = p - self.centre;
        let across = q.dot(self.toward);
        let along = p.dot(self.axis) - self.base;
        let rest = q - across * self.toward - q.dot(self.axis) * self.axis;
        (across, along, rest)
    }

    fn bend_join(&self, across: f32, along: f32, rest: Vec3) -> Vec3 {
        let on_axis = self.centre - self.centre.dot(self.axis) * self.axis;
        on_axis + rest + across * self.toward + (along + self.base) * self.axis
    }

    /// Where a point of the body goes.
    fn forward(&self, p: Vec3) -> Vec3 {
        match self.how.kind {
            DeformKind::Twist => {
                let angle = self.how.amount.to_radians() * self.along(p);
                self.centre + Quat::from_axis_angle(self.axis, angle) * (p - self.centre)
            }
            DeformKind::Taper => {
                let q = p - self.centre;
                let on = q.dot(self.axis) * self.axis;
                self.centre + on + (q - on) * self.taper(self.along(p))
            }
            DeformKind::Bend => {
                let Some(radius) = self.radius() else {
                    return p;
                };
                let (across, along, rest) = self.bend_parts(p);
                let angle = along / radius;
                let from_centre = radius - across;
                self.bend_join(radius - from_centre * angle.cos(), from_centre * angle.sin(), rest)
            }
        }
    }

    /// Where a point of the result came from.
    fn inverse(&self, p: Vec3) -> Vec3 {
        match self.how.kind {
            DeformKind::Twist => {
                let angle = self.how.amount.to_radians() * self.along(p);
                self.centre + Quat::from_axis_angle(self.axis, -angle) * (p - self.centre)
            }
            DeformKind::Taper => {
                let q = p - self.centre;
                let on = q.dot(self.axis) * self.axis;
                self.centre + on + (q - on) / self.taper(self.along(p))
            }
            DeformKind::Bend => {
                let Some(radius) = self.radius() else {
                    return p;
                };
                let (across, along, rest) = self.bend_parts(p);
                let sign = radius.signum();
                let from_centre = sign * (radius - across).hypot(along);
                let angle = (along * sign).atan2((radius - across) * sign);
                self.bend_join(radius - from_centre, angle * radius, rest)
            }
        }
    }
}

impl Volume {
    /// This body deformed. `None` when it has no surface to take a frame
    /// from. The result's distances are not redistanced; see the module
    /// header.
    pub fn deformed(&self, how: Deformation) -> Option<Volume> {
        let voxel_size = self.voxel_size();
        let (low, high) = self.surface_bounds()?;
        let frame = Frame::new(how, low, high);

        // Where the body can go: the forward image of its stored bricks'
        // box, sampled on a grid and padded.
        let (stored_low, stored_high) = self.world_bounds()?;
        let mut reach_low = Vec3::splat(f32::INFINITY);
        let mut reach_high = Vec3::splat(f32::NEG_INFINITY);
        const STEPS: usize = 12;
        for k in 0..=STEPS {
            for j in 0..=STEPS {
                for i in 0..=STEPS {
                    let t = Vec3::new(i as f32, j as f32, k as f32) / STEPS as f32;
                    let moved = frame.forward(stored_low + (stored_high - stored_low) * t);
                    reach_low = reach_low.min(moved);
                    reach_high = reach_high.max(moved);
                }
            }
        }
        let pad = (stored_high - stored_low).max_element() / STEPS as f32 + 4.0 * voxel_size;
        let first = BrickCoord::containing(((reach_low - pad) / voxel_size).floor().as_ivec3()).0;
        let last = BrickCoord::containing(((reach_high + pad) / voxel_size).ceil().as_ivec3()).0;
        let mut coords = Vec::new();
        for z in first.z..=last.z {
            for y in first.y..=last.y {
                for x in first.x..=last.x {
                    coords.push(BrickCoord::new(x, y, z));
                }
            }
        }

        let built: Vec<(BrickCoord, Brick)> = coords
            .par_iter()
            .filter_map(|&coord| {
                let origin = coord.origin();
                // What the brick's source covers: its inverse image, sampled
                // at 27 points and padded for the curve between them.
                let mut source_low = IVec3::MAX;
                let mut source_high = IVec3::MIN;
                for k in 0..3 {
                    for j in 0..3 {
                        for i in 0..3 {
                            let local = IVec3::new(i, j, k) * (BRICK_DIM as i32 - 1) / 2;
                            let at = (origin + local).as_vec3() * voxel_size;
                            let from = (frame.inverse(at) / voxel_size).round().as_ivec3();
                            source_low = source_low.min(from);
                            source_high = source_high.max(from);
                        }
                    }
                }
                let slack = IVec3::splat(BRICK_DIM as i32 / 2 + 2);
                let from = BrickCoord::containing(source_low - slack).0;
                let to = BrickCoord::containing(source_high + slack).0;
                let mut fills = FxHashSet::default();
                let mut detail = false;
                'scan: for z in from.z..=to.z {
                    for y in from.y..=to.y {
                        for x in from.x..=to.x {
                            match self.brick_fill(BrickCoord::new(x, y, z)) {
                                Some(value) => {
                                    fills.insert(value.to_bits());
                                }
                                None => {
                                    detail = true;
                                    break 'scan;
                                }
                            }
                        }
                    }
                }
                if !detail && fills.len() == 1 {
                    let value = f32::from_bits(*fills.iter().next()?);
                    return (value <= INSIDE).then_some((coord, Brick::Uniform(INSIDE)));
                }
                let mut brick = Brick::dense_filled(OUTSIDE);
                let data = brick.make_dense();
                for z in 0..BRICK_DIM {
                    for y in 0..BRICK_DIM {
                        for x in 0..BRICK_DIM {
                            let at = voxel_of(origin, x, y, z).as_vec3() * voxel_size;
                            let value = self.sample_world(frame.inverse(at));
                            data[brick_index(x, y, z)] = value.clamp(INSIDE, OUTSIDE);
                        }
                    }
                }
                settled(brick).map(|brick| (coord, brick))
            })
            .collect();

        let mut out = Volume::new(voxel_size);
        let mut wanted = FxHashSet::default();
        for (coord, brick) in built {
            if matches!(brick, Brick::Dense(_)) {
                wanted.insert(coord);
            }
            out.insert_brick(coord, brick);
        }
        // The paint goes where the surface went, nearest voxel.
        let source_of = |cell: IVec3| {
            (frame.inverse(cell.as_vec3() * voxel_size) / voxel_size).round().as_ivec3()
        };
        *out.colour_mut() = self.colour().gathered_through(wanted.clone(), source_of);
        out.carry_layers_from(self, |paint| paint.gathered_through(wanted.clone(), source_of));
        out.mark_everything_dirty();
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::{PrimitiveKind, build};
    use proptest::prelude::*;

    fn cube() -> Volume {
        build(PrimitiveKind::Cube, 0.25, Vec3::ZERO, 5.0)
    }

    fn solid(volume: &Volume, at: Vec3) -> bool {
        volume.sample_world(at) < 0.0
    }

    #[test]
    fn a_twisted_cube_turns_its_middle_by_half_the_angle() {
        let how = Deformation { kind: DeformKind::Twist, axis: Axis::Y, amount: 90.0 };
        let twisted = cube().deformed(how).expect("a surface");
        // Turned 45 degrees, the square's corner stands on the X axis.
        assert!(solid(&twisted, Vec3::new(6.5, 0.0, 0.0)), "the corner came round");
        assert!(!solid(&twisted, Vec3::new(4.5, 0.0, 4.5)), "and left the diagonal");
        // The base is where it was.
        assert!(solid(&twisted, Vec3::new(3.5, -4.7, 3.5)));
    }

    #[test]
    fn a_bent_cube_lays_its_top_on_its_side() {
        let how = Deformation { kind: DeformKind::Bend, axis: Axis::Y, amount: 90.0 };
        let bent = cube().deformed(how).expect("a surface");
        let radius = 10.0 / 90f32.to_radians();
        let y = -5.0 + radius;
        assert!(solid(&bent, Vec3::new(radius - 0.4, y, 0.0)), "just inside the old top");
        assert!(!solid(&bent, Vec3::new(radius + 0.4, y, 0.0)), "just past it");
        assert!(solid(&bent, Vec3::new(0.0, -4.7, 0.0)), "the base stays");
    }

    #[test]
    fn a_taper_narrows_the_far_end_and_keeps_the_base() {
        let how = Deformation { kind: DeformKind::Taper, axis: Axis::Y, amount: 0.5 };
        let tapered = cube().deformed(how).expect("a surface");
        assert!(solid(&tapered, Vec3::new(2.2, 4.6, 0.0)));
        assert!(!solid(&tapered, Vec3::new(3.2, 4.6, 0.0)), "the top is half as wide");
        assert!(solid(&tapered, Vec3::new(4.7, -4.7, 0.0)), "the base keeps its size");
    }

    #[test]
    fn paint_goes_with_the_surface() {
        let mut volume = cube();
        let on_top = IVec3::new(0, 20, 18);
        // A small block, so that rounding either way still lands in it.
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    volume.colour_mut().write(on_top + IVec3::new(x, y, z), 5);
                }
            }
        }
        let how = Deformation { kind: DeformKind::Twist, axis: Axis::Y, amount: 90.0 };
        let twisted = volume.deformed(how).expect("a surface");
        // (0, 5, 4.5) at the top turns about 85.5 degrees, to near (4.5, 5, 0).
        let (low, high) = volume.surface_bounds().expect("a surface");
        let frame = Frame::new(how, low, high);
        let moved = (frame.forward(on_top.as_vec3() * 0.25) / 0.25).round().as_ivec3();
        assert_eq!(twisted.colour().at(moved), 5);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn inverse_undoes_forward(
            kind in prop::sample::select(DeformKind::ALL.to_vec()),
            axis in prop::sample::select(Axis::ALL.to_vec()),
            amount in -170.0f32..170.0,
            p in (-5.0f32..5.0, -5.0f32..5.0, -5.0f32..5.0),
        ) {
            // A bend tighter than the body is thick folds it through its own
            // centre of curvature, which no inverse undoes: kept to a radius
            // wider than the box.
            let amount = match kind {
                DeformKind::Taper => 1.0 + amount / 200.0,
                DeformKind::Bend => amount / 2.0,
                DeformKind::Twist => amount,
            };
            let how = Deformation { kind, axis, amount };
            let frame = Frame::new(how, Vec3::splat(-5.0), Vec3::splat(5.0));
            let p = Vec3::new(p.0, p.1, p.2);
            let back = frame.inverse(frame.forward(p));
            prop_assert!(back.distance(p) < 1e-3, "{:?} {} came back as {}", how, p, back);
        }

        #[test]
        fn nothing_asked_is_nothing_done(
            kind in prop::sample::select(DeformKind::ALL.to_vec()),
            axis in prop::sample::select(Axis::ALL.to_vec()),
            p in (-5.0f32..5.0, -5.0f32..5.0, -5.0f32..5.0),
        ) {
            let how = Deformation { kind, axis, amount: kind.identity() };
            let frame = Frame::new(how, Vec3::splat(-5.0), Vec3::splat(5.0));
            let p = Vec3::new(p.0, p.1, p.2);
            prop_assert!(frame.forward(p).distance(p) < 1e-4);
        }
    }
}
