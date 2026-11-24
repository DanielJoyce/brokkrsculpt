// SPDX-License-Identifier: AGPL-3.0-only

//! Solids: shapes that are added to a body, or taken out of one.
//!
//! `docs/STUDIO-PLAN.md` section 18. A brush moves the surface that is
//! there. An INSERT sets a whole shape down on it: a ball, a box, a copy
//! of another body, a cable between two points. In a sculptor of meshes
//! that is a second mesh welded on. Here a body is a field of distances,
//! and a shape that can say how far any point is from it is joined to a
//! body by keeping, at each voxel, the nearer of the two surfaces. No
//! weld, no seam, and an overhang costs nothing: this is also what a
//! vector displacement brush is for, without the map.
//!
//! * [`Solid`] is such a shape: [`Placed`] (one of the [`Form`]s, set down
//!   in a frame), [`Lent`] (a copy of another body's field, turned and
//!   sized), [`Step`] (one straight run of a cable).
//! * [`Join`] joins solids to a body **as it was before the open stroke**,
//!   so that a shape being dragged out can be set again and again without
//!   the tries piling up, and the runs of a cable meet without a bulge.
//! * [`body`] builds solids into a body of their own.
//!
//! # Joined to the body as it was, and to each other plainly
//!
//! A soft join (a small fillet where the shape meets the body) is a
//! smooth minimum of the two distances. Done again for every run of a
//! cable against the field as it then stands, each run would be filleted
//! to the one before it, and where two runs overlap that fillet is a
//! bulge: ribs along the cable. So each piece is joined softly to the
//! body **as it was before the stroke** (the stroke's undo record, read
//! back: [`Volume::snapshot_before`]) and plainly, by the nearer surface,
//! to whatever the stroke has added already.
//!
//! # How soft a join can be
//!
//! The field is kept three voxels either side of the surface and no
//! further ([`OUTSIDE`]). A smooth minimum reads both distances wherever
//! they are within its width of each other, and where one of them is a
//! clamp and not a distance it rounds off something that is not there. At
//! no more than two voxels that cannot reach the surface, and up to there
//! the field's own distances are what is read. A wider join, up to
//! [`MOST_BLEND`], works the body's distance out further than the field
//! keeps it: `solid/wide.rs`, stage C9.

use contracts::debug_requires;
use glam::{IVec3, Mat3, Vec2, Vec3};

use crate::brick::{BRICK_DIM, INSIDE, OUTSIDE};
use crate::mask::PROTECTED;
use crate::region::FieldRegion;
use crate::volume::{BrickPreview, BrickVerdict, Volume};

mod relief;
mod wide;

pub use relief::{BuiltInMap, Draped, Heights, MAX_MAP_SIZE, MapError, Relief, VectorMap};

/// The widest a soft join may be, in voxels. See the module documentation.
pub const MOST_BLEND: f32 = 8.0;

/// Half the diagonal of a brick, in voxels: how far any voxel of a brick
/// can be from its middle.
const BRICK_HALF_DIAGONAL: f32 = BRICK_DIM as f32 * 0.866_025_4;

/// A shape that knows how far a point is from it.
pub trait Solid: Sync {
    /// The distance from `point` to the surface, in world units, negative
    /// inside. It has to be right near the surface. Further off it may be
    /// an underestimate (a clamp, or the nearer of two parts), never an
    /// overestimate: [`Join`] skips what this says is far away.
    fn distance(&self, point: Vec3) -> f32;

    /// A box that holds all of it.
    fn bounds(&self) -> (Vec3, Vec3);

    /// How far from its surface [`Solid::distance`] is sure to be the
    /// distance and not an underestimate, in world units. A shape with a
    /// formula is sure of it everywhere, which is what this says unless a
    /// solid says otherwise. A join wider than this works the distance
    /// out for itself (`solid/wide.rs`).
    fn sure(&self) -> f32 {
        f32::INFINITY
    }
}

/// The shapes that come with the application, each a unit across from its
/// middle: it fits a box from -1 to 1, with its axis, where it has one,
/// along z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Form {
    #[default]
    Ball,
    /// A box with its edges eased.
    Box,
    /// A cylinder standing on the surface.
    Cylinder,
    /// A cone, point out.
    Cone,
    /// A ring lying flat.
    Torus,
}

impl Form {
    pub const ALL: [Form; 5] = [Form::Ball, Form::Box, Form::Cylinder, Form::Cone, Form::Torus];

    pub fn label(self) -> &'static str {
        match self {
            Form::Ball => "Ball",
            Form::Box => "Box",
            Form::Cylinder => "Cylinder",
            Form::Cone => "Cone",
            Form::Torus => "Ring",
        }
    }

    /// The distance from a point to the unit shape.
    pub fn distance(self, p: Vec3) -> f32 {
        self.distance_deep(p, 1.0)
    }

    /// The distance from a point to the unit shape, a cylinder being
    /// `deep` from its middle to each end and not a unit: a disc, a
    /// sleeve. The other forms have no such setting.
    ///
    /// A shallow cylinder is not a cylinder with its axis squeezed. That
    /// was tried first, for the ends of a cable, and a distance that has
    /// been squeezed is no longer a distance: six times too steep along
    /// the axis of a flange, so that the three voxels the field is kept
    /// to were half a voxel, and the flange came out in steps. Seen on a
    /// screen.
    pub fn distance_deep(self, p: Vec3, deep: f32) -> f32 {
        let around = Vec2::new(p.x, p.y).length();
        match self {
            Form::Ball => p.length() - 1.0,
            Form::Box => {
                const EASE: f32 = 0.08;
                let q = p.abs() - Vec3::splat(1.0 - EASE);
                q.max(Vec3::ZERO).length() + q.max_element().min(0.0) - EASE
            }
            Form::Cylinder => {
                let d = Vec2::new(around - 1.0, p.z.abs() - deep);
                d.max_element().min(0.0) + d.max(Vec2::ZERO).length()
            }
            Form::Cone => {
                // A cone from a base of radius 1 at z = -1 to a point at
                // z = 1: the capped cone, with nothing left of its cap.
                let q = Vec2::new(around, p.z);
                let (tip, side) = (Vec2::new(0.0, 1.0), Vec2::new(-1.0, 2.0));
                let cap =
                    Vec2::new(q.x - q.x.min(if q.y < 0.0 { 1.0 } else { 0.0 }), q.y.abs() - 1.0);
                let along = ((tip - q).dot(side) / side.length_squared()).clamp(0.0, 1.0);
                let slope = q - tip + side * along;
                let sign = if slope.x < 0.0 && cap.y < 0.0 { -1.0 } else { 1.0 };
                sign * cap.length_squared().min(slope.length_squared()).sqrt()
            }
            Form::Torus => {
                const RING: f32 = 0.7;
                Vec2::new(around - RING, p.z).length() - (1.0 - RING)
            }
        }
    }
}

/// A [`Form`] set down somewhere: where its middle is, which way it faces,
/// and how big it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub form: Form,
    pub centre: Vec3,
    /// The shape's own axes in the world: x, y, and z, which is its axis.
    pub frame: Mat3,
    /// From its middle to its edge, in world units.
    pub half: f32,
    /// For a cylinder, how far it is from its middle to each end, as a
    /// share of `half`. 1 for a cylinder as deep as it is wide.
    pub deep: f32,
}

impl Placed {
    /// A form set down on a surface: at `point`, its axis along `normal`,
    /// its y toward `toward` as near as the surface allows, `half` from
    /// middle to edge, and lifted off the point by `lift` halves along the
    /// normal: at 0 its middle is on the surface, at 1 it rests on it.
    /// Nothing for a normal that is no direction or a size that is none.
    pub fn on(
        form: Form,
        point: Vec3,
        normal: Vec3,
        toward: Vec3,
        half: f32,
        lift: f32,
    ) -> Option<Self> {
        let out = normal.try_normalize()?;
        if !(half.is_finite() && half > 0.0) {
            return None;
        }
        let lying = toward - out * toward.dot(out);
        let up = lying.try_normalize().unwrap_or_else(|| out.any_orthonormal_vector());
        let frame = Mat3::from_cols(up.cross(out), up, out);
        Some(Self { form, centre: point + out * (half * lift), frame, half, deep: 1.0 })
    }
}

impl Solid for Placed {
    fn distance(&self, point: Vec3) -> f32 {
        let local = self.frame.transpose() * (point - self.centre) / self.half;
        self.form.distance_deep(local, self.deep) * self.half
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        // Whichever way it is turned, it is inside the ball round its box.
        let reach = Vec3::splat(self.half * 1.733);
        (self.centre - reach, self.centre + reach)
    }
}

/// A copy of another body's field, turned, sized and set down: a body
/// lent as a shape. What an imported mesh becomes once it is a body, so
/// this is the insert-mesh brush.
#[derive(Clone, Copy)]
pub struct Lent<'a> {
    source: &'a Volume,
    /// The middle of the source's own box, and half its diagonal.
    middle: Vec3,
    radius: f32,
    centre: Vec3,
    frame: Mat3,
    /// How many times bigger than the source the copy is.
    scale: f32,
}

impl<'a> Lent<'a> {
    /// The body `source` set down as [`Placed::on`] sets a form down,
    /// sized so that it fits a ball of radius `half`.
    ///
    /// `extent` is the box of the source's own surface
    /// ([`Volume::surface_bounds`]): asked for and not found here, because
    /// finding it reads every voxel of the body and a shape being dragged
    /// out is set down again on every move of the pointer. The box of its
    /// BRICKS will not do: it is up to a brick bigger each way, and a copy
    /// sized and centred by it is the wrong size and off the pointer,
    /// which is what the first test of this found.
    #[allow(clippy::too_many_arguments)]
    pub fn on(
        source: &'a Volume,
        extent: (Vec3, Vec3),
        point: Vec3,
        normal: Vec3,
        toward: Vec3,
        half: f32,
        lift: f32,
    ) -> Option<Self> {
        // That box is of every voxel that holds a distance, which is the
        // surface and the band kept either side of it: nearly three voxels
        // more each way than the surface itself.
        let band = Vec3::splat((OUTSIDE - 0.5) * source.voxel_size());
        let (low, high) = (extent.0 + band, extent.1 - band);
        if low.cmpge(high).any() {
            return None;
        }
        let radius = (high - low).length() * 0.5;
        let seat = Placed::on(Form::Ball, point, normal, toward, half, lift)?;
        (radius.is_finite() && radius > 0.0).then_some(Self {
            source,
            middle: (low + high) * 0.5,
            radius,
            centre: seat.centre,
            frame: seat.frame,
            scale: half / radius,
        })
    }
}

impl Solid for Lent<'_> {
    fn distance(&self, point: Vec3) -> f32 {
        let local = self.frame.transpose() * (point - self.centre) / self.scale + self.middle;
        // The source's field is in its own voxels, and is a clamp past
        // three of them: an underestimate out there, which is allowed.
        self.source.sample_world(local) * self.source.voxel_size() * self.scale
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let reach = Vec3::splat(self.radius * self.scale);
        (self.centre - reach, self.centre + reach)
    }

    /// As far as the source keeps its own field, less the half voxel a
    /// sample between voxels can be out by.
    fn sure(&self) -> f32 {
        (OUTSIDE - 0.5) * self.source.voxel_size() * self.scale
    }
}

/// One straight run of a cable: a tube from one point to another, round
/// at both ends, which is what lets two runs meet at any angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub from: Vec3,
    pub to: Vec3,
    pub radius: f32,
}

impl Solid for Step {
    fn distance(&self, point: Vec3) -> f32 {
        let run = self.to - self.from;
        let length = run.length_squared();
        let along = if length > 0.0 {
            ((point - self.from).dot(run) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (point - (self.from + run * along)).length() - self.radius
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let reach = Vec3::splat(self.radius);
        (self.from.min(self.to) - reach, self.from.max(self.to) + reach)
    }
}

/// A solid seen in a mirror: the same shape on the other side of the
/// planes through `about` that `flip` turns over, which is -1 on each axis
/// that is mirrored and 1 on the rest.
#[derive(Clone, Copy)]
pub struct Mirrored<'a> {
    pub of: &'a dyn Solid,
    pub flip: Vec3,
    pub about: Vec3,
}

impl Mirrored<'_> {
    fn across(&self, point: Vec3) -> Vec3 {
        self.about + (point - self.about) * self.flip
    }
}

impl Solid for Mirrored<'_> {
    fn distance(&self, point: Vec3) -> f32 {
        self.of.distance(self.across(point))
    }

    fn sure(&self) -> f32 {
        self.of.sure()
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let (low, high) = self.of.bounds();
        let (a, b) = (self.across(low), self.across(high));
        (a.min(b), a.max(b))
    }
}

/// Several solids as one: the nearest of them. What a hung cable is,
/// with its runs and its ends, to a join that works something out once
/// for all of what it joins (`solid/wide.rs`).
pub struct Together<'a> {
    /// Each with the box that holds it.
    parts: Vec<(&'a dyn Solid, (Vec3, Vec3))>,
}

impl<'a> Together<'a> {
    pub fn of(parts: &[&'a dyn Solid]) -> Self {
        Self {
            parts: parts
                .iter()
                .map(|part| (*part, part.bounds()))
                .filter(|(_, (low, high))| low.is_finite() && high.is_finite())
                .collect(),
        }
    }
}

impl Solid for Together<'_> {
    fn distance(&self, point: Vec3) -> f32 {
        let mut least = f32::INFINITY;
        for (part, (low, high)) in &self.parts {
            // A part is no nearer than its box is, so one whose box is
            // further than the nearest found so far is not asked.
            let to_box = (*low - point).max(point - *high).max(Vec3::ZERO).length();
            if to_box < least {
                least = least.min(part.distance(point));
            }
        }
        least
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        self.parts.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(low, high), (_, (part_low, part_high))| (low.min(*part_low), high.max(*part_high)),
        )
    }

    fn sure(&self) -> f32 {
        self.parts.iter().map(|(part, _)| part.sure()).fold(f32::INFINITY, f32::min)
    }
}

/// What is at the end of a cable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum End {
    /// Nothing more: the tube's own round end.
    #[default]
    Round,
    /// A short sleeve, wider than the cable: a ferrule, a plug.
    Collar,
    /// A ball: a knob, a joint.
    Ball,
    /// A thin wide disc: a flange, a washer against the surface.
    Flange,
}

impl End {
    pub const ALL: [End; 4] = [End::Round, End::Collar, End::Ball, End::Flange];

    pub fn label(self) -> &'static str {
        match self {
            End::Round => "Round",
            End::Collar => "Collar",
            End::Ball => "Ball",
            End::Flange => "Flange",
        }
    }

    /// The piece this end adds to a cable of `radius` that ends at `at`
    /// having come along `along`, or nothing for a plain round end.
    pub fn piece(self, at: Vec3, along: Vec3, radius: f32) -> Option<Placed> {
        // How wide across the cable's own width, and for a cylinder how
        // deep as a share of that.
        let (form, across, deep) = match self {
            End::Round => return None,
            End::Collar => (Form::Cylinder, 1.6, 0.75),
            End::Ball => (Form::Ball, 1.8, 1.0),
            End::Flange => (Form::Cylinder, 2.4, 0.17),
        };
        let seat = Placed::on(form, at, along, Vec3::ZERO, radius * across, 0.0)?;
        Some(Placed { deep, ..seat })
    }
}

/// The points of a cable hung between two points: `steps` straight runs
/// along a curve that sags by `sag` of the distance between them, the way
/// `down` points. With no sag it is the straight line.
pub fn hang(from: Vec3, to: Vec3, sag: f32, down: Vec3, steps: usize) -> Vec<Vec3> {
    let steps = steps.max(1);
    let drop = down.normalize_or_zero() * (from.distance(to) * sag.max(0.0));
    (0..=steps)
        .map(|index| {
            let along = index as f32 / steps as f32;
            // A parabola: nothing at either end, all of the sag half way.
            from.lerp(to, along) + drop * (4.0 * along * (1.0 - along))
        })
        .collect()
}

/// How a solid is joined to a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct How {
    /// Taken out of the body and not added to it.
    pub cut: bool,
    /// How soft the join is, in voxels, up to [`MOST_BLEND`].
    pub blend: f32,
}

impl How {
    pub const ADD: How = How { cut: false, blend: 0.0 };
}

/// A voxel with a solid taken out of the body: `value` is what it holds
/// now, `before` what it held before the stroke, `off` its distance from
/// the solid, in voxels.
///
/// Taking a solid out is adding it to the outside, so the body as it was
/// with the solid out of it is the further of `before` and `-off`, eased
/// where the two meet. That used to be all of it, and what the voxel
/// held was raised to that. In a stroke that only takes out that is
/// right. In one that has also ADDED, it put back the body as it was
/// wherever a solid taken out so much as came near, and what had been
/// added there was gone: found by the first vector displacement stamp,
/// which adds what is under its surface and takes out the air over it
/// (`solid/relief.rs`), and stood nothing on the ball. So the solid
/// itself is taken out of what the voxel holds NOW, and the body as it
/// was is read only for the easing, where the easing takes more than
/// the plain cut does. In a stroke that only takes out, this is what it
/// was to the last bit.
fn taken_out(value: f32, before: f32, off: f32, width: f32) -> f32 {
    let plain = value.max(-off);
    if width <= 0.0 {
        return plain;
    }
    let eased = -nearer(-before, off, width);
    if eased > before.max(-off) { plain.max(eased) } else { plain }
}

/// How wide a join is, in voxels: what it asks for, within what may be.
fn width_of(how: How) -> f32 {
    if how.blend.is_finite() { how.blend.clamp(0.0, MOST_BLEND) } else { 0.0 }
}

/// The width of a join that is wider than the field's own distances will
/// do, or nothing for one that is not.
fn wide_part(how: How) -> Option<f32> {
    let width = width_of(how);
    (width > wide::NARROW).then_some(width)
}

/// The nearer of two distances, with the corner where they meet eased
/// over `width`. With no width it is the nearer of the two.
fn nearer(a: f32, b: f32, width: f32) -> f32 {
    if width <= 0.0 {
        return a.min(b);
    }
    let ease = (width - (a - b).abs()).max(0.0) / width;
    a.min(b) - ease * ease * width * 0.25
}

/// Solids being joined to a body over one stroke. See the module
/// documentation.
#[derive(Debug, Default)]
pub struct Join {
    /// The boxes the stroke has written, to put back.
    boxes: Vec<(IVec3, IVec3)>,
    region: FieldRegion,
}

impl Join {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new stroke: nothing joined yet.
    pub fn begin(&mut self) {
        self.boxes.clear();
    }

    /// Whether the stroke has joined anything.
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// Put the body back as it was before the stroke, wherever the stroke
    /// has joined anything to it.
    #[debug_requires(volume.is_recording() || self.boxes.is_empty(), "what a body was before a stroke is the open stroke's record")]
    pub fn clear(&mut self, volume: &mut Volume) {
        for (lo, hi) in std::mem::take(&mut self.boxes) {
            volume.snapshot_before(lo, hi, &mut self.region);
            let region = &self.region;
            volume.edit_voxels_where(
                lo,
                hi,
                0,
                false,
                crate::Settled::NONE,
                |_| BrickVerdict::Whole,
                |voxel, _, _, _| region.get(voxel),
            );
        }
    }

    /// Put the body back as it was and join these to it: what a shape
    /// still being dragged out does on every move.
    ///
    /// The wide part of a wide join is worked out once for all of them
    /// together and not once each: the same join, since the smooth
    /// minimum of the body with the nearest of several solids is the
    /// nearest of its smooth minima with each, and a hung cable of two
    /// dozen runs is then one piece of work on every move of the pointer
    /// and not two dozen.
    pub fn set(&mut self, volume: &mut Volume, solids: &[&dyn Solid], how: How) {
        self.clear(volume);
        for solid in solids {
            self.add_narrow(volume, *solid, how);
        }
        if let Some(width) = wide_part(how)
            && !solids.is_empty()
        {
            self.add_wide(volume, &Together::of(solids), how.cut, width);
        }
    }

    /// Join one more solid: softly to the body as it was before the
    /// stroke, and plainly to what the stroke has joined already. Under a
    /// mask, by as much as the mask lets through.
    #[debug_requires(volume.is_recording(), "a solid is joined to the body as it was before the stroke, which is the open stroke's undo record")]
    pub fn add(&mut self, volume: &mut Volume, solid: &dyn Solid, how: How) {
        self.add_narrow(volume, solid, how);
        if let Some(width) = wide_part(how) {
            self.add_wide(volume, solid, how.cut, width);
        }
    }

    /// [`Join::add`] as far as the field's own distances will do it: the
    /// whole of a join no wider than two voxels.
    fn add_narrow(&mut self, volume: &mut Volume, solid: &dyn Solid, how: How) {
        let voxel_size = volume.voxel_size();
        // From the field's own distances, as wide as they will do. The
        // rest of a wider join is `add_wide`.
        let blend = width_of(how).min(wide::NARROW);
        // Past this many voxels from the solid nothing changes: the band
        // the field is kept to, and the width of the join.
        let reach = OUTSIDE + blend;
        let (low, high) = solid.bounds();
        if !(low.is_finite() && high.is_finite()) {
            return;
        }
        let pad = Vec3::splat((reach + 1.0) * voxel_size);
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
            // Every voxel of the brick is at least this far from the solid.
            let nearest = solid.distance(middle) / voxel_size - BRICK_HALF_DIAGONAL;
            if nearest >= reach { BrickVerdict::Skip } else { BrickVerdict::Whole }
        };
        volume.edit_voxels_where(
            lo,
            hi,
            0,
            true,
            crate::Settled::NONE,
            decide,
            move |voxel, position, value, free| {
                if free <= 0.0 {
                    return value;
                }
                let off = solid.distance(position) / voxel_size;
                if off >= reach {
                    return value;
                }
                let before = region.get(voxel);
                let joined = if how.cut {
                    taken_out(value, before, off, blend)
                } else {
                    value.min(nearer(before, off, blend))
                };
                value + (joined.clamp(INSIDE, OUTSIDE) - value) * free
            },
        );
        self.boxes.push((lo, hi));
    }
}

/// Solids as a body of their own, at `voxel_size`: joined plainly to each
/// other and to nothing else. Every brick comes back marked dirty, as
/// [`crate::primitive::build`]'s does and for its reason.
pub fn body(voxel_size: f32, solids: &[&dyn Solid]) -> Volume {
    let mut volume = Volume::new(voxel_size);
    let mut join = Join::new();
    volume.begin_stroke();
    for solid in solids {
        join.add(&mut volume, *solid, How::ADD);
    }
    // The record is of an empty body, and nobody wants it back.
    drop(volume.end_stroke());
    volume.mark_everything_dirty();
    volume
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTRE: Vec3 = Vec3::splat(48.0);
    const RADIUS: f32 = 30.0;

    fn ball() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(CENTRE, RADIUS);
        volume.take_dirty(&mut Vec::new());
        volume
    }

    /// Where the surface is along `towards` from the fixture ball's
    /// centre, in voxels from that centre.
    fn surface(volume: &Volume, towards: Vec3) -> f32 {
        let mut last = (5.0, volume.sample_world(CENTRE + towards * 5.0));
        assert!(last.1 < 0.0, "the walk began outside the model");
        let mut along = last.0;
        while along < 90.0 {
            along += 0.125;
            let value = volume.sample_world(CENTRE + towards * along);
            if value >= 0.0 {
                return last.0 + (along - last.0) * (last.1 / (last.1 - value));
            }
            last = (along, value);
        }
        panic!("no surface along {towards}");
    }

    fn same_values(one: &Volume, other: &Volume) -> bool {
        (0..100).all(|z| {
            (0..110).all(|y| {
                (0..100).all(|x| {
                    let voxel = IVec3::new(x, y, z);
                    one.sample_voxel(voxel).to_bits() == other.sample_voxel(voxel).to_bits()
                })
            })
        })
    }

    fn knob(half: f32) -> Placed {
        Placed::on(Form::Ball, CENTRE + Vec3::Y * RADIUS, Vec3::Y, Vec3::X, half, 0.0)
            .expect("a knob")
    }

    /// Each form is solid in its middle, is where it says it is, and
    /// reaches as far as a unit and no further along its axes.
    #[test]
    fn each_form_is_a_unit_shape() {
        for form in Form::ALL {
            assert!(
                form.distance(Vec3::new(0.7, 0.0, 0.0)) < 0.0 || form == Form::Cone,
                "{form:?}"
            );
            assert!(form.distance(Vec3::splat(3.0)) > 1.0, "{form:?}");
            // Nothing of any of them is outside the box from -1 to 1.
            for axis in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z] {
                assert!(form.distance(axis * 1.05) > 0.0, "{form:?} is past a unit along {axis}");
            }
        }
        assert!((Form::Ball.distance(Vec3::X * 0.25) + 0.75).abs() < 1e-6);
        assert!(
            Form::Cylinder.distance(Vec3::Z * 0.99) < 0.0
                && Form::Cylinder.distance(Vec3::X * 0.99) < 0.0
        );
        // A cone is a point at the top and a unit across at the bottom.
        assert!(Form::Cone.distance(Vec3::new(0.0, 0.0, 0.9)) < 0.0);
        assert!(Form::Cone.distance(Vec3::new(0.3, 0.0, 0.9)) > 0.0);
        assert!(Form::Cone.distance(Vec3::new(0.9, 0.0, -0.95)) < 0.0);
        // A ring has a hole in it.
        assert!(
            Form::Torus.distance(Vec3::ZERO) > 0.0 && Form::Torus.distance(Vec3::X * 0.7) < 0.0
        );
        // Set down, a form is its size and faces the way it was put.
        let cone = Placed::on(Form::Cone, Vec3::ZERO, Vec3::X, Vec3::Y, 4.0, 1.0).expect("a cone");
        assert_eq!(cone.centre, Vec3::X * 4.0);
        assert!(cone.distance(Vec3::X * 7.5) < 0.0 && cone.distance(Vec3::X * 8.5) > 0.0);
        assert!(cone.distance(Vec3::new(0.5, 3.5, 0.0)) < 0.0, "the base is not 4 across");
        assert!(Placed::on(Form::Ball, Vec3::ZERO, Vec3::ZERO, Vec3::X, 1.0, 0.0).is_none());
        assert!(Placed::on(Form::Ball, Vec3::ZERO, Vec3::X, Vec3::X, 0.0, 0.0).is_none());
    }

    /// A solid joined to a body is part of it, and only where it is; taken
    /// out, it is a hollow; under a mask it is neither; and it is one
    /// undo.
    #[test]
    fn a_solid_is_joined_to_a_body_or_taken_out_of_it() {
        let mut volume = ball();
        let mut join = Join::new();
        volume.begin_stroke();
        join.add(&mut volume, &knob(8.0), How::ADD);
        assert!(!join.is_empty());
        let top = surface(&volume, Vec3::Y);
        assert!((top - (RADIUS + 8.0)).abs() < 0.1, "the top of the knob is at {top}");
        assert!((surface(&volume, Vec3::X) - RADIUS).abs() < 0.1, "the far side of the ball moved");
        // Joined again it is the same body: the nearer of the same two.
        let once = volume.duplicated(IVec3::ZERO);
        join.add(&mut volume, &knob(8.0), How::ADD);
        assert!(same_values(&volume, &once), "a solid joined twice is not the solid joined once");
        let edit = volume.end_stroke().expect("the join changed the body");
        volume.apply_edit(edit);
        assert!(same_values(&volume, &ball()), "a join undone is not the body as it was");

        let mut cut = ball();
        cut.begin_stroke();
        Join::new().add(&mut cut, &knob(8.0), How { cut: true, blend: 0.0 });
        cut.end_stroke();
        let floor = surface(&cut, Vec3::Y);
        assert!((floor - (RADIUS - 8.0)).abs() < 0.1, "the floor of the hollow is at {floor}");

        let mut masked = ball();
        masked.edit_mask(IVec3::splat(0), IVec3::splat(110), |_, _, _| PROTECTED);
        masked.begin_stroke();
        Join::new().add(&mut masked, &knob(8.0), How::ADD);
        assert!(masked.end_stroke().is_none(), "a solid was joined under the mask");
    }

    /// A soft join eases the corner where the solid meets the body and
    /// moves nothing else: the top of the knob is where it was, and the
    /// corner has filled.
    #[test]
    fn a_soft_join_eases_the_corner_and_nothing_else() {
        let joined = |blend: f32| {
            let mut volume = ball();
            volume.begin_stroke();
            Join::new().add(&mut volume, &knob(8.0), How { cut: false, blend });
            volume.end_stroke();
            volume
        };
        let (plain, soft) = (joined(0.0), joined(MOST_BLEND));
        assert!((surface(&soft, Vec3::Y) - surface(&plain, Vec3::Y)).abs() < 0.02);
        assert!((surface(&soft, Vec3::X) - surface(&plain, Vec3::X)).abs() < 0.02);
        // The corner is where the knob meets the ball: 8 across from the
        // top, on the ball's own surface.
        let corner = Vec3::new(8.0, RADIUS - 1.0, 0.0).normalize();
        let filled = surface(&soft, corner) - surface(&plain, corner);
        assert!(filled > 0.2 && filled < MOST_BLEND, "the corner filled by {filled} voxels");
        // A join wider than may be is the widest that may be.
        assert!(same_values(&joined(99.0), &soft));
    }

    /// A shape dragged out is set again on every move, and what is left
    /// is the last one alone: no trace of a bigger one tried on the way.
    #[test]
    fn a_shape_set_again_leaves_no_trace_of_the_one_before() {
        let mut volume = ball();
        let mut join = Join::new();
        volume.begin_stroke();
        join.set(&mut volume, &[&knob(14.0)], How::ADD);
        assert!(surface(&volume, Vec3::Y) > RADIUS + 13.0);
        join.set(&mut volume, &[&knob(5.0)], How::ADD);
        volume.end_stroke();

        let mut alone = ball();
        alone.begin_stroke();
        Join::new().add(&mut alone, &knob(5.0), How::ADD);
        alone.end_stroke();
        assert!(same_values(&volume, &alone), "the bigger knob left something behind");

        // And cleared, nothing is left at all.
        let mut gone = ball();
        gone.begin_stroke();
        join.begin();
        join.set(&mut gone, &[&knob(9.0)], How::ADD);
        join.clear(&mut gone);
        gone.end_stroke();
        assert!(same_values(&gone, &ball()));
    }

    /// The runs of a cable meet without a bulge, at the softest join the
    /// field's own distances make: along the top of two runs laid end to
    /// end on the ball it is one height, at the joint as in the middle of
    /// a run. A wider join has the same test in `solid/wide.rs`.
    #[test]
    fn the_runs_of_a_cable_meet_without_a_bulge() {
        let at = |turn: f32| CENTRE + Vec3::new(turn.sin(), turn.cos(), 0.0) * (RADIUS + 2.0);
        let runs = [
            Step { from: at(-0.3), to: at(0.0), radius: 3.0 },
            Step { from: at(0.0), to: at(0.3), radius: 3.0 },
        ];
        let mut volume = ball();
        let mut join = Join::new();
        volume.begin_stroke();
        for run in &runs {
            join.add(&mut volume, run, How { cut: false, blend: wide::NARROW });
        }
        volume.end_stroke();
        let joint = surface(&volume, Vec3::Y);
        assert!((joint - (RADIUS + 5.0)).abs() < 0.1, "the cable's top at the joint is {joint}");
        // The middle of a run is a chord, a little nearer the ball than its
        // ends: 2 + 3 above the ball, less the chord's sag.
        let half = Vec3::new(0.15_f32.sin(), 0.15_f32.cos(), 0.0);
        let chord = (RADIUS + 2.0) * 0.15_f32.cos();
        assert!(
            (surface(&volume, half) - (chord + 3.0)).abs() < 0.15,
            "{}",
            surface(&volume, half)
        );
    }

    /// An end is a piece of its own at the end of a cable, wider than the
    /// cable, and a round end is no piece.
    #[test]
    fn a_cable_has_ends() {
        assert!(End::Round.piece(Vec3::ZERO, Vec3::X, 2.0).is_none());
        for end in [End::Collar, End::Ball, End::Flange] {
            let piece = end.piece(Vec3::ZERO, Vec3::X, 2.0).expect("a piece");
            // Wider than the cable, across it.
            assert!(piece.distance(Vec3::Y * 2.5) < 0.0, "{end:?} is no wider than the cable");
            assert!(piece.distance(Vec3::Y * 5.5) > 0.0, "{end:?}");
            // And no longer than a few of its widths, along it.
            assert!(piece.distance(Vec3::X * 4.5) > 0.0, "{end:?} is long");
        }
        // A flange is thin and a collar is not.
        let along = |end: End| {
            End::piece(end, Vec3::ZERO, Vec3::X, 2.0).expect("a piece").distance(Vec3::X * 1.5)
        };
        assert!(along(End::Flange) > 0.0 && along(End::Collar) < 0.0);
    }

    /// A solid in a mirror is the same solid on the other side, and its
    /// box is on that side with it.
    #[test]
    fn a_solid_in_a_mirror_is_on_the_other_side() {
        let knob = Placed::on(Form::Cone, Vec3::new(10.0, 2.0, 3.0), Vec3::X, Vec3::Y, 4.0, 1.0)
            .expect("a cone");
        let twin = Mirrored { of: &knob, flip: Vec3::new(-1.0, 1.0, 1.0), about: Vec3::ZERO };
        for probe in
            [Vec3::new(12.0, 2.0, 3.0), Vec3::new(17.0, 2.5, 3.0), Vec3::new(9.0, 9.0, 9.0)]
        {
            let other = probe * Vec3::new(-1.0, 1.0, 1.0);
            assert_eq!(twin.distance(other), knob.distance(probe));
        }
        let ((low, high), (twin_low, twin_high)) = (knob.bounds(), twin.bounds());
        assert_eq!((twin_low.x, twin_high.x), (-high.x, -low.x));
        assert_eq!((twin_low.y, twin_high.y), (low.y, high.y));
        // No flip is no mirror.
        let same = Mirrored { of: &knob, flip: Vec3::ONE, about: Vec3::splat(7.0) };
        assert_eq!(same.distance(Vec3::splat(1.0)), knob.distance(Vec3::splat(1.0)));
    }

    /// A hung cable ends where it was told to, sags by what it was told
    /// to half way, and with no sag is the straight line.
    #[test]
    fn a_cable_hangs_between_two_points() {
        let (from, to) = (Vec3::new(0.0, 10.0, 0.0), Vec3::new(20.0, 10.0, 0.0));
        let hung = hang(from, to, 0.25, -Vec3::Y, 16);
        assert_eq!(hung.len(), 17);
        assert_eq!((hung[0], hung[16]), (from, to));
        assert!((hung[8] - Vec3::new(10.0, 5.0, 0.0)).length() < 1e-5, "{}", hung[8]);
        assert!(hung.iter().all(|point| point.y <= 10.0 && point.y >= 5.0 - 1e-5));
        let taut = hang(from, to, 0.0, -Vec3::Y, 4);
        assert!(taut.iter().all(|point| (point.y - 10.0).abs() < 1e-6));
        assert_eq!(hang(from, to, 0.5, -Vec3::Y, 0).len(), 2);
    }

    /// Solids as a body of their own: a ball built this way is the ball
    /// the primitives build, and a body lent as a shape is that body
    /// again, bigger and elsewhere.
    #[test]
    fn solids_are_a_body_of_their_own_and_a_body_is_lent_as_a_shape() {
        let made = body(
            1.0,
            &[&Placed::on(Form::Ball, CENTRE, Vec3::Y, Vec3::X, 12.0, 0.0).expect("a ball")],
        );
        assert!((surface(&made, Vec3::X) - 12.0).abs() < 0.05);
        assert!((surface(&made, -Vec3::Z) - 12.0).abs() < 0.05);
        assert!(made.world_bounds().is_some());

        // Lent at twice the size, its middle on top of the fixture ball.
        // The box of its surface is a cube round a ball, so half that
        // box's diagonal is the ball's radius times the root of three:
        // asked to fit a ball of twice that, the copy is twice the size.
        let top = CENTRE + Vec3::Y * RADIUS;
        let extent = made.surface_bounds().expect("a body with a surface");
        let fits = 2.0 * 12.0 * 3.0_f32.sqrt();
        let lent = Lent::on(&made, extent, top, Vec3::Y, Vec3::X, fits, 0.0).expect("lent");
        let mut volume = ball();
        volume.begin_stroke();
        Join::new().add(&mut volume, &lent, How::ADD);
        volume.end_stroke();
        let reached = surface(&volume, Vec3::Y);
        // To a voxel and a bit: the source's box is known to half a voxel
        // of its own, and the copy is twice the size.
        assert!((reached - (RADIUS + 24.0)).abs() < 1.2, "the copy's top is at {reached}");
        // And it is round the point it was set down on: as far to one
        // side of it as to the other.
        let side = |x: f32| {
            let towards = Vec3::new(x, RADIUS + 10.0, 0.0).normalize();
            surface(&volume, towards)
        };
        assert!(
            (side(8.0) - side(-8.0)).abs() < 0.3,
            "{} one side, {} the other",
            side(8.0),
            side(-8.0)
        );
        let none = (Vec3::ZERO, Vec3::ZERO);
        assert!(Lent::on(&made, none, top, Vec3::Y, Vec3::X, 5.0, 0.0).is_none());
    }

    /// A solid is joined to the body as it was before the stroke, so with
    /// no stroke open there is nothing to join it to.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a solid is joined to the body as it was before the stroke")]
    fn a_solid_joined_with_no_stroke_open_is_refused() {
        Join::new().add(&mut ball(), &knob(4.0), How::ADD);
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn point(reach: f32) -> impl Strategy<Value = Vec3> {
            (-reach..reach, -reach..reach, -reach..reach).prop_map(|(x, y, z)| Vec3::new(x, y, z))
        }

        proptest! {
            /// A form's distance never changes faster than the point
            /// moves, which is what makes it a distance and what lets a
            /// brick be skipped on the strength of its middle.
            #[test]
            fn a_forms_distance_is_a_distance(
                form in prop::sample::select(Form::ALL.to_vec()),
                deep in 0.1_f32..=1.0,
                a in point(3.0),
                b in point(3.0),
            ) {
                // A cylinder of any depth too: a cable's flange is one.
                let apart = (form.distance_deep(a, deep) - form.distance_deep(b, deep)).abs();
                prop_assert!(
                    apart <= a.distance(b) + 1e-4,
                    "{:?}: {} over {}", form, apart, a.distance(b)
                );
            }

            /// Wherever a form is set down and however it is turned and
            /// sized, all of it is inside the box it says holds it.
            #[test]
            fn a_placed_form_is_inside_its_bounds(
                form in prop::sample::select(Form::ALL.to_vec()),
                normal in point(1.0),
                toward in point(1.0),
                half in 0.5_f32..20.0,
                lift in -1.0_f32..1.0,
                probe in point(60.0),
            ) {
                prop_assume!(normal.length() > 0.1);
                let placed = Placed::on(form, Vec3::splat(5.0), normal, toward, half, lift).expect("placed");
                let (low, high) = placed.bounds();
                if placed.distance(probe) < 0.0 {
                    prop_assert!(probe.cmpge(low).all() && probe.cmple(high).all(), "{} is in it and out of its box", probe);
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(24))]

            /// Whatever is joined and however: what is left is a field,
            /// nothing changes far from the solid, and joining adds and
            /// cutting takes away, never the other.
            #[test]
            fn any_join_leaves_a_field_and_only_adds_or_only_takes(
                form in prop::sample::select(Form::ALL.to_vec()),
                half in 2.0_f32..12.0,
                lift in -0.5_f32..1.0,
                blend in 0.0_f32..3.0,
                cut in any::<bool>(),
            ) {
                let top = CENTRE + Vec3::Y * RADIUS;
                let placed = Placed::on(form, top, Vec3::Y, Vec3::X, half, lift).expect("placed");
                let fresh = ball();
                let mut volume = ball();
                volume.begin_stroke();
                Join::new().add(&mut volume, &placed, How { cut, blend });
                volume.end_stroke();
                let (low, high) = placed.bounds();
                let reach = Vec3::splat(OUTSIDE + MOST_BLEND + 2.0);
                for z in 4..92 {
                    for y in 30..110 {
                        for x in 4..92 {
                            let voxel = IVec3::new(x, y, z);
                            let (now, was) = (volume.sample_voxel(voxel), fresh.sample_voxel(voxel));
                            prop_assert!((INSIDE..=OUTSIDE).contains(&now));
                            if cut { prop_assert!(now >= was) } else { prop_assert!(now <= was) }
                            let at = voxel.as_vec3();
                            if !(at.cmpge(low - reach).all() && at.cmple(high + reach).all()) {
                                prop_assert_eq!(now.to_bits(), was.to_bits(), "changed far from the solid at {}", voxel);
                            }
                        }
                    }
                }
            }
        }
    }
}
