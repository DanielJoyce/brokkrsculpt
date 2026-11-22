// SPDX-License-Identifier: AGPL-3.0-only

//! The brush engine: a brush as a composition rather than as a variant.
//!
//! [`super::Brush`] is one enum variant plus four numbers, and
//! [`super::Brush::stamp`] is one function with an arm per variant. That is
//! the right shape for eight brushes and the wrong one for eighty: a stamp
//! image, a gate on the geometry under the brush and a new operator would each
//! be another condition threaded through the same loop.
//!
//! So a brush here is parts. [`Tip`] is how the weight falls across a stamp,
//! [`FieldOp`] is what one weighted voxel does to the field, and
//! [`BrushDef`] is the two of them with a size and a strength. The parts this
//! module does not have yet -- gates, dynamics, stroke modes, the paint
//! channel -- each slot in at one named place below and are described in
//! `docs/STUDIO-PLAN.md`, section 6.
//!
//! # The old path is the oracle, and it is left exactly where it was
//!
//! Nothing in `brush.rs` was moved or rewritten to make this. The six stamping
//! brushes are implemented a second time here, and
//! `the_engine_matches_the_brush_it_was_built_from` drives both paths through
//! every kind, falloff, pattern, direction, pressure, mirror and mask it can
//! reach and requires the same field and the same undo entry, bit for bit.
//!
//! Two reasons for a second implementation over a refactor of the first, and
//! both are about this being a fork of a moving upstream:
//!
//! * `brush.rs` merges cleanly, because it is not touched beyond the one line
//!   that declares this module.
//! * When upstream changes what a brush does, the equivalence test fails and
//!   names the brush. The arithmetic here is then brought back into step
//!   deliberately, instead of a merge quietly keeping the old behaviour.
//!
//! The test compares against the live upstream code in the same process, so it
//! holds on every platform. A table of recorded hashes would not: the patterns
//! take roots and a hash of the result is a statement about one machine's
//! arithmetic.
//!
//! # What has to stay true of a new operator
//!
//! These are `brush.rs`'s rules, collected because every new [`Kernel`] is a
//! chance to break one:
//!
//! 1. The weight a kernel is handed is in `0..=1`. Everything upstream of the
//!    kernel multiplies; nothing adds.
//! 2. A kernel either blends toward a value the field can legally hold or
//!    reads the field from somewhere else. It never multiplies a difference
//!    from a local average, which has gain above one and grows a crust over a
//!    stroke.
//! 3. One stamp moves the surface by at most [`MAX_STAMP_VOXELS`].
//! 4. [`Kernel::reach_voxels`] bounds how far from the voxel being written the
//!    kernel reads. A read past the snapshot clamps to its rim without
//!    complaint, so an understated reach smears instead of failing.
//! 5. The kernel is chosen once per stamp. It is a type parameter of the loop,
//!    so the loop is compiled once per kernel and the voxel path has no
//!    dispatch in it.
//!
//! Move is not here. It is a gesture and not a stamp, and [`super::MoveStroke`]
//! already is the composition this module would give it. Paint is not here
//! either: it writes another channel, which arrives with that channel.

use glam::{IVec3, Vec3};

use super::{
    Brush, BrushDirection, BrushKind, BrushScratch, CLAY_OFFSET, FalloffCurve, MAX_STAMP_VOXELS,
    PINCH_PULL_VOXELS, STAMP_FRACTION_OF_RADIUS, Stamp, Symmetry, admit,
    blend_toward_plane_clamps_back,
};
use crate::brick::{INSIDE, OUTSIDE};
use crate::mask::PROTECTED;
use crate::pattern::{Pattern, Prepared};
use crate::region::FieldRegion;
use crate::volume::{BrickPreview, BrickVerdict, Volume};

/// What one weighted voxel does to the distance field.
///
/// The six stamping operations [`BrushKind`] has, under the names it gives
/// them. New operators are added here and given a [`Kernel`]; nothing else in
/// the engine has to learn about them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldOp {
    /// Slide the patch along the stroke normal by resampling from behind it.
    Draw,
    /// Blend toward a plane held just proud of the surface, adding only.
    Clay,
    /// Blend toward the mean of the six neighbours.
    Smooth,
    /// Offset the level set.
    Inflate,
    /// Resample from nearer the brush axis, squeezing a ridge into a crease.
    Pinch,
    /// Blend toward the tangent plane under the cursor.
    Flatten,
}

impl FieldOp {
    pub const ALL: [FieldOp; 6] = [
        FieldOp::Draw,
        FieldOp::Clay,
        FieldOp::Smooth,
        FieldOp::Inflate,
        FieldOp::Pinch,
        FieldOp::Flatten,
    ];

    /// The operator a legacy kind stamps with, or `None` for the two kinds
    /// that do not stamp the field: Move is a gesture and Paint writes a slot.
    pub fn from_kind(kind: BrushKind) -> Option<Self> {
        match kind {
            BrushKind::Draw => Some(FieldOp::Draw),
            BrushKind::Clay => Some(FieldOp::Clay),
            BrushKind::Smooth => Some(FieldOp::Smooth),
            BrushKind::Inflate => Some(FieldOp::Inflate),
            BrushKind::Pinch => Some(FieldOp::Pinch),
            BrushKind::Flatten => Some(FieldOp::Flatten),
            BrushKind::Move | BrushKind::Paint => None,
        }
    }
}

/// How the weight is distributed across one stamp.
///
/// A radial curve and a surface pattern today. A stamp image belongs here, as
/// one more factor on the same product.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tip {
    pub falloff: FalloffCurve,
    pub pattern: Pattern,
}

/// A brush as data: an operator, a tip, a size and a strength.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushDef {
    pub op: FieldOp,
    /// Radius of influence in world units.
    pub radius: f32,
    /// How hard one stamp bites. Per stamp and not per stroke, exactly as
    /// [`Brush::strength`] is.
    pub strength: f32,
    pub tip: Tip,
}

impl BrushDef {
    /// The engine brush that does what `brush` does, or `None` when `brush` is
    /// one of the two kinds that have no stamp. See [`FieldOp::from_kind`].
    pub fn from_legacy(brush: &Brush) -> Option<Self> {
        Some(Self {
            op: FieldOp::from_kind(brush.kind)?,
            radius: brush.radius,
            strength: brush.strength,
            tip: Tip { falloff: brush.falloff, pattern: brush.pattern },
        })
    }

    /// Apply one stamp, plus its mirrors when symmetry is on.
    ///
    /// The same shape as [`Brush::apply_symmetric`] without its Move arm, and
    /// `centre` means what it means there.
    pub fn apply_symmetric(
        &self,
        volume: &mut Volume,
        stamp: &Stamp,
        symmetry: Symmetry,
        centre: Vec3,
        scratch: &mut BrushScratch,
    ) {
        self.apply(volume, stamp, scratch);
        if symmetry.is_off() {
            return;
        }
        let mut twins = [*stamp; Symmetry::MAX_MIRRORS];
        let count = symmetry.mirrors(stamp, centre, &mut twins);
        for twin in &twins[..count] {
            self.apply(volume, twin, scratch);
        }
    }

    /// Apply one stamp.
    ///
    /// This is the only place the operator is looked at. Each arm is a
    /// separate instantiation of [`run`], so the choice costs one jump per
    /// stamp and nothing per voxel.
    pub fn apply(&self, volume: &mut Volume, stamp: &Stamp, scratch: &mut BrushScratch) {
        match self.op {
            FieldOp::Draw => run::<DrawKernel>(self, volume, stamp, scratch),
            FieldOp::Clay => run::<ClayKernel>(self, volume, stamp, scratch),
            FieldOp::Smooth => run::<SmoothKernel>(self, volume, stamp, scratch),
            FieldOp::Inflate => run::<InflateKernel>(self, volume, stamp, scratch),
            FieldOp::Pinch => run::<PinchKernel>(self, volume, stamp, scratch),
            FieldOp::Flatten => run::<FlattenKernel>(self, volume, stamp, scratch),
        }
    }
}

/// Everything about one stamp that does not vary from voxel to voxel.
///
/// Resolved once, before the loop, and handed to the kernel by reference. A
/// dynamics curve from pressure to radius would be resolved into this and the
/// loop would never know it had been there.
#[derive(Clone, Copy)]
struct Plan {
    voxel_size: f32,
    centre: Vec3,
    /// Outward surface normal the stroke is working along.
    normal: Vec3,
    direction: BrushDirection,
    /// Displacement at full weight, in voxels. Capped; see
    /// [`MAX_STAMP_VOXELS`].
    displacement: f32,
    /// A point on the reference plane of the kernels that have one.
    plane_point: Vec3,
}

/// What one weighted voxel does, as a type.
///
/// Every function is associated rather than a method, and every implementor
/// is an empty struct: a kernel has no state of its own, because everything a
/// stamp knows is in the [`Plan`]. That is what lets [`run`] be instantiated
/// per kernel with nothing passed around at run time.
trait Kernel {
    /// Whether [`Kernel::write`] reads `region` at all.
    ///
    /// A kernel that answers `false` is never given a fresh snapshot, and
    /// `region` holds whatever the last stamp that wanted one left in it.
    /// Reading it anyway samples stale data without failing; the saving is a
    /// straight copy of the read box, which `brush.rs` measured at 45% of
    /// Clay's cost.
    const READS_THE_FIELD: bool;

    /// Whether this kernel blends toward the stamp's reference plane, which
    /// lets a saturated brick far enough behind that plane be proven
    /// unchanged. See [`blend_toward_plane_clamps_back`].
    const BLENDS_TOWARD_A_PLANE: bool = false;

    /// How far from the voxel being written this kernel can read, in voxels,
    /// not counting the one voxel of padding every snapshot already has.
    fn reach_voxels(_gain: f32, _displacement: f32) -> f32 {
        0.0
    }

    /// A point on the reference plane, for the kernels that blend toward one.
    fn plane_point(stamp: &Stamp, _radius: f32) -> Vec3 {
        stamp.centre
    }

    /// Whether a region holding the saturated `value` everywhere, with
    /// everything the kernel can read around it holding the same, comes out
    /// unchanged.
    ///
    /// Only ever asked about [`INSIDE`] and [`OUTSIDE`]. A constant part way
    /// through the band is a flat patch of real surface, and [`run`] does not
    /// ask about those.
    fn leaves_saturated_alone(value: f32, direction: BrushDirection) -> bool;

    /// The new value of one voxel.
    ///
    /// `patterned` is the weight after the falloff, the strength, the
    /// pressure and the pattern, and `free` is how much of the edit the mask
    /// lets through. They arrive apart because the two plane kernels cannot
    /// fold the mask into the weight; see [`admit`]. Both are above zero.
    fn write(
        plan: &Plan,
        region: &FieldRegion,
        voxel: IVec3,
        position: Vec3,
        value: f32,
        patterned: f32,
        free: f32,
    ) -> f32;
}

/// One stamp of `def` with kernel `K`.
///
/// The structure is [`Brush::stamp`]'s, step for step, and the arithmetic is
/// its arithmetic in its order. That is deliberate and is tested: two
/// expressions that are equal on paper are not equal in binary floating
/// point, and "bit for bit" is the only version of "the same" that a test can
/// hold a refactor to.
fn run<K: Kernel>(def: &BrushDef, volume: &mut Volume, stamp: &Stamp, scratch: &mut BrushScratch) {
    if def.radius <= 0.0 || def.strength <= 0.0 || stamp.pressure <= 0.0 {
        return;
    }

    let voxel_size = volume.voxel_size();
    let radius = def.radius;
    let inverse_radius = 1.0 / radius;
    let gain = def.strength * stamp.pressure;
    let displacement = (radius / voxel_size * STAMP_FRACTION_OF_RADIUS).min(MAX_STAMP_VOXELS);

    // Two boxes: the one written is the brush's own reach, the one read is
    // that grown by however far the kernel says it resamples from.
    let reach = K::reach_voxels(gain, displacement) * voxel_size;
    let extent = Vec3::splat(radius);
    let (lo, hi) = volume.voxel_bounds(stamp.centre - extent, stamp.centre + extent);
    let read_extent = extent + Vec3::splat(reach);
    let (read_lo, read_hi) =
        volume.voxel_bounds(stamp.centre - read_extent, stamp.centre + read_extent);

    if K::READS_THE_FIELD {
        volume.snapshot(read_lo, read_hi, &mut scratch.region);
    }
    let region = &scratch.region;

    let plan = Plan {
        voxel_size,
        centre: stamp.centre,
        normal: stamp.normal,
        direction: stamp.direction,
        displacement,
        plane_point: K::plane_point(stamp, radius),
    };

    let pattern = if def.tip.pattern.is_off() {
        Prepared::OFF
    } else {
        def.tip.pattern.prepare(voxel_size, stamp.normal, stamp.tangent)
    };
    let falloff = def.tip.falloff;
    let centre = stamp.centre;
    let direction = stamp.direction;

    // The trilinear tap lands one voxel past wherever the kernel reads, and
    // the kernels that warp nothing still read their six axis neighbours.
    let read_voxels = (reach / voxel_size).ceil() as i32 + 1;

    // Which bricks of the box can change at all. The reasoning for each arm is
    // in `Brush::stamp`, which this follows verdict for verdict: a different
    // verdict is a different undo entry even where the field agrees.
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
                if (value == INSIDE || value == OUTSIDE)
                    && K::leaves_saturated_alone(value, direction) =>
            {
                BrickVerdict::OnlyNearDifferentNeighbours
            }
            Some(value)
                if K::BLENDS_TOWARD_A_PLANE
                    && blend_toward_plane_clamps_back(
                        value,
                        box_min,
                        box_max,
                        plan.plane_point,
                        plan.normal,
                        voxel_size,
                    ) =>
            {
                BrickVerdict::Skip
            }
            _ => BrickVerdict::Whole,
        }
    };

    let radius_squared = radius * radius;

    volume.edit_voxels_where(
        lo,
        hi,
        read_voxels,
        true,
        decide,
        move |voxel, position, value, free| {
            let offset = position - centre;
            if offset.length_squared() >= radius_squared {
                return value;
            }
            let distance = offset.length() * inverse_radius;
            let shaped = falloff.weight(distance) * gain;
            if shaped <= 0.0 {
                return value;
            }
            // The tip's factors, in the order `Brush::stamp` multiplies them.
            // A stamp image is one more factor here; a gate on the geometry is
            // one more after it. Each stays in 0..=1, so the product does.
            let patterned = shaped * pattern.weight(position);
            if patterned <= 0.0 || free <= 0.0 {
                return value;
            }
            K::write(&plan, region, voxel, position, value, patterned, free)
        },
    );
}

struct InflateKernel;

impl Kernel for InflateKernel {
    const READS_THE_FIELD: bool = false;

    /// A constant offset lands back on the clamp it started at, but only in
    /// the direction the clamp is already holding.
    fn leaves_saturated_alone(value: f32, direction: BrushDirection) -> bool {
        match direction {
            BrushDirection::Add => value == INSIDE,
            BrushDirection::Subtract => value == OUTSIDE,
        }
    }

    #[inline(always)]
    fn write(
        plan: &Plan,
        _region: &FieldRegion,
        _voxel: IVec3,
        _position: Vec3,
        value: f32,
        patterned: f32,
        free: f32,
    ) -> f32 {
        let weight = patterned * free;
        value + plan.direction.field_sign() * weight * plan.displacement
    }
}

struct DrawKernel;

impl Kernel for DrawKernel {
    const READS_THE_FIELD: bool = true;

    fn reach_voxels(gain: f32, displacement: f32) -> f32 {
        gain * displacement
    }

    /// Resampling a constant gives the constant back, wherever it reads from.
    fn leaves_saturated_alone(_value: f32, _direction: BrushDirection) -> bool {
        true
    }

    #[inline(always)]
    fn write(
        plan: &Plan,
        region: &FieldRegion,
        _voxel: IVec3,
        position: Vec3,
        _value: f32,
        patterned: f32,
        free: f32,
    ) -> f32 {
        let weight = patterned * free;
        let shift = plan.normal
            * (weight * plan.displacement * plan.voxel_size * plan.direction.outward_sign());
        region.sample((position - shift) / plan.voxel_size)
    }
}

struct SmoothKernel;

impl Kernel for SmoothKernel {
    const READS_THE_FIELD: bool = true;

    /// The mean of six equal values is that value.
    fn leaves_saturated_alone(_value: f32, _direction: BrushDirection) -> bool {
        true
    }

    #[inline(always)]
    fn write(
        _plan: &Plan,
        region: &FieldRegion,
        voxel: IVec3,
        _position: Vec3,
        value: f32,
        patterned: f32,
        free: f32,
    ) -> f32 {
        let weight = patterned * free;
        let average = region.neighbour_average(voxel);
        value + (average - value) * weight
    }
}

struct PinchKernel;

impl Kernel for PinchKernel {
    const READS_THE_FIELD: bool = true;

    fn reach_voxels(gain: f32, _displacement: f32) -> f32 {
        gain * PINCH_PULL_VOXELS
    }

    fn leaves_saturated_alone(_value: f32, _direction: BrushDirection) -> bool {
        true
    }

    #[inline(always)]
    fn write(
        plan: &Plan,
        region: &FieldRegion,
        _voxel: IVec3,
        position: Vec3,
        value: f32,
        patterned: f32,
        free: f32,
    ) -> f32 {
        let weight = patterned * free;
        let to_centre = plan.centre - position;
        // Along the surface only, or pinch quietly doubles as inflate.
        let lateral = to_centre - plan.normal * to_centre.dot(plan.normal);
        let Some(direction_to_axis) = lateral.try_normalize() else {
            return value;
        };
        let spread = match plan.direction {
            BrushDirection::Add => 1.0,
            BrushDirection::Subtract => -1.0,
        };
        let pull = direction_to_axis * (weight * PINCH_PULL_VOXELS * plan.voxel_size * spread);
        region.sample((position + pull) / plan.voxel_size)
    }
}

struct FlattenKernel;

impl Kernel for FlattenKernel {
    const READS_THE_FIELD: bool = false;
    const BLENDS_TOWARD_A_PLANE: bool = true;

    /// The target varies across a brick, so no claim about a constant alone
    /// can be made. The plane test in [`run`] makes it with the box in hand.
    fn leaves_saturated_alone(_value: f32, _direction: BrushDirection) -> bool {
        false
    }

    #[inline(always)]
    fn write(
        plan: &Plan,
        _region: &FieldRegion,
        _voxel: IVec3,
        position: Vec3,
        value: f32,
        patterned: f32,
        free: f32,
    ) -> f32 {
        let plane = (position - plan.plane_point).dot(plan.normal) / plan.voxel_size;
        let target = (value + (plane - value) * patterned).clamp(INSIDE, OUTSIDE);
        admit(value, target, free)
    }
}

struct ClayKernel;

impl Kernel for ClayKernel {
    const READS_THE_FIELD: bool = false;
    const BLENDS_TOWARD_A_PLANE: bool = true;

    /// Held just outside the surface, so material builds up to it.
    fn plane_point(stamp: &Stamp, radius: f32) -> Vec3 {
        stamp.centre + stamp.normal * (radius * CLAY_OFFSET * stamp.direction.outward_sign())
    }

    fn leaves_saturated_alone(_value: f32, _direction: BrushDirection) -> bool {
        false
    }

    #[inline(always)]
    fn write(
        plan: &Plan,
        _region: &FieldRegion,
        _voxel: IVec3,
        position: Vec3,
        value: f32,
        patterned: f32,
        free: f32,
    ) -> f32 {
        let plane = (position - plan.plane_point).dot(plan.normal) / plan.voxel_size;
        let blended = value + (plane - value) * patterned;
        // Only the half of the blend that moves material the way that was
        // asked for, taken before the mask admits a fraction of it.
        let kept = match plan.direction {
            BrushDirection::Add => blended.min(value),
            BrushDirection::Subtract => blended.max(value),
        };
        admit(value, kept.clamp(INSIDE, OUTSIDE), free)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{MaskOp, MirrorAxis};
    use crate::pattern::PatternKind;
    use crate::testing::assert_same_field;

    const CENTRE: Vec3 = Vec3::splat(48.0);
    const RADIUS: f32 = 30.0;

    /// A ball big enough to have solid interior tiles under the brush and far
    /// enough from the origin to straddle brick boundaries on every axis.
    ///
    /// A ball and not a slab: a slab's field is linear, so smooth's average
    /// and flatten's plane hand every voxel its own value back and two wrong
    /// implementations of either would agree with each other perfectly.
    fn ball() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(CENTRE, RADIUS);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// Where a short stroke lands, as directions from the ball's centre.
    ///
    /// The stamps overlap, so a difference in one compounds into the next and
    /// a kernel that is nearly right does not get to hide in a single stamp.
    /// The first is off every axis so its box crosses brick corners.
    fn stroke() -> [Vec3; 4] {
        [
            Vec3::new(0.6, 0.8, 0.0),
            Vec3::new(0.55, 0.8, 0.2),
            Vec3::new(0.5, 0.8, 0.35),
            Vec3::new(0.4, 0.8, 0.45),
        ]
        .map(Vec3::normalize)
    }

    /// One way of preparing the mask before the stroke.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Masking {
        /// Never touched, which is the path every unmasked model takes.
        None,
        /// A soft stroke of protection over part of the stroke's path, so the
        /// brush meets every value between free and protected.
        Partial,
        /// The same, read inverted: most of the body protected and a soft
        /// window open. Polarity is resolved inside the edit, and a brush that
        /// read the stored byte would get this one backwards.
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

    /// The stamping kinds, which are the ones with an engine brush.
    fn stamping_kinds() -> impl Iterator<Item = BrushKind> {
        BrushKind::ALL.into_iter().filter(|kind| FieldOp::from_kind(*kind).is_some())
    }

    #[test]
    fn every_kind_but_move_and_paint_has_an_operator() {
        let with: Vec<_> = stamping_kinds().collect();
        assert_eq!(with.len(), FieldOp::ALL.len());
        assert_eq!(FieldOp::from_kind(BrushKind::Move), None);
        assert_eq!(FieldOp::from_kind(BrushKind::Paint), None);
        for op in FieldOp::ALL {
            assert!(
                with.iter().any(|kind| FieldOp::from_kind(*kind) == Some(op)),
                "{op:?} has no legacy kind, so nothing checks it against one"
            );
        }
    }

    #[test]
    fn the_engine_matches_the_brush_it_was_built_from() {
        // Bit for bit: the field, how each brick is stored, and the undo
        // entry. Every axis of the matrix is here because each one reaches
        // arithmetic the others do not -- the pattern is a factor on the
        // weight, the pressure a factor on the gain, the direction a sign in
        // four of the six kernels, the mask a second argument to all of them
        // and a different code path in two.
        let mut compared = 0;
        let mut changed = 0;
        for kind in stamping_kinds() {
            for falloff in FalloffCurve::ALL {
                for pattern in PatternKind::ALL {
                    for direction in [BrushDirection::Add, BrushDirection::Subtract] {
                        for pressure in [1.0, 0.4] {
                            for masking in
                                [Masking::None, Masking::Partial, Masking::PartialInverted]
                            {
                                let brush = Brush {
                                    kind,
                                    radius: 9.0,
                                    strength: 0.6,
                                    falloff,
                                    pattern: Pattern { kind: pattern, scale_mm: 5.0, depth: 0.8 },
                                };
                                let def = BrushDef::from_legacy(&brush).expect("a stamping kind");
                                let what = format!(
                                    "{kind} {falloff} {pattern:?} {direction:?} \
                                     pressure {pressure} {masking:?}"
                                );

                                let (old, old_undo) =
                                    stroked(masking, pressure, direction, |v, s, scratch| {
                                        brush.apply(v, s, scratch)
                                    });
                                let (new, new_undo) =
                                    stroked(masking, pressure, direction, |v, s, scratch| {
                                        def.apply(v, s, scratch)
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

        // The control. Two paths that both did nothing agree perfectly, so
        // the comparison is only worth something if the strokes moved the
        // field. Smooth and flatten have no inverse and still change a ball,
        // so every combination should have recorded an undo entry.
        assert_eq!(compared, 6 * 4 * 6 * 2 * 2 * 3);
        assert_eq!(
            changed, compared,
            "some strokes changed nothing, so their agreement proves nothing"
        );
    }

    #[test]
    fn the_engine_matches_under_every_mirror() {
        // The mirrors are the same stamp reflected, so this is cheap to be
        // sure of and worth being sure of: a twin has its normal and tangent
        // negated on an axis, which is input no unmirrored stamp produces.
        let symmetries = [
            Symmetry::OFF.with_axis(MirrorAxis::X, true),
            Symmetry::OFF.with_axis(MirrorAxis::Y, true).with_axis(MirrorAxis::Z, true),
            Symmetry::OFF
                .with_axis(MirrorAxis::X, true)
                .with_axis(MirrorAxis::Y, true)
                .with_axis(MirrorAxis::Z, true),
        ];
        for kind in stamping_kinds() {
            for symmetry in symmetries {
                for direction in [BrushDirection::Add, BrushDirection::Subtract] {
                    let brush = Brush {
                        kind,
                        radius: 9.0,
                        strength: 0.6,
                        pattern: Pattern { kind: PatternKind::Hair, scale_mm: 5.0, depth: 0.8 },
                        ..Brush::default()
                    };
                    let def = BrushDef::from_legacy(&brush).expect("a stamping kind");
                    let what = format!("{kind} {direction:?} mirrored {}", symmetry.label());

                    let (old, old_undo) =
                        stroked(Masking::Partial, 1.0, direction, |v, s, scratch| {
                            brush.apply_symmetric(v, s, symmetry, CENTRE, scratch)
                        });
                    let (new, new_undo) =
                        stroked(Masking::Partial, 1.0, direction, |v, s, scratch| {
                            def.apply_symmetric(v, s, symmetry, CENTRE, scratch)
                        });

                    assert_same_field(&old, &new, &what);
                    assert_eq!(old_undo, new_undo, "{what}: the undo entry differs");
                    assert!(old_undo.is_some(), "{what}: the stroke changed nothing");
                }
            }
        }
    }

    #[test]
    fn the_engine_matches_at_a_radius_that_reaches_the_tiles() {
        // The strokes above use a 9 voxel brush, which never gets a whole
        // brick of solid interior under it. This one does, and that is the
        // only place the verdicts that skip a saturated brick are exercised:
        // a wrong `leaves_saturated_alone` shows up here as a tile made dense
        // on one path and left alone on the other, which `assert_same_field`
        // reports as a difference in storage and the undo entry as a
        // difference in bytes.
        //
        // The plan is compared as well as the result, stamp by stamp, because
        // one mistake leaves no other trace. A kernel that understates
        // `reach_voxels` resolves a thinner rim of each tile, and at these
        // strengths the voxels it wrongly leaves out are ones the stamp would
        // have handed their own value back to: the field agrees, the undo
        // entry agrees, and the brush is one stronger setting away from
        // smearing. Mutating Draw's reach to zero was caught by nothing until
        // this comparison was added.
        for kind in stamping_kinds() {
            for direction in [BrushDirection::Add, BrushDirection::Subtract] {
                let brush = Brush { kind, radius: 44.0, strength: 0.5, ..Brush::default() };
                let def = BrushDef::from_legacy(&brush).expect("a stamping kind");
                let what = format!("{kind} {direction:?} at a large radius");

                let run = |with: &dyn Fn(&mut Volume, &Stamp, &mut BrushScratch)| {
                    let mut volume = ball();
                    volume.begin_stroke();
                    let mut scratch = BrushScratch::new();
                    let mut plans = Vec::new();
                    for stand_off in [0.0, -8.0, 8.0] {
                        let at = CENTRE + Vec3::X * (RADIUS + stand_off);
                        with(&mut volume, &Stamp::new(at, Vec3::X, direction), &mut scratch);
                        plans.push(volume.last_plan());
                    }
                    let undo = volume.end_stroke().map(|edit| (edit.len(), edit.bytes()));
                    (volume, undo, plans)
                };

                let (old, old_undo, old_plans) = run(&|v, s, scratch| brush.apply(v, s, scratch));
                let (new, new_undo, new_plans) = run(&|v, s, scratch| def.apply(v, s, scratch));

                assert_same_field(&old, &new, &what);
                assert_eq!(old_undo, new_undo, "{what}: the undo entry differs");
                assert_eq!(old_plans, new_plans, "{what}: the two paths planned differently");
            }
        }
    }

    #[test]
    fn a_brush_that_cannot_bite_does_nothing() {
        // The three early returns, each of which would otherwise divide by
        // zero or stamp at no strength and record an empty undo entry.
        let stamp = Stamp::new(CENTRE + Vec3::X * RADIUS, Vec3::X, BrushDirection::Add);
        let able = BrushDef { op: FieldOp::Draw, radius: 9.0, strength: 0.6, tip: Tip::default() };
        let cases = [
            (BrushDef { radius: 0.0, ..able }, stamp, "no radius"),
            (BrushDef { strength: 0.0, ..able }, stamp, "no strength"),
            (able, stamp.with_pressure(0.0), "no pressure"),
        ];
        for (def, stamp, what) in cases {
            let mut volume = ball();
            volume.begin_stroke();
            def.apply(&mut volume, &stamp, &mut BrushScratch::new());
            assert!(volume.end_stroke().is_none(), "{what} still recorded an edit");
            assert_same_field(&volume, &ball(), what);
        }

        // And the control: the brush those were cut down from does bite.
        let mut volume = ball();
        volume.begin_stroke();
        able.apply(&mut volume, &stamp, &mut BrushScratch::new());
        assert!(volume.end_stroke().is_some(), "the able brush changed nothing either");
    }
}
