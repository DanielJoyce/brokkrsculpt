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
    /// The point of the surface under the press, and the view's way there.
    at: Vec3,
    along: Vec3,
}

/// The cut a ring proposes.
#[derive(Debug, Clone)]
pub(super) struct Proposal {
    pub body: NodeId,
    pub disc: Disc,
    /// Where the disc meets the surface, as a band lying on it: the line
    /// the cut would leave, a quad a piece.
    contour: Vec<[Vec3; 4]>,
}

/// The ring cut's state.
#[derive(Debug, Default)]
pub(super) struct RingCut {
    pub way: CutWay,
    press: Option<Press>,
    radius_px: f32,
    pub proposal: Option<Proposal>,
}

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
        let Some((body, hit)) = self.pick(pixel) else {
            self.status = "start the ring on the model".to_string();
            return;
        };
        let (_, along) = self.ray_through(pixel);
        self.ring_cut.press = Some(Press { pixel, body, at: hit.position, along });
        self.refresh_overlay();
    }

    /// A move of the ring: its radius.
    pub(super) fn ring_moved(&mut self, pixel: Vec2) {
        let Some(press) = self.ring_cut.press else {
            return;
        };
        self.ring_cut.radius_px = press.pixel.distance(pixel);
        self.refresh_overlay();
    }

    /// The ring let go: the cut it proposes.
    pub(super) fn ring_released(&mut self) {
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
        let disc = volume.disc_across(press.at, press.along, radius).unwrap_or(near);
        let width = BAND_PX * self.world_per_pixel(disc.centre);
        let contour = contour(volume, &disc, width);
        self.ring_cut.proposal = Some(Proposal { body: press.body, disc, contour });
        self.status = format!("a ring round {:.1} mm", radius * 2.0);
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
            RingMessage::Split => self.split_by_the_ring(),
            RingMessage::Dropped => {
                self.ring_cut.proposal = None;
                self.ring_cut.press = None;
            }
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

    /// Split the piece the proposed disc severs into a body of its own.
    fn split_by_the_ring(&mut self) {
        let Some(proposal) = self.ring_cut.proposal.clone() else {
            return;
        };
        let source = proposal.body;
        let name = self.doc.node(source).map(|node| node.name.clone()).unwrap_or_default();
        if let Some(why) = self.doc.split_guard(source) {
            self.status = format!("could not split {name}: {why}");
            return;
        }
        match self.doc.sever(source, &proposal.disc) {
            Ok(outcome) => {
                self.ring_cut.proposal = None;
                self.shared.forget_body(source);
                self.doc.mark_everything_dirty();
                let before = self.history.stats();
                self.history.push(outcome.entry);
                self.unsaved = true;
                self.record_history(before);
                self.status = format!("split {name} in two at the ring");
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
        // on the surface ... the disc projects down to a band"). The disc it
        // comes of is not drawn; inside the model it was out of sight anyway.
        if let Some(proposal) = &self.ring_cut.proposal {
            for [a, b, c, d] in &proposal.contour {
                batch.push_triangle(*a, *b, *c, BAND);
                batch.push_triangle(*a, *c, *d, BAND);
            }
        }
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
        if self.ring_cut.proposal.is_some() {
            line = line
                .push(
                    button(text("Split in two").size(theme::LABEL_SIZE))
                        .padding(theme::PAD_CHOICE)
                        .style(theme::tool_button_active)
                        .on_press(Message::Ring(RingMessage::Split)),
                )
                .push(crate::ui::button_secondary(
                    "Drop",
                    Some(Message::Ring(RingMessage::Dropped)),
                ));
        }
        line.into()
    }
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

    /// The cut tool reads a drag as a ring unless told Line. A small ring
    /// over the ball it opens with proposes a disc wholly inside the ball:
    /// no contour, and a split is refused, the disc not going round
    /// anything. A ring wider than the ball proposes a disc that crosses
    /// it, drawn with its contour, and splits it in two. Drop and Escape
    /// drop a proposal.
    #[test]
    fn a_ring_proposes_a_cut_and_splits_only_what_it_goes_round() {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::ToolChanged(super::super::Tool::Cut));
        assert!(app.rings());
        let middle = Vector { x: 400.0, y: 300.0 };
        ring(&mut app, middle, Vector { x: 430.0, y: 300.0 });
        let proposal = app.ring_cut.proposal.clone().expect("a ring over the ball proposes a cut");
        assert!(proposal.contour.is_empty(), "a disc inside the ball has a contour");
        update(&mut app, Message::Ring(RingMessage::Split));
        assert_eq!(app.doc.bodies().count(), 1, "a disc inside the ball split it");
        assert!(app.status.contains("all the way round"), "{}", app.status);
        update(&mut app, Message::Ring(RingMessage::Dropped));
        assert!(app.ring_cut.proposal.is_none());

        ring(&mut app, middle, Vector { x: 790.0, y: 590.0 });
        let proposal = app.ring_cut.proposal.clone().expect("a wide ring proposes a cut");
        assert!(!proposal.contour.is_empty(), "a disc across the ball traced no contour");
        let mut batch = brokkr_gpu::OverlayBatch::default();
        app.add_ring_preview(&mut batch);
        assert!(!batch.surfaces.is_empty(), "the band was not drawn");
        drop(app.ring_cut_bar());
        update(&mut app, Message::Ring(RingMessage::Split));
        assert_eq!(app.doc.bodies().count(), 2, "{}", app.status);
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.bodies().count(), 1);

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
        // The disc a ring round the arm would propose, set as if proposed:
        // where the arm is on the screen depends on the camera.
        let body = app.doc.active();
        let disc = app
            .doc
            .active_volume()
            .disc_across(Vec3::new(16.0, 0.0, 2.0), -Vec3::Z, 4.0)
            .expect("solid under the ring");
        app.ring_cut.proposal = Some(Proposal { body, disc, contour: Vec::new() });
        update(&mut app, Message::Ring(RingMessage::Split));
        assert_eq!(app.doc.bodies().count(), 2, "{}", app.status);
        assert!(
            app.doc.active_volume().sample_world(Vec3::new(20.0, 0.0, 0.0)) < 0.0,
            "the arm is not the piece"
        );
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
            let disc = Disc { centre: Vec3::new(0.1, 0.2, 0.3), normal: Vec3::X, radius: 40.0 };
            app.ring_cut.proposal = Some(Proposal { body, disc, contour: Vec::new() });
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
