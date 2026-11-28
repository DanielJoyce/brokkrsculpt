// SPDX-License-Identifier: AGPL-3.0-only

//! From the field to the renderer: meshing the bricks that changed, the
//! whole-document rebuild, and what is measured of the document after.
//! Moved out of `app.rs` (issue 189).

use super::*;

impl Hani {
    /// Mesh every brick the volume has marked dirty and hand the results to the
    /// renderer. Never touches a brick that was not marked.
    /// Remesh after the WHOLE model has been replaced.
    ///
    /// Empties the mesh pool first, because the pool's allocator never splits
    /// or merges blocks: a rebuild changes every brick's mesh size at once, so
    /// the free lists fill with granule classes nothing asks for again while
    /// the bump pointer climbs. Two or three trips up and down the detail
    /// buttons exhausted an 11M vertex pool with roughly 7.4M live -- observed
    /// on the dragon, 2755 bricks missing from the screen, on 2026-08-22.
    ///
    /// The reset is only sound paired with marking everything dirty, so the
    /// two live in one function and neither is callable by accident.
    ///
    /// # This is also why no swap site marks the OUTGOING model's bricks
    ///
    /// `reset_sculpt`, `open_project`, `adopt_import` and `orient` each used to
    /// collect the departing volume's brick coordinates and mark them dirty in
    /// the incoming one, so that they would mesh to nothing and release their
    /// pool slices. All four of those loops were dead, and had been since
    /// before the pool learned about bodies: all four call this function, and
    /// `MeshPool::reset` clears its slot map wholesale regardless of what any
    /// key says. The loops cost a walk of the outgoing model plus a remesh of
    /// however many coordinates it had, to release slices that had already
    /// been released.
    ///
    /// The reason to say so here rather than to leave four comments behind is
    /// that the trick reads as necessary, and it is the thing a fifth swap site
    /// would copy. **The rule is: call this, and the outgoing model is gone.**
    /// Anything that removes ONE body while the others stay is a different
    /// problem and has a different answer -- `SharedFrame::forget_body`.
    pub(super) fn rebuild_everything(&mut self) {
        self.shared.request_pool_reset();
        self.doc.mark_everything_dirty();
        self.remesh_dirty();
        // Solo cannot survive a whole-document swap, and the reason it is
        // cleared here rather than left to `forget_a_vanished_solo` is that the
        // incoming document numbers its rows from 1 again: a stale id does not
        // dangle, it names a real and perfectly innocent body in the new
        // document and hides everything else. Three of the four callers replace
        // the whole document, and the fourth -- `orient` -- turns the model
        // without changing which rows exist, so clearing there costs the user a
        // mode they can press one button to get back.
        self.solo = None;
        // Same shape of problem for the eye: an id the outgoing document had
        // hidden could name a perfectly visible body in the incoming one.
        // `update` would put that right on the next message, and the next
        // message is at worst one frame away, but "at worst one frame" is how
        // long a body would be missing from the screen with nothing in the
        // panel saying why.
        self.publish_visibility();
        // A whole-document swap can change both the lattice and the extent,
        // which are the two numbers the camera measures itself against.
        self.refresh_model_radius();
    }

    /// Hand every brick back to the mesh pool and mesh them again.
    ///
    /// **The remedy for a FRAGMENTED pool, and it is deliberately something
    /// the user presses rather than something a cut decides.** The allocator
    /// never splits or merges blocks, and a cut only ever shrinks the meshes it
    /// touches, so each one orphans a block and takes a fresh one off the bump
    /// pointer: thirty trims measured 1.45x watermark over live, with `live`
    /// FALLING 11% while the bump pointer ROSE 16%. The space is there; it is
    /// stranded, and only re-offering every brick from an empty allocator
    /// recovers it.
    ///
    /// # Why a cut does not do this by itself
    ///
    /// It was the open question and the answer is no. This is a synchronous,
    /// unbudgeted remesh of the whole document inside `update` -- the same
    /// shape of work `MOST_BRICKS_TO_WALK` refuses to spend 320 ms on merely to
    /// put five more words in a status line. Spending seconds of it, uninvited,
    /// in the middle of a sculpting gesture, is a worse surprise than the
    /// banner it would silence. A user who is told what is wrong and given the
    /// button can choose the moment; a threshold cannot.
    ///
    /// # What it does NOT do
    ///
    /// **It does not give memory back to the system.** `MeshPool::reset` empties
    /// the allocators over the buffers it already holds and never drops a
    /// `BufferPair`, so a pool that has grown to N pairs keeps them for the life
    /// of the process. What this recovers is allocatable space inside those
    /// buffers, which is what the overflow is actually about. Any wording that
    /// reads as freeing VRAM would be a lie.
    ///
    /// Solo is NOT cleared, and that is the one thing separating this from
    /// [`Hani::rebuild_everything`]. That function clears it because its four
    /// callers replace the whole document and renumber its rows, so a kept solo
    /// id would name a different body. Nothing here changes which rows exist.
    ///
    /// It is not free beyond the remesh: `remesh_dirty` marks every dirtied
    /// body's thumbnail stale, and `mark_everything_dirty` dirties all of them,
    /// so the pictures redraw at one body per idle frame afterwards.
    pub(super) fn rebuild_view(&mut self) {
        self.shared.request_pool_reset();
        self.doc.mark_everything_dirty();
        self.remesh_dirty();
        self.status = "rebuilt the view — the pool is packed again".to_string();
    }

    pub(super) fn remesh_dirty(&mut self) {
        // One of several moves handled together: the bricks stay marked, and
        // the last move's remesh takes them all. See `pointer_in`.
        if self.batching {
            return;
        }
        self.doc.take_dirty(&mut self.dirty);
        self.perf.dirty_bricks = self.dirty.len();
        if self.dirty.is_empty() {
            return;
        }
        // Bricks changed, by whatever road: a crash net part written is of a
        // document that no longer is. See `app/autosave.rs`.
        self.give_up_autosave();

        // **Every picture that is now out of date, marked in the one place that
        // already knows which bricks changed.** Undo, redo, delete, merge, add,
        // import, open, re-orient and resample all reach the GPU through here,
        // so hooking this covers the lot -- where a list of the sites that
        // change geometry is a list that goes out of date silently, which is
        // the argument `publish_visibility` already makes about the eye.
        //
        // **Except during a stroke**, which is the load-bearing exception: a
        // stroke calls this once per pointer event, and the picture is wanted
        // once, at the end. `finish_stroke` is the other half, and it marks the
        // body only when the stroke actually produced an edit -- so a press and
        // release that never touched the model costs no render at all.
        //
        // A MASK drag is a stroke by the same argument and is excepted with it:
        // it calls this per event too, and once the mask is a vertex attribute
        // every one of those events dirties the footprint for real.
        if !matches!(
            self.drag.map(|drag| drag.kind),
            Some(DragKind::Sculpt(_)) | Some(DragKind::Masking)
        ) {
            // One `Instant::now()` and one hash insert per BODY, never per
            // brick: a whole-document rebuild hands this tens of thousands of
            // coordinates and they arrive grouped by body. Destructured so the
            // dirty list and the cells can be borrowed at once.
            let Self { dirty, thumbs, .. } = self;
            let mut last = None;
            for (body, _) in dirty.iter() {
                if last == Some(*body) {
                    continue;
                }
                last = Some(*body);
                thumbs.geometry_changed(*body);
            }
        }

        let started = Instant::now();
        let timed = self.phases.enter(phases::Phase::Remesh);
        let count = self.dirty.len();
        self.phases.count(|frame| frame.meshed += count as u32);
        while self.mesh_buffers.len() < count {
            self.mesh_buffers.push(self.shared.take_mesh());
        }
        // Across every core, and across every BODY in one call: the threshold
        // that chooses between the serial and the parallel path counts the
        // coordinates in one call, so meshing body by body would drop a
        // document's worth of scattered dirty bricks onto a single thread. At
        // the sizes M2 targets the parallel path is the difference between a
        // remesh at 70 percent of its budget and one at under 10.
        if self.show_cavity || !self.doc.paints().is_empty() {
            self.doc.mesh_dirty_with_cavity(&self.dirty, &mut self.mesh_buffers[..count]);
        } else {
            self.doc.mesh_dirty(&self.dirty, &mut self.mesh_buffers[..count]);
        }
        for ((body, coord), mesh) in self.dirty.iter().zip(self.mesh_buffers.drain(..count)) {
            self.shared.publish(*body, *coord, mesh);
        }
        self.perf.remesh_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.phases.leave(timed);

        // **What is measured of the whole document waits for the end of a
        // stroke.** The totals and the radius below each walk every brick of
        // the document, and this is reached on every step of a stroke: on
        // 4610 dense bricks that was 0.17 ms a frame for two numbers nothing
        // reads until the stroke is over (`docs/PERFORMANCE.md`). The readout
        // behind the Info toggle shows a stroke's bricks when the stroke ends.
        // `finish_stroke` is the other half.
        if matches!(
            self.drag.map(|drag| drag.kind),
            Some(DragKind::Sculpt(_)) | Some(DragKind::Masking)
        ) {
            self.measures_stale = true;
        } else {
            self.measure_document();
        }
    }

    /// Take the document's totals and the active body's radius again, after
    /// its bricks changed.
    pub(super) fn measure_document(&mut self) {
        let timed = self.phases.enter(phases::Phase::After);
        self.measures_stale = false;
        self.doc_stats = self.doc.totals();

        // Dynamic resizes the brush when the MODEL changed under it. Comparing
        // against a radius measured on a DIFFERENT body would resize it because
        // the selection changed, which is a different thing entirely and must
        // never happen -- see `rescale_radius`. So the comparison is made only
        // when the previous measurement was of the same body.
        let same_body = self.model_radius_body == self.doc.active();
        let previous = self.model_radius;
        self.refresh_model_radius();
        if same_body {
            self.rescale_radius(previous);
        }
        self.phases.leave(timed);
    }

    /// Re-measure the active body, for the camera, the mirror plane and the
    /// Dynamic brush.
    ///
    /// **Separate from [`Hani::rescale_radius`] on purpose.** This one is
    /// safe to call after a selection change; that one is not.
    pub(super) fn refresh_model_radius(&mut self) {
        self.model_radius = self.doc.active_volume().content_radius().unwrap_or(MODEL_RADIUS_MM);
        self.model_radius_body = self.doc.active();
        // The camera measures its floor in voxels and its far plane in content
        // radii, so it goes stale for exactly the reasons this does.
        self.refresh_camera_lattice();
    }
}

/// What the four whole-document swap sites owe the renderer.
///
/// Increment 6 deleted eleven lines of stale-coordinate marking from
/// `reset_sculpt`, `open_project`, `adopt_import` and `orient` -- loops that
/// collected the outgoing model's brick coordinates and marked them dirty in
/// the incoming volume so they would mesh to nothing and release their pool
/// slices. All four were already dead, and the reason is entirely in this
/// module: all four call `rebuild_everything`, and that asks the renderer to
/// empty the pool, which drops every slot regardless of what key it was under.
///
/// **The reasoning is only as durable as the reset**, and nothing asserted the
/// reset before this. If `rebuild_everything` ever stops asking for one, all
/// four of these functions start leaving the previous model drawn underneath
/// the new one -- and with the loops gone there is nothing else that would
/// have cleared it.
#[cfg(test)]
mod swap_site_tests {
    use super::*;

    /// One whole-document swap, so the four can be driven by one loop and none
    /// of them can be left out of it by accident.
    type Swap = Box<dyn Fn(&mut Hani)>;

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// Every whole-document swap empties the pool, and the swap itself is
    /// enough -- no caller has to remember anything extra.
    #[test]
    fn every_whole_document_swap_asks_the_renderer_to_empty_the_pool() {
        let directory = std::env::temp_dir().join(format!("hani-swaps-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("saved.hani");

        let mut app = app();
        app.save_project(&path);

        // Each case: clear whatever is pending by standing in for the frame
        // that would consume it, do the swap, and assert the request is back.
        let cases: [(&str, Swap); 4] = [
            ("reset", Box::new(|app: &mut Hani| app.reset_sculpt())),
            ("open", Box::new(move |app: &mut Hani| app.open_project(&path))),
            (
                "import",
                Box::new(|app: &mut Hani| {
                    let mut volume = Volume::new(app.doc.voxel_size());
                    volume.seed_sphere(Vec3::ZERO, 12.0);
                    volume.mark_everything_dirty();
                    app.adopt_import(crate::message::Imported {
                        volume,
                        source: std::path::PathBuf::from("fixture.stl"),
                        report: hani_core::voxelise::VoxeliseReport::default(),
                        elapsed_ms: 0.0,
                        resting_up: None,
                    });
                }),
            ),
            (
                "orient",
                Box::new(|app: &mut Hani| {
                    app.orient(hani_core::AxisRotation::taking(
                        hani_core::Facing::Front,
                        hani_core::Facing::Up,
                    ));
                }),
            ),
        ];

        for (name, swap) in cases {
            app.shared.take_pool_reset_for_tests();
            swap(&mut app);
            assert!(
                app.shared.take_pool_reset_for_tests(),
                "a {name} left the outgoing model's slots in the pool, so its triangles stay on \
                 screen underneath the new document"
            );
        }

        let _ = std::fs::remove_dir_all(&directory);
    }
}

#[cfg(test)]
mod rebuild_tests {
    use super::*;
    use crate::app::test_support::*;

    /// **Rebuilding the view keeps solo, where rebuilding everything drops it.**
    ///
    /// That single difference is the whole reason these are two functions, so
    /// it is the only assertion here that discriminates -- and saying so
    /// matters, because the obvious companions do not. Asserting the pool reset
    /// was requested passes under either function, and so does asserting that
    /// the bricks were dirtied: both do those two things identically. A test
    /// built out of those would be green with `Message::ViewRebuilt` wired
    /// straight to `rebuild_everything`, which would silently throw away a
    /// user's solo mode every time they pressed a button about the mesh pool.
    #[test]
    fn rebuilding_the_view_packs_the_pool_without_dropping_solo() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        app.solo = Some(cube);
        // Clear whatever is pending, so the assertions below are about THIS
        // call and not about the primitive that was just added -- adding a body
        // dirties bricks too, and a counter left carrying its value made the
        // dirty assertion below pass with the remesh removed.
        app.shared.take_pool_reset_for_tests();
        app.perf.dirty_bricks = 0;

        update(&mut app, Message::ViewRebuilt);

        assert_eq!(
            app.solo,
            Some(cube),
            "a rebuild of the VIEW dropped solo -- that is `rebuild_everything`'s behaviour, and \
             it is right there only because a document swap renumbers the rows"
        );
        assert!(
            app.shared.take_pool_reset_for_tests(),
            "the pool was never asked to reset, so nothing was recovered"
        );
        // **The reset is only sound paired with marking everything dirty** --
        // `request_pool_reset`'s own doc says the model otherwise leaves the
        // GPU and does not come back. That is a different mutation from the
        // solo one above and it needs its own assertion: dropping the two
        // statements after the reset would leave the solo check green and the
        // screen empty.
        assert!(
            app.perf.dirty_bricks > 0,
            "the pool was emptied and nothing was re-offered, so the model has left the screen"
        );
        assert!(
            app.status.contains("rebuilt the view"),
            "the button reported nothing: {}",
            app.status
        );
    }

    /// The remedy must not claim to give memory back.
    ///
    /// `MeshPool::reset` empties the allocators over the buffers it already
    /// holds and never drops a `BufferPair`, so nothing is returned to the
    /// system. A sentence that read as freeing VRAM would be false in the one
    /// place a user is deciding whether they have a memory problem.
    #[test]
    fn the_rebuild_does_not_claim_to_have_freed_memory() {
        let mut app = app();
        update(&mut app, Message::ViewRebuilt);
        for lie in ["MB", "memory", "VRAM", "freed"] {
            assert!(
                !app.status.contains(lie),
                "the status implies memory came back, and it did not: {}",
                app.status
            );
        }
    }
}
