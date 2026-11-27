// SPDX-License-Identifier: AGPL-3.0-only

//! The gizmo on a folder: every body under it moved, turned and scaled as
//! one, about the middle of them all.
//!
//! Asked for by the maintainer on 2026-11-28, with boolean groups: "group
//! nodes can be transformed with the gizmo". A folder row pressed in the
//! list while the Transform tool is up puts the folder in hand, and the
//! gizmo arms on it in place of the body. A body row pressed, or a body
//! picked on the model, puts the folder down again.
//!
//! The session is the single body's, kept a body at a time: each body's
//! field as it was when the gizmo armed is held, every drag bakes each one
//! again from it at the whole placement (so drags do not compound), the
//! preview draws every body under the folder, and a combining folder's
//! result, through the drag's map, and putting the gizmo away commits one
//! undo entry holding every field, or with Escape puts them all back.
//!
//! What `app.rs` holds for this: a field, a line in `arm_gizmo`,
//! `rebake_gizmo` and `disarm_gizmo` each handing a folder here, a line where
//! a row is pressed and one where a body is picked, and the bake itself
//! (`bake_placed`), moved out of `rebake_gizmo` so that both can use it.

use glam::{Mat4, Vec3};
use hani_core::{NodeId, Similarity, Volume};

use super::Hani;
use crate::gizmo::Gizmo;

/// The folder in hand and the session the gizmo holds on it.
#[derive(Default)]
pub(super) struct GroupGizmo {
    /// The folder last pressed in the list, until a body is.
    pub folder_in_hand: Option<NodeId>,
    /// Each body's field as it was when the gizmo armed, once one is baked.
    bases: Vec<(NodeId, Box<Volume>)>,
    /// The placement the bodies were last baked at.
    baked: Option<Similarity>,
}

impl Hani {
    /// A row was pressed in the list: a folder is taken in hand, a body
    /// puts it down.
    pub(super) fn row_pressed(&mut self, row: NodeId) {
        self.group_gizmo.folder_in_hand =
            self.doc.node(row).filter(|node| !node.is_body()).map(|node| node.id);
    }

    /// A body was picked on the model: the folder is put down.
    pub(super) fn body_picked(&mut self) {
        self.group_gizmo.folder_in_hand = None;
    }

    /// The folder the gizmo takes, if one is in hand and still here.
    pub(super) fn gizmo_folder(&self) -> Option<NodeId> {
        let folder = self.group_gizmo.folder_in_hand?;
        self.doc.node(folder).filter(|node| !node.is_body()).map(|node| node.id)
    }

    /// Whether a gizmo is on a folder.
    pub(super) fn gizmo_on_folder(&self, gizmo: &Gizmo) -> bool {
        self.doc.node(gizmo.body).is_some_and(|node| !node.is_body())
    }

    /// Every body under a folder, in order.
    pub(super) fn bodies_under(&self, folder: NodeId) -> Vec<NodeId> {
        self.doc
            .subtree_of(folder)
            .map(|range| {
                self.doc.nodes()[range]
                    .iter()
                    .filter(|node| node.is_body())
                    .map(|node| node.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Arm the gizmo on a folder, or say why not: the same two ceilings as
    /// a body's, over every body under it together.
    pub(super) fn arm_group_gizmo(&mut self, folder: NodeId) -> Result<(), String> {
        self.group_gizmo.bases.clear();
        self.group_gizmo.baked = None;
        let bodies = self.bodies_under(folder);
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut resident = 0usize;
        let mut max_scale = f32::MAX;
        for body in &bodies {
            let Some(volume) = self.doc.volume(*body) else {
                continue;
            };
            if let Some((min, max)) = volume.world_bounds() {
                low = low.min(min);
                high = high.max(max);
            }
            let stats = volume.stats();
            resident += size_of::<Volume>() + stats.resident_bytes;
            max_scale = max_scale.min(self.largest_scale_that_fits(stats));
        }
        if low.x > high.x {
            return Err("there is nothing in this folder to move".to_string());
        }
        if resident as f64 > hani_core::DEFAULT_RECLAIM_BUDGET as f64 {
            return Err(format!(
                "this folder's bodies are {:.0} MB and undo can hold {:.0} MB, so the move \
                 could not be undone",
                resident as f64 / (1024.0 * 1024.0),
                hani_core::DEFAULT_RECLAIM_BUDGET as f64 / (1024.0 * 1024.0),
            ));
        }
        if self.doc_stats.resident_bytes as f64 + resident as f64 > super::MAX_VOLUME_BYTES {
            return Err("moving this folder would hold more than the memory allows".to_string());
        }
        self.gizmo = Some(Gizmo::new(folder, low, high, max_scale.clamp(1.0, f32::MAX)));
        self.refresh_gizmo();
        Ok(())
    }

    /// Bake every body under the folder at the gizmo's whole placement, each
    /// from its field as it was when the gizmo armed.
    pub(super) fn rebake_group(&mut self, gizmo: Gizmo) {
        let voxel_size = self.doc.voxel_size();
        let route = gizmo.placement.route(voxel_size);
        let bodies = self.bodies_under(gizmo.body);
        if route == hani_core::Bake::Identity {
            if !self.group_gizmo.bases.is_empty() {
                self.restore_group();
            }
            return;
        }
        if self.group_gizmo.baked.is_some_and(|baked| baked.same_bake(gizmo.placement, voxel_size))
        {
            return;
        }
        let started = std::time::Instant::now();
        for body in bodies {
            let placeholder = Volume::new(voxel_size);
            let Some(previous) = self.doc.replace_volume(body, placeholder) else {
                continue;
            };
            let stale: Vec<hani_core::BrickCoord> = previous.brick_coords().collect();
            let at = self.group_gizmo.bases.iter().position(|(id, _)| *id == body);
            let base = match at {
                Some(at) => {
                    drop(previous);
                    self.group_gizmo.bases.swap_remove(at).1
                }
                None => Box::new(previous),
            };
            let mut baked = super::bake_placed(&base, gizmo.placement, route);
            for coord in stale {
                baked.mark_dirty(coord);
            }
            self.doc.replace_volume(body, baked);
            self.group_gizmo.bases.push((body, base));
            // Released and not forgotten: see `rebake_gizmo`.
            self.shared.release_body(body);
            self.thumbs.geometry_changed(body);
        }
        self.group_gizmo.baked = Some(gizmo.placement);
        self.doc.mark_everything_dirty();
        self.refresh_model_radius();
        self.remesh_dirty();
        self.unsaved = true;
        self.status = Self::bake_summary(&gizmo.placement, route, voxel_size, started.elapsed());
    }

    /// Put every body's field back as it was when the gizmo armed.
    fn restore_group(&mut self) -> bool {
        self.group_gizmo.baked = None;
        let bases = std::mem::take(&mut self.group_gizmo.bases);
        if bases.is_empty() {
            return false;
        }
        let voxel_size = self.doc.voxel_size();
        for (body, base) in bases {
            let Some(baked) = self.doc.replace_volume(body, Volume::new(voxel_size)) else {
                continue;
            };
            let mut restored = *base;
            for coord in baked.brick_coords() {
                restored.mark_dirty(coord);
            }
            self.doc.replace_volume(body, restored);
            self.shared.release_body(body);
            self.thumbs.geometry_changed(body);
        }
        self.doc.mark_everything_dirty();
        self.refresh_model_radius();
        self.remesh_dirty();
        true
    }

    /// Put the gizmo on a folder away: one entry holding every field it
    /// moved, or, abandoned, every field put back.
    pub(super) fn disarm_group(&mut self, commit: bool, placement: Similarity) {
        if self.group_gizmo.bases.is_empty() {
            return;
        }
        if commit {
            self.group_gizmo.baked = None;
            let bases = std::mem::take(&mut self.group_gizmo.bases);
            // Every moved body's symmetry goes with it. See
            // `app/symmetry_frame.rs`.
            let bodies: Vec<NodeId> = bases.iter().map(|(body, _)| *body).collect();
            let mut changes: Vec<hani_core::Change> = bases
                .into_iter()
                .map(|(body, volume)| hani_core::Change::WholeVolume { body, volume })
                .collect();
            changes.extend(self.move_frames(&bodies, placement));
            let before = self.history.stats();
            self.history.push(hani_core::Entry::new(changes));
            self.record_history(before);
            return;
        }
        if self.restore_group() {
            self.status = "put the folder back where it was".to_string();
        }
    }

    /// What the drag draws through its map while a handle is held: the
    /// folder (whose result a combining folder is drawn as) and every body
    /// under it.
    pub(super) fn moving_rows(&self, folder: NodeId, map: Mat4) -> Vec<(NodeId, Mat4)> {
        std::iter::once(folder).chain(self.bodies_under(folder)).map(|id| (id, map)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// The first body and a cube, both in a folder.
    fn grouped() -> (Hani, NodeId, [NodeId; 2]) {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        let first = app.doc.nodes()[0].id;
        let (folder, _) = app.doc.group(first, "Pair").expect("room for a folder");
        app.doc.move_to_folder(cube, Some(folder)).expect("moved");
        update(&mut app, Message::ToolChanged(super::super::Tool::Sculpt));
        (app, folder, [first, cube])
    }

    fn middle(app: &Hani, body: NodeId) -> Vec3 {
        // The surface's own bounds: `world_bounds` rounds out to whole bricks.
        let (low, high) = app
            .doc
            .volume(body)
            .and_then(Volume::surface_bounds)
            .expect("a body with something in it");
        (low + high) * 0.5
    }

    /// With the Transform tool up, pressing a folder's row puts the gizmo on
    /// the folder; a move moves every body under it by the same amount, is
    /// one undo entry, and the undo puts both back.
    #[test]
    fn a_folder_moves_as_one_and_is_undone_as_one() {
        let (mut app, folder, bodies) = grouped();
        update(&mut app, Message::ToolChanged(super::super::Tool::Transform));
        update(&mut app, Message::BodySelected(folder));
        assert_eq!(app.gizmo.map(|gizmo| gizmo.body), Some(folder), "{}", app.status);
        let before: Vec<Vec3> = bodies.iter().map(|body| middle(&app, *body)).collect();

        // Eight voxels along X, as a drag of the arrow would leave it.
        let step = Vec3::X * app.doc.voxel_size() * 8.0;
        if let Some(gizmo) = app.gizmo.as_mut() {
            gizmo.placement = Similarity { translation: step, ..Similarity::IDENTITY };
        }
        app.rebake_gizmo();
        for (body, was) in bodies.iter().zip(&before) {
            let now = middle(&app, *body);
            assert!((now - (*was + step)).length() < 1e-3, "{body:?} went from {was} to {now}");
        }
        let entries = app.history_stats.undo_entries;
        update(&mut app, Message::ToolChanged(super::super::Tool::Transform));
        assert!(app.gizmo.is_none());
        assert_eq!(app.history_stats.undo_entries, entries + 1, "not one entry for the folder");
        update(&mut app, Message::Undo);
        for (body, was) in bodies.iter().zip(&before) {
            assert!((middle(&app, *body) - *was).length() < 1e-3, "{body:?} not put back");
        }
    }

    /// A body's row put down the folder: the gizmo goes back on the body.
    #[test]
    fn a_body_row_puts_the_folder_down() {
        let (mut app, folder, [first, _]) = grouped();
        update(&mut app, Message::ToolChanged(super::super::Tool::Transform));
        update(&mut app, Message::BodySelected(folder));
        assert_eq!(app.gizmo.map(|gizmo| gizmo.body), Some(folder));
        update(&mut app, Message::BodySelected(first));
        assert_eq!(app.gizmo.map(|gizmo| gizmo.body), Some(first));
    }

    /// While a handle is held every body under the folder, and the folder
    /// (whose result is drawn if it combines), is drawn through the map.
    #[test]
    fn a_held_folder_draws_every_body_moved() {
        let (app, folder, bodies) = grouped();
        let rows = app.moving_rows(folder, Mat4::from_translation(Vec3::X));
        let ids: Vec<NodeId> = rows.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [folder, bodies[0], bodies[1]]);
    }
}
