// SPDX-License-Identifier: AGPL-3.0-only

//! Brushes that add: a shape set down on the model, a body lent as a
//! shape, a cable.
//!
//! `docs/STUDIO-PLAN.md` section 18, stages C2 and C3. A brush moves the
//! surface that is there. These set something down on it
//! ([`hani_core::solid`]):
//!
//! * **A shape**: a ball, a box, a cylinder, a cone or a ring. Press where
//!   it goes, drag to size it and to turn it.
//! * **A body of the document**, lent as a shape and set down the same
//!   way: an imported mesh is a body, so this is the insert-mesh brush.
//! * **A cable laid along the model**: drag, and a tube follows the
//!   pointer over the surface, as thick as the brush.
//! * **A cable hung between two points**: press at one end, drag to the
//!   other, and it sags between them by as much as it is told to.
//!
//! Each is **baked into** the body in hand, where it is part of that body
//! from then on and a stroke of the Smooth brush blends it in, or left as
//! **a body of its own**, to be moved and turned later and merged or not.
//! Baked in, the invert key takes the shape OUT of the body: a socket, a
//! groove where a cable would run.
//!
//! # A drag is a try, and the last try is what is kept
//!
//! While a shape is being dragged out it is set down again on every move
//! of the pointer, from the body as it was at the press
//! ([`hani_core::solid::Join::set`]), so what is on the screen is always
//! what letting go would leave. A body of its own is shown the same way,
//! as if baked, until the button comes up: then the body in hand is put
//! back exactly as it was and the shape becomes a body.
//!
//! # What `app.rs` holds for this
//!
//! One field, one message arm, a line at the top of the stroke that hands
//! a drag here while one of these is in hand, a line at the release, and
//! a line where a classic brush is chosen, which puts these away.

use glam::{Vec2, Vec3};
use hani_core::solid::{
    self, BuiltInMap, Draped, End, Form, Heights, How, Join, Lent, MOST_BLEND, Mirrored, Placed,
    Solid, Step,
};
use hani_core::{BrushDirection, Change, Entry, MirrorAxis, NodeId, Volume};
use iced::widget::{button, column, row, text};
use iced::{Element, Length};

use super::Hani;
use super::maps::{self, MapName, MapOf};
use crate::message::{AddMessage, MapsMessage, Message};
use crate::theme;

/// How far off the surface a shape's middle may be held, in halves of its
/// size: at -1 it is sunk to its top, at 1 it rests on the surface.
pub(super) const LIFT_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// How far a hung cable may sag, as a share of the distance between its
/// ends.
pub(super) const SAG_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;

/// How soft a join is until it is set, in voxels: a fillet that can be
/// seen. Wider than the field's own distances will do, so it is worked
/// out (`hani_core::solid`, stage C9 of the plan).
pub(super) const DEFAULT_BLEND: f32 = 3.0;

/// What a brush file's `add:` says for a vector displacement map.
pub(super) const MAP: &str = "Map";

/// How many straight runs a hung cable is made of.
/// How far from its middle a shape set down by a drag may reach, in
/// voxels: twice the largest brush ([`super::MAX_RADIUS_VOXELS`]). A shape
/// that size is a few hundred bricks of surface; past it, a drag toward the
/// surface's horizon asked for one with no end, and ran out of memory.
pub(super) const MAX_INSERT_VOXELS: f32 = 160.0;

const HUNG_RUNS: usize = 24;

/// How far apart the points of a laid cable are, in its own radii: near
/// enough that the runs read as a curve.
const LAID_STEP: f32 = 0.6;

/// What is being added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insert {
    /// One of the shapes that come with the application.
    Shape(Form),
    /// A body of the document, lent as a shape.
    Body(NodeId),
    /// A cable laid along the model under the pointer.
    Laid,
    /// A cable hung between two points.
    Hung,
    /// A vector displacement map, stamped: `app/maps.rs`.
    Map(MapOf),
}

impl Insert {
    fn is_cable(self) -> bool {
        matches!(self, Insert::Laid | Insert::Hung)
    }
}

/// What a brush that adds is, apart from any document: what a brush on
/// the palette, and a brush file, keep of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Shape(Form),
    Laid,
    Hung,
    /// A copy of a body. The body is a document's, so a brush cannot keep
    /// which: this is only ever the brush in hand.
    Body,
    /// A vector displacement map. A brush file names it apart, with
    /// `map:`, so it is not among the kinds a line's `add:` can name
    /// outright ([`Kind::kept`]).
    Map(MapOf),
}

impl Kind {
    /// The kinds a brush file can name.
    pub(super) fn kept() -> impl Iterator<Item = Kind> {
        Form::ALL.into_iter().map(Kind::Shape).chain([Kind::Laid, Kind::Hung])
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Kind::Shape(form) => form.label(),
            Kind::Laid => "Cable",
            Kind::Hung => "Hung cable",
            Kind::Body => "Body",
            Kind::Map(_) => MAP,
        }
    }

    fn is_cable(self) -> bool {
        matches!(self, Kind::Laid | Kind::Hung)
    }
}

/// A brush that adds, as it is set up. See [`Kind`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Setup {
    pub what: Kind,
    pub separate: bool,
    pub blend: f32,
    pub lift: f32,
    pub start: End,
    pub end: End,
    pub sag: f32,
}

impl Setup {
    /// One of `what`, baked in plainly with its middle on the surface.
    pub(super) fn plain(what: Kind) -> Self {
        Self {
            what,
            separate: false,
            blend: 0.0,
            lift: 0.0,
            start: End::Round,
            end: End::Round,
            sag: 0.0,
        }
    }

    /// This setup with every number in its range, and with the settings
    /// that mean nothing to its kind at nothing: a shape has no ends, and
    /// only a hung cable sags. Two setups that are the same brush are
    /// then equal.
    pub(super) fn settled(mut self) -> Self {
        let number = |value: f32, range: &std::ops::RangeInclusive<f32>| {
            if value.is_nan() {
                0.0_f32.clamp(*range.start(), *range.end())
            } else {
                value.clamp(*range.start(), *range.end())
            }
        };
        self.blend = number(self.blend, &(0.0..=MOST_BLEND));
        self.lift = number(self.lift, &LIFT_RANGE);
        self.sag = number(self.sag, &SAG_RANGE);
        if !self.what.is_cable() {
            self.start = End::Round;
            self.end = End::Round;
        }
        if self.what != Kind::Hung {
            self.sag = 0.0;
        }
        // A map meets the surface where its own displacement comes to
        // nothing, and is draped on it: no join to soften, nothing to
        // hold it off by.
        if matches!(self.what, Kind::Map(_)) {
            self.blend = 0.0;
            self.lift = 0.0;
        }
        self
    }
}

/// One piece of what a gesture has set down: kept so that the same pieces
/// can be built as a body of their own when the button comes up.
#[derive(Debug, Clone, Copy)]
enum Piece {
    Placed(Placed),
    Step(Step),
    /// The body lent, set down here: its seat is a ball of the size and
    /// the frame it was set down with.
    Lent(Placed),
    /// A map stamped here: its seat, the same way.
    Map(Placed),
}

/// What the application remembers for the brushes that add.
pub(super) struct AddState {
    /// What is in hand to add, or nothing for an ordinary brush.
    pub what: Option<Insert>,
    /// A body of its own, and not baked into the one in hand.
    pub separate: bool,
    /// How soft the join to the body is, in voxels.
    pub blend: f32,
    /// How far off the surface it is held. See [`LIFT_RANGE`].
    pub lift: f32,
    /// What is at each end of a cable.
    pub start: End,
    pub end: End,
    /// How far a hung cable sags.
    pub sag: f32,
    /// A copy of the body lent, taken when it was chosen, and the box of
    /// its surface. A copy, because the body it is set down on may be the
    /// very one it was lent from.
    lent: Option<(Volume, (Vec3, Vec3))>,
    /// The maps that have been taken in, for the panel to list.
    pub maps: Vec<MapName>,
    join: Join,
    /// Where the gesture in progress began: the point on the surface and
    /// the way the surface faces there.
    press: Option<(Vec3, Vec3)>,
    /// The last point of a cable being laid.
    last: Option<Vec3>,
    /// What the gesture has set down so far.
    pieces: Vec<Piece>,
    /// Whether the gesture in progress takes its shape out of the body.
    cut: bool,
    /// A coarse mesh of what a drag holds, drawn until it is set down. See
    /// `app/insert_preview.rs`.
    pub(super) preview: Vec<[Vec3; 3]>,
}

impl Default for AddState {
    /// Baked in with a soft join, half proud of the surface, a cable with
    /// round ends and a little sag: what reads as something set down on
    /// the model and belonging to it.
    fn default() -> Self {
        Self {
            what: None,
            separate: false,
            blend: DEFAULT_BLEND,
            lift: 0.5,
            start: End::Round,
            end: End::Round,
            sag: 0.15,
            lent: None,
            maps: Vec::new(),
            join: Join::new(),
            press: None,
            last: None,
            pieces: Vec::new(),
            cut: false,
            preview: Vec::new(),
        }
    }
}

impl AddState {
    /// Back to an ordinary brush.
    pub(super) fn put_away(&mut self) {
        self.what = None;
        self.lent = None;
    }

    /// What is in hand to add, as a brush keeps it.
    pub(super) fn setup(&self) -> Option<Setup> {
        let what = match self.what? {
            Insert::Shape(form) => Kind::Shape(form),
            Insert::Body(_) => Kind::Body,
            Insert::Laid => Kind::Laid,
            Insert::Hung => Kind::Hung,
            Insert::Map(map) => Kind::Map(map),
        };
        Some(
            Setup {
                what,
                separate: self.separate,
                blend: self.blend,
                lift: self.lift,
                start: self.start,
                end: self.end,
                sag: self.sag,
            }
            .settled(),
        )
    }

    fn how(&self) -> How {
        How { cut: self.cut && !self.separate, blend: self.blend }
    }
}

/// The mirrors of a solid that symmetry asks for: the flips of every
/// combination of the axes that are on, but for no flip at all.
fn flips(symmetry: hani_core::Symmetry) -> Vec<Vec3> {
    let on = [MirrorAxis::X, MirrorAxis::Y, MirrorAxis::Z].map(|axis| symmetry.axis(axis));
    (1..8_u8)
        .filter(|bits| (0..3).all(|axis| bits & (1 << axis) == 0 || on[axis]))
        .map(|bits| {
            let side = |axis: u8| if bits & (1 << axis) != 0 { -1.0 } else { 1.0 };
            Vec3::new(side(0), side(1), side(2))
        })
        .collect()
}

/// The pieces of a gesture, and their mirrors, as solids.
fn with_solids<R>(
    pieces: &[Piece],
    source: Option<&(Volume, (Vec3, Vec3))>,
    flips: &[Vec3],
    then: impl FnOnce(&[&dyn Solid]) -> R,
) -> R {
    let lent: Vec<Lent<'_>> = pieces
        .iter()
        .filter_map(|piece| match (piece, source) {
            (Piece::Lent(seat), Some((source, extent))) => Lent::on(
                source,
                *extent,
                seat.centre,
                seat.frame.z_axis,
                seat.frame.y_axis,
                seat.half,
                0.0,
            ),
            _ => None,
        })
        .collect();
    let mut solids: Vec<&dyn Solid> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Placed(placed) => solids.push(placed),
            Piece::Step(step) => solids.push(step),
            Piece::Lent(_) | Piece::Map(_) => {}
        }
    }
    solids.extend(lent.iter().map(|lent| lent as &dyn Solid));
    let twins: Vec<Mirrored<'_>> = solids
        .iter()
        .flat_map(|solid| {
            flips.iter().map(|flip| Mirrored {
                of: *solid,
                flip: *flip,
                about: super::MIRROR_CENTRE,
            })
        })
        .collect();
    solids.extend(twins.iter().map(|twin| twin as &dyn Solid));
    then(&solids)
}

impl Hani {
    /// Take up a brush that adds, as a brush on the palette has it set up.
    /// The settings that mean nothing to its kind are left as they are in
    /// hand, for the next kind that does use them.
    pub(super) fn take_up(&mut self, setup: Setup) {
        let what = match setup.what {
            Kind::Shape(form) => Insert::Shape(form),
            Kind::Laid => Insert::Laid,
            Kind::Hung => Insert::Hung,
            Kind::Map(map) => Insert::Map(map),
            // Which body is not something a brush knows.
            Kind::Body => return,
        };
        self.add_changed(AddMessage::Chosen(Some(what)));
        let state = &mut self.inserts;
        (state.separate, state.blend, state.lift) = (setup.separate, setup.blend, setup.lift);
        // Baked in or a body of its own is the mode's to say: see
        // `app/workspace.rs`.
        state.separate = self.desk.now == crate::message::Workspace::Transform;
        if setup.what.is_cable() {
            (state.start, state.end) = (setup.start, setup.end);
        }
        if setup.what == Kind::Hung {
            state.sag = setup.sag;
        }
    }

    /// Whether a drag on the model adds something and is not a stroke.
    pub(super) fn inserting(&self) -> bool {
        self.inserts.what.is_some()
    }

    pub(super) fn add_changed(&mut self, message: AddMessage) {
        let state = &mut self.inserts;
        match message {
            AddMessage::Chosen(None) => state.put_away(),
            AddMessage::Chosen(Some(what)) => {
                state.lent = None;
                if let Insert::Body(id) = what {
                    // The copy and the box are found once, here, and not on
                    // every move of the pointer.
                    let Some(source) = self.doc.volume(id) else {
                        self.status = "that body is not in the document any more".to_string();
                        return;
                    };
                    let Some(extent) = source.surface_bounds() else {
                        self.status = "that body has nothing in it to lend".to_string();
                        return;
                    };
                    state.lent = Some((source.duplicated(glam::IVec3::ZERO), extent));
                }
                // A map is made a shape the first time it is taken up,
                // and one that cannot be is not taken up.
                if let Insert::Map(map) = what
                    && let Err(why) = maps::relief(map)
                {
                    self.status = why;
                    return;
                }
                state.what = Some(what);
                // Baked in in Sculpt, a body of its own in Transform: the
                // mode says, whichever way it was chosen.
                state.separate = self.desk.now == crate::message::Workspace::Transform;
                // One thing in hand at a time: the stamp and a brush's own
                // amounts are put away, as they put each other away.
                self.studio.put_away();
                self.status = match what {
                    Insert::Shape(_) | Insert::Body(_) | Insert::Map(_) => {
                        "press where it goes, drag to size and turn it".to_string()
                    }
                    Insert::Laid => "drag along the model to lay a cable".to_string(),
                    Insert::Hung => "press at one end of the cable, drag to the other".to_string(),
                };
            }
            AddMessage::SeparateToggled(separate) => state.separate = separate,
            AddMessage::BlendChanged(voxels) => {
                state.blend = if voxels.is_nan() { 0.0 } else { voxels.clamp(0.0, MOST_BLEND) };
            }
            AddMessage::LiftChanged(lift) => {
                state.lift = if lift.is_nan() {
                    0.0
                } else {
                    lift.clamp(*LIFT_RANGE.start(), *LIFT_RANGE.end())
                };
            }
            AddMessage::StartChosen(end) => state.start = end,
            AddMessage::EndChosen(end) => state.end = end,
            AddMessage::SagChanged(sag) => {
                state.sag = if sag.is_nan() {
                    0.0
                } else {
                    sag.clamp(*SAG_RANGE.start(), *SAG_RANGE.end())
                };
            }
        }
    }

    /// The model under the pointer, lifted off it for a cable: the point
    /// its middle runs through, which at a lift of 1 rests the cable on
    /// the surface.
    ///
    /// **The model as it was before the stroke**, along the pointer's own
    /// ray, and facing the way that faced. The pointer's ray lands on the
    /// cable that has just been laid, and a cable whose next point is on
    /// top of its last climbs toward the eye: seen on a screen, where one
    /// drag across the ball filled the view. Nothing when the ray does not
    /// meet the model as it was within a few of the cable's own widths,
    /// which is a pointer over the cable where it overhangs the model's
    /// edge.
    fn cable_point(&self, pixel: Vec2) -> Option<Vec3> {
        let landed = self.surface_under(pixel)?;
        let radius = self.brush.radius;
        let point = self.before_under(pixel, landed, radius * 6.0)?;
        let normal = self.doc.active_volume().gradient_world_before(point);
        Some(point + normal * (radius * self.inserts.lift))
    }

    /// One pointer event of a drag that adds. See the module
    /// documentation.
    pub(super) fn add_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        let Some(what) = self.inserts.what else {
            return;
        };
        if start {
            self.inserts.press = None;
            self.inserts.last = None;
            self.inserts.pieces.clear();
            self.inserts.cut = direction == BrushDirection::Subtract;
            let Some(point) = self.surface_under(pixel) else {
                return;
            };
            let normal = self.doc.active_volume().gradient_world(point);
            self.arm_recorder();
            self.inserts.join.begin();
            self.inserts.press = Some((point, normal));
            if what.is_cable() {
                self.inserts.last = self.cable_point(pixel);
            }
            return;
        }
        let Some((centre, normal)) = self.inserts.press else {
            return;
        };
        // **What is set down whole is set down once a frame.** A shape, a
        // body, a map and a hung cable are each put back and joined again
        // from where the press was and where the pointer is now, and nothing
        // of where it was on the way. So of several moves handled together
        // only the last is worth the work: with a mouse that reports eight
        // times a frame the other seven were each a whole join, thrown away
        // by the next. A cable laid along the model is the other kind, made
        // of every point the pointer passed, and takes every move. See
        // `Hani::pointer_in`.
        if self.batching && !matches!(what, Insert::Laid) {
            return;
        }
        if !matches!(what, Insert::Laid) {
            self.phases.count(|frame| frame.redone += 1);
        }
        match what {
            Insert::Shape(_) | Insert::Body(_) | Insert::Map(_) => {
                // How far the pointer has gone from the press, in the plane
                // of the surface there: as the stamp is sized.
                let (origin, ray) = self.ray_through(pixel);
                let facing = ray.dot(normal);
                if facing.abs() < 1.0e-3 {
                    return;
                }
                let drag = origin + ray * ((centre - origin).dot(normal) / facing) - centre;
                // No bigger than the largest an insert may be: a drag toward
                // the horizon of the surface goes on for ever, and so did the
                // memory a shape that size took.
                let size = drag.length().min(MAX_INSERT_VOXELS * self.doc.voxel_size());
                let seat = match what {
                    Insert::Shape(form) => {
                        Placed::on(form, centre, normal, drag, size, self.inserts.lift)
                    }
                    // A map's square is in the plane of the surface at
                    // the press, and what it is draped on is measured
                    // from there.
                    Insert::Map(_) => Placed::on(Form::Ball, centre, normal, drag, size, 0.0),
                    _ => Placed::on(Form::Ball, centre, normal, drag, size, self.inserts.lift),
                };
                self.inserts.pieces.clear();
                if let Some(seat) = seat {
                    self.inserts.pieces.push(match what {
                        Insert::Shape(_) => Piece::Placed(seat),
                        Insert::Map(_) => Piece::Map(seat),
                        _ => Piece::Lent(seat),
                    });
                }
                match what {
                    Insert::Map(map) => self.set_map(map),
                    // Drawn coarse, and set down once, on release. See
                    // `app/insert_preview.rs`.
                    _ => self.preview_pieces(),
                }
            }
            Insert::Hung => {
                let (Some(from), Some(to)) = (self.inserts.last, self.cable_point(pixel)) else {
                    return;
                };
                let radius = self.brush.radius;
                let points = solid::hang(from, to, self.inserts.sag, -Vec3::Y, HUNG_RUNS);
                let pieces = &mut self.inserts.pieces;
                pieces.clear();
                pieces.extend(
                    points
                        .windows(2)
                        .map(|run| Piece::Step(Step { from: run[0], to: run[1], radius })),
                );
                // Each end faces away from the cable it is on the end of.
                let ends = [
                    (self.inserts.start, from, points[0] - points[1]),
                    (self.inserts.end, to, points[HUNG_RUNS] - points[HUNG_RUNS - 1]),
                ];
                pieces.extend(
                    ends.into_iter()
                        .filter_map(|(end, at, along)| end.piece(at, along, radius))
                        .map(Piece::Placed),
                );
                self.preview_pieces();
            }
            Insert::Laid => {
                let (Some(last), Some(here)) = (self.inserts.last, self.cable_point(pixel)) else {
                    return;
                };
                let radius = self.brush.radius;
                if last.distance(here) < radius * LAID_STEP {
                    return;
                }
                let first = self.inserts.pieces.is_empty();
                let run = Piece::Step(Step { from: last, to: here, radius });
                self.inserts.pieces.push(run);
                self.add_piece(run);
                if first && let Some(end) = self.inserts.start.piece(last, last - here, radius) {
                    self.inserts.pieces.push(Piece::Placed(end));
                    self.add_piece(Piece::Placed(end));
                }
                self.inserts.last = Some(here);
                self.remesh_dirty();
            }
        }
    }

    /// Join one more piece, and its mirrors, to the body in hand.
    fn add_piece(&mut self, piece: Piece) {
        let how = self.inserts.how();
        let flips = flips(self.symmetry);
        let state = &mut self.inserts;
        let volume = self.doc.active_volume_mut();
        let mut join = |solid: &dyn Solid| {
            state.join.add(volume, solid, how);
            for flip in &flips {
                let twin = Mirrored { of: solid, flip: *flip, about: super::MIRROR_CENTRE };
                state.join.add(volume, &twin, how);
            }
        };
        match piece {
            Piece::Placed(placed) => join(&placed),
            Piece::Step(step) => join(&step),
            // Set down all at once, by `set_map`.
            Piece::Map(_) => {}
            Piece::Lent(seat) => {
                let Some((source, extent)) = &state.lent else {
                    return;
                };
                let lent = Lent::on(
                    source,
                    *extent,
                    seat.centre,
                    seat.frame.z_axis,
                    seat.frame.y_axis,
                    seat.half,
                    0.0,
                );
                if let Some(lent) = lent {
                    join(&lent);
                }
            }
        }
    }

    /// Draw what the gesture holds, coarse, without touching the body. See
    /// `app/insert_preview.rs`.
    fn preview_pieces(&mut self) {
        let flips = flips(self.symmetry);
        let AddState { lent, pieces, preview, .. } = &mut self.inserts;
        *preview = with_solids(pieces, lent.as_ref(), &flips, |solids| {
            super::insert_preview::coarse_mesh(solids)
        });
        self.refresh_overlay();
    }

    /// The preview over the model, translucent: the accent to add, red to
    /// take away.
    pub(super) fn add_insert_preview(&self, batch: &mut hani_gpu::OverlayBatch) {
        if self.inserts.press.is_none() {
            return;
        }
        let colour = if self.inserts.cut { [0.8, 0.12, 0.08, 0.35] } else { [1.0, 0.2, 0.05, 0.3] };
        for [a, b, c] in &self.inserts.preview {
            batch.push_triangle(*a, *b, *c, colour);
        }
    }

    /// Put the body in hand back as it was at the press and set down what
    /// the gesture now holds: every move of a shape still being dragged.
    ///
    /// All of them together and not one at a time, so that a join wider
    /// than the field's own distances is worked out once for the lot
    /// (`hani_core::solid::Join::set`): a hung cable is two dozen runs.
    fn set_pieces(&mut self) {
        let how = self.inserts.how();
        let flips = flips(self.symmetry);
        let AddState { join, lent, pieces, .. } = &mut self.inserts;
        let volume = self.doc.active_volume_mut();
        with_solids(pieces, lent.as_ref(), &flips, |solids| join.set(volume, solids, how));
        self.remesh_dirty();
    }

    /// Where the maps a gesture holds are set down, and in each mirror
    /// that symmetry asks for, each with the surface under it: the body
    /// in hand **as it was before the stroke**, which is what a stamp is
    /// draped on however often it is set down again. A seat in a mirror
    /// has a frame with its axes the other way round, and is draped on
    /// the surface that is THERE, which need not be the mirror of the
    /// surface here.
    fn drapes(&self, relief: &solid::Relief) -> Vec<(Vec3, glam::Mat3, f32, Heights)> {
        let volume = self.doc.active_volume();
        let about = super::MIRROR_CENTRE;
        let mut sides = vec![Vec3::ONE];
        sides.extend(flips(self.symmetry));
        self.inserts
            .pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Map(seat) => Some(*seat),
                _ => None,
            })
            .flat_map(|seat| {
                sides.iter().map(move |flip| {
                    let centre = about + (seat.centre - about) * *flip;
                    let frame = glam::Mat3::from_cols(
                        seat.frame.x_axis * *flip,
                        seat.frame.y_axis * *flip,
                        seat.frame.z_axis * *flip,
                    );
                    (centre, frame, seat.half)
                })
            })
            .map(|(centre, frame, half)| {
                (centre, frame, half, Heights::under(volume, relief, centre, frame, half))
            })
            .collect()
    }

    /// Put the body in hand back as it was at the press and stamp the
    /// map where the gesture now has it: its displacement of the
    /// surface, up and down (`hani_core::solid`, `relief.rs`). With
    /// the invert key held at the press, the map turned inside out.
    fn set_map(&mut self, map: MapOf) {
        let Ok(relief) = maps::relief(map) else {
            return;
        };
        let drapes = self.drapes(&relief);
        let voxel_size = self.doc.voxel_size();
        let turned = self.inserts.cut;
        let join = &mut self.inserts.join;
        let volume = self.doc.active_volume_mut();
        join.clear(volume);
        for (centre, frame, half, heights) in &drapes {
            if let Some(stamp) =
                Draped::on(&relief, heights, *centre, *frame, *half, voxel_size, turned)
            {
                join.stamp(volume, &stamp);
            }
        }
        self.remesh_dirty();
    }

    /// What a map stamped as a body of its own is: what is under its
    /// displaced surface, draped as it was on the body in hand.
    fn map_body(&self, map: MapOf) -> Option<Volume> {
        let relief = maps::relief(map).ok()?;
        let drapes = self.drapes(&relief);
        let voxel_size = self.doc.voxel_size();
        let stamps: Vec<Draped<'_>> = drapes
            .iter()
            .filter_map(|(centre, frame, half, heights)| {
                Draped::on(&relief, heights, *centre, *frame, *half, voxel_size, self.inserts.cut)
            })
            .collect();
        let solids: Vec<&dyn Solid> = stamps.iter().map(|stamp| stamp as &dyn Solid).collect();
        Some(solid::body(voxel_size, &solids))
    }

    /// A map was asked for or taken in.
    pub(super) fn maps_changed(&mut self, message: MapsMessage) -> iced::Task<Message> {
        match message {
            MapsMessage::Requested => {
                return iced::Task::perform(maps::pick(), |taken| {
                    Message::Maps(MapsMessage::Taken(taken))
                });
            }
            MapsMessage::Taken(None) => {}
            MapsMessage::Taken(Some(Err(why))) => {
                self.status = format!("that file cannot be a brush's map: {why}");
            }
            MapsMessage::Taken(Some(Ok(name))) => {
                if !self.inserts.maps.contains(&name) {
                    self.inserts.maps.push(name);
                    self.inserts.maps.sort_by_key(|name| name.as_str().to_lowercase());
                }
                self.add_changed(AddMessage::Chosen(Some(Insert::Map(MapOf::Named(name)))));
            }
        }
        iced::Task::none()
    }

    /// The button came up on a drag that adds. A laid cable gets its far
    /// end. Baked in, the stroke is then finished as any stroke is, by
    /// the caller. As a body of its own, the body in hand is put back as
    /// it was, its stroke is closed with nothing in it, and what was set
    /// down becomes a body.
    pub(super) fn add_released(&mut self) {
        let Some(what) = self.inserts.what else {
            return;
        };
        if self.inserts.press.take().is_none() {
            return;
        }
        // What was previewed is set down now, once. See
        // `app/insert_preview.rs`.
        if !self.inserts.preview.is_empty()
            || matches!(what, Insert::Shape(_) | Insert::Body(_) | Insert::Hung)
        {
            self.inserts.preview.clear();
            if !self.inserts.separate && !matches!(what, Insert::Laid | Insert::Map(_)) {
                self.set_pieces();
            }
            self.refresh_overlay();
        }
        if what == Insert::Laid
            && let Some(Piece::Step(last)) = self
                .inserts
                .pieces
                .iter()
                .rev()
                .find(|piece| matches!(piece, Piece::Step(_)))
                .copied()
            && let Some(end) = self.inserts.end.piece(last.to, last.to - last.from, last.radius)
        {
            self.inserts.pieces.push(Piece::Placed(end));
            self.add_piece(Piece::Placed(end));
            self.remesh_dirty();
        }
        if !self.inserts.separate {
            return;
        }
        // A body of its own: nothing of it stays in the body in hand.
        self.inserts.join.clear(self.doc.active_volume_mut());
        drop(self.doc.active_volume_mut().end_stroke());
        self.remesh_dirty();
        // A map's body is draped on the body in hand, so it is built
        // while the gesture still holds where.
        let stamped = match what {
            Insert::Map(map) => self.map_body(map),
            _ => None,
        };
        let pieces = std::mem::take(&mut self.inserts.pieces);
        if pieces.is_empty() {
            return;
        }
        if self.doc.body_count() >= hani_core::MAX_BODIES
            || self.doc.node_count() >= hani_core::MAX_NODES
        {
            self.status = "could not add a body: the document holds as many as it can".to_string();
            return;
        }
        let flips = flips(self.symmetry);
        let voxel_size = self.doc.voxel_size();
        let made = stamped.unwrap_or_else(|| {
            with_solids(&pieces, self.inserts.lent.as_ref(), &flips, |solids| {
                solid::body(voxel_size, solids)
            })
        });
        if made.world_bounds().is_none() {
            self.status = "nothing was set down: drag further".to_string();
            return;
        }
        let name = match what {
            Insert::Shape(form) => form.label().to_string(),
            Insert::Body(_) => "Insert".to_string(),
            Insert::Laid | Insert::Hung => "Cable".to_string(),
            Insert::Map(map) => map.label().to_string(),
        };
        let name = hani_core::name_that_fits(&name).to_string();
        // Directly below the body it was set down on, at its depth.
        let active = self.doc.active();
        let (at, depth) = match (self.doc.index_of(active), self.doc.node(active)) {
            (Some(at), Some(node)) => (at + 1, node.depth()),
            _ => (self.doc.node_count(), 0),
        };
        let id = self.doc.insert_body(at, depth, name.clone(), made);
        let before = self.history.stats();
        self.history.push(Entry::new(vec![Change::NodeAdded { at, id }]));
        self.record_history(before);
        self.unsaved = true;
        self.status = format!("{name} added as a body of its own");
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// What is in hand to add, for the panel's heading.
    pub(super) fn adding_name(&self) -> Option<String> {
        Some(match self.inserts.what? {
            Insert::Shape(form) => format!("INSERT: {}", form.label().to_uppercase()),
            Insert::Body(id) => {
                let name = self.doc.node(id).map_or("a body", |node| node.name.as_str());
                format!("INSERT: {}", name.to_uppercase())
            }
            Insert::Laid => "CABLE".to_string(),
            Insert::Hung => "HUNG CABLE".to_string(),
            Insert::Map(map) => format!("STAMP: {}", map.label().to_uppercase()),
        })
    }

    /// The brushes that add, and the settings of the one in hand.
    /// Kitbash's library, beyond the palette's brushes that add: a copy
    /// of a body, the maps taken in, and taking one in. See
    /// `app/workspace.rs`.
    pub(super) fn adding_library(&self) -> Element<'_, Message> {
        let changed = |message: AddMessage| Message::Add(message);
        let reading = |label: String| text(label).size(theme::LABEL_SIZE).color(theme::TEXT_DIM);
        let state = &self.inserts;
        // Pressed again, the one in hand is put away.
        let choice = |label: String, what: Insert| {
            let live = state.what == Some(what);
            // A share of the row for each letter, as `ui::segmented` gives:
            // in equal shares, seen on a screen, "Cylinder" ran past its
            // edges.
            let share = label.len() as u16 + 2;
            button(text(label).size(theme::LABEL_SIZE).width(Length::Fill).center())
                .width(Length::FillPortion(share))
                .padding(theme::PAD_CHOICE)
                .style(if live { theme::tool_button_active } else { theme::tool_button })
                .on_press(changed(AddMessage::Chosen((!live).then_some(what))))
        };
        let mut panel = column![].spacing(theme::S1);
        // The bodies of the document, each to be lent as a shape: two to
        // a row, their names being a user's.
        let bodies: Vec<(NodeId, String)> = self
            .doc
            .nodes()
            .iter()
            .filter(|node| node.is_body())
            .map(|node| (node.id, node.name.clone()))
            .collect();
        if !bodies.is_empty() {
            panel = panel.push(reading("A copy of a body".to_string()));
            for pair in bodies.chunks(2) {
                let mut assembled = row![].spacing(theme::S1);
                for (id, name) in pair {
                    assembled = assembled.push(choice(name.clone(), Insert::Body(*id)));
                }
                if pair.len() < 2 {
                    assembled = assembled.push(iced::widget::Space::new().width(Length::Fill));
                }
                panel = panel.push(assembled);
            }
        }
        // The maps: the ones that come with it, three to a row, then the
        // ones taken in, two to a row, their names being a user's.
        panel = panel.push(reading("A displacement map".to_string())).push(
            BuiltInMap::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, kind| {
                assembled.push(choice(kind.label().to_string(), Insert::Map(MapOf::BuiltIn(kind))))
            }),
        );
        for pair in state.maps.chunks(2) {
            let mut assembled = row![].spacing(theme::S1);
            for name in pair {
                assembled = assembled
                    .push(choice(name.as_str().to_string(), Insert::Map(MapOf::Named(*name))));
            }
            if pair.len() < 2 {
                assembled = assembled.push(iced::widget::Space::new().width(Length::Fill));
            }
            panel = panel.push(assembled);
        }
        panel = panel.push(
            button(text("Take in a map…").size(theme::LABEL_SIZE))
                .width(Length::Fill)
                .style(theme::tool_button)
                .on_press(Message::Maps(MapsMessage::Requested)),
        );
        panel.into()
    }

    /// The settings of what is in hand to add: Kitbash's inspector.
    pub(super) fn adding_settings(&self) -> Element<'_, Message> {
        let changed = |message: AddMessage| Message::Add(message);
        let reading = |label: String| text(label).size(theme::LABEL_SIZE).color(theme::TEXT_DIM);
        let state = &self.inserts;
        let mut panel = column![].spacing(theme::S1);
        let Some(what) = state.what else {
            return panel.into();
        };
        let voxel_size = self.doc.voxel_size();
        // Baked in or a body of its own is the mode's to say, not a toggle:
        // a shape added in Sculpt is baked in, a part added in Transform is
        // its own body. See `app/workspace.rs`.
        // A map has no join to soften and nothing to be held off by: it
        // meets the surface where its displacement comes to nothing.
        if matches!(what, Insert::Map(_)) {
            return panel.into();
        }
        panel = panel
            .push(
                crate::ui::number(
                    &self.drafts,
                    "add.blend",
                    "Soft join",
                    state.blend,
                    0.0..=MOST_BLEND,
                    move |voxels| changed(AddMessage::BlendChanged(voxels)),
                )
                .step(0.25)
                .shown(if state.separate || state.blend <= 0.0 {
                    "none".to_string()
                } else {
                    format!("{:.2} mm", state.blend * voxel_size)
                }),
            )
            .push(
                crate::ui::number(
                    &self.drafts,
                    "add.lift",
                    if what.is_cable() {
                        "Held off, of its radius"
                    } else {
                        "Held off, of its size"
                    },
                    state.lift,
                    LIFT_RANGE,
                    move |lift| changed(AddMessage::LiftChanged(lift)),
                )
                .step(0.05)
                .shown(format!("{:+.2}", state.lift)),
            );
        if what.is_cable() {
            let ends = |live: End, chosen: fn(End) -> AddMessage| {
                End::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, end| {
                    assembled.push(
                        button(
                            text(end.label()).size(theme::LABEL_SIZE).width(Length::Fill).center(),
                        )
                        .width(Length::Fill)
                        .padding(theme::PAD_CHOICE)
                        .style(if end == live {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(changed(chosen(end))),
                    )
                })
            };
            panel = panel
                .push(reading("Where it starts".to_string()))
                .push(ends(state.start, AddMessage::StartChosen))
                .push(reading("Where it ends".to_string()))
                .push(ends(state.end, AddMessage::EndChosen));
        }
        if what == Insert::Hung {
            panel = panel.push(
                crate::ui::number(
                    &self.drafts,
                    "add.sag",
                    "Sag, of its span",
                    state.sag,
                    SAG_RANGE,
                    move |sag| changed(AddMessage::SagChanged(sag)),
                )
                .step(0.02)
                .shown(if state.sag > 0.0 {
                    format!("{:.2}", state.sag)
                } else {
                    "none".to_string()
                }),
            );
        }
        panel.into()
    }

    /// What a drag does with the insert in hand, for the status bar. See
    /// `app/cards.rs`.
    pub(super) fn insert_prompt(&self) -> Option<&'static str> {
        // Only while a drag would add it: with the cut or the mask in hand,
        // the prompt hid every status line they wrote (seen on the screen,
        // 2026-11-26, a ring cut's refusal unread under it).
        if self.tool != super::Tool::Sculpt {
            return None;
        }
        let what = self.inserts.what?;
        Some(match (what, self.inserts.separate) {
            (Insert::Laid, _) => "Drag along the model; the brush's radius is its thickness",
            (Insert::Map(_), false) => {
                "Press, drag to size and turn; Ctrl or Alt turns it inside out"
            }
            (Insert::Hung, _) => "Press at one end, drag to the other",
            (_, true) => "Press where it goes, drag to size and turn it",
            (_, false) => "Press, drag to size and turn; Ctrl or Alt takes it out",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
        // As if a frame followed: a move waits for one. See `pointer_in`.
        app.handle_moves();
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    fn press(app: &mut Hani, at: Vec2) {
        let position = iced::Vector::new(at.x, at.y);
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position,
                size: SIZE,
            }),
        );
    }

    fn move_to(app: &mut Hani, at: Vec2) {
        let position = iced::Vector::new(at.x, at.y);
        update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
    }

    fn release(app: &mut Hani) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A drag from one pixel to another, in a dozen moves.
    fn drag(app: &mut Hani, from: Vec2, to: Vec2) {
        press(app, from);
        for step in 1..=12 {
            move_to(app, from.lerp(to, step as f32 / 12.0));
        }
        release(app);
    }

    fn choose(app: &mut Hani, what: Insert) {
        update(app, Message::Add(AddMessage::Chosen(Some(what))));
    }

    fn set(app: &mut Hani, message: AddMessage) {
        update(app, Message::Add(message));
    }

    const MIDDLE: Vec2 = Vec2::new(400.0, 300.0);

    /// Whether the active body of two applications holds the same values.
    fn same_field(one: &Hani, other: &Hani) -> bool {
        let (a, b) = (one.doc.active_volume(), other.doc.active_volume());
        let reach = glam::IVec3::splat(190);
        (-reach.z..=reach.z).step_by(1).all(|z| {
            (-reach.y..=reach.y).step_by(3).all(|y| {
                (-reach.x..=reach.x).step_by(3).all(|x| {
                    let voxel = glam::IVec3::new(x, y, z);
                    a.sample_voxel(voxel).to_bits() == b.sample_voxel(voxel).to_bits()
                })
            })
        })
    }

    /// The point of the fresh ball's surface under a pixel, and the way
    /// out from the ball there.
    fn on_the_ball(pixel: Vec2) -> (Vec3, Vec3) {
        let mut looking = app();
        move_to(&mut looking, pixel);
        let point = looking.surface_under(pixel).expect("the ball is under the pixel");
        (point, point.normalize())
    }

    /// Whether a point is inside the active body.
    fn solid_at(app: &Hani, point: Vec3) -> bool {
        app.doc.active_volume().sample_world(point) < 0.0
    }

    /// A shape dragged out bigger than a copy of the body may hold is set
    /// down all the same. It took the application down: the join copied the
    /// whole of the shape's box, and past 113 voxels from the shape's
    /// middle to its edge that was more than a copy holds, which the core
    /// answers by panicking. At half the voxel size the application opens
    /// with, that is a ball of 14 mm, on a ball of 20.
    #[test]
    fn a_shape_dragged_out_past_any_size_is_set_down() {
        let mut app = app();
        update(&mut app, Message::Resample(super::super::VOXEL_SIZE_MM / 2.0));
        choose(&mut app, Insert::Shape(Form::Ball));
        press(&mut app, MIDDLE);
        for step in 1..=4 {
            move_to(&mut app, MIDDLE + Vec2::new(70.0, 20.0) * step as f32);
        }
        let voxels = match app.inserts.pieces.first() {
            Some(Piece::Placed(seat)) => seat.half / app.doc.voxel_size(),
            _ => panic!("a shape is being dragged out"),
        };
        assert!(voxels > 125.0, "the drag made a ball of only {voxels} voxels");
        let (point, out) = match app.inserts.press {
            Some(press) => press,
            None => panic!("the press found the ball"),
        };
        release(&mut app);
        // Solid a good way out from where the ball's surface was, and one
        // stroke: undone, the ball is as it was.
        let half = voxels * app.doc.voxel_size();
        // Its middle is lifted off the surface by a share of its size.
        let top = half * (app.inserts.lift + 1.0);
        assert!(solid_at(&app, point + out * (top * 0.9)), "nothing was set down");
        assert!(!solid_at(&app, point + out * (top * 1.1)), "what was set down is too big");
        update(&mut app, Message::Undo);
        assert!(!solid_at(&app, point + out * (top * 0.5)), "the shape is still there");
    }

    /// What one move of a drag costs, by the shape's size. A measurement,
    /// run by hand in release: `cargo test --release -p hani-app --
    /// --ignored what_a_move_of_an_insert_costs --nocapture`.
    #[test]
    #[ignore = "a measurement, not a claim: run by hand, in release"]
    fn what_a_move_of_an_insert_costs() {
        for form in [Form::Ball, Form::Box] {
            for pixels in [40.0, 120.0, 240.0] {
                let mut app = app();
                choose(&mut app, Insert::Shape(form));
                press(&mut app, MIDDLE);
                move_to(&mut app, MIDDLE + Vec2::new(pixels * 0.5, 0.0));
                let started = web_time::Instant::now();
                let moves = 6;
                for step in 0..moves {
                    move_to(&mut app, MIDDLE + Vec2::new(pixels + step as f32, 0.0));
                }
                let each = started.elapsed().as_secs_f64() * 1000.0 / moves as f64;
                let half = match app.inserts.pieces.first() {
                    Some(Piece::Placed(seat)) => seat.half / app.doc.voxel_size(),
                    _ => 0.0,
                };
                eprintln!("{form:?} {half:.0} voxels: {each:.1} ms a move");
                release(&mut app);
            }
        }
    }

    /// A shape being dragged is previewed and the body in hand is not
    /// touched until the button comes up; then it is set down once, and the
    /// preview is gone.
    #[test]
    fn a_shape_is_previewed_while_dragged_and_set_down_on_release() {
        let mut app = app();
        choose(&mut app, Insert::Shape(Form::Box));
        press(&mut app, MIDDLE);
        move_to(&mut app, MIDDLE + Vec2::new(60.0, 0.0));
        assert!(!app.inserts.preview.is_empty());
        assert!(same_field(&app, &self::app()), "a move joined the shape");
        release(&mut app);
        assert!(app.inserts.preview.is_empty(), "the preview outlived the drag");
        assert!(!same_field(&app, &self::app()), "nothing was set down");
        update(&mut app, Message::Undo);
        assert!(same_field(&app, &self::app()), "the set-down was not one undo");
    }

    /// A drag far past the model sets down a shape no bigger than the
    /// largest an insert may be, of every form. Unbounded, a drag of three
    /// thousand pixels asked for a ball hundreds of millimetres across, and
    /// the memory it took down the application: reported by the maintainer.
    #[test]
    fn a_drag_far_past_the_model_is_held_to_the_largest_shape() {
        for form in Form::ALL {
            let mut app = app();
            choose(&mut app, Insert::Shape(form));
            press(&mut app, Vec2::new(400.0, 120.0));
            move_to(&mut app, Vec2::new(-3000.0, 300.0));
            let half = match app.inserts.pieces.first() {
                Some(Piece::Placed(seat)) => seat.half,
                _ => panic!("{form:?}: a shape is being dragged out"),
            };
            assert!(
                half <= MAX_INSERT_VOXELS * app.doc.voxel_size() + 1e-3,
                "{form:?} is {half} mm from its middle"
            );
            release(&mut app);
        }
    }

    /// Moves that arrive between two frames set a shape down once, where
    /// the last of them left the pointer, and it is the shape a move a frame
    /// leaves, to the bit.
    #[test]
    fn moves_that_arrive_together_set_a_shape_down_once() {
        let to = MIDDLE + Vec2::new(40.0, 12.0);
        let moved = |step: usize| {
            let at = MIDDLE.lerp(to, step as f32 / 12.0);
            Message::Pointer(PointerEvent::Moved {
                position: iced::Vector::new(at.x, at.y),
                size: SIZE,
            })
        };

        // A frame after every move: the helper handles each as it comes.
        let mut one_a_frame = app();
        choose(&mut one_a_frame, Insert::Shape(Form::Ball));
        press(&mut one_a_frame, MIDDLE);
        one_a_frame.phases.end_frame();
        for step in 1..=12 {
            update(&mut one_a_frame, moved(step));
        }
        assert_eq!(one_a_frame.phases.end_frame().redone, 12);
        release(&mut one_a_frame);

        // Every move, and then the frame.
        let mut together = app();
        choose(&mut together, Insert::Shape(Form::Ball));
        press(&mut together, MIDDLE);
        together.phases.end_frame();
        for step in 1..=12 {
            drop(together.update(moved(step)));
        }
        drop(together.update(Message::Frame));
        let frame = together.phases.last();
        assert_eq!((frame.events, frame.redone), (12, 1), "set down once for twelve moves");
        release(&mut together);

        let (one, other) = (one_a_frame.doc.active_volume(), together.doc.active_volume());
        let (mut differing, mut near) = (0, 0);
        for index in 0..60 * 60 * 60 {
            let cell = glam::IVec3::new(index % 60, index / 60 % 60, index / 3600);
            let point = (cell.as_vec3() - 30.0) * 0.8;
            let (a, b) = (one.sample_world(point), other.sample_world(point));
            differing += usize::from(a.to_bits() != b.to_bits());
            near += usize::from(a.abs() < 2.5);
        }
        assert!(near > 1000, "the points compared miss the surface");
        assert_eq!(differing, 0, "moves handled together set a different shape down");
        assert!(solid_voxels(&together) > solid_voxels(&app()), "nothing was set down");
        assert_eq!(
            together.history_stats.undo_entries, one_a_frame.history_stats.undo_entries,
            "one undo either way"
        );
    }

    /// A shape is set down by a drag: where the press was, as big as the
    /// drag is long. Dragged out and back in it is the size it ended at.
    /// The invert key takes it out of the body. It is one undo.
    #[test]
    fn a_shape_is_set_down_by_a_drag() {
        let (point, out) = on_the_ball(MIDDLE);
        let mut app = app();
        choose(&mut app, Insert::Shape(Form::Ball));
        assert!(app.inserting());
        assert_eq!(app.live_brush_name(), "INSERT: BALL");
        assert_eq!(app.status, "press where it goes, drag to size and turn it");
        set(&mut app, AddMessage::LiftChanged(0.0));
        drag(&mut app, MIDDLE, MIDDLE + Vec2::new(40.0, 0.0));
        // How far the pointer went, on the ball: what the ball is across.
        let half = (on_the_ball(MIDDLE + Vec2::new(40.0, 0.0)).0 - point).length();
        assert!(half > 2.0, "the fixture's drag is only {half} mm");
        assert!(solid_at(&app, point + out * (half * 0.8)), "nothing was set down at the press");
        assert!(!solid_at(&app, point + out * (half * 1.3)), "what was set down is too big");
        assert!(app.history.can_undo());

        // Out to twice that and back: the same as never having gone.
        let mut there_and_back = self::app();
        choose(&mut there_and_back, Insert::Shape(Form::Ball));
        set(&mut there_and_back, AddMessage::LiftChanged(0.0));
        press(&mut there_and_back, MIDDLE);
        for far in [20.0, 60.0, 80.0, 60.0, 40.0] {
            move_to(&mut there_and_back, MIDDLE + Vec2::new(far, 0.0));
        }
        release(&mut there_and_back);
        assert!(!solid_at(&there_and_back, point + out * (half * 1.3)), "the bigger try was left");
        assert!(solid_at(&there_and_back, point + out * (half * 0.8)));

        update(&mut app, Message::Undo);
        assert!(same_field(&app, &self::app()), "a shape set down was not one undo");

        // The invert key takes the shape out of the body.
        app.control = true;
        drag(&mut app, MIDDLE, MIDDLE + Vec2::new(40.0, 0.0));
        app.control = false;
        assert!(!solid_at(&app, point - out * (half * 0.6)), "nothing was taken out");
        assert!(solid_at(&app, point - out * (half * 1.4)), "too much was taken out");

        // A press with no drag sets nothing down.
        let mut pressed = self::app();
        choose(&mut pressed, Insert::Shape(Form::Box));
        press(&mut pressed, MIDDLE);
        release(&mut pressed);
        assert!(same_field(&pressed, &self::app()));
    }

    /// A vector displacement map is stamped by a drag, as a shape is set
    /// down: it stands on the model where the press was, it is one undo,
    /// dragging it about leaves no trace, the invert key turns it inside
    /// out and digs what it raised, and as a body of its own the body in
    /// hand is left as it was.
    #[test]
    fn a_map_is_stamped_by_a_drag() {
        let (point, out) = on_the_ball(MIDDLE);
        let horn = Insert::Map(MapOf::BuiltIn(BuiltInMap::Stud));
        let mut app = app();
        choose(&mut app, horn);
        assert!(app.inserting());
        // It is the brush of that name on the palette, and is called it.
        assert_eq!(app.live_brush_name(), "STUD");
        let before = solid_voxels(&app);
        drag(&mut app, MIDDLE, MIDDLE + Vec2::new(60.0, 0.0));
        let half = (on_the_ball(MIDDLE + Vec2::new(60.0, 0.0)).0 - point).length();
        // The stud is 0.63 of the square's half-width high in its middle.
        assert!(solid_at(&app, point + out * (half * 0.5)), "no stud stands at the press");
        assert!(!solid_at(&app, point + out * (half * 0.8)), "what was stamped is too high");
        assert!(solid_voxels(&app) > before);
        assert!(app.history.can_undo());

        let mut there_and_back = self::app();
        choose(&mut there_and_back, horn);
        press(&mut there_and_back, MIDDLE);
        for far in [30.0, 90.0, 60.0] {
            move_to(&mut there_and_back, MIDDLE + Vec2::new(far, 0.0));
        }
        release(&mut there_and_back);
        assert!(same_field(&there_and_back, &app), "a stamp left a trace of where it was dragged");

        update(&mut app, Message::Undo);
        assert!(same_field(&app, &self::app()), "a stamp was not one undo");

        // Turned inside out: a socket the shape of the stud.
        app.control = true;
        drag(&mut app, MIDDLE, MIDDLE + Vec2::new(60.0, 0.0));
        app.control = false;
        assert!(!solid_at(&app, point - out * (half * 0.4)), "nothing was dug");
        assert!(solid_voxels(&app) < before);

        // A body of its own.
        let mut apart = self::app();
        choose(&mut apart, horn);
        set(&mut apart, AddMessage::SeparateToggled(true));
        let bodies = apart.doc.body_count();
        drag(&mut apart, MIDDLE, MIDDLE + Vec2::new(60.0, 0.0));
        assert_eq!(apart.doc.body_count(), bodies + 1);
        assert_eq!(apart.status, "Stud added as a body of its own");
        update(&mut apart, Message::Undo);
        assert!(same_field(&apart, &self::app()));

        // A map that is not there is said, and is not taken up.
        let mut missing = self::app();
        let gone = MapOf::named("no such map anywhere").expect("a name");
        choose(&mut missing, Insert::Map(gone));
        assert!(!missing.inserting());
        assert!(missing.status.contains("no such map anywhere"), "{}", missing.status);
        // A map taken in is in hand, and one that could not be is said.
        let name = MapName::new("Ear").expect("a name");
        let _ = missing.maps_changed(MapsMessage::Taken(Some(Err("not a picture".to_string()))));
        assert!(missing.status.contains("not a picture"));
        let _ = missing.maps_changed(MapsMessage::Taken(Some(Ok(name))));
        assert_eq!(missing.inserts.maps, vec![name]);
    }

    /// How many voxels of the body in hand are inside it.
    fn solid_voxels(app: &Hani) -> usize {
        let volume = app.doc.active_volume();
        let (low, high) = volume.world_bounds().expect("a body");
        let (lo, hi) = volume.voxel_bounds(low, high);
        let mut count = 0;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    if volume.sample_voxel(glam::IVec3::new(x, y, z)) < 0.0 {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// The soft join goes wider than the field's own distances: a shape
    /// set down with the widest fills more of the crease than one with
    /// the widest there was before, a shape taken out leaves a rounder
    /// rim, dragging it about leaves no trace of where it has been, and
    /// one undo takes the shape and its join together.
    #[test]
    fn a_shape_is_joined_as_softly_as_is_asked() {
        let with = |blend: f32, cut: bool| {
            let mut app = app();
            choose(&mut app, Insert::Shape(Form::Ball));
            set(&mut app, AddMessage::BlendChanged(blend));
            assert_eq!(app.inserts.blend, blend);
            app.control = cut;
            press(&mut app, MIDDLE);
            for far in [30.0, 70.0, 40.0] {
                move_to(&mut app, MIDDLE + Vec2::new(far, 0.0));
            }
            release(&mut app);
            app.control = false;
            app
        };
        let (plain, narrow, wide) = (with(0.0, false), with(2.0, false), with(MOST_BLEND, false));
        let (plain_count, narrow_count, wide_count) =
            (solid_voxels(&plain), solid_voxels(&narrow), solid_voxels(&wide));
        assert!(
            narrow_count > plain_count && wide_count > narrow_count + (narrow_count - plain_count),
            "inside: {plain_count} joined plainly, {narrow_count} over 2, {wide_count} over {MOST_BLEND}"
        );
        // The same drag that went straight there.
        let mut straight = app();
        choose(&mut straight, Insert::Shape(Form::Ball));
        set(&mut straight, AddMessage::BlendChanged(MOST_BLEND));
        drag(&mut straight, MIDDLE, MIDDLE + Vec2::new(40.0, 0.0));
        assert!(
            same_field(&straight, &wide),
            "a wide join left a trace of where the shape had been"
        );

        let mut undone = with(MOST_BLEND, false);
        update(&mut undone, Message::Undo);
        assert!(same_field(&undone, &app()), "a shape and its wide join were not one undo");

        let (cut, cut_wide) = (with(0.0, true), with(MOST_BLEND, true));
        assert!(
            solid_voxels(&cut_wide) < solid_voxels(&cut),
            "a socket with a wide join is no rounder at the rim"
        );
    }

    /// As a body of its own: the body in hand is exactly as it was, the
    /// shape is a new body beside it, and one undo takes that away. A body
    /// of the document is lent as a shape the same way, the one in hand
    /// among them.
    #[test]
    fn what_is_added_can_be_a_body_of_its_own() {
        let (point, out) = on_the_ball(MIDDLE);
        let mut app = app();
        let first = app.doc.active();
        choose(&mut app, Insert::Shape(Form::Cone));
        set(&mut app, AddMessage::SeparateToggled(true));
        press(&mut app, MIDDLE);
        move_to(&mut app, MIDDLE + Vec2::new(45.0, 10.0));
        // While it is dragged it is shown, coarse, over the body in hand,
        // which it does not touch. See `app/insert_preview.rs`.
        assert!(!app.inserts.preview.is_empty(), "nothing was shown during the drag");
        assert!(same_field(&app, &self::app()), "the drag changed the body in hand");
        release(&mut app);
        assert_eq!(app.doc.body_count(), 2);
        assert_eq!(app.doc.active(), first, "the body in hand changed");
        assert!(same_field(&app, &self::app()), "something was left in the body in hand");
        assert_eq!(app.status, "Cone added as a body of its own");
        let added = app.doc.nodes().iter().find(|node| node.id != first).expect("a second body");
        assert_eq!(added.name, "Cone");
        let cone = added.volume().expect("a body has a field");
        assert!(
            cone.sample_world(point + out * 1.0) < 0.0,
            "the new body is not where the press was"
        );
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), 1);
        assert!(!app.history.can_undo(), "the drag left a second thing to undo");

        // The ball itself, lent as a shape and baked into itself: a small
        // ball on the ball.
        choose(&mut app, Insert::Body(first));
        assert_eq!(app.live_brush_name(), "INSERT: BODY 1");
        set(&mut app, AddMessage::SeparateToggled(false));
        set(&mut app, AddMessage::LiftChanged(1.0));
        drag(&mut app, MIDDLE, MIDDLE + Vec2::new(40.0, 0.0));
        assert_eq!(app.doc.body_count(), 1);
        assert!(solid_at(&app, point + out * 2.0), "the lent body was not set down");
        // And as a body of its own.
        set(&mut app, AddMessage::SeparateToggled(true));
        drag(&mut app, Vec2::new(300.0, 250.0), Vec2::new(340.0, 250.0));
        assert_eq!(app.doc.body_count(), 2);
        assert_eq!(app.doc.nodes().iter().filter(|node| node.name == "Insert").count(), 1);
        // A body that is gone cannot be lent.
        choose(&mut app, Insert::Body(NodeId(987_654)));
        assert_eq!(app.status, "that body is not in the document any more");
    }

    /// A cable laid by a drag follows the model: it is on it from end to
    /// end and nowhere far off it, is as thick as the brush, has the ends
    /// it was given, and is one undo.
    #[test]
    fn a_cable_is_laid_along_the_model() {
        let (from, to) = (Vec2::new(300.0, 300.0), Vec2::new(500.0, 300.0));
        let mut app = app();
        choose(&mut app, Insert::Laid);
        assert_eq!(app.live_brush_name(), "CABLE");
        set(&mut app, AddMessage::LiftChanged(1.0));
        update(&mut app, Message::BrushRadiusChanged(2.0));
        drag(&mut app, from, to);
        let radius = super::super::MODEL_RADIUS_MM;
        // Along the drag the cable is there, resting on the ball: solid a
        // radius off the surface, and air three off it.
        for part in [0.1, 0.5, 0.9] {
            let (point, out) = on_the_ball(from.lerp(to, part));
            assert!(solid_at(&app, point + out * 2.0), "no cable {part} of the way along");
            assert!(!solid_at(&app, point + out * 5.0), "the cable is off the model at {part}");
        }
        // Nothing of it is further from the ball than its own thickness:
        // it did not climb. (The first cable did, toward the eye, and
        // filled the view.)
        let (low, high) = app.doc.active_volume().surface_bounds().expect("a surface");
        let furthest = low.abs().max(high.abs()).max_element();
        assert!(
            furthest < radius + 2.0 * 2.0 + 1.5,
            "the cable reaches {furthest} mm from the middle"
        );
        let runs =
            app.inserts.pieces.iter().filter(|piece| matches!(piece, Piece::Step(_))).count();
        assert!(runs >= 5, "a cable of {runs} runs");
        update(&mut app, Message::Undo);
        assert!(same_field(&app, &self::app()), "a cable was not one undo");

        // With ends: a piece at each, wider than the cable.
        set(&mut app, AddMessage::StartChosen(End::Ball));
        set(&mut app, AddMessage::EndChosen(End::Flange));
        drag(&mut app, from, to);
        let ends =
            app.inserts.pieces.iter().filter(|piece| matches!(piece, Piece::Placed(_))).count();
        assert_eq!(ends, 2);
        let (start, out) = on_the_ball(from);
        assert!(solid_at(&app, start + out * 2.0 + Vec3::Y * 3.0), "no ball at the start");
    }

    /// A cable hung between two points ends at both and sags between
    /// them, by more for more sag, and a straight one does not.
    #[test]
    fn a_cable_is_hung_between_two_points() {
        // Two points low on the ball, either side of its middle. The
        // straight line between two points on a ball is inside the ball,
        // so a taut cable here is out of sight but for its ends, and one
        // with enough slack hangs out under the ball where it can be
        // found.
        let (from, to) = (Vec2::new(250.0, 420.0), Vec2::new(550.0, 420.0));
        let hung = |sag: f32| {
            let mut app = app();
            choose(&mut app, Insert::Hung);
            set(&mut app, AddMessage::LiftChanged(1.0));
            set(&mut app, AddMessage::SagChanged(sag));
            update(&mut app, Message::BrushRadiusChanged(1.5));
            drag(&mut app, from, to);
            app
        };
        let (taut, slack) = (hung(0.0), hung(0.6));
        assert_eq!(taut.live_brush_name(), "HUNG CABLE");
        let ((a, out_a), (b, out_b)) = (on_the_ball(from), on_the_ball(to));
        let (start, end) = (a + out_a * 1.5, b + out_b * 1.5);
        for app in [&taut, &slack] {
            assert!(solid_at(app, start) && solid_at(app, end), "the cable is not at its ends");
            let runs =
                app.inserts.pieces.iter().filter(|piece| matches!(piece, Piece::Step(_))).count();
            assert_eq!(runs, HUNG_RUNS);
        }
        // Half way, and down by the sag of the span: under the ball, where
        // there is nothing but the slack cable.
        let hanging = (start + end) * 0.5 - Vec3::Y * (start.distance(end) * 0.6);
        assert!(
            hanging.length() > super::super::MODEL_RADIUS_MM + 3.0,
            "the fixture's cable does not hang clear of the ball: {hanging}"
        );
        assert!(!solid_at(&taut, hanging), "a taut cable sagged");
        assert!(solid_at(&slack, hanging), "the slack cable does not sag to where it should");
        // One undo each.
        let mut undone = slack;
        update(&mut undone, Message::Undo);
        assert!(same_field(&undone, &self::app()));
    }

    /// With the mirror on, what is added is added on both sides.
    #[test]
    fn what_is_added_is_mirrored() {
        let at = Vec2::new(480.0, 300.0);
        let (point, out) = on_the_ball(at);
        assert!(point.x > 3.0, "the fixture's press is on the mirror plane");
        let mut app = app();
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        choose(&mut app, Insert::Shape(Form::Ball));
        set(&mut app, AddMessage::LiftChanged(0.0));
        drag(&mut app, at, at + Vec2::new(25.0, 0.0));
        let flip = Vec3::new(-1.0, 1.0, 1.0);
        assert!(solid_at(&app, point + out * 1.0));
        assert!(solid_at(&app, (point + out * 1.0) * flip), "nothing on the other side");
        // And as a body of its own, both sides are in the one body.
        set(&mut app, AddMessage::SeparateToggled(true));
        drag(&mut app, at, at + Vec2::new(25.0, 0.0));
        let made = app.doc.nodes().iter().find(|node| node.name == "Ball").expect("a ball");
        let made = made.volume().expect("a field");
        assert!(made.sample_world(point + out * 1.0) < 0.0);
        assert!(made.sample_world((point + out * 1.0) * flip) < 0.0);
        assert_eq!(flips(hani_core::Symmetry::OFF), Vec::<Vec3>::new());
        let all = hani_core::Symmetry::OFF
            .with_axis(MirrorAxis::X, true)
            .with_axis(MirrorAxis::Y, true)
            .with_axis(MirrorAxis::Z, true);
        assert_eq!(flips(all).len(), 7);
    }

    /// One thing is in hand at a time: a brush, the stamp, or something
    /// to add. Its settings stay in range, and the panel builds whatever
    /// is in hand.
    #[test]
    fn something_to_add_is_in_hand_until_a_brush_is() {
        let mut app = app();
        {
            drop(app.adding_library());
            drop(app.adding_settings());
        }
        update(&mut app, Message::Studio(crate::message::StudioMessage::StampToggled(true)));
        choose(&mut app, Insert::Shape(Form::Torus));
        assert!(app.inserting() && !app.studio.stamping(), "the stamp stayed up");
        {
            drop(app.adding_library());
            drop(app.adding_settings());
        }
        // Pressed again it is put away.
        set(&mut app, AddMessage::Chosen(None));
        assert!(!app.inserting());

        for what in [Insert::Laid, Insert::Hung, Insert::Shape(Form::Box)] {
            choose(&mut app, what);
            {
                drop(app.adding_library());
                drop(app.adding_settings());
            }
            drop(app.view());
        }
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Clay));
        assert!(!app.inserting(), "a classic brush chosen left the shape in hand");
        choose(&mut app, Insert::Laid);
        update(&mut app, Message::Studio(crate::message::StudioMessage::PresetChosen(0)));
        assert!(!app.inserting(), "a brush from the palette left the cable in hand");
        choose(&mut app, Insert::Hung);
        update(&mut app, Message::Studio(crate::message::StudioMessage::StampToggled(true)));
        assert!(!app.inserting(), "the stamp left the cable in hand");

        set(&mut app, AddMessage::BlendChanged(99.0));
        set(&mut app, AddMessage::LiftChanged(-99.0));
        set(&mut app, AddMessage::SagChanged(99.0));
        let state = &app.inserts;
        assert_eq!((state.blend, state.lift, state.sag), (MOST_BLEND, -1.0, 1.0));
        // Not a number is nothing.
        set(&mut app, AddMessage::BlendChanged(f32::NAN));
        set(&mut app, AddMessage::LiftChanged(f32::NAN));
        set(&mut app, AddMessage::SagChanged(f32::NAN));
        let state = &app.inserts;
        assert_eq!((state.blend, state.lift, state.sag), (0.0, 0.0, 0.0));
    }

    /// The brushes that add are brushes on the palette: taken up from it
    /// they are set up as it says, the one in hand is lit, one made on the
    /// ADD block can be kept under a name and comes back, and a copy of a
    /// body, which is a document's, cannot be kept.
    #[test]
    fn a_brush_that_adds_is_a_brush_on_the_palette() {
        use crate::message::StudioMessage;
        let mut app = app();
        let on_the_palette = |app: &Hani, name: &str| {
            let palette = app.studio.presets.palette();
            palette
                .iter()
                .position(|(preset, _)| preset.name == name)
                .unwrap_or_else(|| panic!("no brush called {name}"))
        };
        let take = |app: &mut Hani, name: &str| {
            let index = on_the_palette(app, name);
            update(app, Message::Studio(StudioMessage::PresetChosen(index)));
        };
        take(&mut app, "Pipe");
        assert_eq!(app.inserts.what, Some(Insert::Laid));
        let state = &app.inserts;
        assert_eq!((state.start, state.end, state.lift), (End::Flange, End::Flange, 1.0));
        assert_eq!(app.live_brush_name(), "PIPE");
        assert_eq!(app.status, "brush: Pipe");
        // A shape has no ends, and taking one up leaves the cable's ends
        // as they were for the next cable.
        take(&mut app, "Knob");
        assert_eq!(app.inserts.what, Some(Insert::Shape(Form::Ball)));
        assert_eq!((app.inserts.start, app.inserts.lift), (End::Flange, 0.8));
        assert_eq!(app.live_brush_name(), "KNOB");
        // An ordinary brush puts it away.
        take(&mut app, "Clay");
        assert!(!app.inserting());

        // Chosen on the ADD block and set up by hand: none of the
        // palette's until it is one of them.
        choose(&mut app, Insert::Shape(Form::Ball));
        set(&mut app, AddMessage::LiftChanged(0.35));
        assert_eq!(app.live_brush_name(), "INSERT: BALL");
        set(&mut app, AddMessage::LiftChanged(0.0));
        assert_eq!(app.live_brush_name(), "BALL");

        // Made by hand and kept: it comes back as it was made.
        choose(&mut app, Insert::Hung);
        set(&mut app, AddMessage::SagChanged(0.4));
        set(&mut app, AddMessage::SeparateToggled(true));
        set(&mut app, AddMessage::StartChosen(End::Ball));
        set(&mut app, AddMessage::EndChosen(End::Collar));
        update(&mut app, Message::BrushRadiusChanged(1.25));
        update(&mut app, Message::Studio(StudioMessage::PresetNamed("Washing line".to_string())));
        update(&mut app, Message::Studio(StudioMessage::PresetSaved));
        assert_eq!(app.status, "brush kept: Washing line");
        take(&mut app, "Draw");
        update(&mut app, Message::BrushRadiusChanged(4.0));
        take(&mut app, "Washing line");
        let state = &app.inserts;
        assert_eq!(state.what, Some(Insert::Hung));
        // Kept and taken up again in Sculpt, where what is added is baked
        // in whatever the brush was kept as: the mode says.
        assert_eq!((state.sag, state.separate), (0.4, false));
        assert_eq!((state.start, state.end), (End::Ball, End::Collar));
        assert_eq!(app.brush.radius, 1.25, "the cable's thickness was not kept");
        assert_eq!(app.live_brush_name(), "WASHING LINE");
        {
            drop(app.adding_library());
            drop(app.adding_settings());
        }
        drop(app.palette_panel());

        // A copy of a body is this document's, and no brush can say which.
        let body = app.doc.active();
        choose(&mut app, Insert::Body(body));
        let kept = app.studio.presets.list.len();
        update(&mut app, Message::Studio(StudioMessage::PresetNamed("My body".to_string())));
        update(&mut app, Message::Studio(StudioMessage::PresetSaved));
        assert!(app.status.starts_with("a copy of a body cannot be kept"), "{}", app.status);
        assert_eq!(app.studio.presets.list.len(), kept);
        assert_eq!(app.live_brush_name(), "INSERT: BODY 1");
    }
}
