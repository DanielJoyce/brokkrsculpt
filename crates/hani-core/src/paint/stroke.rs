// SPDX-License-Identifier: AGPL-3.0-only

//! A stroke of paint: one colour, laid over the surface once, however many
//! stamps the stroke is made of.
//!
//! `docs/STUDIO-PLAN.md` section 15 is the design. This is the brush of it.
//!
//! # A stroke is a layer
//!
//! A brush that blended its colour into the surface at every stamp would
//! compound. A stroke is hundreds of overlapping stamps, a voxel under it is
//! under eight or so of them, and each would lay its share over the last
//! one's result: a stroke at half strength would come out nearly opaque, a
//! slow stroke darker than a fast one, and the soft edge of the brush would
//! be eaten from the inside.
//!
//! So nothing here reads what the last stamp wrote. Through a stroke each
//! voxel keeps its **coverage**, and what it holds is
//!
//! ```text
//! laid(what it held when the stroke began, the pigment, coverage)
//! ```
//!
//! worked out from the paint as it was before the stroke, which the open
//! undo record already holds ([`Volume::edit_paint_over`]). A second
//! stroke lays a second coat.
//!
//! # Coverage: the most, and what was deposited
//!
//! The coverage is the larger of two numbers, held to one coat:
//!
//! - the **most** any one stamp of the stroke gave the voxel, which is
//!   what a click without moving lays: the brush's own falloff;
//! - what the stroke has **deposited** there as it travelled: each stamp
//!   lays what it reaches by the distance it moved, scaled so that the
//!   middle of one straight pass is one coat. A slow stroke and a fast one
//!   deposit the same.
//!
//! The coverage was the most alone until 2026-11-25, when the maintainer
//! saw a return pass laid beside a first one leave a paler seam between
//! them, and a turn leave a pale wedge inside its corner: each of those is
//! reached only by the soft edges of the brush, and the most of two soft
//! edges is less than either middle. Deposited, the two edges add up to a
//! coat. A stroke still never lays more than a coat, so going over paint
//! with the same stroke never makes it paler or darker than a coat of it.
//!
//! # Linear light
//!
//! Colours are stored as sRGB and mixed as light. Half a coat of red over
//! green is the olive a palette gives and not the brown that averaging the
//! bytes does. [`laid`] is the whole of that arithmetic, and the shader's
//! compositing of paint over clay is its twin.

use contracts::{debug_ensures, debug_requires, test_ensures};
use glam::{IVec3, Vec3};
use rustc_hash::FxHashMap;

use crate::brick::{BRICK_VOXELS, BrickCoord, NARROW_BAND};
use crate::brush::{Brush, Flip, Stamp, Symmetry};
use crate::mask::PROTECTED;
use crate::paint::field::{PaintVisit, Rgba, bricks_between};
use crate::paint::touch::{Curvatures, Drybrush, Frame, Sponge, TouchScratch, gradient_at};
use crate::paint::{linear, srgb};
use crate::pattern::Prepared;
use crate::volume::Volume;

/// How much of itself a glaze leaves on bare clay, against what the same
/// stroke of paint would. A glaze is a film: over paint it tints, and over
/// nothing it is a thin coat of its own colour.
pub const GLAZE_BODY: f32 = 0.5;

/// How much of a wash stays on a flat, against a full crease. The rest runs
/// off: all of it from a full edge.
pub const WASH_FLAT: f32 = 0.3;

/// How the pigment combines with what is under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Covers.
    #[default]
    Paint,
    /// Tints without hiding: what is under it is seen through a film of
    /// the colour's hue, which gives back some of the lightness it takes.
    /// See [`tinted`].
    Glaze,
    /// Takes paint off, back toward the clay. The colour is not read.
    Erase,
}

/// What is on the brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pigment {
    /// sRGB.
    pub colour: [u8; 3],
    /// How much of it a full touch of the brush lays, 0..=1.
    pub opacity: f32,
    pub mode: Mode,
    /// For a glaze: how much of the lightness its film takes it gives back,
    /// 0..=1. Nought is the whole film, as dark as light through the
    /// colour; one keeps every shade under it. [`GLAZE_KEEPS`] unless the
    /// paint panel says otherwise. Read by no other mode.
    pub glaze: f32,
}

impl Default for Pigment {
    fn default() -> Self {
        Self { colour: [0, 0, 0], opacity: 1.0, mode: Mode::Paint, glaze: GLAZE_KEEPS }
    }
}

/// What of the surface under the brush the paint comes off on.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Catch {
    /// All of it.
    #[default]
    All,
    /// The recesses: a wash. Full in a crease, [`WASH_FLAT`] on a flat,
    /// nothing on an edge.
    Recesses,
    /// What stands up into a nearly dry brush.
    Raised(Drybrush),
    /// The specks of a sponge.
    Specks(Sponge),
}

/// One stroke's worth of paint: what is on the brush, how it catches, and
/// the things that are fixed from the press to the release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lay {
    pub pigment: Pigment,
    pub catch: Catch,
    /// Fixed for a stroke, different between strokes: which speckle, which
    /// bristles. See [`crate::paint::touch`] for why it is not per stamp.
    pub seed: u32,
    /// Where the stroke began, in world space. Bristle lines are counted
    /// from here.
    pub origin: Vec3,
    /// How much paint is left on a dry brush, 1 down to 0.
    pub load: f32,
    /// How many units of the crease signal a full edge is: the view's gain,
    /// so that what a wash runs into is what "show creases" darkens.
    pub crease_gain: f32,
}

impl Lay {
    /// A loaded brush of `pigment`, which reads none of the other fields.
    pub fn loaded(pigment: Pigment) -> Self {
        Self {
            pigment,
            catch: Catch::All,
            seed: 0,
            origin: Vec3::ZERO,
            load: 1.0,
            crease_gain: 1.0,
        }
    }
}

/// What a voxel holds once `amount` of a full touch of `pigment` is laid
/// over `before`. `amount` is the stroke's coverage of the voxel, 0..=1,
/// and the pigment's opacity scales it.
///
/// Straight alpha in and out, mixed in linear light. Nothing laid is
/// `before` to the bit, and a voxel left with no coverage is bare.
pub fn laid(before: Rgba, pigment: Pigment, amount: f32) -> Rgba {
    Ready::of(pigment).over(before, amount)
}

/// sRGB to linear light, a byte at a time, from a table: a stamp asks this
/// of three channels of every voxel under it, and the curve is a power.
fn light(byte: u8) -> f32 {
    static TABLE: std::sync::LazyLock<[f32; 256]> =
        std::sync::LazyLock::new(|| std::array::from_fn(|byte| linear([byte as u8; 3])[0]));
    TABLE[usize::from(byte)]
}

/// Linear light back to an sRGB byte: [`srgb`] to the bit, without its
/// power.
///
/// A stamp asks this of three channels of every voxel it lays paint on.
/// What the three savings of this file are worth together (this table,
/// the one for [`light`], and making the pigment [`Ready`] once a stamp),
/// measured with `what_a_stamp_of_paint_costs` in release on one core of
/// the build container, for a loaded stamp that lays paint on every voxel
/// under it: 3.8 ms before them and about 3.0 after at 40 voxels of
/// radius, 9.8 and about 5.0 at 50. Runs of the measurement differ from
/// each other by about a fifth, so the first of those is a small gain and
/// the second is a real one.
///
/// Exact and not approximate, because a stroke that passes over paint must
/// hand back the bytes it found. For each level the least value that
/// [`srgb`] gives that level is found once, by bisection on `srgb` itself,
/// and a value is encoded by where it falls among those: a coarse table
/// says where to start looking, and it is never more than a step or two
/// away.
struct Encode {
    /// `least[level]` is the least value that encodes to `level` or more.
    least: [f32; 256],
    /// The level of the low end of each of 4096 equal parts of 0..=1.
    start: [u8; 4097],
}

impl Encode {
    fn new() -> Self {
        let exact = |c: f32| srgb([c; 3])[0];
        let mut least = [0.0_f32; 256];
        for (level, slot) in least.iter_mut().enumerate().skip(1) {
            // The least f32 in 0..=1 that encodes to this level or more.
            // Non-negative floats order as their bits do.
            let (mut below, mut at) = (0.0_f32.to_bits(), 1.0_f32.to_bits());
            while at - below > 1 {
                let middle = below + (at - below) / 2;
                if usize::from(exact(f32::from_bits(middle))) >= level {
                    at = middle;
                } else {
                    below = middle;
                }
            }
            *slot = f32::from_bits(at);
        }
        let start = std::array::from_fn(|part| exact(part as f32 / 4096.0));
        Self { least, start }
    }

    #[test_ensures(ret == srgb([c; 3])[0], "the table is the curve")]
    fn byte(&self, c: f32) -> u8 {
        let c = c.clamp(0.0, 1.0);
        // A value that is not a number is nothing, as `srgb` has it.
        let mut level = usize::from(self.start[(c * 4096.0) as usize]);
        while level < 255 && c >= self.least[level + 1] {
            level += 1;
        }
        level as u8
    }
}

fn encode(colour: [f32; 3]) -> [u8; 3] {
    static ENCODE: std::sync::LazyLock<Encode> = std::sync::LazyLock::new(Encode::new);
    colour.map(|c| ENCODE.byte(c))
}

/// A pigment made ready to lay: its colour as light, worked out once for a
/// stamp and not once for every voxel under it.
#[derive(Debug, Clone, Copy)]
struct Ready {
    over: [f32; 3],
    opacity: f32,
    mode: Mode,
    glaze: f32,
}

impl Ready {
    fn of(pigment: Pigment) -> Self {
        Self {
            over: pigment.colour.map(light),
            opacity: pigment.opacity,
            mode: pigment.mode,
            glaze: pigment.glaze.clamp(0.0, 1.0),
        }
    }

    /// [`laid`], which is this.
    #[debug_requires(!amount.is_nan() && !self.opacity.is_nan(), "an amount is a number")]
    #[debug_ensures(ret == crate::paint::field::stored(ret), "laid as paint is stored")]
    #[debug_ensures(self.mode != Mode::Erase || ret[3] <= before[3], "an eraser adds nothing")]
    #[debug_ensures(self.mode == Mode::Erase || ret[3] >= before[3], "paint takes nothing off")]
    fn over(&self, before: Rgba, amount: f32) -> Rgba {
        let a = (amount * self.opacity).clamp(0.0, 1.0);
        let under = [light(before[0]), light(before[1]), light(before[2])];
        let under_a = f32::from(before[3]) / 255.0;
        let over = self.over;
        let (colour, coverage) = match self.mode {
            Mode::Paint => {
                let coverage = a + under_a * (1.0 - a);
                let mix = |c: usize| (over[c] * a + under[c] * under_a * (1.0 - a)) / coverage;
                ([mix(0), mix(1), mix(2)], coverage)
            }
            Mode::Glaze => {
                // Over the part that is painted, the paint there seen
                // through the glaze, by how much of the glaze there is.
                // Over the part that is bare, a thin coat of the clay so
                // seen. See `tinted`.
                let clay = crate::paint::layers::CLAY.map(light);
                let painted = tinted_keeping(under, over, self.glaze);
                let bare = tinted_keeping(clay, over, self.glaze);
                let thin = (1.0 - under_a) * a * GLAZE_BODY;
                let coverage = under_a + thin;
                let mix = |c: usize| {
                    (under[c] * (1.0 - a) + painted[c] * a) * under_a / coverage.max(1.0e-6)
                        + bare[c] * thin / coverage.max(1.0e-6)
                };
                ([mix(0), mix(1), mix(2)], coverage)
            }
            Mode::Erase => (under, under_a * (1.0 - a)),
        };
        let alpha = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
        let [r, g, b] = encode(colour);
        crate::paint::field::stored([r, g, b, alpha])
    }
}

/// `under` seen through a glaze of `glaze`, both linear.
///
/// A glaze multiplied what was under it by its colour until 2026-11-27,
/// which is light through a film, and at a full touch is the film's whole
/// filter: a red over a mid grey came out dark brown (88, 9, 9) and over a
/// green nearly black (25, 8, 1), worked out with this file's own
/// conversions. The maintainer found it "just seems to make everything
/// darker without the color taking effect".
///
/// The first change kept the lightness under it and gave it the glaze's
/// colour channel by channel. On a screen a dark blue glaze over white
/// came out a bright cyan: the blue channel tops out at white's lightness
/// and the rest of it went to green. So now a glaze is still a film, but
/// a film of its hue only (its colour over its brightest channel, so a
/// dark blue filters as a full blue does), and then [`GLAZE_KEEPS`] of the
/// lightness the film took is given back, keeping the hue, as the
/// W3C "color" blend sets a lightness. The red over the mid grey is
/// (149, 84, 84), over the green (83, 76, 73), and the blue over white
/// (189, 204, 255), worked out in this function's arithmetic.
pub(crate) fn tinted(under: [f32; 3], glaze: [f32; 3]) -> [f32; 3] {
    tinted_keeping(under, glaze, GLAZE_KEEPS)
}

/// [`tinted`], giving back `keeps` of the lightness the film takes, 0..=1:
/// the paint panel's glaze setting ([`Pigment::glaze`]).
pub(crate) fn tinted_keeping(under: [f32; 3], glaze: [f32; 3], keeps: f32) -> [f32; 3] {
    let brightest = glaze[0].max(glaze[1]).max(glaze[2]).max(1.0e-4);
    let film: [f32; 3] = std::array::from_fn(|c| under[c] * (glaze[c] / brightest).min(1.0));
    let dark = luma(film);
    with_lightness(film, dark + (luma(under) - dark) * keeps.clamp(0.0, 1.0))
}

/// How much of the lightness a glaze's film takes that it gives back, unless
/// the paint panel says otherwise ([`Pigment::glaze`]); the layer blend's
/// always.
pub const GLAZE_KEEPS: f32 = 0.5;

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// `colour` moved to `lightness` keeping its hue, brought back inside
/// 0..=1 toward its own grey where moving it took a channel out: the
/// SetLum and ClipColor of the W3C compositing note.
fn with_lightness(colour: [f32; 3], lightness: f32) -> [f32; 3] {
    let shift = lightness - luma(colour);
    let moved = colour.map(|channel| channel + shift);
    let grey = luma(moved);
    let least = moved[0].min(moved[1]).min(moved[2]);
    let most = moved[0].max(moved[1]).max(moved[2]);
    let mut out = moved;
    if least < 0.0 && grey - least > 1.0e-6 {
        out = out.map(|channel| grey + (channel - grey) * grey / (grey - least));
    }
    if most > 1.0 && most - grey > 1.0e-6 {
        out = out.map(|channel| grey + (channel - grey) * (1.0 - grey) / (most - grey));
    }
    out.map(|channel| channel.clamp(0.0, 1.0))
}

/// How much of a wash a point takes. `relief` is how far the surface
/// stands up there: 1 on a full edge, 0 on a flat, -1 in a full crease.
#[debug_ensures((0.0..=1.0).contains(&ret), "a share of the wash")]
#[debug_requires(!relief.is_nan(), "a relief is a number")]
pub fn pooled(relief: f32) -> f32 {
    let recess = (-relief).clamp(0.0, 1.0);
    let ridge = relief.clamp(0.0, 1.0);
    (WASH_FLAT + (1.0 - WASH_FLAT) * recess - ridge).clamp(0.0, 1.0)
}

/// A stroke in progress: the coverage it has given so far, and the scratch
/// its stamps work in.
///
/// Kept by the caller from one stroke to the next, so that a stroke does
/// not allocate once it is under way. [`PaintStroke::begin`] starts a new
/// one.
///
/// **A stroke of paint is laid inside an undo stroke**
/// ([`Volume::begin_stroke`]), one for one. What a voxel held before the
/// stroke is read back from the undo record, and with none open there is
/// no "before": each stamp would lay over the last one's result, which is
/// the compounding this type exists to prevent. A debug build refuses a
/// stamp laid outside one.
#[derive(Debug, Default)]
pub struct PaintStroke {
    /// What this stroke has given each voxel, a brick at a time. Only the
    /// bricks a stamp has laid paint in.
    coverage: FxHashMap<BrickCoord, Cover>,
    /// The last stamp of this stroke: the brush is swept from there to the
    /// next.
    last: Option<End>,
    scratch: TouchScratch,
}

/// The brush's footprint between two stamps: a disc pressed onto the
/// surface and dragged from one to the other.
///
/// Not a ball about each stamp, for two reasons that were both seen on a
/// screen before this was written. A ball lays less on a voxel a layer
/// under the surface than on the one above it, since the one under is
/// further from the middle; neighbouring vertices of the mesh read from
/// different layers, and a stroke came out hatched at the scale of the
/// lattice. And balls a quarter of a radius apart are a string of beads
/// once the stroke is faster than the stamps.
///
/// So how much of the brush a voxel is under is measured ACROSS the
/// surface, along the stamps' own normal, and to the path between the two
/// stamps and not to either one. Every voxel on a line through the band
/// gets the same, and the path is one stripe.
#[derive(Debug, Clone, Copy)]
struct Sweep {
    from: Vec3,
    to: Vec3,
    /// How hard the pen was pressed at each end. Between them it is taken
    /// to have changed evenly, so that a stroke swells and thins along its
    /// length and not in steps a stamp long.
    pressed: (f32, f32),
    along: Vec3,
    /// One over the squared length of the path as seen along the axis, or
    /// nothing for a stamp that has not moved.
    per_length: f32,
    axis: Vec3,
    radius: f32,
}

impl Sweep {
    /// The sweep from `one` stamp to `other`.
    ///
    /// The same sweep whichever of the two is given first, to the bit, and
    /// its mirror image for mirrored stamps, to the bit: the ends are put
    /// in an order that neither a reversal nor a mirror changes before
    /// anything is worked out from them. A stroke walked backwards lays
    /// the paint it lays forwards, and a property test holds it to that.
    fn between(one: End, other: End, radius: f32) -> Self {
        let key = |end: End| end.at.abs().to_array();
        let (first, second) = if key(other) < key(one) { (other, one) } else { (one, other) };
        let axis = (first.facing.normalize_or_zero() + second.facing.normalize_or_zero())
            .normalize_or_zero();
        let path = second.at - first.at;
        let along = path - axis * path.dot(axis);
        let length_squared = along.length_squared();
        let per_length = if length_squared > 0.0 { 1.0 / length_squared } else { 0.0 };
        let pressed = (first.pressure.clamp(0.0, 1.0), second.pressure.clamp(0.0, 1.0));
        // A stamp that has not moved is both its ends at once: pressed as
        // hard as the harder of them.
        let pressed = if length_squared > 0.0 {
            pressed
        } else {
            (pressed.0.max(pressed.1), pressed.0.max(pressed.1))
        };
        Self { from: first.at, to: second.at, pressed, along, per_length, axis, radius }
    }

    /// Whether any voxel of the brick at `coord` can be within reach.
    ///
    /// [`Sweep::reach`] takes a voxel a radius across the surface and a
    /// radius above or below it at most, so nothing further than the
    /// radius times the square root of two from the path is reached. A
    /// brick whose middle is further than that and half its own diagonal
    /// is not. Measured on 2026-11-26: a 10 mm stamp on a ball at a
    /// sixteenth of a millimetre visited 762 bricks.
    fn may_reach_brick(&self, coord: BrickCoord, voxel_size: f32) -> bool {
        let half = crate::brick::BRICK_DIM as f32 * 0.5 * voxel_size;
        let middle = (coord.origin().as_vec3() + Vec3::splat(crate::brick::BRICK_DIM as f32 * 0.5))
            * voxel_size;
        let path = self.to - self.from;
        let length_squared = path.length_squared();
        let t = if length_squared > 0.0 {
            ((middle - self.from).dot(path) / length_squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let nearest = self.from + path * t;
        let reach = self.radius * std::f32::consts::SQRT_2 + half * 3.0_f32.sqrt();
        nearest.distance_squared(middle) <= reach * reach
    }

    /// The box of the world the sweep can reach.
    fn bounds(&self) -> (Vec3, Vec3) {
        let extent = Vec3::splat(self.radius);
        (self.from.min(self.to) - extent, self.from.max(self.to) + extent)
    }

    /// How far `position` is from the path, across the surface, as a share
    /// of the radius: 0 on the path, 1 at the brush's rim. More than 1,
    /// and out of the brush, for a point further than a radius above or
    /// below the path, which is some other part of the model. And how hard
    /// the pen was pressed at the point of the path nearest it.
    #[debug_requires(self.radius > 0.0, "a brush of some size")]
    #[debug_ensures(ret.0 >= 0.0 && (0.0..=1.0).contains(&ret.1), "a distance and a pressure")]
    fn reach(&self, position: Vec3) -> (f32, f32) {
        let offset = position - self.from;
        let across = offset - self.axis * offset.dot(self.axis);
        let t = (across.dot(self.along) * self.per_length).clamp(0.0, 1.0);
        let nearest = self.from + (self.to - self.from) * t;
        let pressed = self.pressed.0 + (self.pressed.1 - self.pressed.0) * t;
        let gap = position - nearest;
        let depth = gap.dot(self.axis);
        if depth.abs() > self.radius {
            return (2.0, pressed);
        }
        ((gap - self.axis * depth).length() / self.radius, pressed)
    }
}

/// One end of a sweep: where a stamp was, which way the surface faced
/// there, and how hard the pen was pressed.
#[derive(Debug, Clone, Copy)]
struct End {
    at: Vec3,
    facing: Vec3,
    pressure: f32,
}

impl End {
    fn of(stamp: &Stamp) -> Self {
        Self { at: stamp.centre, facing: stamp.normal, pressure: stamp.pressure }
    }
}

impl PaintStroke {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new stroke: what the last one covered is forgotten, and this
    /// one lays a coat over it.
    pub fn begin(&mut self) {
        self.coverage.clear();
        self.last = None;
    }

    /// How many bricks this stroke has laid paint in.
    pub fn bricks(&self) -> usize {
        self.coverage.len()
    }

    /// Lay one stamp of the stroke: the brush, swept here from where the
    /// stroke's last stamp was.
    ///
    /// The brush gives the radius, the falloff and the pattern. The mask
    /// holds paint off in proportion: a voxel half protected takes half.
    /// The stamp's pressure scales what it lays.
    pub fn stamp(&mut self, volume: &mut Volume, brush: &Brush, stamp: &Stamp, lay: &Lay) {
        self.stamp_symmetric(volume, brush, stamp, lay, Symmetry::OFF, Vec3::ZERO);
    }

    /// One stamp and its mirrors.
    ///
    /// A mirrored twin asks what it catches at the point its voxel is the
    /// mirror image of, so that the speckle on one side of the plane is the
    /// reflection of the speckle on the other.
    #[debug_requires(
        volume.is_recording(),
        "a stroke of paint is laid inside an undo stroke: that is where what was there is kept"
    )]
    pub fn stamp_symmetric(
        &mut self,
        volume: &mut Volume,
        brush: &Brush,
        stamp: &Stamp,
        lay: &Lay,
        symmetry: Symmetry,
        centre: Vec3,
    ) {
        let here = End::of(stamp);
        // The first stamp of a stroke is swept from itself.
        let from = self.last.unwrap_or(here);
        let own = Mirror { of: stamp, flip: Flip::IDENTITY, from };
        self.stamp_flipped(volume, brush, stamp, own, lay);
        if !symmetry.is_off() {
            let mut twins = [*stamp; Symmetry::MAX_MIRRORS];
            let mut flips = [Flip::IDENTITY; Symmetry::MAX_MIRRORS];
            let count = symmetry.mirrors(stamp, centre, &mut twins);
            let flipped = symmetry.flips(centre, &mut flips);
            debug_assert_eq!(count, flipped, "a stamp has one twin a flip");
            for (twin, flip) in twins[..count].iter().zip(&flips[..count]) {
                // Where the twin is swept from is the mirror image of
                // where the stamp is.
                let from = End {
                    at: from.at * flip.sign + flip.offset,
                    facing: from.facing * flip.sign,
                    ..from
                };
                let mirror = Mirror { of: stamp, flip: *flip, from };
                self.stamp_flipped(volume, brush, twin, mirror, lay);
            }
        }
        self.last = Some(here);
    }

    /// `stamp` is where the paint goes. `mirror` says which stamp it is the
    /// mirror image of, and where it is swept from.
    #[debug_requires(brush.radius.is_finite(), "a brush has a size")]
    #[debug_ensures(
        self.coverage.len() >= old(self.coverage.len()),
        "what a stroke has covered, it has covered"
    )]
    fn stamp_flipped(
        &mut self,
        volume: &mut Volume,
        brush: &Brush,
        stamp: &Stamp,
        mirror: Mirror<'_>,
        lay: &Lay,
    ) {
        let radius = brush.radius;
        if radius <= 0.0 || lay.pigment.opacity <= 0.0 {
            return;
        }
        let Mirror { of: source, flip, from } = mirror;
        // The path here from the last stamp, and the brush set down here
        // as well, square to the surface where it is: on a curved surface
        // the path's own axis is a compromise between its two ends, and
        // each end is owed its own. (It is also what makes a path the same
        // paint walked either way: each stamp is set down once, whichever
        // end the stroke came from.)
        let here = End::of(stamp);
        let sweep = Sweep::between(from, here, radius);
        // A stamp that has not moved or turned is swept from itself, and
        // the sweep is the brush set down already.
        let moved = from.at != here.at || from.facing != here.facing;
        let dab = moved.then(|| Sweep::between(here, here, radius));
        // What this stamp deposits on a voxel the sweep reaches fully, in
        // coats: the distance moved across the surface over what one
        // straight pass deposits on its middle line. See the module
        // documentation.
        let moved_across =
            if sweep.per_length > 0.0 { sweep.per_length.recip().sqrt() } else { 0.0 };
        let per_reach = moved_across / (2.0 * radius * falloff_mean(brush.falloff)) * COAT as f32;
        let Self { coverage, scratch, .. } = self;
        let voxel_size = volume.voxel_size();
        let (low, high) = sweep.bounds();
        let (lo, hi) = volume.voxel_bounds(low, high);

        // What the surface is like under the stamp, for a brush that asks.
        // The surface point a voxel answers for is up to a band away, and
        // the curvature there is read from a block reaching two further.
        let curvatures = if lay.catch == Catch::All {
            None
        } else {
            let reach = IVec3::splat(NARROW_BAND.ceil() as i32 + 3);
            volume.snapshot(lo - reach, hi + reach, &mut scratch.region);
            Some(Curvatures::over(&scratch.region, &mut scratch.cavity))
        };
        let region = &scratch.region;

        let pattern = if brush.pattern.is_off() {
            Prepared::OFF
        } else {
            brush.pattern.prepare(voxel_size, stamp.normal, stamp.tangent)
        };
        let frame = Frame::of(source, lay.origin);
        let falloff = brush.falloff;
        // Which way the brush is travelling over this stamp, for the lee
        // side of a drybrush. A stroke that has not moved has none.
        let travel = stamp.tangent.normalize_or_zero();
        let lay = *lay;
        let ready = Ready::of(lay.pigment);

        // Each brick of the stamp with its own coverage, taken out for the
        // stamp and put back after it, the bricks written across every
        // core (`Volume::edit_paint_across_cores`). A brick's coverage is
        // only made when the stamp has something to lay in it.
        let mut visits: Vec<PaintVisit<Option<Cover>>> =
            bricks_between(BrickCoord::containing(lo).0, BrickCoord::containing(hi).0)
                .filter_map(|coord| {
                    let origin = coord.origin();
                    let (from, to) = (lo.max(origin), hi.min(coord.max_voxel()));
                    // A brick the sweep cannot reach is passed over: the
                    // corners of the box round a stamp are most of it.
                    if !sweep.may_reach_brick(coord, voxel_size) {
                        return None;
                    }
                    (!from.cmpgt(to).any()).then(|| PaintVisit {
                        coord,
                        lo: from,
                        hi: to,
                        state: coverage.remove(&coord),
                    })
                })
                .collect();
        volume.edit_paint_across_cores(
            &mut visits,
            |cover, voxel, index, position, held, before, protection| {
                let free = 1.0 - f32::from(protection) / f32::from(PROTECTED);
                let (across, pressed) = sweep.reach(position);
                let swept = falloff.weight(across) * pressed * free;
                let mut reach = swept;
                if let Some(dab) = &dab {
                    let (across, pressed) = dab.reach(position);
                    reach = reach.max(falloff.weight(across) * pressed * free);
                }
                // Outside the brush, or wholly protected. Only for speed:
                // nothing would be laid here further down either.
                if reach <= 0.0 {
                    return held;
                }
                // What thins the paint apart from the brush: the pattern, and
                // what of the surface the touch catches.
                let mut thinned = pattern.weight(position);
                if let Some(curvatures) = &curvatures {
                    // One step down the gradient to the surface this voxel
                    // is paint for, as a gated slot stamp takes it.
                    let distance = region.get(voxel);
                    let normal = gradient_at(region, voxel).normalize_or_zero();
                    let on_surface = voxel.as_vec3() - normal * distance;
                    let relief = curvatures.relief(on_surface, lay.crease_gain);
                    let world = position - normal * (distance * voxel_size);
                    let in_source = (world - flip.offset) * flip.sign;
                    thinned *= match lay.catch {
                        Catch::All => 1.0,
                        Catch::Recesses => pooled(relief),
                        Catch::Raised(dry) => {
                            let facing = 0.5 - 0.5 * normal.dot(travel);
                            let bristle = frame.bristle(in_source, dry, lay.seed);
                            dry.amount(relief.max(0.0), facing, bristle, lay.load)
                        }
                        Catch::Specks(sponge) => sponge.amount(
                            relief,
                            normal.dot(sponge.light),
                            Frame::speck(in_source, sponge, lay.seed),
                        ),
                    };
                }
                let thinned = thinned.clamp(0.0, 1.0);
                let most = ((reach * thinned).clamp(0.0, 1.0) * 255.0).round() as u8;
                // Whole numbers, so that the sum is the same in any order.
                // Only the sweep deposits, not the dab at its end, which is
                // at the other end when the stroke is walked the other way.
                let deposit = (swept * thinned * per_reach).round() as u32;
                if most == 0 && deposit == 0 {
                    return held;
                }
                let cover = cover.get_or_insert_with(Cover::nothing);
                let was = cover.at(index);
                cover.most[index] = cover.most[index].max(most);
                cover.deposited[index] =
                    (u32::from(cover.deposited[index]) + deposit).min(COAT) as u16;
                let wanted = cover.at(index);
                // Only ever more: a voxel a stamp has covered is not
                // uncovered by a later one that gives it less.
                if wanted <= was {
                    return held;
                }
                ready.over(before, f32::from(wanted) / 255.0)
            },
        );
        for visit in visits {
            if let Some(cover) = visit.state {
                coverage.insert(visit.coord, cover);
            }
        }
    }
}

/// Which stamp a stamp is the mirror image of, through which flip, and
/// where it is swept from: the last stamp's place and facing, mirrored the
/// same way. A stamp that is nobody's twin is its own, through no flip.
#[derive(Debug, Clone, Copy)]
struct Mirror<'a> {
    of: &'a Stamp,
    flip: Flip,
    from: End,
}

/// One coat deposited, in [`Cover::deposited`]'s units. Whole numbers, so
/// that what is deposited adds up the same in any order: a stroke walked
/// backwards lays the paint it lays forwards, to the bit.
const COAT: u32 = 1 << 15;

/// What a stroke has given the voxels of one brick. See the module
/// documentation.
#[derive(Debug, Clone)]
struct Cover {
    /// The most any one stamp gave, 0..=255.
    most: Box<[u8; BRICK_VOXELS]>,
    /// What the stroke has deposited, [`COAT`] being one coat, held there.
    deposited: Box<[u16; BRICK_VOXELS]>,
}

impl Cover {
    fn nothing() -> Self {
        Self {
            most: vec![0_u8; BRICK_VOXELS].into_boxed_slice().try_into().expect("BRICK_VOXELS"),
            deposited: vec![0_u16; BRICK_VOXELS]
                .into_boxed_slice()
                .try_into()
                .expect("BRICK_VOXELS"),
        }
    }

    /// The coverage at `index`, 0..=255: the larger of the most and what
    /// was deposited, held to a coat.
    fn at(&self, index: usize) -> u8 {
        let deposited = (u32::from(self.deposited[index]) * 255 + COAT / 2) / COAT;
        self.most[index].max(deposited.min(255) as u8)
    }
}

/// The mean of a falloff over the brush's radius: what a straight pass
/// deposits on its middle line is this times the brush's width.
fn falloff_mean(falloff: crate::brush::FalloffCurve) -> f32 {
    const SAMPLES: usize = 64;
    let sum: f32 = (0..SAMPLES).map(|at| falloff.weight((at as f32 + 0.5) / SAMPLES as f32)).sum();
    (sum / SAMPLES as f32).max(1e-3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{BrushDirection, BrushKind, FalloffCurve};
    use crate::paint::field::{BARE, PaintField};

    const VOXEL: f32 = 0.5;
    const HALF: f32 = 12.0;
    const RED: [u8; 3] = [200, 30, 20];
    const GREEN: [u8; 3] = [30, 160, 40];

    /// The middle of the cube's top face, and the middle of the edge where
    /// the +x and +y faces meet.
    const ON_FLAT: Vec3 = Vec3::new(0.0, 0.0, HALF);
    const ON_EDGE: Vec3 = Vec3::new(HALF, HALF, 0.0);

    fn cube_distance(position: Vec3) -> f32 {
        let outside = position.abs() - Vec3::splat(HALF);
        outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0)
    }

    /// A cube of side 24 mm about the origin: six flats, twelve sharp edges.
    fn cube() -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(HALF + NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            (cube_distance(position) / VOXEL).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        volume
    }

    /// How deep the slot is and how far it is from its middle to a wall.
    const SLOT_DEPTH: f32 = 4.0;
    const SLOT_HALF: f32 = 3.0;

    /// The cube with a square slot cut along y across its top face: two
    /// creases along the foot of the slot's walls, two edges along their
    /// tops, and flats between.
    fn slotted() -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(HALF + NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            let slot = (position.x.abs() - SLOT_HALF).max(HALF - SLOT_DEPTH - position.z);
            let distance = cube_distance(position).max(-slot);
            (distance / VOXEL).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        volume
    }

    fn brush(radius: f32) -> Brush {
        Brush { kind: BrushKind::Paint, radius, ..Brush::default() }
    }

    /// A pass of the paint brush fades to nothing at its radius, with no
    /// ring: asked after by the maintainer on 2026-11-27, of the paint and
    /// mask brushes. Run then: at radius 8, half a millimetre a voxel, it
    /// was [255, 255, 251, 237, 218, 196, 174, 151, 128, 104, 81, 59, 40,
    /// 24, 11, 3, 0, 0] from the middle out. The mask's ring was the
    /// mask's own (`mask_stroke.rs`).
    #[test]
    fn a_pass_fades_to_nothing_at_its_radius() {
        for radius in [4.0_f32, 8.0] {
            let brush = brush(radius);
            let red = pigment(RED, 1.0);
            let mut volume = cube();
            let mut stroke = PaintStroke::new();
            volume.begin_stroke();
            stroke.begin();
            let mut x = -8.0;
            while x <= 8.0 {
                stroke.stamp(
                    &mut volume,
                    &brush,
                    &on(ON_FLAT + Vec3::X * x, Vec3::Z),
                    &Lay::loaded(red),
                );
                x += 0.25;
            }
            let row: Vec<u8> = (0..=(radius / VOXEL) as i32 + 1)
                .map(|y| volume.paint().at(IVec3::new(0, y, (HALF / VOXEL) as i32))[3])
                .collect();
            let last = (radius / VOXEL) as usize;
            assert_eq!(row[0], 255, "{row:?}");
            assert!(row.windows(2).all(|pair| pair[0] >= pair[1]), "{row:?}");
            // No step anywhere across it bigger than the falloff's own
            // steepest, a voxel's worth of it, and a little for the sum.
            let steepest = 1.5 * VOXEL / radius * 255.0 * 1.2;
            assert!(row.windows(2).all(|pair| f32::from(pair[0] - pair[1]) <= steepest), "{row:?}");
            assert!(row[last - 1] <= 12 && row[last] == 0, "{row:?}");
        }
    }

    fn pigment(colour: [u8; 3], opacity: f32) -> Pigment {
        Pigment { colour, opacity, mode: Mode::Paint, ..Pigment::default() }
    }

    fn on(centre: Vec3, normal: Vec3) -> Stamp {
        Stamp::new(centre, normal, BrushDirection::Add)
    }

    /// One stamp, as a stroke of its own.
    fn dab(volume: &mut Volume, brush: &Brush, stamp: &Stamp, lay: &Lay) {
        dab_mirrored(volume, brush, stamp, lay, Symmetry::OFF);
    }

    fn dab_mirrored(
        volume: &mut Volume,
        brush: &Brush,
        stamp: &Stamp,
        lay: &Lay,
        symmetry: Symmetry,
    ) {
        volume.begin_stroke();
        PaintStroke::new().stamp_symmetric(volume, brush, stamp, lay, symmetry, Vec3::ZERO);
        let _ = volume.end_stroke();
    }

    fn lay(pigment: Pigment, catch: Catch) -> Lay {
        Lay { catch, seed: 7, ..Lay::loaded(pigment) }
    }

    /// Every paintable voxel within `radius` of `centre`, with where it is
    /// and its paint.
    fn near(volume: &Volume, centre: Vec3, radius: f32) -> Vec<(IVec3, Vec3, Rgba)> {
        let (lo, hi) =
            volume.voxel_bounds(centre - Vec3::splat(radius), centre + Vec3::splat(radius));
        let mut out = Vec::new();
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = IVec3::new(x, y, z);
                    let position = voxel.as_vec3() * VOXEL;
                    if (position - centre).length() < radius
                        && PaintField::paintable(volume.sample_voxel(voxel))
                    {
                        out.push((voxel, position, volume.paint().at(voxel)));
                    }
                }
            }
        }
        out
    }

    fn mixed(under: [u8; 3], over: [u8; 3], amount: f32) -> [u8; 3] {
        let (under, over) = (linear(under), linear(over));
        srgb(std::array::from_fn(|c| under[c] * (1.0 - amount) + over[c] * amount))
    }

    /// Nothing laid is what was there, to the bit, and a colour comes back
    /// from linear light as the byte it went in as: a stroke that passes
    /// over paint without laying anything on it must not walk its colour.
    #[test]
    fn nothing_laid_is_what_was_there() {
        for byte in 0..=u8::MAX {
            assert_eq!(srgb(linear([byte; 3])), [byte; 3], "byte {byte} does not survive");
            let before = [byte, 255 - byte, byte / 2, 200];
            for mode in [Mode::Paint, Mode::Glaze, Mode::Erase] {
                let brush = Pigment { colour: RED, opacity: 1.0, mode, ..Pigment::default() };
                assert_eq!(laid(before, brush, 0.0), before);
                assert_eq!(laid(before, Pigment { opacity: 0.0, ..brush }, 1.0), before);
                assert_eq!(laid(BARE, brush, 0.0), BARE, "nothing over nothing, {mode:?}");
            }
            // Half of it taken off again: the colour is kept as it was.
            let erased = laid(
                before,
                Pigment { colour: RED, opacity: 1.0, mode: Mode::Erase, ..Pigment::default() },
                0.5,
            );
            assert_eq!(erased, [byte, 255 - byte, byte / 2, 100]);
        }
    }

    /// The tables are the curves: every byte decodes as [`linear`] decodes
    /// it, and every value encodes as [`srgb`] encodes it, at each level's
    /// own threshold, a step either side of it, and at a few hundred
    /// thousand values between.
    #[test]
    fn the_tables_are_the_curves_they_stand_in_for() {
        for byte in 0..=u8::MAX {
            assert_eq!(light(byte).to_bits(), linear([byte; 3])[0].to_bits(), "byte {byte}");
        }
        let same = |c: f32| assert_eq!(encode([c; 3]), srgb([c; 3]), "the value {c:e}");
        let table = Encode::new();
        for level in 1..=255 {
            let least = table.least[level];
            assert!(least > table.least[level - 1], "level {level} begins at {least}");
            for bits in least.to_bits() - 2..=least.to_bits() + 2 {
                same(f32::from_bits(bits));
            }
            assert_eq!(usize::from(srgb([least; 3])[0]), level);
            assert_eq!(usize::from(srgb([f32::from_bits(least.to_bits() - 1); 3])[0]), level - 1);
        }
        for step in 0..=400_000 {
            same(step as f32 / 400_000.0);
            // Thick where the curve is steep.
            same(step as f32 / 400_000.0 * 0.01);
        }
        for odd in [-1.0, 0.0, 1.0, 2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            same(odd);
        }
    }

    /// Paint covers by its amount, in linear light.
    #[test]
    fn paint_covers_by_its_amount_in_linear_light() {
        let red = pigment(RED, 1.0);
        let green = [GREEN[0], GREEN[1], GREEN[2], 255];
        // All of it, over anything, is the colour.
        for before in [BARE, green, [9, 9, 9, 40]] {
            assert_eq!(laid(before, red, 1.0), [200, 30, 20, 255], "over {before:?}");
        }
        // Half of it over bare clay is the colour at half coverage.
        assert_eq!(laid(BARE, red, 0.5), [200, 30, 20, 128]);
        // Half of it over a full coat of green is half way between the two
        // as light, which is not half way between their bytes.
        let [r, g, b] = mixed(GREEN, RED, 0.5);
        assert_eq!(laid(green, red, 0.5), [r, g, b, 255]);
        let averaged = [115, 95, 30];
        assert!(
            (0..3).any(|c| (i32::from([r, g, b][c]) - averaged[c]).abs() > 20),
            "the fixture cannot tell linear light from bytes: {:?}",
            [r, g, b]
        );
        // Over half a coat, what shows of the green is what the red left
        // uncovered of it: the coverage adds up as two films do.
        let over_half = laid([GREEN[0], GREEN[1], GREEN[2], 128], red, 0.5);
        assert_eq!(over_half[3], 192, "half over half is three quarters");
        let share = 0.5 / (0.5 + (128.0 / 255.0) * 0.5);
        let [r, g, b] = mixed(GREEN, RED, share);
        assert_eq!(over_half[..3], [r, g, b]);
        // The opacity is a scale on the amount and nothing else, and the
        // two together are kept to what a coat can be: no more than all of
        // it, and no less than none.
        for before in [BARE, green] {
            assert_eq!(laid(before, pigment(RED, 0.5), 1.0), laid(before, red, 0.5));
            assert_eq!(laid(before, pigment(RED, 3.0), 1.0), [200, 30, 20, 255]);
            assert_eq!(laid(before, red, -0.5), before);
        }
    }

    /// A glaze shows its hue over what is painted without taking it dark,
    /// and over bare clay leaves a thin coat of the clay so tinted; an
    /// eraser takes paint off and leaves its colour.
    #[test]
    fn a_glaze_tints_and_an_eraser_takes_off() {
        let glaze = Pigment { colour: RED, opacity: 1.0, mode: Mode::Glaze, ..Pigment::default() };
        let encoded = |c: [f32; 3]| srgb(c);
        let red = linear(RED);
        let lightness = |c: [u8; 3]| luma(linear(c));
        // Over white it is a light red, and what was covered stays covered.
        let [r, g, b] = encoded(tinted([1.0; 3], red));
        assert_eq!(laid([255, 255, 255, 255], glaze, 1.0), [r, g, b, 255]);
        assert!(r == 255 && g.max(b) < 230 && g > RED[1], "red over white is {:?}", [r, g, b]);
        // Over a mid grey, red, and a good deal lighter than multiplying
        // left it ((88, 9, 9) in `tinted`).
        let over_grey = laid([128, 128, 128, 255], glaze, 1.0);
        assert!(over_grey[0] > over_grey[1].max(over_grey[2]) + 40, "{over_grey:?}");
        let kept = lightness([over_grey[0], over_grey[1], over_grey[2]]) / lightness([128; 3]);
        assert!(kept > 0.5, "the grey's lightness went to {kept} of itself");
        // A dark blue over white is a pale blue, and not the cyan that
        // giving it white's lightness channel by channel made.
        let blue = [13, 61, 131];
        let over_white = encoded(tinted([1.0; 3], linear(blue)));
        assert!(over_white[2] > over_white[1] && over_white[1] > over_white[0], "{over_white:?}");
        assert!(over_white[2] - over_white[0] > 40, "{over_white:?}");
        // Over a green, redder and less green, and never lighter.
        let over_green = laid([GREEN[0], GREEN[1], GREEN[2], 255], glaze, 1.0);
        let [r, g, b] = encoded(tinted(linear(GREEN), red));
        assert_eq!(over_green, [r, g, b, 255]);
        assert!(over_green[0] > GREEN[0] && over_green[1] < GREEN[1], "{over_green:?}");
        assert!(lightness([r, g, b]) <= lightness(GREEN) + 1.0e-3);
        // Half a glaze over white is half way to the whole one, in light.
        let whole = tinted([1.0; 3], red);
        let [r, g, b] = encoded(std::array::from_fn(|c| 0.5 + 0.5 * whole[c]));
        assert_eq!(laid([255, 255, 255, 255], glaze, 0.5), [r, g, b, 255]);
        // Over bare clay, a thin coat of the clay tinted red.
        let [r, g, b] = encoded(tinted(linear(crate::paint::layers::CLAY), red));
        assert_eq!(laid(BARE, glaze, 1.0), [r, g, b, (GLAZE_BODY * 255.0).round() as u8]);
        assert_eq!(GLAZE_BODY, 0.5);

        let eraser = Pigment { colour: RED, opacity: 1.0, mode: Mode::Erase, ..Pigment::default() };
        assert_eq!(laid([GREEN[0], GREEN[1], GREEN[2], 255], eraser, 1.0), BARE);
        assert_eq!(laid(BARE, eraser, 1.0), BARE);
        assert_eq!(laid([30, 160, 40, 200], eraser, 0.25), [30, 160, 40, 150]);
    }

    /// The glaze setting: nought is the whole film, one keeps the lightness
    /// under it, and between is between, never lighter for keeping less.
    #[test]
    fn a_glaze_keeps_as_much_light_as_it_is_set_to() {
        let lightness = |c: [u8; 4]| luma(linear([c[0], c[1], c[2]]));
        let under = [180, 170, 150, 255];
        let laid_keeping = |keeps: f32| {
            let glaze = Pigment { colour: RED, opacity: 1.0, mode: Mode::Glaze, glaze: keeps };
            laid(under, glaze, 1.0)
        };
        let whole = laid_keeping(0.0);
        let all = laid_keeping(1.0);
        let film = linear(RED);
        let brightest = film[0].max(film[1]).max(film[2]);
        let filtered: [f32; 3] = std::array::from_fn(|c| {
            linear([under[0], under[1], under[2]])[c] * film[c] / brightest
        });
        assert!((lightness(whole) - luma(filtered)).abs() < 0.01, "{whole:?}");
        assert!((lightness(all) - lightness(under)).abs() < 0.01, "{all:?}");
        let mut last = 0.0;
        for step in 0..=10 {
            let now = lightness(laid_keeping(step as f32 / 10.0));
            assert!(now + 1.0e-3 >= last, "keeping {step} tenths went darker");
            last = now;
        }
        // Red at every setting: the hue is the glaze's.
        for keeps in [0.0, 0.5, 1.0] {
            let [r, g, b, _] = laid_keeping(keeps);
            assert!(r > g && r > b, "{keeps}: {:?}", [r, g, b]);
        }
        assert_eq!(Pigment::default().glaze, GLAZE_KEEPS);
    }

    /// A stroke lays one coat: each voxel takes the most any stamp gave it,
    /// over what it held before the stroke, however many stamps cross it.
    /// A click lays the brush's falloff, and a stamp twice over in the same
    /// place lays it once. A pass lays a coat down its middle and never
    /// more; the brush is measured across the surface. Going back over a
    /// pass in the same stroke is no thinner and no more than a coat, and a
    /// second stroke lays a second coat.
    #[test]
    fn a_stroke_lays_one_coat_however_many_stamps_cross_a_voxel() {
        let brush = brush(4.0);
        let half_red = pigment(RED, 0.5);
        let stamps: Vec<Stamp> =
            (0..=12).map(|step| on(ON_FLAT + Vec3::X * (step as f32 * 0.5), Vec3::Z)).collect();

        for under in [BARE, [GREEN[0], GREEN[1], GREEN[2], 255]] {
            let coat = laid(under, half_red, 1.0);

            // A click: the falloff, to the bit, measured across the face.
            let mut volume = cube();
            volume.edit_paint(IVec3::splat(-40), IVec3::splat(40), |_, _, _, _| under);
            let mut stroke = PaintStroke::new();
            volume.begin_stroke();
            stroke.begin();
            stroke.stamp(&mut volume, &brush, &stamps[0], &Lay::loaded(half_red));
            stroke.stamp(&mut volume, &brush, &stamps[0], &Lay::loaded(half_red));
            for (voxel, position, paint) in near(&volume, ON_FLAT, 5.0) {
                let across = Vec3::new(position.x - ON_FLAT.x, position.y, 0.0).length();
                let coverage = (brush.falloff.weight(across / 4.0) * 255.0).round() as u8;
                assert_eq!(paint, laid(under, half_red, f32::from(coverage) / 255.0), "{voxel}");
            }
            drop(volume.end_stroke());

            let mut volume = cube();
            volume.edit_paint(IVec3::splat(-40), IVec3::splat(40), |_, _, _, _| under);
            let mut stroke = PaintStroke::new();
            volume.begin_stroke();
            stroke.begin();
            for stamp in &stamps {
                // Each stamp twice over, which is a stamp too many.
                stroke.stamp(&mut volume, &brush, stamp, &Lay::loaded(half_red));
                stroke.stamp(&mut volume, &brush, stamp, &Lay::loaded(half_red));
            }
            let after = near(&volume, ON_FLAT + Vec3::X * 3.0, 9.0);
            let (mut soft, mut full) = (0, 0);
            for (voxel, position, paint) in &after {
                // A coat down the middle of the pass, and nothing past the
                // brush's rim.
                let along = position.x.clamp(0.0, 6.0);
                let across = Vec3::new(position.x - along, position.y, 0.0).length();
                if across < 0.5 && (1.0..5.0).contains(&position.x) {
                    assert_eq!(*paint, coat, "{voxel} on the middle line");
                }
                if across > 4.0 {
                    assert_eq!(*paint, under, "{voxel} past the rim");
                }
                assert!(paint[3] <= coat[3], "{voxel}: more than a coat");
                let laid_here = *paint != under;
                soft += usize::from(laid_here && *paint != coat);
                full += usize::from(*paint == coat);
            }
            // Controls: the brush has a soft edge, and a middle that is not.
            assert!(soft > 500 && full > 5, "{soft} voxels part covered and {full} wholly");
            // And through the band a voxel holds what the one on the
            // surface holds, to the bit: the brush is measured across the
            // surface, and a voxel under it is no further from it.
            for (voxel, _, paint) in &after {
                let on_the_face = IVec3::new(voxel.x, voxel.y, (HALF / VOXEL) as i32);
                assert_eq!(*paint, volume.paint().at(on_the_face), "{voxel} and the face over it");
            }
            assert!(stroke.bricks() > 0);

            // The same stamps again, still in the stroke: thicker on the
            // soft edge, and the middle still a coat.
            for stamp in stamps.iter().rev() {
                stroke.stamp(&mut volume, &brush, stamp, &Lay::loaded(half_red));
            }
            let again = near(&volume, ON_FLAT + Vec3::X * 3.0, 9.0);
            let mut thicker = 0;
            for ((voxel, _, was), (_, _, now)) in after.iter().zip(&again) {
                assert!(now[3] >= was[3] && now[3] <= coat[3], "{voxel}: {was:?} then {now:?}");
                if *was == coat {
                    assert_eq!(*now, coat, "{voxel}");
                }
                thicker += usize::from(now != was);
            }
            assert!(thicker > 100, "going back added to {thicker} voxels");
            let edit = volume.end_stroke().expect("a stroke that painted");

            // A second stroke over the first lays a second coat.
            volume.begin_stroke();
            stroke.begin();
            stroke.stamp(&mut volume, &brush, &stamps[0], &Lay::loaded(half_red));
            let twice = volume.paint().at((ON_FLAT / VOXEL).as_ivec3());
            assert_eq!(twice, laid(laid(under, half_red, 1.0), half_red, 1.0));
            assert_ne!(twice, laid(under, half_red, 1.0));
            let second = volume.end_stroke().expect("a second stroke");

            // And each is one undo, back to what was there.
            volume.apply_edit(second);
            assert_eq!(near(&volume, ON_FLAT + Vec3::X * 3.0, 9.0), again);
            volume.apply_edit(edit);
            for (voxel, _, paint) in near(&volume, ON_FLAT + Vec3::X * 3.0, 9.0) {
                assert_eq!(paint, under, "{voxel} after the undo");
            }
        }
    }

    /// A pass and a return beside it in one stroke, three quarters of a
    /// radius apart: the line between them is a coat, as their middles
    /// are, and not the paler seam the most of two soft edges left (seen
    /// by the maintainer, 2026-11-25). The same for the inside of a turn.
    #[test]
    fn a_return_pass_beside_the_first_leaves_no_paler_seam() {
        let brush = brush(4.0);
        let half_red = pigment(RED, 0.5);
        let coat = laid(BARE, half_red, 1.0);
        let gap = 3.0;
        let out = (0..=24).map(|step| on(ON_FLAT + Vec3::X * (step as f32 * 0.5), Vec3::Z));
        let back = (0..=24)
            .rev()
            .map(|step| on(ON_FLAT + Vec3::new(step as f32 * 0.5, gap, 0.0), Vec3::Z));
        let mut volume = cube();
        let mut stroke = PaintStroke::new();
        volume.begin_stroke();
        stroke.begin();
        for stamp in out.chain(back) {
            stroke.stamp(&mut volume, &brush, &stamp, &Lay::loaded(half_red));
        }
        let face = (HALF / VOXEL) as i32;
        let at = |x: f32, y: f32| {
            volume.paint().at(IVec3::new((x / VOXEL) as i32, (y / VOXEL) as i32, face))
        };
        for x in [4.0, 6.0, 8.0] {
            assert_eq!(at(x, 0.0), coat, "the first pass's middle at {x}");
            assert_eq!(at(x, gap), coat, "the return's middle at {x}");
            assert_eq!(at(x, gap / 2.0), coat, "the seam between them at {x}");
        }
        drop(volume.end_stroke());
    }

    /// The mask holds paint off in proportion, the pen's pressure scales
    /// what a stamp lays, a pattern thins it, and a brush of no size or a
    /// pigment of no opacity lays nothing.
    #[test]
    fn the_mask_the_pressure_and_the_pattern_each_scale_what_is_laid() {
        let brush = brush(5.0);
        let red = pigment(RED, 1.0);
        let stamp = on(ON_FLAT, Vec3::Z);
        let stamped = |brush: &Brush, stamp: &Stamp, prepare: &dyn Fn(&mut Volume)| {
            let mut volume = cube();
            prepare(&mut volume);
            dab(&mut volume, brush, stamp, &Lay::loaded(red));
            volume
        };
        // Across the face, as the brush is measured.
        let weight =
            |position: Vec3| brush.falloff.weight((position - ON_FLAT).truncate().length() / 5.0);
        let alpha = |amount: f32| (((amount * 255.0).round() / 255.0) * 255.0).round() as u8;

        // A ramp of protection across the left half of the stamp, and the
        // right half protected wholly.
        let protection =
            |position: Vec3| ((position.x + 5.0) / 5.0 * 255.0).clamp(0.0, 255.0) as u8;
        let masked = stamped(&brush, &stamp, &|volume| {
            volume.edit_mask(IVec3::splat(-40), IVec3::splat(40), |_, position, _| {
                protection(position)
            });
        });
        let (mut held_off, mut part) = (0, 0);
        for (voxel, position, paint) in near(&masked, ON_FLAT, 6.0) {
            let free = 1.0 - f32::from(protection(position)) / 255.0;
            assert_eq!(paint[3], alpha(weight(position) * free), "{voxel}");
            held_off += usize::from(weight(position) > 0.5 && paint == BARE);
            part += usize::from(free > 0.3 && free < 0.7 && paint[3] > 20);
        }
        assert!(held_off > 5 && part > 100, "{held_off} held off wholly and {part} in part");

        // Half the pressure, half the paint.
        let light = stamped(&brush, &stamp.with_pressure(0.5), &|_| {});
        let mut judged = 0;
        for (voxel, position, paint) in near(&light, ON_FLAT, 6.0) {
            assert_eq!(paint[3], alpha(weight(position) * 0.5), "{voxel}");
            judged += usize::from(paint[3] > 100);
        }
        assert!(judged > 50, "only {judged} voxels near the middle of a light stamp");

        // A pattern takes some of it off, somewhere under the brush.
        let pattern = crate::pattern::Pattern {
            kind: crate::pattern::PatternKind::Scales,
            scale_mm: 3.0,
            depth: 1.0,
        };
        let plain = stamped(&brush, &stamp, &|_| {});
        let scaled = stamped(&Brush { pattern, ..brush }, &stamp.with_tangent(Vec3::X), &|_| {});
        let (mut less, mut same) = (0, 0);
        for ((voxel, _, with), (_, _, without)) in
            near(&scaled, ON_FLAT, 6.0).into_iter().zip(near(&plain, ON_FLAT, 6.0))
        {
            assert!(with[3] <= without[3], "{voxel}: a pattern added paint");
            less += usize::from(with[3] + 40 < without[3]);
            same += usize::from(with[3] == without[3] && with[3] > 100);
        }
        assert!(less > 100 && same > 5, "{less} voxels thinned and {same} left alone");

        // Nothing to lay it with, nothing to lay, or nowhere it may go: no
        // paint, and nothing kept of the stamp either.
        assert!(stamped(&Brush { radius: 0.0, ..brush }, &stamp, &|_| {}).paint().is_empty());
        let mut volume = cube();
        let mut stroke = PaintStroke::new();
        volume.begin_stroke();
        stroke.stamp(&mut volume, &brush, &stamp, &Lay::loaded(pigment(RED, 0.0)));
        assert!(volume.paint().is_empty() && stroke.bricks() == 0);
        volume.edit_mask(IVec3::splat(-40), IVec3::splat(40), |_, _, _| PROTECTED);
        stroke.stamp(&mut volume, &brush, &stamp, &Lay::loaded(red));
        assert!(volume.paint().is_empty() && stroke.bricks() == 0);
        // Control: a stamp that does lay something is kept count of.
        let mut volume = cube();
        volume.begin_stroke();
        stroke.stamp(&mut volume, &brush, &stamp, &Lay::loaded(red));
        assert!(stroke.bricks() > 0);
        // And it is paint and not sculpture: the field is as it was.
        let fresh = cube();
        for (voxel, _, _) in near(&plain, ON_FLAT, 6.0) {
            assert_eq!(plain.sample_voxel(voxel).to_bits(), fresh.sample_voxel(voxel).to_bits());
        }
    }

    /// A wash runs into the recesses: all of it into a crease, some of it
    /// on a flat, none of it on an edge.
    #[test]
    fn a_wash_pools_in_the_creases_and_runs_off_the_edges() {
        assert_eq!(pooled(-1.0), 1.0);
        assert_eq!(pooled(0.0), WASH_FLAT);
        assert_eq!(pooled(1.0), 0.0);
        assert!(pooled(-0.5) > pooled(0.0) && pooled(0.3) < pooled(0.0));

        // Over the slot: wide and flat-topped, so that what differs from
        // place to place is the wash and not the brush's own edge.
        let wide = Brush { falloff: FalloffCurve::Wide, ..brush(9.0) };
        let centre = Vec3::new(0.0, 0.0, HALF - SLOT_DEPTH * 0.5);
        let stamp = on(centre, Vec3::Z);
        let wash =
            Pigment { colour: [40, 30, 20], opacity: 1.0, mode: Mode::Glaze, ..Pigment::default() };
        let mut washed = slotted();
        dab(&mut washed, &wide, &stamp, &lay(wash, Catch::Recesses));
        let mut loaded = slotted();
        dab(&mut loaded, &wide, &stamp, &Lay::loaded(wash));

        // The share of a loaded brush's paint that the wash left, by where
        // on the slot a voxel is. Near the middle of the stamp only, in y.
        let foot =
            |at: Vec3| Vec3::new(at.x.abs() - SLOT_HALF, 0.0, at.z - (HALF - SLOT_DEPTH)).length();
        let lip = |at: Vec3| Vec3::new(at.x.abs() - SLOT_HALF, 0.0, at.z - HALF).length();
        let share = |inside: &dyn Fn(Vec3) -> bool| {
            let (mut wash_sum, mut full_sum, mut count) = (0.0, 0.0, 0);
            for ((_, at, with), (_, _, without)) in
                near(&washed, centre, 8.0).into_iter().zip(near(&loaded, centre, 8.0))
            {
                assert!(with[3] <= without[3], "a wash laid more than a loaded brush at {at}");
                if at.y.abs() < 5.0 && inside(at) {
                    wash_sum += f32::from(with[3]);
                    full_sum += f32::from(without[3]);
                    count += 1;
                }
            }
            assert!(count > 30 && full_sum > 0.0, "only {count} voxels to judge by");
            wash_sum / full_sum
        };
        let crease = share(&|at| foot(at) < VOXEL);
        let floor = share(&|at| at.x.abs() < 1.0 && at.z < HALF - 2.0);
        let edge = share(&|at| lip(at) < VOXEL);
        assert!(crease > 0.8, "a crease kept {crease} of the wash");
        assert!((floor - WASH_FLAT).abs() < 0.05, "a flat kept {floor} of the wash");
        assert!(edge < 0.1, "an edge kept {edge} of the wash");

        // Through the band, a voxel takes what the surface under it takes
        // and not what the field happens to be like where it is: over the
        // middle of the slot's floor, every voxel above and below the one
        // on the surface holds what that one holds. (A wash alone would not
        // show this on a flat. The slot's walls are near enough to curve
        // the field above the floor, and not the floor.)
        let floor_z = ((HALF - SLOT_DEPTH) / VOXEL) as i32;
        let mut columns = 0;
        for x in -2..=2 {
            for y in -6..=6 {
                let on_it = washed.paint().at(IVec3::new(x, y, floor_z));
                assert_ne!(on_it, BARE);
                for dz in [-2, -1, 1, 2] {
                    let off_it = washed.paint().at(IVec3::new(x, y, floor_z + dz));
                    assert!(
                        (i32::from(off_it[3]) - i32::from(on_it[3])).abs() <= 3,
                        "{dz} off the floor at {x} {y}: {off_it:?} against {on_it:?}"
                    );
                }
                columns += 1;
            }
        }
        assert!(columns > 50);

        // What a full crease is, is the view's to say: with no gain there
        // is no crease anywhere, and the wash lies as it does on a flat.
        let mut level = slotted();
        let flat_everywhere = Lay { crease_gain: 0.0, ..lay(wash, Catch::Recesses) };
        dab(&mut level, &wide, &stamp, &flat_everywhere);
        for ((_, at, with), (_, _, without)) in
            near(&level, centre, 8.0).into_iter().zip(near(&loaded, centre, 8.0))
        {
            let kept = f32::from(with[3]) / f32::from(without[3].max(1));
            assert!(
                without[3] < 40 || (kept - WASH_FLAT).abs() < 0.03,
                "with no gain the wash kept {kept} at {at}"
            );
        }
    }

    /// A drybrush lays paint on what stands up, most on the sharpest of it,
    /// and none on the faces; a sponge lays its density of a flat in specks
    /// with soft rims.
    #[test]
    fn a_drybrush_catches_edges_and_a_sponge_lays_specks() {
        let wide = Brush { falloff: FalloffCurve::Wide, ..brush(8.0) };
        let red = pigment(RED, 1.0);
        let from_an_edge = |position: Vec3| {
            let mut gaps = (Vec3::splat(HALF) - position.abs()).abs().to_array();
            gaps.sort_by(f32::total_cmp);
            gaps[1]
        };

        let dry = Drybrush { catch: 0.7, streak: 0.0, bristle_mm: 0.4 };
        let stamp = on(ON_EDGE, Vec3::new(1.0, 1.0, 0.0).normalize());
        let mut volume = cube();
        dab(&mut volume, &wide, &stamp, &lay(red, Catch::Raised(dry)));
        let voxels = near(&volume, ON_EDGE, 6.0);
        let on_edge: Vec<_> =
            voxels.iter().filter(|(_, at, _)| from_an_edge(*at) <= VOXEL * 0.75).collect();
        let on_face: Vec<_> =
            voxels.iter().filter(|(_, at, _)| from_an_edge(*at) >= VOXEL * 3.0).collect();
        assert!(on_edge.len() > 20 && on_face.len() > 500, "{} {}", on_edge.len(), on_face.len());
        let caught = on_edge.iter().filter(|(_, _, paint)| paint[3] > 128).count();
        assert!(caught * 10 > on_edge.len() * 8, "{caught} of {} on the edge", on_edge.len());
        let stray = on_face.iter().filter(|(_, _, paint)| *paint != BARE).count();
        assert_eq!(stray, 0, "{stray} voxels of the flat faces took a drybrush");
        // Off the edge and still in its band, a voxel is paint for the edge
        // and takes what the edge takes: the edge is what is nearest it,
        // though the field out there is curved a good deal less.
        let mut judged = 0;
        for z in -4..=4 {
            let on_it = volume.paint().at(IVec3::new(24, 24, z));
            let off_it = volume.paint().at(IVec3::new(26, 26, z));
            assert!(on_it[3] > 128, "the edge itself took {on_it:?} at z = {z}");
            assert!(
                i32::from(off_it[3]) + 40 > i32::from(on_it[3]),
                "off the edge {off_it:?}, on it {on_it:?}"
            );
            judged += 1;
        }
        assert_eq!(judged, 9);
        // Nothing stands up out of the middle of a face: no paint, and
        // nothing kept of the stamp.
        let mut face = cube();
        let mut stroke = PaintStroke::new();
        face.begin_stroke();
        stroke.stamp(&mut face, &wide, &on(ON_FLAT, Vec3::Z), &lay(red, Catch::Raised(dry)));
        assert!(near(&face, ON_FLAT, 4.0).iter().all(|(_, _, paint)| *paint == BARE));
        assert_eq!(stroke.bricks(), 0, "a stamp that laid nothing kept count of it");
        // Streaked, it is a different set of bristles each stroke.
        let streaked = |seed: u32| {
            let streaky = Drybrush { streak: 1.0, ..dry };
            let mut volume = cube();
            let lay = Lay { seed, ..lay(red, Catch::Raised(streaky)) };
            dab(&mut volume, &wide, &stamp.with_tangent(Vec3::Z), &lay);
            near(&volume, ON_EDGE, 6.0)
        };
        assert_ne!(streaked(7), streaked(8), "the seed changed no bristle");
        assert_eq!(streaked(7), streaked(7));
        // With less on the brush, less on the edge, and not nothing.
        let mut running_out = cube();
        let low = Lay { load: 0.6, ..lay(red, Catch::Raised(dry)) };
        dab(&mut running_out, &wide, &stamp, &low);
        let sum = |volume: &Volume| {
            near(volume, ON_EDGE, 6.0).iter().map(|(_, _, paint)| u32::from(paint[3])).sum::<u32>()
        };
        assert!(sum(&running_out) > 0 && sum(&running_out) * 4 < sum(&volume) * 3);
        // It catches more on the side it comes at: brushed toward -x, the
        // edge takes more on its +x face than its +y, and the other way
        // about brushed toward -y.
        let sides = |travel: Vec3| {
            let mut volume = cube();
            let coming = stamp.with_tangent(travel);
            dab(&mut volume, &wide, &coming, &lay(red, Catch::Raised(dry)));
            let (mut x_face, mut y_face) = (0, 0);
            for (_, at, paint) in near(&volume, ON_EDGE, 6.0) {
                if at.x > at.y {
                    x_face += u32::from(paint[3]);
                } else if at.y > at.x {
                    y_face += u32::from(paint[3]);
                }
            }
            (x_face, y_face)
        };
        let (x_face, y_face) = sides(-Vec3::X);
        assert!(x_face > y_face + y_face / 4, "toward -x: {x_face} on +x and {y_face} on +y");
        let (x_face, y_face) = sides(-Vec3::Y);
        assert!(y_face > x_face + x_face / 4, "toward -y: {x_face} on +x and {y_face} on +y");

        // A sponge on the flat, at its density of it: the layer of voxels on
        // the surface itself, well inside the brush.
        // A brush much wider than the patch judged, so that what varies
        // across the patch is the sponge and not the brush's own edge.
        let wide = Brush { radius: 30.0, ..wide };
        let sponge = Sponge { density: 0.4, grain_mm: 1.0, edges: 0.0, ..Sponge::PLAIN };
        let stamp = on(ON_FLAT, Vec3::Z);
        let mut volume = cube();
        let mut stroke = PaintStroke::new();
        volume.begin_stroke();
        stroke.stamp(&mut volume, &wide, &stamp, &lay(red, Catch::Specks(sponge)));
        let layer = |volume: &Volume, radius: f32| -> Vec<Rgba> {
            near(volume, ON_FLAT, radius)
                .into_iter()
                .filter(|(voxel, _, _)| voxel.z == 24)
                .map(|(_, _, paint)| paint)
                .collect()
        };
        let voxels = layer(&volume, 9.0);
        assert!(voxels.len() > 900, "the layer is only {} voxels", voxels.len());
        // Counted where it is at least half covered: the rim is soft.
        let covered = voxels.iter().filter(|paint| paint[3] >= 128).count() as f32;
        let share = covered / voxels.len() as f32;
        assert!((share - 0.4).abs() < 0.08, "a sponge at 0.4 covered {share} of the flat");
        // Near the middle, where the brush itself lays all it has: a speck
        // has a middle that is wholly covered and a rim that is not.
        let middle = layer(&volume, 4.0);
        let rim = middle.iter().filter(|paint| paint[3] > 20 && paint[3] < 200).count();
        let solid = middle.iter().filter(|paint| paint[3] > 230).count();
        assert!(rim > 15 && solid > 15, "{rim} voxels of rim and {solid} of speck");
        // The same specks through the band: every voxel above and below one
        // on the surface holds what it holds.
        let mut columns = 0;
        for x in -8..=8 {
            for y in -8..=8 {
                let on_it = volume.paint().at(IVec3::new(x, y, 24));
                for dz in [-2, -1, 1, 2] {
                    let off_it = volume.paint().at(IVec3::new(x, y, 24 + dz));
                    assert!(
                        (i32::from(off_it[3]) - i32::from(on_it[3])).abs() <= 3,
                        "{dz} off the face at {x} {y}: {off_it:?} against {on_it:?}"
                    );
                }
                columns += 1;
            }
        }
        assert!(columns > 200);
        // A stamp beside it in the same stroke agrees about the specks
        // they share, and another seed is another speckle.
        let specks = |volume: &Volume| -> Vec<bool> {
            layer(volume, 5.0).iter().map(|paint| *paint != BARE).collect()
        };
        let before = specks(&volume);
        let beside = on(ON_FLAT + Vec3::X * 2.0, Vec3::Z);
        stroke.stamp(&mut volume, &wide, &beside, &lay(red, Catch::Specks(sponge)));
        assert_eq!(specks(&volume), before, "a second stamp moved the specks");
        let mut other = cube();
        let reseeded = Lay { seed: 8, ..lay(red, Catch::Specks(sponge)) };
        dab(&mut other, &wide, &stamp, &reseeded);
        assert_ne!(specks(&other), before, "the seed changed nothing");
        // All of a flat is all of it, and none of it is none.
        for (density, alpha) in [(1.0, 255), (0.0, 0)] {
            let sponge = Sponge { density, grain_mm: 1.0, edges: 0.0, ..Sponge::PLAIN };
            let mut volume = cube();
            dab(&mut volume, &wide, &stamp, &lay(red, Catch::Specks(sponge)));
            let patch = layer(&volume, 2.0);
            assert!(patch.len() > 40);
            assert!(patch.iter().all(|paint| paint[3] == alpha), "a density of {density}");
        }
    }

    /// A mirrored stamp lays the mirror image, specks and all.
    #[test]
    fn a_mirrored_stamp_lays_the_mirror_image() {
        let brush = brush(5.0);
        let sponge = Sponge { density: 0.5, grain_mm: 1.0, edges: 0.0, ..Sponge::PLAIN };
        let at = Vec3::new(6.0, 2.0, HALF);
        let mut volume = cube();
        dab_mirrored(
            &mut volume,
            &brush,
            &on(at, Vec3::Z),
            &lay(pigment(RED, 1.0), Catch::Specks(sponge)),
            Symmetry::X,
        );
        let here = near(&volume, at, 5.5);
        let painted = here.iter().filter(|(_, _, paint)| *paint != BARE).count();
        assert!(painted > 200 && painted < here.len() - 200, "{painted} of {}", here.len());
        for (voxel, _, paint) in here {
            let twin = IVec3::new(-voxel.x, voxel.y, voxel.z);
            assert_eq!(volume.paint().at(twin), paint, "{voxel} and its twin {twin}");
        }
        // A streaky drybrush along an edge, mirrored: the same bristles on
        // the twin edge.
        let dry = Drybrush { catch: 0.8, streak: 1.0, bristle_mm: 0.6 };
        let along = on(ON_EDGE, Vec3::new(1.0, 1.0, 0.0).normalize()).with_tangent(Vec3::Z);
        let mut volume = cube();
        dab_mirrored(
            &mut volume,
            &Brush { radius: 8.0, ..brush },
            &along,
            &lay(pigment(RED, 1.0), Catch::Raised(dry)),
            Symmetry::X,
        );
        let here = near(&volume, ON_EDGE, 7.0);
        let painted = here.iter().filter(|(_, _, paint)| *paint != BARE).count();
        let (mut matched, mut differed) = (0, 0);
        for (voxel, _, paint) in &here {
            let twin = IVec3::new(-voxel.x, voxel.y, voxel.z);
            matched += usize::from(volume.paint().at(twin) == *paint && *paint != BARE);
            differed += usize::from(volume.paint().at(twin) != *paint);
        }
        // To a voxel or two: a voxel within rounding of the line can fall
        // the other side of it from its twin. See `touch::Curvatures`.
        assert!(painted > 20 && matched * 20 >= painted * 19, "{matched} of {painted}");
        assert!(differed * 20 <= painted, "{differed} voxels differ from their twins");

        // With the symmetry off, the other side is bare.
        let mut alone = cube();
        dab_mirrored(
            &mut alone,
            &brush,
            &on(at, Vec3::Z),
            &Lay::loaded(pigment(RED, 1.0)),
            Symmetry::OFF,
        );
        assert!(near(&alone, at * Vec3::new(-1.0, 1.0, 1.0), 5.5).iter().all(|v| v.2 == BARE));
        assert!(near(&alone, at, 5.5).iter().any(|v| v.2 != BARE));
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use crate::brush::{BrushDirection, BrushKind};

    #[test]
    #[ignore = "a measurement, not a claim: run by hand, in release"]
    fn what_a_stamp_of_paint_costs() {
        for (voxel, radius) in [(0.25_f32, 3.0_f32), (0.25, 10.0), (0.1, 5.0)] {
            let mut volume = Volume::new(voxel);
            volume.seed_sphere(Vec3::ZERO, 30.0);
            let brush = Brush { kind: BrushKind::Paint, radius, ..Brush::default() };
            let stamp = Stamp::new(Vec3::new(0.0, 0.0, 30.0), Vec3::Z, BrushDirection::Add)
                .with_tangent(Vec3::X);
            let pigment = Pigment {
                colour: [200, 30, 20],
                opacity: 0.8,
                mode: Mode::Paint,
                ..Pigment::default()
            };
            // As a stroke lays them: twenty stamps a quarter of a radius
            // apart, where most of what a stamp covers the one before it has
            // covered already.
            let along = {
                let mut stroke = PaintStroke::new();
                let lay = Lay::loaded(pigment);
                volume.begin_stroke();
                let started = web_time::Instant::now();
                for step in 0..20 {
                    let centre = Vec3::new(step as f32 * radius * 0.25, 0.0, 30.0);
                    let stamp = Stamp::new(centre, Vec3::Z, BrushDirection::Add);
                    stroke.stamp(&mut volume, &brush, &stamp, &lay);
                }
                let each = started.elapsed().as_secs_f64() * 1000.0 / 20.0;
                let _ = volume.end_stroke();
                each
            };
            // One undo stroke for all that follows, so that every round
            // lays its paint over the same thing: what was there before.
            volume.begin_stroke();
            let mut time = |catch: Catch| {
                let rounds = 20;
                let mut stroke = PaintStroke::new();
                let lay = Lay { catch, seed: 1, ..Lay::loaded(pigment) };
                let started = web_time::Instant::now();
                for _ in 0..rounds {
                    // A new stroke each time, so that every stamp lays its
                    // paint: a second stamp in the same place in the same
                    // stroke lays none, and costs less.
                    stroke.begin();
                    stroke.stamp(&mut volume, &brush, &stamp, &lay);
                }
                started.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds)
            };
            let loaded = time(Catch::All);
            let wash = time(Catch::Recesses);
            let dry = time(Catch::Raised(Drybrush::default()));
            let sponge = time(Catch::Specks(Sponge::default()));
            eprintln!(
                "{:.0} voxels of radius at {voxel} mm: loaded {loaded:.2} ms, along a stroke \
                 {along:.2}, wash {wash:.2}, drybrush {dry:.2}, sponge {sponge:.2}",
                radius / voxel
            );
        }
    }
}
