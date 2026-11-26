// SPDX-License-Identifier: AGPL-3.0-only

//! The ring cut, stage 1 of `docs/CUT-RING.md`: drag a ring round what to
//! cut, see the cut it proposes, and split the piece off into a body of its
//! own. The engine is [`brokkr_core::sever`].
//!
//! Asked for by the maintainer on 2026-11-26: a line drawn across an arm
//! is a plane through the whole model and takes the torso too. A ring says
//! which piece. The press is the ring's middle, the drag its radius; when
//! the button comes up the solid under the ring proposes a disc square to
//! its long axis, drawn with the line it would leave on the model, and
//! *Split in two* on the bar makes the piece a body. Another ring replaces
//! the proposal, and Escape drops it. *Line* on the bar is the cut as it
//! was, which cuts straight through everything on purpose.
//!
//! # Stage 2: adjusting it
//!
//! The cut proposed is a ring of its own, which the maintainer asked to be
//! positioned and turned ("people can just rotate it as needed"): a middle,
//! a normal into the piece, an axis across, and a radius. Four handles are
//! drawn on it: on its axis to slide it along, on its rim at the axis
//! across and a quarter turn on to tilt it either way, and on the far side
//! of its rim to widen or narrow it. Each move of a handle fits the cut to
//! what it crosses again ([`brokkr_core::Volume::fit_to_islands`]) and
//! draws the band again; a ring that fits nothing is drawn alone, in red,
//! and Split in two waits. While a cut is proposed, everything but the
//! piece's side of it is screened (*See-through* on the bar), so the band
//! can be seen where the rest of the model hides it.

use brokkr_core::NodeId;
use brokkr_core::sever::{Disc, Unsevered};
use glam::{Vec2, Vec3};
use iced::widget::{button, row, text};
use iced::{Alignment, Element};

use super::Brokkr;
use crate::message::{Message, RingMessage};
use crate::theme;

/// Which way the cut tool reads a drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CutWay {
    /// A ring round what to cut.
    #[default]
    Ring,
    /// A line, curve or loop on the screen, through everything.
    Line,
}

/// A ring being dragged.
#[derive(Debug, Clone, Copy)]
struct Press {
    pixel: Vec2,
    body: NodeId,
    /// The point of the surface under the press, the view's way there,
    /// and the way the surface faces there.
    at: Vec3,
    along: Vec3,
    facing: Vec3,
}

/// The cut a ring proposes, and what it is adjusted by.
#[derive(Debug, Clone)]
pub(super) struct Proposal {
    pub body: NodeId,
    /// The ring as adjusted: its middle, its normal into the piece, the
    /// axis across it that its first tilt handle is on, and its radius.
    pub ring: Ring,
    /// The cut fitted to what the ring crosses, or `None` where it fits
    /// nothing.
    pub disc: Option<Disc>,
    /// Where the cut meets the surface, as a band lying on it: the line it
    /// would leave, a quad a piece.
    contour: Vec<[Vec3; 4]>,
    /// The keys a parting would put across the cut: planned when it is
    /// fitted, and each draggable across the cut face.
    pub keys: Vec<brokkr_core::part::Key>,
}

/// A ring in the model: what the handles move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Ring {
    pub centre: Vec3,
    pub normal: Vec3,
    pub across: Vec3,
    pub radius: f32,
}

impl Ring {
    /// The other axis across, a quarter turn on from `across`.
    fn beside(&self) -> Vec3 {
        self.normal.cross(self.across).normalize_or_zero()
    }

    fn disc(&self) -> Disc {
        Disc::round(self.centre, self.normal, self.radius)
    }

    /// Where each handle is.
    fn handle(&self, which: Handle) -> Vec3 {
        match which {
            Handle::Slide => self.centre + self.normal * self.radius * 1.4,
            Handle::Tilt => self.centre + self.across * self.radius,
            Handle::Turn => self.centre + self.beside() * self.radius,
            Handle::Size => self.centre - self.across * self.radius,
        }
    }
}

/// The handles on a proposed cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle {
    /// Along the normal: slides the cut.
    Slide,
    /// On the rim at the axis across: tilts the cut about the other axis.
    Tilt,
    /// On the rim a quarter turn on: tilts it about the first.
    Turn,
    /// On the far side of the rim: widens or narrows it.
    Size,
}

impl Handle {
    const ALL: [Handle; 4] = [Handle::Slide, Handle::Tilt, Handle::Turn, Handle::Size];
}

/// A handle being dragged: which, from where, and the ring as it was.
#[derive(Debug, Clone, Copy)]
struct Held {
    which: Handle,
    pixel: Vec2,
    ring: Ring,
}

/// What a press on a proposed cut took.
#[derive(Debug, Clone, Copy)]
enum Taken {
    Handle(Held),
    /// A key, by its place in the proposal's list.
    Key(usize),
}

/// The ring cut's state.
#[derive(Debug)]
pub(super) struct RingCut {
    pub way: CutWay,
    press: Option<Press>,
    radius_px: f32,
    pub proposal: Option<Proposal>,
    held: Option<Taken>,
    /// How much of the rest of the model is screened while a cut is
    /// proposed, 0..=1.
    pub see_through: f32,
}

impl Default for RingCut {
    fn default() -> Self {
        Self {
            way: CutWay::default(),
            press: None,
            radius_px: 0.0,
            proposal: None,
            held: None,
            see_through: SEE_THROUGH,
        }
    }
}

/// How much of the rest is screened unless set otherwise.
const SEE_THROUGH: f32 = 0.6;
/// How near a handle a press takes it, in pixels.
const GRAB_PX: f32 = 12.0;
/// How big a handle is drawn, in pixels.
const HANDLE_PX: f32 = 6.0;
const HANDLE: [f32; 4] = [1.0, 0.85, 0.3, 1.0];
const MISFIT: [f32; 4] = [0.9, 0.2, 0.15, 0.9];
const KEY: [f32; 4] = [0.35, 0.8, 1.0, 1.0];

/// Segments round the ring and the disc as they are drawn.
const SEGMENTS: usize = 64;
/// Cells across the disc that its contour is traced in.
const CONTOUR_CELLS: usize = 96;
/// A ring smaller than this, in pixels, is a click and proposes nothing.
const SMALLEST_PX: f32 = 6.0;

/// How wide the band on the surface is drawn, in pixels.
const BAND_PX: f32 = 5.0;

const RING: [f32; 4] = [1.0, 0.55, 0.15, 0.9];
const BAND: [f32; 4] = [1.0, 0.7, 0.2, 0.95];

impl Brokkr {
    /// Whether the cut tool is in hand and reads a drag as a ring.
    pub(super) fn rings(&self) -> bool {
        self.tool == super::Tool::Cut && self.ring_cut.way == CutWay::Ring
    }

    /// The press of a ring: its middle, on a body.
    pub(super) fn ring_pressed(&mut self, pixel: Vec2) {
        self.ring_cut.press = None;
        self.ring_cut.radius_px = 0.0;
        self.ring_cut.held = None;
        // A handle of the cut proposed, if the press is on one.
        if let Some(proposal) = &self.ring_cut.proposal {
            let ring = proposal.ring;
            let nearest = Handle::ALL
                .into_iter()
                .filter_map(|which| {
                    let at = super::measure::to_pixels(
                        &self.camera,
                        ring.handle(which),
                        self.viewport_size,
                    )?;
                    Some((which, at.distance(pixel)))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((which, distance)) = nearest
                && distance <= GRAB_PX
            {
                self.ring_cut.held = Some(Taken::Handle(Held { which, pixel, ring }));
                return;
            }
            // A key's middle, to drag it across the cut face.
            let key = proposal
                .keys
                .iter()
                .enumerate()
                .filter_map(|(at, key)| {
                    let on =
                        super::measure::to_pixels(&self.camera, key.centre, self.viewport_size)?;
                    Some((at, on.distance(pixel)))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((at, distance)) = key
                && distance <= GRAB_PX
            {
                self.ring_cut.held = Some(Taken::Key(at));
                return;
            }
        }
        let Some((body, hit)) = self.pick(pixel) else {
            self.status = "start the ring on the model".to_string();
            return;
        };
        let (_, along) = self.ray_through(pixel);
        self.ring_cut.press =
            Some(Press { pixel, body, at: hit.position, along, facing: hit.normal });
        self.refresh_overlay();
    }

    /// A move of the ring: its radius.
    pub(super) fn ring_moved(&mut self, pixel: Vec2) {
        match self.ring_cut.held {
            Some(Taken::Handle(held)) => {
                self.move_handle(held, pixel);
                return;
            }
            Some(Taken::Key(at)) => {
                self.move_key(at, pixel);
                return;
            }
            None => {}
        }
        let Some(press) = self.ring_cut.press else {
            return;
        };
        self.ring_cut.radius_px = press.pixel.distance(pixel);
        self.refresh_overlay();
    }

    /// The ring let go: the cut it proposes.
    pub(super) fn ring_released(&mut self) {
        if self.ring_cut.held.take().is_some() {
            return;
        }
        let Some(press) = self.ring_cut.press.take() else {
            return;
        };
        if self.ring_cut.radius_px < SMALLEST_PX {
            self.refresh_overlay();
            return;
        }
        let Some(volume) = self.doc.volume(press.body) else {
            return;
        };
        // The ring's radius in millimetres at the depth of the disc it
        // proposes, which is the middle of the solid and not its front: at
        // the front, a ring drawn wider than a ball was a disc smaller than
        // the ball, inside it and out of sight (seen on the screen). So the
        // disc is found once at the front's scale and again at its own.
        let px = self.ring_cut.radius_px;
        let first = px * self.world_per_pixel(press.at);
        let Some(near) = volume.disc_across(press.at, press.along, first) else {
            self.status = "nothing solid under the ring".to_string();
            self.refresh_overlay();
            return;
        };
        let radius = px * self.world_per_pixel(near.centre);
        // The narrowest cut round what the ring is round, fitted to the
        // pieces it crosses. See `brokkr_core::sever::Volume::propose_cut`.
        let Some(disc) = volume.propose_cut(press.at, press.along, press.facing, radius) else {
            self.ring_cut.proposal = None;
            self.status =
                "the ring is not round a part: draw it round a limb or a spur".to_string();
            self.refresh_overlay();
            return;
        };
        // The ring the handles move: the cut's plane and middle, its normal
        // turned into the piece, at the radius the ring was drawn.
        let normal =
            if piece_is_in_front(volume, &disc, radius) { disc.normal } else { -disc.normal };
        let ring =
            Ring { centre: disc.centre, normal, across: normal.any_orthonormal_vector(), radius };
        self.ring_cut.proposal = Some(Proposal {
            body: press.body,
            ring,
            disc: None,
            contour: Vec::new(),
            keys: Vec::new(),
        });
        self.fit_the_ring();
        self.status = format!("a ring round {:.1} mm", radius * 2.0);
        self.refresh_overlay();
    }

    /// Fit the proposed ring to what it crosses, and its band, again.
    fn fit_the_ring(&mut self) {
        let Some(proposal) = &self.ring_cut.proposal else {
            return;
        };
        let (body, ring) = (proposal.body, proposal.ring);
        let Some(volume) = self.doc.volume(body) else {
            return;
        };
        let disc = volume.fit_to_islands(&ring.disc());
        let width = BAND_PX * self.world_per_pixel(ring.centre);
        let band = disc.as_ref().map(|disc| contour(volume, disc, width)).unwrap_or_default();
        // Planned again with the cut: a key moved by hand stays only until
        // the cut itself is moved.
        let keys =
            disc.as_ref().map(|disc| volume.plan_keys(disc, self.key_spec)).unwrap_or_default();
        if let Some(proposal) = self.ring_cut.proposal.as_mut() {
            proposal.disc = disc;
            proposal.contour = band;
            proposal.keys = keys;
        }
    }

    /// A key dragged to `pixel`: to where the pointer's ray meets the cut's
    /// plane, if that is within the cut.
    fn move_key(&mut self, at: usize, pixel: Vec2) {
        let (origin, ray) = self.ray_through(pixel);
        let Some(proposal) = self.ring_cut.proposal.as_mut() else {
            return;
        };
        let Some(disc) = &proposal.disc else {
            return;
        };
        let facing = ray.dot(disc.normal);
        if facing.abs() < 1e-4 {
            return;
        }
        let point = origin + ray * ((disc.centre - origin).dot(disc.normal) / facing);
        if let Some(key) = proposal.keys.get_mut(at)
            && disc.within(point)
        {
            key.centre = point;
        }
        self.refresh_overlay();
    }

    /// A handle dragged to `pixel`: the ring moved from where it was when
    /// the handle was taken, by the drag's pixels measured along the way
    /// the handle moves on the screen.
    fn move_handle(&mut self, held: Held, pixel: Vec2) {
        let ring = held.ring;
        let size = self.viewport_size;
        let on_screen = |point: Vec3| super::measure::to_pixels(&self.camera, point, size);
        let drag = pixel - held.pixel;
        // How far along `way` (from `from`, a radius long in the model) the
        // drag went, in radii.
        let along = |from: Vec3, way: Vec3| -> Option<f32> {
            let (a, b) = (on_screen(from)?, on_screen(from + way * ring.radius)?);
            let span = b - a;
            let length = span.length();
            (length > 1.0).then(|| drag.dot(span / length) / length)
        };
        let moved = match held.which {
            Handle::Slide => along(ring.centre, ring.normal).map(|radii| Ring {
                centre: ring.centre + ring.normal * radii * ring.radius,
                ..ring
            }),
            Handle::Tilt => along(ring.handle(Handle::Tilt), ring.normal).map(|turn| {
                let (sin, cos) = turn.sin_cos();
                Ring {
                    normal: (ring.normal * cos - ring.across * sin).normalize(),
                    across: (ring.across * cos + ring.normal * sin).normalize(),
                    ..ring
                }
            }),
            Handle::Turn => along(ring.handle(Handle::Turn), ring.normal).map(|turn| {
                let (sin, cos) = turn.sin_cos();
                let beside = ring.beside();
                Ring { normal: (ring.normal * cos - beside * sin).normalize(), ..ring }
            }),
            Handle::Size => along(ring.handle(Handle::Size), -ring.across).map(|grown| Ring {
                radius: (ring.radius * (1.0 + grown)).max(self.doc.voxel_size() * 4.0),
                ..ring
            }),
        };
        let Some(moved) = moved else {
            return;
        };
        if let Some(proposal) = self.ring_cut.proposal.as_mut() {
            proposal.ring = moved;
        }
        self.fit_the_ring();
        self.refresh_overlay();
    }

    /// World units a pixel spans at the depth of `at`.
    fn world_per_pixel(&self, at: Vec3) -> f32 {
        let away = at.distance(self.camera.eye());
        2.0 * away * (self.camera.fov_y * 0.5).tan() / self.viewport_size.y.max(1.0)
    }

    pub(super) fn ring_changed(&mut self, message: RingMessage) {
        match message {
            RingMessage::Way(way) => {
                self.ring_cut.way = way;
                self.ring_cut.proposal = None;
                self.ring_cut.press = None;
            }
            RingMessage::Split => self.split_by_the_ring(false),
            RingMessage::Part => self.split_by_the_ring(true),
            RingMessage::Dropped => {
                self.ring_cut.proposal = None;
                self.ring_cut.press = None;
            }
            RingMessage::SeeThrough(amount) => self.ring_cut.see_through = amount.clamp(0.0, 1.0),
        }
        self.refresh_overlay();
    }

    /// Whether Escape has a ring to drop, and drop it.
    pub(super) fn ring_escaped(&mut self) -> bool {
        let had = self.ring_cut.proposal.is_some() || self.ring_cut.press.is_some();
        self.ring_cut.proposal = None;
        self.ring_cut.press = None;
        if had {
            self.refresh_overlay();
        }
        had
    }

    /// Split the piece the proposed disc severs into a body of its own,
    /// keyed to the rest when `keyed`.
    fn split_by_the_ring(&mut self, keyed: bool) {
        let Some(proposal) = self.ring_cut.proposal.clone() else {
            return;
        };
        let Some(disc) = proposal.disc else {
            self.status = "not split: the ring is not round a part where it is".to_string();
            return;
        };
        let source = proposal.body;
        let name = self.doc.node(source).map(|node| node.name.clone()).unwrap_or_default();
        if let Some(why) = self.doc.split_guard(source) {
            self.status = format!("could not split {name}: {why}");
            return;
        }
        let keys = if keyed { proposal.keys.clone() } else { Vec::new() };
        if keyed && keys.is_empty() {
            self.status = format!("not parted: the cut across {name} has no room for a key");
            return;
        }
        match self.doc.sever_keyed(source, &disc, &keys) {
            Ok(outcome) => {
                self.ring_cut.proposal = None;
                self.shared.forget_body(source);
                self.doc.mark_everything_dirty();
                let before = self.history.stats();
                self.history.push(outcome.entry);
                self.unsaved = true;
                self.record_history(before);
                self.status = match (outcome.pieces, keyed) {
                    (_, true) => format!("parted {name} at the ring with {} keys", keys.len()),
                    (1, false) => format!("split {name} in two at the ring"),
                    (pieces, false) => format!("split {pieces} pieces off {name} at the ring"),
                };
                self.tool = super::Tool::Sculpt;
                self.refresh_model_radius();
                self.remesh_dirty();
            }
            Err(Unsevered::NotRound) => {
                self.status = format!(
                    "not split: the ring does not go all the way round that part of {name}"
                );
            }
            Err(Unsevered::Misses) => {
                self.status = format!("not split: the ring's cut misses {name}");
            }
            Err(Unsevered::NoBody) => {}
        }
    }

    /// The ring being dragged, or the cut it proposed, over the model.
    pub(super) fn add_ring_preview(&self, batch: &mut brokkr_gpu::OverlayBatch) {
        if self.tool != super::Tool::Cut {
            self.publish_ghost();
            return;
        }
        if let Some(press) = self.ring_cut.press
            && self.ring_cut.radius_px >= SMALLEST_PX
        {
            // A ring square to the camera through the point pressed, so that
            // it is a circle round the press on the screen: square to the
            // ray instead, it sat off to one side.
            let radius = self.ring_cut.radius_px * self.world_per_pixel(press.at);
            let facing = (press.at - self.camera.eye()).normalize_or(press.along);
            let forward = (self.camera.target - self.camera.eye()).normalize_or(facing);
            let (u, v) = forward.any_orthonormal_pair();
            circle(batch, press.at, u, v, radius, RING);
        }
        // The cut proposed, as a band on the surface where it would part
        // it: asked for by the maintainer on 2026-11-26 ("drag a rubber band
        // on the surface ... the disc projects down to a band"), with the
        // ring it is adjusted by and its handles.
        if let Some(proposal) = &self.ring_cut.proposal {
            for [a, b, c, d] in &proposal.contour {
                batch.push_triangle(*a, *b, *c, BAND);
                batch.push_triangle(*a, *c, *d, BAND);
            }
            let ring = proposal.ring;
            let rim = if proposal.disc.is_some() { RING } else { MISFIT };
            circle(batch, ring.centre, ring.across, ring.beside(), ring.radius, rim);
            batch.push_line(ring.centre, ring.handle(Handle::Slide), RING);
            let toward_eye = (self.camera.eye() - ring.centre).normalize_or(ring.normal);
            let (u, v) = toward_eye.any_orthonormal_pair();
            for which in Handle::ALL {
                let half = HANDLE_PX * self.world_per_pixel(ring.handle(which));
                // Lifted toward the eye: on the rim they sit where it meets
                // the surface, and the surface hid three of the four (seen
                // on the screen).
                // Along its own line of sight, so that it stays where it is
                // on the screen: lifted along one shared direction, those
                // off to the side of the view slid off the rim.
                let point = ring.handle(which);
                let at = point + (self.camera.eye() - point).normalize_or(toward_eye) * half * 3.0;
                let corner = |a: f32, b: f32| at + (u * a + v * b) * half;
                batch.push_triangle(
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    HANDLE,
                );
                batch.push_triangle(
                    corner(-1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                    HANDLE,
                );
            }
        }
        if let Some(proposal) = &self.ring_cut.proposal {
            let ring = proposal.ring;
            // Each key on the cut face: its rim, and its middle to drag.
            for key in &proposal.keys {
                circle(batch, key.centre, ring.across, ring.beside(), key.radius_mm, HANDLE);
                let half = HANDLE_PX * 0.6 * self.world_per_pixel(key.centre);
                let lift = (self.camera.eye() - key.centre).normalize_or(ring.normal) * half * 3.0;
                let at = key.centre + lift;
                let (u, v) =
                    (self.camera.eye() - at).normalize_or(ring.normal).any_orthonormal_pair();
                let corner = |a: f32, b: f32| at + (u * a + v * b) * half;
                batch.push_triangle(corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), KEY);
                batch.push_triangle(corner(-1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0), KEY);
            }
        }
        self.publish_ghost();
    }

    /// The see-through view: while a cut is proposed, the piece's side of
    /// it (within four radii) solid and the rest screened; otherwise off.
    fn publish_ghost(&self) {
        let view = match &self.ring_cut.proposal {
            Some(proposal) if self.tool == super::Tool::Cut => {
                let ring = proposal.ring;
                // A couple of voxels behind the plane, so the cut face itself
                // is never screened.
                let offset = ring.normal.dot(ring.centre) - self.doc.voxel_size() * 2.0;
                crate::viewport::GhostView {
                    plane: [ring.normal.x, ring.normal.y, ring.normal.z, offset],
                    keep: [ring.centre.x, ring.centre.y, ring.centre.z, ring.radius * 4.0],
                    amount: self.ring_cut.see_through,
                    // Half as much: the keys on the cut face are inside the
                    // piece, and drawn solid it hid them.
                    kept_amount: self.ring_cut.see_through * 0.5,
                }
            }
            _ => crate::viewport::GhostView::default(),
        };
        self.shared.set_ghost_view(view);
    }

    /// The ring cut's part of the bar: Ring or Line, and while a cut is
    /// proposed, Split in two.
    pub(super) fn ring_cut_bar(&self) -> Element<'_, Message> {
        let mut line = row![crate::ui::segmented(
            [(CutWay::Ring, "Ring"), (CutWay::Line, "Line")],
            self.ring_cut.way,
            |way| Message::Ring(RingMessage::Way(way)),
        )]
        .spacing(theme::S2)
        .align_y(Alignment::Center);
        if let Some(proposal) = &self.ring_cut.proposal {
            line = line
                .push(
                    crate::ui::number(
                        &self.drafts,
                        "ring.see_through",
                        "See-through",
                        self.ring_cut.see_through,
                        0.0..=1.0,
                        |amount| Message::Ring(RingMessage::SeeThrough(amount)),
                    )
                    .shown(format!("{:.0}%", self.ring_cut.see_through * 100.0))
                    .inline(),
                )
                .push(
                    button(text("Split in two").size(theme::LABEL_SIZE))
                        .padding(theme::PAD_CHOICE)
                        .style(theme::tool_button_active)
                        .on_press_maybe(
                            proposal.disc.is_some().then_some(Message::Ring(RingMessage::Split)),
                        ),
                )
                .push(
                    button(text("Part with pegs").size(theme::LABEL_SIZE))
                        .padding(theme::PAD_CHOICE)
                        .style(theme::tool_button)
                        .on_press_maybe(
                            (proposal.disc.is_some() && !proposal.keys.is_empty())
                                .then_some(Message::Ring(RingMessage::Part)),
                        ),
                )
                .push(crate::ui::button_secondary(
                    "Drop",
                    Some(Message::Ring(RingMessage::Dropped)),
                ));
        }
        line.into()
    }
}

/// Whether the piece a cut takes off is in front of its plane: the side
/// with less solid within three radii of its middle, the body being on the
/// other.
fn piece_is_in_front(volume: &brokkr_core::Volume, disc: &Disc, radius: f32) -> bool {
    let reach = radius * 3.0;
    let pitch = (reach * 2.0 / 24.0).max(volume.voxel_size());
    let steps = (reach / pitch).ceil() as i32;
    let (mut front, mut back) = (0_usize, 0_usize);
    for z in -steps..=steps {
        for y in -steps..=steps {
            for x in -steps..=steps {
                let offset = Vec3::new(x as f32, y as f32, z as f32) * pitch;
                if offset.length_squared() > reach * reach
                    || volume.sample_world(disc.centre + offset) >= 0.0
                {
                    continue;
                }
                if offset.dot(disc.normal) >= 0.0 { front += 1 } else { back += 1 }
            }
        }
    }
    front <= back
}

/// A circle of `radius` round `centre` in the plane of `u` and `v`.
fn circle(
    batch: &mut brokkr_gpu::OverlayBatch,
    centre: Vec3,
    u: Vec3,
    v: Vec3,
    radius: f32,
    colour: [f32; 4],
) {
    let at = |step: usize| {
        let turn = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        centre + (u * turn.cos() + v * turn.sin()) * radius
    };
    for step in 0..SEGMENTS {
        batch.push_line(at(step), at(step + 1), colour);
    }
}

/// Where the disc meets the surface, traced on a grid across it by marching
/// squares, as a band `width` wide lying on the surface and lifted a voxel
/// off it: the line the cut would leave on the model.
fn contour(volume: &brokkr_core::Volume, disc: &Disc, width: f32) -> Vec<[Vec3; 4]> {
    let (u, v) = disc.axes();
    let cells = CONTOUR_CELLS;
    let pitch = disc.radius * 2.0 / cells as f32;
    let point = |i: usize, j: usize| {
        disc.centre + u * (i as f32 * pitch - disc.radius) + v * (j as f32 * pitch - disc.radius)
    };
    let values: Vec<f32> = (0..=cells)
        .flat_map(|j| (0..=cells).map(move |i| (i, j)))
        .map(|(i, j)| volume.sample_world(point(i, j)))
        .collect();
    let value = |i: usize, j: usize| values[j * (cells + 1) + i];
    let lift = volume.voxel_size();
    let mut band = Vec::new();
    for j in 0..cells {
        for i in 0..cells {
            if !disc.within((point(i, j) + point(i + 1, j + 1)) * 0.5) {
                continue;
            }
            let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let mut crossings = Vec::with_capacity(4);
            for edge in 0..4 {
                let (a, b) = (corners[edge], corners[(edge + 1) % 4]);
                let (da, db) = (value(a.0, a.1), value(b.0, b.1));
                if (da < 0.0) != (db < 0.0) {
                    let t = da / (da - db);
                    crossings.push(point(a.0, a.1).lerp(point(b.0, b.1), t));
                }
            }
            for [from, to] in crossings.as_chunks::<2>().0.iter().copied() {
                // Across the line in the surface: square to the line and to
                // the surface's normal there.
                let normal = volume.gradient_world((from + to) * 0.5).normalize_or(disc.normal);
                let across = normal.cross(to - from).normalize_or(disc.normal) * (width * 0.5);
                let up = normal * lift;
                band.push([
                    from - across + up,
                    to - across + up,
                    to + across + up,
                    from + across + up,
                ]);
            }
        }
    }
    band
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    /// Set `disc` as the cut proposed on `body`, as a ring would.
    fn set_proposal(app: &mut Brokkr, body: NodeId, disc: &Disc) {
        let ring = Ring {
            centre: disc.centre,
            normal: disc.normal,
            across: disc.normal.any_orthonormal_vector(),
            radius: disc.radius,
        };
        app.ring_cut.proposal = Some(Proposal {
            body,
            ring,
            disc: Some(disc.clone()),
            contour: Vec::new(),
            keys: Vec::new(),
        });
    }

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn ring(app: &mut Brokkr, at: Vector, to: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
        update(app, Message::Pointer(PointerEvent::Moved { position: to, size: SIZE }));
        update(app, Message::Frame);
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// The cut tool reads a drag as a ring unless told Line. A ring over
    /// the ball it opens with, small or wider than the ball, is round no
    /// part of it, proposes nothing, and says so.
    #[test]
    fn a_ring_over_a_lone_ball_proposes_nothing() {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::ToolChanged(super::super::Tool::Cut));
        assert!(app.rings());
        let middle = Vector { x: 400.0, y: 300.0 };
        ring(&mut app, middle, Vector { x: 430.0, y: 300.0 });
        // A ring over the middle of the ball is round no part of it: the
        // ball's cross-section runs far past it.
        assert!(app.ring_cut.proposal.is_none(), "a ring inside the ball proposed a cut");
        assert!(app.status.contains("not round a part"), "{}", app.status);

        // Nor is a ring round the whole ball: a ball alone has no neck,
        // and halving it is the line cut's.
        ring(&mut app, middle, Vector { x: 790.0, y: 590.0 });
        assert!(app.ring_cut.proposal.is_none(), "a ring round a lone ball proposed a cut");

        update(&mut app, Message::ToolChanged(super::super::Tool::Cut));
        update(&mut app, Message::Ring(RingMessage::Way(CutWay::Line)));
        assert!(!app.rings());
    }

    /// A body with an arm: a ring round the arm splits it off as a body of
    /// its own, the torso whole, in one undo.
    #[test]
    fn a_ring_round_an_arm_splits_it_off() {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        let voxel = app.doc.voxel_size();
        // A ball for a body and a rod for an arm, as one shape.
        let mut figure = brokkr_core::Volume::new(voxel);
        figure.edit_box(Vec3::splat(-11.0), Vec3::new(26.0, 11.0, 11.0), move |position, _| {
            let ball = position.length() - 8.0;
            let along = (position.x - 14.0).abs() - 9.0;
            let round = Vec3::new(0.0, position.y, position.z).length() - 2.0;
            (ball.min(along.max(round)) / voxel)
                .clamp(brokkr_core::brick::INSIDE, brokkr_core::brick::OUTSIDE)
        });
        app.doc.replace_active_volume(figure);
        update(&mut app, Message::ToolChanged(super::super::Tool::Cut));
        // The cut a ring round the arm proposes, set as if proposed: where
        // the arm is on the screen depends on the camera.
        let body = app.doc.active();
        let volume = app.doc.active_volume();
        let disc = volume
            .propose_cut(Vec3::new(16.0, 0.0, 2.0), -Vec3::Z, Vec3::Z, 4.0)
            .expect("a cut round the arm");
        assert!(
            disc.normal.dot(Vec3::X).abs() > 0.9,
            "the cut is not across the arm: {:?}",
            disc.normal
        );
        let band = contour(volume, &disc, 0.3);
        assert!(!band.is_empty(), "no band round the arm");
        set_proposal(&mut app, body, &disc);
        if let Some(proposal) = app.ring_cut.proposal.as_mut() {
            proposal.contour = band;
        }
        let mut batch = brokkr_gpu::OverlayBatch::default();
        app.add_ring_preview(&mut batch);
        assert!(!batch.surfaces.is_empty(), "the band was not drawn");
        drop(app.ring_cut_bar());
        update(&mut app, Message::Ring(RingMessage::Split));
        assert_eq!(app.doc.bodies().count(), 2, "{}", app.status);
        assert!(
            app.doc.active_volume().sample_world(Vec3::new(20.0, 0.0, 0.0)) < 0.0,
            "the arm is not the piece"
        );
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.bodies().count(), 1);
    }

    /// A body with an arm along X, as one shape, in an application with
    /// the cut tool in hand and a cut proposed square across the arm.
    fn armed_with_a_cut() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        let voxel = app.doc.voxel_size();
        let mut figure = brokkr_core::Volume::new(voxel);
        figure.edit_box(Vec3::splat(-11.0), Vec3::new(26.0, 11.0, 11.0), move |position, _| {
            let ball = position.length() - 8.0;
            let along = (position.x - 14.0).abs() - 9.0;
            let round = Vec3::new(0.0, position.y, position.z).length() - 2.0;
            (ball.min(along.max(round)) / voxel)
                .clamp(brokkr_core::brick::INSIDE, brokkr_core::brick::OUTSIDE)
        });
        app.doc.replace_active_volume(figure);
        update(&mut app, Message::ToolChanged(super::super::Tool::Cut));
        let body = app.doc.active();
        set_proposal(&mut app, body, &Disc::round(Vec3::new(16.0, 0.0, 0.0), Vec3::X, 4.0));
        app.fit_the_ring();
        app
    }

    /// Drag a handle of the proposed cut by `by` pixels, through the
    /// pointer as a user does.
    fn drag_handle(app: &mut Brokkr, which: Handle, by: Vector) {
        let ring = app.ring_cut.proposal.as_ref().expect("a cut").ring;
        let at =
            super::super::measure::to_pixels(&app.camera, ring.handle(which), app.viewport_size)
                .expect("the handle is on the screen");
        let at = Vector { x: at.x, y: at.y };
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
        update(app, Message::Pointer(PointerEvent::Moved { position: at + by, size: SIZE }));
        update(app, Message::Frame);
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// The handles move the cut: the slide handle along its normal, the
    /// tilt handles turn it, the size handle widens it, each fitting the
    /// cut again; a press away from every handle is a new ring and leaves
    /// the cut alone until it is let go.
    #[test]
    fn the_handles_slide_turn_and_size_the_cut() {
        let mut app = armed_with_a_cut();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        let before = app.ring_cut.proposal.as_ref().expect("a cut").ring;
        assert!(app.ring_cut.proposal.as_ref().is_some_and(|proposal| proposal.disc.is_some()));
        // Along the normal's way on the screen.
        let way = |app: &Brokkr, from: Vec3, to: Vec3| {
            let size = app.viewport_size;
            let a = super::super::measure::to_pixels(&app.camera, from, size).expect("on screen");
            let b = super::super::measure::to_pixels(&app.camera, to, size).expect("on screen");
            let d = (b - a).normalize_or_zero() * 30.0;
            Vector { x: d.x, y: d.y }
        };
        let push = way(&app, before.centre, before.centre + before.normal);
        drag_handle(&mut app, Handle::Slide, push);
        let slid = app.ring_cut.proposal.as_ref().expect("a cut").ring;
        let moved = (slid.centre - before.centre).dot(before.normal);
        assert!(moved > 0.5, "the slide handle moved the cut {moved} along its normal");
        assert!((slid.normal - before.normal).length() < 1e-4, "sliding turned it");

        let tilt = way(&app, slid.handle(Handle::Tilt), slid.handle(Handle::Tilt) + slid.normal);
        drag_handle(&mut app, Handle::Tilt, tilt);
        let tilted = app.ring_cut.proposal.as_ref().expect("a cut").ring;
        assert!(tilted.normal.dot(slid.normal) < 0.995, "the tilt handle did not turn the cut");
        assert!((tilted.normal.length() - 1.0).abs() < 1e-4);

        let grow =
            way(&app, tilted.handle(Handle::Size), tilted.handle(Handle::Size) - tilted.across);
        drag_handle(&mut app, Handle::Size, grow);
        let grown = app.ring_cut.proposal.as_ref().expect("a cut").ring;
        assert!(grown.radius > tilted.radius, "the size handle did not widen the cut");
        assert!(
            app.ring_cut.proposal.as_ref().is_some_and(|proposal| proposal.disc.is_some()),
            "it no longer fits"
        );
    }

    /// While a cut is proposed the rest is screened by the see-through
    /// setting, the piece's side kept; dropped, or with another tool, it is
    /// off.
    #[test]
    fn the_rest_is_see_through_while_a_cut_is_proposed() {
        let mut app = armed_with_a_cut();
        update(&mut app, Message::Ring(RingMessage::SeeThrough(0.4)));
        let view = app.shared.ghost_view();
        assert_eq!(view.amount, 0.4);
        let ring = app.ring_cut.proposal.as_ref().expect("a cut").ring;
        assert_eq!(view.plane[..3], ring.normal.to_array());
        update(&mut app, Message::Ring(RingMessage::Dropped));
        assert_eq!(app.shared.ghost_view().amount, 0.0, "dropped, the rest is still screened");
    }

    /// A cut turned so that it fits nothing is not split, and says so.
    #[test]
    fn a_cut_that_fits_nothing_is_not_split() {
        let mut app = armed_with_a_cut();
        let body = app.doc.active();
        // Lengthwise down the arm into the ball: it crosses the body too.
        set_proposal(&mut app, body, &Disc::round(Vec3::new(10.0, 0.0, 0.0), Vec3::Y, 4.0));
        app.fit_the_ring();
        assert!(app.ring_cut.proposal.as_ref().is_some_and(|proposal| proposal.disc.is_none()));
        drop(app.ring_cut_bar());
        update(&mut app, Message::Ring(RingMessage::Split));
        assert_eq!(app.doc.bodies().count(), 1);
        assert!(app.status.contains("not round a part"), "{}", app.status);
    }

    /// Keys are planned on a cut that fits; one dragged through the
    /// pointer moves across the cut face and stays on it; Part with pegs
    /// parts the arm off with a peg standing in its socket, one undo.
    #[test]
    fn part_with_pegs_keys_the_arm_to_the_body() {
        let mut app = armed_with_a_cut();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        app.key_spec = brokkr_core::part::KeySpec {
            radius_mm: 0.6,
            depth_mm: 1.0,
            clearance_mm: 0.1,
            wall_mm: 0.4,
            ..brokkr_core::part::KeySpec::peg()
        };
        app.fit_the_ring();
        let proposal = app.ring_cut.proposal.clone().expect("a cut");
        assert!(!proposal.keys.is_empty(), "no key planned on the arm's face");
        let key = proposal.keys[0];

        // Drag the key a little across the face.
        let at = super::super::measure::to_pixels(&app.camera, key.centre, app.viewport_size)
            .expect("on screen");
        let start = Vector { x: at.x, y: at.y };
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: start,
                size: SIZE,
            }),
        );
        update(
            &mut app,
            Message::Pointer(PointerEvent::Moved {
                position: start + Vector { x: 0.0, y: 3.0 },
                size: SIZE,
            }),
        );
        update(&mut app, Message::Frame);
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        let moved = app.ring_cut.proposal.as_ref().expect("a cut").keys[0];
        let disc = app
            .ring_cut
            .proposal
            .as_ref()
            .and_then(|proposal| proposal.disc.clone())
            .expect("fits");
        assert!(moved.centre != key.centre, "the key did not move");
        assert!(
            disc.side(moved.centre).abs() < 1e-3 && disc.within(moved.centre),
            "it left the face"
        );

        update(&mut app, Message::Ring(RingMessage::Part));
        assert_eq!(app.doc.bodies().count(), 2, "{}", app.status);
        assert!(app.status.contains("keys"), "{}", app.status);
        let tip = moved.centre + moved.axis * (moved.depth_mm * 0.5);
        let solid_in = |app: &Brokkr| {
            app.doc.bodies().filter(|(_, volume)| volume.sample_world(tip) < 0.0).count()
        };
        assert_eq!(solid_in(&app), 1, "the peg's tip is in neither body, or in both");
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.bodies().count(), 1);
    }

    /// What a split at a ring costs, and what a parting along a line
    /// costs, on the ball the application opens with and on the same ball
    /// at a quarter of the voxel, timed through the application. A
    /// measurement, run by hand:
    /// `cargo test --release -p brokkr-app -- --ignored what_a_split_costs --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn what_a_split_costs() {
        for voxel in [super::super::VOXEL_SIZE_MM, super::super::VOXEL_SIZE_MM / 4.0] {
            let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
            if voxel != super::super::VOXEL_SIZE_MM {
                update(&mut app, Message::Resample(voxel));
            }
            update(&mut app, Message::Frame);
            let body = app.doc.active();
            let disc = Disc::round(Vec3::new(0.1, 0.2, 0.3), Vec3::X, 40.0);
            set_proposal(&mut app, body, &disc);
            let started = std::time::Instant::now();
            update(&mut app, Message::Ring(RingMessage::Split));
            let split = started.elapsed().as_secs_f32() * 1000.0;
            assert_eq!(app.doc.bodies().count(), 2, "{}", app.status);
            let started = std::time::Instant::now();
            update(&mut app, Message::Frame);
            let frame = started.elapsed().as_secs_f32() * 1000.0;
            eprintln!(
                "voxel {voxel} mm, {} bricks: the split {split:.0} ms, the frame after it {frame:.0} ms",
                app.doc.bodies().map(|(_, volume)| volume.brick_count()).sum::<usize>()
            );
        }
    }
}
