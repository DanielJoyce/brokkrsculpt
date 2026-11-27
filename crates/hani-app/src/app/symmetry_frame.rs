// SPDX-License-Identifier: AGPL-3.0-only

//! Symmetry local to the body: the mirror planes and the radial axis in the
//! active body's own frame, carried with it when the gizmo moves it.
//!
//! Asked for by the maintainer on 2026-11-28: "when you transform a node
//! with the gizmo the symmetry transforms with it (ie symmetry is local to
//! the object not global)". The frames are the document's
//! (`hani_core::symmetry_frames`); which planes are on and how many copies
//! round which axis stay the session's, in `Hani::symmetry`. Everything that
//! lays a copy, draws a plane or asks whether a body straddles one reads
//! [`Hani::framed_symmetry`], the two put together.
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
        self.symmetry.in_frame(self.doc.symmetry_frame(self.doc.active()))
    }

    /// Carry each body's frame by the placement the gizmo left it at, and
    /// answer the changes that put them back, for the gizmo's one entry.
    pub(super) fn move_frames(&mut self, bodies: &[NodeId], placement: Similarity) -> Vec<Change> {
        bodies
            .iter()
            .filter_map(|body| {
                let frame = self.doc.symmetry_frame(*body).moved(placement);
                self.doc.set_symmetry_frame(*body, frame)
            })
            .collect()
    }

    /// Whether the active body's frame is the room's.
    pub(super) fn frame_is_the_rooms(&self) -> bool {
        self.doc.symmetry_frame(self.doc.active()) == SymmetryFrame::IDENTITY
    }

    /// Put the active body's planes back through the middle of the room,
    /// unturned: one undo step.
    pub(super) fn reset_frame(&mut self) {
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
        assert!(app.symmetry.axis(MirrorAxis::X), "refused a plane the body is on: {}", app.status);
        let mut flips = [hani_core::Flip::IDENTITY; Symmetry::MAX_MIRRORS];
        assert_eq!(app.framed_symmetry().flips(super::super::MIRROR_CENTRE, &mut flips), 1);
        let across = frame.origin + turn * Vec3::new(5.0, 1.0, -2.0);
        let expected = frame.origin + turn * Vec3::new(-5.0, 1.0, -2.0);
        assert!(flips[0].point(across).distance(expected) < 1.0e-3);

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
        moved(&mut app, folder, Similarity { translation: step, ..Similarity::IDENTITY });
        for body in [first, cube] {
            let origin = app.doc.symmetry_frame(body).origin;
            assert!(origin.distance(step) < 1.0e-5, "{body:?} frame at {origin}");
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
        assert!(app.symmetry.axis(MirrorAxis::X));
        update(&mut app, Message::Radial(super::super::RadialMessage::ResetFrame));
        assert!(app.frame_is_the_rooms());
        assert!(!app.symmetry.axis(MirrorAxis::X), "a plane that misses the body stayed on");
    }
}
