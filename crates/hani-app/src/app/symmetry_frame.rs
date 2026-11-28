// SPDX-License-Identifier: AGPL-3.0-only

//! Symmetry that belongs to the body: which mirror planes are on, the radial
//! copies and their axis, and the frame they are in, carried with it when
//! the gizmo moves it.
//!
//! Asked for by the maintainer on 2026-11-28: "when you transform a node
//! with the gizmo the symmetry transforms with it (ie symmetry is local to
//! the object not global)", and then "Each body should have its own
//! settings. Yes. Just like zbrush". All of it is the document's
//! (`hani_core::symmetry_frames`). The controls show and set the active
//! body's ([`Hani::symmetry_settings`], [`Hani::set_symmetry`]); choosing
//! another body shows its own. Everything that lays a copy, draws a plane
//! or asks whether a body straddles one reads [`Hani::framed_symmetry`],
//! the active body's symmetry with its frame carried by a gizmo that holds
//! it.
//!
//! What `app.rs` holds for this: the gizmo's commit, for a body and for a
//! folder (`app/group_gizmo.rs`), adds the frames' changes to its one undo
//! entry; the straddle test is asked in the frame; the file's state carries
//! the radial copies.

use hani_core::{Change, Entry, NodeId, Similarity, Symmetry, SymmetryFrame};

use super::Hani;

impl Hani {
    /// The symmetry a stroke lays copies with: the session's planes and
    /// copies, in the active body's frame.
    pub(super) fn framed_symmetry(&self) -> Symmetry {
        let active = self.doc.active();
        let symmetry = self.doc.symmetry(active);
        let frame = symmetry.frame();
        // Under the gizmo, the planes are where the placement has the body,
        // handle held or not: the frame itself is only moved when the gizmo
        // is put away (`move_frames`), and the body is drawn and baked at the
        // placement before then.
        let held = self.gizmo.filter(|gizmo| self.holds(gizmo.body, active));
        symmetry.in_frame(held.map_or(frame, |gizmo| frame.moved(gizmo.placement)))
    }

    /// The active body's planes and radial copies, as the controls show
    /// them.
    pub(super) fn symmetry_settings(&self) -> Symmetry {
        self.doc.symmetry(self.doc.active())
    }

    /// Set the active body's planes and radial copies, keeping its frame,
    /// as no undo step: for what the application does by itself, a plane
    /// turned off because the body no longer reaches it.
    pub(super) fn set_symmetry(&mut self, settings: Symmetry) {
        let active = self.doc.active();
        if self.doc.set_symmetry_settings(active, settings).is_some() {
            self.unsaved = true;
        }
    }

    /// Set the active body's planes and radial copies as one undo step.
    /// Asked for by the maintainer: "Make symmetry setting changes undoable
    /// too".
    pub(super) fn set_symmetry_recorded(&mut self, settings: Symmetry) {
        self.finish_symmetry_drag();
        let active = self.doc.active();
        if let Some(change) = self.doc.set_symmetry_settings(active, settings) {
            let before = self.history.stats();
            self.history.push(Entry::new(vec![change]));
            self.record_history(before);
            self.unsaved = true;
        }
    }

    /// Set them during a drag of Radial copies: the step is made once, from
    /// where the drag started, by [`Hani::finish_symmetry_drag`].
    pub(super) fn set_symmetry_dragged(&mut self, settings: Symmetry) {
        let active = self.doc.active();
        if self.symmetry_drag.is_some_and(|(body, _)| body != active) {
            self.finish_symmetry_drag();
        }
        if let Some(Change::Symmetry { symmetry, .. }) =
            self.doc.set_symmetry_settings(active, settings)
        {
            self.symmetry_drag.get_or_insert((active, symmetry));
            self.unsaved = true;
        }
    }

    /// The drag let go, or a value typed in (which has no release): one undo
    /// step from where it started, if it ended anywhere else. Also run before
    /// an undo or a redo, so a typed value is a step too.
    pub(super) fn finish_symmetry_drag(&mut self) {
        let Some((id, before)) = self.symmetry_drag.take() else {
            return;
        };
        if self.doc.symmetry(id) == before {
            return;
        }
        let stats = self.history.stats();
        self.history.push(Entry::new(vec![Change::Symmetry { id, symmetry: before }]));
        self.record_history(stats);
    }

    /// Whether any body has a plane on or radial copies.
    fn holds_any_symmetry(&self) -> bool {
        self.doc.nodes().iter().any(|node| !self.doc.symmetry(node.id).is_off())
    }

    /// The mirror planes a file kept for the session, before each body had
    /// its own, laid on every body: what they meant when it was saved. A
    /// file with any body's own settings has none to lay.
    pub(super) fn adopt_session_mirrors(&mut self, mirror: [bool; 3]) {
        if self.holds_any_symmetry() || !mirror.contains(&true) {
            return;
        }
        let bodies: Vec<NodeId> =
            self.doc.nodes().iter().filter(|node| node.is_body()).map(|node| node.id).collect();
        for body in bodies {
            let settings = hani_core::MirrorAxis::ALL
                .into_iter()
                .zip(mirror)
                .fold(self.doc.symmetry(body), |set, (axis, on)| set.with_axis(axis, on));
            // What the file meant, and no step: the history starts here.
            drop(self.doc.set_symmetry_settings(body, settings));
        }
    }

    /// The radial copies a file kept for the session, laid on every body on
    /// the same terms.
    pub(super) fn adopt_session_radial(&mut self, radial: hani_core::Radial) {
        let any_radial =
            self.doc.nodes().iter().any(|node| self.doc.symmetry(node.id).radial() > 1);
        if radial.copies <= 1 || any_radial {
            return;
        }
        let bodies: Vec<NodeId> =
            self.doc.nodes().iter().filter(|node| node.is_body()).map(|node| node.id).collect();
        for body in bodies {
            let settings = self.doc.symmetry(body).with_radial(radial.copies, radial.axis);
            drop(self.doc.set_symmetry_settings(body, settings));
        }
    }

    /// Whether the gizmo's row is the body or a folder above it.
    fn holds(&self, row: NodeId, body: NodeId) -> bool {
        let mut at = Some(body);
        while let Some(id) = at {
            if id == row {
                return true;
            }
            at = self.doc.parent_of(id);
        }
        false
    }

    /// A new body's planes: through its own middle, unturned. A body made
    /// from another takes [`Hani::frame_from`] instead.
    pub(super) fn frame_at_middle(&mut self, body: NodeId) {
        let bounds = self.doc.volume(body).and_then(hani_core::Volume::surface_bounds);
        if let Some((low, high)) = bounds {
            let origin = (low + high) * 0.5 - super::MIRROR_CENTRE;
            self.doc.set_symmetry_frame(body, SymmetryFrame { origin, ..SymmetryFrame::IDENTITY });
        }
    }

    /// A body made from another, a copy or a shell or an extract, keeps
    /// that one's symmetry: its settings and its frame.
    pub(super) fn frame_from(&mut self, body: NodeId, source: NodeId) {
        let symmetry = self.doc.symmetry(source);
        self.doc.put_symmetry(body, symmetry);
    }

    /// Whether the radial axis passes through the active body: the two
    /// planes that hold the axis both straddled. An empty body passes.
    pub(super) fn radial_reaches(&self) -> bool {
        let Some(bounds) = self.doc.active_volume().surface_bounds() else {
            return true;
        };
        let axis = self.symmetry_settings().radial_axis();
        let frame = self.framed_symmetry().frame();
        hani_core::MirrorAxis::ALL
            .into_iter()
            .filter(|plane| *plane != axis)
            .all(|plane| super::straddles(bounds, plane, frame))
    }

    /// Turn the radial copies off where the axis misses the active body,
    /// saying so: every copy would land in empty space.
    pub(super) fn refuse_radial_off_axis(&mut self, verb: &str) {
        let settings = self.symmetry_settings();
        if settings.radial() <= 1 || self.radial_reaches() {
            return;
        }
        let axis = settings.radial_axis();
        self.set_symmetry(settings.with_radial(1, axis));
        let name = self.doc.node(self.doc.active()).map_or("the body", |node| node.name.as_str());
        self.status = format!(
            "radial copies {verb}: the {} axis misses {name}, so every copy would land in empty \
             space",
            axis.label(),
        );
    }

    /// Carry each body's frame by the placement the gizmo left it at, and
    /// answer the changes that put them back, for the gizmo's one entry.
    pub(super) fn move_frames(&mut self, bodies: &[NodeId], placement: Similarity) -> Vec<Change> {
        let changes: Vec<Change> = bodies
            .iter()
            .filter_map(|body| {
                let frame = self.doc.symmetry_frame(*body).moved(placement);
                self.doc.set_symmetry_frame(*body, frame)
            })
            .collect();
        // The planes were drawn through the gizmo; now through the frame.
        self.refresh_overlay();
        changes
    }

    /// Whether the active body's frame is the room's.
    pub(super) fn frame_is_the_rooms(&self) -> bool {
        self.doc.symmetry_frame(self.doc.active()) == SymmetryFrame::IDENTITY
    }

    /// Put the active body's planes back through the middle of the room,
    /// unturned: one undo step.
    pub(super) fn reset_frame(&mut self) {
        self.finish_symmetry_drag();
        let body = self.doc.active();
        let Some(change) = self.doc.set_symmetry_frame(body, SymmetryFrame::IDENTITY) else {
            return;
        };
        let before = self.history.stats();
        self.history.push(Entry::new(vec![change]));
        self.record_history(before);
        self.unsaved = true;
        self.status = "symmetry: back to the middle of the room".to_string();
        // A plane through the middle of the room may miss the body now.
        self.refuse_mirrors_the_body_does_not_straddle("turned off");
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};
    use hani_core::{MirrorAxis, Volume};

    use super::*;
    use crate::message::Message;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn middle(app: &Hani, body: NodeId) -> Vec3 {
        let (low, high) = app
            .doc
            .volume(body)
            .and_then(Volume::surface_bounds)
            .expect("a body with something in it");
        (low + high) * 0.5
    }

    /// Put the gizmo on a row, leave it at a placement, and put it away.
    fn moved(app: &mut Hani, row: NodeId, placement: Similarity) {
        // From Sculpt: choosing Transform while it is up puts it away.
        update(app, Message::ToolChanged(super::super::Tool::Sculpt));
        update(app, Message::ToolChanged(super::super::Tool::Transform));
        update(app, Message::BodySelected(row));
        assert_eq!(app.gizmo.map(|gizmo| gizmo.body), Some(row), "{}", app.status);
        if let Some(gizmo) = app.gizmo.as_mut() {
            gizmo.placement = placement;
        }
        app.rebake_gizmo();
        update(app, Message::ToolChanged(super::super::Tool::Sculpt));
        assert!(app.gizmo.is_none());
    }

    /// A body moved well off the room's X plane takes its own plane with it:
    /// the mirror is not refused, a copy lands across the moved plane, the
    /// turn goes with it, and one undo puts the body and its frame back.
    #[test]
    fn the_gizmo_carries_a_bodys_planes_with_it() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let body = app.doc.active();
        let (low, high) = app.doc.active_volume().surface_bounds().expect("the ball");
        let away = Vec3::X * (high.x - low.x) * 2.0;
        let turn = Quat::from_rotation_z(0.5);
        let entries = app.history_stats.undo_entries;
        moved(
            &mut app,
            body,
            Similarity { rotation: turn, translation: away, ..Similarity::IDENTITY },
        );
        assert_eq!(app.history_stats.undo_entries, entries + 1, "one entry for body and frame");
        let frame = app.framed_symmetry().frame();
        let centre = middle(&app, body);
        assert!(frame.origin.distance(centre) < app.doc.voxel_size() * 2.0, "{frame:?}, {centre}");
        assert!(frame.rotation.angle_between(turn) < 1.0e-4);

        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(
            app.symmetry_settings().axis(MirrorAxis::X),
            "refused a plane the body is on: {}",
            app.status
        );
        let mut flips = [hani_core::Flip::IDENTITY; Symmetry::MAX_MIRRORS];
        assert_eq!(app.framed_symmetry().flips(super::super::MIRROR_CENTRE, &mut flips), 1);
        let across = frame.origin + turn * Vec3::new(5.0, 1.0, -2.0);
        let expected = frame.origin + turn * Vec3::new(-5.0, 1.0, -2.0);
        assert!(flips[0].point(across).distance(expected) < 1.0e-3);

        // The mirror is a step of its own, then the move.
        update(&mut app, Message::Undo);
        assert!(!app.symmetry_settings().axis(MirrorAxis::X), "the first undo left X on");
        assert!(app.doc.symmetry_frame(body).origin.distance(frame.origin) < 1.0e-5);
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.symmetry_frame(body), SymmetryFrame::IDENTITY);
        assert!(middle(&app, body).length() < app.doc.voxel_size() * 2.0, "the body stayed");
        update(&mut app, Message::Redo);
        assert!(app.doc.symmetry_frame(body).origin.distance(frame.origin) < 1.0e-5);
    }

    /// A folder moved as one moves every body's frame under it, in its one
    /// entry; Reset puts the active body's frame back, one entry of its own.
    #[test]
    fn a_folder_moves_every_frame_and_reset_puts_one_back() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        let first = app.doc.nodes()[0].id;
        let (folder, _) = app.doc.group(first, "Pair").expect("room for a folder");
        app.doc.move_to_folder(cube, Some(folder)).expect("moved");
        let step = Vec3::X * app.doc.voxel_size() * 8.0;
        let was = [first, cube].map(|body| app.doc.symmetry_frame(body).origin);
        moved(&mut app, folder, Similarity { translation: step, ..Similarity::IDENTITY });
        for (body, was) in [first, cube].into_iter().zip(was) {
            let origin = app.doc.symmetry_frame(body).origin;
            assert!(origin.distance(was + step) < 1.0e-4, "{body:?} frame at {origin}");
        }
        update(&mut app, Message::BodySelected(cube));
        assert!(!app.frame_is_the_rooms());
        let entries = app.history_stats.undo_entries;
        app.reset_frame();
        assert!(app.frame_is_the_rooms());
        assert_eq!(app.history_stats.undo_entries, entries + 1);
        assert!(app.doc.symmetry_frame(first).origin.distance(step) < 1.0e-5, "reset is one body");
        update(&mut app, Message::Undo);
        assert!(!app.frame_is_the_rooms(), "reset was not undone");
    }

    /// While the gizmo holds a body, its planes are where the placement has
    /// it, handle held or not; the frame kept is moved only when the gizmo
    /// is put away.
    #[test]
    fn the_planes_follow_a_held_handle() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let body = app.doc.active();
        update(&mut app, Message::ToolChanged(super::super::Tool::Sculpt));
        update(&mut app, Message::ToolChanged(super::super::Tool::Transform));
        update(&mut app, Message::BodySelected(body));
        let step = Vec3::new(12.0, 0.0, 3.0);
        if let Some(gizmo) = app.gizmo.as_mut() {
            gizmo.placement = Similarity { translation: step, ..Similarity::IDENTITY };
        }
        assert!(app.framed_symmetry().frame().origin.distance(step) < 1.0e-5);
        assert_eq!(app.doc.symmetry_frame(body), SymmetryFrame::IDENTITY, "kept before commit");
        if let Some(gizmo) = app.gizmo.as_mut() {
            gizmo.placement = Similarity::IDENTITY;
        }
        assert_eq!(app.framed_symmetry().frame(), SymmetryFrame::IDENTITY);
    }

    /// A shape added has its planes through its own middle; a copy keeps
    /// its source's, settings and frame.
    #[test]
    fn a_new_body_has_its_own_planes_and_a_copy_its_sources() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        let frame = app.doc.symmetry_frame(cube);
        assert!(frame.origin.distance(middle(&app, cube)) < 1.0e-4, "{frame:?}");
        let turned = SymmetryFrame { rotation: Quat::from_rotation_y(0.3), ..frame };
        app.doc.set_symmetry_frame(cube, turned);
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(app.symmetry_settings().axis(MirrorAxis::X), "{}", app.status);
        update(&mut app, Message::BodyDuplicated);
        let copy = app.doc.active();
        assert_ne!(copy, cube, "{}", app.status);
        assert_eq!(app.doc.symmetry_frame(copy), turned);
        assert!(app.symmetry_settings().axis(MirrorAxis::X), "the copy lost the cube's mirror");
    }

    /// Radial copies are each body's own: the ball keeps its six when
    /// another body is chosen, which starts with none; copies round an axis
    /// that misses that body are refused, and allowed round the axis it
    /// lies along.
    #[test]
    fn radial_copies_are_each_bodys_and_refused_off_its_axis() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let ball = app.doc.active();
        let (low, high) = app.doc.active_volume().surface_bounds().expect("the ball");
        let away = Vec3::X * (high.x - low.x) * 2.0;
        let shifted =
            app.doc.active_volume().shifted((away / app.doc.voxel_size()).round().as_ivec3());
        let other = app.doc.add_body("Away", shifted);
        update(&mut app, Message::Radial(super::super::RadialMessage::Copies(6.0)));
        assert_eq!(app.symmetry_settings().radial(), 6, "the ball is on the axis: {}", app.status);
        update(&mut app, Message::BodySelected(other));
        assert_eq!(app.symmetry_settings().radial(), 1, "the other body took the ball's copies");
        update(&mut app, Message::Radial(super::super::RadialMessage::Copies(6.0)));
        assert_eq!(app.symmetry_settings().radial(), 1);
        assert!(app.status.contains("radial copies refused"), "{}", app.status);
        // Round X, the axis runs along the moved body: allowed.
        update(&mut app, Message::Radial(super::super::RadialMessage::Axis(MirrorAxis::X)));
        update(&mut app, Message::Radial(super::super::RadialMessage::Copies(6.0)));
        assert_eq!(app.symmetry_settings().radial(), 6, "{}", app.status);
        assert_eq!(app.doc.symmetry(ball).radial_axis(), MirrorAxis::Y, "the ball's axis moved");
        update(&mut app, Message::BodySelected(ball));
        assert_eq!(app.symmetry_settings().radial(), 6);
    }

    /// A mirror turned on is one undo step, and the redo puts it back; the
    /// axis of the radial copies is a step of its own.
    #[test]
    fn a_mirror_and_the_axis_are_each_one_step() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let entries = app.history_stats.undo_entries;
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(app.symmetry_settings().axis(MirrorAxis::X));
        assert_eq!(app.history_stats.undo_entries, entries + 1, "the toggle was not a step");
        update(&mut app, Message::Radial(super::super::RadialMessage::Axis(MirrorAxis::Z)));
        assert_eq!(app.history_stats.undo_entries, entries + 2, "the axis was not a step");
        update(&mut app, Message::Undo);
        assert_eq!(app.symmetry_settings().radial_axis(), MirrorAxis::Y);
        assert!(app.symmetry_settings().axis(MirrorAxis::X));
        update(&mut app, Message::Undo);
        assert!(app.symmetry_settings().is_off(), "the undo left X on");
        update(&mut app, Message::Redo);
        assert!(app.symmetry_settings().axis(MirrorAxis::X), "the redo did not put X back");
    }

    /// A drag of Radial copies through several values is one step from
    /// where it started; a value typed in, which has no release, is a step
    /// once an undo asks for it.
    #[test]
    fn a_drag_of_the_copies_is_one_step_and_a_typed_value_is_undone() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let entries = app.history_stats.undo_entries;
        for copies in [2.0, 3.0, 5.0, 6.0] {
            update(&mut app, Message::Radial(super::super::RadialMessage::Copies(copies)));
        }
        assert_eq!(app.history_stats.undo_entries, entries, "a step before the release");
        update(&mut app, Message::Radial(super::super::RadialMessage::CopiesDone));
        assert_eq!(app.history_stats.undo_entries, entries + 1, "not one step for the drag");
        assert_eq!(app.symmetry_settings().radial(), 6);
        update(&mut app, Message::Undo);
        assert_eq!(app.symmetry_settings().radial(), 1, "the undo did not take the drag back");

        update(&mut app, Message::Radial(super::super::RadialMessage::Copies(4.0)));
        assert_eq!(app.symmetry_settings().radial(), 4);
        update(&mut app, Message::Undo);
        assert_eq!(app.symmetry_settings().radial(), 1, "a typed value was not undone");
    }

    /// Recentred, a plane the body no longer straddles is turned off, as
    /// choosing such a body turns it off.
    #[test]
    fn recentring_turns_off_a_plane_the_body_misses() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let body = app.doc.active();
        let (low, high) = app.doc.active_volume().surface_bounds().expect("the ball");
        let away = Vec3::X * (high.x - low.x) * 2.0;
        moved(&mut app, body, Similarity { translation: away, ..Similarity::IDENTITY });
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        assert!(app.symmetry_settings().axis(MirrorAxis::X));
        update(&mut app, Message::Radial(super::super::RadialMessage::ResetFrame));
        assert!(app.frame_is_the_rooms());
        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "a plane that misses the body stayed on"
        );
    }
}
