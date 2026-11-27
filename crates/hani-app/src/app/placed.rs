// SPDX-License-Identifier: AGPL-3.0-only

//! A shape set down in Sculpt is placed, and joined when done.
//!
//! Asked for by the maintainer on 2026-11-28: "when using the insert brush
//! why not treat it like the transform insert brush but the only difference
//! is that it is unioned (or subtracted) with the target when done."
//!
//! In Transform a shape dragged out is a body of its own. In Sculpt it was
//! joined into the body in hand on the release, where it could no longer be
//! moved. Now, on the release, the shape is a body of its own as in
//! Transform, put with the body it was set down on into a folder that
//! combines them (`app/booleans.rs`): the body the base, the shape added,
//! or cut where Ctrl or Alt was held, with the brush's soft join. The
//! result is drawn at once, a cut's shape see-through, and the shape can be
//! moved, turned and scaled with the Transform tool. It is **joined when
//! done**: the next shape set down, or the insert put away for another
//! brush, bakes the folder into the body (one undo puts the folder back),
//! as does Bake under the list.
//!
//! # The largest shape
//!
//! A drag toward the horizon of a surface goes on for ever, and a shape
//! that size once ran the application out of memory. It was held to 160
//! voxels from its middle. It cannot simply go: building the shape as a
//! body costs memory as its surface grows, and time as its box does. The
//! bound now comes from the memory there is: the largest box whose shell of
//! dense bricks, counted twice over for the band crossing brick faces, fits
//! in a sixteenth of the room left under the volume ceiling. Arithmetic,
//! not measured: a box of half size `h` has `24 h^2` of surface, a dense
//! brick 128 KiB and `s` across, so the shell is `2 * 24 h^2 / s^2 * 128
//! KiB` and `h = s * sqrt(room / (48 * 128 KiB))`. With the whole 6 GiB
//! ceiling free at a 0.25 mm voxel that is 63 mm, 253 voxels, against the
//! 160 it was. Half of the room was tried first: 724 voxels, whose box the
//! tests' drags past the horizon built several at a time until this
//! machine's kernel killed one for memory.

use hani_core::combine::{BoolOp, BoolState, MOST_BLEND_HALVES};
use hani_core::{Change, Entry, Volume};

use super::Hani;

/// The bytes of one dense brick of distances.
const DENSE_BRICK_BYTES: f64 = hani_core::BRICK_DIM.pow(3) as f64 * 4.0;

impl Hani {
    /// The furthest from its middle a shape set down by a drag may reach,
    /// in millimetres: see the module's documentation.
    pub(super) fn largest_insert_mm(&self) -> f32 {
        let room = (super::MAX_VOLUME_BYTES - self.doc_stats.resident_bytes as f64).max(0.0) / 16.0;
        let across = hani_core::BRICK_DIM as f64 * f64::from(self.doc.voxel_size());
        let half = across * (room / (48.0 * DENSE_BRICK_BYTES)).sqrt();
        // Never less than a brush: a shape the size of the largest brush
        // always fits.
        (half as f32).max(super::MAX_RADIUS_VOXELS * self.doc.voxel_size())
    }

    /// Place `made` with the body in hand in a folder that combines them,
    /// added or cut; one undo entry.
    pub(super) fn place(&mut self, made: Volume, name: String, cut: bool) {
        let target = self.doc.active();
        if self.doc.node_count() + 2 > hani_core::MAX_NODES {
            self.status =
                "could not place it: the document holds as many rows as it can".to_string();
            return;
        }
        // The folder takes the body's own name, which the bake gives the
        // result: the body is called what it was once the shape is joined.
        // Seen on a screen with "Group 1", which renamed Body 1.
        let folder_name =
            self.doc.node(target).map_or_else(|| self.next_group_name(), |node| node.name.clone());
        let Some((folder, mut changes)) = self.doc.group(target, folder_name) else {
            self.status = format!(
                "could not place it: {} folders deep is as far as the panel goes",
                hani_core::MAX_DEPTH
            );
            return;
        };
        let (Some(at), Some(depth)) =
            (self.doc.index_of(target), self.doc.node(target).map(hani_core::Node::depth))
        else {
            return;
        };
        let id = self.doc.insert_body(at + 1, depth, name.clone(), made);
        changes.push(Change::NodeAdded { at: at + 1, id });
        changes.extend(
            self.doc
                .set_bool_state(folder, BoolState { combines: true, op: BoolOp::Add, blend: 0 }),
        );
        let op = if cut { BoolOp::Cut } else { BoolOp::Add };
        let blend =
            (self.inserts.blend * 2.0).round().clamp(0.0, f32::from(MOST_BLEND_HALVES)) as u8;
        changes.extend(self.doc.set_bool_state(id, BoolState { combines: false, op, blend }));
        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        self.unsaved = true;
        self.inserts.placed = Some(folder);
        self.status = format!(
            "{name} placed, {}: move it with the Transform tool; joined when done",
            if cut { "cutting" } else { "adding" }
        );
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// Join the shape last placed, if it is still placed: the next shape set
    /// down, or the insert put away, is what "done" is.
    pub(super) fn finish_placed(&mut self) {
        let Some(folder) = self.inserts.placed.take() else {
            return;
        };
        if !self.doc.combines(folder) {
            return;
        }
        let changes = self.bake_folder(folder);
        if changes.is_empty() {
            return;
        }
        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        self.unsaved = true;
        self.remesh_dirty();
    }

    /// After every message: an insert put away for another brush is done.
    pub(super) fn finish_placed_when_put_away(&mut self) {
        if self.inserts.placed.is_some() && self.inserts.what.is_none() {
            self.finish_placed();
        }
    }
}
