// SPDX-License-Identifier: AGPL-3.0-only

//! How a paint stroke touches the surface: all of what is under the brush,
//! or only what a nearly dry brush or a sponge would catch.
//!
//! [`crate::brush::Brush::apply_paint`] writes a paint number into every
//! voxel under the brush. That is a loaded brush, and it is the right tool
//! for a base coat and for detail. A miniature painter has two more ways of
//! putting a brush to a model, and both of them are about what the brush does
//! NOT touch:
//!
//! * **Drybrush.** Most of the paint is wiped off first, and the brush is
//!   dragged lightly across the model. Paint comes off only on what stands
//!   up into the bristles: raised edges, and more on the side of an edge the
//!   brush is coming at. It comes off in streaks, because a dry brush is
//!   separate bristles and not a wet point, and it runs out.
//! * **Sponge.** A torn piece of foam is dabbed on. It leaves a speckle, and
//!   a bit more of it on edges, which is how chipped paint is done.
//!
//! Both are the plain paint stamp with one more question asked of every
//! voxel: does this brush catch here. That question is a [`Touch`], and
//! `docs/STUDIO-PLAN.md` decision D4 is that it is asked by a gate on the
//! brush and is not a tool of its own.
//!
//! # Why this suits a paint number so well
//!
//! A voxel holds one paint or another and nothing between
//! ([`crate::paint`]). A soft fade is the one thing that cannot hold. A
//! drybrush and a sponge are the opposite of a soft fade: every bristle mark
//! and every speck is paint or no paint. So the two techniques that are
//! hardest to fake in an RGB painting program, where they are textures
//! stamped onto a picture, are here exactly what the storage is good at, and
//! they are true to the shape under them because the shape is asked.
//!
//! # A gate has to give the same answer twice
//!
//! A stroke is hundreds of overlapping stamps, and a voxel under a stroke is
//! under eight or so of them. If each stamp rolled its own dice, a voxel
//! would be painted if ANY of the eight said yes, and a sponge set to cover a
//! tenth of the surface would cover more than half of it. So nothing here is
//! random per stamp. Whether a voxel is caught is a function of where it is,
//! of the shape of the surface there, and of a seed that is fixed for the
//! whole stroke. Two stamps over one voxel agree, and painting over the same
//! place twice within a stroke changes nothing. A second STROKE has a second
//! seed and adds to the first, as a second pass with a real sponge does.
//!
//! The one thing that does differ between the stamps of a stroke is the
//! direction of travel, which a drybrush reads, and the load, which runs
//! down. Both only ever lose paint along the stroke, so a voxel caught early
//! stays caught.
//!
//! # Where on the model the question is asked
//!
//! Paint is written through the whole narrow band and not only at the
//! surface, so that it is still there after the surface has been sculpted a
//! voxel deeper. The gate is therefore asked at the point of the surface
//! nearest each voxel, found by one step down the gradient, and every voxel
//! along a line through the band gets the answer its surface point gets.
//! Without that, the speckle at depth would be a different speckle, and the
//! first smoothing stroke would shuffle it.

use contracts::debug_ensures;
use glam::{IVec3, Vec3};

use crate::brick::NARROW_BAND;
use crate::brush::{Brush, Flip, Stamp, Symmetry};
use crate::mask::PROTECTED;
use crate::pattern::Prepared;
use crate::region::FieldRegion;
use crate::volume::Volume;

/// A voxel at least this protected is left alone by a paint stamp, on
/// upstream's own line: `brush.rs` has the same number, privately, and
/// `a_gated_stamp_respects_the_mask_and_the_pattern_as_a_plain_one_does`
/// holds this copy to it by comparing what the two stamps write.
const PAINT_PROTECTED: u8 = PROTECTED / 2 + 1;

/// A pattern weight below this stencils the paint out. Also upstream's line,
/// held by the same test.
const PAINT_STENCIL: f32 = 0.5;

/// The least a drybrush has to be asked for before it leaves anything. With
/// this above zero a flat, which is raised by nothing, can never be caught
/// however the sliders are set. A brush that paints flats is the loaded
/// brush, and there is a button for it.
const DRY_FLOOR: f32 = 0.02;

/// How much an edge facing away from the stroke catches, against one facing
/// into it. Half: the far side of a ridge still gets some, which is what a
/// real pass leaves and why a painter goes over a model from two directions.
const LEE_SHARE: f32 = 0.5;

/// How far past just caught a drybrush has to be to lay a full touch, in
/// the units `raised` is in: a quarter of the way from a flat to a full
/// edge.
const DRY_RAMP: f32 = 0.25;

/// How wide the soft rim of a sponge's speck is, as a share of the noise's
/// range. The noise crosses about this much in a voxel at a grain of two
/// voxels, so the rim is about a voxel wide there and thinner, in voxels,
/// on a coarser grain.
const SPECK_RIM: f32 = 0.25;

/// How much longer than it is wide one bristle mark is. Value noise is
/// sampled this much more slowly along the stroke than across it.
const STREAK_LENGTH: f32 = 12.0;

/// A nearly dry brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drybrush {
    /// How lightly the brush has to touch to leave paint, 0..=1. At 0.2 only
    /// the sharpest edges are caught. At 0.9 every raised form is.
    pub catch: f32,
    /// How broken the marks are, 0..=1. At 0 the paint goes on wherever the
    /// shape says. At 1 it goes on in separate bristle lines.
    pub streak: f32,
    /// How wide one bristle's mark is, in millimetres.
    pub bristle_mm: f32,
}

impl Default for Drybrush {
    fn default() -> Self {
        Self { catch: 0.6, streak: 0.5, bristle_mm: 0.4 }
    }
}

/// A sponge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sponge {
    /// How much of a flat is covered, 0..=1, as a share of its area.
    pub density: f32,
    /// How big one speck is, in millimetres.
    pub grain_mm: f32,
    /// How much more lands on raised edges, 0..=1. At 1 a full edge is
    /// covered whatever the density.
    pub edges: f32,
}

impl Default for Sponge {
    fn default() -> Self {
        Self { density: 0.35, grain_mm: 0.6, edges: 0.5 }
    }
}

/// What a paint stroke catches.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Touch {
    /// Everything under the brush.
    #[default]
    Loaded,
    Drybrush(Drybrush),
    Sponge(Sponge),
}

/// One stroke's worth of touch: the kind, and the things that are fixed from
/// the press to the release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coat {
    pub touch: Touch,
    /// Fixed for a stroke, different between strokes. See the module
    /// documentation for why it is not per stamp.
    pub seed: u32,
    /// Where the stroke began, in world space. The bristle lines are counted
    /// from here, so that they turn about the stroke and not about the
    /// origin of the world, which may be a long way off.
    pub origin: Vec3,
    /// How much paint is left on the brush, 1 down to 0. The caller runs it
    /// down along the stroke. A sponge does not read it.
    pub load: f32,
    /// How many units of the crease signal a full edge is. The view's gain
    /// ([`crate::signals`]), so that what the brush catches is what
    /// "show creases" lights.
    pub crease_gain: f32,
}

impl Coat {
    /// A loaded brush, which reads none of the other fields.
    pub const LOADED: Coat =
        Coat { touch: Touch::Loaded, seed: 0, origin: Vec3::ZERO, load: 1.0, crease_gain: 1.0 };

    /// Write `slot` into the voxels of one stamp that this touch catches.
    ///
    /// A loaded brush is [`Brush::apply_paint`] itself, not a copy of it, so
    /// the plain path stays upstream's to the bit. A loaded brush does not
    /// touch the scratch.
    pub fn apply(
        &self,
        brush: &Brush,
        volume: &mut Volume,
        stamp: &Stamp,
        slot: u8,
        scratch: &mut TouchScratch,
    ) {
        self.apply_flipped(brush, volume, stamp, stamp, Flip::IDENTITY, slot, scratch);
    }

    /// One stamp and its mirrors.
    ///
    /// A mirrored twin asks the gate at the point its voxel is the mirror
    /// image OF, so the speckle on one side of the plane is the reflection
    /// of the speckle on the other, as the stamp's shape already is.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_symmetric(
        &self,
        brush: &Brush,
        volume: &mut Volume,
        stamp: &Stamp,
        slot: u8,
        symmetry: Symmetry,
        centre: Vec3,
        scratch: &mut TouchScratch,
    ) {
        self.apply(brush, volume, stamp, slot, scratch);
        if symmetry.is_off() {
            return;
        }
        let mut twins = [*stamp; Symmetry::MAX_MIRRORS];
        let mut flips = [Flip::IDENTITY; Symmetry::MAX_MIRRORS];
        let count = symmetry.mirrors(stamp, centre, &mut twins);
        let flipped = symmetry.flips(centre, &mut flips);
        debug_assert_eq!(count, flipped, "a stamp has one twin a flip");
        for (twin, flip) in twins[..count].iter().zip(&flips[..count]) {
            self.apply_flipped(brush, volume, twin, stamp, *flip, slot, scratch);
        }
    }

    /// `stamp` is where the paint goes. `source` and `flip` say which stamp
    /// it is the mirror image of: the gate's pattern is laid out in the
    /// source's frame and read through the flip.
    #[allow(clippy::too_many_arguments)]
    fn apply_flipped(
        &self,
        brush: &Brush,
        volume: &mut Volume,
        stamp: &Stamp,
        source: &Stamp,
        flip: Flip,
        slot: u8,
        scratch: &mut TouchScratch,
    ) {
        let gate = match self.touch {
            Touch::Loaded => {
                let mut unused = crate::brush::BrushScratch::default();
                brush.apply_paint(volume, stamp, slot, &mut unused);
                return;
            }
            Touch::Drybrush(dry) => Gate::Dry(dry),
            Touch::Sponge(sponge) => Gate::Sponge(sponge),
        };
        if brush.radius <= 0.0 {
            return;
        }

        let voxel_size = volume.voxel_size();
        let extent = Vec3::splat(brush.radius);
        let (lo, hi) = volume.voxel_bounds(stamp.centre - extent, stamp.centre + extent);
        // The surface point a voxel answers for is up to a band away, and
        // the curvature there is read from a block reaching two further.
        let reach = IVec3::splat(NARROW_BAND.ceil() as i32 + 3);
        volume.snapshot(lo - reach, hi + reach, &mut scratch.region);
        let region = &scratch.region;
        let curvatures = Curvatures::over(region, &mut scratch.cavity);

        let pattern = if brush.pattern.is_off() {
            Prepared::OFF
        } else {
            brush.pattern.prepare(voxel_size, stamp.normal, stamp.tangent)
        };
        let frame = Frame::of(source, self.origin);
        let centre = stamp.centre;
        let radius_squared = brush.radius * brush.radius;
        // Which way the brush is travelling over THIS stamp, for the lee
        // side. A stroke that has not moved yet has no lee side.
        let travel = stamp.tangent.normalize_or_zero();
        let coat = *self;

        volume.edit_colour(lo, hi, |voxel, position, held, protection| {
            if (position - centre).length_squared() >= radius_squared {
                return held;
            }
            if protection >= PAINT_PROTECTED {
                return held;
            }
            if pattern.weight(position) < PAINT_STENCIL {
                return held;
            }

            // One step down the gradient to the surface this voxel is
            // paint for.
            let distance = region.get(voxel);
            let gradient = gradient_at(region, voxel);
            let normal = gradient.normalize_or_zero();
            let on_surface = voxel.as_vec3() - normal * distance;
            let raised = curvatures.raised(on_surface, coat.crease_gain);

            // The gate's pattern is read where this point is the image of.
            let world = position - normal * (distance * voxel_size);
            let in_source = (world - flip.offset) * flip.sign;
            let caught = match gate {
                Gate::Dry(dry) => {
                    let facing = 0.5 - 0.5 * normal.dot(travel);
                    dry.catches(raised, facing, frame.bristle(in_source, dry, coat.seed), coat.load)
                }
                Gate::Sponge(sponge) => {
                    sponge.catches(raised, Frame::speck(in_source, sponge, coat.seed))
                }
            };
            if caught { slot } else { held }
        });
    }
}

#[derive(Clone, Copy)]
enum Gate {
    Dry(Drybrush),
    Sponge(Sponge),
}

impl Drybrush {
    /// Whether a point is caught.
    ///
    /// `raised` is how far the surface stands up there, 0 on a flat to 1 on
    /// a full edge. `facing` is 1 where the surface faces into the stroke,
    /// 0 where it faces away and one half where there is no direction to
    /// face. `bristle` is this point's bristle, 0..1. `load` is the paint
    /// left on the brush.
    ///
    /// Monotone in every argument a painter would call "more": more raised,
    /// more facing, more load and more catch never lose a point that less
    /// had. `a_drybrush_never_loses_a_point_by_being_given_more` holds it,
    /// and the stroke's consistency depends on it.
    pub fn catches(&self, raised: f32, facing: f32, bristle: f32, load: f32) -> bool {
        self.margin(raised, facing, bristle, load) > 0.0
    }

    /// By how much a point is caught: what reaches it, less what the brush
    /// asks for before it leaves anything.
    fn margin(&self, raised: f32, facing: f32, bristle: f32, load: f32) -> f32 {
        let reach = raised * (LEE_SHARE + (1.0 - LEE_SHARE) * facing.clamp(0.0, 1.0));
        let asked =
            (1.0 - self.catch.clamp(0.0, 1.0)) + self.streak.clamp(0.0, 1.0) * (bristle - 0.5);
        reach * load.clamp(0.0, 1.0) - asked.max(DRY_FLOOR)
    }

    /// How much of a full touch a point takes, 0..=1, for paint that can be
    /// laid in part ([`crate::paint::stroke`]). More than nothing exactly
    /// where [`Drybrush::catches`], and more the further the surface stands
    /// into the brush: a drybrush is heaviest on the sharpest edge and
    /// fades off down its sides.
    #[debug_ensures((0.0..=1.0).contains(&ret), "a share of a full touch")]
    pub fn amount(&self, raised: f32, facing: f32, bristle: f32, load: f32) -> f32 {
        (self.margin(raised, facing, bristle, load) / DRY_RAMP).clamp(0.0, 1.0)
    }
}

impl Sponge {
    /// Whether a point is caught. `speck` is uniform in 0..1 over the
    /// surface, so on a flat the share caught is the density.
    pub fn catches(&self, raised: f32, speck: f32) -> bool {
        speck < self.share(raised)
    }

    /// The share of the surface a sponge covers where it is this raised.
    fn share(&self, raised: f32) -> f32 {
        self.density.clamp(0.0, 1.0)
            + (1.0 - self.density.clamp(0.0, 1.0)) * self.edges.clamp(0.0, 1.0) * raised
    }

    /// How much of a full touch a point takes, 0..=1, for paint that can be
    /// laid in part ([`crate::paint::stroke`]). A speck has a soft rim and
    /// not a staircase: full inside it, nothing outside, and half about
    /// where [`Sponge::catches`] draws the line. The rim is laid so that a
    /// share of everything is still everything and a share of nothing is
    /// still nothing.
    #[debug_ensures((0.0..=1.0).contains(&ret), "a share of a full touch")]
    pub fn amount(&self, raised: f32, speck: f32) -> f32 {
        ((self.share(raised) * (1.0 + SPECK_RIM) - speck) / SPECK_RIM).clamp(0.0, 1.0)
    }
}

/// The directions a gate's pattern is laid out along: the stroke, across it,
/// and out of the surface.
#[derive(Clone, Copy)]
pub(super) struct Frame {
    origin: Vec3,
    along: Vec3,
    across: Vec3,
    out: Vec3,
}

impl Frame {
    pub(super) fn of(stamp: &Stamp, origin: Vec3) -> Self {
        let out = stamp.normal.normalize_or(Vec3::Z);
        // The stroke's direction, laid into the surface. A stroke that has
        // not travelled, or is travelling straight in, has none: then any
        // direction across the surface does, and the same one each time.
        let lying = stamp.tangent - out * stamp.tangent.dot(out);
        let along = if lying.length_squared() > 1e-12 {
            lying.normalize()
        } else {
            out.any_orthonormal_vector()
        };
        Self { origin, along, across: out.cross(along), out }
    }

    /// Which bristle a point is under, as a number in 0..1 that is nearly
    /// constant along the stroke and changes from bristle to bristle across
    /// it.
    pub(super) fn bristle(&self, world: Vec3, dry: Drybrush, seed: u32) -> f32 {
        let width = dry.bristle_mm.max(1e-3);
        let from = world - self.origin;
        // Evened like the sponge's, so that "streak" takes the same share of
        // bristles off the brush at every setting of "catch".
        evened(value_noise(
            Vec3::new(
                from.dot(self.along) / (width * STREAK_LENGTH),
                from.dot(self.across) / width,
                from.dot(self.out) / width,
            ),
            seed,
        ))
    }

    /// The sponge's texture at a point, uniform in 0..1.
    ///
    /// Fixed in the world and not in the stroke's frame: a sponge has no
    /// direction, and a pattern that turned with the stroke would give one
    /// voxel two answers from two stamps of a curving stroke. It does not
    /// read the frame at all, and is here beside `bristle` so that the two
    /// gates are fed the same way.
    pub(super) fn speck(world: Vec3, sponge: Sponge, seed: u32) -> f32 {
        let grain = sponge.grain_mm.max(1e-3);
        evened(value_noise(TURNED * (world / grain), seed))
    }
}

/// A fixed turn put on a sponge's texture before it is sampled.
///
/// Without it a flat face of the model that lies along the world's axes,
/// which on a printed part is most of them, can cut the noise exactly on one
/// of its lattice planes. There the noise is a blend of four numbers and
/// not eight, it bunches less, and a sponge set to cover 60% of the face
/// covered 50% of it: measured, on the top of the test cube, before this was
/// here. Turned, no axis of the world is an axis of the lattice.
///
/// Orthonormal, from the 3-4-5 triangle twice over, so it is exact in
/// decimal and changes no distance.
const TURNED: glam::Mat3 = glam::Mat3::from_cols(
    Vec3::new(0.36, -0.8, 0.48),
    Vec3::new(0.48, 0.6, 0.64),
    Vec3::new(-0.8, 0.0, 0.6),
);

/// Scratch a gated stamp works in, kept by the caller so that a stroke does
/// not allocate once it is under way.
#[derive(Debug, Default)]
pub struct TouchScratch {
    /// The field under the stamp, and a margin round it.
    pub(super) region: FieldRegion,
    /// The curvature of each cell of the region, worked out the first time
    /// a voxel asks for it. See [`Curvatures`].
    pub(super) cavity: Vec<std::cell::Cell<f32>>,
}

impl TouchScratch {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A cell whose curvature has not been asked for yet.
const NOT_ASKED: f32 = f32::NAN;

/// The curvature of the cells under one stamp, each worked out once.
///
/// A voxel answers for the point of the surface nearest it, and the six or
/// so voxels on a line through the band all answer for the same point. So
/// the 32 reads of [`crate::signals::cell_cavity`] are done once a cell and
/// not once a voxel.
///
/// What that is worth, measured with `what_a_gated_stamp_costs` on a brush
/// 40 voxels in radius, in release, on one core of the build container:
/// on a ball, a drybrush stamp is 5.5 ms with it and 6.8 without. On a face
/// lying exactly on a lattice plane, where every voxel's point is in eight
/// cells ([`Curvatures::raised`]), it is 5.6 ms with it and 20.5 without.
/// So it is there for the flat faces of imported parts, which are common,
/// and is a small gain everywhere else. A loaded stamp of the same size is
/// 0.7 ms; the rest of a gated one is the gradient and the noise, a voxel at
/// a time.
pub(super) struct Curvatures<'a> {
    region: &'a FieldRegion,
    lo: IVec3,
    size: IVec3,
    cells: &'a [std::cell::Cell<f32>],
}

impl<'a> Curvatures<'a> {
    pub(super) fn over(region: &'a FieldRegion, cells: &'a mut Vec<std::cell::Cell<f32>>) -> Self {
        let (lo, hi) = region.bounds();
        let size = hi - lo + IVec3::ONE;
        let count = (size.x * size.y * size.z) as usize;
        cells.clear();
        cells.resize(count, std::cell::Cell::new(NOT_ASKED));
        Self { region, lo, size, cells }
    }

    /// The curvature of one cell. A cell that cannot be measured, because
    /// its stencil reaches the edge of the band, is flat: the same answer
    /// the viewport draws for it ([`crate::signals::CAVITY_ABSENT`]).
    fn of(&self, cell: IVec3) -> f32 {
        let local = cell - self.lo;
        let inside = local.cmpge(IVec3::ZERO).all() && local.cmplt(self.size).all();
        let slot = inside.then(|| {
            &self.cells
                [(local.x + local.y * self.size.x + local.z * self.size.x * self.size.y) as usize]
        });
        if let Some(known) = slot.map(std::cell::Cell::get)
            && !known.is_nan()
        {
            return known;
        }
        let region = self.region;
        let cavity = crate::signals::cell_cavity(|x, y, z| region.get(cell + IVec3::new(x, y, z)))
            .unwrap_or(0.0);
        if let Some(slot) = slot {
            slot.set(cavity);
        }
        cavity
    }

    /// How far the surface stands up at a point, 0 on a flat or in a hollow
    /// to 1 on a full edge. `at` is in voxels.
    ///
    /// The curvature is the cell's that holds the point. A point exactly on
    /// a lattice plane is in two cells, on a lattice line in four and on a
    /// lattice point in eight, and "the one above" is not the same cell seen
    /// from the other side of a mirror. A voxel lying exactly on the
    /// surface, which every voxel of a flat face of an imported part does,
    /// was caught on one side of the plane and its twin not: measured, as
    /// one voxel in 29 along a cube's edge. So such a point takes the mean
    /// of every cell it is in, and a point in general position is in one.
    ///
    /// That makes the twin's answer the same number up to rounding, and not
    /// to the bit: a cell's curvature is itself a sum in a fixed order, and
    /// a mirror visits the samples in another. A voxel sitting within a
    /// rounding error of the threshold can still differ from its twin.
    fn raised(&self, at: Vec3, gain: f32) -> f32 {
        self.relief(at, gain).max(0.0)
    }

    /// How far the surface stands up at a point: 1 on a full edge, 0 on a
    /// flat, -1 in a full crease. [`Curvatures::raised`] is the upper half
    /// of it, and a wash runs into the lower.
    #[debug_ensures((-1.0..=1.0).contains(&ret), "from a full crease to a full edge")]
    pub(super) fn relief(&self, at: Vec3, gain: f32) -> f32 {
        let below = at.floor().as_ivec3();
        // One step back along each axis the point is exactly on.
        let span = below - (at.ceil().as_ivec3() - IVec3::ONE);
        let mut sum = 0.0;
        for dz in 0..=span.z {
            for dy in 0..=span.y {
                for dx in 0..=span.x {
                    sum += self.of(below - IVec3::new(dx, dy, dz));
                }
            }
        }
        let cells = (span + IVec3::ONE).element_product() as f32;
        (-(sum / cells) * gain).clamp(-1.0, 1.0)
    }
}

/// The gradient of the stored field at a voxel, by central differences.
pub(super) fn gradient_at(region: &FieldRegion, voxel: IVec3) -> Vec3 {
    Vec3::new(
        region.get(voxel + IVec3::X) - region.get(voxel - IVec3::X),
        region.get(voxel + IVec3::Y) - region.get(voxel - IVec3::Y),
        region.get(voxel + IVec3::Z) - region.get(voxel - IVec3::Z),
    ) * 0.5
}

/// One octave of value noise in 0..1: a hash at each lattice point, blended
/// between them. Smooth, so that a threshold on it cuts blobs and not dust.
fn value_noise(p: Vec3, seed: u32) -> f32 {
    let base = p.floor();
    let f = p - base;
    let t = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let (x, y, z) = (base.x as i32, base.y as i32, base.z as i32);
    let corner = |dx: i32, dy: i32, dz: i32| {
        (hash(x + dx, y + dy, z + dz, seed) >> 8) as f32 / (1 << 24) as f32
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c00 = lerp(corner(0, 0, 0), corner(1, 0, 0), t.x);
    let c10 = lerp(corner(0, 1, 0), corner(1, 1, 0), t.x);
    let c01 = lerp(corner(0, 0, 1), corner(1, 0, 1), t.x);
    let c11 = lerp(corner(0, 1, 1), corner(1, 1, 1), t.x);
    lerp(lerp(c00, c10, t.y), lerp(c01, c11, t.y), t.z)
}

/// An integer hash of a lattice point and a seed. The mixing steps are the
/// finaliser of MurmurHash3, which is public domain.
fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed
        ^ (x as u32).wrapping_mul(0x9e37_79b1)
        ^ (y as u32).wrapping_mul(0x85eb_ca6b)
        ^ (z as u32).wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

/// Value noise is a blend of eight uniform numbers, so it bunches round one
/// half: a threshold at 0.35 would catch far less than 35% of a surface.
/// This maps a noise value to the share of all noise values below it, which
/// is uniform, so that a sponge's density is a share of area.
///
/// The table is the measured distribution, in twentieths, and
/// `the_evening_table_is_the_noises_own_distribution` measures it again.
fn evened(noise: f32) -> f32 {
    let scaled = noise.clamp(0.0, 1.0) * (QUANTILES.len() - 1) as f32;
    let index = (scaled as usize).min(QUANTILES.len() - 2);
    let within = scaled - index as f32;
    QUANTILES[index] + (QUANTILES[index + 1] - QUANTILES[index]) * within
}

/// The share of value noise that falls below each twentieth of its range.
/// Filled in from the measurement the test repeats.
const QUANTILES: [f32; 21] = [
    0.0, 0.0019, 0.0098, 0.0274, 0.0576, 0.1021, 0.1623, 0.2360, 0.3227, 0.4164, 0.5133, 0.6106,
    0.7022, 0.7840, 0.8520, 0.9071, 0.9472, 0.9740, 0.9908, 0.9983, 1.0,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::BrushDirection;

    /// The share of value noise below each twentieth, measured.
    fn measured_quantiles() -> [f32; 21] {
        let mut counts = [0_u64; 21];
        let mut total = 0_u64;
        // Off the lattice in every axis, over several seeds, so that the
        // figure is the noise's and not one seed's.
        for seed in [1_u32, 77, 4242, 90_001] {
            for i in 0..60 {
                for j in 0..60 {
                    for k in 0..60 {
                        let p =
                            Vec3::new(i as f32, j as f32, k as f32) * 0.173 + Vec3::splat(0.031);
                        let noise = value_noise(p, seed);
                        assert!((0.0..=1.0).contains(&noise), "noise left 0..1: {noise}");
                        for (index, count) in counts.iter_mut().enumerate() {
                            if noise < index as f32 / 20.0 {
                                *count += 1;
                            }
                        }
                        total += 1;
                    }
                }
            }
        }
        let mut quantiles = [0.0; 21];
        for (quantile, count) in quantiles.iter_mut().zip(counts) {
            *quantile = count as f32 / total as f32;
        }
        quantiles[20] = 1.0;
        quantiles
    }

    const VOXEL: f32 = 0.5;
    const HALF: f32 = 12.0;

    /// A cube of side 24 mm about the origin: six flats, twelve sharp edges.
    fn cube() -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(HALF + NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(HALF);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / VOXEL).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        volume
    }

    fn paint_brush(radius: f32) -> Brush {
        Brush { kind: crate::brush::BrushKind::Paint, radius, ..Brush::default() }
    }

    /// Every paintable voxel within `radius` of `centre`, with its slot.
    fn painted_near(volume: &Volume, centre: Vec3, radius: f32) -> Vec<(IVec3, Vec3, u8)> {
        let (lo, hi) =
            volume.voxel_bounds(centre - Vec3::splat(radius), centre + Vec3::splat(radius));
        let mut out = Vec::new();
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = IVec3::new(x, y, z);
                    let position = voxel.as_vec3() * VOXEL;
                    if (position - centre).length() >= radius {
                        continue;
                    }
                    if !crate::colour::ColourField::paintable(volume.sample_voxel(voxel)) {
                        continue;
                    }
                    out.push((voxel, position, volume.colour().at(voxel)));
                }
            }
        }
        out
    }

    /// The middle of the cube's top face, and the middle of the edge where
    /// the +x and +y faces meet.
    const ON_FLAT: Vec3 = Vec3::new(0.0, 0.0, HALF);
    const ON_EDGE: Vec3 = Vec3::new(HALF, HALF, 0.0);

    fn coat(touch: Touch) -> Coat {
        Coat { touch, seed: 7, origin: Vec3::ZERO, load: 1.0, crease_gain: 1.0 }
    }

    /// How far a point is from the nearest edge of the cube, in millimetres.
    fn from_an_edge(position: Vec3) -> f32 {
        let mut gaps = (Vec3::splat(HALF) - position.abs()).abs().to_array();
        gaps.sort_by(f32::total_cmp);
        // On an edge the two smallest gaps are both nothing.
        gaps[1]
    }

    /// An edge is raised and a crease is not: what stands up is the upper
    /// half of the relief, and a crease's is all in the lower.
    #[test]
    fn a_crease_is_not_raised() {
        // The cube with a square slot cut across its top face: creases
        // along the foot of the slot's walls, edges along their tops.
        let (depth, half) = (4.0, 3.0);
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(HALF + NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(HALF);
            let cube = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            let slot = (position.x.abs() - half).max(HALF - depth - position.z);
            (cube.max(-slot) / VOXEL).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        let mut region = FieldRegion::new();
        volume.snapshot(IVec3::new(-16, -8, 8), IVec3::new(16, 8, 32), &mut region);
        let mut cells = Vec::new();
        let curvatures = Curvatures::over(&region, &mut cells);
        // In voxels, a little off the lattice so that each is in one cell.
        let foot = Vec3::new(half / VOXEL - 0.3, 0.4, (HALF - depth) / VOXEL + 0.3);
        let lip = Vec3::new(half / VOXEL + 0.3, 0.4, HALF / VOXEL - 0.3);
        assert!(curvatures.relief(foot, 1.0) < -0.5, "{}", curvatures.relief(foot, 1.0));
        assert_eq!(curvatures.raised(foot, 1.0), 0.0);
        assert!(curvatures.relief(lip, 1.0) > 0.5, "{}", curvatures.relief(lip, 1.0));
        assert_eq!(curvatures.raised(lip, 1.0), curvatures.relief(lip, 1.0));
    }

    /// A loaded coat is upstream's paint stamp and not a copy of it.
    #[test]
    fn a_loaded_coat_is_the_plain_paint_stamp() {
        let brush = paint_brush(6.0);
        let stamp = Stamp::new(ON_EDGE, Vec3::new(1.0, 1.0, 0.0).normalize(), BrushDirection::Add)
            .with_tangent(Vec3::Z);
        let mut plain = cube();
        brush.apply_paint(&mut plain, &stamp, 3, &mut crate::brush::BrushScratch::new());
        let mut loaded = cube();
        Coat::LOADED.apply(&brush, &mut loaded, &stamp, 3, &mut TouchScratch::new());

        let wanted = painted_near(&plain, ON_EDGE, 8.0);
        assert!(wanted.iter().any(|(_, _, slot)| *slot == 3), "the plain stamp painted nothing");
        assert_eq!(painted_near(&loaded, ON_EDGE, 8.0), wanted);
    }

    /// The gated stamp has its own copy of the three tests the plain one
    /// makes: the disc, the mask and the pattern's stencil. A sponge at full
    /// density catches everything, so what it writes has to be exactly what
    /// the plain stamp writes, under a mask and through a pattern.
    #[test]
    fn a_gated_stamp_respects_the_mask_and_the_pattern_as_a_plain_one_does() {
        let pattern = crate::pattern::Pattern {
            kind: crate::pattern::PatternKind::Scales,
            scale_mm: 3.0,
            depth: 1.0,
        };
        let brush = Brush { pattern, ..paint_brush(7.0) };
        let stamp = Stamp::new(ON_FLAT, Vec3::Z, BrushDirection::Add).with_tangent(Vec3::X);
        let masked = || {
            let mut volume = cube();
            // A ramp of protection across the stamp, so the half line falls
            // inside it.
            volume.edit_mask(IVec3::splat(-40), IVec3::splat(40), |_, position, _| {
                ((position.x + 7.0) / 14.0 * 255.0).clamp(0.0, 255.0) as u8
            });
            volume
        };
        let mut plain = masked();
        brush.apply_paint(&mut plain, &stamp, 2, &mut crate::brush::BrushScratch::new());
        let mut gated = masked();
        let everything = Touch::Sponge(Sponge { density: 1.0, grain_mm: 1.0, edges: 0.0 });
        coat(everything).apply(&brush, &mut gated, &stamp, 2, &mut TouchScratch::new());

        let wanted = painted_near(&plain, ON_FLAT, 9.0);
        let on = wanted.iter().filter(|(_, _, slot)| *slot == 2).count();
        // The fixture has to exercise all three: some painted, and some left
        // out by the mask and by the pattern, inside the disc.
        assert!(on > 200, "the plain stamp painted only {on} voxels");
        assert!(
            wanted
                .iter()
                .any(|(_, at, slot)| *slot == 0 && at.x < -1.0 && (*at - ON_FLAT).length() < 5.0),
            "the pattern stencilled nothing out of the unprotected half"
        );
        assert!(
            wanted.iter().all(|(_, at, slot)| *slot == 0 || at.x < 0.5),
            "the mask protected nothing"
        );
        assert_eq!(painted_near(&gated, ON_FLAT, 9.0), wanted);
    }

    /// A drybrush paints what stands up and nothing else: over an edge it
    /// leaves a line of paint along the edge and the two faces clean.
    #[test]
    fn a_drybrush_paints_the_edge_and_leaves_the_faces_clean() {
        let brush = paint_brush(8.0);
        let stamp = Stamp::new(ON_EDGE, Vec3::new(1.0, 1.0, 0.0).normalize(), BrushDirection::Add);
        let dry = Touch::Drybrush(Drybrush { catch: 0.7, streak: 0.0, bristle_mm: 0.4 });
        let mut volume = cube();
        coat(dry).apply(&brush, &mut volume, &stamp, 4, &mut TouchScratch::new());

        let voxels = painted_near(&volume, ON_EDGE, 7.5);
        let on_edge: Vec<_> =
            voxels.iter().filter(|(_, at, _)| from_an_edge(*at) <= VOXEL * 0.75).collect();
        let on_face: Vec<_> =
            voxels.iter().filter(|(_, at, _)| from_an_edge(*at) >= VOXEL * 3.0).collect();
        assert!(
            on_edge.len() > 25 && on_face.len() > 1000,
            "the fixture has {} voxels on the edge and {} on the faces",
            on_edge.len(),
            on_face.len()
        );
        let caught = on_edge.iter().filter(|(_, _, slot)| *slot == 4).count();
        assert!(
            caught * 10 > on_edge.len() * 8,
            "only {caught} of the {} voxels on the edge were caught",
            on_edge.len()
        );
        let stray = on_face.iter().filter(|(_, _, slot)| *slot != 0).count();
        assert_eq!(stray, 0, "{stray} voxels of the flat faces took a drybrush");

        // The control for "leaves the faces clean": a loaded brush does not.
        let mut loaded = cube();
        Coat::LOADED.apply(&brush, &mut loaded, &stamp, 4, &mut TouchScratch::new());
        let filled = painted_near(&loaded, ON_EDGE, 7.5)
            .iter()
            .filter(|(_, at, slot)| from_an_edge(*at) >= VOXEL * 3.0 && *slot == 4)
            .count();
        assert_eq!(filled, on_face.len(), "the loaded control did not paint the faces");
    }

    /// An edge catches more on the side the brush comes at. The edge between
    /// the +x and +y faces, brushed toward -x, takes more paint on its +x
    /// side; brushed toward -y, more on its +y side.
    #[test]
    fn a_drybrush_catches_more_on_the_side_it_comes_at() {
        let brush = paint_brush(8.0);
        let normal = Vec3::new(1.0, 1.0, 0.0).normalize();
        // Asked for enough that a side facing away falls short and a side
        // facing in does not.
        let dry = Touch::Drybrush(Drybrush { catch: 0.45, streak: 0.0, bristle_mm: 0.4 });
        let sides = |travel: Vec3| {
            let stamp = Stamp::new(ON_EDGE, normal, BrushDirection::Add).with_tangent(travel);
            let mut volume = cube();
            coat(dry).apply(&brush, &mut volume, &stamp, 4, &mut TouchScratch::new());
            let caught: Vec<_> = painted_near(&volume, ON_EDGE, 7.5)
                .into_iter()
                .filter(|(_, _, slot)| *slot == 4)
                .collect();
            // Which side of the edge's own diagonal a voxel is on.
            let on_x = caught.iter().filter(|(_, at, _)| at.x > at.y).count();
            let on_y = caught.iter().filter(|(_, at, _)| at.y > at.x).count();
            (on_x, on_y)
        };
        let (x_side, y_side) = sides(-Vec3::X);
        assert!(
            x_side > 20 && x_side > y_side * 2,
            "brushed toward -x the edge took {x_side} on its x side and {y_side} on its y side"
        );
        let (x_side, y_side) = sides(-Vec3::Y);
        assert!(
            y_side > 20 && y_side > x_side * 2,
            "brushed toward -y the edge took {x_side} on its x side and {y_side} on its y side"
        );
        // And with no direction of travel it has no preference.
        let (x_side, y_side) = sides(Vec3::ZERO);
        assert_eq!(x_side, y_side, "a brush that has not moved favoured a side");
    }

    /// What counts as a full edge is the caller's to say. At a tenth of the
    /// gain the same edge is a tenth as raised, and a brush that caught it
    /// lets it go.
    #[test]
    fn the_gain_decides_what_a_full_edge_is() {
        let brush = paint_brush(8.0);
        let stamp = Stamp::new(ON_EDGE, Vec3::new(1.0, 1.0, 0.0).normalize(), BrushDirection::Add);
        let dry = Touch::Drybrush(Drybrush { catch: 0.7, streak: 0.0, bristle_mm: 0.4 });
        let caught = |crease_gain: f32| {
            let mut volume = cube();
            Coat { crease_gain, ..coat(dry) }.apply(
                &brush,
                &mut volume,
                &stamp,
                4,
                &mut TouchScratch::new(),
            );
            painted_near(&volume, ON_EDGE, 7.5).iter().filter(|v| v.2 == 4).count()
        };
        assert!(caught(1.0) > 50, "the fixture catches only {} at full gain", caught(1.0));
        assert_eq!(caught(0.1), 0, "a tenth of the gain still caught the edge");
        assert_eq!(caught(0.0), 0, "no gain still caught the edge");
    }

    /// The brush runs out. Less load never catches a voxel that more load
    /// missed, and none catches nothing.
    #[test]
    fn a_drybrush_runs_out() {
        let brush = paint_brush(8.0);
        let stamp = Stamp::new(ON_EDGE, Vec3::new(1.0, 1.0, 0.0).normalize(), BrushDirection::Add);
        let dry = Touch::Drybrush(Drybrush { catch: 0.6, streak: 0.6, bristle_mm: 0.5 });
        let with_load = |load: f32| {
            let mut volume = cube();
            Coat { load, ..coat(dry) }.apply(
                &brush,
                &mut volume,
                &stamp,
                4,
                &mut TouchScratch::new(),
            );
            painted_near(&volume, ON_EDGE, 7.5)
        };
        let (full, half, none) = (with_load(1.0), with_load(0.6), with_load(0.0));
        let count = |voxels: &[(IVec3, Vec3, u8)]| voxels.iter().filter(|v| v.2 == 4).count();
        assert!(count(&full) > 100, "a full brush caught only {}", count(&full));
        assert!(
            count(&half) > 10 && count(&half) * 10 < count(&full) * 9,
            "a brush at 0.6 caught {} against {} when full",
            count(&half),
            count(&full)
        );
        assert_eq!(count(&none), 0, "an empty brush left paint");
        for (less, more) in half.iter().zip(&full) {
            assert!(
                less.2 == 0 || more.2 == 4,
                "{} was caught by less load and not by more",
                less.0
            );
        }
    }

    /// Streaks: with `streak` up, the paint along an edge is broken into
    /// runs, and the runs lie along the stroke.
    #[test]
    fn a_streaky_drybrush_breaks_the_line_into_runs_along_the_stroke() {
        let brush = paint_brush(10.0);
        let normal = Vec3::new(1.0, 1.0, 0.0).normalize();
        // Along the edge, so facing plays no part: the edge's normal is
        // square to the travel.
        let stamp = Stamp::new(ON_EDGE, normal, BrushDirection::Add).with_tangent(Vec3::Z);
        let along_the_edge = |streak: f32| {
            let dry = Touch::Drybrush(Drybrush { catch: 0.6, streak, bristle_mm: 0.5 });
            let mut volume = cube();
            coat(dry).apply(&brush, &mut volume, &stamp, 4, &mut TouchScratch::new());
            // The voxels of the edge itself, in order along it.
            (-16..=16).map(|z| volume.colour().at(IVec3::new(24, 24, z)) == 4).collect::<Vec<_>>()
        };
        let solid = along_the_edge(0.0);
        assert!(solid.iter().all(|on| *on), "with no streak the edge is not painted end to end");

        // Brushed ALONG the edge, a bristle either rides the edge or does
        // not: the line is nearly all on or nearly all off, in long runs.
        let streaked = along_the_edge(1.0);
        let changes = streaked.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(
            changes <= 4,
            "brushed along the edge the line changed {changes} times in 33 voxels"
        );

        // Brushed ACROSS it, each bristle crosses the edge once, so the line
        // is cut into as many pieces as there are bristles.
        let across = Stamp::new(ON_EDGE, normal, BrushDirection::Add)
            .with_tangent(Vec3::new(1.0, -1.0, 0.0).normalize());
        let dry = Touch::Drybrush(Drybrush { catch: 0.6, streak: 1.0, bristle_mm: 0.5 });
        let mut volume = cube();
        coat(dry).apply(&brush, &mut volume, &across, 4, &mut TouchScratch::new());
        let crossed: Vec<bool> =
            (-16..=16).map(|z| volume.colour().at(IVec3::new(24, 24, z)) == 4).collect();
        let on = crossed.iter().filter(|on| **on).count();
        let cuts = crossed.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(on > 6 && on < 30, "brushed across, {on} of 33 edge voxels took paint");
        assert!(cuts >= 5, "brushed across the edge the line was cut only {cuts} times");
    }

    /// More of anything a painter would call more never loses a point.
    #[test]
    fn a_drybrush_never_loses_a_point_by_being_given_more() {
        let steps = [0.0, 0.1, 0.3, 0.5, 0.7, 0.9, 1.0];
        for catch in steps {
            for streak in [0.0, 0.5, 1.0] {
                for bristle in steps {
                    for raised in steps {
                        for facing in steps {
                            for load in steps {
                                let dry = Drybrush { catch, streak, bristle_mm: 0.4 };
                                if !dry.catches(raised, facing, bristle, load) {
                                    continue;
                                }
                                let label = format!(
                                    "catch {catch} streak {streak} bristle {bristle} \
                                     raised {raised} facing {facing} load {load}"
                                );
                                assert!(raised > 0.0, "a flat was caught: {label}");
                                assert!(dry.catches(1.0, facing, bristle, load), "raised: {label}");
                                assert!(dry.catches(raised, 1.0, bristle, load), "facing: {label}");
                                assert!(dry.catches(raised, facing, bristle, 1.0), "load: {label}");
                                let more = Drybrush { catch: 1.0, ..dry };
                                assert!(
                                    more.catches(raised, facing, bristle, load),
                                    "catch: {label}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// A sponge's density is the share of a flat it covers, more density
    /// only adds, and the speckle is one speckle all the way through the
    /// band.
    #[test]
    fn a_sponge_covers_its_density_of_a_flat_and_the_same_specks_through_the_band() {
        let brush = paint_brush(10.0);
        let stamp = Stamp::new(ON_FLAT, Vec3::Z, BrushDirection::Add);
        let sponged = |density: f32| {
            let sponge = Touch::Sponge(Sponge { density, grain_mm: 1.0, edges: 0.0 });
            let mut volume = cube();
            coat(sponge).apply(&brush, &mut volume, &stamp, 5, &mut TouchScratch::new());
            volume
        };
        // The layer of voxels on the surface itself, well inside the disc.
        let share = |volume: &Volume| {
            let layer: Vec<_> = painted_near(volume, ON_FLAT, 9.0)
                .into_iter()
                .filter(|(voxel, _, _)| voxel.z == 24)
                .collect();
            assert!(layer.len() > 900, "the layer is only {} voxels", layer.len());
            layer.iter().filter(|v| v.2 == 5).count() as f32 / layer.len() as f32
        };
        let (light, heavy) = (sponged(0.3), sponged(0.6));
        let (light_share, heavy_share) = (share(&light), share(&heavy));
        assert!(
            (light_share - 0.3).abs() < 0.08,
            "a sponge of density 0.3 covered {light_share} of a flat"
        );
        assert!(
            (heavy_share - 0.6).abs() < 0.08,
            "a sponge of density 0.6 covered {heavy_share} of a flat"
        );
        for (less, more) in
            painted_near(&light, ON_FLAT, 9.0).iter().zip(painted_near(&heavy, ON_FLAT, 9.0))
        {
            assert!(less.2 == 0 || more.2 == 5, "{} lost its speck to more density", less.0);
        }

        // Through the band: every voxel above and below a surface voxel has
        // what the surface voxel has.
        let mut columns = 0;
        for (voxel, _, slot) in painted_near(&light, ON_FLAT, 8.0) {
            if voxel.z != 24 {
                continue;
            }
            columns += 1;
            for depth in [-2, -1, 1, 2] {
                let other = light.colour().at(voxel + IVec3::new(0, 0, depth));
                assert_eq!(other, slot, "the speck at {voxel} is not the speck {depth} from it");
            }
        }
        assert!(columns > 700, "only {columns} columns were compared");
    }

    /// A sponge leaves more on edges when asked to, and at full bias an edge
    /// is covered whatever the density.
    #[test]
    fn a_sponge_can_favour_the_edges() {
        let brush = paint_brush(8.0);
        let stamp = Stamp::new(ON_EDGE, Vec3::new(1.0, 1.0, 0.0).normalize(), BrushDirection::Add);
        let on_the_edge = |edges: f32| {
            let sponge = Touch::Sponge(Sponge { density: 0.1, grain_mm: 1.0, edges });
            let mut volume = cube();
            coat(sponge).apply(&brush, &mut volume, &stamp, 5, &mut TouchScratch::new());
            let line: Vec<bool> =
                (-12..=12).map(|z| volume.colour().at(IVec3::new(24, 24, z)) == 5).collect();
            line.iter().filter(|on| **on).count() as f32 / line.len() as f32
        };
        let (plain, biased) = (on_the_edge(0.0), on_the_edge(1.0));
        assert!(plain < 0.4, "with no bias a thin sponge covered {plain} of the edge");
        assert!(biased > 0.8, "at full bias a thin sponge covered only {biased} of the edge");
    }

    /// Two stamps of one stroke give a voxel the same answer, so going over a
    /// place twice within a stroke adds nothing. A second stroke, with its
    /// own seed, does add.
    #[test]
    fn stamps_of_one_stroke_agree_and_a_second_stroke_adds() {
        let brush = paint_brush(8.0);
        let first = Stamp::new(ON_FLAT, Vec3::Z, BrushDirection::Add).with_tangent(Vec3::X);
        // A quarter of a radius on, as a stroke spaces them, and the stroke
        // has turned a little.
        let second = Stamp::new(ON_FLAT + Vec3::new(2.0, 0.0, 0.0), Vec3::Z, BrushDirection::Add)
            .with_tangent(Vec3::new(1.0, 0.3, 0.0).normalize());
        let sponge = coat(Touch::Sponge(Sponge { density: 0.3, grain_mm: 1.0, edges: 0.5 }));
        let under_the_first = |volume: &Volume| painted_near(volume, ON_FLAT, 7.9);

        let mut once = cube();
        sponge.apply(&brush, &mut once, &first, 5, &mut TouchScratch::new());
        let mut twice = cube();
        sponge.apply(&brush, &mut twice, &first, 5, &mut TouchScratch::new());
        sponge.apply(&brush, &mut twice, &second, 5, &mut TouchScratch::new());
        assert_eq!(
            under_the_first(&twice),
            under_the_first(&once),
            "a second stamp of the same stroke changed what the first had decided"
        );

        let mut again = cube();
        sponge.apply(&brush, &mut again, &first, 5, &mut TouchScratch::new());
        Coat { seed: 8, ..sponge }.apply(&brush, &mut again, &second, 5, &mut TouchScratch::new());
        let count = |voxels: Vec<(IVec3, Vec3, u8)>| voxels.iter().filter(|v| v.2 == 5).count();
        let (one, two) = (count(under_the_first(&once)), count(under_the_first(&again)));
        assert!(two > one + one / 4, "a second stroke added only {} to {one}", two - one);
    }

    /// A mirrored sponge is the mirror image, speck for speck.
    #[test]
    fn a_mirrored_stamp_leaves_the_mirror_image() {
        let brush = paint_brush(6.0);
        let at = Vec3::new(5.0, 2.0, HALF);
        let stamp = Stamp::new(at, Vec3::Z, BrushDirection::Add).with_tangent(Vec3::X);
        let sponge = coat(Touch::Sponge(Sponge { density: 0.4, grain_mm: 1.0, edges: 0.0 }));
        let mut volume = cube();
        sponge.apply_symmetric(
            &brush,
            &mut volume,
            &stamp,
            5,
            Symmetry::X,
            Vec3::ZERO,
            &mut TouchScratch::new(),
        );
        let here = painted_near(&volume, at, 5.5);
        let on = here.iter().filter(|v| v.2 == 5).count();
        assert!(on > 100 && on < here.len() - 100, "the fixture is all or nothing: {on}");
        for (voxel, _, slot) in &here {
            let twin = IVec3::new(-voxel.x, voxel.y, voxel.z);
            assert_eq!(volume.colour().at(twin), *slot, "{voxel} and {twin} are not twins");
        }

        // The control: the same sponge put down at the twin's place as a
        // stamp of its own is NOT the mirror image, so the agreement above
        // was the flip's doing and not the noise happening to be symmetric.
        let mut unflipped = cube();
        let twin_stamp = Stamp::new(at * Vec3::new(-1.0, 1.0, 1.0), Vec3::Z, BrushDirection::Add)
            .with_tangent(-Vec3::X);
        sponge.apply(&brush, &mut unflipped, &stamp, 5, &mut TouchScratch::new());
        sponge.apply(&brush, &mut unflipped, &twin_stamp, 5, &mut TouchScratch::new());
        let differing = here
            .iter()
            .filter(|(voxel, _, slot)| {
                unflipped.colour().at(IVec3::new(-voxel.x, voxel.y, voxel.z)) != *slot
            })
            .count();
        assert!(differing > 100, "the control is symmetric by accident: {differing} differ");
    }

    /// A mirrored drybrush lays its bristles as the mirror image too. The
    /// sponge above cannot show this, because a sponge has no direction: the
    /// bristle lines are laid out along the stroke, and the twin's have to
    /// be laid out along the SOURCE stroke and read through the flip.
    #[test]
    fn a_mirrored_drybrush_streaks_the_mirror_image() {
        let brush = paint_brush(9.0);
        // The edge between the +x and +z faces, and its twin between -x and
        // +z. Brushed across, with full streak, so the line is cut by the
        // bristles and where it is cut is the thing compared.
        let at = Vec3::new(HALF, 0.0, HALF);
        let normal = Vec3::new(1.0, 0.0, 1.0).normalize();
        let travel = Vec3::new(1.0, 0.3, -1.0).normalize();
        let stamp = Stamp::new(at, normal, BrushDirection::Add).with_tangent(travel);
        let dry = coat(Touch::Drybrush(Drybrush { catch: 0.45, streak: 1.0, bristle_mm: 0.5 }));
        let mut volume = cube();
        dry.apply_symmetric(
            &brush,
            &mut volume,
            &stamp,
            4,
            Symmetry::X,
            Vec3::ZERO,
            &mut TouchScratch::new(),
        );
        let line = |x: i32| -> Vec<bool> {
            (-14..=14).map(|y| volume.colour().at(IVec3::new(x, y, 24)) == 4).collect()
        };
        let (here, there) = (line(24), line(-24));
        let on = here.iter().filter(|on| **on).count();
        let cuts = here.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(on > 4 && on < 26 && cuts >= 4, "the line is not streaked: {on} on, {cuts} cuts");
        assert_eq!(there, here, "the twin's streaks are not the mirror image");
    }

    /// Erasing goes through the same gate: a sponge with slot 0 chips paint
    /// off, and leaves the field alone like every paint stamp.
    #[test]
    fn a_sponge_can_chip_paint_off_and_never_moves_the_field() {
        let brush = paint_brush(8.0);
        let stamp = Stamp::new(ON_FLAT, Vec3::Z, BrushDirection::Add);
        let untouched = cube();
        let mut volume = cube();
        Coat::LOADED.apply(&brush, &mut volume, &stamp, 6, &mut TouchScratch::new());
        let before = painted_near(&volume, ON_FLAT, 7.5);
        let sponge = coat(Touch::Sponge(Sponge { density: 0.3, grain_mm: 1.0, edges: 0.0 }));
        sponge.apply(
            &brush,
            &mut volume,
            &stamp,
            crate::colour::UNPAINTED,
            &mut TouchScratch::new(),
        );
        let after = painted_near(&volume, ON_FLAT, 7.5);
        let chipped = after.iter().filter(|v| v.2 == 0).count();
        assert!(before.iter().all(|v| v.2 == 6), "the base coat is not whole");
        assert!(
            chipped * 10 > after.len() && chipped * 2 < after.len(),
            "{chipped} of {} voxels were chipped",
            after.len()
        );
        crate::testing::assert_same_field(
            &volume,
            &untouched,
            "a gated paint stamp moved the field",
        );
    }

    #[test]
    fn the_evening_table_is_the_noises_own_distribution() {
        let measured = measured_quantiles();
        for (index, (table, measured)) in QUANTILES.iter().zip(measured).enumerate() {
            assert!(
                (table - measured).abs() < 0.01,
                "at {}: the table says {table} and the noise measures {measured}",
                index as f32 / 20.0
            );
        }
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use crate::brush::BrushDirection;

    #[test]
    #[ignore = "a measurement, not a claim: run by hand, in release"]
    fn what_a_gated_stamp_costs() {
        for (voxel, radius, flat) in
            [(0.25_f32, 3.0_f32, false), (0.25, 10.0, false), (0.1, 5.0, false), (0.25, 10.0, true)]
        {
            let mut volume = Volume::new(voxel);
            if flat {
                // A face lying exactly on a lattice plane: the worst case,
                // where every voxel answers for a point in eight cells.
                let reach = Vec3::splat(30.0 + NARROW_BAND * voxel * 2.0);
                volume.edit_box(-reach, reach, move |position, _| {
                    let outside = position.abs() - Vec3::splat(30.0);
                    let distance =
                        outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
                    (distance / voxel).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
                });
            } else {
                volume.seed_sphere(Vec3::ZERO, 30.0);
            }
            let brush = Brush { kind: crate::brush::BrushKind::Paint, radius, ..Brush::default() };
            let stamp = Stamp::new(Vec3::new(0.0, 0.0, 30.0), Vec3::Z, BrushDirection::Add)
                .with_tangent(Vec3::X);
            let mut region = TouchScratch::new();
            let mut time = |coat: Coat| {
                let rounds = 20;
                let started = std::time::Instant::now();
                for round in 0..rounds {
                    coat.apply(&brush, &mut volume, &stamp, 1 + (round % 2) as u8, &mut region);
                }
                started.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds)
            };
            let base = Coat {
                seed: 1,
                origin: Vec3::ZERO,
                load: 1.0,
                crease_gain: 1.0,
                touch: Touch::Loaded,
            };
            let loaded = time(base);
            let dry = time(Coat { touch: Touch::Drybrush(Drybrush::default()), ..base });
            let sponge = time(Coat { touch: Touch::Sponge(Sponge::default()), ..base });
            eprintln!(
                "flat {flat}, voxel {voxel} mm, radius {radius} mm ({} voxels): loaded {loaded:.2} ms, \
                 drybrush {dry:.2} ms, sponge {sponge:.2} ms",
                radius / voxel
            );
        }
    }
}
