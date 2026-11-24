// SPDX-License-Identifier: AGPL-3.0-only

//! A brush as its settings: amounts of the few things a brush can do.
//!
//! `docs/STUDIO-PLAN.md` section 16 is the design. [`super::engine`] made a
//! brush out of parts, and one of the parts was an operator chosen from a
//! list: Draw, Clay, Smooth, Inflate, Pinch, Flatten. A brush was exactly
//! one of them, and Crease had to be written as a seventh because it is
//! Draw and Pinch at once.
//!
//! Looked at for what they do to the field, the list is five **actions**:
//!
//! * **push**: read the field from behind, along the stroke's normal, so
//!   the surface moves out (Draw);
//! * **inflate**: offset the level set, so the surface swells along its own
//!   normals (Inflate);
//! * **pinch**: read the field from further from the brush's axis, so the
//!   surface is drawn toward it, or with the sign turned, spread from it
//!   (Pinch);
//! * **smooth**: blend toward the mean of the neighbours (Smooth);
//! * **level**: blend toward a plane, the one under the brush or one held
//!   off it, filling up to it, cutting down to it, or both (Flatten, Clay).
//!
//! A [`Compound`] is an amount of each, with a size, a strength and a tip.
//! The two that read the field from somewhere else add, as displacements,
//! into one read. Then the offset, then the two blends. One kernel, and a
//! crease is `push -0.6, pinch 1` with nothing written for it.
//!
//! # The classic brushes are the oracle
//!
//! A compound of one action at full amount does what the classic kernel of
//! that name does **to the bit**: the field, how each brick is stored, and
//! the undo entry. `one_action_is_the_classic_brush_of_that_name` holds it,
//! over the strokes and the matrix the engine itself was first held to.
//! That is why the arithmetic below is written the way it is, with each
//! amount multiplied in last (a multiplication by one is exact) and each
//! action skipped outright when its amount is nothing (an addition of
//! nothing is exact too, but a read that is not made is cheaper).
//!
//! # What the invert key reverses
//!
//! What the brush BUILDS: the push, the inflation, and which side of its
//! plane a levelling brush fills. It reverses the pinch only when pinch is
//! all the brush does, which is what Pinch has always done with it, and
//! leaves the pinch of a crease a pinch, which is what Crease has always
//! done.

use contracts::{debug_ensures, debug_requires};
use glam::Vec3;

mod layer;

pub use layer::{LAYER_DEPTH, LAYER_STAMPS, Layer, MOST_OFFSET_VOXELS};

use super::alpha::Alpha;
use super::{
    BrushDirection, BrushKind, BrushScratch, CLAY_OFFSET, FalloffCurve, MAX_STAMP_VOXELS,
    PINCH_PULL_VOXELS, STAMP_FRACTION_OF_RADIUS, Stamp, Symmetry, admit,
    blend_toward_plane_clamps_back,
};
use crate::brick::{INSIDE, OUTSIDE};
use crate::mask::PROTECTED;
use crate::pattern::{Pattern, Prepared};
use crate::volume::{BrickPreview, BrickVerdict, Volume};

/// How far a crease cuts as a share of a full push. The classic Crease's
/// own number, under its own name: see [`super::engine::CREASE_DEPTH`].
pub const CREASE_PUSH: f32 = -super::engine::CREASE_DEPTH;

/// How far a gather reads from beyond where a voxel is, as a share of the
/// voxel's distance from the stroke's line, at full strength and full
/// weight.
///
/// **A share of the distance, and that is the whole of what makes it a
/// pinch.** Upstream's pinch reads from a fixed step away, however near
/// the axis the voxel is. A voxel beside the axis then reads from most of
/// a voxel off, and the top of a ridge under the brush is either smeared
/// flat (reading from nearer) or lost (reading from further). A step that
/// is a share of the distance is nothing ON the line and grows away from
/// it, which is a squeeze of everything toward the line: a ridge is
/// narrower by that share at each stamp, and its top, on the line, is
/// where it was.
pub const GATHER: f32 = 0.2;

/// The most a gather's share may be at one voxel, whatever the strength:
/// less than all of the distance, so that a spread never reads from the
/// other side of the line.
const MOST_GATHER: f32 = 0.5;

/// The most of the middle of a stroke that may be flat. Less than all of
/// it, so that a brush always has a rim that tapers and never a wall.
pub const MOST_PLATEAU: f32 = 0.95;

/// Which way a levelling brush moves the surface to its plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    /// Up to it and down to it: Flatten.
    #[default]
    Both,
    /// Up to it only, so that material is added and none is taken: Clay.
    Fill,
    /// Down to it only, so that what stands proud is taken off and nothing
    /// is filled: a scraper.
    Cut,
}

impl Side {
    pub const ALL: [Side; 3] = [Side::Both, Side::Fill, Side::Cut];

    pub fn label(self) -> &'static str {
        match self {
            Side::Both => "Both",
            Side::Fill => "Fill",
            Side::Cut => "Cut",
        }
    }
}

/// What a brush does to the surface, as an amount of each thing a brush
/// can do. Nothing of all of them is a brush that does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Actions {
    /// Move the surface out along the stroke's normal, -1..=1. Negative
    /// moves it in.
    pub push: f32,
    /// Swell the surface along its own normals, -1..=1. Negative shrinks.
    pub inflate: f32,
    /// Upstream's pinch, -1..=1: read the field from nearer the brush's
    /// axis by a fixed step, or with a negative amount from further off.
    /// Named for what upstream calls it and not for what it does to a
    /// ridge under the brush, which is to keep its top and make it
    /// broader and steeper sided; negative, it wears the ridge away. See
    /// [`Actions::gather`], which is the pinch a ridge is narrowed by.
    pub pinch: f32,
    /// Gather the surface toward the stroke's line, -1..=1: a ridge under
    /// the brush is narrower and as tall. Negative spreads it from the
    /// line: wider, and as tall. See [`GATHER`].
    pub gather: f32,
    /// Blend toward the mean of the neighbours, 0..=1.
    pub smooth: f32,
    /// Blend toward the plane, 0..=1.
    pub level: f32,
    /// How far off the surface the plane is held, in radii of the brush,
    /// -1..=1. Positive is proud of it, which is where clay builds to.
    pub plane: f32,
    pub side: Side,
}

impl Actions {
    /// These amounts brought to what a brush can be: each within its range,
    /// anything that is not a number taken as nothing, and the push and the
    /// inflation together no more than one stamp may move the surface.
    ///
    /// The last is the brush's own rule ([`super::engine`], rule 3): a
    /// stamp moves the surface by at most a voxel, because stamps overlap
    /// and what each does compounds. Two actions that each move it a voxel
    /// would be two voxels.
    #[debug_ensures(ret.is_legal(), "what is made legal is legal")]
    pub fn legal(self) -> Self {
        let number = |amount: f32| if amount.is_finite() { amount } else { 0.0 };
        let mut push = number(self.push).clamp(-1.0, 1.0);
        let mut inflate = number(self.inflate).clamp(-1.0, 1.0);
        let moved = push.abs() + inflate.abs();
        if moved > 1.0 {
            push /= moved;
            inflate /= moved;
            // A division is not exact, and the sum of two that should come
            // to one may come to a hair over it.
            if push.abs() + inflate.abs() > 1.0 {
                inflate = inflate.signum() * (1.0 - push.abs());
            }
        }
        Self {
            push,
            inflate,
            pinch: number(self.pinch).clamp(-1.0, 1.0),
            gather: number(self.gather).clamp(-1.0, 1.0),
            smooth: number(self.smooth).clamp(0.0, 1.0),
            level: number(self.level).clamp(0.0, 1.0),
            plane: number(self.plane).clamp(-1.0, 1.0),
            side: self.side,
        }
    }

    /// Whether these amounts are ones [`Actions::legal`] would leave alone.
    pub fn is_legal(&self) -> bool {
        (-1.0..=1.0).contains(&self.push)
            && (-1.0..=1.0).contains(&self.inflate)
            && self.push.abs() + self.inflate.abs() <= 1.0
            && (-1.0..=1.0).contains(&self.pinch)
            && (-1.0..=1.0).contains(&self.gather)
            && (0.0..=1.0).contains(&self.smooth)
            && (0.0..=1.0).contains(&self.level)
            && (-1.0..=1.0).contains(&self.plane)
    }

    /// Whether the brush does nothing at all.
    pub fn is_nothing(&self) -> bool {
        self.push == 0.0
            && self.inflate == 0.0
            && self.pinch == 0.0
            && self.gather == 0.0
            && self.smooth == 0.0
            && self.level == 0.0
    }

    /// What the classic brush of this kind does, or nothing for the two
    /// that are not brushes on the field: Move, which drags the surface
    /// with the pointer, and Paint, which writes no distances at all.
    pub fn of(kind: BrushKind) -> Option<Actions> {
        match kind {
            BrushKind::Draw => Some(Actions::DRAW),
            BrushKind::Clay => Some(Actions::CLAY),
            BrushKind::Smooth => Some(Actions::SMOOTH),
            BrushKind::Inflate => Some(Actions::INFLATE),
            BrushKind::Pinch => Some(Actions::PINCH),
            BrushKind::Flatten => Some(Actions::FLATTEN),
            BrushKind::Move | BrushKind::Paint => None,
        }
    }

    /// The classic brush these amounts are nearest to: the one whose key,
    /// whose remembered strength and whose answer to "does the invert key
    /// do anything" a brush made of them goes by.
    ///
    /// The last is the one that has to be right. A brush with no push, no
    /// inflation and no pinch, which levels to the plane under it from
    /// both sides or not at all, has no opposite, and its family is one of
    /// the two classic brushes that have none ([`BrushKind::is_directional`]).
    /// `a_brush_of_a_family_with_no_opposite_has_none` holds that to the
    /// kernel.
    #[debug_ensures(Actions::of(ret).is_some(), "a family is a brush on the field")]
    pub fn family(&self) -> BrushKind {
        if self.push != 0.0 {
            BrushKind::Draw
        } else if self.inflate != 0.0 {
            BrushKind::Inflate
        } else if self.pinch != 0.0 || self.gather != 0.0 {
            BrushKind::Pinch
        } else if self.level == 0.0 {
            BrushKind::Smooth
        } else if self.side == Side::Both && self.plane == 0.0 {
            BrushKind::Flatten
        } else {
            BrushKind::Clay
        }
    }

    /// What Draw does: push.
    pub const DRAW: Actions = Actions { push: 1.0, ..Actions::NOTHING };
    /// What Inflate does.
    pub const INFLATE: Actions = Actions { inflate: 1.0, ..Actions::NOTHING };
    /// What Pinch does.
    pub const PINCH: Actions = Actions { pinch: 1.0, ..Actions::NOTHING };
    /// A true pinch: gather the surface toward the stroke's line.
    pub const GATHER: Actions = Actions { gather: 1.0, ..Actions::NOTHING };
    /// What Smooth does.
    pub const SMOOTH: Actions = Actions { smooth: 1.0, ..Actions::NOTHING };
    /// What Flatten does: level to the plane under the brush.
    pub const FLATTEN: Actions = Actions { level: 1.0, ..Actions::NOTHING };
    /// What Clay does: fill up to a plane held proud of the surface.
    pub const CLAY: Actions =
        Actions { level: 1.0, plane: CLAY_OFFSET, side: Side::Fill, ..Actions::NOTHING };
    /// What Crease does: cut in and pinch shut.
    pub const CREASE: Actions = Actions { push: CREASE_PUSH, pinch: 1.0, ..Actions::NOTHING };

    const NOTHING: Actions = Actions {
        push: 0.0,
        inflate: 0.0,
        pinch: 0.0,
        gather: 0.0,
        smooth: 0.0,
        level: 0.0,
        plane: 0.0,
        side: Side::Both,
    };
}

/// The shape of a stroke's cross-section, from its middle to its rim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    /// No slope at the middle or the rim. [`FalloffCurve::Smooth`].
    #[default]
    Smooth,
    /// A straight side: a cone. [`FalloffCurve::Linear`].
    Linear,
    /// Gathered at the middle: a spike. [`FalloffCurve::Sharp`].
    Sharp,
    /// Broad, with a quick roll off at the rim. [`FalloffCurve::Wide`].
    Wide,
    /// A dome: the cross-section of a ball, upright at the rim.
    Round,
}

impl Shape {
    pub const ALL: [Shape; 5] =
        [Shape::Smooth, Shape::Linear, Shape::Sharp, Shape::Wide, Shape::Round];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Smooth => "Smooth",
            Shape::Linear => "Linear",
            Shape::Sharp => "Sharp",
            Shape::Wide => "Wide",
            Shape::Round => "Round",
        }
    }

    /// The falloff curve this shape is, or nothing for the one shape no
    /// curve of upstream's has.
    pub fn curve(self) -> Option<FalloffCurve> {
        match self {
            Shape::Smooth => Some(FalloffCurve::Smooth),
            Shape::Linear => Some(FalloffCurve::Linear),
            Shape::Sharp => Some(FalloffCurve::Sharp),
            Shape::Wide => Some(FalloffCurve::Wide),
            Shape::Round => None,
        }
    }

    /// The shape a falloff curve is.
    pub fn of(curve: FalloffCurve) -> Self {
        match curve {
            FalloffCurve::Smooth => Shape::Smooth,
            FalloffCurve::Linear => Shape::Linear,
            FalloffCurve::Sharp => Shape::Sharp,
            FalloffCurve::Wide => Shape::Wide,
        }
    }
}

/// The cross-section of a stroke: a shape, and how much of the middle is
/// flat before the shape begins.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Profile {
    pub shape: Shape,
    /// The share of the radius that is flat, 0..=[`MOST_PLATEAU`]. The
    /// shape runs from there to the rim.
    pub plateau: f32,
}

impl Profile {
    /// The profile a falloff curve is: its shape and no plateau.
    pub fn of(curve: FalloffCurve) -> Self {
        Self { shape: Shape::of(curve), plateau: 0.0 }
    }

    /// The weight at a distance from the middle, as a share of the radius:
    /// 1 on the plateau, falling to 0 at the rim.
    ///
    /// With no plateau the four shapes upstream has are upstream's curves,
    /// to the bit: the distance is handed to the curve as it came.
    #[inline]
    #[debug_ensures((0.0..=1.0).contains(&ret) || distance.is_nan(), "a weight is a share")]
    pub fn weight(self, distance: f32) -> f32 {
        self.slope().weight(distance)
    }

    /// This profile made ready for a stamp: the plateau brought into its
    /// range and the division it implies done, once and not at every voxel.
    #[inline]
    fn slope(self) -> Slope {
        let plateau =
            if self.plateau.is_finite() { self.plateau.clamp(0.0, MOST_PLATEAU) } else { 0.0 };
        Slope { shape: self.shape, plateau, steepness: 1.0 / (1.0 - plateau) }
    }
}

/// A [`Profile`] with its arithmetic done. A stamp asks the weight of every
/// voxel in its box, and asked the plain way (bring the plateau into range,
/// divide, choose) it was a third of what a stamp cost: measured.
#[derive(Debug, Clone, Copy)]
struct Slope {
    shape: Shape,
    plateau: f32,
    /// One over what is left of the radius past the plateau.
    steepness: f32,
}

impl Slope {
    /// With no plateau this is the distance as it came, to the bit: nothing
    /// taken from it and one multiplied into it.
    #[inline(always)]
    fn weight(&self, distance: f32) -> f32 {
        let along = ((distance - self.plateau) * self.steepness).max(0.0);
        match self.shape {
            Shape::Smooth => FalloffCurve::Smooth.weight(along),
            Shape::Linear => FalloffCurve::Linear.weight(along),
            Shape::Sharp => FalloffCurve::Sharp.weight(along),
            Shape::Wide => FalloffCurve::Wide.weight(along),
            Shape::Round => {
                let along = along.min(1.0);
                (1.0 - along * along).sqrt()
            }
        }
    }
}

/// A brush as its settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compound {
    pub actions: Actions,
    /// Radius of influence in world units.
    pub radius: f32,
    /// How hard one stamp bites. Per stamp and not per stroke.
    pub strength: f32,
    pub profile: Profile,
    pub pattern: Pattern,
}

impl Compound {
    /// Apply one stamp, plus its mirrors when symmetry is on. `tip` is an
    /// image across the brush, or nothing for a plain one.
    pub fn apply_symmetric(
        &self,
        volume: &mut Volume,
        stamp: &Stamp,
        tip: Option<&Alpha>,
        symmetry: Symmetry,
        centre: Vec3,
        scratch: &mut BrushScratch,
    ) {
        self.apply(volume, stamp, tip, scratch);
        if symmetry.is_off() {
            return;
        }
        let mut twins = [*stamp; Symmetry::MAX_MIRRORS];
        let count = symmetry.mirrors(stamp, centre, &mut twins);
        for twin in &twins[..count] {
            self.apply(volume, twin, tip, scratch);
        }
    }

    /// Apply one stamp.
    ///
    /// Which of the actions the brush has is looked at here, once, and the
    /// stamp is run by the loop compiled for exactly those: the engine's
    /// own rule ([`super::engine`], rule 5) that the choice costs one jump
    /// a stamp and nothing a voxel. Written the plain way first, with each
    /// action asked about at each voxel, a stamp of one action cost from a
    /// quarter to nine tenths more than the classic kernel of that action:
    /// measured, with `what_a_generic_stamp_costs_beside_a_classic_one`.
    #[debug_requires(self.actions.is_legal(), "a brush's amounts are within what a brush can be")]
    pub fn apply(
        &self,
        volume: &mut Volume,
        stamp: &Stamp,
        tip: Option<&Alpha>,
        scratch: &mut BrushScratch,
    ) {
        let actions = self.actions;
        let has = (
            actions.push != 0.0 || actions.pinch != 0.0 || actions.gather != 0.0,
            actions.inflate != 0.0,
            actions.smooth != 0.0,
            actions.level != 0.0,
        );
        // Thirty-two loops: one for each set of actions a brush can have,
        // with an image across its tip and without.
        macro_rules! run {
            ($($warps:literal $inflates:literal $smooths:literal $levels:literal,)*) => {
                match (has, tip.is_some()) {
                    $((($warps, $inflates, $smooths, $levels), false) => self
                        .run::<$warps, $inflates, $smooths, $levels, false>(
                            volume, stamp, tip, scratch,
                        ),
                    (($warps, $inflates, $smooths, $levels), true) => self
                        .run::<$warps, $inflates, $smooths, $levels, true>(
                            volume, stamp, tip, scratch,
                        ),)*
                }
            };
        }
        run! {
            false false false false, false false false true,
            false false true false, false false true true,
            false true false false, false true false true,
            false true true false, false true true true,
            true false false false, true false false true,
            true false true false, true false true true,
            true true false false, true true false true,
            true true true false, true true true true,
        }
    }

    /// One stamp, by the loop for a brush that has these actions and no
    /// others: `WARPS` for a push or a pinch, which are one read, one each
    /// for the other three, and `TIPPED` for an image across the tip.
    ///
    /// The structure is [`super::Brush::stamp`]'s and the engine's, step
    /// for step, and where one action is all the brush does the arithmetic
    /// is that kernel's arithmetic in its order.
    fn run<
        const WARPS: bool,
        const INFLATES: bool,
        const SMOOTHS: bool,
        const LEVELS: bool,
        const TIPPED: bool,
    >(
        &self,
        volume: &mut Volume,
        stamp: &Stamp,
        tip: Option<&Alpha>,
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
        let gain = self.strength * stamp.pressure;
        let displacement = (radius / voxel_size * STAMP_FRACTION_OF_RADIUS).min(MAX_STAMP_VOXELS);
        let direction = stamp.direction;
        let normal = stamp.normal;
        let centre = stamp.centre;

        // What each action needs of the stamp. The two that read the field
        // from somewhere else, and the one that reads its neighbours, need a
        // copy of it; the other two write from the voxel's own value.
        let reads = WARPS || SMOOTHS;
        let pinch_alone = actions.push == 0.0
            && actions.inflate == 0.0
            && actions.smooth == 0.0
            && actions.level == 0.0;
        // Pinch turns into spread under the invert key only when it is all
        // the brush does. See the module documentation.
        // Which way the stroke is going, laid into the surface: what a
        // gather draws toward the line of. Nothing for a stamp that has
        // not travelled.
        let along = {
            let lying = stamp.tangent - normal * stamp.tangent.dot(normal);
            lying.try_normalize()
        };
        let pinch_turn = match direction {
            BrushDirection::Subtract if pinch_alone => -1.0,
            _ => 1.0,
        };
        // The plane: under the brush, or held off it along the normal, on
        // the side the brush is building toward.
        let plane_point = if actions.plane != 0.0 {
            centre + normal * (radius * actions.plane * direction.outward_sign())
        } else {
            centre
        };

        // Two boxes: the one written is the brush's own reach, the one read
        // is that grown by however far the warps read from.
        let reach_voxels = gain
            * (actions.push.abs() * displacement
                + (actions.pinch.abs() + actions.gather.abs()) * PINCH_PULL_VOXELS);
        let reach = reach_voxels * voxel_size;
        let extent = Vec3::splat(radius);
        let (lo, hi) = volume.voxel_bounds(centre - extent, centre + extent);
        let read_extent = extent + Vec3::splat(reach);
        let (read_lo, read_hi) = volume.voxel_bounds(centre - read_extent, centre + read_extent);
        if reads {
            volume.snapshot(read_lo, read_hi, &mut scratch.region);
        }
        let region = &scratch.region;

        let pattern = if self.pattern.is_off() {
            Prepared::OFF
        } else {
            self.pattern.prepare(voxel_size, normal, stamp.tangent)
        };
        let slope = self.profile.slope();
        let frame = Frame::of(stamp);

        let read_voxels = (reach / voxel_size).ceil() as i32 + 1;

        // Which bricks of the box can change at all. Each action's own
        // answer, as the classic kernel of that action gives it, and the
        // brush's answer is the most cautious of them: a different verdict
        // is a different undo entry even where the field agrees.
        let level_alone = actions.push == 0.0
            && actions.inflate == 0.0
            && actions.pinch == 0.0
            && actions.gather == 0.0
            && actions.smooth == 0.0;
        let inflating = match direction {
            // Which way the offset goes, with the amount's own sign.
            BrushDirection::Add => actions.inflate,
            BrushDirection::Subtract => -actions.inflate,
        };
        let leaves_saturated_alone = move |value: f32| {
            // A blend toward a plane varies across a brick, so nothing can
            // be said of a constant. An offset lands back on the clamp it
            // started at only in the direction the clamp already holds. A
            // read of a constant from anywhere, and the mean of six equal
            // values, are that constant.
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
            if centre.clamp(box_min, box_max).distance(centre) >= radius {
                return BrickVerdict::Skip;
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

        let radius_squared = radius * radius;

        // What `decide` says of a brick that holds one value, said of a
        // voxel. See `volume/settled.rs`.
        let settled = crate::Settled {
            inside: leaves_saturated_alone(INSIDE),
            outside: leaves_saturated_alone(OUTSIDE),
        };

        volume.edit_voxels_where(
            lo,
            hi,
            read_voxels,
            true,
            settled,
            decide,
            move |voxel, position, value, free| {
                let offset = position - centre;
                if offset.length_squared() >= radius_squared {
                    return value;
                }
                let distance = offset.length() * inverse_radius;
                let shaped = slope.weight(distance) * gain;
                if shaped <= 0.0 {
                    return value;
                }
                // The tip's factors. Each stays in 0..=1, so the product
                // does.
                let mut patterned = shaped * pattern.weight(position);
                if TIPPED && let Some(image) = tip {
                    patterned *= frame.height(image, offset, inverse_radius);
                }
                if patterned <= 0.0 || free <= 0.0 {
                    return value;
                }
                let weight = patterned * free;
                let mut held = value;

                // The reads from somewhere else, as one read. Each amount
                // is multiplied in last: by one, that is exact.
                if WARPS {
                    let mut from = position;
                    let mut read = false;
                    if actions.push != 0.0 {
                        let shift = normal
                            * (weight
                                * displacement
                                * voxel_size
                                * direction.outward_sign()
                                * actions.push);
                        from -= shift;
                        read = true;
                    }
                    if actions.pinch != 0.0 {
                        let to_centre = centre - position;
                        // Along the surface only, or pinch quietly doubles
                        // as inflate.
                        let lateral = to_centre - normal * to_centre.dot(normal);
                        // On the axis there is nowhere to be drawn toward.
                        if let Some(to_axis) = lateral.try_normalize() {
                            let pull = to_axis
                                * (weight
                                    * PINCH_PULL_VOXELS
                                    * voxel_size
                                    * pinch_turn
                                    * actions.pinch);
                            from += pull;
                            read = true;
                        }
                    }
                    if actions.gather != 0.0 {
                        // How far the voxel is from the stroke's line,
                        // along the surface: from the line where the
                        // stroke has a way it is going, and from the
                        // brush's axis where it has not yet.
                        let from_centre = position - centre;
                        let mut off = from_centre - normal * from_centre.dot(normal);
                        if let Some(along) = along {
                            off -= along * off.dot(along);
                        }
                        // Read from further off the line by a share of how
                        // far off it the voxel is, which is what narrows
                        // whatever is there and leaves what is ON the line
                        // where it was: see `GATHER`. Spreading reads from
                        // nearer the line by the same share, and a share
                        // of less than all of it cannot cross the line.
                        let share = (GATHER * weight * pinch_turn * actions.gather)
                            .clamp(-MOST_GATHER, MOST_GATHER);
                        let step = off * share;
                        let most = PINCH_PULL_VOXELS * voxel_size;
                        from += step.clamp_length_max(most);
                        read = true;
                    }
                    if read {
                        held = region.sample(from / voxel_size);
                    }
                }

                // The offset of the level set.
                if INFLATES {
                    held += direction.field_sign() * weight * displacement * actions.inflate;
                }

                // Toward the mean of the neighbours.
                if SMOOTHS {
                    let average = region.neighbour_average(voxel);
                    held += (average - held) * (weight * actions.smooth);
                }

                // Toward the plane. The mask is let in after the blend and
                // not folded into its weight, as Flatten and Clay have it:
                // see `admit`.
                if LEVELS {
                    let plane = (position - plane_point).dot(normal) / voxel_size;
                    let blended = held + (plane - held) * (patterned * actions.level);
                    // Only the half of the blend that moves material the
                    // way this side of the brush moves it.
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
    }
}

/// Where a point of a stamp is across its tip: along the stroke and across
/// it, on the surface.
#[derive(Debug, Clone, Copy)]
struct Frame {
    along: Vec3,
    across: Vec3,
}

impl Frame {
    fn of(stamp: &Stamp) -> Self {
        let out = stamp.normal.normalize_or(Vec3::Z);
        // The stroke's direction, laid into the surface. A stamp that has
        // not travelled has none: then any direction across the surface
        // does, and the same one each time.
        let lying = stamp.tangent - out * stamp.tangent.dot(out);
        let along = if lying.length_squared() > 1e-12 {
            lying.normalize()
        } else {
            out.any_orthonormal_vector()
        };
        Self { along, across: out.cross(along) }
    }

    /// The image's height over a point `offset` from the stamp's centre.
    #[inline]
    fn height(&self, image: &Alpha, offset: Vec3, inverse_radius: f32) -> f32 {
        let u = offset.dot(self.across) * inverse_radius;
        let v = offset.dot(self.along) * inverse_radius;
        image.sample(u, v).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::engine::{BrushDef, FieldOp, Tip};
    use crate::brush::{Brush, MaskOp, MirrorAxis};
    use crate::pattern::PatternKind;
    use crate::testing::assert_same_field;

    const CENTRE: Vec3 = Vec3::splat(48.0);
    const RADIUS: f32 = 30.0;

    /// The engine's own fixture: a ball big enough to have solid interior
    /// under the brush and far enough from the origin to straddle brick
    /// boundaries on every axis.
    fn ball() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(CENTRE, RADIUS);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// Where a short stroke lands, as directions from the ball's centre.
    /// The stamps overlap, so a difference in one compounds into the next.
    fn stroke() -> [Vec3; 4] {
        [
            Vec3::new(0.6, 0.8, 0.0),
            Vec3::new(0.55, 0.8, 0.2),
            Vec3::new(0.5, 0.8, 0.35),
            Vec3::new(0.4, 0.8, 0.45),
        ]
        .map(Vec3::normalize)
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Masking {
        None,
        Partial,
        PartialInverted,
    }

    fn prepare(masking: Masking) -> Volume {
        let mut volume = ball();
        if masking == Masking::None {
            return volume;
        }
        let masker = Brush { radius: 11.0, strength: 0.7, ..Brush::default() };
        let mut scratch = BrushScratch::new();
        for towards in &stroke()[..2] {
            let at = CENTRE + *towards * RADIUS;
            masker.apply_mask(
                &mut volume,
                &Stamp::new(at, *towards, BrushDirection::Add),
                MaskOp::Raise,
                &mut scratch,
            );
        }
        if masking == Masking::PartialInverted {
            let _ = volume.flip_mask_polarity();
        }
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// Run the stroke through `stamp_with` and hand back the volume and the
    /// undo entry it recorded.
    fn stroked(
        masking: Masking,
        pressure: f32,
        direction: BrushDirection,
        mut stamp_with: impl FnMut(&mut Volume, &Stamp, &mut BrushScratch),
    ) -> (Volume, Option<(usize, usize)>) {
        let mut volume = prepare(masking);
        volume.begin_stroke();
        let mut scratch = BrushScratch::new();
        let path = stroke();
        for (index, towards) in path.iter().enumerate() {
            let at = CENTRE + *towards * RADIUS;
            let next = path[(index + 1) % path.len()];
            let stamp = Stamp::new(at, *towards, direction)
                .with_pressure(pressure)
                .with_tangent((next - *towards).normalize_or_zero());
            stamp_with(&mut volume, &stamp, &mut scratch);
        }
        let undo = volume.end_stroke().map(|edit| (edit.len(), edit.bytes()));
        (volume, undo)
    }

    /// The compound that is each classic operator, and nothing else.
    fn classic(op: FieldOp) -> Actions {
        match op {
            FieldOp::Draw => Actions::DRAW,
            FieldOp::Clay => Actions::CLAY,
            FieldOp::Smooth => Actions::SMOOTH,
            FieldOp::Inflate => Actions::INFLATE,
            FieldOp::Pinch => Actions::PINCH,
            FieldOp::Flatten => Actions::FLATTEN,
            FieldOp::Crease => Actions::CREASE,
        }
    }

    /// One action at full amount is the classic brush of that name, bit for
    /// bit: the field, how each brick is stored, and the undo entry. Over
    /// every falloff, pattern, direction, pressure and mask the engine was
    /// first held to its own oracle with.
    #[test]
    fn one_action_is_the_classic_brush_of_that_name() {
        let mut compared = 0;
        let mut changed = 0;
        for op in FieldOp::CLASSIC {
            for falloff in FalloffCurve::ALL {
                for pattern in PatternKind::ALL {
                    for direction in [BrushDirection::Add, BrushDirection::Subtract] {
                        for pressure in [1.0, 0.4] {
                            for masking in
                                [Masking::None, Masking::Partial, Masking::PartialInverted]
                            {
                                let pattern = Pattern { kind: pattern, scale_mm: 5.0, depth: 0.8 };
                                let def = BrushDef {
                                    op,
                                    radius: 9.0,
                                    strength: 0.6,
                                    tip: Tip { falloff, pattern },
                                };
                                let compound = Compound {
                                    actions: classic(op),
                                    radius: 9.0,
                                    strength: 0.6,
                                    profile: Profile::of(falloff),
                                    pattern,
                                };
                                let what = format!(
                                    "{op:?} {falloff} {:?} {direction:?} pressure {pressure} \
                                     {masking:?}",
                                    pattern.kind
                                );
                                let (old, old_undo) =
                                    stroked(masking, pressure, direction, |v, s, scratch| {
                                        def.apply(v, s, scratch)
                                    });
                                let (new, new_undo) =
                                    stroked(masking, pressure, direction, |v, s, scratch| {
                                        compound.apply(v, s, None, scratch)
                                    });
                                assert_same_field(&old, &new, &what);
                                assert_eq!(old_undo, new_undo, "{what}: the undo entry differs");
                                compared += 1;
                                changed += usize::from(old_undo.is_some());
                            }
                        }
                    }
                }
            }
        }
        // The control: two paths that both did nothing agree perfectly.
        assert_eq!(compared, 6 * 4 * 6 * 2 * 2 * 3);
        assert_eq!(changed, compared, "some strokes changed nothing");
    }

    /// And under every mirror, and with a brush big enough to have whole
    /// bricks of solid under it, which is the only place the verdicts that
    /// skip a saturated brick are exercised.
    #[test]
    fn one_action_is_the_classic_brush_mirrored_and_at_a_radius_that_reaches_the_tiles() {
        let symmetries = [
            Symmetry::OFF.with_axis(MirrorAxis::X, true),
            Symmetry::OFF
                .with_axis(MirrorAxis::X, true)
                .with_axis(MirrorAxis::Y, true)
                .with_axis(MirrorAxis::Z, true),
        ];
        for op in FieldOp::CLASSIC {
            for direction in [BrushDirection::Add, BrushDirection::Subtract] {
                let pattern = Pattern { kind: PatternKind::Hair, scale_mm: 5.0, depth: 0.8 };
                for (radius, symmetry) in
                    [(9.0, symmetries[0]), (9.0, symmetries[1]), (26.0, Symmetry::OFF)]
                {
                    let def = BrushDef {
                        op,
                        radius,
                        strength: 0.6,
                        tip: Tip { falloff: FalloffCurve::Smooth, pattern },
                    };
                    let compound = Compound {
                        actions: classic(op),
                        radius,
                        strength: 0.6,
                        profile: Profile::default(),
                        pattern,
                    };
                    let what = format!("{op:?} {direction:?} radius {radius} {}", symmetry.label());
                    let (old, old_undo) =
                        stroked(Masking::Partial, 1.0, direction, |v, s, scratch| {
                            def.apply_symmetric(v, s, symmetry, CENTRE, scratch)
                        });
                    let (new, new_undo) =
                        stroked(Masking::Partial, 1.0, direction, |v, s, scratch| {
                            compound.apply_symmetric(v, s, None, symmetry, CENTRE, scratch)
                        });
                    assert_same_field(&old, &new, &what);
                    assert_eq!(old_undo, new_undo, "{what}: the undo entry differs");
                    assert!(old_undo.is_some(), "{what}: the stroke changed nothing");
                }
            }
        }
    }

    /// Every voxel the two volumes hold, and the most they differ by.
    /// Leaving settled voxels alone changes nothing a compound brush
    /// writes: each classic set of actions and two that are several at
    /// once, both ways, at a radius where there are bricks with detail in
    /// them to settle, against the same stamps with no voxel treated as
    /// settled.
    #[test]
    fn settled_voxels_change_nothing_a_compound_brush_writes() {
        let mut settled_some = 0;
        let several = [
            Actions { push: 0.6, pinch: 0.5, ..Actions::NOTHING },
            Actions { smooth: 0.5, gather: 0.4, ..Actions::NOTHING },
        ];
        let all = FieldOp::CLASSIC.into_iter().map(classic).chain(several);
        for (index, actions) in all.enumerate() {
            for direction in [BrushDirection::Add, BrushDirection::Subtract] {
                let compound = Compound {
                    actions,
                    radius: 40.0,
                    strength: 0.6,
                    profile: Profile::default(),
                    pattern: Pattern::default(),
                };
                let what = format!("actions {index} {direction:?}");
                let run = || {
                    let mut volume = ball();
                    volume.begin_stroke();
                    let mut scratch = BrushScratch::new();
                    let mut settled = 0;
                    for stand_off in [0.0, -6.0, 6.0, 0.0] {
                        let at = CENTRE + Vec3::X * (RADIUS + stand_off);
                        let stamp = Stamp::new(at, Vec3::X, direction);
                        compound.apply(&mut volume, &stamp, None, &mut scratch);
                        settled += volume.last_plan().bricks_settled;
                    }
                    let undo = volume.end_stroke().map(|edit| (edit.len(), edit.bytes()));
                    let mut dirty = Vec::new();
                    volume.take_dirty(&mut dirty);
                    dirty.sort_unstable();
                    (volume, undo, dirty, settled)
                };
                let (with, with_undo, with_dirty, settled) = run();
                let (without, without_undo, without_dirty, none) =
                    crate::volume::without_settled(run);
                assert_eq!(none, 0, "{what}: the comparison settled voxels too");
                assert_same_field(&with, &without, &what);
                assert_eq!(with_undo, without_undo, "{what}: the undo entry differs");
                assert_eq!(with_dirty, without_dirty, "{what}: different bricks to remesh");
                settled_some += usize::from(settled > 0);
            }
        }
        assert!(settled_some >= 6, "only {settled_some} of the runs settled any voxel");
    }

    fn furthest_apart(one: &Volume, other: &Volume) -> f32 {
        let mut most = 0.0_f32;
        for z in 10..90 {
            for y in 10..90 {
                for x in 10..90 {
                    let voxel = glam::IVec3::new(x, y, z);
                    most = most.max((one.sample_voxel(voxel) - other.sample_voxel(voxel)).abs());
                }
            }
        }
        most
    }

    /// A crease is a push inward and a pinch, with nothing written for it:
    /// the classic Crease to within rounding, in both directions. Not to
    /// the bit, because the classic multiplies its depth in at another
    /// place in the product.
    #[test]
    fn a_crease_is_a_push_and_a_pinch() {
        for direction in [BrushDirection::Add, BrushDirection::Subtract] {
            let def =
                BrushDef { op: FieldOp::Crease, radius: 9.0, strength: 0.6, tip: Tip::default() };
            let compound = Compound {
                actions: Actions::CREASE,
                radius: 9.0,
                strength: 0.6,
                profile: Profile::default(),
                pattern: Pattern::default(),
            };
            let (old, _) =
                stroked(Masking::None, 1.0, direction, |v, s, scratch| def.apply(v, s, scratch));
            let (new, _) = stroked(Masking::None, 1.0, direction, |v, s, scratch| {
                compound.apply(v, s, None, scratch)
            });
            let apart = furthest_apart(&old, &new);
            assert!(apart < 1e-4, "{direction:?}: a crease and its two actions differ by {apart}");
            // Control: it cut something.
            assert!(furthest_apart(&old, &ball()) > 0.2);
        }
    }

    /// Two actions at once do something neither does alone, and a brush of
    /// no actions does nothing and records nothing.
    #[test]
    fn actions_add_up_and_none_of_them_is_nothing() {
        let with = |actions: Actions| {
            let compound = Compound {
                actions,
                radius: 9.0,
                strength: 0.6,
                profile: Profile::default(),
                pattern: Pattern::default(),
            };
            stroked(Masking::None, 1.0, BrushDirection::Add, |v, s, scratch| {
                compound.apply(v, s, None, scratch)
            })
        };
        let (nothing, undo) = with(Actions::default());
        assert!(undo.is_none(), "a brush of no actions recorded an edit");
        assert_same_field(&nothing, &ball(), "a brush of no actions");

        let (pushed, _) = with(Actions::DRAW);
        let (smoothed, _) = with(Actions::SMOOTH);
        let (both, _) = with(Actions { smooth: 1.0, ..Actions::DRAW });
        assert!(furthest_apart(&both, &pushed) > 0.01, "smoothing a push changed nothing");
        assert!(furthest_apart(&both, &smoothed) > 0.2, "pushing a smooth changed nothing");
        // Half a push moves the surface less than a whole one, and the
        // other way with the sign turned.
        let (half, _) = with(Actions { push: 0.5, ..Actions::default() });
        let (back, _) = with(Actions { push: -1.0, ..Actions::default() });
        let moved = |volume: &Volume| furthest_apart(volume, &ball());
        assert!(moved(&half) > 0.1 && moved(&half) < moved(&pushed) * 0.8);
        let at = (CENTRE + stroke()[0] * RADIUS).round().as_ivec3();
        assert!(pushed.sample_voxel(at) < ball().sample_voxel(at), "a push did not add");
        assert!(back.sample_voxel(at) > ball().sample_voxel(at), "a negative push did not cut");
    }

    /// A slab with a ridge along it: the ground at z = 40, and a ridge
    /// drawn along x through y = 48.
    fn ridged() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.edit_voxels(
            glam::IVec3::new(0, 16, 20),
            glam::IVec3::new(96, 80, 60),
            |_, at, _| at.z - 40.0,
        );
        let draw = Compound {
            actions: Actions::DRAW,
            radius: 6.0,
            strength: 0.6,
            profile: Profile::default(),
            pattern: Pattern::default(),
        };
        along_the_ridge(&mut volume, &draw, 3);
        volume
    }

    /// A stroke along the ridge's line, `passes` times over.
    fn along_the_ridge(volume: &mut Volume, compound: &Compound, passes: usize) {
        let mut scratch = BrushScratch::new();
        for _ in 0..passes {
            for step in 0..40 {
                let at = Vec3::new(18.0 + step as f32 * 1.5, 48.0, 40.0);
                let stamp = Stamp::new(at, Vec3::Z, BrushDirection::Add).with_tangent(Vec3::X);
                compound.apply(volume, &stamp, None, &mut scratch);
            }
        }
    }

    /// How tall the ridge is at the middle of its length, and how wide at
    /// half that height, in voxels.
    fn ridge(volume: &Volume) -> (f32, f32) {
        let height = |y: f32| {
            let mut z = 56.0;
            while z > 30.0 && volume.sample_world(Vec3::new(48.0, y, z)) > 0.0 {
                z -= 0.02;
            }
            z - 40.0
        };
        let top = height(48.0);
        let wide = (0..240).filter(|step| height(36.0 + *step as f32 * 0.1) > top * 0.5).count();
        (top, wide as f32 * 0.1)
    }

    /// A gather is a pinch: a ridge under the stroke is narrower and as
    /// tall. Turned round it is wider and as tall. Upstream's pinch on the
    /// same ridge makes it broader, which is why there is a gather.
    #[test]
    fn a_gather_narrows_a_ridge_and_keeps_its_top() {
        let brush = |actions: Actions| Compound {
            actions,
            radius: 8.0,
            strength: 0.6,
            profile: Profile::default(),
            pattern: Pattern::default(),
        };
        let after = |actions: Actions| {
            let mut volume = ridged();
            along_the_ridge(&mut volume, &brush(actions), 2);
            ridge(&volume)
        };
        let (top, wide) = ridge(&ridged());
        assert!(top > 2.0 && wide > 5.0, "the fixture's ridge is {top} tall and {wide} wide");

        let (gathered_top, gathered) = after(Actions::GATHER);
        assert!(gathered < wide * 0.75, "gathered, the ridge is {gathered} wide of {wide}");
        assert!((gathered_top - top).abs() < top * 0.1, "and {gathered_top} tall of {top}");

        let (spread_top, spread) = after(Actions { gather: -1.0, ..Actions::default() });
        assert!(spread > wide * 1.2, "spread, the ridge is {spread} wide of {wide}");
        assert!((spread_top - top).abs() < top * 0.1, "and {spread_top} tall of {top}");

        // The invert key turns a gather that is all the brush does into a
        // spread, as it turns upstream's pinch round.
        let mut inverted = ridged();
        let mut scratch = BrushScratch::new();
        for _ in 0..2 {
            for step in 0..40 {
                let at = Vec3::new(18.0 + step as f32 * 1.5, 48.0, 40.0);
                let stamp = Stamp::new(at, Vec3::Z, BrushDirection::Subtract).with_tangent(Vec3::X);
                brush(Actions::GATHER).apply(&mut inverted, &stamp, None, &mut scratch);
            }
        }
        assert!((ridge(&inverted).1 - spread).abs() < 0.3, "{} and {spread}", ridge(&inverted).1);

        // Control: upstream's pinch on the same ridge is broader, which
        // is what there is a gather for. (Its opposite wore the ridge of
        // the brush pictures down to a quarter, `app/swatch.rs`, over
        // more passes than this; here, on a ridge the stroke runs dead
        // along, two passes leave its top, so that is not asked.)
        let (_, pinched) = after(Actions::PINCH);
        assert!(pinched > wide * 1.1, "upstream's pinch left the ridge {pinched} wide of {wide}");

        // And a gather on flat ground, with nothing to gather, moves
        // nothing: it only ever reads the field from beside where it is.
        let mut flat = Volume::new(1.0);
        flat.edit_voxels(glam::IVec3::new(0, 16, 20), glam::IVec3::new(96, 80, 60), |_, at, _| {
            at.z - 40.0
        });
        let before = flat.sample_world(Vec3::new(48.0, 50.0, 40.3));
        along_the_ridge(&mut flat, &brush(Actions::GATHER), 2);
        assert!((flat.sample_world(Vec3::new(48.0, 50.0, 40.3)) - before).abs() < 1e-4);
    }

    /// The six classic brushes are each their own family, and the two
    /// that are not brushes on the field have no actions.
    #[test]
    fn a_classic_brush_is_its_own_family() {
        for kind in BrushKind::ALL {
            match Actions::of(kind) {
                Some(actions) => {
                    assert!(actions.is_legal());
                    assert_eq!(actions.family(), kind, "{kind:?}");
                }
                None => assert!(matches!(kind, BrushKind::Move | BrushKind::Paint)),
            }
        }
        // And the ones that cost only their settings.
        assert_eq!(Actions::CREASE.family(), BrushKind::Draw);
        assert_eq!(Actions { pinch: -1.0, ..Actions::default() }.family(), BrushKind::Pinch);
        assert_eq!(Actions::GATHER.family(), BrushKind::Pinch);
        assert_eq!(Actions { gather: -1.0, ..Actions::default() }.family(), BrushKind::Pinch);
        // Every shape upstream has a curve for is that curve's shape.
        for shape in Shape::ALL {
            match shape.curve() {
                Some(curve) => assert_eq!(Shape::of(curve), shape),
                None => assert_eq!(shape, Shape::Round),
            }
        }
    }

    /// A scraper takes down what stands proud of its plane and fills
    /// nothing, where Flatten does both and Clay only fills.
    #[test]
    fn the_side_of_the_plane_decides_what_levelling_moves() {
        let levelled = |side: Side| {
            let compound = Compound {
                // A plane held a little under the top of the ball: the
                // middle of the brush stands proud of it and the rim of
                // the brush is under it, so there is something to cut and
                // something to fill.
                actions: Actions { level: 1.0, plane: -0.1, side, ..Actions::default() },
                radius: 12.0,
                strength: 1.0,
                profile: Profile::default(),
                pattern: Pattern::default(),
            };
            stroked(Masking::None, 1.0, BrushDirection::Add, |v, s, scratch| {
                compound.apply(v, s, None, scratch)
            })
            .0
        };
        let fresh = ball();
        let (mut raised, mut lowered) = ([0; 3], [0; 3]);
        for (index, side) in Side::ALL.into_iter().enumerate() {
            let after = levelled(side);
            for z in 10..90 {
                for y in 10..90 {
                    for x in 10..90 {
                        let voxel = glam::IVec3::new(x, y, z);
                        let change = after.sample_voxel(voxel) - fresh.sample_voxel(voxel);
                        // The field falls where material is added.
                        raised[index] += usize::from(change < -1e-3);
                        lowered[index] += usize::from(change > 1e-3);
                    }
                }
            }
        }
        let [both, fill, cut] = [0, 1, 2];
        assert!(
            lowered[both] > 300 && raised[both] > 300,
            "the fixture: levelling took off {} voxels and filled {}",
            lowered[both],
            raised[both]
        );
        assert_eq!(lowered[fill], 0, "a brush that only fills took material off");
        assert_eq!(raised[cut], 0, "a brush that only cuts added material");
        // About what flatten does of each, and not exactly: the stamps
        // overlap, and what one filled the next does not find to cut.
        assert!(lowered[cut] > 300, "a scraper took off {} voxels", lowered[cut]);
        assert!(raised[fill] > 300, "a filler filled {} voxels", raised[fill]);
    }

    /// A profile is its shape past its plateau: flat across the plateau,
    /// the shape's own curve from there to the rim, never rising outward,
    /// and the four shapes upstream has are upstream's curves with none.
    #[test]
    fn a_profile_is_flat_on_its_plateau_and_its_shape_beyond() {
        for curve in FalloffCurve::ALL {
            for step in 0..=100 {
                let distance = step as f32 / 80.0;
                assert_eq!(
                    Profile::of(curve).weight(distance).to_bits(),
                    curve.weight(distance).to_bits(),
                    "{curve} at {distance}"
                );
            }
        }
        for shape in Shape::ALL {
            for plateau in [0.0, 0.3, 0.6, 0.95, 4.0] {
                let profile = Profile { shape, plateau };
                let flat = plateau.min(MOST_PLATEAU);
                let mut last = 1.0;
                for step in 0..=200 {
                    let distance = step as f32 / 160.0;
                    let weight = profile.weight(distance);
                    assert!(weight <= last + 1e-6, "{shape:?} rises at {distance}");
                    if distance <= flat {
                        assert_eq!(weight, 1.0, "{shape:?} {plateau} at {distance}");
                    }
                    if distance >= 1.0 {
                        assert_eq!(weight, 0.0, "{shape:?} {plateau} at {distance}");
                    }
                    last = weight;
                }
            }
        }
        // A dome is a dome: at three fifths of the way out, four fifths
        // as high.
        let dome = Profile { shape: Shape::Round, plateau: 0.0 };
        assert!((dome.weight(0.6) - 0.8).abs() < 1e-6);
        assert!(dome.weight(0.9) > Profile::default().weight(0.9) * 4.0, "a dome has no shoulder");
    }

    /// An image across the tip is one more factor on the weight: nothing
    /// where the image is nothing, and laid out along the stroke.
    #[test]
    fn an_image_across_the_tip_shapes_what_a_stamp_does() {
        // A bar across the middle of the image: rows of full height from a
        // quarter to three quarters of the way down, nothing above or below.
        let size = 32;
        let heights: Vec<f32> = (0..size * size)
            .map(|texel| {
                let row = texel / size;
                if (size / 4..size * 3 / 4).contains(&row) { 1.0 } else { 0.0 }
            })
            .collect();
        let bar = Alpha::from_heights(size, size, &heights).expect("a bar");
        let compound = Compound {
            actions: Actions::DRAW,
            radius: 9.0,
            strength: 1.0,
            profile: Profile { shape: Shape::Wide, plateau: 0.9 },
            pattern: Pattern::default(),
        };
        let towards = Vec3::Y;
        let at = CENTRE + towards * RADIUS;
        let stamped = |tangent: Vec3, tip: Option<&Alpha>| {
            let mut volume = ball();
            let stamp = Stamp::new(at, towards, BrushDirection::Add).with_tangent(tangent);
            compound.apply(&mut volume, &stamp, tip, &mut BrushScratch::new());
            volume
        };
        let fresh = ball();
        let moved = |volume: &Volume, offset: Vec3| {
            let voxel = (at + offset).round().as_ivec3();
            (volume.sample_voxel(voxel) - fresh.sample_voxel(voxel)).abs()
        };
        // Travelling along x, "down the image" is along x: the bar lies
        // across the stroke, so the middle moves and the ends of the
        // stroke's own line do not.
        let along_x = stamped(Vec3::X, Some(&bar));
        assert!(moved(&along_x, Vec3::ZERO) > 0.3, "under the bar nothing moved");
        assert!(moved(&along_x, Vec3::Z * 6.0) > 0.3, "along the bar nothing moved");
        assert!(moved(&along_x, Vec3::X * 6.0) < 1e-6, "off the bar something moved");
        // Turned to travel along z, the bar turns with it.
        let along_z = stamped(Vec3::Z, Some(&bar));
        assert!(moved(&along_z, Vec3::X * 6.0) > 0.3, "the image did not turn with the stroke");
        assert!(moved(&along_z, Vec3::Z * 6.0) < 1e-6);
        // Control: with no image, all four places move.
        let plain = stamped(Vec3::X, None);
        assert!(moved(&plain, Vec3::X * 6.0) > 0.3 && moved(&plain, Vec3::Z * 6.0) > 0.3);
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn side() -> impl Strategy<Value = Side> {
            prop_oneof![Just(Side::Both), Just(Side::Fill), Just(Side::Cut)]
        }

        /// Amounts as a file or a slider might hand them over: mostly in
        /// range, sometimes well out of it.
        fn loose() -> impl Strategy<Value = Actions> {
            let amount = || -3.0_f32..3.0;
            let gather = prop_oneof![Just(0.0_f32), -3.0_f32..3.0];
            (amount(), amount(), amount(), amount(), amount(), amount(), side(), gather).prop_map(
                |(push, inflate, pinch, smooth, level, plane, side, gather)| Actions {
                    push,
                    inflate,
                    pinch,
                    gather,
                    smooth,
                    level,
                    plane,
                    side,
                },
            )
        }

        fn shape() -> impl Strategy<Value = Shape> {
            prop::sample::select(Shape::ALL.to_vec())
        }

        proptest! {
            /// Every brush has a family, and the family of anything that
            /// moves the surface one way has an opposite.
            #[test]
            fn every_brush_has_a_family(actions in loose()) {
                let actions = actions.legal();
                let family = actions.family();
                prop_assert!(Actions::of(family).is_some());
                let one_way = actions.push != 0.0
                    || actions.inflate != 0.0
                    || actions.pinch != 0.0
                    || actions.gather != 0.0
                    || (actions.level != 0.0
                        && (actions.side != Side::Both || actions.plane != 0.0));
                prop_assert_eq!(family.is_directional(), one_way);
            }

            /// Any amounts at all are brought to what a brush can be, and
            /// what already is one is left exactly as it was.
            #[test]
            fn amounts_are_made_legal_and_legal_ones_left_alone(actions in loose()) {
                let legal = actions.legal();
                prop_assert!(legal.is_legal(), "{:?}", legal);
                prop_assert_eq!(legal.legal(), legal);
                if actions.is_legal() {
                    prop_assert_eq!(legal, actions);
                }
                // The push and the inflation keep their signs and their
                // proportion: only their size is brought down.
                prop_assert!(legal.push * actions.push >= 0.0);
                prop_assert!(legal.inflate * actions.inflate >= 0.0);
            }

            /// A weight is a share, whatever the shape, the plateau or the
            /// distance, and is all of it at the middle.
            #[test]
            fn a_profile_gives_a_share(
                shape in shape(),
                plateau in -1.0_f32..2.0,
                distance in 0.0_f32..3.0,
            ) {
                let profile = Profile { shape, plateau };
                let weight = profile.weight(distance);
                prop_assert!((0.0..=1.0).contains(&weight), "{}", weight);
                prop_assert_eq!(profile.weight(0.0), 1.0);
            }
        }

        proptest! {
            // Each case stamps a ball: fewer of them.
            #![proptest_config(ProptestConfig::with_cases(32))]

            /// A brush whose family is a classic brush with no opposite
            /// has none itself: the invert key changes nothing it does.
            /// The application asks the family whether to invert at all,
            /// so a family that said "no opposite" of a brush that has one
            /// would be a brush that cannot be turned round.
            #[test]
            fn a_brush_of_a_family_with_no_opposite_has_none(
                smooth in 0.0_f32..=1.0,
                level in 0.0_f32..=1.0,
                radius in 4.0_f32..12.0,
            ) {
                let actions = Actions { smooth, level, ..Actions::default() };
                prop_assert!(!actions.family().is_directional());
                let compound = Compound {
                    actions,
                    radius,
                    strength: 0.6,
                    profile: Profile::default(),
                    pattern: Pattern::default(),
                };
                let towards = stroke()[0];
                let at = CENTRE + towards * RADIUS;
                let (mut added, mut taken) = (ball(), ball());
                for (volume, direction) in [
                    (&mut added, BrushDirection::Add),
                    (&mut taken, BrushDirection::Subtract),
                ] {
                    let stamp = Stamp::new(at, towards, direction).with_tangent(Vec3::Z);
                    compound.apply(volume, &stamp, None, &mut BrushScratch::new());
                }
                for z in 20..80 {
                    for y in 20..80 {
                        for x in 20..80 {
                            let voxel = glam::IVec3::new(x, y, z);
                            prop_assert_eq!(
                                added.sample_voxel(voxel).to_bits(),
                                taken.sample_voxel(voxel).to_bits()
                            );
                        }
                    }
                }
            }

            /// Whatever a brush is made of, a stamp of it leaves a field
            /// that is still a field (every value within the band), touches
            /// nothing outside its own radius, and touches nothing under a
            /// mask that protects everything.
            #[test]
            fn any_brush_leaves_a_field_and_stays_in_its_radius(
                actions in loose(),
                shape in shape(),
                plateau in 0.0_f32..0.95,
                radius in 3.0_f32..14.0,
                strength in 0.05_f32..=1.0,
                subtract in any::<bool>(),
            ) {
                let compound = Compound {
                    actions: actions.legal(),
                    radius,
                    strength,
                    profile: Profile { shape, plateau },
                    pattern: Pattern::default(),
                };
                let towards = stroke()[0];
                let at = CENTRE + towards * RADIUS;
                let direction =
                    if subtract { BrushDirection::Subtract } else { BrushDirection::Add };
                let stamp = Stamp::new(at, towards, direction).with_tangent(Vec3::Z);
                let fresh = ball();
                let mut volume = ball();
                for _ in 0..3 {
                    compound.apply(&mut volume, &stamp, None, &mut BrushScratch::new());
                }
                for z in 10..90 {
                    for y in 10..90 {
                        for x in 10..90 {
                            let voxel = glam::IVec3::new(x, y, z);
                            let value = volume.sample_voxel(voxel);
                            prop_assert!(
                                (INSIDE..=OUTSIDE).contains(&value),
                                "{} at {} is outside the band",
                                value,
                                voxel
                            );
                            if (voxel.as_vec3() - at).length() >= radius {
                                prop_assert_eq!(
                                    value.to_bits(),
                                    fresh.sample_voxel(voxel).to_bits(),
                                    "changed outside the brush at {}",
                                    voxel
                                );
                            }
                        }
                    }
                }

                let mut masked = ball();
                masked.edit_mask(
                    glam::IVec3::splat(0),
                    glam::IVec3::splat(100),
                    |_, _, _| PROTECTED,
                );
                masked.begin_stroke();
                compound.apply(&mut masked, &stamp, None, &mut BrushScratch::new());
                prop_assert!(masked.end_stroke().is_none(), "a stamp got under the mask");
            }
        }
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use crate::brush::engine::{BrushDef, FieldOp, Tip};

    /// What a stamp of the generic brush costs beside the classic kernel
    /// it stands in for.
    #[test]
    #[ignore = "a measurement, not a claim: run by hand, in release"]
    fn what_a_generic_stamp_costs_beside_a_classic_one() {
        let centre = Vec3::splat(48.0);
        for (op, actions) in [
            (FieldOp::Draw, Actions::DRAW),
            (FieldOp::Clay, Actions::CLAY),
            (FieldOp::Smooth, Actions::SMOOTH),
            (FieldOp::Inflate, Actions::INFLATE),
            (FieldOp::Pinch, Actions::PINCH),
            (FieldOp::Flatten, Actions::FLATTEN),
        ] {
            for radius in [8.0_f32, 24.0] {
                let stamp = Stamp::new(centre + Vec3::Y * 40.0, Vec3::Y, BrushDirection::Add)
                    .with_tangent(Vec3::X);
                let rounds = 200;
                let time = |run: &dyn Fn(&mut Volume, &mut BrushScratch)| {
                    let mut best = f64::MAX;
                    for _ in 0..5 {
                        let mut volume = Volume::new(1.0);
                        volume.seed_sphere(centre, 40.0);
                        let mut scratch = BrushScratch::new();
                        let started = std::time::Instant::now();
                        for _ in 0..rounds {
                            run(&mut volume, &mut scratch);
                        }
                        best =
                            best.min(started.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds));
                    }
                    best
                };
                let def = BrushDef { op, radius, strength: 0.3, tip: Tip::default() };
                let compound = Compound {
                    actions,
                    radius,
                    strength: 0.3,
                    profile: Profile::default(),
                    pattern: Pattern::default(),
                };
                let classic = time(&|volume, scratch| def.apply(volume, &stamp, scratch));
                let generic =
                    time(&|volume, scratch| compound.apply(volume, &stamp, None, scratch));
                eprintln!(
                    "{op:?} at {radius} voxels: classic {classic:.3} ms, generic {generic:.3} ms, \
                     {:+.0}%",
                    (generic / classic - 1.0) * 100.0
                );
            }
        }
    }
}
