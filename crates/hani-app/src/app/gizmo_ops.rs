// SPDX-License-Identifier: AGPL-3.0-only

//! The transform gizmo on one body: arming it, carrying a drag, baking the
//! field at its placement, and putting it away. Moved out of `app.rs`
//! (issue 189). A folder's gizmo is `group_gizmo.rs`.

use super::*;

/// How much a scaled bake is assumed to overshoot the square law.
///
/// Measured rather than chosen: `Volume::warped` of a 10 mm sphere at 0.25 mm
/// gives 4.66x its resident bytes at scale 2 and 15.25x at scale 4, against the
/// 4 and 16 a pure square law predicts. The excess at small scales is the
/// narrow band, which is a fixed thickness in VOXELS and so a bigger share of a
/// small shell. One number covering both, rounded up.
///
/// See [`Hani::largest_scale_that_fits`], its only reader.
const SCALE_GROWTH_MARGIN: f64 = 1.25;

impl Hani {
    /// Rebuild the transform gizmo and hand it to the renderer.
    ///
    /// **Beside `refresh_cube` and never inside `refresh_overlay`.** That
    /// function's own header says it is deliberately not camera dependent --
    /// the ring and the mirror planes are world-space geometry, so a camera
    /// moving under them needs no rebuild. A gizmo held at a constant size on
    /// screen falsifies exactly that premise: every one of its dimensions is a
    /// number of pixels converted through the camera. So it travels the way the
    /// cube does, through `publish_camera`, and by the same buffer rotation, so
    /// nothing allocates once warm.
    pub(super) fn refresh_gizmo(&mut self) {
        let mut batch = std::mem::take(&mut self.gizmo_batch);
        match &self.gizmo {
            Some(gizmo) => gizmo::build(&mut batch, &self.camera, self.viewport_size, gizmo),
            // Cleared rather than left: a stale gizmo is one that is still
            // drawn on a body nothing is armed on.
            None => batch.clear(),
        }
        self.shared.swap_gizmo(&mut batch);
        self.gizmo_batch = batch;
        // The body itself, drawn where the drag has it: `app/moving.rs`.
        // A folder's bodies all move with it: `app/group_gizmo.rs`.
        let drawn = match moving::drawn(self.gizmo.as_ref()) {
            Some((folder, map)) if self.doc.node(folder).is_some_and(|node| !node.is_body()) => {
                self.moving_rows(folder, map)
            }
            drawn => drawn.into_iter().collect(),
        };
        self.shared.set_moving(drawn);
        // The planes go with the placement. See `app/symmetry_frame.rs`.
        if !self.symmetry_settings().is_off() {
            self.refresh_overlay();
        }
    }

    /// Take the active body under the gizmo, or say why not.
    ///
    /// # The refusal is at ARM time, before anything is allocated
    ///
    /// `merge_plan`'s header states the rule this follows: the user is asked
    /// before the allocation, and never after it. Arming is the commitment
    /// point here and not the first release, because arming is what promises to
    /// hold a second copy of the body for as long as the gizmo is up -- and by
    /// the first release the memory is already spent and the field already
    /// baked.
    ///
    /// Two ceilings, and they fail differently:
    ///
    /// * The reclaim allowance. A field larger than it would be evicted from
    ///   history the instant it was pushed, so the transform would be
    ///   **un-undoable** -- and a free-angle transform has no "do it backwards"
    ///   recovery the way a quarter turn does. That is the one that matters.
    /// * The document's own volume ceiling, against holding base and baked at
    ///   once.
    ///
    /// Both name the voxel size that would fit, from the same square law
    /// [`Hani::too_fine_for_the_pool`] uses: a surface has a fixed area, so
    /// halving the voxel quadruples the shell that stores it.
    pub(super) fn arm_gizmo(&mut self) -> Result<(), String> {
        if self.gizmo.is_some() {
            return Ok(());
        }
        // A folder in hand takes the gizmo. See `app/group_gizmo.rs`.
        if let Some(folder) = self.gizmo_folder() {
            return self.arm_group_gizmo(folder);
        }
        // A fresh session has baked nothing, whatever the last one baked.
        self.gizmo_baked = None;
        let body = self.doc.active();
        if !self.active_is_drawn() {
            return Err("that body is hidden — turn its eye on to move it".to_string());
        }
        let volume = self.doc.active_volume();
        let Some((low, high)) = volume.world_bounds() else {
            return Err("there is nothing in this body to move".to_string());
        };
        let stats = volume.stats();
        let resident = stats.resident_bytes;
        let entry_bytes = size_of::<hani_core::Volume>() + resident;

        let coarser = |headroom: f64| self.doc.voxel_size() / headroom.sqrt() as f32 * 1.03;
        if entry_bytes as f64 > hani_core::DEFAULT_RECLAIM_BUDGET as f64 {
            let headroom = hani_core::DEFAULT_RECLAIM_BUDGET as f64 / entry_bytes.max(1) as f64;
            return Err(format!(
                "this body is {:.0} MB and undo can hold {:.0} MB, so the move could not be \
                 undone — {:.3} mm is the coarsest voxel that would fit",
                entry_bytes as f64 / (1024.0 * 1024.0),
                hani_core::DEFAULT_RECLAIM_BUDGET as f64 / (1024.0 * 1024.0),
                coarser(headroom),
            ));
        }
        // Base and baked are both resident for as long as the gizmo is up.
        let peak = self.doc_stats.resident_bytes as f64 + resident as f64;
        if peak > MAX_VOLUME_BYTES {
            let headroom = MAX_VOLUME_BYTES / peak.max(1.0);
            return Err(format!(
                "moving this body needs about {:.1} GB while the gizmo is up, against a {:.0} GB \
                 ceiling — {:.3} mm is the coarsest voxel that would fit",
                peak / (1024.0 * 1024.0 * 1024.0),
                MAX_VOLUME_BYTES / (1024.0 * 1024.0 * 1024.0),
                coarser(headroom),
            ));
        }

        let max_scale = self.largest_scale_that_fits(stats);
        self.gizmo = Some(gizmo::Gizmo::new(body, low, high, max_scale));
        self.refresh_gizmo();
        Ok(())
    }

    /// The largest TOTAL scale a bake of this body could be built at.
    ///
    /// # Why this is a clamp on the gesture and not a refusal at the release
    ///
    /// `merge_plan`'s rule is that the user is asked before the allocation and
    /// never after it, and a gizmo has no moment in which to ask: the gesture
    /// IS the question, so a refusal that arrives once the pointer has been
    /// released is an answer too late to act on. Folding the ceiling into the
    /// gesture instead means the preview box stops growing exactly where the
    /// bake would stop fitting, and everything the user is shown is a placement
    /// that can actually be built. See the clamp in `gizmo::drag`.
    ///
    /// Without it there is no bound anywhere on the path. `Volume::warped`
    /// builds a dense grid of destination brick coordinates between the moved
    /// bounds with no cap, `arm_gizmo`'s two refusals are evaluated once
    /// against the UNSCALED body, and `Similarity::then` composes gestures by
    /// multiplying their scales.
    ///
    /// # The prediction, and where each term comes from
    ///
    /// Two terms, because the field and the mask do not grow at the same rate
    /// and only the field obeys the square law the rest of this file uses.
    ///
    /// * A surface has fixed area, so scaling by `s` multiplies the shell that
    ///   stores it by `s * s`. Measured on a 10 mm sphere at 0.25 mm:
    ///   4.66x resident at `s = 2` and 15.25x at `s = 4`, against 4 and 16 --
    ///   which is where [`SCALE_GROWTH_MARGIN`] comes from.
    /// * `MaskField::warped` has no interior shortcut to collapse a thickened
    ///   protection region with, so one source mask brick's forward image is
    ///   `s * s * s` destination bricks. A heavily masked body scaled far up is
    ///   the case a single square law would miss.
    ///
    /// `A s^2 + B s^3 <= headroom` has no closed form worth writing, so the
    /// answer is bisected. It runs once, at arming.
    ///
    /// Never below 1.0: a body can always be placed at the size it already is,
    /// and a ceiling that said otherwise would make even the identity
    /// unreachable at the moment the document is fullest.
    pub(super) fn largest_scale_that_fits(&self, stats: hani_core::VolumeStats) -> f32 {
        // What the BAKED field may occupy. The base is held aside for as long
        // as the gizmo is up and `doc_stats` was measured with it still in the
        // document, so the two cancel: the room left for the baked field is the
        // room left beside the document as it stands.
        let headroom = MAX_VOLUME_BYTES - self.doc_stats.resident_bytes as f64;
        let mask = stats.mask_bytes as f64;
        let field = (stats.resident_bytes as f64 - mask).max(0.0);
        let cost =
            |scale: f64| (field * scale * scale + mask * scale.powi(3)) * SCALE_GROWTH_MARGIN;

        let mut fits = 1.0f64;
        // Above `gizmo::MAX_SCALE`, so the search never becomes the binding
        // constraint on a document with room to spare.
        let mut refused = 64.0f64;
        for _ in 0..24 {
            let middle = (fits + refused) * 0.5;
            if cost(middle) <= headroom {
                fits = middle;
            } else {
                refused = middle;
            }
        }
        fits as f32
    }

    /// Rebuild the body from the field the gizmo armed on.
    ///
    /// # Always from the base, and that is the whole design
    ///
    /// The obvious implementation bakes the current field through whatever the
    /// last drag did, which makes N gestures cost N trilinear passes. Placement
    /// is fiddly: thirty nudges over a session is an ordinary number, and
    /// thirty passes is detail the user sculpted, gone, with nothing having
    /// said so -- discovered at the printer. Baking from the base instead
    /// bounds the erosion at **one pass per arming**, whatever the user does in
    /// between, and makes dragging in a circle and back exactly the identity
    /// rather than approximately it.
    ///
    /// The exact routes cost zero passes, not one: a snapped move is a
    /// value-exact gather and a snapped quarter turn is a permutation, so an
    /// entire session of snapped gestures leaves the field bit-identical to
    /// what it was.
    ///
    /// The base is taken out of the document on the FIRST bake rather than at
    /// arming, so that arming alone never touches the field -- pressing `w` and
    /// pressing it again has to be free.
    pub(super) fn rebake_gizmo(&mut self) {
        let Some(gizmo) = self.gizmo else {
            return;
        };
        // See `app/group_gizmo.rs`.
        if self.gizmo_on_folder(&gizmo) {
            return self.rebake_group(gizmo);
        }
        let body = gizmo.body;
        let voxel_size = self.doc.voxel_size();
        let route = gizmo.placement.route(voxel_size);

        // Nothing is asked for. Two cases, and the second one is the trap.
        //
        // Nothing baked yet: leave the document alone entirely, so an armed
        // gizmo that is only looked at costs nothing at all.
        //
        // Something baked and then undone by hand -- a nudge out and a nudge
        // back, which is an ordinary way to change your mind -- must not go on
        // to `base.shifted(IVec3::ZERO)`. That route is real work:
        // `Volume::duplicated`'s own header quotes 1.53 GiB on the dragon, and
        // it would be spent producing a field identical to the one already
        // held. Worse, it leaves `gizmo_base` set, so the disarm pushes a
        // whole-field undo entry for a placement that changed nothing --
        // charged against the reclaim allowance, where `History::trim` can
        // evict real strokes to make room for it. Putting the base BACK returns
        // the session to armed-and-untouched, which is exact, allocates
        // nothing, and makes the disarm correctly push no entry at all.
        if route == hani_core::Bake::Identity {
            if self.gizmo_base.is_none() {
                return;
            }
            let started = Instant::now();
            self.restore_base(body);
            self.status =
                Self::bake_summary(&gizmo.placement, route, voxel_size, started.elapsed());
            self.gizmo_baked = None;
            return;
        }

        // Already baked, and nothing has moved since. Covers a press and
        // release on a handle with no motion, and a drag out and back across
        // two presses.
        if self.gizmo_baked.is_some_and(|baked| baked.same_bake(gizmo.placement, voxel_size)) {
            return;
        }

        // Move the field out on the first bake. Two steps rather than one so
        // the old baked field is dropped BEFORE the new one is built: peak
        // memory stays at two copies of the body rather than three.
        let placeholder = hani_core::Volume::new(voxel_size);
        let Some(previous) = self.doc.replace_volume(body, placeholder) else {
            return;
        };
        let stale: Vec<hani_core::BrickCoord> = previous.brick_coords().collect();
        let base = match self.gizmo_base.take() {
            Some(base) => {
                drop(previous);
                base
            }
            None => Box::new(previous),
        };

        let started = Instant::now();
        let mut baked = bake_placed(&base, gizmo.placement, route);
        // The bricks the outgoing field occupied, marked in the INCOMING one:
        // without this the renderer keeps drawing triangles nothing will ever
        // overwrite. The same rule `Document::resample` follows.
        for coord in stale {
            baked.mark_dirty(coord);
        }
        self.doc.replace_volume(body, baked);
        self.gizmo_base = Some(base);
        self.gizmo_baked = Some(gizmo.placement);

        // The slots the old field held are in nobody's dirty set now -- the
        // bricks are at new coordinates -- so they have to be let go, and the
        // whole-document remesh is the other half: freeing pool space without
        // re-offering everything takes the pool-full warning down and leaves
        // the geometry that was refused missing.
        //
        // **`release_body` and NOT `forget_body`, and that distinction is the
        // whole of a bug that shipped.** A forget means the body has left the
        // document, so `SharedFrame::apply` drops every queued upload and the
        // pending thumbnail naming it. Here the body is still very much in the
        // document and its new meshes are queued in the same frame, so a forget
        // threw them away: the cube disappeared from the viewport, its
        // thumbnail went black, and the status line said "moved 50.63 mm --
        // exact". The field was never touched, which is why nothing in the
        // document looked wrong.
        self.shared.release_body(body);
        self.doc.mark_everything_dirty();
        self.refresh_model_radius();
        self.remesh_dirty();
        self.unsaved = true;
        self.thumbs.geometry_changed(body);
        self.status = Self::bake_summary(&gizmo.placement, route, voxel_size, started.elapsed());
    }

    /// What just happened to the field, in the words that matter.
    ///
    /// **The route is named on every bake, not only the lossy ones.** A user
    /// who is told nothing when a move is exact has no way to learn that some
    /// of them are not, and the whole point of snapping by default is that the
    /// ordinary gesture is free. Saying so is what teaches that.
    pub(super) fn bake_summary(
        placement: &hani_core::Similarity,
        route: hani_core::Bake,
        voxel_size: f32,
        elapsed: std::time::Duration,
    ) -> String {
        let (axis, angle) = placement.rotation.to_axis_angle();
        let degrees = crate::camera::wrap_angle(angle).to_degrees() * axis.length().max(1.0);
        let mut what = Vec::new();
        if placement.translation.length() > voxel_size * 0.5 {
            what.push(format!("moved {:.2} mm", placement.translation.length()));
        }
        if degrees.abs() > 0.05 {
            what.push(format!("turned {:.0}°", degrees.abs()));
        }
        if (placement.scale - glam::Vec3::ONE).abs().max_element() > 1.0e-4 {
            // Named per axis when they differ, because "scaled to 60%" over a
            // squash that only touched Z is the status line telling the user
            // something that did not happen.
            if placement.is_uniform_scale() {
                what.push(format!("scaled to {:.0}%", placement.scale.x * 100.0));
            } else {
                what.push(format!(
                    "resized to {:.0}/{:.0}/{:.0}%",
                    placement.scale.x * 100.0,
                    placement.scale.y * 100.0,
                    placement.scale.z * 100.0
                ));
            }
        }
        if what.is_empty() {
            what.push("put back where it started".to_string());
        }
        let ms = elapsed.as_secs_f64() * 1000.0;
        if route.is_lossy() {
            format!(
                "{} — resampled in {ms:.0} ms; turning it back will not restore it exactly, \
                 and undo will",
                what.join(", ")
            )
        } else {
            format!("{} — exact, {ms:.0} ms, not one voxel recomputed", what.join(", "))
        }
    }

    /// Put the gizmo away.
    ///
    /// `commit` decides which of the two answers the user asked for, and the
    /// split follows the convention every modelling tool already has: Escape
    /// abandons, anything else commits.
    ///
    /// * Committing pushes ONE [`hani_core::Change::WholeVolume`] holding the
    ///   field as it was when the gizmo armed -- one entry per SESSION and not
    ///   one per drag, because a per-drag entry would be a full field each and
    ///   thirty of them would evict each other and every stroke behind them.
    /// * Abandoning puts that field straight back and pushes nothing, so a
    ///   session the user changed their mind about leaves no trace in the
    ///   history at all.
    ///
    /// Either way the base is given up, which is what makes the erosion bound
    /// -- one pass per arming -- a statement about the whole session.
    pub(super) fn disarm_gizmo(&mut self, commit: bool) {
        let Some(gizmo) = self.gizmo.take() else {
            return;
        };
        self.refresh_gizmo();
        // See `app/group_gizmo.rs`.
        if self.gizmo_on_folder(&gizmo) {
            return self.disarm_group(commit, gizmo.placement);
        }
        if self.gizmo_base.is_none() {
            // Armed and never baked, or baked and then returned to the identity
            // by hand -- `rebake_gizmo` puts the base back in that case, which
            // is what makes this branch cover both. Nothing was moved, so there
            // is nothing to commit and nothing to put back.
            return;
        }
        let body = gizmo.body;

        if commit {
            self.gizmo_baked = None;
            let base = self.gizmo_base.take().expect("checked just above");
            let before = self.history.stats();
            let mut changes = vec![hani_core::Change::WholeVolume { body, volume: base }];
            // The body's symmetry goes with it. See `app/symmetry_frame.rs`.
            changes.extend(self.move_frames(&[body], gizmo.placement));
            self.history.push(hani_core::Entry::new(changes));
            self.record_history(before);
            return;
        }

        if self.restore_base(body) {
            self.status = "put the body back where it was".to_string();
        }
    }

    /// Put the field the gizmo armed on back into the document, exactly, and
    /// give the base up.
    ///
    /// Two callers, and they want it for two different reasons: Escape
    /// abandoning a session, and a gesture that has returned the placement to
    /// the identity. Both leave the document holding the field the gizmo armed
    /// on and `gizmo_base` empty, which is the state a disarm has nothing to
    /// commit from -- so a session the user changed their mind about leaves no
    /// entry on the stack whether they said so with Escape or with the pointer.
    ///
    /// Answers whether there was a base to put back.
    fn restore_base(&mut self, body: NodeId) -> bool {
        self.gizmo_baked = None;
        let Some(base) = self.gizmo_base.take() else {
            return false;
        };
        let voxel_size = self.doc.voxel_size();
        let placeholder = hani_core::Volume::new(voxel_size);
        let Some(baked) = self.doc.replace_volume(body, placeholder) else {
            return false;
        };
        // The bricks the baked field occupied, marked in the one going back:
        // without them the renderer keeps drawing triangles nothing will ever
        // overwrite. The same pairing `rebake_gizmo` makes in the other
        // direction.
        let stale: Vec<hani_core::BrickCoord> = baked.brick_coords().collect();
        drop(baked);
        let mut restored = base;
        for coord in stale {
            restored.mark_dirty(coord);
        }
        restored.mark_everything_dirty();
        self.doc.replace_volume(body, *restored);
        self.shared.release_body(body);
        self.doc.mark_everything_dirty();
        self.refresh_model_radius();
        self.remesh_dirty();
        true
    }

    /// Carry a live gizmo drag to the pointer.
    ///
    /// **Nothing is baked here.** The placement is arithmetic and the preview
    /// box is twelve lines; the field is rebuilt exactly once, on the release,
    /// by [`Hani::rebake_gizmo`]. That is the difference between a gesture
    /// costing one trilinear pass and costing one per pointer event.
    ///
    /// Shift frees the snap rather than imposing it, which is the opposite of
    /// every other application's convention and is deliberate: here the snapped
    /// gesture is the LOSSLESS one, so the default has to be the one that costs
    /// the surface nothing and the modifier has to be the one that admits a
    /// price. See `gizmo::ROTATION_SNAP`.
    pub(super) fn transform_to(&mut self, handle: gizmo::Handle, position: Vec2) {
        let (Some(drag), Some(gizmo)) = (self.drag, self.gizmo) else {
            return;
        };
        let gesture = gizmo::drag(
            &self.camera,
            self.viewport_size,
            &gizmo,
            handle,
            drag.origin,
            position,
            (!self.shift).then(|| self.doc.voxel_size()),
        );
        let placement = gizmo.pinned.then(gesture);
        if let Some(live) = &mut self.gizmo {
            live.placement = placement;
        }
        self.refresh_gizmo();
        // What the release is ABOUT to cost, before it is spent. The armed
        // badge the plan asked for, in the one place the interface already
        // reads from.
        let route = placement.route(self.doc.voxel_size());
        // **The clamp says so.** A preview box that stops growing under the
        // pointer with nothing said reads as the gizmo having lost the drag.
        // Naming the ceiling is what turns it into an answer.
        let capped = matches!(handle, gizmo::Handle::Uniform | gizmo::Handle::Scale(_))
            && placement.scale.max_element() >= gizmo.max_scale - 1.0e-3
            && gizmo.max_scale < gizmo::MAX_SCALE;
        if capped {
            self.status = format!(
                "scale — {:.0}% is as large as this body fits in memory",
                gizmo.max_scale * 100.0
            );
            return;
        }
        self.status = format!(
            "{} — {}",
            handle.verb(),
            if route.is_lossy() { "will resample" } else { "exact" }
        );
    }
}

#[cfg(test)]
mod gizmo_tests {
    use super::*;
    use hani_core::{Bake, BrickCoord, warps_made_on_this_thread};
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        // Every gesture below is measured in widget pixels, so the application
        // has to know how big the widget is before the first one.
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        app
    }

    /// **A transform must not tell the renderer the body has gone.**
    ///
    /// `SharedFrame::apply` drops every queued upload and the pending thumbnail
    /// naming a FORGOTTEN body, which is right for a delete and catastrophic
    /// for a move: the new meshes are queued in the same frame, so forgetting
    /// throws away the very geometry that replaces what was released. That
    /// shipped. Thomas moved a cube, the status line said "moved 50.63 mm --
    /// exact", and the cube vanished from the viewport with a black thumbnail
    /// while its field sat intact in the document.
    ///
    /// The old test asserted `world_bounds()` had changed -- that the DATA
    /// moved -- and passed the whole time, which is this project's oldest
    /// failure wearing a new hat. This asserts what the RENDERER was told.
    #[test]
    fn a_transform_releases_the_body_rather_than_forgetting_it() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let body = app.doc.active();
        let _ = app.shared.take_forgotten_for_tests();
        let _ = app.shared.take_released_for_tests();

        arm(&mut app);
        drag_the_x_arrow(&mut app, 120.0);

        let forgotten = app.shared.take_forgotten_for_tests();
        let released = app.shared.take_released_for_tests();
        assert!(
            !forgotten.contains(&body),
            "the moved body was FORGOTTEN, so its re-meshed bricks and its thumbnail              are dropped by SharedFrame::apply and it disappears from the screen"
        );
        assert!(
            released.contains(&body),
            "the moved body's stale slots were never released, so it draws at both              its old and its new position: forgotten {forgotten:?} released {released:?}"
        );
        // And the field is untouched by any of this, which is what made the bug
        // invisible from the document's side.
        assert!(app.doc.active_volume().stats().dense_bricks > 0, "the move emptied the body");
    }

    /// A folder under the gizmo, dragged by its arrow through the pointer:
    /// every body under it goes the same way, and the bodies outside it do
    /// not. See `app/group_gizmo.rs`.
    #[test]
    fn a_folder_dragged_by_its_arrow_moves_every_body_under_it() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        let first = app.doc.nodes()[0].id;
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cylinder));
        let outside = app.doc.active();
        let (folder, _) = app.doc.group(first, "Pair").expect("room for a folder");
        app.doc.move_to_folder(cube, Some(folder)).expect("moved");
        // Combining, so that what is drawn is the folder's result.
        app.doc.set_bool_state(
            folder,
            hani_core::combine::BoolState {
                combines: true,
                op: hani_core::combine::BoolOp::Add,
                blend: 0,
            },
        );
        update(&mut app, Message::ToolChanged(Tool::Sculpt));
        let result_middle = |app: &Hani| {
            let (low, high) =
                app.doc.boolean_result(folder).and_then(Volume::surface_bounds).expect("result");
            (low + high) * 0.5
        };
        let result_before = result_middle(&app);
        let middle = |app: &Hani, body| {
            let (low, high) = app.doc.volume(body).and_then(Volume::surface_bounds).expect("body");
            (low + high) * 0.5
        };
        let before = [middle(&app, first), middle(&app, cube), middle(&app, outside)];
        arm(&mut app);
        update(&mut app, Message::BodySelected(folder));
        assert_eq!(app.gizmo.map(|gizmo| gizmo.body), Some(folder), "{}", app.status);
        drag_the_x_arrow(&mut app, 120.0);
        let after = [middle(&app, first), middle(&app, cube), middle(&app, outside)];
        let moved = after[0] - before[0];
        assert!(moved.length() > 1.0, "the folder did not move: {}", app.status);
        assert!((after[1] - before[1] - moved).length() < 1e-3, "the cube went another way");
        assert!((after[2] - before[2]).length() < 1e-3, "a body outside the folder moved");
        let result_after = result_middle(&app);
        assert!(
            (result_after - result_before - moved).length() < 1e-3,
            "the result stayed at {result_before} where its rows went to {after:?}"
        );
    }

    fn arm(app: &mut Hani) {
        update(app, Message::ToolChanged(Tool::Transform));
        assert_eq!(app.tool, Tool::Transform, "the gizmo did not arm: {}", app.status);
        assert!(app.gizmo.is_some(), "the tool is up with no gizmo under it");
    }

    /// Where the gizmo's own origin lands on screen.
    fn gizmo_centre(app: &Hani) -> Vec2 {
        let gizmo = app.gizmo.expect("armed");
        let aspect = SIZE.x / SIZE.y;
        let clip = app.camera.view_projection(aspect) * gizmo.origin().extend(1.0);
        let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
        Vec2::new((ndc.x + 1.0) * 0.5 * SIZE.x, (1.0 - ndc.y) * 0.5 * SIZE.y)
    }

    fn moved(app: &mut Hani, at: Vec2) {
        app.on_pointer(PointerEvent::Moved { position: Vector::new(at.x, at.y), size: SIZE });
    }

    /// A press, one or more moves, and a release, through the real pointer path.
    ///
    /// **It asserts that the press was actually taken as a handle**, and that
    /// is not belt and braces: a press that misses the gizmo by a pixel falls
    /// through to choosing a body, the placement then never changes, and every
    /// test built on this helper passes while testing nothing. That happened
    /// once already, to an earlier version of `drag_the_x_arrow` that pressed
    /// at the shaft's screen x-offset with y left at zero.
    fn gesture(app: &mut Hani, from: Vec2, through: &[Vec2]) {
        moved(app, from);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(from.x, from.y),
            size: SIZE,
        });
        assert!(
            matches!(app.drag.map(|drag| drag.kind), Some(DragKind::Transforming(_))),
            "the press at {from:?} did not land on a handle, so this gesture tests nothing"
        );
        for at in through {
            moved(app, *at);
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
    }

    /// Where a world point lands, in widget pixels.
    fn project(app: &Hani, point: Vec3) -> Vec2 {
        let aspect = SIZE.x / SIZE.y;
        let clip = app.camera.view_projection(aspect) * point.extend(1.0);
        let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
        Vec2::new((ndc.x + 1.0) * 0.5 * SIZE.x, (1.0 - ndc.y) * 0.5 * SIZE.y)
    }

    /// A pixel on the X shaft, and the unit screen direction that runs along it.
    ///
    /// **Both components of the projection**, because a world axis leans on
    /// screen: the X arrow of the default camera runs down and to the right, so
    /// a press at its x-offset with y left at zero is eight pixels off the
    /// shaft and lands or misses depending on where the gizmo happens to be.
    fn x_shaft_grip(app: &Hani) -> (Vec2, Vec2) {
        let gizmo = app.gizmo.expect("armed");
        let origin = gizmo.origin();
        let scale = crate::gizmo::world_per_pixel(&app.camera, origin, SIZE.y);
        let middle = project(app, origin);
        // Just over half way along, clear of the centre box and the arrowhead.
        let grip = project(app, origin + Vec3::X * (34.0 * scale));
        (grip, (grip - middle).normalize())
    }

    /// Every stored voxel of the active body, for comparing two states value by
    /// value rather than through a sampled surface.
    fn field(app: &Hani) -> Vec<(BrickCoord, Vec<f32>)> {
        let volume = app.doc.active_volume();
        let mut coords: Vec<BrickCoord> = volume.brick_coords().collect();
        coords.sort_unstable();
        coords
            .into_iter()
            .map(|coord| {
                let mut values = Vec::new();
                let origin = coord.origin();
                for index in 0..(hani_core::BRICK_DIM as i32).pow(3) {
                    let dim = hani_core::BRICK_DIM as i32;
                    let local =
                        glam::IVec3::new(index % dim, (index / dim) % dim, index / (dim * dim));
                    values.push(volume.sample_voxel(origin + local));
                }
                (coord, values)
            })
            .collect()
    }

    /// A drag along the X arrow, `pixels` of screen travel.
    fn drag_the_x_arrow(app: &mut Hani, pixels: f32) {
        let (grip, along) = x_shaft_grip(app);
        gesture(app, grip, &[grip + along * pixels]);
    }

    /// A free-angle drag round a ring, from `+u` toward `+v`.
    /// **A per-axis scale, through the real pointer path.**
    ///
    /// The handle is easy to ship dead: the variant existed for one build with
    /// nothing constructing it, which the compiler said out loud, and then the
    /// hit-test that constructed it swallowed every ring press instead. So this
    /// drives a press on the box, asserts the press was TAKEN as that handle,
    /// and then asserts the body actually changed shape rather than size.
    #[test]
    fn a_per_axis_drag_squashes_one_axis_and_leaves_the_others() {
        let mut app = app();
        arm(&mut app);
        let gizmo = app.gizmo.expect("armed");
        let origin = gizmo.origin();
        let scale = crate::gizmo::world_per_pixel(&app.camera, origin, SIZE.y);

        // The X box, at SCALE_BOX_PX along X.
        let axis = Vec3::X;
        let at = project(&app, origin + axis * (92.0 * scale));
        assert_eq!(
            crate::gizmo::pick(&app.camera, Vec2::new(SIZE.x, SIZE.y), &gizmo, at),
            Some(crate::gizmo::Handle::Scale(0)),
            "the press did not land on the per-axis box, so this tests nothing"
        );

        let before = app.doc.active_volume().world_bounds().expect("a body");
        // Inward along the axis: a squash.
        let to = project(&app, origin + axis * (46.0 * scale));
        gesture(&mut app, at, &[to]);
        let after = app.doc.active_volume().world_bounds().expect("a body");

        let span = |b: (Vec3, Vec3)| b.1 - b.0;
        let (was, now) = (span(before), span(after));
        assert!(now.x < was.x - app.doc.voxel_size(), "x did not squash: {was:?} then {now:?}");
        assert!(
            (now.y - was.y).abs() < was.y * 0.15 && (now.z - was.z).abs() < was.z * 0.15,
            "a per-axis scale changed the other two axes: {was:?} then {now:?}"
        );
        assert!(
            app.status.contains("resized"),
            "the status line called a per-axis scale something else: {}",
            app.status
        );
    }

    fn turn_a_ring(app: &mut Hani, radians: f32) {
        let gizmo = app.gizmo.expect("armed");
        let origin = gizmo.origin();
        let scale = crate::gizmo::world_per_pixel(&app.camera, origin, SIZE.y);
        // The Y ring, whose plane is the one the default camera looks most
        // squarely at, so both its ends project well clear of the middle.
        // From the constant, not a literal: the ring moved once already and a
        // stale copy here would silently aim this at empty space.
        let radius = crate::gizmo::RING_PX * scale;
        let from = project(app, origin + Vec3::Z * radius);
        let to =
            project(app, origin + (Vec3::Z * radians.cos() + Vec3::X * radians.sin()) * radius);
        let free = app.shift;
        app.shift = true;
        gesture(app, from, &[to]);
        app.shift = free;
    }

    // ------------------------------------------------------- the whole point

    /// **Coarsening the lattice must pull the camera up to the new floor.**
    ///
    /// `set_lattice` writes the voxel size and the content radius, from which
    /// the distance floor and ceiling are derived, and never re-checked the
    /// distance against them. One press of "coarser" from 0.25 mm takes the
    /// floor from 0.500 to 2.000 with a floored camera still at 0.500, and the
    /// next wheel notch INWARD then jumps it out to 2.000 and drags the target
    /// four times away from its anchor -- a zoom in that flies backwards.
    ///
    /// The clamp also publishes, because neither `resample` nor `remove_body`
    /// does and `Message::Frame` only publishes while a flight is running, so
    /// the corrected distance would otherwise sit in `self.camera` while
    /// `SharedFrame` held the old one. There is no camera getter on
    /// `SharedFrame` to assert that through, so it is stated here rather than
    /// claimed as a check.
    #[test]
    fn coarsening_the_lattice_pulls_the_camera_up_to_the_new_floor() {
        let mut app = app();

        // Put the camera on its floor at the current lattice.
        app.camera.distance = app.camera.min_distance();
        app.publish_camera();
        let floor_before = app.camera.min_distance();
        let sat_on_the_floor = app.camera.distance;

        let coarse = app.doc.voxel_size() * 4.0;
        app.doc.resample(coarse);
        app.refresh_camera_lattice();

        let floor_after = app.camera.min_distance();
        assert!(
            floor_after > floor_before * 2.0,
            "the fixture did not actually raise the floor: {floor_before} then {floor_after}"
        );
        assert!(
            sat_on_the_floor < floor_after,
            "the camera was not under the new floor to begin with, so this tests nothing"
        );
        assert!(
            app.camera.distance >= floor_after - 1.0e-6,
            "the camera is at {} against a floor of {floor_after}, so the next notch inward \
             will jump it outward",
            app.camera.distance
        );
    }

    /// **A squash after a turn must land on the axis the handle was drawn on.**
    ///
    /// `Similarity::then` composes `R2 R1 S2 S1`, so a scale composed onto a
    /// rotation already in `pinned` squashes along the BODY's axes -- while the
    /// gizmo drew its boxes, and took its drag direction, from the WORLD axes.
    /// Measured on an ordinary quarter-turn-then-squash-to-0.6 with an
    /// off-origin pivot: 8.49 mm between where the handle said the material
    /// would go and where it went.
    ///
    /// **The default gesture reaches this.** `ROTATION_SNAP` is a quarter turn,
    /// so an unshifted ring drag leaves a non-identity rotation in `placement`;
    /// the next press latches it into `pinned` and any scale drag after that
    /// composes onto it.
    ///
    /// Asserts the composed placement against the STEPWISE definition -- turn,
    /// then squash along the body -- rather than against whatever the code
    /// produces, because a test written the other way round canonises the bug.
    #[test]
    fn a_squash_after_a_quarter_turn_lands_on_the_axis_the_handle_was_drawn_on() {
        let mut app = app();
        arm(&mut app);

        // A snapped quarter turn, which is the default unshifted ring drag.
        turn_a_ring(&mut app, std::f32::consts::FRAC_PI_2);
        let turned = app.gizmo.expect("armed").placement;
        assert!(
            turned.rotation.angle_between(glam::Quat::IDENTITY) > 0.1,
            "the fixture did not actually turn the body: {:?}",
            turned.rotation
        );

        let origin = app.gizmo.expect("armed").origin();
        let scale = crate::gizmo::world_per_pixel(&app.camera, origin, SIZE.y);

        // **The Y box, not the X one.** `turn_a_ring` grabs `Ring(0)` and so
        // turns about X, which leaves X exactly where it was -- a fixture
        // aimed at the X box passes on a broken build, which is what the first
        // version of this test did.
        let body_y = turned.rotation * Vec3::Y;
        assert!(
            body_y.distance(Vec3::Y) * (92.0 * scale) > 20.0 * scale,
            "the turn moved the Y box by less than a grab radius, so the two placements are \
             indistinguishable and this test cannot fail"
        );

        let at = project(&app, origin + body_y * (92.0 * scale));
        assert_eq!(
            crate::gizmo::pick(
                &app.camera,
                Vec2::new(SIZE.x, SIZE.y),
                &app.gizmo.expect("armed"),
                at
            ),
            Some(crate::gizmo::Handle::Scale(1)),
            "after a turn the Y scale box is not where the body's own Y points, so the handle \
             is drawn somewhere the squash will not go"
        );
    }

    /// **A composed scale must not pass the floor on any axis.**
    ///
    /// The uniform handle bounded its floor against
    /// `pinned.scale.max_element()`. After a per-axis squash to (1, 1, 0.05)
    /// that reads 1.0 again, so the floor was computed as though nothing had
    /// been squashed -- and `Similarity::then` multiplies scales component by
    /// component, so the next uniform shrink composed to (0.05, 0.05, 0.0025):
    /// twenty times under `MIN_SCALE`, which is precisely the state that
    /// constant exists to prevent.
    ///
    /// The existing floor test drives `Handle::Uniform` only, so it cannot
    /// reach this: the bug needs a per-axis squash FIRST to make the smallest
    /// and largest axes disagree.
    #[test]
    fn a_squash_then_a_uniform_shrink_never_passes_the_floor_on_any_axis() {
        let mut app = app();
        arm(&mut app);
        let origin = app.gizmo.expect("armed").origin();
        let scale = crate::gizmo::world_per_pixel(&app.camera, origin, SIZE.y);

        // Squash X as far inward as the handle allows.
        let box_at = project(&app, origin + Vec3::X * (92.0 * scale));
        let inward = project(&app, origin + Vec3::X * (0.5 * scale));
        gesture(&mut app, box_at, &[inward]);
        let squashed = app.gizmo.expect("armed").placement.scale;
        assert!(
            squashed.min_element() < 0.5,
            "the fixture did not actually squash an axis: {squashed:?}"
        );

        // Then drag the centre handle inward, which composes onto it. The
        // press has to land within `CENTRE_PX + GRAB_PX` of the middle to
        // select `Uniform` at all, so the shrink is the RATIO of two short
        // radii rather than a long sweep.
        let centre = project(&app, origin);
        gesture(&mut app, centre + Vec2::new(14.0, 0.0), &[centre + Vec2::new(1.0, 0.0)]);

        let composed = app.gizmo.expect("armed").placement.scale;
        assert!(
            composed.min_element() >= crate::gizmo::MIN_SCALE - 1.0e-6,
            "the composed scale {composed:?} fell to {} on its smallest axis, under the {} floor",
            composed.min_element(),
            crate::gizmo::MIN_SCALE,
        );
    }

    /// **A second button down during a live transform abandons it.**
    ///
    /// There was no `self.drag.is_some()` guard on `Pressed`, so a right press
    /// assigned an Orbit drag over a live `Transforming` one while
    /// `gizmo.grabbed` and the partial `placement` survived. The right release
    /// then took the Orbit arm, the Transforming arm never ran, and the left
    /// release found no drag: the handle stayed lit and the gizmo floated away
    /// from its body.
    ///
    /// The stray placement is not committed by the next `disarm_gizmo` -- that
    /// pushes the pre-arm base -- but by the NEXT drag's release, because that
    /// press latches `pinned = placement` and adopts it.
    #[test]
    fn a_second_button_during_a_transform_drag_abandons_it() {
        let mut app = app();
        arm(&mut app);
        let pinned = app.gizmo.expect("armed").pinned;

        let origin = app.gizmo.expect("armed").origin();
        let scale = crate::gizmo::world_per_pixel(&app.camera, origin, SIZE.y);
        let radius = crate::gizmo::RING_PX * scale;
        let from = project(&app, origin + Vec3::Z * radius);
        let to = project(&app, origin + (Vec3::Z * 0.9f32.cos() + Vec3::X * 0.9f32.sin()) * radius);

        moved(&mut app, from);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(from.x, from.y),
            size: SIZE,
        });
        moved(&mut app, to);
        assert!(
            app.gizmo.expect("armed").grabbed.is_some(),
            "the fixture never grabbed a handle, so nothing below is exercised"
        );

        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Right,
            position: Vector::new(to.x, to.y),
            size: SIZE,
        });

        let live = app.gizmo.expect("still armed");
        assert!(live.grabbed.is_none(), "the handle is still held after a second button went down");
        assert_eq!(live.placement, pinned, "the abandoned gesture survived in the placement");
        assert!(app.drag.is_none(), "a drag survived the cancel");
    }

    /// **Moving the camera under a held handle must not move the body.**
    ///
    /// `gizmo::drag` rebuilds both rays from the current camera, so a scroll, a
    /// SpaceMouse nudge or a navigation-cube flight still in the air
    /// re-interprets the pixel the button went down on. Measured before the
    /// fix: the X-axis offset walked 5.4696 mm to 6.5737 mm for a 0.4 radian
    /// yaw with the pointer stationary.
    ///
    /// **The axis arrow, not a ring, and shift is held.** `Ring(1)` under a
    /// pure yaw orbit is exactly immune -- yaw is a rotation about that ring's
    /// own axis -- so a ring fixture passes on a broken build, which is what
    /// the first version of this test did. And with snapping on, a drift
    /// smaller than half a snap step rounds away and hides the same bug.
    #[test]
    fn a_camera_nudge_during_a_transform_drag_does_not_move_the_body() {
        let mut app = app();
        arm(&mut app);

        let (grip, along) = x_shaft_grip(&app);
        let free = app.shift;
        app.shift = true;
        moved(&mut app, grip);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(grip.x, grip.y),
            size: SIZE,
        });
        moved(&mut app, grip + along * 40.0);
        assert!(
            app.gizmo.expect("armed").grabbed.is_some(),
            "the fixture never grabbed the axis arrow, so nothing below is exercised"
        );
        let before = app.gizmo.expect("armed").placement.translation;
        assert!(
            before.length() > 0.5,
            "the drag moved the body by {} mm, too little to see a drift against",
            before.length()
        );

        // The camera moves, and then the SAME pixel is reported again. The
        // placement only changes on a `Moved` event, so the damage does not
        // appear at the moment the camera turns -- it appears on the next
        // pointer report, when `gizmo::drag` re-interprets the press pixel
        // through the new camera. A test that only orbits and looks passes on
        // a broken build, which is what the first version of this did.
        app.camera.orbit_radians(Vec2::new(0.4, 0.0));
        app.publish_camera();
        moved(&mut app, grip + along * 40.0);
        let after = app.gizmo.expect("armed").placement.translation;

        assert!(
            (after - before).length() <= 1.0e-3,
            "the body moved {} mm because the camera did, with the pointer stationary \
             ({before:?} then {after:?})",
            (after - before).length()
        );
        app.shift = free;
    }

    /// **A press and release that moved nothing must not touch the field.**
    ///
    /// The Released arm calls `rebake_gizmo` for any `Transforming` drag with
    /// no reference to whether anything moved, and the bake always restarts
    /// from `gizmo_base` -- so an accidental click reproduced a bit-identical
    /// field at full price: 159 ms of warp and 7.1 s of repair on the largest
    /// arm-able body, plus a document-wide remesh and a stale thumbnail.
    ///
    /// `Bake::Identity` does not cover this. It fires only when the placement
    /// is the identity outright; after any earlier transform in the session the
    /// placement is a real one that simply has not changed since it was baked.
    /// So this fixture turns a ring FIRST, and only then does the empty press.
    #[test]
    fn a_press_and_release_with_no_motion_does_not_re_bake_the_body() {
        let mut app = app();
        arm(&mut app);

        // Put a real, non-identity placement in first, or the pre-existing
        // identity short circuit would carry this test and it would pass on a
        // build without the fix.
        turn_a_ring(&mut app, 0.35);
        let placement = app.gizmo.expect("armed").placement;
        assert!(
            placement.route(app.doc.voxel_size()).is_lossy(),
            "the fixture snapped to the identity, so the cheap path would cover it anyway"
        );

        let before = warps_made_on_this_thread();
        // A press and release on the same pixel, on a handle.
        let gizmo = app.gizmo.expect("armed");
        let origin = gizmo.origin();
        let scale = crate::gizmo::world_per_pixel(&app.camera, origin, SIZE.y);
        let at = project(&app, origin + Vec3::Z * (crate::gizmo::RING_PX * scale));
        gesture(&mut app, at, &[at]);

        assert_eq!(
            warps_made_on_this_thread() - before,
            0,
            "a press and release that moved nothing still re-baked the body"
        );
    }

    /// **The test the entire anti-degradation design exists for.**
    ///
    /// Placement is fiddly and thirty nudges over a session is an ordinary
    /// number. Baking each gesture onto the last result would be thirty
    /// trilinear passes -- detail the user sculpted, gone, with nothing having
    /// said so, discovered at the printer. Baking from the field the gizmo
    /// ARMED on is one pass however many gestures there were.
    #[test]
    fn thirty_adjustments_cost_exactly_one_resample_pass() {
        let mut app = app();
        arm(&mut app);
        let before = warps_made_on_this_thread();

        // Free-angle turns, so every one of them genuinely takes the lossy
        // route. Snapped ones would pass this by taking no pass at all, which
        // is a different and weaker claim.
        for step in 0..30 {
            turn_a_ring(&mut app, 0.2 + step as f32 * 0.03);
        }
        let after_drags = warps_made_on_this_thread() - before;
        assert!(after_drags > 0, "the fixture never dragged a ring at all");
        assert_eq!(
            after_drags, 30,
            "each gesture should bake once while the gizmo is up, and did {after_drags} times"
        );

        // And the claim that matters: those thirty bakes all came off the SAME
        // field, so the body has been through one pass, not thirty. Undoing the
        // session puts back a field that was never resampled at all.
        let placement = app.gizmo.expect("armed").placement;
        assert!(placement.route(app.doc.voxel_size()).is_lossy(), "the fixture snapped");
        update(&mut app, Message::ToolChanged(Tool::Sculpt));
        assert!(app.gizmo.is_none(), "the gizmo did not disarm");
        assert!(app.history.can_undo(), "the session pushed no entry");
    }

    /// Baking from the base makes this exact rather than approximate: the
    /// placement really is the identity again, so the routing declines to run
    /// anything at all.
    /// Dragging away and back WITHIN one gesture is exactly the identity, not
    /// approximately it, because the placement is measured from the press pixel
    /// rather than accumulated from the last motion event.
    ///
    /// **Within one gesture and not across two, and the difference is honest.**
    /// Two opposite gestures need not cancel to the last voxel: the second one
    /// is measured about a moved origin through a perspective projection, so
    /// its pixels are not worth quite the same number of millimetres as the
    /// first one's, and each is snapped separately. The exactness that was
    /// promised is the one this checks.
    #[test]
    fn dragging_out_and_back_within_a_gesture_bakes_nothing() {
        let mut app = app();
        arm(&mut app);
        let before = field(&app);
        let warps = warps_made_on_this_thread();

        let (grip, along) = x_shaft_grip(&app);
        app.shift = true;
        gesture(&mut app, grip, &[grip + along * 90.0, grip + along * 210.0, grip]);
        app.shift = false;

        assert_eq!(
            app.gizmo.expect("armed").placement.route(app.doc.voxel_size()),
            Bake::Identity,
            "coming back to the press pixel left a placement behind"
        );
        assert_eq!(field(&app), before, "the field moved after a drag out and back");
        assert_eq!(warps_made_on_this_thread(), warps, "a round trip ran a trilinear pass");
    }

    /// The ordinary gesture is snapped, and a snapped move is a value-exact
    /// gather. If this ever routes to a resample the default has silently
    /// become the lossy one.
    #[test]
    fn a_snapped_move_through_the_application_never_resamples() {
        let mut app = app();
        arm(&mut app);
        assert!(!app.shift, "the fixture must start unsnapped-free, i.e. snapping ON");
        let warps = warps_made_on_this_thread();

        for pixels in [30.0_f32, -55.0, 120.0] {
            drag_the_x_arrow(&mut app, pixels);
            let route = app.gizmo.expect("armed").placement.route(app.doc.voxel_size());
            assert!(!route.is_lossy(), "a snapped move of {pixels} px routed to {route:?}");
        }
        assert_eq!(
            warps_made_on_this_thread(),
            warps,
            "a snapped move ran a trilinear pass it did not need"
        );
    }

    #[test]
    fn a_snapped_move_actually_moves_the_body() {
        let mut app = app();
        arm(&mut app);
        let before = app.doc.active_volume().world_bounds().expect("a body");
        drag_the_x_arrow(&mut app, 120.0);
        let after = app.doc.active_volume().world_bounds().expect("a body");
        assert!(
            (after.0.x - before.0.x).abs() > app.doc.voxel_size(),
            "the body did not move: {before:?} then {after:?}"
        );
        // Moved, not grown: a translation is rigid.
        let span = |b: (Vec3, Vec3)| b.1 - b.0;
        assert!(span(after).distance(span(before)) < app.doc.voxel_size() * 2.0);
    }

    // ------------------------------------------------------------ the guards

    /// **`orient` clears the whole history and this must not.** A quarter turn
    /// leaves every older entry naming bricks of a volume that no longer
    /// exists, so its caller drops the stack; the gizmo puts the original field
    /// ON the stack instead, so undoing the transform makes those coordinates
    /// valid again.
    #[test]
    fn a_transform_gesture_does_not_clear_the_undo_history() {
        let mut app = app();
        // A stroke to have some history to lose.
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(SIZE.x / 2.0, SIZE.y / 2.0),
            size: SIZE,
        });
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        assert!(app.history.can_undo(), "the fixture laid down no stroke");
        let entries = app.history_stats.undo_entries;

        arm(&mut app);
        drag_the_x_arrow(&mut app, 90.0);
        update(&mut app, Message::ToolChanged(Tool::Sculpt));

        assert!(
            app.history_stats.undo_entries > entries,
            "the transform pushed nothing: {} then {}",
            entries,
            app.history_stats.undo_entries
        );
        // Undo the transform, then the stroke behind it.
        update(&mut app, Message::Undo);
        update(&mut app, Message::Undo);
        assert!(!app.history.can_undo(), "the stroke behind the transform was not reachable");
    }

    /// Escape ABANDONS, where leaving the tool commits. Bit-identical, not
    /// close: the whole reason the base is held is that it is the only copy of
    /// a field the forward direction cannot reproduce.
    #[test]
    fn escape_leaves_the_body_bit_identical() {
        let mut app = app();
        arm(&mut app);
        let before = field(&app);

        turn_a_ring(&mut app, 0.6);
        assert_ne!(field(&app), before, "the fixture's transform changed nothing");
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::MenuClosed);

        assert!(app.gizmo.is_none(), "escape left the gizmo armed");
        assert_eq!(app.tool, Tool::Sculpt);
        assert_eq!(field(&app), before, "escape did not put the field back bit for bit");
        assert_eq!(app.history_stats.undo_entries, entries, "an abandoned session pushed an entry");
    }

    /// Escape during a LIVE drag cancels that drag and nothing more. "Not that
    /// one" is a different instruction from "not any of it", and the recovery
    /// is throwing the gesture away rather than inverting it.
    /// While a handle is held the body is drawn where the drag has it, and
    /// the release, which rebuilds the field there, stops drawing it moved;
    /// so does an escape, which puts it back. See `app/moving.rs`.
    #[test]
    fn a_dragged_body_is_drawn_moved_until_the_release() {
        let mut app = app();
        arm(&mut app);
        assert!(app.shared.moving_snapshot().is_none(), "drawn moved before any drag");
        let (grip, along) = x_shaft_grip(&app);
        moved(&mut app, grip);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(grip.x, grip.y),
            size: SIZE,
        });
        moved(&mut app, grip + along * 90.0);
        let gizmo = app.gizmo.expect("armed");
        let (body, map) = app.shared.moving_snapshot().expect("held, the body is drawn moved");
        assert_eq!(body, gizmo.body);
        let from = gizmo.pinned.transform_point(gizmo.base_pivot);
        let to = gizmo.placement.transform_point(gizmo.base_pivot);
        assert!(to.distance(from) > 0.5, "the fixture's drag went nowhere");
        assert!(
            map.transform_point3(from).distance(to) < 1.0e-3,
            "not drawn where the drag has it"
        );

        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        assert!(app.shared.moving_snapshot().is_none(), "still drawn moved after the release");

        // Escaped mid-drag: back where it was, and drawn there.
        let (grip, along) = x_shaft_grip(&app);
        moved(&mut app, grip);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(grip.x, grip.y),
            size: SIZE,
        });
        moved(&mut app, grip + along * 60.0);
        assert!(app.shared.moving_snapshot().is_some());
        update(&mut app, Message::MenuClosed);
        assert!(app.shared.moving_snapshot().is_none(), "still drawn moved after the escape");
    }

    #[test]
    fn escape_during_a_drag_cancels_the_drag_and_keeps_the_gizmo() {
        let mut app = app();
        arm(&mut app);
        drag_the_x_arrow(&mut app, 90.0);
        let committed = app.gizmo.expect("armed").placement;
        assert_ne!(committed.route(app.doc.voxel_size()), Bake::Identity, "the fixture is idle");

        // A second drag, escaped before its release.
        let (grip, along) = x_shaft_grip(&app);
        moved(&mut app, grip);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(grip.x, grip.y),
            size: SIZE,
        });
        assert!(matches!(app.drag.map(|drag| drag.kind), Some(DragKind::Transforming(_))));
        moved(&mut app, grip + along * 200.0);
        update(&mut app, Message::MenuClosed);

        assert!(app.gizmo.is_some(), "escape mid-drag disarmed the whole gizmo");
        assert!(app.drag.is_none(), "the drag survived the escape");
        assert_eq!(
            app.gizmo.expect("armed").placement.translation,
            committed.translation,
            "the cancelled drag was kept"
        );
    }

    /// ONLY BOUNDS-CHECKED EVENTS MAY CAPTURE. The gizmo claims its own disc
    /// and nothing else; a press well away from it has to reach the body under
    /// the cursor, which under this tool means choosing it.
    #[test]
    fn a_press_away_from_the_gizmo_is_not_taken_as_a_handle() {
        let mut app = app();
        arm(&mut app);
        let middle = gizmo_centre(&app);
        for offset in [Vec2::new(340.0, 0.0), Vec2::new(-340.0, 0.0), Vec2::new(0.0, 260.0)] {
            let at = (middle + offset).clamp(Vec2::splat(2.0), Vec2::new(SIZE.x, SIZE.y) - 2.0);
            app.on_pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: Vector::new(at.x, at.y),
                size: SIZE,
            });
            assert!(
                !matches!(app.drag.map(|drag| drag.kind), Some(DragKind::Transforming(_))),
                "a press at {at:?} was taken as a gizmo handle"
            );
            app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        }
    }

    /// A held resize owns the pointer whether or not a button is down, and the
    /// press dispatch already settled that precedence between sizing and the
    /// plane cut. The gizmo is the third claimant and does not get to jump it:
    /// a resize whose press turned into a transform is the worse surprise.
    #[test]
    fn a_held_resize_outranks_the_gizmo() {
        let mut app = app();
        arm(&mut app);
        let (grip, _) = x_shaft_grip(&app);
        // A resize anchors on the last cursor position, so the pointer has to
        // have been seen before the key is pressed.
        moved(&mut app, grip);
        update(&mut app, Message::SizingStarted(SizingTarget::Radius));
        assert!(app.sizing.is_some(), "the fixture did not start a resize");
        // `SizingStarted` is not on `keeps_the_gizmo_armed`, so it committed
        // the session and the pass after `dispatch` re-armed on the same body.
        // That is the state the precedence below is actually about.
        assert!(app.gizmo.is_some(), "the gizmo did not come back after the commit");

        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(grip.x, grip.y),
            size: SIZE,
        });
        assert_eq!(
            app.drag.map(|drag| drag.kind),
            Some(DragKind::Sizing),
            "the gizmo took a press that belonged to a held resize"
        );
    }

    /// **The draw order and the click order have to agree.** `on_pointer` tests
    /// the navigation cube before the gizmo may claim anything, so the renderer
    /// draws the gizmo first and the cube over it. Get that backwards and the
    /// user sees an arrow, clicks it, and the camera flies.
    #[test]
    fn the_navigation_cube_outranks_the_gizmo_where_they_overlap() {
        let mut app = app();
        arm(&mut app);
        let (corner, size) = crate::navcube::corner_rect(Vec2::new(SIZE.x, SIZE.y));
        let middle = corner + size * 0.5;

        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(middle.x, middle.y),
            size: SIZE,
        });
        assert!(
            !matches!(app.drag.map(|drag| drag.kind), Some(DragKind::Transforming(_))),
            "the gizmo claimed a press inside the navigation cube"
        );
        assert!(app.flight.is_some(), "the cube did not get the press either");
    }

    /// A brush ring under the gizmo would say "a press here carves this much",
    /// which is not what a press there does.
    #[test]
    fn the_brush_ring_goes_away_while_the_gizmo_is_up() {
        let mut app = app();
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x / 2.0, SIZE.y / 2.0),
            size: SIZE,
        });
        assert!(app.hover.is_some(), "the fixture's cursor is not over the model");

        arm(&mut app);
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x / 2.0, SIZE.y / 2.0),
            size: SIZE,
        });
        assert!(app.hover.is_none(), "the ring is still being placed on the surface");
    }

    /// Pressing `w` twice is a toggle and has to be free: arming alone must not
    /// touch the field, so the base is only taken on the first bake.
    #[test]
    fn arming_and_disarming_without_dragging_costs_nothing() {
        let mut app = app();
        let before = field(&app);
        let entries = app.history_stats.undo_entries;

        arm(&mut app);
        assert!(app.gizmo_base.is_none(), "arming took the field out before it had to");
        update(&mut app, Message::ToolChanged(Tool::Transform));

        assert!(app.gizmo.is_none(), "the toggle did not put it away");
        assert_eq!(app.tool, Tool::Sculpt);
        assert_eq!(field(&app), before);
        assert_eq!(app.history_stats.undo_entries, entries, "an untouched session pushed an entry");
        assert!(!app.unsaved, "an untouched session marked the document dirty");
    }

    /// The status line names the route on EVERY bake. A user told nothing when
    /// a move is exact has no way to learn that some of them are not.
    #[test]
    fn the_status_line_says_which_route_the_bake_took() {
        let mut app = app();
        arm(&mut app);
        drag_the_x_arrow(&mut app, 90.0);
        assert!(
            app.status.contains("exact"),
            "a snapped move said {:?} rather than naming the route",
            app.status
        );

        turn_a_ring(&mut app, 0.8);
        assert!(
            app.status.contains("resampled"),
            "a free turn said {:?} rather than admitting the resample",
            app.status
        );
    }

    /// **The guard that stops a document changing ahead of its own history.**
    /// While the gizmo is armed the document holds the baked field and the
    /// stack does not yet hold the entry that undoes it, so anything outside
    /// the gesture has to close the session first. Undo is the sharpest case.
    #[test]
    fn undoing_while_the_gizmo_is_armed_commits_it_first_and_then_undoes_it() {
        let mut app = app();
        let before = field(&app);
        arm(&mut app);
        drag_the_x_arrow(&mut app, 120.0);
        assert_ne!(field(&app), before, "the fixture did not move the body");

        update(&mut app, Message::Undo);

        assert_eq!(field(&app), before, "undo did not reach the transform");
    }

    /// **Asserting on the document is not enough, and this is the assertion
    /// that says so.**
    ///
    /// The test above passed while the bug was live: it reads `field(&app)`,
    /// which is the document, and the document was always right. What was
    /// wrong was that nothing told the renderer -- a field arriving from the
    /// history carries an EMPTY dirty set, because whatever remeshed it last
    /// drained it -- so the screen went on drawing the body where the bake had
    /// put it, and every stroke, pick and brush ring after that acted on a
    /// position the user could not see. `perf.dirty_bricks` is the one number
    /// that can tell the two apart.
    #[test]
    fn undoing_a_bake_schedules_the_remesh_that_puts_the_body_back_on_screen() {
        let mut app = app();
        arm(&mut app);
        drag_the_x_arrow(&mut app, 120.0);
        // Out of the tool, so the entry is already on the stack and the undo
        // below is a plain undo rather than a commit followed by one.
        update(&mut app, Message::ToolChanged(Tool::Sculpt));

        app.perf.dirty_bricks = 0;
        update(&mut app, Message::Undo);
        assert!(
            app.perf.dirty_bricks > 0,
            "undoing a bake remeshed nothing, so the screen still shows the body where the bake \
             put it"
        );

        app.perf.dirty_bricks = 0;
        update(&mut app, Message::Redo);
        assert!(app.perf.dirty_bricks > 0, "redoing a bake remeshed nothing");
    }

    /// **A gesture that comes back to where it started must cost what arming
    /// and not dragging costs: nothing.**
    ///
    /// Nudging a body and nudging it back is an ordinary way to change your
    /// mind, and the identity route used to run `base.shifted(IVec3::ZERO)` --
    /// a full deep copy, quoted at 1.53 GiB on the dragon -- to produce a field
    /// identical to the one already held, then a whole-document remesh, and
    /// then a whole-field undo entry charged against the reclaim allowance,
    /// where `History::trim` can evict real strokes to make room for it.
    #[test]
    fn a_gesture_that_returns_to_the_identity_puts_the_base_back_rather_than_copying_it() {
        let mut app = app();
        let before = field(&app);
        arm(&mut app);
        let copies = hani_core::volume::copies_made_on_this_thread();
        let warps = warps_made_on_this_thread();
        let undoable = app.history.can_undo();

        drag_the_x_arrow(&mut app, 40.0);
        assert_ne!(field(&app), before, "the fixture did not move the body");
        drag_the_x_arrow(&mut app, -40.0);

        assert_eq!(
            app.gizmo.expect("armed").placement,
            hani_core::Similarity::IDENTITY,
            "the fixture did not bring the placement back to the identity"
        );
        assert_eq!(field(&app), before, "the body did not come back bit for bit");
        assert_eq!(
            hani_core::volume::copies_made_on_this_thread(),
            copies,
            "returning to the identity deep copied the field instead of putting the base back"
        );
        assert_eq!(warps_made_on_this_thread(), warps, "an exact gesture resampled");

        update(&mut app, Message::ToolChanged(Tool::Sculpt));
        assert_eq!(
            app.history.can_undo(),
            undoable,
            "a session that changed nothing was charged to the undo history"
        );
    }

    /// **The ceiling is folded into the gesture, so a scale the bake could not
    /// build is never offered.**
    ///
    /// `Handle::Uniform` is selected by a press within sixteen pixels of the
    /// centre, so one long drag saturates `gizmo::MAX_SCALE`, and
    /// `Similarity::then` multiplies scales -- while `Volume::warped` builds a
    /// dense destination brick grid with no cap of its own and `arm_gizmo`'s
    /// refusals were evaluated once, against the UNSCALED body.
    ///
    /// The document is stuffed rather than a huge body being built, for the
    /// reason `a_body_the_pool_cannot_hold_is_refused` gives: the ceiling is
    /// read off `doc_stats`, so a fixture that writes `doc_stats` exercises the
    /// same arithmetic in a fraction of the time.
    #[test]
    fn a_scale_the_bake_could_not_afford_is_never_offered_by_the_gesture() {
        let mut app = app();
        let resident = app.doc.active_volume().stats().resident_bytes as f64;
        // Room for about four times the body, so a ceiling near two -- small
        // enough that the bakes this test really does run stay cheap.
        app.doc_stats.resident_bytes = (MAX_VOLUME_BYTES - resident * 4.0) as usize;

        arm(&mut app);
        let ceiling = app.gizmo.expect("armed").max_scale;
        assert!(
            (1.0..gizmo::MAX_SCALE).contains(&ceiling),
            "a document with room for four copies gave a ceiling of {ceiling}"
        );

        let centre = gizmo_centre(&app);
        // Two, because one gesture cannot compose with anything and it is the
        // composition that used to square the ceiling.
        for round in 0..2 {
            let from = centre + Vec2::new(4.0, 0.0);
            gesture(&mut app, from, &[from + Vec2::new(320.0, 0.0)]);
            let scale = app.gizmo.expect("armed").placement.scale;
            assert!(
                scale.max_element() <= ceiling + 1.0e-3,
                "round {round} composed to {scale}, past a ceiling of {ceiling}"
            );
        }
        assert!(
            app.gizmo.expect("armed").placement.scale.max_element() > 1.0,
            "the fixture never grew the body, so it proves nothing"
        );

        // And it has to SAY so. A preview box that stops growing under the
        // pointer with nothing in the status line reads as the gizmo having
        // lost the drag. Read mid-gesture, before the release replaces the line
        // with the bake summary.
        let from = centre + Vec2::new(4.0, 0.0);
        moved(&mut app, from);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(from.x, from.y),
            size: SIZE,
        });
        moved(&mut app, from + Vec2::new(320.0, 0.0));
        assert!(app.status.contains("memory"), "the clamp bit without saying why: {}", app.status);
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
    }

    /// The ceiling has to shrink as the document fills, and it has to admit the
    /// identity even when the document is full: a body can always be placed at
    /// the size it already is, and a ceiling below one would make the gizmo
    /// refuse to leave it alone.
    #[test]
    fn the_scale_ceiling_falls_as_the_document_fills_and_never_below_one() {
        let stats = app().doc.active_volume().stats();
        let resident = stats.resident_bytes as f64;

        let mut roomy = app();
        roomy.doc_stats.resident_bytes = (resident * 4.0) as usize;
        let mut tight = app();
        tight.doc_stats.resident_bytes = (MAX_VOLUME_BYTES - resident * 4.0) as usize;
        let mut full = app();
        full.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize;

        let roomy = roomy.largest_scale_that_fits(stats);
        let tight = tight.largest_scale_that_fits(stats);
        let full = full.largest_scale_that_fits(stats);

        assert!(roomy > tight, "an empty document offered no more room than a nearly full one");
        assert!(tight > 1.0, "a document with room for four copies refused to scale at all");
        assert_eq!(full, 1.0, "a full document must still let the body stay where it is");
    }

    /// The gizmo follows the active body rather than being left pointing at a
    /// row nobody selected any more.
    #[test]
    fn choosing_another_body_moves_the_gizmo_to_it_and_commits_the_last_one() {
        let mut app = app();
        let first = app.doc.active();
        let mut second_volume = hani_core::Volume::new(app.doc.voxel_size());
        second_volume.seed_sphere(Vec3::new(90.0, 0.0, 0.0), 12.0);
        let second = app.doc.add_body("Second", second_volume);
        app.doc.mark_everything_dirty();

        arm(&mut app);
        assert_eq!(app.gizmo.expect("armed").body, first);
        drag_the_x_arrow(&mut app, 120.0);

        update(&mut app, Message::BodySelected(second));

        assert_eq!(app.tool, Tool::Transform, "selecting a row dropped the tool");
        assert_eq!(app.gizmo.expect("armed").body, second, "the gizmo stayed on the old body");
        assert!(app.history.can_undo(), "the first body's session was not committed");
    }

    /// Arming is what commits to holding a second copy of the body, so that is
    /// where the refusal has to be -- before the allocation and never after it,
    /// which is the rule `merge_plan` already follows.
    #[test]
    fn arming_a_gizmo_on_a_hidden_body_is_refused_before_anything_is_allocated() {
        let mut app = app();
        let body = app.doc.active();
        let copies = hani_core::volume::copies_made_on_this_thread();

        let mut meta = app.doc.meta(body).expect("a body");
        meta.visible = false;
        app.doc.set_meta(&meta);
        app.publish_visibility();

        update(&mut app, Message::ToolChanged(Tool::Transform));
        assert_ne!(app.tool, Tool::Transform, "the gizmo armed on a hidden body");
        assert!(app.gizmo.is_none());
        assert!(app.status.contains("hidden"), "the refusal did not say why: {:?}", app.status);
        assert_eq!(
            hani_core::volume::copies_made_on_this_thread(),
            copies,
            "the refusal allocated a copy on its way to refusing"
        );
    }

    /// The bake replaces the whole field, so the bricks the OLD one occupied
    /// have to be marked in the NEW one or the renderer keeps drawing triangles
    /// nothing will ever overwrite.
    #[test]
    fn a_bake_marks_the_bricks_the_body_has_left() {
        let mut app = app();
        arm(&mut app);
        let vacated: Vec<BrickCoord> = app.doc.active_volume().brick_coords().collect();
        assert!(!vacated.is_empty(), "the fixture is empty");

        // Far enough that the body lands on entirely different bricks.
        drag_the_x_arrow(&mut app, 260.0);

        // `remesh_dirty` has already drained the set, so the check is that the
        // renderer was told about the old body at all.
        let placement = app.gizmo.expect("armed").placement;
        assert!(
            placement.translation.length() > hani_core::BRICK_DIM as f32 * app.doc.voxel_size(),
            "the fixture did not move the body off its old bricks"
        );
        let now: Vec<BrickCoord> = app.doc.active_volume().brick_coords().collect();
        assert!(
            now.iter().any(|coord| !vacated.contains(coord)),
            "the body is still on exactly the bricks it started on"
        );
    }

    /// The placement is the caller's business and the routing is the core's,
    /// and the two have to agree about what a gesture produced -- the status
    /// line, the bake and the undo entry all read the same answer.
    #[test]
    fn the_route_the_status_reports_is_the_route_the_bake_took() {
        let mut app = app();
        arm(&mut app);
        let warps = warps_made_on_this_thread();
        drag_the_x_arrow(&mut app, 90.0);
        let exact = warps_made_on_this_thread() == warps;
        assert_eq!(
            exact,
            !app.gizmo.expect("armed").placement.route(app.doc.voxel_size()).is_lossy(),
            "the bake and the routing disagreed about whether a pass ran"
        );
    }

    /// Read from the SHARED frame and not from `app.gizmo_batch`: the two
    /// buffers rotate on every refresh, so the application's own field holds
    /// last frame's recycled capacity and the renderer's holds this frame's
    /// geometry.
    #[test]
    fn a_gizmo_is_drawn_only_while_it_is_armed() {
        let mut app = app();
        assert!(app.shared.gizmo_snapshot().is_empty(), "a gizmo was drawn before one existed");
        arm(&mut app);
        assert!(!app.shared.gizmo_snapshot().is_empty(), "the armed gizmo drew nothing");
        update(&mut app, Message::ToolChanged(Tool::Sculpt));
        assert!(
            app.shared.gizmo_snapshot().is_empty(),
            "the gizmo is still on screen after disarming"
        );
    }

    /// The composition the drag path actually performs, checked against
    /// applying the two placements in turn.
    #[test]
    fn two_gestures_compose_into_one_placement_rather_than_replacing_it() {
        let mut app = app();
        arm(&mut app);
        drag_the_x_arrow(&mut app, 60.0);
        let first = app.gizmo.expect("armed").placement;
        drag_the_x_arrow(&mut app, 60.0);
        let both = app.gizmo.expect("armed").placement;

        assert!(
            both.translation.length() > first.translation.length() * 1.5,
            "the second gesture replaced the first rather than composing: \
             {first:?} then {both:?}"
        );
        // And the placement is still exactly a translation, so both stayed on
        // the exact route.
        assert!(!both.route(app.doc.voxel_size()).is_lossy());
    }
}
