// SPDX-License-Identifier: AGPL-3.0-only

//! The bodies and their rows: selecting, adding, duplicating, merging,
//! splitting, renaming, the eye and solo, deleting, folders, dragging a row,
//! the mirror planes a body allows, and the rows' pictures. Moved out of
//! `app.rs` (issue 189).

use super::*;

impl Hani {
    /// Choose the body that edits land on, with everything that has to follow.
    ///
    /// The one place a selection changes, so that the three things which follow
    /// one cannot be forgotten: the measured radius the camera and the mirror
    /// planes are sized by, the mirror axes the newly chosen body may not be
    /// allowed to use, and a status line saying what was chosen — there is no
    /// panel to show it yet.
    ///
    /// **It re-measures and must never rescale the brush**: selecting changes
    /// no geometry, and a Dynamic brush that resized because the selection
    /// changed would carve at a radius the user never chose. See
    /// [`Hani::rescale_radius`], which spells out that failure in full.
    ///
    /// **It deliberately does not set `unsaved`.** Which row is active is
    /// written to the file, so this does leave the document a shade out of step
    /// with what is on disk; raising a "discard your work?" prompt because
    /// somebody clicked a different body would be far worse, and no geometry is
    /// at stake either way.
    ///
    /// **A press on a FOLDER row selects the first body inside it**, because
    /// the active row always holds a field -- that invariant is what keeps
    /// `Option<NodeId>` out of every signature downstream, and it is not worth
    /// trading for a second notion of "which row is selected" that the brush,
    /// the mirror and the solo scope would all then have to disambiguate
    /// against. A folder's own row keeps its own affordances: the chevron, the
    /// eye and the trash all name the folder they sit on.
    pub(super) fn select_body(&mut self, row: NodeId) {
        let Some(body) = self.first_body_in(row) else {
            return;
        };
        let name = self.doc.node(body).map_or("another body", |node| node.name.as_str());
        let note = format!("selected {name}");
        self.select_body_saying(body, note);
    }

    /// The row itself when it holds a field, otherwise the first body under it.
    ///
    /// `None` only for a row that is not in the document; a folder always has
    /// at least one child, and a document always has at least one body.
    fn first_body_in(&self, row: NodeId) -> Option<NodeId> {
        let range = self.doc.subtree_of(row)?;
        self.doc.nodes()[range].iter().find(|node| node.is_body()).map(|node| node.id)
    }

    /// [`Hani::select_body`] with the caller's own line instead of "selected
    /// X".
    ///
    /// The note has to be set HERE rather than by the caller afterwards,
    /// because the mirror refusal below must have the last word: which body is
    /// selected is visible in the panel, and a mirror plane going off is not
    /// visible anywhere else. A caller that set its own status after selecting
    /// would silently eat that refusal.
    pub(super) fn select_body_saying(&mut self, body: NodeId, note: String) {
        if body == self.doc.active() {
            return;
        }
        self.doc.set_active(body);
        self.refresh_model_radius();
        self.status = note;
        self.refuse_mirrors_the_body_does_not_straddle("turned off");
    }

    /// Add one primitive as a NEW body, and select it.
    ///
    /// **The whole feature, from the user's chair: press `+`, pick Cube, and a
    /// cube appears in the list and on screen.** Everything in the eight
    /// increments before this one exists so that these few lines are the only
    /// thing that had to be written to get there.
    ///
    /// One [`Change::NodeAdded`], so ctrl+Z removes it and ctrl+shift+Z brings
    /// it back. **The camera does not move**: framing is something the user set,
    /// and a tool that re-frames on every add makes a twelve-body layout
    /// impossible to build.
    pub(super) fn add_primitive(&mut self, kind: hani_core::PrimitiveKind) {
        // Refused BEFORE anything allocates, and named rather than silently
        // ignored. The reader's own clamps do not cover this: nothing built by
        // the interface goes through the reader.
        if self.doc.body_count() >= hani_core::MAX_BODIES {
            self.status = format!(
                "could not add a {}: this document holds {} bodies, which is the limit",
                kind.label().to_lowercase(),
                self.doc.body_count()
            );
            return;
        }
        if self.doc.node_count() >= hani_core::MAX_NODES {
            self.status = format!(
                "could not add a {}: this document holds {} rows, which is the limit",
                kind.label().to_lowercase(),
                self.doc.node_count()
            );
            return;
        }

        let (centre, half) = hani_core::primitive::placement(&self.doc, MODEL_RADIUS_MM);

        // The size ceilings, checked BEFORE `build` allocates a single brick.
        // `MAX_BODIES` is nowhere near the binding limit: a cube is sized off
        // the biggest body, so at a fine voxel the very first one can be tens
        // of gigabytes, and sixty-four of anything is not what runs the machine
        // out of room. The pool figure is the WATERMARK, because adding a body
        // empties nothing and the bump pointer is what overflows -- see
        // `GrowthGuard`, whose doc comment is the argument in full.
        let pool = self.shared.stats();
        // A capacity of zero is a pool that has never reported: before the
        // first frame, and in every headless test. Judging against it would
        // refuse every add there has ever been, so the vertex ceiling is simply
        // not applied and the memory one still is. The resample guard makes the
        // same call one line into itself -- `if pool.vertices_reserved == 0 {
        // return None }` -- for the same reason.
        let headroom = if pool.vertex_capacity == 0 {
            u64::MAX
        } else {
            pool.vertex_capacity.saturating_sub(pool.vertices_watermark)
        };
        let (bytes, vertices) = hani_core::primitive::cost(kind, self.doc.voxel_size(), half);
        if let Some((why, workable)) = self.doc.growth_guard(headroom).no_room_for(bytes, vertices)
        {
            self.status = format!(
                "could not add a {} {:.1} mm across: {why} ({:.1} mm)",
                kind.label().to_lowercase(),
                half * 2.0,
                half * 2.0 * workable,
            );
            return;
        }

        let volume = hani_core::primitive::build(kind, self.doc.voxel_size(), centre, half);
        let id = self.doc.add_body(kind.label(), volume);
        let at = self.doc.node_count() - 1;
        // Its planes through its own middle. See `app/symmetry_frame.rs`.
        self.frame_at_middle(id);

        let before = self.history.stats();
        self.history.push(Entry::new(vec![Change::NodeAdded { at, id }]));
        self.record_history(before);
        self.unsaved = true;

        let note = format!("added a {} {:.1} mm across", kind.label().to_lowercase(), half * 2.0);
        self.select_body_saying(id, note);
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// Copy the active body in place, as a new row directly below it.
    ///
    /// **In place, with no offset, and that is the reference behaviour rather
    /// than a shortcut.** Photoshop and ZBrush both duplicate where the
    /// original stands. There is no move gizmo in this application, so an
    /// auto-offset copy would land in a position the user then cannot adjust;
    /// the row appearing in the list is the feedback, which is why the "a
    /// primitive at the origin is invisible on the first press" argument that
    /// governs [`Hani::add_primitive`] does not carry over here.
    ///
    /// **ZBrush is inverted on all three counts, deliberately.** Duplicating
    /// "object" there renames the ORIGINAL to "object1", hands the copy the
    /// original's name, and leaves the original selected. Users worked that out
    /// from their own undo history, and it breaks GoZ round-trips because there
    /// the name is the identity key. Here the original keeps its name, the copy
    /// gets " copy", and the copy becomes active -- names are free to collide
    /// because [`NodeId`] is the identity and nothing downstream keys off one.
    ///
    /// # What it costs, and what is refused rather than evicted
    ///
    /// The copy is the expensive operation in this application: 765 MB of
    /// dragon is 6,120 dense bricks, so it is 6,120 allocations and 1.53 GiB of
    /// memory traffic, and [`Volume::duplicated`]'s own header is why that is
    /// spelled at the call site instead of hidden behind a `.clone()`.
    ///
    /// So it is refused before it allocates, against both ceilings, with the
    /// numbers in the message -- never allowed through to be recovered from
    /// afterwards. The history is not the constraint: one `Change::NodeAdded`
    /// is a few bytes against a 256 MB budget, and it is the RAM and the mesh
    /// pool that the second copy of a large body runs out of.
    pub(super) fn duplicate_active_body(&mut self) {
        let id = self.doc.active();
        // Refused BEFORE anything allocates, and named rather than silently
        // ignored, exactly as the add path refuses. Nothing built by the
        // interface goes through the reader, so the reader's clamps do not
        // cover this.
        if self.doc.body_count() >= hani_core::MAX_BODIES {
            self.status = format!(
                "could not duplicate: this document holds {} bodies, which is the limit",
                self.doc.body_count()
            );
            return;
        }
        if self.doc.node_count() >= hani_core::MAX_NODES {
            self.status = format!(
                "could not duplicate: this document holds {} rows, which is the limit",
                self.doc.node_count()
            );
            return;
        }
        let Some(at) = self.doc.index_of(id) else {
            return;
        };
        let Some(node) = self.doc.node(id) else {
            return;
        };
        let name = node.name.clone();
        // Read before the copy is built, because the copy has to land BESIDE
        // its source rather than at the top level: a body inside a folder that
        // is duplicated at depth 0 ends the folder's run at the copy, and every
        // sibling below it falls out of the folder.
        let depth = node.depth();
        let Some(source) = node.volume() else {
            // A folder, once folders exist. Duplicating a subtree is its own
            // operation and belongs with the increment that can build one.
            return;
        };

        // Measured and not estimated, which no other add path in this
        // application can say: the copy is this body's brick map again, brick
        // for brick, so the voxel data it will hold is the voxel data this one
        // holds, to the byte. Only the brick map's own capacity can differ, and
        // that is the small term.
        let bytes = source.stats().resident_bytes as f64;
        if let Some(why) = self.no_room_to_duplicate(bytes) {
            self.status = format!("could not duplicate {name}: {why}");
            return;
        }

        // Whole bricks, and zero of them. See `Volume::duplicated` for why the
        // offset is measured in bricks at all: anything finer would be a
        // resample of the field rather than a copy of it.
        let copy = source.duplicated(glam::IVec3::ZERO);
        let copy_name = hani_core::name_that_fits(&format!("{name} copy")).to_string();
        // Directly below the row it came from and at the same depth, which is
        // where the user is looking and inside whatever folder the source sits
        // in. `insert_body` and not `add_body`: at sixty-four rows the bottom
        // of the list is off screen.
        let new_id = self.doc.insert_body(at + 1, depth, copy_name.clone(), copy);
        // The copy keeps the source's planes. See `app/symmetry_frame.rs`.
        self.frame_from(new_id, id);

        let before = self.history.stats();
        self.history.push(Entry::new(vec![Change::NodeAdded { at: at + 1, id: new_id }]));
        self.record_history(before);
        self.unsaved = true;

        self.select_body_saying(new_id, format!("duplicated {name} as {copy_name}"));
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// Why a second copy of a body costing `bytes` will not fit, or `None` to
    /// go ahead.
    ///
    /// # The vertex figure is apportioned from the pool, not estimated
    ///
    /// [`hani_core::primitive::cost`] predicts vertices from a closed-form
    /// surface area, because a cube that does not exist yet has no other
    /// answer. A duplicate does: the body is meshed and its triangles are on
    /// the GPU right now, and the copy's meshes are bit-identical to them. So
    /// the honest basis is what the pool has actually RESERVED, taken as this
    /// body's share of the document -- which is the same call
    /// [`Hani::too_fine_for_the_pool`] makes one screen up, and for the same
    /// reason its header gives: starting from a measured reservation bakes in
    /// the allocator's own padding rather than guessing at it.
    ///
    /// The share is by resident bytes and the two figures are of the same
    /// vintage -- `doc_stats` and the pool snapshot are both written by a
    /// remesh. Bytes are the right apportionment because both costs are a shell
    /// over a surface: a body holding a third of the document's voxel data has
    /// about a third of its surface and therefore about a third of its
    /// vertices.
    ///
    /// # A pool that has never reported does not veto
    ///
    /// `vertex_capacity` is zero before the first frame and in every headless
    /// test. Judging against it would refuse every duplicate there has ever
    /// been, so the vertex ceiling is simply not applied there and the memory
    /// one still is -- the same short-circuit `add_primitive` and the resample
    /// guard each make, one line into themselves.
    fn no_room_to_duplicate(&self, bytes: f64) -> Option<String> {
        let pool = self.shared.stats();
        let (headroom, vertices) = if pool.vertex_capacity == 0 {
            (u64::MAX, 0.0)
        } else {
            // The WATERMARK, never `reserved`: a duplicate empties nothing, so
            // what runs the pool out of room is the bump pointer rather than
            // the live count. `GrowthGuard`'s own header is the argument in
            // full, and this project has shipped the other reading twice.
            let headroom = pool.vertex_capacity.saturating_sub(pool.vertices_watermark);
            let share = bytes / self.doc_stats.resident_bytes.max(1) as f64;
            (headroom, pool.vertices_reserved as f64 * share)
        };
        self.doc.growth_guard(headroom).no_room_for_a_copy(bytes, vertices)
    }

    /// Merge the active body down into the body directly below it, asking first
    /// when the one entry it pushes may be too big for undo to hold.
    ///
    /// **The union of two signed distance fields is their `min`, and on a shared
    /// lattice that is all it is** -- brick `c` covers the same world box in
    /// both bodies, so there is no resampling and nothing is interpolated. See
    /// `hani_core::merge` for the engine half; everything here is the
    /// question of whether to run it and what to say afterwards.
    ///
    /// # Refused by name, never greyed and never silent
    ///
    /// The two ways there is nothing to merge into -- the bottom of a list, and
    /// a folder on the next line -- are said out loud, exactly as the other
    /// verbs in that row refuse. A merge into a folder is ZBrush's MergeVisible
    /// and its universal "the button did nothing" reaction; naming the folder is
    /// the whole difference.
    pub(super) fn merge_active_body_down(&mut self) {
        let source = self.doc.active();
        let name = self.doc.node(source).map_or_else(String::new, |node| node.name.clone());
        let target = match self.doc.merge_target(source) {
            hani_core::MergeTarget::Body(target) => target,
            hani_core::MergeTarget::Bottom => {
                self.status = format!("could not merge {name}: there is no body below it");
                return;
            }
            hani_core::MergeTarget::Folder(folder) => {
                let folder = self
                    .doc
                    .node(folder)
                    .map_or_else(|| "a folder".to_string(), |node| node.name.clone());
                self.status = format!(
                    "could not merge {name}: {folder} is a folder, and a merge joins two bodies"
                );
                return;
            }
        };

        // Microseconds and no allocation: two map lookups per brick of the
        // SOURCE, which is what makes it affordable to ask before the merge
        // rather than to discover the size afterwards.
        let Some(plan) = self.doc.merge_plan(source) else {
            return;
        };
        if plan.bytes() >= self.merge_prompt_bytes {
            let target_name =
                self.doc.node(target).map_or_else(String::new, |node| node.name.clone());
            self.pending_merge = Some(PendingMerge {
                source,
                source_name: name,
                target_name,
                bytes: plan.bytes(),
                stroke_bytes: plan.stroke_bytes,
                reclaim_bytes: plan.reclaim_bytes,
            });
            return;
        }
        self.apply_merge(source);
    }

    /// Run the merge and put the renderer back in step with it.
    ///
    /// **The three lines after the merge are the same unit `Hani::remove_body`
    /// documents, and for the same reason**: a merge consumes a body, that
    /// body's bricks are in nobody's dirty set any more, so
    /// `SharedFrame::forget_body` is what releases its slots -- queued rather
    /// than called -- and the whole-document remesh is the other half.
    /// `MeshPool::forget_body` clears the pool-full banner when it frees space,
    /// and a brick the pool refused while it was full was dropped with its
    /// coordinate long gone from the dirty set; freeing the space without
    /// re-offering everything takes the warning down and leaves the geometry
    /// missing.
    ///
    /// # This is deliberately NOT `rebuild_everything`, and the plan asked for
    /// one
    ///
    /// The plan for this increment says to call it when `PoolStats.overflowed`
    /// is non-zero afterwards. That was written before increment 6 landed
    /// `forget_body`, which now does strictly more of what the rebuild was
    /// wanted for -- it resets the allocators of any buffer pair it empties and
    /// clears `overflowed` itself -- and it does it without the one thing
    /// `rebuild_everything` also does: **clear solo.** Solo is cleared there
    /// because a whole-document swap renumbers the rows, and a merge is not a
    /// swap. Dropping the user's view mode on a merge would be a regression
    /// against increment 13 for no gain, so the rebuild is not called and this
    /// paragraph is why.
    pub(super) fn apply_merge(&mut self, source: hani_core::NodeId) {
        let source_name = self.doc.node(source).map_or_else(String::new, |node| node.name.clone());
        let Some(outcome) = self.doc.merge_down(source) else {
            return;
        };
        let target_name =
            self.doc.node(outcome.target).map_or_else(String::new, |node| node.name.clone());
        self.shared.forget_body(source);
        self.doc.mark_everything_dirty();

        let before = self.history.stats();
        self.history.push(outcome.entry);
        self.unsaved = true;
        self.status = if outcome.bricks == 0 {
            format!("merged {source_name} into {target_name} — it was already inside it")
        } else {
            format!("merged {source_name} into {target_name} — {} bricks changed", outcome.bricks)
        };
        // After the merge's own line, so an eviction wins the status.
        self.record_history(before);
        self.refresh_model_radius();
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// Walk the active body's loose parts and raise the preview card.
    ///
    /// **The refusal comes first and it is free**, read off cached stats: a
    /// split holds the original and every part at the same time, so the one
    /// body it cannot run on is a large one -- and finding that out after a
    /// minute of walking, having already doubled the process's memory, is not a
    /// refusal, it is a crash with a message attached. See `hani_core::split`
    /// for the arithmetic and for what the message offers instead.
    ///
    /// Refused when the body is not DISPLAY-visible, and that is deliberate
    /// rather than defensive. A split replaces one row with several, and the
    /// children inherit the source's own eye -- so splitting something that is
    /// hidden, or that solo is hiding, produces a pile of new rows and no
    /// change at all on screen. Pixologic published a whole #AskZBrush episode
    /// on that confusion.
    ///
    /// # There is no size prompt, and there is no greyed button
    ///
    /// The card is always raised, because "how many parts is this" is the whole
    /// question and nothing in the panel can answer it in advance. Whether a
    /// body is worth splitting is not a property a row can display: an imported
    /// scan is one row whether it is one shell or four thousand.
    pub(super) fn split_active_body(&mut self) {
        let source = self.doc.active();
        let name = self.doc.node(source).map_or_else(String::new, |node| node.name.clone());

        // `shown` is the resolved display visibility the update pass keeps in
        // step with the document, so this costs an index rather than a walk.
        let visible = self
            .doc
            .index_of(source)
            .and_then(|index| self.shown.get(index).copied())
            .unwrap_or(true);
        if !visible {
            self.status = format!(
                "could not split {name}: it is not on screen, and its parts would not be either"
            );
            return;
        }

        if let Some(why) = self.doc.split_guard(source) {
            self.status = format!("could not split {name}: {why}");
            return;
        }

        let Some(plan) = self.doc.split_plan(source) else {
            return;
        };
        if plan.is_one_piece() {
            self.status = format!("{name} is one connected part -- there is nothing to split");
            return;
        }
        self.pending_split = Some(PendingSplit { name, plan });
    }

    /// Run the split and put the renderer back in step with it.
    ///
    /// The three lines after the split are the unit `Hani::remove_body` and
    /// `Hani::apply_merge` both document, and for the same reason: the source
    /// stops existing, its bricks are in nobody's dirty set any more, so
    /// `SharedFrame::forget_body` is what releases its slots and the
    /// whole-document remesh is the other half. Freeing pool space without
    /// re-offering every brick takes the MESH POOL FULL banner down and leaves
    /// the geometry that was refused while it was full missing.
    ///
    /// **Not `rebuild_everything`**, for the reason `apply_merge` gives: a
    /// rebuild clears solo, and a split is not a whole-document swap. Solo is
    /// dropped anyway when it named the source, but that happens in
    /// `Hani::forget_a_vanished_solo`, which rides the visibility pass and
    /// therefore covers every route a row can vanish by.
    pub(super) fn apply_split(&mut self, pending: PendingSplit) {
        let PendingSplit { name, plan } = pending;
        let source = plan.source;
        let found = plan.found();
        let swept = plan.swept();
        let walked = plan.elapsed;

        let Some(outcome) = self.doc.split(plan) else {
            return;
        };
        self.shared.forget_body(source);
        self.doc.mark_everything_dirty();

        let built = outcome.elapsed;
        let slow = outcome.is_slow(walked);
        let bodies = outcome.bodies.len();

        let before = self.history.stats();
        self.history.push(outcome.entry);
        self.unsaved = true;
        self.status = if swept == 0 {
            format!("split {name} into {found} parts")
        } else {
            format!(
                "split {name}: {} parts, and {swept} of {found} swept into {name} fragments",
                bodies - 1
            )
        };
        // After the split's own line, so an eviction wins the status.
        self.record_history(before);

        // Not a status line: a slow operation is a fact about this machine and
        // this model, and the status bar is already carrying the result. The
        // perf readout's `worst_frame_ms` is the only instrument that can see
        // the frame this cost, and it cannot see a frame that never came.
        if slow {
            eprintln!(
                "split of {name} took {} ms to walk and {} ms to build",
                walked.as_millis(),
                built.as_millis(),
            );
        }

        self.refresh_model_radius();
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// Begin renaming a row: hold its current name as the field's text, and
    /// return the task that focuses the field and selects what is in it.
    ///
    /// Select-all rather than a caret at the end, because a rename is far more
    /// often a replacement than an edit -- "Cube 2" is a placeholder, and
    /// having to clear it first would make every rename two gestures.
    pub(super) fn begin_rename(&mut self, id: NodeId) -> Task<Message> {
        let Some(node) = self.doc.node(id) else {
            return Task::none();
        };
        self.renaming = Some((id, node.name.clone()));
        // **Drag is disabled while renaming**, and this is where the two can
        // meet: `mouse_area` publishes `on_press` and THEN the double click
        // (`mouse_area.rs:376-392`), so the press that opened the field has
        // already picked the row up. Put it down. Otherwise the list is armed
        // behind an open text field and every row is offering to be a drop.
        self.row_drag = None;
        // Chained rather than batched: `select_all` acts on the field's cursor
        // and there is no reason to find out what a runtime that ran them the
        // other way round would do to it.
        iced::widget::operation::focus(RENAME_FIELD)
            .chain(iced::widget::operation::select_all(RENAME_FIELD))
    }

    /// Put the typed name on the row, as one undoable change.
    ///
    /// **The single commit point**, called from [`Hani::update`]'s guard and
    /// nowhere else, so that Enter, a click on another row, a press in the
    /// viewport and a menu command all end a rename the same way. See
    /// [`keeps_the_rename_open`].
    ///
    /// Three things it refuses, each because the alternative is a silent
    /// change the user never sees:
    ///
    /// * a name that is only whitespace. The reader repairs an empty name field
    ///   to `Body {n}` (`project.rs`, `read_name`), so committing one would
    ///   rename the body to something else entirely on the next open;
    /// * a name equal to the one already there, which would cost a real undo
    ///   press for a change that is not one;
    /// * a row that has gone. Nothing can reach this today -- the guard runs
    ///   before `dispatch`, so the message that deletes a row commits the
    ///   rename against the document that still holds it -- and it is a
    ///   `let else` rather than an `expect` because "a rename outlived its
    ///   row" is not worth a crash.
    ///
    /// The clamp to [`hani_core::MAX_NAME_BYTES`] is applied on the way IN,
    /// in the `BodyRenameEdited` arm, so that the field cannot show a
    /// thirty-third byte that the file would then drop. It is applied again
    /// here anyway: this is the last point before the document, and one line
    /// is cheaper than proving that no other route into `renaming` will ever
    /// exist.
    pub(super) fn commit_rename(&mut self) {
        let Some((id, typed)) = self.renaming.take() else {
            return;
        };
        let Some(before) = self.doc.meta(id) else {
            return;
        };
        // Trimmed on both sides of the cut: leading and trailing space first,
        // then the cut, then the tail again in case the cut landed just after
        // a space.
        let name = hani_core::name_that_fits(typed.trim()).trim_end();
        if name.is_empty() {
            self.status = format!("a body needs a name — kept {}", before.name);
            return;
        }
        if name == before.name {
            return;
        }

        let after = NodeMeta { name: name.to_string(), ..before.clone() };
        let change = self.outline_change(|doc| doc.set_meta(&after));
        let stats = self.history.stats();
        self.history.push(Entry::new(vec![change]));
        self.record_history(stats);
        // The name is written to the file, so changing it is a change to the
        // document.
        self.unsaved = true;
        self.status = format!("renamed {} to {name}", before.name);
    }

    /// Snapshot the outline, make one edit to it, and hand back the change that
    /// undoes it.
    ///
    /// [`Change::Outline`] is a permutation plus field edits over a fixed id
    /// set, so its two halves have to be taken either side of the same
    /// mutation. Getting that wrong is silent -- the undo reapplies the state
    /// it was meant to replace -- so the pairing lives here rather than at each
    /// call site.
    ///
    /// **[`Hani::toggle_visibility`] deliberately does not use it**, and that
    /// is the one exception: it has to look at the RESOLVED mask between the
    /// two snapshots and may put the eye back and refuse, so its `before` has
    /// to be taken before a mutation it might undo by hand.
    ///
    /// It costs two whole-outline clones -- about 10 KB at 128 rows -- and it
    /// runs at a user action, never per frame.
    fn outline_change(&mut self, edit: impl FnOnce(&mut hani_core::Document)) -> Change {
        let before = self.doc.outline();
        edit(&mut self.doc);
        Change::Outline { before, after: self.doc.outline() }
    }

    /// Throw a rename away and leave the row exactly as it was.
    ///
    /// Escape, by way of `MenuClosed`. Nothing to undo, because nothing was
    /// ever written to the document: the typed text only ever lived in
    /// `renaming`.
    pub(super) fn cancel_rename(&mut self) {
        self.renaming = None;
    }

    /// Flip one row's OWN eye, as one undoable change.
    ///
    /// **Hiding the body that is active moves the selection**, evaluated against
    /// [`Document::saved_visibility`] and never against what solo is showing: a
    /// view mode must not be able to veto a structural rule. When there is no
    /// other visible body to move to, the hide is refused outright rather than
    /// leaving the application with an active body nobody can see and every
    /// press reporting that it is hidden.
    ///
    /// # An eye click outside the solo scope is refused, and that is not fussy
    ///
    /// While solo is on, every out-of-scope row still shows an open eye, because
    /// its own bit really is on -- solo is a mask over that bit and never a
    /// write to it. So ten rows claim "visible" over a viewport drawing one.
    /// Letting the click through means the user turns an eye off, sees nothing
    /// change, clicks again, sees nothing change -- and each of those clicks
    /// sets `unsaved` and arms an autosave of a multi-gigabyte document, with
    /// the whole effect arriving later, when they leave the mode. The refusal
    /// costs a status line and names the mode that caused it.
    pub(super) fn toggle_visibility(&mut self, id: NodeId) {
        let Some(before_meta) = self.doc.meta(id) else {
            return;
        };
        if !self.in_solo_scope(id) {
            self.status =
                format!("solo is on — leave it before changing {}'s eye", before_meta.name);
            return;
        }
        let outline_before = self.doc.outline();
        let after = NodeMeta { visible: !before_meta.visible, ..before_meta.clone() };
        self.doc.set_meta(&after);

        // Resolved rather than read off the bit, because an ancestor folder can
        // hide a row whose own eye is on.
        let mut saved = Vec::new();
        self.doc.saved_visibility(&mut saved);
        let active_is_hidden = self
            .doc
            .index_of(self.doc.active())
            .and_then(|index| saved.get(index))
            .is_some_and(|shown| !shown);

        let mut note =
            format!("{} {}", if after.visible { "showing" } else { "hiding" }, after.name);
        let mut moved_to = None;
        if active_is_hidden {
            let replacement = self
                .doc
                .nodes()
                .iter()
                .zip(&saved)
                .find(|(node, shown)| **shown && node.is_body())
                .map(|(node, _)| (node.id, node.name.clone()));
            match replacement {
                Some((next, name)) => {
                    note = format!("hiding {} — {name} is now selected", after.name);
                    moved_to = Some(next);
                }
                None => {
                    self.doc.set_meta(&before_meta);
                    self.status =
                        "cannot hide the last visible body — there would be nothing to sculpt"
                            .to_string();
                    return;
                }
            }
        }

        let before = self.history.stats();
        self.history.push(Entry::new(vec![Change::Outline {
            before: outline_before,
            after: self.doc.outline(),
        }]));
        self.record_history(before);
        // The eye is written to the file, so flipping it is a change to the
        // document.
        self.unsaved = true;
        match moved_to {
            Some(next) => self.select_body_saying(next, note),
            None => self.status = note,
        }
    }

    /// Turn every eye in the document on, as ONE undo entry, and leave solo.
    ///
    /// The way out of having hidden things and lost track of what. A no-op when
    /// nothing is hidden, and it says so rather than pushing an empty entry that
    /// would cost the user a real undo.
    ///
    /// One [`Change::Outline`] whatever the count, because the outline snapshot
    /// is whole-document either way: N eyes for the price of one.
    ///
    /// **Solo goes first and unconditionally**, before the "already showing"
    /// return: a document with every eye on and solo still on shows exactly one
    /// subtree, which is the opposite of what the user just asked for. This is
    /// solo's second exit and the lesser one -- it is also a document change,
    /// and Escape exists precisely so that leaving the mode need not be.
    pub(super) fn show_everything(&mut self) {
        let left_solo = self.solo.take().is_some();
        let hidden: Vec<NodeMeta> = self
            .doc
            .nodes()
            .iter()
            .filter(|node| !node.visible)
            .map(|node| NodeMeta { visible: true, ..node.meta() })
            .collect();
        if hidden.is_empty() {
            self.status = if left_solo {
                "left solo — everything is showing".to_string()
            } else {
                "everything is already showing".to_string()
            };
            return;
        }

        let shown = hidden.len();
        let change = self.outline_change(|doc| {
            for meta in &hidden {
                doc.set_meta(meta);
            }
        });
        let before = self.history.stats();
        self.history.push(Entry::new(vec![change]));
        self.record_history(before);
        self.unsaved = true;
        let rows = if shown == 1 { "row" } else { "rows" };
        self.status = if left_solo {
            format!("left solo, showing {shown} hidden {rows}")
        } else {
            format!("showing {shown} hidden {rows}")
        };
    }

    // --- solo, which is a MODE ------------------------------------------------

    /// Whether a row is inside the solo scope, and so whether a click on it
    /// means what it looks like.
    ///
    /// `true` whenever solo is off, which is what lets every caller be a plain
    /// guard rather than an `Option` dance. A subtree is a contiguous preorder
    /// run, so this is a range test and never a search -- the same property
    /// [`hani_core::resolve_visibility`] leans on to make the scope test one
    /// integer comparison.
    ///
    /// A solo naming a row that is not in the document answers `true` for
    /// everything, which is the safe direction: the answer only ever refuses,
    /// and [`Hani::forget_a_vanished_solo`] clears that state on the same
    /// pass anyway.
    pub(super) fn in_solo_scope(&self, id: NodeId) -> bool {
        let Some(solo) = self.solo else {
            return true;
        };
        match (self.doc.subtree_of(solo), self.doc.index_of(id)) {
            (Some(range), Some(at)) => range.contains(&at),
            _ => true,
        }
    }

    /// Show only this row's subtree.
    ///
    /// **Nothing about the mode is written to the document**, which is the whole
    /// design: leaving it restores the hand-set eyes bit for bit because none of
    /// them was ever touched. See [`Hani::solo`].
    ///
    /// # Soloing a hidden row turns its eye on, and that part IS an edit
    ///
    /// [`hani_core::resolve_visibility`] narrows and never widens -- soloing a
    /// row whose eye is off would otherwise leave the screen empty with the
    /// indicator claiming to be showing something. The resolver deliberately
    /// does not rewrite a bit the user set, and says so; it is this handler's
    /// job, and here it is an ordinary undoable change that sets `unsaved`,
    /// exactly as clicking the eye would be.
    ///
    /// The ancestors go with it. An ancestor's eye is an AND-mask, so turning on
    /// a row inside a hidden folder and stopping there shows nothing at all --
    /// the same empty screen by a longer route. At most [`hani_core::MAX_DEPTH`]
    /// rows are touched and they land in ONE [`Change::Outline`], because the
    /// snapshot is whole-document either way.
    pub(super) fn enter_solo(&mut self, id: NodeId) {
        let Some(node) = self.doc.node(id) else {
            return;
        };
        let name = node.name.clone();

        // The row and every folder above it. Bounded by `MAX_DEPTH`, and walked
        // upward through `parent_of`, which preorder makes a backward scan
        // rather than a search.
        let mut chain = vec![id];
        while let Some(parent) = self.doc.parent_of(*chain.last().expect("seeded with the row")) {
            chain.push(parent);
        }
        let closed: Vec<NodeMeta> = chain
            .iter()
            .filter_map(|row| self.doc.meta(*row))
            .filter(|meta| !meta.visible)
            .map(|meta| NodeMeta { visible: true, ..meta })
            .collect();

        self.solo = Some(id);
        if closed.is_empty() {
            self.status = format!("solo: {name} — escape leaves it");
            return;
        }

        let change = self.outline_change(|doc| {
            for meta in &closed {
                doc.set_meta(meta);
            }
        });
        let before = self.history.stats();
        self.history.push(Entry::new(vec![change]));
        self.record_history(before);
        // Turning an eye on is written to the file, so this half of the gesture
        // is a change to the document even though the mode itself is not.
        self.unsaved = true;
        self.status = format!("solo: {name} — its eye was off and is now on");
    }

    /// Leave solo.
    ///
    /// **It restores nothing, because it changed nothing.** That single line is
    /// what the mode buys over the saved-visibility vector every other tool
    /// ships: Photoshop's alt-click eye remembers the previous set only "if you
    /// haven't changed anything else", Plasticity's manual says Unisolate
    /// "does not step back to the previous hierarchical isolation layer --
    /// everything becomes visible instead", and Blender's Alt+H is documented as
    /// ruining the scene configuration. Blender's Local View is the mode version
    /// and it exits correctly, for exactly this reason.
    ///
    /// Silent when solo was already off, so that Escape falling through to the
    /// armed cut does not first announce leaving a mode nobody was in.
    pub(super) fn exit_solo(&mut self) {
        if self.solo.take().is_some() {
            self.status = "left solo".to_string();
        }
    }

    /// Delete the active body, asking first when undo may not be able to hold
    /// it.
    ///
    /// **The active row is always a body**, so this can never take a folder by
    /// accident -- which is how "deleting a body inside a collapsed folder
    /// deletes the body, never the folder" is guaranteed rather than
    /// remembered. ZBrush gets that wrong and a user reported losing an
    /// unrecoverable hour to it. Deleting a folder is [`Hani::delete_folder`],
    /// reached from that folder's own row and from nowhere else.
    pub(super) fn delete_active_body(&mut self) {
        self.delete_row(self.doc.active());
    }

    /// Delete a folder and everything in it, asking first when undo may not be
    /// able to hold the lot.
    ///
    /// Its own entry point rather than a mode of the Delete verb, and that is
    /// the structural half of the rule above: the verb names the active body
    /// and this names a folder, so no state of the panel -- collapsed least of
    /// all -- can make one of them do the other's job.
    pub(super) fn delete_folder(&mut self, id: NodeId) {
        if self.doc.node(id).is_none_or(hani_core::Node::is_body) {
            return;
        }
        self.delete_row(id);
    }

    /// One row and its whole subtree, prompted on the SUM of what it takes.
    ///
    /// **The prompt is on the SIZE and nothing else**, and the threshold is the
    /// reclaim allowance itself -- named, not copied, so raising one raises the
    /// other -- because a delete that would be evicted before it could be
    /// undone is exactly the one that has to warn.
    /// It is measured over the whole subtree, so a folder delete asks about
    /// what it is really taking -- and folders make the prompt the common case
    /// rather than the exception.
    fn delete_row(&mut self, id: NodeId) {
        if self.doc.subtree_body_count(id) >= self.doc.body_count() {
            self.status = "cannot delete the last body — a sculpt always holds one".to_string();
            return;
        }
        let Some(node) = self.doc.node(id) else {
            return;
        };
        let name = node.name.clone();
        let bytes = self.subtree_bytes(id);
        if bytes >= self.delete_prompt_bytes {
            let bodies = self.doc.subtree_body_count(id);
            self.pending_delete = Some(PendingDelete { id, name, bytes, bodies });
            return;
        }
        self.remove_body(id);
    }

    /// What deleting this row would take with it, in bytes.
    ///
    /// A sum over the whole subtree, which for a body is the body and for a
    /// folder is every field beneath it. Walks each brick map, so it belongs to
    /// a user action and never to a frame.
    fn subtree_bytes(&self, id: NodeId) -> usize {
        let Some(range) = self.doc.subtree_of(id) else {
            return 0;
        };
        self.doc.nodes()[range]
            .iter()
            .filter_map(hani_core::Node::volume)
            .map(|volume| volume.stats().resident_bytes)
            .sum()
    }

    /// Take one row and its subtree out of the document, recording every node
    /// so undo can put the lot back.
    ///
    /// The volumes MOVE into the entry -- `Volume` has no `Clone` at all -- so a
    /// delete allocates nothing and peak memory does not rise; it merely does
    /// not fall until the entry is evicted.
    ///
    /// **N removals in ONE entry**, so one ctrl+Z restores a folder of three
    /// bodies whole. [`hani_core::Document::delete_subtree`] records them in
    /// the order that makes undo put the FOLDER back before the bodies, and a
    /// folder the delete leaves empty is dissolved into the same entry.
    ///
    /// **The three lines after the removal are one unit and increment 6 wrote
    /// down why.** A removed body's bricks are in nobody's dirty set, so
    /// `SharedFrame::forget_body` is what releases its slots -- queued rather
    /// than called, because the pool lives inside the pipeline Iced owns and
    /// releasing slots while that body's meshes still sit in `pending` would
    /// re-upload them into fresh slices on the same frame. And the whole-document
    /// remesh is the other half: `MeshPool::forget_body` clears the pool-full
    /// banner when it frees space, and a brick the pool refused while it was
    /// full was dropped on the floor with its coordinate long gone from the
    /// dirty set. Freeing the space without re-offering everything takes the
    /// warning down and leaves the geometry missing.
    pub(super) fn remove_body(&mut self, id: NodeId) {
        let name = self.doc.node(id).map_or_else(String::new, |node| node.name.clone());
        let Some(changes) = self.doc.delete_subtree(id) else {
            return;
        };
        let mut bodies = 0usize;
        for change in &changes {
            if let Change::NodeRemoved { node, .. } = change
                && node.is_body()
            {
                bodies += 1;
                self.shared.forget_body(node.id);
            }
        }
        self.doc.mark_everything_dirty();

        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.unsaved = true;
        self.status = if bodies > 1 {
            format!("deleted {name} and the {bodies} bodies in it")
        } else {
            format!("deleted {name}")
        };
        // After the delete's own line, so an eviction that took an older
        // deleted body with it wins the status.
        self.record_history(before);
        self.refresh_model_radius();
        self.remesh_dirty();
        self.refresh_overlay();
    }

    // --- folders -------------------------------------------------------------

    /// Wrap the active row in a new folder, in place, with no dialog.
    ///
    /// `ctrl+G`. Photoshop's own chord, and its own behaviour: the group
    /// appears where the row was and the row is inside it, still selected.
    /// Pressed again it nests -- which is how a tree gets deeper than one level
    /// without a drag, and it is the only way to reach depth seven before
    /// increment 17 lands.
    ///
    /// The name is the first `Group n` nothing else is using, counted over the
    /// folders that exist rather than over the ids handed out, so deleting
    /// Group 2 and grouping again gives Group 2 back rather than Group 9.
    pub(super) fn group_active_body(&mut self) {
        let id = self.doc.active();
        if self.doc.node_count() >= hani_core::MAX_NODES {
            self.status = format!(
                "could not group: this document holds {} rows, which is the limit",
                self.doc.node_count()
            );
            return;
        }
        let name = self.next_group_name();
        let Some((folder, changes)) = self.doc.group(id, name.clone()) else {
            self.status = format!(
                "could not group: {} folders deep is as far as the panel goes",
                hani_core::MAX_DEPTH
            );
            return;
        };
        let _ = folder;

        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        // The tree is written to the file, so grouping is a change to the
        // document.
        self.unsaved = true;
        self.status = format!("grouped into {name}");
    }

    /// Dissolve the folder the active row sits in; its children rise out of it.
    ///
    /// `ctrl+shift+G`. It acts on the PARENT rather than on the active row,
    /// because the active row is always a body and a body has nothing to
    /// dissolve -- and because that is what makes the pair symmetric: ctrl+G
    /// then ctrl+shift+G leaves the document exactly as it was.
    pub(super) fn ungroup_active_body(&mut self) {
        let Some(parent) = self.doc.parent_of(self.doc.active()) else {
            self.status = "nothing to ungroup — this row is not in a folder".to_string();
            return;
        };
        let name = self.doc.node(parent).map_or_else(String::new, |node| node.name.clone());
        let Some(changes) = self.doc.ungroup(parent) else {
            return;
        };

        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        self.unsaved = true;
        self.status = format!("dissolved {name}");
    }

    /// Pick a row up. The row's own press, and nothing more than arming.
    ///
    /// It arms on EVERY press, including one that never moves: a press that
    /// stays still leaves `target` at `None`, and `finish_row_drag` then
    /// commits nothing. That is what keeps a click a click without a slop
    /// threshold to tune -- the threshold is the row band the pointer would
    /// have to leave for [`hani_core::drop_target`] to name a different gap.
    pub(super) fn arm_row_drag(&mut self, id: NodeId) {
        self.row_drag = Some(RowDrag { id, target: None, into: None, said: None });
    }

    /// The pointer moved over row `over`, `fraction` of the way down it.
    ///
    /// The whole of the state machine is [`hani_core::drop_target`]; this
    /// stores its answer, works out how the indicator draws, and puts a
    /// refusal's reason on the status line the first time it changes.
    pub(super) fn track_row_drag(&mut self, over: usize, fraction: f32) {
        let Some(RowDrag { id, said, .. }) = self.row_drag else {
            return;
        };
        // The row went while the button was down -- only reachable through a
        // message queued before something else took it. Let go of it rather
        // than dragging a row that is not there.
        let Some(dragged) = self.doc.index_of(id) else {
            self.row_drag = None;
            return;
        };

        match hani_core::drop_target(self.doc.nodes(), dragged, over, fraction) {
            Ok(target) => {
                // Which band produced it, read back off the target rather than
                // recomputed from the pointer: only a closed folder's middle
                // band lands at the END of a row's subtree ONE level deeper
                // than that row.
                let node = &self.doc.nodes()[over];
                let inside = target.depth == node.depth() + 1
                    && target.at == hani_core::subtree(self.doc.nodes(), over).end;
                self.row_drag = Some(RowDrag {
                    id,
                    target: Some(target),
                    into: inside.then_some(over),
                    said: None,
                });
            }
            Err(refusal) => {
                if said != Some(refusal) {
                    let line = {
                        let name = self.doc.node(id).map_or("that row", |node| node.name.as_str());
                        format!("{name} {}", refusal.reason())
                    };
                    self.status = line;
                }
                self.row_drag = Some(RowDrag { id, target: None, into: None, said: Some(refusal) });
            }
        }
    }

    /// The button came up: perform the move the indicator was drawing.
    ///
    /// One [`Change::Outline`] for the whole reorder however many rows the
    /// block holds, plus a `NodeRemoved` for any folder the departure left
    /// empty -- one entry, so one ctrl+Z puts the tree back exactly as it was.
    ///
    /// **A drag that changes nothing pushes no entry and does not set
    /// `unsaved`.** `Document::reparent` answers that question, not this: the
    /// panel is allowed to draw a line where the row already is, because
    /// refusing to draw one there would read as "you cannot put it back".
    pub(super) fn finish_row_drag(&mut self) {
        let Some(drag) = self.row_drag.take() else {
            return;
        };
        let Some(target) = drag.target else {
            return;
        };
        let was = self.doc.parent_of(drag.id);
        let Some(changes) = self.doc.reparent(drag.id, target) else {
            return;
        };

        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        // The tree is written to the file, so a reorder is a change to the
        // document even though not one voxel moved.
        self.unsaved = true;

        let name = self.doc.node(drag.id).map_or_else(String::new, |node| node.name.clone());
        let now = self.doc.parent_of(drag.id);
        self.status = if was == now {
            // A pure reorder among its own siblings. Saying "into Group 1"
            // here would be true and useless -- it was already in Group 1.
            format!("moved {name}")
        } else {
            match now.and_then(|folder| self.doc.node(folder)) {
                Some(folder) => format!("moved {name} into {}", folder.name),
                None => format!("moved {name} out to the top level"),
            }
        };
    }

    /// Fold a folder's children away, or show them again.
    ///
    /// **Collapse changes only what is DRAWN.** It never changes what a command
    /// does, which is the ZBrush failure this whole design is written against:
    /// there, deleting a subtool inside a closed folder deletes the folder.
    ///
    /// Recorded like any other field of the outline, because `collapsed` is
    /// written to the file -- a change the next save keeps and no ctrl+Z can
    /// reach would be the only such change in the application.
    pub(super) fn toggle_collapse(&mut self, folder: NodeId) {
        let Some(node) = self.doc.node(folder) else {
            return;
        };
        let collapsed = !node.collapsed;
        let Some(changes) = self.doc.set_collapsed(folder, collapsed) else {
            return;
        };

        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        self.unsaved = true;
    }

    /// The first `Group n` no folder in the document is called.
    ///
    /// Counted over the names in use rather than off a running number, so the
    /// list does not creep up to `Group 40` in a document that has never held
    /// more than three folders. At most [`hani_core::MAX_NODES`] tries, and
    /// it runs once per ctrl+G.
    pub(super) fn next_group_name(&self) -> String {
        (1..=hani_core::MAX_NODES + 1)
            .map(|n| format!("Group {n}"))
            .find(|name| self.doc.folders().all(|folder| &folder.name != name))
            .expect("more candidate names than there are rows to use them")
    }

    /// Turn off every enabled mirror plane the active body sits wholly to one
    /// side of, and say which.
    ///
    /// **This is the mitigation for the centre being the lattice origin.** With
    /// [`MIRROR_CENTRE`] pinned there, a dent carved into a body at x = +80 has
    /// its twin written at x = -80: free-floating geometry, in empty space, that
    /// exports as an extra shell no slicer can print and nothing on screen
    /// explains. Refusing the mirror is what stops the first primitive a user
    /// adds from growing one.
    ///
    /// Turning the axis off rather than keeping it on and quietly ignoring it
    /// is the honest form: there is then one answer to "is X mirroring on", the
    /// strip highlight and the drawn plane agree with the sculpt, and nothing
    /// has to carry an effective-versus-actual pair.
    ///
    /// # What it costs, and when it is allowed to run
    ///
    /// [`Volume::surface_bounds`] scans every dense brick and its own
    /// documentation forbids calling it per frame, so this asks only at a user
    /// action and only while a mirror is actually enabled — the `is_off` check
    /// is what keeps the common case free. The two call sites are enabling an
    /// axis and choosing a different body, which are the two ways the pairing
    /// of "this axis" with "this body" can change.
    ///
    /// **The residual, said out loud:** sculpting a body across the plane while
    /// the axis is off does not turn it back on by itself. The user toggles the
    /// axis and it is re-measured. Re-measuring per stroke would put a
    /// full-model scan on every button release, which is the thing the cache in
    /// `hani-core` refuses to hold for exactly this reason.
    pub(super) fn refuse_mirrors_the_body_does_not_straddle(&mut self, verb: &str) {
        if self.symmetry_settings().is_off() {
            return;
        }
        let Some(bounds) = self.doc.active_volume().surface_bounds() else {
            // No surface, so there is nothing to be on one side of. A mirror of
            // nothing is nothing.
            return;
        };
        for axis in MirrorAxis::ALL {
            if !self.symmetry_settings().axis(axis)
                || straddles(bounds, axis, self.framed_symmetry().frame())
            {
                continue;
            }
            self.set_symmetry(self.symmetry_settings().with_axis(axis, false));
            self.status = self.mirror_refusal(axis, verb);
        }
        // And the radial axis. See `app/symmetry_frame.rs`.
        self.refuse_radial_off_axis(verb);
    }

    /// Whether the active body has material on both sides of one mirror plane.
    ///
    /// Separate from the sweep above because enabling an axis asks about one
    /// axis and a selection asks about all three, and the sweep is what makes
    /// one scan of the field answer for all of them.
    fn mirror_straddles(&self, axis: MirrorAxis) -> bool {
        let frame = self.framed_symmetry().frame();
        self.doc
            .active_volume()
            .surface_bounds()
            .is_none_or(|bounds| straddles(bounds, axis, frame))
    }

    /// What the user is told when a mirror plane misses the body.
    fn mirror_refusal(&self, axis: MirrorAxis, verb: &str) -> String {
        let name = self.doc.node(self.doc.active()).map_or("the body", |node| node.name.as_str());
        format!(
            "{} mirroring {verb}: {name} sits entirely to one side of the {} plane, so every \
             mirrored stroke would land in empty space",
            axis.label(),
            axis.label(),
        )
    }

    /// Turn one mirror plane on or off, refusing to turn one on that the
    /// active body does not straddle.
    ///
    /// **The one gate every way of enabling a mirror goes through.** There is
    /// more than one way: the symmetry strip sends a message, and a SpaceMouse
    /// button mapped to `ToggleSymmetry` toggles X directly. While the refusal
    /// lived inside the message arm the SpaceMouse route turned X on
    /// unrefused, and every stroke after it wrote its twin into empty space --
    /// with the strip highlight and the drawn plane both agreeing that nothing
    /// was wrong, and the status line saying nothing at all. One function
    /// rather than two spellings of one.
    ///
    /// Turning one OFF is never refused: the refusal exists to stop material
    /// appearing where the user cannot see it, and nothing appears when a plane
    /// goes away.
    pub(super) fn toggle_mirror(&mut self, axis: MirrorAxis) {
        // The active body's own. See `app/symmetry_frame.rs`.
        let settings = self.symmetry_settings();
        if settings.axis(axis) {
            self.set_symmetry_recorded(settings.with_axis(axis, false));
        } else if self.mirror_straddles(axis) {
            self.set_symmetry_recorded(settings.with_axis(axis, true));
        } else {
            self.status = self.mirror_refusal(axis, "refused");
        }
        // **The planes are part of what the overlay draws, so changing them has
        // to rebuild it.** `refresh_overlay`'s own doc already lists "a brush or
        // mirror setting" among its callers and this was not one of them, so the
        // planes lagged a toggle behind: whatever the last refresh had built
        // stayed on screen until some unrelated pointer motion rebuilt it.
        // Turning X OFF left its plane drawn, and turning it back ON drew
        // nothing until the next interaction. Observed by driving the running
        // application; no test could see it, because the assertion everyone
        // writes is on `self.symmetry`, which was right the whole time.
        //
        // Here rather than at the two call sites, so the keyboard and the strip
        // cannot drift apart.
        self.refresh_overlay();
    }

    /// Which rows are DRAWN, indexed by node position, solo included.
    ///
    /// The plane cut is the caller, and direct manipulation acts on what is
    /// drawn: a cut is a line the user draws across what they can see, so solo
    /// narrows it. Nothing on screen distinguishes an eye-hidden body from a
    /// solo-hidden one -- in both cases it is simply not there -- so a line
    /// drawn across one body cutting five would have no explanation anywhere.
    ///
    /// Allocating, like [`Hani::saved_nodes`] and for the same reason: a
    /// gesture, never a frame. The frame-rate answer is
    /// [`Hani::publish_visibility`]'s kept buffer.
    pub(super) fn drawn_nodes(&self) -> Vec<bool> {
        let mut shown = Vec::new();
        self.doc.display_visibility(self.solo, &mut shown);
        shown
    }

    /// Work out what is drawn and tell the renderer, wholesale.
    ///
    /// **This is the second caller of the one visibility rule**, and the reason
    /// the rule is one function: the eye in the panel, the pick gate, the plane
    /// cut and this all have to agree, and they agree by asking rather than by
    /// each remembering. The answer is recomputed in full every time and never
    /// patched -- an incremental version would be a second owner of it, and two
    /// owners is how the eye and the viewport come to disagree after an undo,
    /// leaving a body invisible on screen that still raycasts and still carves.
    ///
    /// Called from [`Hani::update`] after every message rather than from the
    /// handful of places that change an eye, because "the handful of places" is
    /// a list that goes out of date silently. It runs on the frame tick too,
    /// so it allocates nothing: both buffers are kept and refilled, **and so
    /// are the two inside [`crate::thumbnails::Thumbnails::sync`] at the
    /// bottom of it**. Anything added here is held to the same rule.
    ///
    /// **Bodies only.** Folder rows have no geometry in the pool, and passing
    /// one to the renderer would be a name it can never match.
    pub(super) fn publish_visibility(&mut self) {
        self.forget_a_vanished_solo();
        self.doc.display_visibility(self.solo, &mut self.shown);
        self.leave_a_solo_that_shows_nothing();
        self.hidden_bodies.clear();
        for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
            if !shown && node.is_body() {
                self.hidden_bodies.push(node.id);
            }
        }
        // Operands drawn through a boolean group's result. See
        // `app/booleans.rs`.
        self.refresh_booleans();
        self.hide_boolean_rows();
        self.shared.set_hidden(&self.hidden_bodies);
        // Explode lays the drawn bodies out again. See `app/explode.rs`.
        self.publish_spread();
        self.publish_mask_view();
        // Cells ride along on the same walk, and for the same argument: adding
        // and removing a body happens in a handful of places, and a handful of
        // places is a list that goes out of date silently. `sync` is idempotent
        // -- a call where the body set has not changed marks nothing stale --
        // so hiding, soloing, renaming and reordering still queue zero renders.
        self.thumbs.sync(self.doc.nodes().iter().filter(|node| node.is_body()).map(|node| node.id));
        // Here for the reason the visibility pass itself is here: the set of
        // messages that can change what the card says -- a stroke, an undo, a
        // rename, a delete, a merge, a resample, an open, a solo -- is not a
        // list anyone can keep correct, and a card that goes stale is a card
        // naming the wrong body. The expensive half is behind a revision check;
        // see `refresh_mask_card`.
        self.refresh_mask_card();
    }

    /// One body's name, for a status line.
    ///
    /// Separate from [`Hani::active_body_name`] because a status line about a
    /// gesture that crossed several bodies has to be able to name the one it
    /// actually touched. Naming the active body regardless is how the mask
    /// cutter came to report a body the lasso never went near.
    pub(super) fn body_name(&self, body: NodeId) -> String {
        self.doc.node(body).map_or("this body", |node| node.name.as_str()).to_string()
    }

    /// The active body's name, for a status line.
    pub(super) fn active_body_name(&self) -> String {
        self.body_name(self.doc.active())
    }

    /// Ask for one body's picture, if this is a frame that may pay for one.
    ///
    /// **Every gate here is about cost, and they are in cheapest-first order.**
    /// A body the size this application is built for is planned against about
    /// 8 ms, which fits in a 16 ms frame exactly once and only when the frame
    /// is otherwise idle.
    ///
    /// * the pictures are switched off -- the panel's own pressure valve;
    /// * a drag or a hold-and-drag resize is in progress, so the user is mid
    ///   gesture and every millisecond is visible;
    /// * the geometry has not been still for [`crate::thumbnails::SETTLE`];
    /// * one is already queued. The mailbox holds exactly one, which is what
    ///   makes "at most one body per frame" a property of the type rather than
    ///   a rule someone has to keep.
    ///
    /// Driven off the frame tick, like the autosave, and never off `view()`:
    /// `view()` runs at display rate and a render queued from there would be
    /// one per frame per row.
    pub(super) fn request_a_thumbnail(&mut self) {
        if !self.thumbnails
            || self.drag.is_some()
            || self.sizing.is_some()
            || !self.thumbs.settled()
            || self.shared.thumbnail_pending()
        {
            return;
        }
        let Some(body) = self.thumbs.next_stale() else {
            return;
        };
        let Some(cell) = self.thumbs.cell(body) else {
            // Past the atlas's sixty-four layers. Nothing to draw into, and
            // nothing to keep offering.
            self.thumbs.requested(body);
            return;
        };
        // `world_bounds` and not `surface_bounds`: see `ThumbRequest`, where
        // the refusal this follows is quoted in full. An empty body has no box
        // and no picture -- its cell keeps the placeholder, which is the right
        // answer for a body with nothing in it.
        let bounds = self.doc.volume(body).and_then(hani_core::Volume::world_bounds);
        self.thumbs.requested(body);
        if let Some(bounds) = bounds {
            self.shared.request_thumbnail(ThumbRequest { body, cell, bounds });
        }
    }

    /// Drop solo the moment it stops showing anything, and say so.
    ///
    /// **"Solo always shows something" is an invariant here rather than a
    /// courtesy in [`Hani::enter_solo`]**, because entry is not the only way
    /// into the empty state and the other ways are one keystroke each:
    ///
    /// * ctrl+Z. Turning the soloed row's eye on is an ordinary undoable change
    ///   and undo is deliberately not vetoed by solo (see
    ///   [`Hani::saved_nodes`]), so the undo straight after soloing a hidden
    ///   row turns that eye back off underneath the mode.
    /// * The soloed row's own eye. A subtree contains its own root, so
    ///   [`Hani::in_solo_scope`] passes that click -- correctly, it is the one
    ///   eye in the panel whose state the user can actually see the effect of.
    ///
    /// Both leave every row resolving to false: a black viewport, a badge
    /// naming a row nobody can see, and a status line about something else
    /// entirely. Escape recovers it and nothing on screen points at Escape.
    /// Catching it here costs one fold over a buffer that was just computed
    /// anyway, and it catches the third route nobody has thought of yet -- the
    /// same argument [`Hani::forget_a_vanished_solo`] makes one function
    /// down.
    ///
    /// **It replaces whatever status the gesture wrote.** The gesture's own
    /// note is the smaller news: the viewport just changed wholesale, and a
    /// message about which body got selected does not explain that. It refills
    /// `shown` rather than leaving the caller a mask it is about to contradict.
    ///
    /// Bodies only, matching [`Hani::publish_visibility`]: a folder resolving
    /// to true draws nothing, so counting folders would call an empty screen
    /// full.
    fn leave_a_solo_that_shows_nothing(&mut self) {
        let Some(id) = self.solo else {
            return;
        };
        let showing =
            self.doc.nodes().iter().zip(&self.shown).any(|(node, shown)| *shown && node.is_body());
        if showing {
            return;
        }
        let name =
            self.doc.node(id).map_or_else(|| "that row".to_string(), |node| node.name.clone());
        self.solo = None;
        self.doc.display_visibility(None, &mut self.shown);
        self.status = format!("left solo — {name} is hidden, so it was showing nothing");
    }

    /// Drop solo the moment the row it names stops existing.
    ///
    /// A dangling `Some(id)` is worse than either state it sits between:
    /// [`hani_core::resolve_visibility`] finds no node to scope to and shows
    /// **nothing at all**, while the mode is still on and its only exit -- the
    /// indicator naming the row -- has no name to draw.
    ///
    /// **Here rather than in the delete path, and that is deliberate.** A row
    /// stops existing on a delete, on a folder dissolving, on the undo of a
    /// group, on the redo of a delete, and on a merge and a split when those
    /// land. "The places that remove a row" is a list that goes out of date
    /// silently, which is the argument [`Hani::update`] already makes about
    /// the visibility pass itself -- so this rides on the same pass, one
    /// `index_of` over at most [`hani_core::MAX_NODES`] rows, allocating
    /// nothing.
    ///
    /// A whole-document swap is a different problem and has a different answer:
    /// the incoming document numbers its rows from 1 again, so a stale id names
    /// a real and perfectly innocent body rather than nothing. See
    /// [`Hani::rebuild_everything`], which clears solo outright.
    fn forget_a_vanished_solo(&mut self) {
        if let Some(id) = self.solo
            && self.doc.index_of(id).is_none()
        {
            self.solo = None;
        }
    }
}

/// The one rule about what is drawn, checked from both ends.
///
/// Visibility has three inputs -- a body's own eye, every ancestor folder's
/// eye, and solo -- resolved in one function in `hani-core` and read by the
/// panel, the pick gate, the plane cut and the renderer. These tests are about
/// the last of those: that what the renderer has been told never disagrees
/// with what the document says. When those two drift, a body is missing from
/// the viewport while its row still reads "visible", and it still raycasts and
/// still carves -- which is not a failure anyone can reproduce from a
/// description.
#[cfg(test)]
mod visibility_tests {
    use super::*;
    use hani_core::NodeMeta;
    use iced::Vector;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// A second body with something in it, so that hiding it is a question
    /// about geometry rather than about an empty row.
    fn add_body(app: &mut Hani, name: &str) -> NodeId {
        let mut volume = Volume::new(app.doc.voxel_size());
        volume.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 10.0);
        volume.mark_everything_dirty();
        app.doc.add_body(name, volume)
    }

    fn set_eye(app: &mut Hani, id: NodeId, visible: bool) {
        let meta = NodeMeta { visible, ..app.doc.meta(id).expect("the body is in the document") };
        app.doc.set_meta(&meta);
    }

    /// What the document says is hidden, worked out from the resolver rather
    /// than from `publish_visibility`'s own fold.
    ///
    /// **`app.solo` and not `None`.** The pinned check is that
    /// `hidden_snapshot()` equals `doc.display_visibility(app.solo)` after every
    /// message, so passing `None` here would make every solo message look like
    /// a disagreement -- and, worse, would keep passing if solo ever stopped
    /// reaching the renderer at all.
    fn hidden_by_the_document(app: &Hani) -> Vec<NodeId> {
        let mut shown = Vec::new();
        app.doc.display_visibility(app.solo, &mut shown);
        app.doc
            .nodes()
            .iter()
            .zip(&shown)
            .filter(|(node, shown)| !**shown && node.is_body())
            .map(|(node, _)| node.id)
            .collect()
    }

    fn assert_agrees(app: &Hani, after: &str) {
        assert_eq!(
            app.shared.hidden_snapshot(),
            hidden_by_the_document(app),
            "the renderer and the document disagree about what is drawn, after {after}"
        );
    }

    /// **After every message**, which is why the check is written as a loop
    /// over messages rather than as one assertion per feature.
    ///
    /// The pass runs in `update` rather than in the arms that change an eye,
    /// precisely because "the arms that change an eye" is a list that goes out
    /// of date silently -- open, reset, import, undo, redo, delete and solo all
    /// belong to it, and increments 9 to 13 add more. If someone moves the pass
    /// back into the arms, this test is what fails.
    #[test]
    fn the_renderers_hidden_set_agrees_with_the_document_after_every_message() {
        let mut app = app();
        let second = add_body(&mut app, "Body 2");
        set_eye(&mut app, second, false);

        // Nothing has been dispatched yet, so this is also the check that the
        // constructor published something rather than leaving the renderer to
        // find out on the first frame.
        update(&mut app, Message::Frame);
        assert_eq!(
            app.shared.hidden_snapshot(),
            vec![second],
            "a hidden body never reached the renderer at all"
        );

        // A stroke, so undo and redo have something to move.
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(640.0, 360.0),
            size: Vector::new(1280.0, 720.0),
        });
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: Vector::new(640.0, 360.0),
            size: Vector::new(1280.0, 720.0),
        });
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });

        let messages = [
            Message::Frame,
            Message::BrushKindChanged(BrushKind::Draw),
            Message::SymmetryAxisToggled(MirrorAxis::X),
            Message::StatsToggled,
            Message::Undo,
            Message::Redo,
            Message::Undo,
            Message::Frame,
        ];
        for message in messages {
            let named = format!("{message:?}");
            update(&mut app, message);
            assert_agrees(&app, &named);
        }

        // Showing it again has to travel the same way, or the eye turns things
        // off and never back on.
        set_eye(&mut app, second, true);
        update(&mut app, Message::Frame);
        assert!(app.shared.hidden_snapshot().is_empty(), "the body never came back");
        assert_agrees(&app, "showing the body again");
    }

    /// A whole-document swap must not leave the renderer holding the previous
    /// document's ids.
    ///
    /// Ids restart from 1 in every new document, so a stale hidden set does not
    /// merely name something absent -- it names a real body in the new
    /// document, and hides it.
    #[test]
    fn opening_and_resetting_do_not_leave_the_previous_documents_ids_hidden() {
        let directory =
            std::env::temp_dir().join(format!("hani-visibility-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("two-bodies.hani");

        let mut app = app();
        let second = add_body(&mut app, "Body 2");
        set_eye(&mut app, second, false);
        update(&mut app, Message::Frame);
        assert_eq!(app.shared.hidden_snapshot(), vec![second]);
        app.save_project(&path);

        // Reset first: one body, nothing hidden, and the id the old second body
        // used is now free for something else to be given.
        update(&mut app, Message::ResetSphere);
        assert!(app.confirm.is_none(), "the fixture had unsaved work and never reset");
        assert!(
            app.shared.hidden_snapshot().is_empty(),
            "the reset document is still hiding an id from the document before it"
        );
        assert_agrees(&app, "a reset");

        // ...and opening the saved file brings the hidden body back, because
        // the eye is persisted state and a reopen is not a view mode.
        app.open_project(&path);
        assert!(!app.status.contains("could not"), "open reported: {}", app.status);
        assert_eq!(
            app.shared.hidden_snapshot().len(),
            1,
            "the saved eye did not reach the renderer after the open"
        );
        assert_agrees(&app, "an open");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// **Hiding is a draw-time skip**, so it must not dirty a single brick.
    ///
    /// The other half of this rule -- that no pool space moves either -- is in
    /// `hani-gpu`, because `vertices_reserved` and `vertices_watermark` do
    /// not exist outside it. This is the half that can be seen from here, and
    /// it is the one that would be broken by the obvious wrong implementation:
    /// marking the hidden body's bricks dirty so they mesh to nothing.
    #[test]
    fn hiding_and_showing_a_body_marks_no_brick_dirty() {
        let mut app = app();
        let second = add_body(&mut app, "Body 2");
        update(&mut app, Message::Frame);

        // Settle: whatever the new body brought with it is meshed, so anything
        // dirty after this point was dirtied by the eye. The second call is
        // what makes the fixture honest -- `perf.dirty_bricks` is written by
        // `remesh_dirty` and by nothing else, so a stale one from the first
        // call would make every assertion below pass or fail for the wrong
        // reason.
        app.remesh_dirty();
        app.remesh_dirty();
        assert_eq!(app.perf.dirty_bricks, 0, "the fixture did not settle");

        for visible in [false, true, false] {
            set_eye(&mut app, second, visible);
            update(&mut app, Message::Frame);
            app.remesh_dirty();
            assert_eq!(
                app.perf.dirty_bricks,
                0,
                "turning the eye {} marked bricks for a remesh",
                if visible { "on" } else { "off" }
            );
        }
    }

    /// **Hiding a FOLDER moves the active body**, and the reason it has to is
    /// that a folder hides the active body TRANSITIVELY: the row's own eye is
    /// still on, so a rule that read the bit rather than the resolved mask
    /// would see nothing wrong and leave the brush pointing at a body that is
    /// not drawn -- where every press carves, sets `unsaved`, pays a remesh and
    /// changes not one pixel.
    #[test]
    fn hiding_a_folder_moves_the_active_body_out_of_it() {
        let mut app = app();
        let inside = add_body(&mut app, "Inside");
        update(&mut app, Message::BodySelected(inside));
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.parent_of(inside).expect("the new folder");

        update(&mut app, Message::BodyVisibilityToggled(folder));

        assert!(app.doc.node(inside).unwrap().visible, "the folder's eye was written into the row");
        assert_ne!(app.doc.active(), inside, "the active body is inside a hidden folder");
        assert!(app.doc.volume(app.doc.active()).is_some());
        assert_agrees(&app, "hiding a folder");

        // And back: the child's own bit was never touched, so re-showing the
        // folder restores exactly what was there.
        update(&mut app, Message::BodyVisibilityToggled(folder));
        let mut shown = Vec::new();
        app.doc.saved_visibility(&mut shown);
        assert!(shown.iter().all(|shown| *shown), "re-showing the folder left something hidden");
    }
}

/// Solo: a MODE, and everything that follows from it being one.
///
/// The whole increment rests on one property -- **solo is never written
/// anywhere** -- and the tests here are the two halves of it. Leaving the mode
/// restores the hand-set eyes bit for bit because none of them was ever
/// touched, and the file cannot carry a trace of it because `project::write`
/// takes a `&Document` and a `&ProjectState` and solo is a field of neither.
/// The second half is a type-system guarantee, and it is asserted anyway: the
/// obvious "tidy" is to move solo onto `Document` or `View` so a call site
/// loses a parameter, and that change compiles.
#[cfg(test)]
mod solo_tests {
    use super::*;
    use hani_core::NodeMeta;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// A body with geometry in it, so hiding one is a question about a model
    /// rather than about an empty row. Spaced along X so no two overlap.
    fn add_body(app: &mut Hani, name: &str, at: f32) -> NodeId {
        let mut volume = Volume::new(app.doc.voxel_size());
        volume.seed_sphere(Vec3::new(at, 0.0, 0.0), 6.0);
        volume.mark_everything_dirty();
        let id = app.doc.add_body(name, volume);
        app.remesh_dirty();
        id
    }

    /// Which rows are DRAWN, by id, straight from the resolver.
    fn drawn(app: &Hani) -> Vec<NodeId> {
        let mut shown = Vec::new();
        app.doc.display_visibility(app.solo, &mut shown);
        app.doc
            .nodes()
            .iter()
            .zip(&shown)
            .filter(|(_, shown)| **shown)
            .map(|(node, _)| node.id)
            .collect()
    }

    fn assert_renderer_agrees(app: &Hani, after: &str) {
        let mut shown = Vec::new();
        app.doc.display_visibility(app.solo, &mut shown);
        let expected: Vec<NodeId> = app
            .doc
            .nodes()
            .iter()
            .zip(&shown)
            .filter(|(node, shown)| !**shown && node.is_body())
            .map(|(node, _)| node.id)
            .collect();
        assert_eq!(
            app.shared.hidden_snapshot(),
            expected,
            "the renderer is not being told what solo is doing, after {after}"
        );
    }

    /// **The property the whole design exists for.** An arbitrary sequence of
    /// eye toggles, then in and out of solo, and every bit is exactly where the
    /// user left it.
    ///
    /// Every shipped alternative fails this. Photoshop's alt-click eye restores
    /// the previous set only "if you haven't changed anything else"; Plasticity's
    /// manual says Unisolate "does not step back to the previous hierarchical
    /// isolation layer -- everything becomes visible instead"; Blender's Alt+H
    /// is documented as ruining the scene configuration. They all save a vector
    /// and restore it. This saves nothing, so there is nothing to get wrong.
    #[test]
    fn leaving_solo_restores_every_hand_set_eye_bit_for_bit() {
        let mut app = app();
        let first = app.doc.active();
        let second = add_body(&mut app, "Body 2", 30.0);
        let third = add_body(&mut app, "Body 3", 60.0);
        let fourth = add_body(&mut app, "Body 4", 90.0);

        // An arbitrary sequence, through the real message, including a row
        // toggled twice and one toggled three times. The active body is never
        // in it, so nothing here can move the selection or be refused.
        for id in [second, third, second, fourth, third, third] {
            update(&mut app, Message::BodyVisibilityToggled(id));
        }
        let hand_set: Vec<NodeMeta> = app.doc.outline();
        assert_eq!(
            hand_set.iter().filter(|meta| !meta.visible).count(),
            2,
            "the fixture left no hand-set eyes to restore, so this would pass on anything"
        );
        let unsaved_before = app.unsaved;
        let history_before = app.history.stats();

        update(&mut app, Message::SoloEntered(first));
        assert_eq!(drawn(&app), vec![first], "solo is showing more than the row it names");
        assert_renderer_agrees(&app, "entering solo");

        update(&mut app, Message::SoloExited);

        assert_eq!(
            app.doc.outline(),
            hand_set,
            "leaving solo did not put the hand-set eyes back exactly as they were"
        );
        assert_eq!(app.unsaved, unsaved_before, "a view mode dirtied the document");
        assert_eq!(
            app.history.stats().undo_entries,
            history_before.undo_entries,
            "a view mode pushed an undo entry"
        );
        assert!(app.solo.is_none());
        assert_renderer_agrees(&app, "leaving solo");
    }

    /// **A document saved while soloed is byte-identical to one saved without
    /// it**, and the writer is called directly so that the claim is about the
    /// bytes and not about a round trip that might normalise something.
    ///
    /// This is the assertion that would fail if solo were ever moved onto
    /// `Document`, `ProjectState` or `View` -- which is exactly the tidy-up
    /// somebody reaches for when a call site has one parameter too many, and it
    /// is a change that compiles.
    #[test]
    fn saving_while_soloed_writes_the_same_bytes_as_saving_without_it() {
        let mut app = app();
        let first = app.doc.active();
        let second = add_body(&mut app, "Body 2", 30.0);
        add_body(&mut app, "Body 3", 60.0);
        // Something hand-set in there, so "identical" is not identical because
        // every eye happens to be on.
        update(&mut app, Message::BodyVisibilityToggled(second));

        let mut without = Vec::new();
        hani_core::project::write(&mut without, &app.doc, &app.project_state())
            .expect("writing to memory");

        update(&mut app, Message::SoloEntered(first));
        assert!(app.solo.is_some(), "the fixture never entered solo");
        let mut with = Vec::new();
        hani_core::project::write(&mut with, &app.doc, &app.project_state())
            .expect("writing to memory");

        assert_eq!(with, without, "solo left a trace in the file");

        // And the file that comes back has no trace of it either: the eye the
        // user set is still off, and solo is not a thing the reader can even
        // name.
        let (reopened, _) = hani_core::project::read(&mut with.as_slice()).expect("reading back");
        let mut shown = Vec::new();
        reopened.saved_visibility(&mut shown);
        assert_eq!(
            shown.iter().filter(|shown| !**shown).count(),
            1,
            "the reopened document does not hold exactly the one eye that was turned off"
        );
    }

    /// **Soloing a row whose eye is off turns the eye on**, because the
    /// resolver deliberately will not.
    ///
    /// [`hani_core::resolve_visibility`] narrows and never widens, and says so
    /// in its own documentation: rewriting a bit the user set is not its
    /// business. Without this handler doing it, "show me only this" would show
    /// nothing at all, with the header indicator naming a row that is not on
    /// screen. It is an ordinary undoable edit and it sets `unsaved`, exactly as
    /// clicking the eye would.
    #[test]
    fn soloing_a_hidden_body_turns_its_eye_on() {
        let mut app = app();
        let second = add_body(&mut app, "Body 2", 30.0);
        update(&mut app, Message::BodySelected(second));
        update(&mut app, Message::BodyVisibilityToggled(second));
        assert!(!app.doc.node(second).expect("the body").visible, "the fixture never hid it");
        app.unsaved = false;
        let entries_before = app.history.stats().undo_entries;

        update(&mut app, Message::SoloEntered(second));

        assert!(app.doc.node(second).expect("the body").visible, "solo showed nothing at all");
        assert_eq!(drawn(&app), vec![second]);
        assert!(app.unsaved, "turning an eye on is a change to the document");
        assert_eq!(
            app.history.stats().undo_entries,
            entries_before + 1,
            "the eye that was turned on is not undoable"
        );
        assert_renderer_agrees(&app, "soloing a hidden body");
    }

    /// The ancestors go with it, or the row is still masked by the folder above
    /// it and the screen is still empty.
    #[test]
    fn soloing_a_body_inside_a_hidden_folder_opens_the_folders_eye_too() {
        let mut app = app();
        let inside = add_body(&mut app, "Inside", 30.0);
        update(&mut app, Message::BodySelected(inside));
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.parent_of(inside).expect("the new folder");
        // Hiding the folder moves the selection out of it, which is the rule
        // `toggle_visibility` documents -- so the fixture puts it back.
        update(&mut app, Message::BodyVisibilityToggled(folder));
        assert!(!app.doc.node(folder).expect("the folder").visible);

        update(&mut app, Message::SoloEntered(inside));

        assert!(
            app.doc.node(folder).expect("the folder").visible,
            "the folder still masks the row"
        );
        assert_eq!(drawn(&app), vec![inside]);
        assert_renderer_agrees(&app, "soloing inside a hidden folder");
    }

    /// **An eye click on a row solo is not showing is refused**, and nothing
    /// moves.
    ///
    /// Without the refusal the row's open eye is a lie: the click turns a bit
    /// off, the screen does not change because solo was already hiding it, the
    /// user clicks again -- and each of those presses sets `unsaved` and arms an
    /// autosave of a multi-gigabyte document, with the whole effect arriving
    /// later, when they leave the mode.
    #[test]
    fn an_eye_click_outside_the_solo_scope_is_refused_and_changes_nothing() {
        let mut app = app();
        let first = app.doc.active();
        let second = add_body(&mut app, "Body 2", 30.0);
        update(&mut app, Message::SoloEntered(first));
        app.unsaved = false;
        let outline_before = app.doc.outline();
        let entries_before = app.history.stats().undo_entries;

        update(&mut app, Message::BodyVisibilityToggled(second));

        assert_eq!(app.doc.outline(), outline_before, "the refused click wrote an eye anyway");
        assert!(!app.unsaved, "the refused click dirtied the document");
        assert_eq!(
            app.history.stats().undo_entries,
            entries_before,
            "the refused click pushed an entry"
        );
        assert!(app.status.contains("solo"), "the refusal does not name the mode: {}", app.status);
        assert!(app.status.contains("Body 2"), "the refusal does not name the row: {}", app.status);

        // And in scope it goes through, so the guard is not simply off.
        update(&mut app, Message::SoloExited);
        update(&mut app, Message::BodyVisibilityToggled(second));
        assert!(!app.doc.node(second).expect("the body").visible, "the eye stopped working");
    }

    /// **Deleting the soloed row clears solo.** A dangling `Some(id)` shows
    /// nothing at all while the mode is still on, and its only exit is an
    /// indicator with no name to draw.
    #[test]
    fn deleting_the_soloed_body_clears_solo() {
        let mut app = app();
        let second = add_body(&mut app, "Body 2", 30.0);
        update(&mut app, Message::BodySelected(second));
        update(&mut app, Message::SoloEntered(second));
        assert_eq!(app.solo, Some(second));

        update(&mut app, Message::BodyDeleted);

        assert!(app.doc.node(second).is_none(), "the fixture never deleted anything");
        assert_eq!(app.solo, None, "solo is still naming a row that is gone");
        assert!(!drawn(&app).is_empty(), "the document went dark");
        assert_renderer_agrees(&app, "deleting the soloed body");
    }

    /// The same for a folder, and for the routes that remove a row without
    /// anybody calling a delete: undo of a group, and redo of the delete.
    ///
    /// This is why the check rides on the visibility pass rather than sitting in
    /// the delete path. "The places that remove a row" is a list, and a list
    /// goes out of date silently.
    #[test]
    fn every_route_that_removes_the_soloed_row_clears_solo() {
        let mut app = app();
        let inside = add_body(&mut app, "Inside", 30.0);
        update(&mut app, Message::BodySelected(inside));
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.parent_of(inside).expect("the new folder");

        // Dissolving the folder takes the soloed row out from under solo.
        update(&mut app, Message::SoloEntered(folder));
        assert_eq!(app.solo, Some(folder));
        update(&mut app, Message::BodyUngrouped);
        assert!(app.doc.node(folder).is_none(), "the fixture never dissolved the folder");
        assert_eq!(app.solo, None, "dissolving the folder left solo dangling");

        // And the undo of a group, which removes a row with no delete anywhere
        // near it.
        update(&mut app, Message::BodyGrouped);
        let again = app.doc.parent_of(app.doc.active()).expect("the second folder");
        update(&mut app, Message::SoloEntered(again));
        assert_eq!(app.solo, Some(again));
        update(&mut app, Message::Undo);
        assert!(app.doc.node(again).is_none(), "the undo did not remove the folder");
        assert_eq!(app.solo, None, "undoing the group left solo dangling");
    }

    /// **A whole-document swap clears solo outright**, and the reason is
    /// different from the one above: the incoming document numbers its rows from
    /// 1 again, so a stale id does not dangle -- it names a real body and hides
    /// everything else.
    #[test]
    fn resetting_clears_solo_rather_than_soloing_whatever_gets_that_id_next() {
        let mut app = app();
        let second = add_body(&mut app, "Body 2", 30.0);
        update(&mut app, Message::BodySelected(second));
        update(&mut app, Message::SoloEntered(second));
        // `reset_sculpt` rather than the message, so the unsaved-work prompt is
        // not what this test is about.
        app.reset_sculpt();

        assert_eq!(app.solo, None, "the new document is soloing an id from the old one");
        assert_eq!(app.doc.body_count(), 1);
        assert_eq!(drawn(&app).len(), 1, "the reset document is hiding its only body");
        assert_renderer_agrees(&app, "a reset");
    }

    /// **Escape leaves solo, and it does so BEFORE it disarms the cut.**
    ///
    /// Both are modes, and one Escape has to pick. It picks the one whose exit
    /// costs nothing: leaving solo puts every body back on screen and changes
    /// not a byte, where disarming the cut throws away something the user
    /// deliberately armed. The second Escape reaches the cut.
    #[test]
    fn escape_leaves_solo_first_and_the_armed_cut_second() {
        let mut app = app();
        let first = app.doc.active();
        add_body(&mut app, "Body 2", 30.0);
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        update(&mut app, Message::SoloEntered(first));
        assert!(app.tool == Tool::Cut && app.solo.is_some(), "the fixture did not arm both modes");

        update(&mut app, Message::MenuClosed);
        assert_eq!(app.solo, None, "escape did not leave solo");
        assert_eq!(app.tool, Tool::Cut, "escape disarmed the cut on its way past solo");

        update(&mut app, Message::MenuClosed);
        assert_ne!(app.tool, Tool::Cut, "the second escape did not reach the cut");
    }

    /// `ctrl+alt+comma` leaves solo as well as turning every eye on -- a
    /// document with every eye on and solo still on shows one subtree, which is
    /// the opposite of what was asked for.
    ///
    /// It is solo's lesser exit precisely because it IS a document change.
    /// Escape exists so that leaving the mode need not be one.
    #[test]
    fn showing_everything_also_leaves_solo() {
        let mut app = app();
        let first = app.doc.active();
        let second = add_body(&mut app, "Body 2", 30.0);
        update(&mut app, Message::BodyVisibilityToggled(second));
        update(&mut app, Message::SoloEntered(first));

        update(&mut app, Message::EveryBodyShown);

        assert_eq!(app.solo, None, "everything is showing except that solo is still on");
        assert_eq!(drawn(&app).len(), 2, "a body is still missing from the screen");
        assert!(app.status.contains("solo"), "the status does not mention it: {}", app.status);

        // And with nothing hidden it still leaves the mode, which is the arm
        // that returns early.
        update(&mut app, Message::SoloEntered(first));
        update(&mut app, Message::EveryBodyShown);
        assert_eq!(app.solo, None, "the early return skipped the exit");
    }

    /// **Solo never vetoes undo.** The refusal that stops an undo changing a
    /// body the user cannot see is evaluated against `saved_visibility`: it is
    /// about an eye the user set and can see in the panel. A transient view
    /// mode turning ctrl+Z off -- with a message calling a body "hidden" whose
    /// eye is plainly open -- is a different thing wearing the same words.
    #[test]
    fn solo_does_not_veto_an_undo_of_a_body_it_is_hiding() {
        let mut app = app();
        let first = app.doc.active();
        let second = add_body(&mut app, "Body 2", 30.0);

        // A real stroke on the first body, driven the way a user would, so
        // there is a `Change::Bricks` naming it in the history.
        update(&mut app, Message::BodySelected(first));
        let probe = app.surface_under(Vec2::new(640.0, 360.0)).expect("the centre hits the model");
        let before = app.doc.volume(first).expect("a live body").sample_world(probe);
        app.on_pointer(PointerEvent::Moved {
            position: iced::Vector::new(640.0, 360.0),
            size: iced::Vector::new(1280.0, 720.0),
        });
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: iced::Vector::new(640.0, 360.0),
            size: iced::Vector::new(1280.0, 720.0),
        });
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        let after = app.doc.volume(first).expect("a live body").sample_world(probe);
        assert_ne!(after, before, "the fixture carved nothing");

        // Solo the OTHER body, so the one the undo would change is not drawn.
        update(&mut app, Message::BodySelected(second));
        update(&mut app, Message::SoloEntered(second));
        assert!(!drawn(&app).contains(&first), "the fixture is still drawing the body");

        update(&mut app, Message::Undo);

        assert_eq!(
            app.doc.volume(first).expect("a live body").sample_world(probe),
            before,
            "solo refused an undo: {}",
            app.status
        );
    }

    /// **Solo showing nothing is the one state it must never be left in**, and
    /// it is reachable by one keystroke from two directions, so the guarantee
    /// is an invariant on the visibility pass rather than a check at entry.
    ///
    /// Route A is undo. Turning the soloed row's eye on is an ordinary undoable
    /// change and undo is deliberately not vetoed by solo, so ctrl+Z straight
    /// after soloing a hidden row turns that eye back off underneath the mode.
    /// Route B is the soloed row's own eye: it is inside its own subtree, so
    /// the scope guard passes it, and the "there is another visible body" test
    /// that lets the hide through reads `saved_visibility` -- which is right,
    /// and which knows nothing about solo.
    ///
    /// Both end with every row resolving to false: a black viewport, a SOLO
    /// badge naming a row nobody can see and a status line about something
    /// else. Escape recovers it and nothing on screen says so.
    #[test]
    fn soloing_can_never_leave_the_viewport_empty() {
        // Route A: undo the eye that entering solo turned on.
        {
            let mut app = app();
            let second = add_body(&mut app, "Body 2", 30.0);
            update(&mut app, Message::BodyVisibilityToggled(second));
            update(&mut app, Message::SoloEntered(second));
            assert_eq!(drawn(&app), vec![second], "the fixture never entered solo");

            update(&mut app, Message::Undo);

            assert!(!drawn(&app).is_empty(), "undo emptied the viewport: {}", app.status);
            assert_eq!(app.solo, None, "solo is still on with nothing to show");
            assert!(app.status.contains("solo"), "nothing says why: {}", app.status);
            assert_renderer_agrees(&app, "undoing the eye solo turned on");
        }

        // Route B: turn the soloed row's own eye off.
        {
            let mut app = app();
            let first = app.doc.active();
            add_body(&mut app, "Body 2", 30.0);
            update(&mut app, Message::BodySelected(first));
            update(&mut app, Message::SoloEntered(first));
            assert_eq!(drawn(&app), vec![first], "the fixture never entered solo");

            update(&mut app, Message::BodyVisibilityToggled(first));

            assert!(!drawn(&app).is_empty(), "the eye emptied the viewport: {}", app.status);
            assert_eq!(app.solo, None, "solo is still on with nothing to show");
            assert!(app.status.contains("solo"), "nothing says why: {}", app.status);
            assert_renderer_agrees(&app, "hiding the soloed row");
        }
    }

    /// **The renderer is told what solo is doing, after every solo message.**
    ///
    /// `hidden_snapshot()` equals `doc.display_visibility(app.solo)` or a body
    /// is missing from the viewport while its row reads "visible" -- and it
    /// still raycasts and still carves, which is not a failure anyone can
    /// reproduce from a description.
    #[test]
    fn the_renderer_agrees_with_the_document_after_every_solo_message() {
        let mut app = app();
        let first = app.doc.active();
        let second = add_body(&mut app, "Body 2", 30.0);
        let third = add_body(&mut app, "Body 3", 60.0);

        let messages = [
            Message::SoloEntered(second),
            Message::Frame,
            Message::SoloEntered(third),
            Message::BodyVisibilityToggled(third),
            Message::SoloExited,
            Message::BodyVisibilityToggled(first),
            Message::SoloEntered(second),
            Message::EveryBodyShown,
            Message::SoloEntered(first),
            Message::MenuClosed,
            Message::Frame,
        ];
        for message in messages {
            let named = format!("{message:?}");
            update(&mut app, message);
            assert_renderer_agrees(&app, &named);
        }
    }
}

/// The body panel: primitives, the eye, delete, and the element tree that draws
/// them.
///
/// **`panel.rs` had literally zero coverage before this module**, because
/// `view()` was never called by any test in the crate -- so no assertion at any
/// level could see a row panic, a heading vanish, or a widget tree fail to
/// build. Building an `Element` needs no GPU and no window, which is what makes
/// the smoke test below possible at all.
#[cfg(test)]
mod body_panel_tests {
    use super::*;
    use hani_core::{MAX_BODIES, NodeMeta, PrimitiveKind};

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// A body with nothing in it, for the cases that need rows rather than
    /// geometry.
    ///
    /// A real primitive at every one of sixty-four rows is a few hundred
    /// megabytes of bricks to draw a list, and the list does not read a voxel.
    fn cheap_body(app: &mut Hani, name: &str) -> NodeId {
        let volume = Volume::new(app.doc.voxel_size());
        app.doc.add_body(name, volume)
    }

    fn names(app: &Hani) -> Vec<String> {
        app.doc.nodes().iter().map(|node| node.name.clone()).collect()
    }

    /// The whole gesture: press a row, move the pointer to `fraction` of the
    /// way down the row at `over`, and let go.
    ///
    /// **The release is the GLOBAL one** and not the row's, because that is the
    /// one the application commits on -- a test that called `finish_row_drag`
    /// directly would not notice the day the wiring came undone.
    fn drag_row(app: &mut Hani, row: NodeId, over: usize, fraction: f32) {
        update(app, Message::BodySelected(row));
        update(app, Message::BodyRowDragged { over, fraction });
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// Drag `row` in as the FIRST child of `folder`, which is the lower band of
    /// an open folder's own row: the gap under it is already inside it.
    fn drag_into_folder(app: &mut Hani, row: NodeId, folder: NodeId) {
        let over = app.doc.index_of(folder).expect("the folder is in the document");
        drag_row(app, row, over, 0.9);
    }

    /// Drag `row` so it lands directly below `sibling`, at the same depth --
    /// the lower band of a body's row.
    fn drag_below(app: &mut Hani, row: NodeId, sibling: NodeId) {
        let over = app.doc.index_of(sibling).expect("the sibling is in the document");
        drag_row(app, row, over, 0.9);
    }

    /// Drag `row` out to the top level, above everything: the top band of the
    /// very first row, which is always at depth 0.
    fn drag_to_top_level(app: &mut Hani, row: NodeId) {
        drag_row(app, row, 0, 0.1);
    }

    /// Open the tool's own controls the way the user does: a right click in the
    /// viewport that neither moves nor lingers.
    ///
    /// Through the real gesture rather than by setting `menu` directly, so that
    /// the day the click test changes shape this stops opening the menu and the
    /// tree it builds stops being covered -- loudly, because the assertion is
    /// right here.
    fn open_the_tool_menu(app: &mut Hani) {
        // Widget-local pixels, and a size this module has no constant for --
        // the panel tests build no viewport. Anywhere inside it will do: the
        // menu opens where the click was.
        let size = iced::Vector::new(800.0, 600.0);
        let at = iced::Vector::new(size.x / 2.0, size.y / 2.0);
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Right,
                position: at,
                size,
            }),
        );
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Right }));
        assert!(app.menu.is_some(), "the right click did not open the tool menu");
    }

    /// **The waterline.** Press `+`, pick Cube, and there is a second body in
    /// the document that does not overlap the first.
    #[test]
    fn adding_a_primitive_puts_a_new_body_clear_of_everything_already_there() {
        let mut app = app();
        let camera_before = (app.camera.yaw, app.camera.pitch, app.camera.distance);

        update(&mut app, Message::PrimitiveMenuToggled);
        assert!(app.adding, "the plus button did not open the menu");
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));

        assert!(!app.adding, "the menu stayed open after a choice");
        assert_eq!(app.doc.body_count(), 2, "the cube did not become a body");
        assert_eq!(names(&app)[1], "Cube");
        assert_eq!(app.doc.active(), app.doc.nodes()[1].id, "the new body was not selected");
        assert!(app.unsaved, "adding a body did not mark the document unsaved");

        // The decision that makes the feature demonstrate itself: clear of the
        // model, not at the origin inside it.
        assert!(
            app.doc.overlaps().is_empty(),
            "the cube interpenetrates the ball: {:?}",
            app.doc.overlaps()
        );
        let ball = app.doc.nodes()[0].volume().unwrap().surface_bounds().unwrap();
        let cube = app.doc.nodes()[1].volume().unwrap().surface_bounds().unwrap();
        assert!(cube.0.x > ball.1.x, "the cube's surface box overlaps the ball's along X");

        // The camera is something the user set. A tool that re-frames on every
        // add makes a twelve-body layout impossible to build.
        assert_eq!(
            (app.camera.yaw, app.camera.pitch, app.camera.distance),
            camera_before,
            "adding a body moved the camera"
        );
    }

    /// Each of the three, so a new kind cannot be added without a shape.
    #[test]
    fn every_primitive_kind_lands_as_its_own_body() {
        let mut app = app();
        for kind in PrimitiveKind::ALL {
            update(&mut app, Message::PrimitiveAdded(kind));
        }
        assert_eq!(app.doc.body_count(), 4);
        assert_eq!(names(&app)[1..], ["Cube", "Sphere", "Cylinder"]);
        assert!(app.doc.overlaps().is_empty(), "two primitives were placed inside each other");
    }

    /// One `Change::NodeAdded`, so one ctrl+Z removes it and one ctrl+shift+Z
    /// brings it back.
    #[test]
    fn adding_a_primitive_is_one_undo_entry() {
        let mut app = app();
        let before = app.history_stats.undo_entries;
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        assert_eq!(
            app.history_stats.undo_entries,
            before + 1,
            "adding a body was not exactly one entry"
        );

        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), 1, "undo did not remove the primitive");
        update(&mut app, Message::Redo);
        assert_eq!(app.doc.body_count(), 2, "redo did not bring it back");
        assert_eq!(names(&app)[1], "Sphere");
    }

    /// Dynamic radius follows the MODEL changing under the brush, never the
    /// selection changing -- and an add does both at once, which is the case
    /// that catches it.
    #[test]
    fn adding_a_primitive_dirties_only_the_new_body_and_leaves_the_brush_alone() {
        let mut app = app();
        update(&mut app, Message::DynamicRadiusToggled(true));
        let radius = app.brush.radius;
        // Drain whatever the fixture left behind, so what is measured is what
        // the add produced.
        app.remesh_dirty();

        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let added = app.doc.nodes()[1].id;

        // `self.dirty` is the batch the add's own remesh consumed, and it is
        // still there afterwards: every pair in it has to name the new body.
        // The ball's bricks did not change, so not one of them may be in it.
        assert!(!app.dirty.is_empty(), "the add dirtied nothing at all");
        assert!(
            app.dirty.iter().all(|(body, _)| *body == added),
            "the add dirtied bricks belonging to a body it did not touch"
        );
        let mut left = Vec::new();
        app.doc.take_dirty(&mut left);
        assert!(left.is_empty(), "something dirtied bricks after the add: {left:?}");
        assert!(
            app.doc.volume(added).is_some_and(|volume| volume.brick_count() > 0),
            "the cube has no bricks"
        );
        assert_eq!(app.brush.radius, radius, "adding a body resized a Dynamic brush");
    }

    /// A cap is a refusal with a line that names it, never a panic and never a
    /// silent no-op. Nothing the interface builds goes through the reader, so
    /// the reader's clamps do not cover this.
    #[test]
    fn adding_past_the_body_cap_is_refused_and_says_so() {
        let mut app = app();
        while app.doc.body_count() < MAX_BODIES {
            let name = format!("Body {}", app.doc.body_count() + 1);
            cheap_body(&mut app, &name);
        }
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));

        assert_eq!(app.doc.body_count(), MAX_BODIES, "the cap let a body through");
        assert!(
            app.status.contains("could not add") && app.status.contains(&MAX_BODIES.to_string()),
            "the refusal does not name the cap: {}",
            app.status
        );
    }

    /// **The ceiling `MAX_BODIES` is not.** A primitive is sized off the
    /// biggest body in the document, so at a fine voxel the very FIRST one can
    /// be tens of gigabytes; sixty-four rows is not a limit anybody reaches
    /// before the machine does. And the refusal has to land before
    /// `primitive::build`, because `build` is the allocation it is refusing.
    ///
    /// The fixture also pins WHICH pool number is read. The pool here has four
    /// times the vertices this cube needs by `vertices_reserved` and half of
    /// what it needs by the watermark, so a guard that took the reserved figure
    /// -- the substitution `GrowthGuard`'s doc comment exists to forbid -- would
    /// wave it through.
    #[test]
    fn a_primitive_the_pool_cannot_hold_is_refused_before_it_is_built() {
        let mut app = app();
        let (_, half) = hani_core::primitive::placement(&app.doc, MODEL_RADIUS_MM);
        let (_, vertices) =
            hani_core::primitive::cost(PrimitiveKind::Cube, app.doc.voxel_size(), half);
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 0,
            vertices_watermark: (vertices * 2.0) as u64,
            vertex_capacity: (vertices * 2.5) as u64,
            ..Default::default()
        });
        let before = app.doc.body_count();
        let bricks = app.doc.totals().dense_bricks;

        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));

        assert_eq!(app.doc.body_count(), before, "the cube was added anyway");
        assert_eq!(app.doc.totals().dense_bricks, bricks, "something was allocated regardless");
        assert!(
            app.status.contains("could not add a cube") && app.status.contains("mesh pool"),
            "the refusal does not say what ran out: {}",
            app.status
        );
        // Established pattern in this codebase: a refusal names the size that
        // WOULD work, rather than leaving the user to bisect.
        assert!(app.status.contains("mm)"), "the refusal names no workable size: {}", app.status);
    }

    /// Clicking one row's eye must leave the active body exactly where it was.
    ///
    /// This is the message-level half. The other half is that the eye's button
    /// captures the press before the row's `mouse_area` can see it, which is a
    /// widget property and is documented on `Hani::body_row`.
    #[test]
    fn toggling_another_rows_eye_does_not_change_the_active_body() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let first = app.doc.nodes()[0].id;
        let cube = app.doc.nodes()[1].id;
        update(&mut app, Message::BodySelected(cube));

        update(&mut app, Message::BodyVisibilityToggled(first));
        assert!(!app.doc.node(first).unwrap().visible, "the eye did not go off");
        assert_eq!(app.doc.active(), cube, "hiding another row moved the selection");
    }

    /// Hiding the body edits land on has to move the selection somewhere the
    /// user can see, or every press afterwards reports that it is hidden.
    #[test]
    fn hiding_the_active_body_moves_the_selection_to_one_that_is_visible() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let first = app.doc.nodes()[0].id;
        let cube = app.doc.nodes()[1].id;
        assert_eq!(app.doc.active(), cube);

        update(&mut app, Message::BodyVisibilityToggled(cube));
        assert_eq!(app.doc.active(), first, "the selection stayed on a body nobody can see");
        assert!(app.status.contains("is now selected"), "nothing said why: {}", app.status);
    }

    /// ...and when there is nowhere for it to go, the hide is refused rather
    /// than leaving the application with nothing to sculpt.
    #[test]
    fn hiding_the_last_visible_body_is_refused_and_changes_nothing() {
        let mut app = app();
        let only = app.doc.active();
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::BodyVisibilityToggled(only));

        assert!(app.doc.node(only).unwrap().visible, "the last visible body was hidden");
        assert!(!app.unsaved, "a refused hide marked the document unsaved");
        assert_eq!(app.history_stats.undo_entries, entries, "a refused hide recorded an entry");
        assert!(app.status.contains("cannot hide"), "the refusal said nothing: {}", app.status);
    }

    /// `ctrl+comma` acts on the active row, and `ctrl+alt+comma` is the way back
    /// from having hidden things and lost track of what.
    #[test]
    fn the_visibility_chords_toggle_the_active_row_and_reveal_everything() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let first = app.doc.nodes()[0].id;
        let cube = app.doc.nodes()[1].id;

        assert!(
            matches!(
                crate::viewport::shortcut(",", true, false, false),
                Some(Message::ActiveBodyVisibilityToggled)
            ),
            "ctrl+comma spells nothing"
        );
        assert!(
            matches!(
                crate::viewport::shortcut(",", true, false, true),
                Some(Message::EveryBodyShown)
            ),
            "ctrl+alt+comma spells nothing"
        );

        // The active body is the cube, and hiding it moves the selection.
        update(&mut app, Message::ActiveBodyVisibilityToggled);
        assert!(!app.doc.node(cube).unwrap().visible);
        assert_eq!(app.doc.active(), first);

        update(&mut app, Message::EveryBodyShown);
        assert!(
            app.doc.nodes().iter().all(|node| node.visible),
            "ctrl+alt+comma left something hidden"
        );
        // One entry for the reveal, so one ctrl+Z puts the hide back.
        update(&mut app, Message::Undo);
        assert!(!app.doc.node(cube).unwrap().visible, "undoing the reveal did not restore the eye");
    }

    /// Nothing to reveal is said out loud rather than pushed onto the stack: an
    /// empty entry costs the user a real undo.
    #[test]
    fn revealing_when_nothing_is_hidden_records_nothing() {
        let mut app = app();
        let entries = app.history_stats.undo_entries;
        update(&mut app, Message::EveryBodyShown);
        assert_eq!(app.history_stats.undo_entries, entries);
        assert!(app.status.contains("already showing"), "{}", app.status);
    }

    /// Delete takes the active body, and one ctrl+Z brings it back whole --
    /// bricks, name, eye and position in the list.
    #[test]
    fn deleting_a_body_is_undoable() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        let bricks = app.doc.volume(cube).unwrap().brick_count();

        update(&mut app, Message::BodyDeleted);
        assert_eq!(app.doc.body_count(), 1, "the delete did nothing");
        assert!(app.status.contains("deleted Cube"), "{}", app.status);

        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), 2, "undo did not bring the body back");
        assert_eq!(names(&app)[1], "Cube");
        assert_eq!(
            app.doc.volume(cube).map(|volume| volume.brick_count()),
            Some(bricks),
            "the body came back with a different number of bricks"
        );
    }

    /// **Toggling a mirror must rebuild the overlay, not just the flag.**
    ///
    /// `toggle_mirror` set `self.symmetry` and stopped. The planes are drawn by
    /// `cursor::build` through `refresh_overlay`, which nothing called here, so
    /// what stayed on screen was whatever the last refresh had produced --
    /// turning X OFF left its plane drawn, and turning it back ON drew nothing
    /// until some unrelated pointer motion rebuilt the batch. The planes were a
    /// toggle behind.
    ///
    /// **`cursor.rs` already has `only_the_enabled_mirror_planes_are_drawn` and
    /// it passed throughout**, because the geometry function was never wrong --
    /// nothing checked that the hot path calls it. That is this project's
    /// documented failure mode: a repair that is real, correct, unit-tested and
    /// dead. So this asserts on what the RENDERER was handed, not on
    /// `self.symmetry`, which was right the whole time.
    #[test]
    fn toggling_a_mirror_rebuilds_the_planes_the_renderer_holds() {
        let mut app = app();
        // A pointer event first, so there is a settled overlay to compare
        // against rather than an empty one.
        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        let with_x = app.shared.overlay_snapshot();
        assert!(
            app.symmetry_settings().axis(MirrorAxis::X),
            "the fixture did not turn the mirror on"
        );
        assert!(
            !with_x.is_empty(),
            "the renderer holds no overlay at all after a mirror was turned on"
        );

        update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
        let without_x = app.shared.overlay_snapshot();
        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "the fixture did not turn the mirror off"
        );
        assert!(
            without_x.surfaces.len() < with_x.surfaces.len(),
            "the renderer still holds {} overlay surface vertices with the mirror off against {} with \
             it on, so the plane is still being drawn for an axis that is now off",
            without_x.surfaces.len(),
            with_x.surfaces.len(),
        );
    }

    /// **A body added with the Transform tool up must arm, not be called hidden.**
    ///
    /// `arm_gizmo` refuses a body that is not drawn, asking `active_is_drawn`,
    /// which indexes `self.shown` by node position -- and `publish_visibility`
    /// is the only thing that writes `self.shown`. It used to run AFTER the
    /// re-arm, so the message that added a body armed against a `shown` one
    /// entry short: `get(index)` was `None`, `unwrap_or(false)` said hidden,
    /// and the add refused with "that body is hidden -- turn its eye on to move
    /// it" and dropped the user back to Sculpt.
    ///
    /// Found by driving the running application: add a cube with the gizmo up
    /// and the status says the body is hidden while the body is plainly in the
    /// list, visible, with its eye on.
    ///
    /// The tool assertion is the one that matters. A wrong status line is
    /// annoying; being silently thrown out of the mode you were working in is
    /// the actual damage.
    #[test]
    fn adding_a_body_with_the_transform_tool_up_arms_on_it() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Transform));
        assert!(app.gizmo.is_some(), "the fixture never armed on the first body");

        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));

        assert_eq!(
            app.tool,
            Tool::Transform,
            "adding a body threw the user out of the Transform tool: {}",
            app.status
        );
        let armed = app.gizmo.expect("the gizmo should have re-armed on the new body");
        assert_eq!(
            armed.body,
            app.doc.active(),
            "the gizmo armed on something other than the body that was just added"
        );
        assert!(
            !app.status.contains("hidden"),
            "a visible body with its eye on was called hidden: {}",
            app.status
        );
    }

    /// **Undoing an added body must tell the renderer, or it is drawn for ever.**
    ///
    /// Reported from the running application: add a primitive, press ctrl+Z,
    /// and the body leaves the BODIES list and stays on screen. Undo applies
    /// `Change::NodeAdded`, which takes the node back out of the document; its
    /// bricks are then in nobody's dirty set, so the remesh never touches them
    /// and the pool goes on drawing a body nothing owns.
    ///
    /// `remove_body` had always forgotten a body it deleted. The history had no
    /// equivalent, because `UndoOutcome::Applied` names only the FIRST node an
    /// entry touched -- which is a different question from "what did this step
    /// remove".
    ///
    /// **This is the mirror of the bug that shipped**, and it asserts the same
    /// thing that one taught: what the RENDERER was told. A test that checked
    /// only `doc.node(cube).is_none()` -- the DATA -- passes on the broken
    /// build, because the data was never in doubt.
    #[test]
    fn undoing_an_added_body_tells_the_renderer_to_drop_it() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        let survivor = app.doc.nodes()[0].id;
        // Whatever the add queued is not what is being measured.
        let _ = app.shared.take_forgotten_for_tests();

        update(&mut app, Message::Undo);

        assert!(
            app.doc.node(cube).is_none(),
            "the fixture did not actually undo the add, so nothing below is exercised"
        );
        assert_eq!(
            app.shared.take_forgotten_for_tests(),
            vec![cube],
            "undo removed the body from the document and left its slots in the pool, so it is \
             still drawn with nothing in the BODIES list owning it"
        );
        assert!(
            app.dirty.iter().any(|(body, _)| *body == survivor),
            "the undo did not re-offer the surviving body, so a brick the pool refused while \
             it was full stays missing with the banner gone"
        );
    }

    /// And redo must put it back on screen, not merely back in the list.
    #[test]
    fn redoing_an_added_body_draws_it_again() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        update(&mut app, Message::Undo);
        let _ = app.shared.take_forgotten_for_tests();

        update(&mut app, Message::Redo);

        assert!(app.doc.node(cube).is_some(), "redo did not put the body back in the document");
        assert!(
            app.shared.take_forgotten_for_tests().is_empty(),
            "redo told the renderer to forget the body it had just restored"
        );
        assert!(
            app.dirty.iter().any(|(body, _)| *body == cube),
            "the restored body was never offered to the pool, so it is in the list and not on \
             screen -- the same defect the other way round"
        );
    }

    /// **A delete owes the renderer two things, and neither is visible on
    /// screen if it is missing.**
    ///
    /// The forget releases the deleted body's slots -- without it a sliver of
    /// it is drawn forever, holding pool space, with no counter moving. The
    /// whole-document remesh is the other half: `MeshPool::forget_body` clears
    /// the pool-full banner when it frees space, and a brick the pool refused
    /// while it was full is long gone from the dirty set, so freeing the space
    /// without re-offering everything takes the warning down and leaves the
    /// geometry missing.
    #[test]
    fn a_delete_tells_the_renderer_to_drop_the_body_and_re_offers_everything_left() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        let survivor = app.doc.nodes()[0].id;
        // Whatever the add queued is not what is being measured.
        let _ = app.shared.take_forgotten_for_tests();

        update(&mut app, Message::BodyDeleted);

        assert_eq!(
            app.shared.take_forgotten_for_tests(),
            vec![cube],
            "the deleted body's slots were left in the pool"
        );
        assert!(
            app.dirty.iter().any(|(body, _)| *body == survivor),
            "the delete did not re-offer the surviving body, so a brick the pool refused while \
             it was full stays missing with the banner gone"
        );
    }

    /// **A primitive placed off the origin has its own symmetry, off, about
    /// its own middle.**
    ///
    /// While every plane went through [`MIRROR_CENTRE`], a dent carved into a
    /// body at x = +80 had its twin written at x = -80, free-floating geometry
    /// in empty space, and selecting the new cube turned the mirror off and
    /// said why. Since symmetry became the body's own (`app/symmetry_frame.rs`)
    /// the cube starts with its planes off, through its own middle, so a mirror
    /// turned on for it lands on it; and the ball keeps its X.
    #[test]
    fn adding_a_primitive_clear_of_the_model_gives_it_its_own_symmetry() {
        let mut app = app();
        let ball = app.doc.active();
        update(&mut app, Message::SymmetryAxisToggled(hani_core::MirrorAxis::X));
        assert!(app.symmetry_settings().axis(hani_core::MirrorAxis::X), "the ball straddles X");

        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));

        assert!(app.symmetry_settings().is_off(), "the cube started with the ball's symmetry");
        assert!(app.doc.symmetry(ball).axis(hani_core::MirrorAxis::X), "the ball lost its X");
        update(&mut app, Message::SymmetryAxisToggled(hani_core::MirrorAxis::X));
        assert!(
            app.symmetry_settings().axis(hani_core::MirrorAxis::X),
            "X was refused on a cube whose own plane runs through it: {}",
            app.status
        );
        let (low, high) = app.doc.active_volume().surface_bounds().expect("the cube has a surface");
        let plane = app.framed_symmetry().frame().origin + MIRROR_CENTRE;
        assert!(low.x < plane.x && plane.x < high.x, "the plane at {plane} misses the cube");
        assert!(low.x > 0.0, "the fixture's cube is no longer clear of the room's plane");
    }

    /// A document always holds one body, so the last one cannot go. Refused
    #[test]
    fn deleting_the_last_body_is_refused() {
        let mut app = app();
        update(&mut app, Message::BodyDeleted);
        assert_eq!(app.doc.body_count(), 1);
        assert!(app.status.contains("cannot delete the last body"), "{}", app.status);
        assert!(app.pending_delete.is_none(), "the last body raised a prompt");
    }

    /// A delete big enough that undo may not hold it asks first, names the size,
    /// and on Cancel changes nothing.
    ///
    /// The threshold is a field rather than the constant read in place
    /// precisely so this test can exist: at the real 512 MB the fixture would
    /// have to allocate half a gigabyte of bricks.
    #[test]
    fn a_large_delete_asks_first_and_cancelling_changes_nothing() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        let bytes = app.doc.volume(cube).unwrap().stats().resident_bytes;
        assert!(bytes > 0, "the fixture body costs nothing, so nothing can be over the threshold");
        app.delete_prompt_bytes = bytes;

        update(&mut app, Message::BodyDeleted);
        let pending = app.pending_delete.as_ref().expect("a large delete did not ask");
        assert_eq!(pending.id, cube);
        assert_eq!(pending.bytes, bytes);
        assert_eq!(app.doc.body_count(), 2, "the prompt deleted the body anyway");
        assert!(app.modal_open(), "the delete prompt is not modal, so a press behind it sculpts");

        update(&mut app, Message::BodyDeleteCancelled);
        assert_eq!(app.doc.body_count(), 2, "cancelling deleted the body");
        assert!(app.pending_delete.is_none());

        // ...and confirming goes through.
        update(&mut app, Message::BodyDeleted);
        update(&mut app, Message::BodyDeleteConfirmed);
        assert_eq!(app.doc.body_count(), 1, "confirming did not delete");
    }

    // --- merge down ------------------------------------------------------------

    /// The active body, the one below it, and the upper of the two selected --
    /// which is the only arrangement merge down does anything in.
    ///
    /// Two primitives rather than two `cheap_body` rows: a merge with no voxels
    /// on either side would pass every assertion about the list while doing
    /// nothing to a field.
    fn two_to_merge() -> (Hani, NodeId, NodeId) {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let upper = app.doc.nodes()[0].id;
        let lower = app.doc.nodes()[1].id;
        update(&mut app, Message::BodySelected(upper));
        // Adding the cube dirtied the document; the merge under test is what
        // every `unsaved` assertion below is about.
        app.unsaved = false;
        (app, upper, lower)
    }

    /// **The whole feature from the user's chair**: press the button and the two
    /// bodies are one, where the upper one stood, holding both fields.
    #[test]
    fn merging_down_consumes_the_source_and_leaves_the_target_where_it_stood() {
        let (mut app, upper, lower) = two_to_merge();
        let upper_bricks = app.doc.volume(upper).expect("a field").brick_count();
        let lower_bricks = app.doc.volume(lower).expect("a field").brick_count();

        update(&mut app, Message::BodyMergedDown);

        assert_eq!(app.doc.body_count(), 1, "the merge did nothing: {}", app.status);
        assert!(app.doc.node(upper).is_none(), "the source row survived");
        assert_eq!(app.doc.index_of(lower), Some(0), "the result is not where the source stood");
        assert_eq!(app.doc.active(), lower, "the result was not selected");
        assert!(app.unsaved, "merging did not mark the document unsaved");
        // The two primitives are placed clear of one another, so the union holds
        // every brick of both.
        assert_eq!(
            app.doc.volume(lower).expect("a field").brick_count(),
            upper_bricks + lower_bricks,
            "the merged body is not the union of the two"
        );
        assert!(app.status.contains("bricks changed"), "the merge said nothing: {}", app.status);
    }

    /// One ctrl+Z, and both bodies are back. A merge is one gesture, so half of
    /// it coming back would be a document nothing downstream is written for.
    #[test]
    fn one_undo_takes_a_merge_apart_again() {
        let (mut app, upper, lower) = two_to_merge();
        let before = app.doc.volume(lower).expect("a field").brick_count();
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::BodyMergedDown);
        assert_eq!(app.history_stats.undo_entries, entries + 1, "a merge is not one entry");

        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), 2, "undo did not bring the consumed body back");
        assert_eq!(names(&app), ["Body 1", "Cube"], "the row came back in the wrong place");
        assert_eq!(
            app.doc.volume(lower).expect("a field").brick_count(),
            before,
            "undo left the target holding the source's bricks"
        );
        assert!(app.doc.node(upper).is_some(), "the source row did not come back");
    }

    /// The bottom of a list has nothing below it. Refused by name, with the
    /// document untouched -- never a button that silently does nothing.
    #[test]
    fn merging_the_bottom_body_is_refused_by_name_and_changes_nothing() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let entries = app.history_stats.undo_entries;
        app.unsaved = false;

        update(&mut app, Message::BodyMergedDown);

        assert_eq!(app.doc.body_count(), 2, "the bottom body merged into something");
        assert_eq!(app.history_stats.undo_entries, entries, "a refused merge recorded an entry");
        assert!(!app.unsaved, "a refused merge dirtied the document");
        assert!(
            app.status.contains("could not merge Cube") && app.status.contains("no body below"),
            "the refusal does not say why: {}",
            app.status
        );
    }

    /// A folder on the next line is not a body. Merging into a container is
    /// ZBrush's MergeVisible and its universal "the button did nothing"
    /// reaction; naming the folder is the whole difference.
    #[test]
    fn merging_into_a_folder_below_is_refused_and_names_it() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        // Wrap the lower body, so a folder row sits directly below the upper.
        update(&mut app, Message::BodyGrouped);
        let upper = app.doc.nodes()[0].id;
        update(&mut app, Message::BodySelected(upper));

        update(&mut app, Message::BodyMergedDown);

        assert_eq!(app.doc.body_count(), 2, "a body was merged into a folder");
        assert!(
            app.status.contains("Group 1 is a folder"),
            "the refusal does not name the folder: {}",
            app.status
        );
    }

    /// A merge whose one entry is bigger than history can be relied on to keep
    /// asks first, names the size, and on Cancel changes nothing.
    ///
    /// The threshold is a field rather than the constant read in place for the
    /// reason the delete prompt's own test gives: at the real 512 MB the fixture
    /// would have to allocate half a gigabyte of bricks.
    #[test]
    fn a_large_merge_asks_first_and_cancelling_changes_nothing() {
        let (mut app, upper, _) = two_to_merge();
        let plan = app.doc.merge_plan(upper).expect("there is a body below");
        assert!(plan.bytes() > 0, "the fixture costs nothing, so nothing can be over a threshold");
        app.merge_prompt_bytes = plan.bytes();

        update(&mut app, Message::BodyMergedDown);
        let pending = app.pending_merge.as_ref().expect("a large merge did not ask");
        assert_eq!(pending.source, upper);
        assert_eq!(pending.bytes, plan.bytes());
        assert_eq!(pending.stroke_bytes, plan.stroke_bytes);
        assert_eq!(pending.reclaim_bytes, plan.reclaim_bytes);
        assert_eq!(app.doc.body_count(), 2, "the prompt merged anyway");
        assert!(app.modal_open(), "the merge prompt is not modal, so a press behind it sculpts");

        update(&mut app, Message::BodyMergeCancelled);
        assert_eq!(app.doc.body_count(), 2, "cancelling merged the bodies");
        assert!(!app.unsaved, "cancelling dirtied the document");
        assert!(app.pending_merge.is_none());

        // Escape is the other way out, and it means the same harmless thing.
        update(&mut app, Message::BodyMergedDown);
        assert!(app.pending_merge.is_some());
        update(&mut app, Message::MenuClosed);
        assert!(app.pending_merge.is_none(), "escape left the merge prompt up");
        assert_eq!(app.doc.body_count(), 2, "escape merged the bodies");

        // ...and confirming goes through.
        update(&mut app, Message::BodyMergedDown);
        update(&mut app, Message::BodyMergeConfirmed);
        assert_eq!(app.doc.body_count(), 1, "confirming did not merge");
    }

    /// **A merge owes the renderer the same two things a delete does**, and for
    /// the same reasons: the consumed body's slots have to be released, or a
    /// sliver of it is drawn forever holding pool space, and everything left has
    /// to be re-offered, or a brick the pool refused while it was full stays
    /// missing after the banner comes down.
    #[test]
    fn a_merge_tells_the_renderer_to_drop_the_consumed_body_and_re_offers_the_rest() {
        let (mut app, upper, lower) = two_to_merge();
        // Whatever the add and the selection queued is not what is measured.
        let _ = app.shared.take_forgotten_for_tests();

        update(&mut app, Message::BodyMergedDown);

        assert_eq!(
            app.shared.take_forgotten_for_tests(),
            vec![upper],
            "the consumed body's slots were left in the pool"
        );
        assert!(
            app.dirty.iter().any(|(body, _)| *body == lower),
            "the merge did not re-offer the surviving body"
        );
    }

    /// Merging while soloed keeps the mode. **`rebuild_everything` clears solo
    /// and the plan for this increment asked for it; that call was written
    /// before `forget_body` landed and it would drop a view mode a merge has no
    /// business touching.** Solo is cleared only when the row it names stops
    /// existing, which `forget_a_vanished_solo` already does on the update pass.
    #[test]
    fn merging_a_body_that_is_not_soloed_leaves_solo_alone() {
        let (mut app, upper, lower) = two_to_merge();
        update(&mut app, Message::SoloEntered(upper));
        assert_eq!(app.solo, Some(upper), "the fixture never entered solo");

        // Solo the target, then merge the source into it, so the mode names a
        // row that survives.
        update(&mut app, Message::BodySelected(lower));
        update(&mut app, Message::SoloEntered(lower));
        update(&mut app, Message::BodySelected(upper));
        assert_eq!(app.solo, Some(lower), "the fixture is not soloing the survivor");

        update(&mut app, Message::BodyMergedDown);

        assert_eq!(app.doc.body_count(), 1, "the merge did nothing: {}", app.status);
        assert_eq!(app.solo, Some(lower), "the merge dropped solo");
    }

    /// And the other half of the same rule: merging the SOLOED body away leaves
    /// the mode, because a solo naming a row that no longer exists shows nothing
    /// at all while its only exit has no name to draw. It is
    /// `forget_a_vanished_solo` on the update pass that does it, which is where
    /// increment 13 put every "the row stopped existing" case by name --
    /// including this one, before it could be written.
    #[test]
    fn merging_the_soloed_body_away_leaves_solo() {
        let (mut app, upper, _) = two_to_merge();
        update(&mut app, Message::SoloEntered(upper));
        assert_eq!(app.solo, Some(upper), "the fixture never entered solo");

        update(&mut app, Message::BodyMergedDown);

        assert_eq!(app.doc.body_count(), 1, "the merge did nothing: {}", app.status);
        assert!(app.solo.is_none(), "solo still names a row that was merged away");
    }

    /// The thumbnail switch is session state. Nothing about it is written to the
    /// file, so by the rule that governs that it must not dirty the document.
    ///
    /// It also pins the DEFAULT, which is ON. That was the deliberate act this
    /// assertion existed to force: the go/no-go bench ran on a quiet machine on
    /// 2026-08-26 and read 8.03, 7.89 and 7.88 ms against an 8.0 ms budget, so
    /// the median clears it and the single failure is 0.4% over. The figure is
    /// also a worst case rather than a typical one -- extrapolated to 45,567
    /// bricks, the largest model the pool can hold, where the bench's own 5,435
    /// brick model costs 0.94 to 0.96 ms.
    ///
    /// **An earlier reading defaulted it off** on the grounds that the bench
    /// measures a SCULPTED sphere while the application is "really" for
    /// imported scans, which carry several times more geometry per brick. That
    /// premise was wrong -- this is a sculpting tool first -- so the sculpted
    /// anchor is the representative measurement rather than the flattering one.
    #[test]
    fn the_thumbnail_switch_does_not_dirty_the_document() {
        let mut app = app();
        assert!(
            app.thumbnails,
            "the pictures default OFF, but the bench cleared them and Thomas asked for them on"
        );
        update(&mut app, Message::ThumbnailsToggled);
        assert!(!app.thumbnails);
        assert!(!app.unsaved, "turning the pictures off marked the sculpt unsaved");
        update(&mut app, Message::ThumbnailsToggled);
        assert!(app.thumbnails);
        assert!(!app.unsaved, "turning the pictures on marked the sculpt unsaved");
    }

    // --- duplicate -----------------------------------------------------------

    /// **The whole gesture from the user's chair**: the copy appears directly
    /// under the row it came from, carrying its field, and becomes the body
    /// edits land on.
    ///
    /// Every one of the four assertions about naming and selection is ZBrush
    /// inverted. There, duplicating "object" renames the ORIGINAL to "object1",
    /// hands the copy the original's name, and leaves the original selected --
    /// which users had to work out from their own undo history, and which
    /// breaks GoZ round-trips because there the name is the identity key.
    #[test]
    fn a_duplicate_lands_directly_below_its_original_and_becomes_the_active_body() {
        let mut app = app();
        let original = app.doc.active();
        let bricks = app.doc.volume(original).expect("the default body").brick_count();
        // A third row, so "directly below" is a real claim rather than "at the
        // end of the list" wearing a different name.
        cheap_body(&mut app, "Last");

        update(&mut app, Message::BodyDuplicated);

        assert_eq!(app.doc.body_count(), 3, "the copy did not become a body");
        assert_eq!(names(&app), ["Body 1", "Body 1 copy", "Last"]);
        let copy = app.doc.nodes()[1].id;
        assert_ne!(copy, original, "the copy took the original's id");
        assert_eq!(app.doc.active(), copy, "the copy did not become the active body");
        assert!(app.unsaved, "duplicating did not mark the document unsaved");
        assert_eq!(
            app.doc.volume(copy).expect("the copy holds a field").brick_count(),
            bricks,
            "the copy does not hold the original's field"
        );
        assert_eq!(
            app.doc.volume(original).expect("the original is still here").brick_count(),
            bricks,
            "duplicating moved the original's bricks instead of copying them"
        );
    }

    /// **The same button pressed on a row that lives inside a folder**, which
    /// is the case the panel's own smoke test cannot see: there the active row
    /// happens to be at the top level when duplicate runs, so a copy that lands
    /// at depth 0 looks exactly like a copy that lands beside its source.
    ///
    /// It is not the same. A depth-0 copy in the middle of a folder's preorder
    /// run ends that run at the copy, so every sibling below it leaves the
    /// folder -- silently in a release build, and as a failed fold in a debug
    /// one. The sibling is here for precisely that reason: without a row after
    /// the copy the mistake is invisible.
    #[test]
    fn a_duplicate_made_inside_a_folder_stays_inside_it_and_keeps_its_siblings() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.parent_of(cube).expect("the new folder");
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        drag_below(&mut app, sphere, cube);
        update(&mut app, Message::BodySelected(cube));
        assert_eq!(
            app.doc.subtree_body_count(folder),
            2,
            "the fixture is not the shape under test"
        );

        update(&mut app, Message::BodyDuplicated);

        let copy = app.doc.active();
        assert_ne!(copy, cube, "nothing was duplicated: {}", app.status);
        assert_eq!(app.doc.parent_of(copy), Some(folder), "the copy landed outside the folder");
        assert_eq!(
            app.doc.parent_of(sphere),
            Some(folder),
            "the sibling below the copy fell out of the folder"
        );
        assert_eq!(app.doc.subtree_body_count(folder), 3);
    }

    /// **A body that never reaches the GPU is in the document, exports
    /// correctly, and is invisible for the rest of the session.**
    ///
    /// `perf.dirty_bricks` and deliberately not `perf.remesh_ms`: the remesh
    /// returns early on an empty dirty set without writing the timing, so a
    /// check written against the milliseconds passes on exactly the broken
    /// build it exists to catch.
    #[test]
    fn the_copy_reaches_the_gpu() {
        let mut app = app();
        update(&mut app, Message::BodyDuplicated);

        let copy = app.doc.active();
        let bricks = app.doc.volume(copy).expect("the copy holds a field").brick_count();
        assert!(bricks > 0, "the fixture body has no bricks, so this asserts nothing");
        assert!(
            app.perf.dirty_bricks >= bricks,
            "the copy holds {bricks} bricks and the remesh saw only {}",
            app.perf.dirty_bricks
        );
    }

    /// Duplicate, then ctrl+Z, and the file that would be written is the file
    /// that would have been written before -- byte for byte.
    ///
    /// Byte-identical rather than "the row is gone", because a row removed
    /// while the document's id counter, its active row or its node order moved
    /// underneath is a document that reads back as something else. The file is
    /// the only place all of that is visible at once.
    ///
    /// `assert!` and not `assert_eq!` throughout: the fixture's brick stream
    /// runs to megabytes, and a failure that prints both copies of it is a
    /// failure nobody can read.
    #[test]
    fn duplicating_and_undoing_leaves_the_document_byte_identical() {
        let mut app = app();
        let before = written(&app);

        update(&mut app, Message::BodyDuplicated);
        assert_eq!(app.doc.body_count(), 2, "the fixture did not actually duplicate");
        assert!(written(&app) != before, "the copy left no trace in the file");

        update(&mut app, Message::Undo);
        let after = written(&app);
        assert!(
            after == before,
            "undoing the duplicate did not restore the document: {} bytes against {}",
            after.len(),
            before.len()
        );
    }

    /// **The residual, said out loud: undoing a duplicate of a row that is not
    /// the last one leaves the selection on the row BELOW, not on the original.**
    ///
    /// It is `Document::remove`'s documented policy and not an accident there:
    /// a row taken out of the list is replaced on screen by the one below it,
    /// which is the right answer for the delete that policy was written for,
    /// and its header says restoring the selection is the business of whatever
    /// built the undo entry. `Change::NodeAdded` carries no room to do that --
    /// it is a position and an id, and the plan's own budget for it is "about
    /// 8 bytes of history".
    ///
    /// Fixing it means widening that variant with the row that was active
    /// before the add, and giving its inverse the symmetric behaviour on redo,
    /// which changes what undoing a DELETE selects as well. That is a change to
    /// the undo model rather than to duplicate, so it is named here instead of
    /// smuggled in: when someone makes it, this test fails and this comment is
    /// what they should read.
    ///
    /// Nothing about the geometry is at stake -- the document is otherwise
    /// restored exactly, which the test above pins.
    #[test]
    fn undoing_a_duplicate_from_the_middle_of_the_list_leaves_the_selection_below_it() {
        let mut app = app();
        let original = app.doc.active();
        let last = cheap_body(&mut app, "Last");

        update(&mut app, Message::BodyDuplicated);
        update(&mut app, Message::Undo);

        assert_eq!(names(&app), ["Body 1", "Last"], "the copy is still in the document");
        assert_eq!(
            app.doc.active(),
            last,
            "the selection no longer lands below the removed row -- read this test's header"
        );
        assert_ne!(app.doc.active(), original);
    }

    /// The document a save would write right now.
    fn written(app: &Hani) -> Vec<u8> {
        let mut bytes = Vec::new();
        hani_core::project::write(&mut bytes, &app.doc, &hani_core::ProjectState::default())
            .expect("writing the sculpt failed");
        bytes
    }

    /// At the cap, duplicate is a refusal with the number in it -- never a
    /// panic and never a silent no-op. The reader's own clamps do not cover
    /// this: nothing built by the interface goes through the reader.
    #[test]
    fn duplicate_at_the_body_cap_is_refused_by_name_and_changes_nothing() {
        let mut app = app();
        while app.doc.body_count() < MAX_BODIES {
            let name = format!("Body {}", app.doc.body_count() + 1);
            cheap_body(&mut app, &name);
        }
        let active = app.doc.active();
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::BodyDuplicated);

        assert_eq!(app.doc.body_count(), MAX_BODIES, "a body was added past the cap");
        assert_eq!(app.doc.active(), active, "the refusal moved the selection");
        assert_eq!(app.history_stats.undo_entries, entries, "the refusal pushed an undo entry");
        assert!(!app.unsaved, "a refusal marked the document unsaved");
        assert!(
            app.status.contains("could not duplicate") && app.status.contains("64"),
            "the refusal does not name the cap: {}",
            app.status
        );
    }

    /// A name at the file format's limit still duplicates, and what comes back
    /// is a name the file can hold rather than one it will repair to "Body 1"
    /// on the next open.
    ///
    /// The " copy" is what gets cut, and that is the right end to lose: the
    /// alternative is a copy whose name is not a prefix of anything the user
    /// typed. Two rows may share a name -- `NodeId` is the identity and nothing
    /// downstream keys off one.
    #[test]
    fn a_copy_of_a_full_length_name_still_fits_the_file() {
        let mut app = app();
        let id = app.doc.active();
        let full = "0123456789abcdef0123456789abcdef";
        assert_eq!(full.len(), hani_core::MAX_NAME_BYTES);
        let meta = NodeMeta { name: full.to_string(), ..app.doc.meta(id).expect("the row") };
        app.doc.set_meta(&meta);

        update(&mut app, Message::BodyDuplicated);

        let copy = app.doc.active();
        let name = app.doc.node(copy).expect("the copy").name.clone();
        assert_eq!(name.len(), hani_core::MAX_NAME_BYTES);
        assert_eq!(name, full, "the copy's name is not what the file will hold");
    }

    /// **A copy the pool cannot hold is refused before one brick is copied**,
    /// and the refusal names no size, because a copy has none to offer.
    ///
    /// The 765 MB dragon is 6,120 dense bricks and 1.53 GiB of memory traffic
    /// (`Volume::duplicated`), so a refusal that arrives after the allocation
    /// is not a refusal.
    ///
    /// **The copy counter is what asserts that half, and it is not decoration.**
    /// The document's body count and brick total do NOT distinguish the two
    /// orderings: a copy that is built and then dropped on the refusal never
    /// enters the document either, so both totals hold whichever side of the
    /// guard the allocation happens on. Hoisting `Volume::duplicated` above the
    /// guard -- which reads as a tidy-up, because it would let `bytes` be
    /// measured off the copy -- passes every other assertion here.
    ///
    /// The fixture also pins WHICH pool number the guard reads, in both
    /// directions. The copy needs 4M vertices; the pool has 2M behind its
    /// watermark and 6M behind its live reservation. A guard taking
    /// `capacity - reserved` -- the substitution `GrowthGuard`'s header exists
    /// to forbid, and one this project has shipped twice -- would wave this
    /// through and then overflow, because a duplicate empties nothing and it is
    /// the bump pointer that runs out.
    #[test]
    fn a_copy_the_pool_cannot_hold_is_refused_before_a_brick_is_copied() {
        let mut app = app();
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 4_000_000,
            vertices_watermark: 8_000_000,
            vertex_capacity: 10_000_000,
            ..Default::default()
        });
        let bodies = app.doc.body_count();
        let bricks = app.doc.totals().dense_bricks;
        let copies = hani_core::volume::copies_made_on_this_thread();

        update(&mut app, Message::BodyDuplicated);

        assert_eq!(
            hani_core::volume::copies_made_on_this_thread(),
            copies,
            "the field was copied and only then refused -- read this test's header"
        );
        assert_eq!(app.doc.body_count(), bodies, "the copy was made anyway");
        assert_eq!(app.doc.totals().dense_bricks, bricks, "something was allocated regardless");
        assert!(!app.unsaved, "a refusal marked the document unsaved");
        assert!(
            app.status.contains("could not duplicate Body 1")
                && app.status.contains("mesh pool")
                && app.status.contains("2.0M left"),
            "the refusal does not say what ran out and how much is left: {}",
            app.status
        );
        assert!(
            !app.status.contains('%'),
            "duplicate has no size lever, so the refusal must offer no size: {}",
            app.status
        );
    }

    /// **The vertex ceiling judges the BODY being copied, not the document.**
    ///
    /// Every other duplicate test runs on a one-body document, where the body's
    /// share of the document's resident bytes is degenerately 1.0 and the
    /// apportionment in `no_room_to_duplicate` is invisible -- a constant 1.0,
    /// an inverted ratio, or no apportionment at all all pass. This one is the
    /// only thing that reads it, so read its header before touching that line.
    ///
    /// The fixture is two bodies of deliberately different size and ONE pool,
    /// with the headroom set between what the two copies cost. The small one's
    /// copy must be admitted and the large one's refused. The consequences of
    /// each way of getting it wrong:
    ///
    /// * a constant share judges a small body's copy against the WHOLE
    ///   document's reservation and refuses it, quoting a vertex figure many
    ///   times too large -- "it needs about 8M vertices" for a body that needs
    ///   one;
    /// * an inverted share admits a copy that overflows the pool, and an
    ///   overflow is silently missing geometry, which is the whole reason
    ///   `GrowthGuard` exists.
    ///
    /// The large body is refused FIRST and the small one second, because
    /// admitting a copy grows `doc_stats` and every share after it moves. The
    /// order is the fixture, not a preference.
    #[test]
    fn the_vertex_ceiling_is_apportioned_to_the_body_and_not_to_the_document() {
        let mut app = app();
        let mut large = Volume::new(1.0);
        large.seed_sphere(Vec3::ZERO, 24.0);
        let mut small = Volume::new(1.0);
        // Inside a single brick: the point of the fixture is that the two
        // bodies cost visibly different amounts.
        small.seed_sphere(Vec3::new(112.0, 16.0, 16.0), 3.0);
        let mut doc = Document::from_volume(large);
        let small_id = doc.add_body("Small", small);
        let large_id = doc.nodes()[0].id;
        app.doc = doc;
        app.rebuild_everything();

        let bytes = |app: &Hani, id| app.doc.volume(id).expect("the body").stats().resident_bytes;
        let large_bytes = bytes(&app, large_id) as f64;
        let small_bytes = bytes(&app, small_id) as f64;
        let total = app.doc_stats.resident_bytes as f64;
        assert!(
            small_bytes * 3.0 < large_bytes,
            "the fixture's two bodies must differ enough to tell their shares apart: \
             {small_bytes} against {large_bytes}"
        );

        // Halfway between the two copies' costs, so that exactly one of them
        // fits and the test says which by arithmetic rather than by a constant
        // someone would have to re-derive.
        let reserved: u64 = 8_000_000;
        let cost = |body: f64| reserved as f64 * (body / total);
        let headroom = (cost(small_bytes) + cost(large_bytes)) / 2.0;
        assert!(cost(small_bytes) < headroom && headroom < cost(large_bytes));
        let capacity: u64 = 20_000_000;
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: reserved,
            vertices_watermark: capacity - headroom as u64,
            vertex_capacity: capacity,
            ..Default::default()
        });

        update(&mut app, Message::BodySelected(large_id));
        update(&mut app, Message::BodyDuplicated);
        assert_eq!(app.doc.body_count(), 2, "the large body's copy did not fit and was made");
        assert!(
            app.status.contains("could not duplicate") && app.status.contains("mesh pool"),
            "the large body's copy was refused for the wrong reason: {}",
            app.status
        );

        update(&mut app, Message::BodySelected(small_id));
        update(&mut app, Message::BodyDuplicated);
        assert_eq!(
            app.doc.body_count(),
            3,
            "the small body's copy fits in the same pool and was refused: {}",
            app.status
        );
    }

    /// A cut drawn across the viewport crosses every body the user can see and
    /// leaves the hidden one bit-identical, in ONE undo entry.
    #[test]
    fn a_cut_leaves_a_hidden_body_it_crosses_untouched() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();

        let mut second = Volume::new(app.doc.voxel_size());
        second.seed_sphere(Vec3::new(10.0, 0.0, 0.0), MODEL_RADIUS_MM * 0.5);
        second.mark_everything_dirty();
        let other = app.doc.add_body("Body 2", second);
        let meta = NodeMeta { visible: false, ..app.doc.meta(other).unwrap() };
        app.doc.set_meta(&meta);
        app.publish_visibility();
        app.remesh_dirty();

        let probe = Vec3::new(10.0, 12.0, 0.0);
        let before = app.doc.volume(other).unwrap().sample_world(probe);
        let bricks = app.doc.volume(other).unwrap().brick_count();
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: iced::Vector::new(80.0, 300.0),
            size: iced::Vector::new(800.0, 600.0),
        });
        app.on_pointer(PointerEvent::Moved {
            position: iced::Vector::new(720.0, 300.0),
            size: iced::Vector::new(800.0, 600.0),
        });
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });

        assert_eq!(
            app.doc.volume(other).unwrap().sample_world(probe),
            before,
            "the cut went through a body nobody can see"
        );
        assert_eq!(app.doc.volume(other).unwrap().brick_count(), bricks);
        assert_eq!(
            app.history_stats.undo_entries,
            entries + 1,
            "one gesture became more than one undo entry"
        );
    }

    /// **The panel's smoke test.** Build the element tree after every operation,
    /// at every list length that matters, with the pictures on and off.
    ///
    /// It asserts nothing about what the tree looks like, on purpose: what it
    /// catches is a row that panics, a widget built from an index that is no
    /// longer there, and a `view()` that stops compiling against its own state.
    /// Before this existed, `panel.rs` had no coverage of any kind.
    ///
    /// The folder steps are here for the same reason the rename ones are: a
    /// folder row is a DIFFERENT shape from a body row -- a chevron and a count
    /// and a trash where the thumbnail goes -- and a collapsed one changes
    /// which rows are built at all.
    #[test]
    fn the_widget_tree_builds_after_every_operation_at_every_size() {
        type Step = (&'static str, fn(&mut Hani));
        let steps: [Step; 39] = [
            ("add a cube", |app| update(app, Message::PrimitiveAdded(PrimitiveKind::Cube))),
            ("add a sphere", |app| update(app, Message::PrimitiveAdded(PrimitiveKind::Sphere))),
            ("add a cylinder", |app| update(app, Message::PrimitiveAdded(PrimitiveKind::Cylinder))),
            ("open the add menu", |app| update(app, Message::PrimitiveMenuToggled)),
            ("select the first row", |app| {
                let first = app.doc.nodes()[0].id;
                update(app, Message::BodySelected(first));
            }),
            ("toggle an eye", |app| {
                let last = app.doc.nodes().last().unwrap().id;
                update(app, Message::BodyVisibilityToggled(last));
            }),
            ("reveal everything", |app| update(app, Message::EveryBodyShown)),
            // Solo changes three things about the tree: the badge beside the
            // section heading, the circle on the soloable rows, and the dimmed
            // background on every row outside the scope. It stays on for the
            // three rename steps below, so all three are built.
            ("solo the active row", |app| {
                let active = app.doc.active();
                update(app, Message::SoloEntered(active));
            }),
            // Three steps and not one: the tree has to be built with a rename
            // field OPEN, which is a different row shape from every other
            // state here, and then again after both ways out of it.
            ("start a rename", |app| {
                let last = app.doc.nodes().last().unwrap().id;
                update(app, Message::BodyRenameBegan(last));
            }),
            ("type into the rename field", |app| {
                update(app, Message::BodyRenameEdited("Renamed".to_string()));
            }),
            ("commit the rename", |app| update(app, Message::BodyRenameSubmitted)),
            ("leave solo", |app| update(app, Message::SoloExited)),
            ("group", |app| update(app, Message::BodyGrouped)),
            ("group again, to nest", |app| update(app, Message::BodyGrouped)),
            // A FOLDER carrying the circle, which is a different row shape from
            // the body that carried it above -- and it stays on across the
            // select below, which moves the active row OUT of the scope and is
            // the one state where a row is dimmed and marked at once.
            ("solo the folder holding the active row", |app| {
                if let Some(folder) = app.doc.parent_of(app.doc.active()) {
                    update(app, Message::SoloEntered(folder));
                }
            }),
            // Duplicate while the active row is NESTED, which the "duplicate"
            // step near the end of this list cannot cover: by then the row is
            // back at the top level, where a copy at the wrong depth is
            // indistinguishable from a copy at the right one.
            ("duplicate a nested body", |app| {
                let parent = app.doc.parent_of(app.doc.active());
                assert!(parent.is_some(), "the step above did not nest the active row");
                update(app, Message::BodyDuplicated);
                assert_eq!(
                    app.doc.parent_of(app.doc.active()),
                    parent,
                    "the copy left the folder it was made in"
                );
            }),
            ("collapse the folder", |app| {
                let folder = app.doc.parent_of(app.doc.active());
                if let Some(folder) = folder {
                    update(app, Message::FolderCollapseToggled(folder));
                }
            }),
            ("select a row inside a collapsed folder", |app| {
                let first = app.doc.nodes()[0].id;
                update(app, Message::BodySelected(first));
            }),
            ("leave the folder's solo", |app| update(app, Message::SoloExited)),
            ("drag a body into a folder", |app| {
                let folder = app.doc.folders().next().map(|folder| folder.id);
                let active = app.doc.active();
                if let Some(folder) = folder {
                    drag_into_folder(app, active, folder);
                }
            }),
            ("drag it back out", |app| {
                let active = app.doc.active();
                drag_to_top_level(app, active);
            }),
            // A drag in flight, which is a different tree from every state
            // above it: the dragged row is folded away, one row carries the
            // filled indicator or a two-pixel line sits between two others,
            // and every row is tracking the pointer.
            ("hold a row over another one", |app| {
                let last = app.doc.nodes().last().expect("a row").id;
                update(app, Message::BodySelected(last));
                for over in 0..app.doc.node_count() {
                    for band in [0.1_f32, 0.5, 0.9] {
                        update(app, Message::BodyRowDragged { over, fraction: band });
                        let _ = app.view();
                    }
                }
            }),
            ("let go", |app| {
                update(
                    app,
                    Message::Pointer(PointerEvent::Released { button: PointerButton::Left }),
                );
            }),
            ("hide a folder", |app| {
                let folder = app.doc.folders().next().map(|folder| folder.id);
                if let Some(folder) = folder {
                    update(app, Message::BodyVisibilityToggled(folder));
                }
            }),
            ("delete a folder", |app| {
                let folder = app.doc.folders().next().map(|folder| folder.id);
                if let Some(folder) = folder {
                    update(app, Message::FolderDeleted(folder));
                }
            }),
            ("ungroup", |app| update(app, Message::BodyUngrouped)),
            // At 64 rows this is the refusal and at the others it is the copy,
            // and both have to build a tree: the refusal leaves the list as it
            // was with a status line under it, and the copy adds a row in the
            // middle rather than at the end.
            ("duplicate", |app| update(app, Message::BodyDuplicated)),
            // At one row this is the refusal and at the others it is the
            // merge, and both have to build a tree.
            ("merge down", |app| {
                let first = app.doc.nodes()[0].id;
                update(app, Message::BodySelected(first));
                update(app, Message::BodyMergedDown);
            }),
            // The prompt card, which nothing else here builds. A threshold of
            // zero raises it for any merge that has a target at all.
            ("raise the merge prompt", |app| {
                let first = app.doc.nodes()[0].id;
                update(app, Message::BodySelected(first));
                app.merge_prompt_bytes = 0;
                update(app, Message::BodyMergedDown);
            }),
            ("cancel the merge prompt", |app| {
                app.merge_prompt_bytes = hani_core::DEFAULT_RECLAIM_BUDGET;
                update(app, Message::BodyMergeCancelled);
            }),
            ("delete", |app| update(app, Message::BodyDeleted)),
            // The mask tool changes two things about the tree at once: the
            // strip grows a lit eleventh button, and the viewport grows a
            // standing overlay card that nothing else here builds. Both are
            // built before the mask exists and again after it does, which is
            // why this is two steps rather than one.
            ("enter mask mode", |app| update(app, Message::ToolChanged(Tool::Mask))),
            ("paint a mask", |app| {
                let volume = app.doc.active_volume_mut();
                volume.mask_mut().write(glam::IVec3::ZERO, 128);
            }),
            ("undo", |app| update(app, Message::Undo)),
            ("redo", |app| update(app, Message::Redo)),
            // The right-click tool menu, whose foot is a different block in
            // mask mode from every other tool: the tint slider, the show-mask
            // switch and the pattern readout, in place of the pattern's own
            // controls. Nothing else in this list opens the menu at all.
            ("choose a pattern", |app| {
                update(app, Message::PatternChanged(hani_core::PatternKind::Scales));
            }),
            ("open the tool menu", |app| open_the_tool_menu(app)),
            ("open the tool menu while masking", |app| {
                update(app, Message::MenuClosed);
                if app.tool != Tool::Mask {
                    update(app, Message::ToolChanged(Tool::Mask));
                }
                open_the_tool_menu(app);
            }),
            ("close the tool menu", |app| update(app, Message::MenuClosed)),
        ];

        for rows in [1usize, 2, 12, 64] {
            for thumbnails in [true, false] {
                let mut app = app();
                app.thumbnails = thumbnails;
                while app.doc.node_count() < rows {
                    let name = format!("Body {}", app.doc.node_count() + 1);
                    cheap_body(&mut app, &name);
                }
                app.publish_visibility();
                let _ = app.view();

                for (what, step) in steps {
                    step(&mut app);
                    // The tree is built and dropped. `Element` borrows the
                    // application, so it cannot outlive the loop body -- which
                    // is also what stops a row from caching anything.
                    let _ = app.view();
                    assert!(
                        app.doc.body_count() >= 1,
                        "{what} at {rows} rows left the document with no body"
                    );
                }
            }
        }
    }

    /// BODIES is FIRST and open; DETAIL and FILAMENT start closed.
    ///
    /// The count is pinned so that adding a section is a deliberate act: the
    /// budget is a 1080 high window with every heading reachable without a
    /// scroll, and a section added without thinking about that budget is how
    /// the one below it goes under the fold.
    #[test]
    fn the_bodies_section_is_first_and_open_and_the_long_ones_start_closed() {
        assert_eq!(PanelSection::ALL[0], PanelSection::Bodies);
        assert!(PanelSection::Bodies.open_by_default());
        assert!(!PanelSection::Detail.open_by_default());
        assert!(!PanelSection::Filament.open_by_default());
        // FORM works on the whole body, now and then, and starts closed since
        // every heading folds. See `docs/WORKSPACES.md`.
        assert!(!PanelSection::Form.open_by_default());
        // FILAMENT sits directly above EXPORT: it says what the sculpt's slot
        // numbers mean, and it is read when somebody exports.
        let position = |which: PanelSection| {
            PanelSection::ALL.iter().position(|section| *section == which).expect("in ALL")
        };
        assert_eq!(position(PanelSection::Filament) + 1, position(PanelSection::Export));

        let app = app();
        assert!(app.expanded[PanelSection::Bodies as usize], "BODIES did not start open");
        assert!(!app.expanded[PanelSection::Detail as usize], "DETAIL did not start closed");
        assert!(!app.expanded[PanelSection::Filament as usize], "FILAMENT did not start closed");
    }

    // --- folders -------------------------------------------------------------

    /// The shape of the tree, as `(depth, name)` per row, which is what almost
    /// every folder assertion below is really about.
    fn shape(app: &Hani) -> Vec<(u8, String)> {
        app.doc.nodes().iter().map(|node| (node.depth(), node.name.clone())).collect()
    }

    /// `ctrl+G` wraps the active row in place, and pressing it again NESTS --
    /// which is the only route to a deep tree before a drag lands, and the one
    /// the depth-seven fixture in the format tests is built with.
    #[test]
    fn ctrl_g_wraps_the_active_row_and_wraps_again_to_nest() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();

        update(&mut app, Message::BodyGrouped);
        assert_eq!(
            shape(&app),
            vec![(0, "Body 1".into()), (0, "Group 1".into()), (1, "Cube".into())],
            "the folder did not appear where the row was"
        );
        assert_eq!(app.doc.active(), cube, "grouping moved the selection");
        assert!(app.unsaved, "the tree is written to the file, so grouping is a change");

        update(&mut app, Message::BodyGrouped);
        assert_eq!(
            shape(&app),
            vec![
                (0, "Body 1".into()),
                (0, "Group 1".into()),
                (1, "Group 2".into()),
                (2, "Cube".into()),
            ],
            "the second press did not nest"
        );
    }

    /// `ctrl+shift+G` is `ctrl+G` read backwards, outline for outline, and one
    /// ctrl+Z is enough to reverse either of them.
    #[test]
    fn ctrl_shift_g_dissolves_the_folder_the_active_row_is_in() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let was = shape(&app);

        update(&mut app, Message::BodyGrouped);
        update(&mut app, Message::BodyUngrouped);
        assert_eq!(shape(&app), was, "the pair is not each other's inverse");

        // And the pair reversed by hand, one press each.
        update(&mut app, Message::BodyGrouped);
        let grouped = shape(&app);
        update(&mut app, Message::Undo);
        assert_eq!(shape(&app), was, "one ctrl+Z did not take the group apart");
        update(&mut app, Message::Redo);
        assert_eq!(shape(&app), grouped, "one ctrl+shift+Z did not put it back");
    }

    /// A row with no folder above it has nothing to dissolve, and it says so
    /// rather than doing something surprising to the nearest folder it can find.
    #[test]
    fn ungrouping_a_row_that_is_not_in_a_folder_is_refused_with_a_line() {
        let mut app = app();
        let was = shape(&app);
        update(&mut app, Message::BodyUngrouped);
        assert_eq!(shape(&app), was);
        assert!(app.status.contains("not in a folder"), "no line explains it: {}", app.status);
    }

    /// The chevron only folds rows away. It must not change the document's
    /// shape, and it must survive a save -- `collapsed` is a bit in the file.
    #[test]
    fn the_chevron_folds_rows_away_and_changes_nothing_else() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.nodes()[1].id;
        let was = shape(&app);

        update(&mut app, Message::FolderCollapseToggled(folder));
        assert!(app.doc.node(folder).unwrap().collapsed, "the chevron did nothing");
        assert_eq!(shape(&app), was, "collapsing changed the tree");

        update(&mut app, Message::Undo);
        assert!(!app.doc.node(folder).unwrap().collapsed, "collapse is not undoable");
    }

    /// **Deleting a body inside a collapsed folder deletes the body, never the
    /// folder.**
    ///
    /// A user lost an unrecoverable hour to ZBrush doing the other thing, its
    /// own bundled Delete macro had the same hole, and a third-party plugin
    /// exists solely to intercept it. Here it is structural rather than
    /// remembered: the verb row's Delete names `Document::active`, which always
    /// holds a field, and the folder's trash names the folder.
    #[test]
    fn deleting_a_body_in_a_collapsed_folder_never_takes_the_folder() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.parent_of(sphere).expect("the new folder");
        // A second body inside, so the folder has a reason to survive.
        let cube = app.doc.nodes().iter().find(|node| node.name == "Cube").expect("the cube").id;
        drag_into_folder(&mut app, cube, folder);
        update(&mut app, Message::FolderCollapseToggled(folder));
        update(&mut app, Message::BodySelected(sphere));

        update(&mut app, Message::BodyDeleted);
        assert!(app.doc.node(folder).is_some(), "a collapsed folder swallowed a body delete");
        assert!(app.doc.node(cube).is_some(), "the folder's other body went with it");
        assert!(app.doc.node(sphere).is_none(), "the body the user asked about survived");
        assert!(app.doc.node(folder).unwrap().collapsed, "the folder came open");
    }

    /// A folder delete takes its contents, in ONE entry, and one ctrl+Z brings
    /// the whole thing back with the folder above the bodies again.
    #[test]
    fn a_folder_delete_takes_its_bodies_and_one_undo_brings_them_all_back() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.nodes()[1].id;
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        drag_into_folder(&mut app, sphere, folder);
        let was = shape(&app);
        assert_eq!(app.doc.subtree_body_count(folder), 2);

        let entries = app.history_stats.undo_entries;
        update(&mut app, Message::FolderDeleted(folder));
        assert_eq!(app.doc.body_count(), 1, "the folder's bodies were left behind");
        assert_eq!(
            app.history_stats.undo_entries,
            entries + 1,
            "three removals became three gestures"
        );
        assert!(app.status.contains("2 bodies"), "the line does not say what went: {}", app.status);

        update(&mut app, Message::Undo);
        assert_eq!(shape(&app), was, "one ctrl+Z did not put the whole folder back");
    }

    /// The folder trash never takes a body row, whatever it is handed. The
    /// message is the folder's own affordance and a body has its own.
    #[test]
    fn the_folder_delete_refuses_a_body_row() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        let was = shape(&app);

        update(&mut app, Message::FolderDeleted(cube));
        assert_eq!(shape(&app), was, "the folder trash deleted a body");
    }

    /// A folder delete over the reclaim allowance prompts with the SUMMED size,
    /// and cancelling changes nothing.
    ///
    /// Folders are what make the prompt the common case: forty modest bodies
    /// pass a per-body threshold every time and then evict each other, which is
    /// the failure the threshold and the allowance being ONE number exists to
    /// catch.
    #[test]
    fn a_folder_delete_over_the_allowance_prompts_with_the_summed_size() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.nodes()[1].id;
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        drag_into_folder(&mut app, sphere, folder);

        let summed = app.subtree_bytes(folder);
        let one = app
            .doc
            .node(app.doc.active())
            .and_then(hani_core::Node::volume)
            .map_or(0, |volume| volume.stats().resident_bytes);
        assert!(summed > one, "the fixture's two bodies are not big enough to tell apart");
        // Between the two, so a per-body threshold would NOT fire and the
        // summed one must.
        app.delete_prompt_bytes = (one + summed) / 2;

        let was = shape(&app);
        update(&mut app, Message::FolderDeleted(folder));
        let pending = app.pending_delete.as_ref().expect("the prompt did not open");
        assert_eq!(pending.bytes, summed, "the prompt named one body's size, not the folder's");
        assert_eq!(shape(&app), was, "the prompt deleted before it asked");

        update(&mut app, Message::BodyDeleteCancelled);
        assert_eq!(shape(&app), was, "cancelling changed the document");

        update(&mut app, Message::FolderDeleted(folder));
        update(&mut app, Message::BodyDeleteConfirmed);
        assert!(app.doc.node(folder).is_none(), "confirming did not delete the folder");
    }

    /// A press on a folder row selects the first body inside it, because the
    /// active row always holds a field.
    #[test]
    fn pressing_a_folder_row_selects_the_first_body_inside_it() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.nodes()[1].id;
        let first = app.doc.nodes()[0].id;
        update(&mut app, Message::BodySelected(first));
        assert_ne!(app.doc.active(), cube);

        update(&mut app, Message::BodySelected(folder));
        assert_eq!(app.doc.active(), cube, "pressing a folder did not reach the body inside it");
        assert!(app.doc.volume(app.doc.active()).is_some(), "a folder became the active row");
    }

    /// Dragging a folder's last child out dissolves the folder in the SAME
    /// entry, so one ctrl+Z puts both back.
    #[test]
    fn dragging_the_last_child_out_of_a_folder_dissolves_it_in_one_entry() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.nodes()[1].id;
        let cube = app.doc.active();
        let was = shape(&app);

        let entries = app.history_stats.undo_entries;
        drag_to_top_level(&mut app, cube);
        assert!(app.doc.node(folder).is_none(), "an empty folder was left behind");
        assert_eq!(
            app.history_stats.undo_entries,
            entries + 1,
            "the move and the dissolve are two gestures"
        );

        update(&mut app, Message::Undo);
        assert_eq!(shape(&app), was, "one ctrl+Z did not restore the folder and the row together");
    }

    // --- dragging a row ------------------------------------------------------

    /// A folder holding one body, a body beside it, and a second body inside
    /// the folder below the first. Returns the folder and its two children in
    /// panel order.
    ///
    /// ```text
    /// 0 Body 1
    /// 1 Group 1
    /// 2   Cube
    /// 3   Sphere
    /// ```
    fn a_folder_of_two(app: &mut Hani) -> (NodeId, NodeId, NodeId) {
        update(app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        update(app, Message::BodyGrouped);
        let folder = app.doc.parent_of(cube).expect("the new folder");
        update(app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        drag_below(app, sphere, cube);
        assert_eq!(
            shape(app),
            vec![
                (0, "Body 1".into()),
                (0, "Group 1".into()),
                (1, "Cube".into()),
                (1, "Sphere".into()),
            ],
            "the fixture is not the shape under test"
        );
        (folder, cube, sphere)
    }

    /// **The round trip the increment exists for**: a row goes into a folder
    /// and comes back out again, one gesture and one undo press each way.
    #[test]
    fn a_row_drags_into_a_folder_and_back_out_again() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.parent_of(cube).expect("the new folder");
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        let was = shape(&app);
        let entries = app.history_stats.undo_entries;

        drag_into_folder(&mut app, sphere, folder);
        assert_eq!(app.doc.parent_of(sphere), Some(folder), "it did not go in: {}", app.status);
        assert_eq!(app.history_stats.undo_entries, entries + 1, "a drag in is not one entry");
        assert!(app.unsaved, "the tree is in the file, so a reorder is a change");
        assert!(app.status.contains("Group 1"), "the line does not say where: {}", app.status);

        drag_to_top_level(&mut app, sphere);
        assert_eq!(app.doc.parent_of(sphere), None, "it did not come out: {}", app.status);
        assert_eq!(app.history_stats.undo_entries, entries + 2);
        assert!(app.status.contains("top level"), "the line does not say where: {}", app.status);

        update(&mut app, Message::Undo);
        update(&mut app, Message::Undo);
        assert_eq!(shape(&app), was, "two ctrl+Z did not put the two drags back");
    }

    /// A whole subtree moves as a BLOCK, and one ctrl+Z puts all of it back.
    ///
    /// The rows under a folder are a contiguous preorder run, so this is one
    /// splice however many rows are in it -- which is also why it is one
    /// `Change::Outline` and one entry rather than one per row.
    #[test]
    fn dragging_a_folder_takes_everything_in_it_and_one_undo_brings_it_back() {
        let mut app = app();
        let (folder, cube, sphere) = a_folder_of_two(&mut app);
        let outer = app.doc.nodes()[0].id;
        update(&mut app, Message::BodySelected(outer));
        update(&mut app, Message::BodyGrouped);
        // 0 Group 2, 1 Body 1, 2 Group 1, 3 Cube, 4 Sphere
        let target = app.doc.parent_of(outer).expect("the outer folder");
        let was = shape(&app);
        let entries = app.history_stats.undo_entries;

        drag_into_folder(&mut app, folder, target);
        assert_eq!(app.doc.parent_of(folder), Some(target), "the folder did not move");
        assert_eq!(app.doc.parent_of(cube), Some(folder), "the cube did not come with it");
        assert_eq!(app.doc.parent_of(sphere), Some(folder), "the sphere did not come with it");
        assert_eq!(
            app.history_stats.undo_entries,
            entries + 1,
            "three rows moved as three gestures"
        );

        update(&mut app, Message::Undo);
        assert_eq!(shape(&app), was, "one ctrl+Z did not take the whole block back");
    }

    /// **A drag that changes nothing pushes no entry and does not set
    /// `unsaved`.** Three ways to ask for where the row already is.
    #[test]
    fn a_drag_that_changes_nothing_costs_no_undo_press() {
        let mut app = app();
        let (_, cube, sphere) = a_folder_of_two(&mut app);
        let was = shape(&app);
        app.unsaved = false;
        let entries = app.history_stats.undo_entries;

        let cube_at = app.doc.index_of(cube).expect("the cube");
        let sphere_at = app.doc.index_of(sphere).expect("the sphere");
        for (what, row, over, band) in [
            ("above itself", cube, cube_at, 0.1),
            ("below itself", cube, cube_at, 0.9),
            ("below the row it already follows", sphere, cube_at, 0.9),
            ("above the row it already precedes", cube, sphere_at, 0.1),
        ] {
            drag_row(&mut app, row, over, band);
            assert_eq!(shape(&app), was, "dropping {what} moved something");
            assert_eq!(
                app.history_stats.undo_entries, entries,
                "dropping {what} cost an undo press"
            );
            assert!(!app.unsaved, "dropping {what} marked the document unsaved");
        }
    }

    /// A press that never moves is a selection, not a drag. There is no slop
    /// threshold to tune: without a move message there is no target, and
    /// without a target there is nothing to commit.
    #[test]
    fn a_press_that_never_moves_selects_and_moves_nothing() {
        let mut app = app();
        let (_, cube, _) = a_folder_of_two(&mut app);
        let was = shape(&app);
        app.unsaved = false;
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::BodySelected(cube));
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));

        assert_eq!(app.doc.active(), cube, "the press did not select the row");
        assert_eq!(shape(&app), was);
        assert_eq!(app.history_stats.undo_entries, entries);
        assert!(!app.unsaved);
    }

    /// **A folder cannot be dropped into its own child**, and the refusal
    /// arrives as a line rather than as an indicator that quietly fails to
    /// appear.
    #[test]
    fn a_folder_dropped_inside_itself_is_refused_with_the_reason() {
        let mut app = app();
        let (folder, cube, _) = a_folder_of_two(&mut app);
        let was = shape(&app);
        app.unsaved = false;
        let entries = app.history_stats.undo_entries;

        // The folder's own row, middle band. It is folded away for the duration
        // of its own drag, so that band means "inside me".
        let folder_at = app.doc.index_of(folder).expect("the folder");
        update(&mut app, Message::BodySelected(folder));
        update(&mut app, Message::BodyRowDragged { over: folder_at, fraction: 0.5 });
        assert!(
            app.row_drag.expect("the drag is armed").target.is_none(),
            "an illegal gap still offered a target"
        );
        assert!(app.status.contains("inside itself"), "no line explains it: {}", app.status);

        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert_eq!(shape(&app), was, "a refused drop moved rows anyway");
        assert_eq!(app.history_stats.undo_entries, entries);
        assert!(!app.unsaved);
        // And the row under it -- which is only reachable through a message
        // queued before the fold hid it -- is refused the same way.
        let cube_at = app.doc.index_of(cube).expect("the cube");
        update(&mut app, Message::BodySelected(folder));
        update(&mut app, Message::BodyRowDragged { over: cube_at, fraction: 0.5 });
        assert!(
            app.row_drag.expect("armed").target.is_none(),
            "a row inside the block took a drop"
        );
    }

    /// Eight levels is the cap, and a drop that would go past it is refused
    /// with the reason rather than silently flattened into one that fits.
    #[test]
    fn a_drop_past_the_eighth_level_is_refused_and_says_so() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        // A chain as deep as the panel goes, with the cube at the bottom of it.
        // ctrl+G is the only way to get there before a drag exists, which is
        // why this fixture is built out of presses.
        while app.doc.node(cube).expect("the cube").depth() < hani_core::MAX_DEPTH - 1 {
            update(&mut app, Message::BodySelected(cube));
            update(&mut app, Message::BodyGrouped);
        }
        // A two-level block: a folder holding one body. A single body would
        // FIT under the cube -- it is the block's own height that goes over.
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        update(&mut app, Message::BodyGrouped);
        let block = app.doc.parent_of(sphere).expect("the block's folder");
        let was = shape(&app);
        app.unsaved = false;

        let cube_at = app.doc.index_of(cube).expect("the cube");
        update(&mut app, Message::BodySelected(block));
        update(&mut app, Message::BodyRowDragged { over: cube_at, fraction: 0.9 });

        assert!(app.row_drag.expect("armed").target.is_none(), "the cap did not refuse");
        assert!(app.status.contains("eighth level"), "no line names the cap: {}", app.status);
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert_eq!(shape(&app), was, "a drop past the cap still moved rows");
        assert!(!app.unsaved);
    }

    /// **Drag is disabled while renaming.** A double click publishes the press
    /// FIRST and the double click second, so the row is picked up and then the
    /// field opens on top of it -- and the pickup has to be undone, or the list
    /// sits armed behind an open text field.
    #[test]
    fn opening_a_rename_puts_the_row_back_down() {
        let mut app = app();
        let (_, cube, _) = a_folder_of_two(&mut app);
        let was = shape(&app);

        // Exactly what a double click delivers, in that order.
        update(&mut app, Message::BodySelected(cube));
        drop(app.update(Message::BodyRenameBegan(cube)));
        assert!(app.renaming.is_some(), "the field did not open");
        assert!(app.row_drag.is_none(), "the list is still armed behind the rename field");

        // ...and a pointer that keeps moving cannot pick it back up.
        update(&mut app, Message::BodyRowDragged { over: 0, fraction: 0.1 });
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert_eq!(shape(&app), was, "a drag ran while a rename was open");
    }

    /// The commit rides on the GLOBAL release, so a drag that ends anywhere --
    /// over the tool strip, over the viewport, outside the window -- still
    /// lands, and a card that opened mid-drag drops the row rather than
    /// performing the move behind its scrim.
    #[test]
    fn a_drag_interrupted_by_a_card_is_dropped_rather_than_performed() {
        let mut app = app();
        let (folder, _, sphere) = a_folder_of_two(&mut app);
        let folder_at = app.doc.index_of(folder).expect("the folder");
        let was = shape(&app);
        app.unsaved = false;

        update(&mut app, Message::BodySelected(sphere));
        update(&mut app, Message::BodyRowDragged { over: folder_at, fraction: 0.1 });
        assert!(app.row_drag.expect("armed").target.is_some(), "the fixture drag is not legal");

        // A prompt goes up while the button is still down.
        app.confirm = Some(PendingAction::NewSculpt);
        assert!(app.modal_open(), "the fixture did not raise a modal");
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));

        assert!(app.row_drag.is_none(), "the list stayed armed behind the card");
        assert_eq!(shape(&app), was, "the move ran from behind a scrim");
        assert!(!app.unsaved);
    }

    /// **The third indicator**, which is the one every shipped tree drag gets
    /// wrong: a CLOSED folder's middle band means "inside it", and the row
    /// itself is filled to say so where a line between two rows could not.
    ///
    /// The bands either side of it still mean before and after, so all three
    /// answers are reachable on the one row a user cannot see the inside of.
    #[test]
    fn a_closed_folders_middle_band_puts_the_row_inside_it() {
        let mut app = app();
        let (folder, _, _) = a_folder_of_two(&mut app);
        update(&mut app, Message::FolderCollapseToggled(folder));
        let loose = app.doc.nodes()[0].id;
        let folder_at = app.doc.index_of(folder).expect("the folder");
        let was = shape(&app);

        // Above it: a line at the top level, and no row is filled.
        update(&mut app, Message::BodySelected(loose));
        update(&mut app, Message::BodyRowDragged { over: folder_at, fraction: 0.1 });
        let drag = app.row_drag.expect("armed");
        assert_eq!(drag.into, None, "the top band filled the folder");
        assert_eq!(drag.target.expect("a target").depth, 0, "the top band went inside");

        // Its middle: the folder is the destination, and the panel is told to
        // fill that row rather than draw a line anywhere.
        update(&mut app, Message::BodyRowDragged { over: folder_at, fraction: 0.5 });
        let drag = app.row_drag.expect("armed");
        assert_eq!(drag.into, Some(folder_at), "the middle band did not name the folder");
        assert_eq!(drag.target.expect("a target").depth, 1, "the middle band stayed outside");
        // The tree still builds with a row filled, which nothing else covers.
        drop(app.view());

        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert_eq!(app.doc.parent_of(loose), Some(folder), "it did not go in: {}", app.status);
        assert!(
            app.doc.node(folder).expect("the folder").collapsed,
            "the drop opened the folder it landed in"
        );

        update(&mut app, Message::Undo);
        assert_eq!(shape(&app), was, "one ctrl+Z did not put it back");
    }

    /// The gap under a folder's last child and the gap above the row after it
    /// are the SAME index at two depths, and choosing between them by which
    /// side of the boundary the pointer is on is the whole of how a row gets
    /// back out of a folder.
    #[test]
    fn the_row_below_a_folders_last_child_is_the_way_out_of_it() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let cube = app.doc.active();
        update(&mut app, Message::BodyGrouped);
        let folder = app.doc.parent_of(cube).expect("the folder");
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Sphere));
        let sphere = app.doc.active();
        drag_below(&mut app, sphere, cube);
        // 0 Body 1, 1 Group 1, 2 Cube, 3 Sphere
        assert_eq!(app.doc.parent_of(sphere), Some(folder), "the fixture is wrong");

        // The bottom band of the last child keeps it in the folder...
        let sphere_at = app.doc.index_of(sphere).expect("the sphere");
        update(&mut app, Message::BodySelected(cube));
        update(&mut app, Message::BodyRowDragged { over: sphere_at, fraction: 0.9 });
        assert_eq!(
            app.row_drag.expect("armed").target.expect("a target").depth,
            1,
            "the gap below the last child left the folder"
        );

        // ...and the top band of the row above the folder takes it out.
        update(&mut app, Message::BodyRowDragged { over: 0, fraction: 0.1 });
        assert_eq!(
            app.row_drag.expect("armed").target.expect("a target").depth,
            0,
            "the top band of a top-level row did not mean the top level"
        );
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert_eq!(app.doc.parent_of(cube), None, "it did not come out: {}", app.status);
        assert_eq!(app.doc.parent_of(sphere), Some(folder), "the folder lost its other child");
    }

    /// **No cell is staled by any drag.** Reordering the panel moves no
    /// geometry, so a picture that was up to date before the gesture is up to
    /// date after it -- and a list of sixty-four bodies does not queue
    /// sixty-four full-body renders because somebody tidied it.
    #[test]
    fn no_drag_ever_stales_a_thumbnail() {
        let mut app = app();
        app.thumbnails = true;
        let (folder, cube, sphere) = a_folder_of_two(&mut app);
        // Every body's picture up to date, which is the state a drag has to
        // leave alone.
        for node in app.doc.nodes().iter().filter(|node| node.is_body()) {
            app.thumbs.requested(node.id);
        }
        assert_eq!(app.thumbs.stale_count(), 0, "the fixture did not settle");
        let cells: Vec<(NodeId, Option<u32>)> = app
            .doc
            .nodes()
            .iter()
            .filter(|node| node.is_body())
            .map(|node| (node.id, app.thumbs.cell(node.id)))
            .collect();

        drag_to_top_level(&mut app, sphere);
        drag_into_folder(&mut app, sphere, folder);
        drag_below(&mut app, cube, sphere);
        drag_to_top_level(&mut app, folder);
        drop(app.view());

        assert_eq!(app.thumbs.stale_count(), 0, "a drag marked a picture stale");
        assert!(!app.shared.thumbnail_pending(), "a drag queued a render");
        for (body, cell) in cells {
            assert_eq!(app.thumbs.cell(body), cell, "a body's cell moved under it");
        }
    }

    /// The row cap is a refusal with a line that names it, exactly as the body
    /// cap is: nothing the interface builds goes through the reader.
    ///
    /// **`MAX_NODES` is twice `MAX_BODIES`**, so the only way to fill the list
    /// is with folders as well as bodies -- which is exactly why the two caps
    /// are separate numbers, and why the fixture has to be built this way.
    #[test]
    fn grouping_past_the_row_cap_is_refused_and_says_so() {
        let mut app = app();
        while app.doc.body_count() < MAX_BODIES {
            let name = format!("Body {}", app.doc.body_count() + 1);
            cheap_body(&mut app, &name);
        }
        // One folder per body, each wrapping only its own row, until the rows
        // run out.
        let bodies: Vec<NodeId> = app.doc.bodies().map(|(id, _)| id).collect();
        for id in bodies {
            if app.doc.node_count() >= hani_core::MAX_NODES {
                break;
            }
            update(&mut app, Message::BodySelected(id));
            update(&mut app, Message::BodyGrouped);
        }
        assert_eq!(app.doc.node_count(), hani_core::MAX_NODES, "the fixture did not fill up");
        let rows = app.doc.node_count();

        update(&mut app, Message::BodyGrouped);
        assert_eq!(app.doc.node_count(), rows, "the cap let a folder through");
        assert!(
            app.status.contains("could not group")
                && app.status.contains(&hani_core::MAX_NODES.to_string()),
            "the refusal does not name the cap: {}",
            app.status
        );
    }

    /// Eight levels is the cap, and the ninth press is refused with a line
    /// rather than clamped into a folder that did nothing.
    #[test]
    fn grouping_past_the_eighth_level_is_refused_and_says_so() {
        let mut app = app();
        for _ in 0..hani_core::MAX_DEPTH - 1 {
            update(&mut app, Message::BodyGrouped);
        }
        assert_eq!(
            app.doc.node(app.doc.active()).unwrap().depth(),
            hani_core::MAX_DEPTH - 1,
            "seven presses did not reach the deepest legal level"
        );
        let rows = app.doc.node_count();

        update(&mut app, Message::BodyGrouped);
        assert_eq!(app.doc.node_count(), rows, "an eighth level was allowed");
        assert!(
            app.status.contains("as far as the panel goes"),
            "the refusal does not say why: {}",
            app.status
        );
    }

    // --- split by loose parts ---------------------------------------------------

    /// ONE body holding TWO loose parts, built out of the application's own
    /// verbs.
    ///
    /// The ball and a cube placed clear of it, merged: `merge_down` is a `min`
    /// of two fields that do not touch, so the result is a single row whose
    /// material is in two pieces. That is exactly the shape an imported scan
    /// arrives in, and building it this way means the fixture cannot drift away
    /// from what the application really produces.
    fn one_body_of_two_parts() -> Hani {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let upper = app.doc.nodes()[0].id;
        update(&mut app, Message::BodySelected(upper));
        update(&mut app, Message::BodyMergedDown);
        assert_eq!(app.doc.body_count(), 1, "the fixture is not one body: {}", app.status);
        app.unsaved = false;
        app
    }

    /// **The whole feature from the user's chair**: press the button, read how
    /// many parts there are, say yes, and the row is two rows.
    #[test]
    fn splitting_asks_first_and_then_makes_one_body_per_part() {
        let mut app = one_body_of_two_parts();
        let source = app.doc.active();
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::BodySplit);
        let pending = app.pending_split.as_ref().expect("split raised no card");
        assert_eq!(pending.plan.found(), 2, "the walk did not find the two parts");
        assert!(!pending.plan.has_fragments(), "a cube and a ball are not debris");
        assert_eq!(app.doc.body_count(), 1, "the card split it anyway");
        assert!(!app.unsaved, "raising the card dirtied the document");
        assert!(app.modal_open(), "the split card is not modal, so a press behind it sculpts");

        update(&mut app, Message::BodySplitConfirmed);
        assert_eq!(app.doc.body_count(), 2, "confirming did not split: {}", app.status);
        assert!(app.doc.node(source).is_none(), "the source row survived its own split");
        assert!(app.unsaved, "splitting did not mark the document unsaved");
        assert!(app.pending_split.is_none(), "the card stayed up");
        assert_eq!(app.history_stats.undo_entries, entries + 1, "a split is not one entry");
        assert!(app.status.contains("into 2 parts"), "the split said nothing: {}", app.status);

        // And one press puts it back, whole.
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), 1, "one undo did not take the split apart");
        assert!(app.doc.node(source).is_some(), "the source row did not come back");
    }

    /// Cancelling, by either route, changes not one byte -- which is the whole
    /// reason it is safe to ask.
    #[test]
    fn cancelling_a_split_leaves_the_document_exactly_as_it_was() {
        for cancel in [Message::BodySplitCancelled, Message::MenuClosed] {
            let mut app = one_body_of_two_parts();
            let entries = app.history_stats.undo_entries;
            update(&mut app, Message::BodySplit);
            assert!(app.pending_split.is_some(), "split raised no card");

            update(&mut app, cancel);
            assert!(app.pending_split.is_none(), "the card stayed up");
            assert_eq!(app.doc.body_count(), 1, "cancelling split it anyway");
            assert!(!app.unsaved, "cancelling dirtied the document");
            assert_eq!(app.history_stats.undo_entries, entries, "cancelling recorded an entry");
        }
    }

    /// Escape reaches the card through the real key path, and does not fall
    /// through to disarm the cut on its way past.
    #[test]
    fn escape_answers_the_split_card_before_anything_else() {
        let mut app = one_body_of_two_parts();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        assert_eq!(app.tool, Tool::Cut, "the fixture did not arm the cut");
        update(&mut app, Message::BodySplit);

        update(
            &mut app,
            Message::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                modifiers: iced::keyboard::Modifiers::default(),
            },
        );
        assert!(app.pending_split.is_none(), "escape did not reach the card");
        assert_eq!(app.tool, Tool::Cut, "escape went past the card and disarmed the cut as well");
    }

    /// A body that is one piece says so and raises nothing. The alternative is
    /// a card that asks a question with one answer.
    #[test]
    fn a_body_that_is_one_piece_says_so_and_raises_no_card() {
        let mut app = app();
        update(&mut app, Message::BodySplit);
        assert!(app.pending_split.is_none(), "one connected ball raised a card");
        assert_eq!(app.doc.body_count(), 1);
        assert!(!app.unsaved, "a refused split dirtied the document");
        assert!(
            app.status.contains("one connected part"),
            "the refusal does not say why: {}",
            app.status
        );
    }

    /// Refused when the source is not on screen, because its parts would not be
    /// either: children inherit the source's own eye.
    #[test]
    fn splitting_a_body_that_is_not_on_screen_is_refused_by_name() {
        let mut app = one_body_of_two_parts();
        let source = app.doc.active();
        cheap_body(&mut app, "Somewhere else");
        update(&mut app, Message::BodyVisibilityToggled(source));
        // Hiding the active row moved the selection; put it back on the hidden
        // one, which is what a click on its name does.
        update(&mut app, Message::BodySelected(source));
        assert_eq!(app.doc.active(), source, "the fixture did not select the hidden row");
        app.unsaved = false;

        update(&mut app, Message::BodySplit);
        assert!(app.pending_split.is_none(), "a hidden body raised a card");
        assert_eq!(app.doc.body_count(), 2, "a hidden body was split anyway");
        assert!(
            app.status.contains("not on screen"),
            "the refusal does not say why: {}",
            app.status
        );
    }

    /// The parts are meshed after the split, and the source's slots are
    /// released. Without the pair of them the new bodies are right in the
    /// document and absent from the viewport, which is the failure this project
    /// has shipped twice.
    #[test]
    fn every_part_is_offered_to_the_renderer_and_the_source_is_forgotten() {
        let mut app = one_body_of_two_parts();
        let source = app.doc.active();
        update(&mut app, Message::BodySplit);
        update(&mut app, Message::BodySplitConfirmed);

        assert!(
            app.shared.take_forgotten_for_tests().contains(&source),
            "the source's pool slots were never released"
        );
        for (id, volume) in app.doc.bodies() {
            assert_eq!(
                volume.dirty_count(),
                0,
                "{id:?} still has bricks nobody has meshed after the split"
            );
            assert!(volume.brick_count() > 0, "{id:?} came out with no bricks at all");
        }
    }
}

/// Inline rename: click a name, type, commit.
///
/// **The two things worth testing here are both about what a rename must NOT
/// do**: it must not let the keyboard shortcuts fire while the field has focus,
/// and it must not put a name in the document that the file cannot hold. The
/// happy path is three lines of state; those two are where the bugs are.
#[cfg(test)]
mod rename_tests {
    use super::*;
    use hani_core::{MAX_NAME_BYTES, PrimitiveKind};

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    fn name_of(app: &Hani, id: NodeId) -> String {
        app.doc.node(id).expect("the row is in the document").name.clone()
    }

    /// Begin a rename on `id` and type `typed` into the field, without
    /// committing.
    fn type_into_the_field(app: &mut Hani, id: NodeId, typed: &str) {
        update(app, Message::BodyRenameBegan(id));
        update(app, Message::BodyRenameEdited(typed.to_string()));
    }

    /// Write the document out and read it back, which is the only way to ask
    /// what a name will really be worth tomorrow.
    fn round_trip(app: &Hani) -> hani_core::Document {
        let mut bytes = Vec::new();
        hani_core::project::write(&mut bytes, &app.doc, &hani_core::ProjectState::default())
            .expect("writing the sculpt failed");
        let (doc, _) = hani_core::project::read(&mut bytes.as_slice()).expect("read failed");
        doc
    }

    /// A window event carrying `character`. The fields beyond `key` and
    /// `modifiers` are what a real winit press fills in and `key_event`
    /// ignores; they are here because the literal does not compile without
    /// them.
    fn digit_event(character: &str) -> iced::Event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(character.into()),
            modified_key: iced::keyboard::Key::Character(character.into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }

    /// Opening a rename ISSUES the two widget operations that focus the field.
    ///
    /// **Read what this does NOT say.** A widget operation can only be run
    /// against a live `UserInterface` and a real `Renderer`, neither of which
    /// a headless test can build, so nothing here proves that focus ARRIVES --
    /// only that `begin_rename` hands the runtime two units of work rather
    /// than `Task::none()`. That gap is real and is what the visual pass has
    /// to close: if focus does not land, the field renders unfocused, and
    /// every keystroke meant for it goes to `key_event` as Ignored instead.
    ///
    /// It is still worth pinning, because "tidy the task away" is a one-line
    /// edit that no other test in this file notices: the field would still be
    /// drawn, the text would still commit, and only a human at the keyboard
    /// would see that typing did nothing. The row that has gone is the
    /// control -- without it, `units()` could be reading a constant.
    #[test]
    fn opening_a_rename_hands_the_runtime_the_work_that_focuses_the_field() {
        let mut opened = app();
        let mut missing = app();
        let first = opened.doc.nodes()[0].id;

        let task = opened.update(Message::BodyRenameBegan(first));
        assert_eq!(
            task.units(),
            2,
            "the focus and select-all operations are not being issued, so the field opens dead"
        );

        let gone = missing.update(Message::BodyRenameBegan(NodeId(9_999)));
        assert_eq!(gone.units(), 0, "a rename of a row that is not there still asked for work");
    }

    /// **The check the whole increment 7 plumbing exists for.** A `1` typed
    /// into a focused rename field selects the letter, not the Draw brush.
    ///
    /// The guarantee lives in `key_event`, which drops captured events, and a
    /// focused `text_input` reports its keystrokes captured. This test asserts
    /// both halves against each other: the same key, the same application, the
    /// only difference being who wanted the event. Without the Ignored half the
    /// Captured half would still pass with the shortcut table deleted.
    #[test]
    fn a_digit_typed_into_the_rename_field_does_not_switch_the_brush() {
        let mut app = app();
        let first = app.doc.nodes()[0].id;
        // Something other than what `1` selects, so that "unchanged" is
        // distinguishable from "changed to the default".
        app.brush.kind = BrushKind::ALL[3];
        type_into_the_field(&mut app, first, "Hea");

        let captured =
            key_event(digit_event("1"), iced::event::Status::Captured, iced::window::Id::unique());
        assert!(captured.is_none(), "the field's own keystroke was forwarded as a shortcut");
        assert_eq!(app.brush.kind, BrushKind::ALL[3], "typing a name changed the brush");
        assert!(app.renaming.is_some(), "the field closed on a keystroke it never saw");

        // The control: the same key, wanted by nobody, does select the brush.
        // If this stops firing the assertion above means nothing.
        let ignored =
            key_event(digit_event("1"), iced::event::Status::Ignored, iced::window::Id::unique());
        let Some(message) = ignored else {
            panic!("an ignored digit produced no message, so the check above proves nothing");
        };
        update(&mut app, message);
        assert_eq!(app.brush.kind, BrushKind::ALL[0], "the control never switched the brush");
    }

    /// Enter commits, as ONE undoable change, and undo puts the old name back.
    #[test]
    fn enter_commits_the_typed_name_as_one_undoable_change() {
        let mut app = app();
        let first = app.doc.nodes()[0].id;
        let was = name_of(&app, first);
        let entries = app.history_stats.undo_entries;

        type_into_the_field(&mut app, first, "Left ear");
        assert_eq!(name_of(&app, first), was, "the document changed before the commit");

        update(&mut app, Message::BodyRenameSubmitted);

        assert_eq!(name_of(&app, first), "Left ear");
        assert!(app.renaming.is_none(), "the field stayed open after Enter");
        assert!(app.unsaved, "a rename did not mark the document unsaved");
        assert_eq!(
            app.history_stats.undo_entries,
            entries + 1,
            "one rename was not one undo entry"
        );

        update(&mut app, Message::Undo);
        assert_eq!(name_of(&app, first), was, "undo did not put the old name back");
        update(&mut app, Message::Redo);
        assert_eq!(name_of(&app, first), "Left ear", "redo did not bring the new name back");
    }

    /// Escape reverts, and leaves nothing behind: not the name, not an undo
    /// entry, not the unsaved marker.
    ///
    /// **It is the SECOND Escape in the running application**, because the
    /// focused field captures the first one and unfocuses itself. That is a
    /// property of iced's `text_input` and not of this code, so the test drives
    /// the message the second press produces.
    #[test]
    fn escape_reverts_a_rename_and_leaves_nothing_behind() {
        let mut app = app();
        let first = app.doc.nodes()[0].id;
        let was = name_of(&app, first);
        let entries = app.history_stats.undo_entries;

        type_into_the_field(&mut app, first, "Discarded");
        update(&mut app, Message::MenuClosed);

        assert!(app.renaming.is_none(), "escape left the field open");
        assert_eq!(name_of(&app, first), was, "escape committed the name it was meant to discard");
        assert!(!app.unsaved, "a reverted rename marked the document unsaved");
        assert_eq!(app.history_stats.undo_entries, entries, "a reverted rename cost an undo entry");
    }

    /// Escape against a rename must not also disarm the cut, which is the other
    /// thing Escape does. The early return in the `MenuClosed` arm is what says
    /// so, and without it one press would undo two decisions.
    #[test]
    fn escape_against_a_rename_leaves_an_armed_cut_armed() {
        let mut app = app();
        let first = app.doc.nodes()[0].id;
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        assert_eq!(app.tool, Tool::Cut, "the cut did not arm");

        type_into_the_field(&mut app, first, "Discarded");
        update(&mut app, Message::MenuClosed);

        assert!(app.renaming.is_none(), "escape left the field open");
        assert_eq!(app.tool, Tool::Cut, "escaping a rename also disarmed the cut");
    }

    /// **What "blur commits" means here.** Every way of leaving the field that
    /// is not Escape keeps what was typed.
    ///
    /// Table-driven over the four routes out, because the guard in `update` is
    /// a single decision covering all of them and one example would not show
    /// that: a press in the viewport, clicking another row, a menu command, and
    /// a keyboard shortcut arriving once the field has been blurred.
    #[test]
    fn every_way_out_of_the_field_except_escape_keeps_what_was_typed() {
        type Exit = (&'static str, fn(&mut Hani));
        let exits: [Exit; 5] = [
            // Through `key_event` and not `Message::PressedNothing` directly,
            // because the message is worth nothing unless the subscription
            // raises it: this is the press that used to produce NO message,
            // leaving the field drawn, unfocused and eating the next keystroke
            // as a tool shortcut.
            ("a press on the empty panel below the last row", |app| {
                let message = key_event(
                    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
                        iced::mouse::Button::Left,
                    )),
                    iced::event::Status::Ignored,
                    iced::window::Id::unique(),
                )
                .expect("a press nobody wanted raised no message, so a blur is invisible");
                update(app, message);
            }),
            ("a press in the viewport", |app| {
                update(
                    app,
                    Message::Pointer(PointerEvent::Pressed {
                        button: PointerButton::Left,
                        position: iced::Vector::new(400.0, 300.0),
                        size: iced::Vector::new(800.0, 600.0),
                    }),
                );
            }),
            ("clicking another row", |app| {
                let other = app.doc.nodes()[1].id;
                update(app, Message::BodySelected(other));
            }),
            ("a menu command", |app| update(app, Message::ThumbnailsToggled)),
            ("a shortcut once the field has been blurred", |app| {
                // Through `KeyPressed` and not the message it decodes to, so
                // this covers the nesting: `on_key` calls `update` again, and
                // the guard has to fire on the INNER message. `KeyPressed`
                // itself is on the keep list, because Escape has to survive it.
                update(
                    app,
                    Message::KeyPressed {
                        key: iced::keyboard::Key::Character("2".into()),
                        modifiers: iced::keyboard::Modifiers::empty(),
                    },
                );
            }),
        ];

        for (what, exit) in exits {
            let mut app = app();
            update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
            let first = app.doc.nodes()[0].id;
            type_into_the_field(&mut app, first, "Kept");

            exit(&mut app);

            assert!(app.renaming.is_none(), "the field survived {what}");
            assert_eq!(name_of(&app, first), "Kept", "{what} threw the typed name away");
        }
    }

    /// The frame tick and a pointer merely moving are not the user leaving.
    ///
    /// Named separately from the exits above because the failure is silent and
    /// total: `Frame` arrives at display rate, so a guard that committed on it
    /// would close the field before the first letter landed and the feature
    /// would look like it was never wired up.
    #[test]
    fn the_frame_tick_and_a_moving_pointer_leave_the_field_alone() {
        let mut app = app();
        let first = app.doc.nodes()[0].id;
        type_into_the_field(&mut app, first, "Still typing");

        update(&mut app, Message::Frame);
        update(
            &mut app,
            Message::Pointer(PointerEvent::Moved {
                position: iced::Vector::new(400.0, 300.0),
                size: iced::Vector::new(800.0, 600.0),
            }),
        );
        update(&mut app, Message::BodyRenameEdited("Still typing more".to_string()));

        let Some((id, typed)) = &app.renaming else {
            panic!("the field closed on a frame tick or a mouse move");
        };
        assert_eq!(*id, first);
        assert_eq!(typed, "Still typing more");
    }

    /// **Typing a name with an `h`, an `s` or a `u` in it does not commit the
    /// rename halfway through the word.**
    ///
    /// The trap is that a focused `text_input` captures its key PRESSES but
    /// not its key RELEASES: iced 0.14 captures `KeyReleased` for the literal
    /// `v` and nothing else (`text_input.rs:1249-1261`). So every other
    /// release arrives at [`key_event`] Ignored, exactly as if no field were
    /// focused, and `s`, `u` and `h` are the three letters it decodes there.
    /// Before the arm in [`keeps_the_rename_open`], the user typing "Head" got
    /// a body called "H" and a closed field on the release of the very first
    /// letter -- and "Skull" lost its own name to the `S`.
    ///
    /// The events are fed through `key_event` rather than the messages being
    /// posted directly, because the whole defect lives in that decode: a test
    /// that sent `MaskPeekEnded` itself would prove nothing about whether a
    /// release ever becomes one while a field is focused.
    #[test]
    fn releasing_a_gesture_key_while_renaming_does_not_commit_the_half_typed_name() {
        fn release(character: &str) -> iced::Event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                key: iced::keyboard::Key::Character(character.into()),
                modified_key: iced::keyboard::Key::Character(character.into()),
                physical_key: iced::keyboard::key::Physical::Unidentified(
                    iced::keyboard::key::NativeCode::Unidentified,
                ),
                location: iced::keyboard::Location::Standard,
                modifiers: iced::keyboard::Modifiers::empty(),
            })
        }

        for letter in ["h", "s", "u"] {
            let mut app = app();
            let first = app.doc.nodes()[0].id;
            let was = name_of(&app, first);
            type_into_the_field(&mut app, first, "H");

            let message = key_event(
                release(letter),
                iced::event::Status::Ignored,
                iced::window::Id::unique(),
            );
            let Some(message) = message else {
                panic!("releasing {letter} produced no message, so this test proves nothing");
            };
            update(&mut app, message);

            let Some((id, typed)) = &app.renaming else {
                panic!("releasing {letter} closed the rename field mid-word");
            };
            assert_eq!(*id, first);
            assert_eq!(typed, "H", "releasing {letter} threw away what had been typed");
            assert_eq!(
                name_of(&app, first),
                was,
                "releasing {letter} committed the half-typed name to the document"
            );
        }
    }

    /// **A multi-byte name at exactly the field's length round-trips
    /// unchanged.** Thirty-two bytes fills the fixed field with no NUL after
    /// it, which is the one name a "must be terminated" reader would destroy.
    #[test]
    fn a_multi_byte_name_at_exactly_the_field_length_round_trips_unchanged() {
        // Eight four-byte characters is thirty-two bytes on the nose.
        let name = "𝄞𝄞𝄞𝄞𝄞𝄞𝄞𝄞";
        assert_eq!(name.len(), MAX_NAME_BYTES);

        let mut app = app();
        let first = app.doc.nodes()[0].id;
        type_into_the_field(&mut app, first, name);
        update(&mut app, Message::BodyRenameSubmitted);

        assert_eq!(name_of(&app, first), name, "the field would not accept a full-length name");
        assert_eq!(
            round_trip(&app).nodes()[0].name,
            name,
            "a full-length name did not survive the file"
        );
    }

    /// **One byte past the field is a valid PREFIX, never "Body 1".** The
    /// field refuses the byte as it is typed, so what is on screen is what the
    /// file will hold.
    ///
    /// Cut on a char boundary or the field writes bytes that are not UTF-8, the
    /// reader repairs the whole name to a default, and the user silently loses
    /// a name this build wrote itself -- eleven characters typed, none kept.
    #[test]
    fn a_name_past_the_field_is_clamped_to_a_valid_prefix_and_never_to_a_default() {
        // Eleven three-byte characters is thirty-three bytes, so the eleventh
        // straddles byte thirty-two.
        let typed = "。。。。。。。。。。。";
        let expected = "。。。。。。。。。。";
        assert_eq!(typed.len(), MAX_NAME_BYTES + 1);
        assert_eq!(expected.len(), 30);

        let mut app = app();
        let first = app.doc.nodes()[0].id;
        type_into_the_field(&mut app, first, typed);

        let Some((_, held)) = &app.renaming else {
            panic!("the field closed while it was being typed into");
        };
        assert_eq!(held, expected, "the field held a name the file cannot store");

        update(&mut app, Message::BodyRenameSubmitted);
        assert_eq!(name_of(&app, first), expected);
        assert_eq!(
            round_trip(&app).nodes()[0].name,
            expected,
            "the name came back repaired to a default rather than as a prefix"
        );
    }

    /// A name that is only whitespace keeps the old one, and says so.
    ///
    /// Committing it would not leave the body nameless: the reader repairs an
    /// empty name field to `Body {n}`, so the body would come back called
    /// something the user never typed.
    #[test]
    fn a_rename_to_nothing_keeps_the_old_name() {
        let mut app = app();
        let first = app.doc.nodes()[0].id;
        let was = name_of(&app, first);
        let entries = app.history_stats.undo_entries;

        type_into_the_field(&mut app, first, "   ");
        update(&mut app, Message::BodyRenameSubmitted);

        assert_eq!(name_of(&app, first), was, "an empty name was committed");
        assert_eq!(app.history_stats.undo_entries, entries, "an empty name cost an undo entry");
        assert!(!app.unsaved, "an empty name marked the document unsaved");
        assert!(app.status.contains(&was), "the refusal did not say which name was kept");
    }

    /// Committing the name that is already there costs nothing at all --
    /// no entry, no unsaved marker. Opening a field and pressing Enter is not
    /// an edit, and charging a real undo press for it is how a history stops
    /// being trustworthy.
    #[test]
    fn committing_an_unchanged_name_costs_no_undo_entry() {
        let mut app = app();
        let first = app.doc.nodes()[0].id;
        let was = name_of(&app, first);
        let entries = app.history_stats.undo_entries;

        update(&mut app, Message::BodyRenameBegan(first));
        // Whitespace either side, so this also pins that the trim happens
        // before the comparison rather than after it.
        update(&mut app, Message::BodyRenameEdited(format!("  {was}  ")));
        update(&mut app, Message::BodyRenameSubmitted);

        assert_eq!(name_of(&app, first), was);
        assert_eq!(app.history_stats.undo_entries, entries, "a no-op rename cost an undo entry");
        assert!(!app.unsaved, "a no-op rename marked the document unsaved");
    }

    /// Beginning a rename on a second row commits the first, rather than
    /// silently swapping the field's owner and losing what was typed.
    #[test]
    fn beginning_a_second_rename_commits_the_first() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(PrimitiveKind::Cube));
        let first = app.doc.nodes()[0].id;
        let second = app.doc.nodes()[1].id;

        type_into_the_field(&mut app, first, "One");
        update(&mut app, Message::BodyRenameBegan(second));

        assert_eq!(name_of(&app, first), "One", "the first rename was thrown away");
        let Some((id, typed)) = &app.renaming else {
            panic!("the second rename did not open a field");
        };
        assert_eq!(*id, second);
        assert_eq!(typed, &name_of(&app, second), "the field did not start from the row's name");
    }

    /// The decode on its own, as a table.
    ///
    /// `keeps_the_rename_open` is inverted on purpose -- an unlisted message
    /// commits -- so what this pins is the short list of exceptions. A message
    /// wrongly ADDED to it leaves a field open over a document that has moved
    /// on; a message wrongly left out closes the field mid-word.
    #[test]
    fn only_the_field_its_keys_and_the_ambient_ticks_keep_a_rename_open() {
        let keeps: [Message; 6] = [
            Message::BodyRenameEdited("half a name".to_string()),
            Message::KeyPressed {
                key: iced::keyboard::Key::Character("1".into()),
                modifiers: iced::keyboard::Modifiers::empty(),
            },
            Message::MenuClosed,
            Message::Frame,
            Message::Pointer(PointerEvent::Moved {
                position: iced::Vector::new(1.0, 1.0),
                size: iced::Vector::new(800.0, 600.0),
            }),
            Message::Pointer(PointerEvent::Released { button: PointerButton::Left }),
        ];
        for message in keeps {
            assert!(keeps_the_rename_open(&message), "{message:?} closed the rename field");
        }

        let commits: [Message; 6] = [
            Message::BodyRenameSubmitted,
            Message::BodyRenameBegan(NodeId(1)),
            Message::BodySelected(NodeId(1)),
            Message::Undo,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: iced::Vector::new(1.0, 1.0),
                size: iced::Vector::new(800.0, 600.0),
            }),
            // The press that landed on nothing. `Pointer(Pressed)` above is
            // bounds-checked to the viewport and does not cover it.
            Message::PressedNothing,
        ];
        for message in commits {
            assert!(!keeps_the_rename_open(&message), "{message:?} left the rename field open");
        }
    }
}

/// The thumbnail refresh rule, which is the load-bearing part of live pictures.
///
/// Every test here is headless: the pixels are `hani-gpu`'s problem and are
/// checked in `offscreen.rs`. What is checked here is WHEN a picture is asked
/// for, because the two ways of getting that wrong are a stutter the user
/// cannot explain and a panel that quietly stops telling the truth.
#[cfg(test)]
mod thumbnail_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// A fresh application with the pictures switched ON.
    ///
    /// **The default is off** -- the go/no-go bench has produced no number yet,
    /// so the feature does not ship enabled; see the `thumbnails` field. Every
    /// test in this module is about the refresh RULE rather than about the
    /// default, so they all turn it on the way the user's own switch does. The
    /// default itself is pinned in
    /// `the_thumbnail_switch_does_not_dirty_the_document`.
    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.thumbnails = true;
        app
    }

    /// Everything the starting document wants drawn, taken, so a test is about
    /// what its own gesture asked for.
    fn settle(app: &mut Hani) {
        for _ in 0..hani_core::MAX_BODIES + 2 {
            app.thumbs.settle_now();
            update(app, Message::Frame);
            if app.shared.take_thumbnail_for_tests().is_none() {
                return;
            }
        }
        panic!("the document never stopped asking for pictures");
    }

    /// One frame, with the geometry pretended to have been still long enough.
    fn quiet_frame(app: &mut Hani) -> Option<crate::viewport::ThumbRequest> {
        app.thumbs.settle_now();
        update(app, Message::Frame);
        app.shared.take_thumbnail_for_tests()
    }

    fn press(app: &mut Hani, at: Vector) {
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: at,
            size: SIZE,
        });
    }

    fn drag_to(app: &mut Hani, at: Vector) {
        app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
    }

    fn release(app: &mut Hani) {
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
    }

    /// One presentation gesture: what it is called, and how to perform it on a
    /// named body.
    type Gesture = (&'static str, fn(&mut Hani, NodeId));

    /// **`view()` may not ask for anything, ever.**    ///
    /// It runs at display rate off `window::frames()`, so a row that enqueued a
    /// render, diffed staleness or measured a bound would repeat the
    /// `detail_advice` mistake multiplied by the number of bodies -- and this
    /// one would be multiplied again by a full-body GPU draw. The cell a row
    /// reads is allocated in `update`; `view` only ever looks it up.
    #[test]
    fn nothing_in_view_ever_asks_for_a_picture() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        settle(&mut app);

        let stale_before = app.thumbs.stale_count();
        for _ in 0..8 {
            app.thumbs.settle_now();
            drop(app.view());
        }

        assert!(!app.shared.thumbnail_pending(), "view() queued a render");
        assert_eq!(app.thumbs.stale_count(), stale_before, "view() marked a picture stale");

        // ...and every body still has a cell to draw, which is what a row
        // reads. A `view` that had to allocate one would be the same mistake
        // wearing a different hat.
        for node in app.doc.nodes().iter().filter(|node| node.is_body()) {
            assert!(app.thumbs.cell(node.id).is_some(), "{} has no cell", node.name);
        }
    }

    /// **A stroke is one picture, at the end, however long it lasted.**
    ///
    /// A stroke calls `remesh_dirty` once per pointer event -- hundreds of
    /// times over a few seconds -- and a picture per stamp would be a full-body
    /// render riding on top of every one of them. So `remesh_dirty` skips a
    /// sculpt drag and `finish_stroke` marks the body once.
    #[test]
    fn a_five_second_stroke_asks_for_exactly_one_picture_after_it_ends() {
        let mut app = app();
        settle(&mut app);
        let body = app.doc.active();

        let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        press(&mut app, centre);
        // Three hundred pointer events is about five seconds of drawing at a
        // display rate, and the frames between them are what a real stroke
        // interleaves.
        for step in 0..300 {
            drag_to(&mut app, Vector::new(centre.x + (step % 20) as f32, centre.y));
            app.thumbs.settle_now();
            update(&mut app, Message::Frame);
            assert!(
                app.shared.take_thumbnail_for_tests().is_none(),
                "a picture was asked for mid stroke, at stamp {step}"
            );
        }
        release(&mut app);

        let request = quiet_frame(&mut app).expect("the stroke never asked for its picture");
        assert_eq!(request.body, body, "the wrong body's picture was asked for");
        assert_eq!(
            request.cell,
            app.thumbs.cell(body).expect("the active body has a cell"),
            "the request names a cell the panel is not reading"
        );
        assert!(quiet_frame(&mut app).is_none(), "one stroke asked for two pictures");
    }

    /// **A press and release that never touched the model costs nothing.**
    ///
    /// The counterpart to the test above, and the reason the stroke's mark
    /// lives inside `finish_stroke`'s edit guard rather than beside it.
    #[test]
    fn a_click_that_changes_nothing_asks_for_no_picture() {
        let mut app = app();
        settle(&mut app);

        // Well clear of the model, so the raycast misses and no recorder opens.
        press(&mut app, Vector::new(4.0, 4.0));
        release(&mut app);

        assert!(quiet_frame(&mut app).is_none(), "a click on empty space asked for a picture");
    }

    /// **A held ctrl+Z asks for one picture, after it stops.**
    ///
    /// This is the failure the settle timer exists for and the reason it is not
    /// `last_activity`. Key repeat runs at about 25 a second and touches no
    /// pointer event at all, so a gate reading pointer activity would be wide
    /// open throughout -- and every undo in the burst would pay a full-body
    /// render on top of its own remesh.
    ///
    /// The burst is checked by comparing the timer against ITSELF rather than
    /// by asserting "not settled yet". The second form would be asserting that
    /// the machine did not deschedule this thread for 200 ms, which under a
    /// full suite it sometimes does, and a test that fails on a loaded runner
    /// teaches people to rerun it.
    #[test]
    fn a_held_undo_asks_for_one_picture_only_once_it_stops() {
        let mut app = app();
        let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        for _ in 0..4 {
            press(&mut app, centre);
            drag_to(&mut app, Vector::new(centre.x + 6.0, centre.y));
            release(&mut app);
        }
        settle(&mut app);
        // Driven off what the history really holds: an undo with nothing left
        // to undo changes no geometry and restarts no timer.
        let available = app.history.stats().undo_entries;
        assert!(available >= 3, "the fixture has only {available} entries to undo");

        for step in 0..available {
            // Pretend a long quiet gap, so the gate is wide open going in.
            app.thumbs.settle_now();
            let opened = app.thumbs.last_change();
            update(&mut app, Message::Undo);
            assert!(
                app.thumbs.last_change() > opened,
                "undo {step} of a held ctrl+Z did not restart the settle timer, so the next \
                 frame would pay a full-body render on top of the undo's own remesh -- and none \
                 of these undos touches the pointer, which is what a gate on `last_activity` \
                 would be reading"
            );
        }

        assert!(quiet_frame(&mut app).is_some(), "the undo burst never asked for its picture");
        assert!(quiet_frame(&mut app).is_none(), "an undo burst asked for two pictures");
    }

    /// **Nothing about how a body is PRESENTED stales its picture.**
    ///
    /// Visibility, solo, rename, folder membership and selection all leave the
    /// geometry exactly as it was, so soloing sixty-four bodies has to cost
    /// zero renders. A staleness rule hung off "the document changed" rather
    /// than off "a brick was marked dirty" would fail every line of this.
    #[test]
    fn hiding_soloing_renaming_and_reordering_ask_for_nothing() {
        let mut app = app();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let second = app.doc.active();
        settle(&mut app);

        let gestures: [Gesture; 8] = [
            ("hiding a body", |app, id| update(app, Message::BodyVisibilityToggled(id))),
            ("showing it again", |app, id| update(app, Message::BodyVisibilityToggled(id))),
            ("selecting another row", |app, _| {
                let first = app.doc.nodes()[0].id;
                update(app, Message::BodySelected(first));
            }),
            ("entering solo", |app, id| {
                update(app, Message::BodySelected(id));
                update(app, Message::SoloEntered(id));
            }),
            ("leaving solo", |app, _| update(app, Message::SoloExited)),
            ("starting a rename", |app, id| update(app, Message::BodyRenameBegan(id))),
            ("finishing a rename", |app, _| {
                update(app, Message::BodyRenameEdited("Renamed".into()));
                update(app, Message::BodyRenameSubmitted);
            }),
            ("grouping the row into a folder", |app, _| update(app, Message::BodyGrouped)),
        ];

        for (what, gesture) in gestures {
            gesture(&mut app, second);
            assert!(
                quiet_frame(&mut app).is_none(),
                "{what} asked for a picture, which it must never do -- the geometry did not change"
            );
        }
    }

    /// A new body gets a cell of its own and one picture; a deleted body gives
    /// its cell back and asks for nothing more.
    #[test]
    fn a_new_body_is_drawn_once_and_a_deleted_one_gives_its_cell_back() {
        let mut app = app();
        settle(&mut app);
        let first = app.doc.active();
        let first_cell = app.thumbs.cell(first).expect("the starting ball has a cell");

        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        let cube_cell = app.thumbs.cell(cube).expect("a new body gets a cell");
        assert_ne!(cube_cell, first_cell, "the new body was given the ball's cell");

        let request = quiet_frame(&mut app).expect("a new body was never drawn");
        assert_eq!(request.body, cube);
        assert_eq!(request.cell, cube_cell);
        assert!(quiet_frame(&mut app).is_none(), "adding one body asked for two pictures");

        update(&mut app, Message::BodyDeleted);
        assert_eq!(app.thumbs.cell(cube), None, "a deleted body kept its cell");
        assert_eq!(app.thumbs.cell(first), Some(first_cell), "the delete moved another picture");

        // **A delete DOES redraw what is left, and that is the rule working
        // rather than the rule leaking.** `remove_body` pairs the pool's forget
        // with `mark_everything_dirty` -- it has to, or a brick the pool
        // refused while it was full stays missing -- so every surviving body's
        // bricks really were marked dirty and every surviving picture really is
        // a guess. What must never happen is a request naming the body that has
        // gone: its cell is back on the free list and may belong to someone
        // else by the time the drain runs.
        let mut asked_for = Vec::new();
        while let Some(request) = quiet_frame(&mut app) {
            assert_ne!(request.body, cube, "a picture was asked for of a deleted body");
            asked_for.push(request.body);
            assert!(asked_for.len() <= 4, "the delete never stopped asking for pictures");
        }
        assert_eq!(asked_for, vec![first], "the delete redrew something other than what is left");
    }

    /// Turning the pictures off costs nothing and turning them back on redraws
    /// them all, because nothing was kept up to date in between.
    #[test]
    fn the_pictures_switch_stops_and_restarts_the_renders() {
        let mut app = app();
        settle(&mut app);

        update(&mut app, Message::ThumbnailsToggled);
        assert!(!app.thumbnails);
        // A stroke with the pictures off must not queue one.
        let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        press(&mut app, centre);
        drag_to(&mut app, Vector::new(centre.x + 6.0, centre.y));
        release(&mut app);
        assert!(
            quiet_frame(&mut app).is_none(),
            "a render was queued with the pictures switched off"
        );

        update(&mut app, Message::ThumbnailsToggled);
        assert!(app.thumbnails);
        assert!(
            quiet_frame(&mut app).is_some(),
            "turning the pictures back on left them showing whatever they held when they went off"
        );
        // ...and it is not a document change.
        assert_eq!(app.thumbs.stale_count(), 0, "one body took more than one render to come back");
    }
}
