// SPDX-License-Identifier: AGPL-3.0-only

//! The workbench: somewhere to try a brush that is not the model.
//!
//! `docs/STUDIO-PLAN.md` section 18, stage C4. A brush is made by changing
//! its settings and seeing what a stroke of it does, and the only thing
//! there was to stroke was the work. On the workbench the document is set
//! aside, whole, and a piece that does not matter takes its place: a ball,
//! or a copy of the body in hand, for a brush that is being made for that
//! body's kind of surface. Everything works there as it does anywhere,
//! undo included. Leaving puts the document back exactly as it was, and
//! the brush that was made comes back in hand.
//!
//! # The document is set aside, and not edited and undone
//!
//! Trying a brush on the model and undoing it afterwards is what a user
//! does without this, and it costs them their redo, a place in an undo
//! history that has an end, and the nerve to press hard. So the workbench
//! does not touch the document at all: it, its undo history, where it is
//! saved, whether it has unsaved work and the view of it are lifted out of
//! the application and held ([`Bench`]), and put back on the way out.
//!
//! # Nothing that is for keeping happens here
//!
//! Saving, exporting, opening, importing, a new sculpt and closing the
//! window are about the document, and with a test piece standing where
//! the document was they would save the test piece under the document's
//! name or ask whether to discard work that is not the work. Each of them
//! first leaves the workbench ([`leaves_the_bench`]) and then does what it
//! does. The crash net is not written while the workbench is up, for the
//! same reason.

use brokkr_core::{Document, History, Volume};
use glam::Vec3;
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length};

use super::{Brokkr, MODEL_RADIUS_MM};
use crate::camera::OrbitCamera;
use crate::message::{Message, WorkbenchMessage};
use crate::theme;

/// What is on the workbench to try a brush on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    /// A ball the size a new sculpt starts at.
    Ball,
    /// A copy of the body that was in hand.
    Copy,
}

/// The document, set aside while the workbench is up.
pub(super) struct Bench {
    doc: Document,
    history: History,
    camera: OrbitCamera,
    unsaved: bool,
    project_path: Option<std::path::PathBuf>,
    piece: Piece,
}

/// Whether a message is about the document as something to keep, and so
/// has to find the document where it expects it. See the module
/// documentation.
pub(super) fn leaves_the_bench(message: &Message) -> bool {
    matches!(
        message,
        Message::SaveRequested
            | Message::SaveAsRequested
            | Message::SaveChosen(_)
            | Message::SavedThenContinue(_)
            | Message::ExportRequested(_)
            | Message::ExportChosen(..)
            | Message::Export(_)
            | Message::OpenInSlicer
            | Message::FitTestRequested
            | Message::NewSculpt
            | Message::ResetSphere
            | Message::OpenRequested
            | Message::OpenChosen(_)
            | Message::OpenRecent(_)
            | Message::ImportRequested
            | Message::ImportChosen(_)
            | Message::ImportLoaded(_)
            | Message::WindowClose
            | Message::CloseRequested(_)
    )
}

impl Brokkr {
    /// Whether the workbench is up, and what is in the document's place is
    /// a piece to try a brush on.
    pub(super) fn on_the_bench(&self) -> bool {
        self.bench.is_some()
    }

    pub(super) fn workbench_changed(&mut self, message: WorkbenchMessage) {
        match message {
            WorkbenchMessage::Opened(piece) => self.enter_workbench(piece),
            WorkbenchMessage::Fresh => {
                if let Some(piece) = self.bench.as_ref().map(|bench| bench.piece) {
                    self.enter_workbench(piece);
                }
            }
            WorkbenchMessage::Left => self.leave_workbench(),
        }
    }

    /// A piece to try a brush on: a ball at the document's own voxel size,
    /// so that a brush behaves on it as it will on the model, or a copy of
    /// the document's body in hand.
    fn test_piece(&self, piece: Piece) -> Volume {
        // The document is the one set aside, if the workbench is up.
        let doc = self.bench.as_ref().map_or(&self.doc, |bench| &bench.doc);
        let mut volume = match piece {
            Piece::Ball => {
                let mut ball = Volume::new(doc.voxel_size());
                ball.seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM);
                ball
            }
            Piece::Copy => doc.active_volume().duplicated(glam::IVec3::ZERO),
        };
        volume.mark_everything_dirty();
        volume
    }

    /// Put a test piece where the document is, setting the document aside
    /// if it is not aside already: with the workbench up, this is a fresh
    /// piece and the old one is thrown away.
    fn enter_workbench(&mut self, piece: Piece) {
        // Whatever gesture is in flight is on the piece that is about to
        // go.
        self.finish_stroke();
        self.drag = None;
        let volume = self.test_piece(piece);
        let on_the_bench = Document::from_volume(volume);
        match &mut self.bench {
            Some(bench) => {
                bench.piece = piece;
                self.doc = on_the_bench;
            }
            None => {
                let doc = std::mem::replace(&mut self.doc, on_the_bench);
                self.bench = Some(Box::new(Bench {
                    doc,
                    history: std::mem::take(&mut self.history),
                    camera: self.camera,
                    unsaved: self.unsaved,
                    project_path: self.project_path.take(),
                    piece,
                }));
            }
        }
        // The piece's history is its own, and begins empty.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.unsaved = false;
        // A ball is looked at as a new sculpt is. A copy of the body in
        // hand is looked at as that body was.
        if piece == Piece::Ball {
            self.camera = OrbitCamera::framing(Vec3::ZERO, MODEL_RADIUS_MM);
            self.refresh_camera_lattice();
            self.publish_camera();
        }
        self.swapped();
        self.status = "on the workbench: nothing done here is kept, but the brush is".to_string();
    }

    /// Put the document back as it was, and throw the test piece away. The
    /// brush in hand stays in hand: making it was what this was for.
    pub(super) fn leave_workbench(&mut self) {
        let Some(bench) = self.bench.take() else {
            return;
        };
        self.finish_stroke();
        self.drag = None;
        let bench = *bench;
        self.doc = bench.doc;
        self.history = bench.history;
        self.history_stats = self.history.stats();
        self.camera = bench.camera;
        self.refresh_camera_lattice();
        self.publish_camera();
        self.unsaved = bench.unsaved;
        self.project_path = bench.project_path;
        self.swapped();
        self.status = "back at the model, with the brush in hand".to_string();
    }

    /// What every swap of the document owes the rest of the application.
    fn swapped(&mut self) {
        // What was in hand that names a body or a point of the other
        // document.
        self.inserts.put_away();
        self.measure = super::measure::MeasureState::default();
        self.rebuild_everything();
        // A row's picture is kept by the row's number, and the two
        // documents number their rows alike.
        let bodies: Vec<_> =
            self.doc.nodes().iter().filter(|node| node.is_body()).map(|node| node.id).collect();
        for body in bodies {
            self.thumbs.geometry_changed(body);
        }
        self.refresh_overlay();
    }

    /// The workbench's own block of the panel: the way onto it, and with
    /// it up, a fresh piece and the way back.
    pub(super) fn workbench_panel(&self) -> Element<'_, Message> {
        let changed = |message: WorkbenchMessage| Message::Workbench(message);
        let wide = |label: &'static str, message: WorkbenchMessage| {
            button(text(label).size(theme::LABEL_SIZE).width(Length::Fill).center())
                .width(Length::Fill)
                .padding(theme::PAD_CHOICE)
                .style(theme::tool_button)
                .on_press(changed(message))
        };
        let caption =
            |words: &'static str| text(words).size(theme::LABEL_SIZE).color(theme::TEXT_MUTE);
        if self.on_the_bench() {
            column![
                row![
                    wide("A fresh piece", WorkbenchMessage::Fresh),
                    wide("Back to the model", WorkbenchMessage::Left),
                ]
                .spacing(theme::S1),
                caption("nothing done here is kept. the brush in hand is: Keep it by name above"),
            ]
        } else {
            column![
                row![
                    wide("Try on a ball", WorkbenchMessage::Opened(Piece::Ball)),
                    wide("On a copy of this", WorkbenchMessage::Opened(Piece::Copy)),
                ]
                .spacing(theme::S1),
                caption("set the model aside and try a brush on something that does not matter"),
            ]
        }
        .spacing(theme::S1)
        .into()
    }

    /// Over the top of the viewport while the workbench is up: what this
    /// is, so that a piece that is not the model is never taken for it,
    /// and the way back.
    pub(super) fn workbench_banner(&self) -> Option<Element<'_, Message>> {
        let piece = self.bench.as_ref()?.piece;
        let card = row![
            text(match piece {
                // Short, to stay clear of the view cube in the corner: seen
                // on a screen, a longer line ran under it.
                Piece::Ball => "WORKBENCH   a ball. nothing here is kept",
                Piece::Copy => "WORKBENCH   a copy. nothing here is kept",
            })
            .size(theme::LABEL_SIZE)
            .color(theme::ACCENT_HOT),
            button(text("A fresh piece").size(theme::LABEL_SIZE))
                .padding(theme::S2)
                .style(theme::tool_button)
                .on_press(Message::Workbench(WorkbenchMessage::Fresh)),
            button(text("Back to the model").size(theme::LABEL_SIZE))
                .padding(theme::S2)
                .style(theme::tool_button_active)
                .on_press(Message::Workbench(WorkbenchMessage::Left)),
        ]
        .spacing(theme::S3)
        .align_y(Alignment::Center);
        Some(
            container(container(card).padding(theme::S2).style(theme::overlay_card))
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .padding(theme::S4)
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent, StudioMessage};

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn stroke(app: &mut Brokkr) {
        let at = |x: f32| iced::Vector::new(x, 300.0);
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at(400.0),
                size: SIZE,
            }),
        );
        for step in 1..=4 {
            let position = at(400.0 + 12.0 * step as f32);
            update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
        }
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// Every value of every body, the bodies in their order.
    fn fields(app: &Brokkr) -> Vec<Vec<u32>> {
        app.doc
            .nodes()
            .iter()
            .filter_map(|node| node.volume())
            .map(|volume| {
                let mut values = Vec::new();
                for z in (-140..=140).step_by(2) {
                    for y in (-140..=140).step_by(4) {
                        for x in (-140..=140).step_by(4) {
                            values.push(volume.sample_voxel(glam::IVec3::new(x, y, z)).to_bits());
                        }
                    }
                }
                values
            })
            .collect()
    }

    /// Where a camera is looking from and at: what "the same view" means.
    fn seen_from(camera: &OrbitCamera) -> (Vec3, Vec3, f32) {
        (camera.eye(), camera.target, camera.distance)
    }

    /// A document worth setting aside: two bodies, a stroke on one of
    /// them, something to undo, unsaved work and a view of its own.
    fn at_work() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        stroke(&mut app);
        update(&mut app, Message::BodyDuplicated);
        update(&mut app, Message::BrushKindChanged(brokkr_core::BrushKind::Clay));
        stroke(&mut app);
        app.camera.orbit_radians(glam::Vec2::new(0.4, 0.2));
        app.camera.zoom_by(0.8);
        app
    }

    /// The workbench sets the document aside whole and puts it back
    /// whole: its bodies, bit for bit, its undo history, its unsaved work
    /// and the view of it. What is done on the bench is done to the piece
    /// and to nothing else, and the brush made there comes back in hand.
    #[test]
    fn the_workbench_sets_the_document_aside_and_puts_it_back() {
        let mut app = at_work();
        let (before, bodies) = (fields(&app), app.doc.body_count());
        let (camera, unsaved) = (app.camera, app.unsaved);
        assert!(bodies == 2 && unsaved && app.history.can_undo(), "the fixture is not at work");
        let undo_entries = app.history.stats().undo_entries;
        drop(app.workbench_panel());
        assert!(app.workbench_banner().is_none());

        update(&mut app, Message::Workbench(WorkbenchMessage::Opened(Piece::Ball)));
        assert!(app.on_the_bench());
        assert_eq!(app.doc.body_count(), 1, "the bench holds one piece");
        assert!(!app.history.can_undo(), "the piece came with something to undo");
        assert!(app.status.starts_with("on the workbench"), "{}", app.status);
        drop(app.workbench_panel());
        assert!(app.workbench_banner().is_some());
        drop(app.view());
        // The piece is a ball nobody has touched.
        let fresh = fields(&Brokkr::with_tablet(crate::tablet::Tablet::inert()));
        assert_eq!(fields(&app), fresh);

        // A brush is made and tried: strokes land on the piece, and undo
        // there undoes them and nothing of the document's.
        let crease = super::super::presets::core()
            .iter()
            .position(|preset| preset.name == "Crease")
            .expect("Crease comes with the application");
        update(&mut app, Message::Studio(StudioMessage::PresetChosen(crease)));
        update(&mut app, Message::BrushStrengthChanged(0.6));
        stroke(&mut app);
        stroke(&mut app);
        assert_ne!(fields(&app), fresh, "a stroke on the bench did nothing");
        update(&mut app, Message::Undo);
        update(&mut app, Message::Undo);
        assert_eq!(fields(&app), fresh);
        assert!(!app.history.can_undo(), "undo on the bench reached past the piece");
        stroke(&mut app);

        // A fresh piece is a fresh piece.
        update(&mut app, Message::Workbench(WorkbenchMessage::Fresh));
        assert!(app.on_the_bench());
        assert_eq!(fields(&app), fresh);

        stroke(&mut app);
        update(&mut app, Message::Workbench(WorkbenchMessage::Left));
        assert!(!app.on_the_bench());
        assert_eq!(app.doc.body_count(), bodies);
        assert_eq!(fields(&app), before, "the document did not come back as it was");
        assert_eq!(app.history.stats().undo_entries, undo_entries);
        assert_eq!(app.unsaved, unsaved);
        assert_eq!(seen_from(&app.camera), seen_from(&camera), "the view of the document moved");
        assert_eq!(app.status, "back at the model, with the brush in hand");
        // The brush made on the bench is in hand.
        assert_eq!(app.live_brush_name(), "CREASE");
        assert_eq!(app.brush.strength, 0.6);
        // And the document's own undo still undoes the document.
        update(&mut app, Message::Undo);
        assert_ne!(fields(&app), before);

        // Leaving when not there, and a fresh piece when not there, are
        // nothing.
        let now = fields(&app);
        update(&mut app, Message::Workbench(WorkbenchMessage::Left));
        update(&mut app, Message::Workbench(WorkbenchMessage::Fresh));
        assert!(!app.on_the_bench());
        assert_eq!(fields(&app), now);
    }

    /// A copy of the body in hand is that body, to be tried on without
    /// being touched, seen from where it was being seen.
    #[test]
    fn a_copy_of_the_body_in_hand_is_tried_on_and_not_touched() {
        let mut app = at_work();
        let before = fields(&app);
        let camera = app.camera;
        let active = app.doc.index_of(app.doc.active()).expect("a body in hand");
        update(&mut app, Message::Workbench(WorkbenchMessage::Opened(Piece::Copy)));
        assert_eq!(app.doc.body_count(), 1);
        assert_eq!(fields(&app)[0], before[active], "the piece is not a copy of the body in hand");
        assert_eq!(seen_from(&app.camera), seen_from(&camera), "the view moved");
        stroke(&mut app);
        assert_ne!(fields(&app)[0], before[active]);
        // A fresh piece is the body again, as the document has it.
        update(&mut app, Message::Workbench(WorkbenchMessage::Fresh));
        assert_eq!(fields(&app)[0], before[active]);
        update(&mut app, Message::Workbench(WorkbenchMessage::Left));
        assert_eq!(fields(&app), before);
    }

    /// Anything that is about the document as something to keep puts the
    /// document back first: a save from the workbench saves the work, and
    /// never the test piece.
    #[test]
    fn keeping_the_document_leaves_the_workbench_first() {
        for message in [
            Message::SaveRequested,
            Message::SaveAsRequested,
            Message::NewSculpt,
            Message::OpenRequested,
            Message::ImportRequested,
            Message::WindowClose,
        ] {
            assert!(leaves_the_bench(&message), "{message:?}");
        }
        for message in [Message::Undo, Message::Frame, Message::BodyDuplicated] {
            assert!(!leaves_the_bench(&message), "{message:?}");
        }
        let mut app = at_work();
        let before = fields(&app);
        update(&mut app, Message::Workbench(WorkbenchMessage::Opened(Piece::Ball)));
        stroke(&mut app);
        // The dialog this asks for is never answered here: what matters
        // is what it would have been asked about.
        update(&mut app, Message::SaveAsRequested);
        assert!(
            !app.on_the_bench(),
            "a save was begun with the test piece in the document's place"
        );
        assert_eq!(fields(&app), before);
        assert!(app.unsaved, "the document's unsaved work was forgotten");
    }
}
