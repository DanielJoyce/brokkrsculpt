// SPDX-License-Identifier: AGPL-3.0-only

//! Brushes that add: a shape set down on the model, a body lent as a
//! shape, a cable.
//!
//! `docs/STUDIO-PLAN.md` section 18, stages C2 and C3. A brush moves the
//! surface that is there. These set something down on it
//! ([`brokkr_core::solid`]):
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
//! ([`brokkr_core::solid::Join::set`]), so what is on the screen is always
//! what letting go would leave. A body of its own is shown the same way,
//! as if baked, until the button comes up: then the body in hand is put
//! back exactly as it was and the shape becomes a body.
//!
//! # What `app.rs` holds for this
//!
//! One field, one message arm, a line at the top of the stroke that hands
//! a drag here while one of these is in hand, a line at the release, and
//! a line where a classic brush is chosen, which puts these away.

use brokkr_core::solid::{
    self, End, Form, How, Join, Lent, MOST_BLEND, Mirrored, Placed, Solid, Step,
};
use brokkr_core::{BrushDirection, Change, Entry, MirrorAxis, NodeId, Volume};
use glam::{Vec2, Vec3};
use iced::widget::{button, column, row, slider, text};
use iced::{Element, Length};

use super::Brokkr;
use crate::message::{AddMessage, Message};
use crate::theme;

/// How far off the surface a shape's middle may be held, in halves of its
/// size: at -1 it is sunk to its top, at 1 it rests on the surface.
pub(super) const LIFT_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// How far a hung cable may sag, as a share of the distance between its
/// ends.
pub(super) const SAG_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;

/// How many straight runs a hung cable is made of.
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
}

impl Insert {
    fn is_cable(self) -> bool {
        matches!(self, Insert::Laid | Insert::Hung)
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
}

impl Default for AddState {
    /// Baked in with a soft join, half proud of the surface, a cable with
    /// round ends and a little sag: what reads as something set down on
    /// the model and belonging to it.
    fn default() -> Self {
        Self {
            what: None,
            separate: false,
            blend: 1.0,
            lift: 0.5,
            start: End::Round,
            end: End::Round,
            sag: 0.15,
            lent: None,
            join: Join::new(),
            press: None,
            last: None,
            pieces: Vec::new(),
            cut: false,
        }
    }
}

impl AddState {
    /// Back to an ordinary brush.
    pub(super) fn put_away(&mut self) {
        self.what = None;
        self.lent = None;
    }

    fn how(&self) -> How {
        How { cut: self.cut && !self.separate, blend: self.blend }
    }
}

/// The mirrors of a solid that symmetry asks for: the flips of every
/// combination of the axes that are on, but for no flip at all.
fn flips(symmetry: brokkr_core::Symmetry) -> Vec<Vec3> {
    let on = [MirrorAxis::X, MirrorAxis::Y, MirrorAxis::Z].map(|axis| symmetry.axis(axis));
    (1..8_u8)
        .filter(|bits| (0..3).all(|axis| bits & (1 << axis) == 0 || on[axis]))
        .map(|bits| {
            let side = |axis: u8| if bits & (1 << axis) != 0 { -1.0 } else { 1.0 };
            Vec3::new(side(0), side(1), side(2))
        })
        .collect()
}

impl Brokkr {
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
                state.what = Some(what);
                // One thing in hand at a time: the stamp and a brush's own
                // amounts are put away, as they put each other away.
                self.studio.put_away();
                self.status = match what {
                    Insert::Shape(_) | Insert::Body(_) => {
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
        match what {
            Insert::Shape(_) | Insert::Body(_) => {
                // How far the pointer has gone from the press, in the plane
                // of the surface there: as the stamp is sized.
                let (origin, ray) = self.ray_through(pixel);
                let facing = ray.dot(normal);
                if facing.abs() < 1.0e-3 {
                    return;
                }
                let drag = origin + ray * ((centre - origin).dot(normal) / facing) - centre;
                let seat = match what {
                    Insert::Shape(form) => {
                        Placed::on(form, centre, normal, drag, drag.length(), self.inserts.lift)
                    }
                    _ => Placed::on(
                        Form::Ball,
                        centre,
                        normal,
                        drag,
                        drag.length(),
                        self.inserts.lift,
                    ),
                };
                self.inserts.pieces.clear();
                if let Some(seat) = seat {
                    self.inserts.pieces.push(match what {
                        Insert::Shape(_) => Piece::Placed(seat),
                        _ => Piece::Lent(seat),
                    });
                }
                self.set_pieces();
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
                self.set_pieces();
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

    /// Put the body in hand back as it was at the press and set down what
    /// the gesture now holds: every move of a shape still being dragged.
    fn set_pieces(&mut self) {
        self.inserts.join.clear(self.doc.active_volume_mut());
        for piece in self.inserts.pieces.clone() {
            self.add_piece(piece);
        }
        self.remesh_dirty();
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
        let pieces = std::mem::take(&mut self.inserts.pieces);
        if pieces.is_empty() {
            return;
        }
        if self.doc.body_count() >= brokkr_core::MAX_BODIES
            || self.doc.node_count() >= brokkr_core::MAX_NODES
        {
            self.status = "could not add a body: the document holds as many as it can".to_string();
            return;
        }
        // The pieces, and their mirrors, as solids.
        let flips = flips(self.symmetry);
        let lent: Vec<Lent<'_>> = pieces
            .iter()
            .filter_map(|piece| match (piece, &self.inserts.lent) {
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
        for piece in &pieces {
            match piece {
                Piece::Placed(placed) => solids.push(placed),
                Piece::Step(step) => solids.push(step),
                Piece::Lent(_) => {}
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
        let made = solid::body(self.doc.voxel_size(), &solids);
        drop(solids);
        if made.world_bounds().is_none() {
            self.status = "nothing was set down: drag further".to_string();
            return;
        }
        let name = match what {
            Insert::Shape(form) => form.label(),
            Insert::Body(_) => "Insert",
            Insert::Laid | Insert::Hung => "Cable",
        };
        let name = brokkr_core::name_that_fits(name).to_string();
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
        })
    }

    /// The brushes that add, and the settings of the one in hand.
    pub(super) fn adding_panel(&self) -> Element<'_, Message> {
        let changed = |message: AddMessage| Message::Add(message);
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let state = &self.inserts;
        // Pressed again, the one in hand is put away.
        let choice = |label: String, what: Insert| {
            let live = state.what == Some(what);
            button(text(label).size(theme::CAPTION_SIZE).width(Length::Fill).center())
                .width(Length::Fill)
                .padding(iced::Padding { top: 5.0, bottom: 5.0, left: 2.0, right: 2.0 })
                .style(if live { theme::tool_button_active } else { theme::tool_button })
                .on_press(changed(AddMessage::Chosen((!live).then_some(what))))
        };
        let shapes = Form::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, form| {
            assembled.push(choice(form.label().to_string(), Insert::Shape(form)))
        });
        let cables = row![
            choice("Cable".to_string(), Insert::Laid),
            choice("Hung cable".to_string(), Insert::Hung),
        ]
        .spacing(theme::S1);
        let mut panel =
            column![text("ADD").size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE), shapes, cables,]
                .spacing(theme::S1);
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
        let Some(what) = state.what else {
            return panel.into();
        };
        let way = |label: &'static str, separate: bool| {
            button(text(label).size(theme::CAPTION_SIZE).width(Length::Fill).center())
                .width(Length::Fill)
                .style(if state.separate == separate {
                    theme::tool_button_active
                } else {
                    theme::tool_button
                })
                .on_press(changed(AddMessage::SeparateToggled(separate)))
        };
        let voxel_size = self.doc.voxel_size();
        panel = panel
            .push(row![way("Baked in", false), way("Its own body", true)].spacing(theme::S1))
            .push(reading(if state.separate {
                "Soft join  none: it is not joined".to_string()
            } else if state.blend > 0.0 {
                format!("Soft join  {:.2} mm", state.blend * voxel_size)
            } else {
                "Soft join  none".to_string()
            }))
            .push(
                slider(0.0..=MOST_BLEND, state.blend, move |voxels| {
                    changed(AddMessage::BlendChanged(voxels))
                })
                .step(0.25_f32),
            )
            .push(reading(if what.is_cable() {
                format!("Held off the surface  {:.2} of its radius", state.lift)
            } else {
                format!("Held off the surface  {:.2} of its size", state.lift)
            }))
            .push(
                slider(LIFT_RANGE, state.lift, move |lift| changed(AddMessage::LiftChanged(lift)))
                    .step(0.05_f32),
            );
        if what.is_cable() {
            let ends = |live: End, chosen: fn(End) -> AddMessage| {
                End::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, end| {
                    assembled.push(
                        button(
                            text(end.label())
                                .size(theme::CAPTION_SIZE)
                                .width(Length::Fill)
                                .center(),
                        )
                        .width(Length::Fill)
                        .padding(iced::Padding { top: 5.0, bottom: 5.0, left: 2.0, right: 2.0 })
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
            panel = panel
                .push(reading(if state.sag > 0.0 {
                    format!("Sag  {:.2} of its span", state.sag)
                } else {
                    "Sag  none: a straight run".to_string()
                }))
                .push(
                    slider(SAG_RANGE, state.sag, move |sag| changed(AddMessage::SagChanged(sag)))
                        .step(0.02_f32),
                );
        }
        panel
            .push(
                text(match (what, state.separate) {
                    (Insert::Laid, _) => {
                        "drag along the model. the brush's radius is its thickness"
                    }
                    (Insert::Hung, _) => "press at one end, drag to the other",
                    (_, true) => "press where it goes, drag to size and turn it",
                    (_, false) => "press, drag to size and turn. ctrl or alt takes it out",
                })
                .size(theme::CAPTION_SIZE)
                .color(theme::TEXT_MUTE),
            )
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    fn press(app: &mut Brokkr, at: Vec2) {
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

    fn move_to(app: &mut Brokkr, at: Vec2) {
        let position = iced::Vector::new(at.x, at.y);
        update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
    }

    fn release(app: &mut Brokkr) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A drag from one pixel to another, in a dozen moves.
    fn drag(app: &mut Brokkr, from: Vec2, to: Vec2) {
        press(app, from);
        for step in 1..=12 {
            move_to(app, from.lerp(to, step as f32 / 12.0));
        }
        release(app);
    }

    fn choose(app: &mut Brokkr, what: Insert) {
        update(app, Message::Add(AddMessage::Chosen(Some(what))));
    }

    fn set(app: &mut Brokkr, message: AddMessage) {
        update(app, Message::Add(message));
    }

    const MIDDLE: Vec2 = Vec2::new(400.0, 300.0);

    /// Whether the active body of two applications holds the same values.
    fn same_field(one: &Brokkr, other: &Brokkr) -> bool {
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
    fn solid_at(app: &Brokkr, point: Vec3) -> bool {
        app.doc.active_volume().sample_world(point) < 0.0
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
        // While it is dragged it is shown on the body in hand.
        assert!(!same_field(&app, &self::app()), "nothing was shown during the drag");
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
        assert_eq!(flips(brokkr_core::Symmetry::OFF), Vec::<Vec3>::new());
        let all = brokkr_core::Symmetry::OFF
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
        drop(app.adding_panel());
        update(&mut app, Message::Studio(crate::message::StudioMessage::StampToggled(true)));
        choose(&mut app, Insert::Shape(Form::Torus));
        assert!(app.inserting() && !app.studio.stamping(), "the stamp stayed up");
        drop(app.adding_panel());
        // Pressed again it is put away.
        set(&mut app, AddMessage::Chosen(None));
        assert!(!app.inserting());

        for what in [Insert::Laid, Insert::Hung, Insert::Shape(Form::Box)] {
            choose(&mut app, what);
            drop(app.adding_panel());
            drop(app.view());
        }
        update(&mut app, Message::BrushKindChanged(brokkr_core::BrushKind::Clay));
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
}
