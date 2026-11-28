// SPDX-License-Identifier: AGPL-3.0-only

//! The mask: how it is drawn, the standing card, the whole-mask verbs and
//! filters, the generated masks, the mask cutter, and splitting or parting a
//! body along a mask or a line. Moved out of `app.rs` (issue 189).

use super::*;

impl Hani {
    /// Tell the renderer how to draw the mask: the active body's polarity, the
    /// bodies that disagree with it, and the tint the view preferences add up
    /// to.
    ///
    /// **Here, on the every-message path, for the reason the visibility pass
    /// itself is here.** The set of things that change any of the three is
    /// bigger than it looks -- the toggle, the slider, the held peek key,
    /// choosing another body, an undo that puts a polarity back -- and a list of
    /// them is a list that goes silently out of date. What it would go out of
    /// date INTO is a mask drawn with the wrong polarity, which shows protected
    /// material as free. Two locks, two word writes and a walk of at most
    /// `MAX_BODIES` rows asking each one a bool, so the frame tick can afford it
    /// unconditionally; it allocates nothing, which
    /// [`Hani::publish_visibility`]'s contract requires of everything in it.
    ///
    /// # One word cannot answer for a document, so it does not have to
    ///
    /// A mask is per BODY and `Uniforms::mask_inverted` is one word for the
    /// whole draw. The polarity published here is the ACTIVE body's, and every
    /// body that disagrees with it is named in a set the pool consults per
    /// bucket -- see [`hani_gpu::MaskPolarity`]. Increment 24 shipped Invert
    /// and Mask All without that set, and its review found what that costs: a
    /// Mask All on a body that is not the active one drew that body's stored
    /// zeros through the active body's polarity, so a fully protected body sat
    /// on screen with no tint on it at all.
    ///
    /// The set is relative to the word rather than absolute so that a document
    /// where every body agrees -- which is every document that has not used the
    /// verbs -- publishes an empty set and costs the draw nothing.
    pub(super) fn publish_mask_view(&mut self) {
        let inverted =
            self.doc.volume(self.doc.active()).is_some_and(|volume| volume.mask().inverted());
        self.opposite_polarity.clear();
        for node in self.doc.nodes() {
            if let Some(volume) = node.volume()
                && volume.mask().inverted() != inverted
            {
                self.opposite_polarity.push(node.id);
            }
        }
        self.shared.set_opposite_polarity(&self.opposite_polarity);
        // The peek key overrides the toggle rather than being overridden by it:
        // it is what a user who switched the tint off and forgot reaches for.
        let shown = self.show_mask || self.mask_peek;
        self.shared.set_mask_view(crate::viewport::MaskView {
            inverted,
            tint: if shown { self.mask_tint } else { 0.0 },
        });
        // Published beside the mask's view for the same reason and at the same
        // cost: a fixed array and no allocation, on the frame path.
        self.shared.set_paint_view(crate::viewport::PaintView {
            shown: self.show_paint,
            flat: self.flat_paint,
            palette: self.palette.shader_palette(),
            paints: hani_gpu::PaintBlock::from(self.doc.paints()),
        });
        // The gain is what makes the setting a size in millimetres: a feature
        // `w` voxels across reads a curvature of `4 / w`, so `w / 4` brings a
        // feature of exactly that size to a full crease, and one twice as
        // broad to half of one. The document's voxel size is in the ratio, so
        // the same model at a finer voxel draws the same creases.
        let overhang = self.overhang_view();
        self.shared.set_cavity_view(crate::viewport::CavityView {
            strength: if self.show_cavity { CAVITY_STRENGTH } else { 0.0 },
            gain: CAVITY_FEATURE_MM / self.doc.voxel_size() / 4.0,
            overhang,
        });
    }

    /// Rebuild the standing mask card, cheaply enough to do it every message.
    ///
    /// **This runs on the frame tick**, so what it costs when nothing has
    /// changed is what matters, and what it must cost is nothing at all: not a
    /// `mask_fill`, and not a `format!` either -- `publish_visibility`'s own
    /// contract two functions up is that the frame path allocates NOTHING, and
    /// this is at the bottom of it. So everything above the cache gate is a walk
    /// of at most `MAX_BODIES` rows asking each body a map-empty question, both
    /// counts stay a `usize` until they are known to be new, and every `String`
    /// in here is built below the gate.
    ///
    /// Three gates, in cheapest-first order: a document with no protection
    /// anywhere leaves with `None`; a document whose active body, revision and
    /// two counts are all unchanged leaves without touching the card; and only
    /// a genuine change pays for [`hani_core::Volume::mask_fill`].
    ///
    /// # Every masked body is counted somewhere, and that is the point
    ///
    /// The card is what makes "a mask is active and nothing on screen says so"
    /// unreachable, and that has to be a DOCUMENT-wide property rather than an
    /// active-body one. Increment 24's review found it was not: only the active
    /// body and the masked bodies that were HIDDEN were counted, so a body that
    /// was masked, visible and not active fell through both. Mask all on a
    /// cube, then click another body, and the card vanished while a fully
    /// protected cube sat on screen; the first the user heard of it was a clip
    /// plane sparing the whole thing.
    pub(super) fn refresh_mask_card(&mut self) {
        let active = self.doc.active();
        let active_masked = self.doc.volume(active).is_some_and(|volume| !volume.mask().is_free());

        // Masked bodies that are not the one the headline is about, split by
        // whether the user can see them. The active body is excluded from both:
        // the counts mean "as well as what this card already says", so a hidden
        // active body counting itself would be a card announcing its own
        // subject twice.
        let mut off_screen = 0usize;
        let mut elsewhere = 0usize;
        for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
            if !node.is_body() || node.id == active {
                continue;
            }
            if node.volume().is_some_and(|volume| !volume.mask().is_free()) {
                if *shown {
                    elsewhere += 1;
                } else {
                    off_screen += 1;
                }
            }
        }

        if !active_masked && off_screen == 0 && elsewhere == 0 {
            self.mask_card = None;
            return;
        }

        // Zero is the revision of a card about a body carrying no mask of its
        // own, and no live mask can collide with it: `revision` counts writes,
        // so a mask that is not free has had at least one.
        let revision =
            if active_masked { self.doc.active_volume().mask().revision() } else { NO_MASK_HERE };

        if let Some(card) = &mut self.mask_card
            && card.body == active
            && card.revision == revision
        {
            if card.off_screen_count != off_screen || card.elsewhere_count != elsewhere {
                card.rewrite_the_other_counts(off_screen, elsewhere);
            }
            return;
        }

        if !active_masked {
            // Nothing on the active body, but protection is out there on
            // something else. No percentage, because there is no one body for
            // it to be about.
            let (headline, hidden) = MaskCard::about_other_bodies(off_screen, elsewhere);
            self.mask_card = Some(MaskCard {
                body: active,
                revision,
                off_screen_count: off_screen,
                elsewhere_count: elsewhere,
                headline,
                off_screen: hidden,
                elsewhere: String::new(),
            });
            return;
        }

        let name = self.doc.node(active).map_or("this body", |node| node.name.as_str()).to_string();
        let percent = (self.doc.active_volume().mask_fill() * 100.0).round() as u32;
        self.mask_card = Some(MaskCard {
            body: active,
            revision,
            off_screen_count: off_screen,
            elsewhere_count: elsewhere,
            // Never "0%": a mask that exists but rounds to nothing still blocks
            // a cut and still weakens a stroke, and a card reading zero is a
            // card saying "ignore me".
            headline: format!("MASK — {name}, {}%", percent.max(1)),
            off_screen: MaskCard::hidden_count_line(off_screen),
            elsewhere: MaskCard::elsewhere_count_line(elsewhere),
        });
    }

    // ------------------------------------------------- the whole-mask verbs

    /// Throw the active body's mask away.
    ///
    /// **The map is MOVED into the history entry and never copied**, which is
    /// the [`Change::NodeRemoved`] trick: a gigabyte of protection is cleared
    /// with no allocation, peak memory does not rise, and `resident_bytes`
    /// falls back to what it was before the mask existed. The bytes go against
    /// the reclaim allowance rather than the 256 MB stroke budget, so a Clear
    /// does not evict the strokes behind it.
    pub(super) fn clear_mask(&mut self) {
        let body = self.doc.active();
        if self.doc.active_volume().mask().is_free() {
            self.status = format!("{} carries no mask", self.active_body_name());
            return;
        }
        let volume = self.doc.active_volume_mut();
        let cleared = volume.mask().cleared(false);
        let previous = volume.replace_mask(cleared);
        self.commit_whole_mask(body, previous);
        self.status = format!("cleared the mask on {}", self.active_body_name());
    }

    /// Flip which side of the mask is protected.
    ///
    /// One bool, whatever the mask holds: no walk, no allocation, no brick
    /// marked for a remesh. Without that, ctrl+I on a lightly masked
    /// 45,567-brick model would allocate 1.04 GiB from one keystroke.
    pub(super) fn invert_mask(&mut self) {
        let body = self.doc.active();
        let edit = self.doc.active_volume_mut().flip_mask_polarity();
        let before = self.history.stats();
        self.history.push(Entry::stroke(body, edit));
        self.record_history(before);
        self.unsaved = true;
        // The picture follows from `publish_mask_view`, which `update` runs
        // after every message: the polarity is a uniform and not a vertex
        // attribute, so there is deliberately nothing to remesh. The thumbnail
        // is a different matter -- see `geometry_changed`'s caller list.
        self.thumbs.geometry_changed(body);
        self.status = format!("inverted the mask on {}", self.active_body_name());
    }

    /// Protect the whole body: clear, then invert, as ONE change.
    ///
    /// They are one change and not two because the polarity rides inside the
    /// field, so `cleared(true)` IS clear-then-invert -- and undoing it puts
    /// both halves back together, which is the only state either half means
    /// anything in.
    pub(super) fn mask_all(&mut self) {
        let body = self.doc.active();
        if self.doc.active_volume().mask().protects_everything() {
            self.status = format!("{} is already fully masked", self.active_body_name());
            return;
        }
        let volume = self.doc.active_volume_mut();
        let all = volume.mask().cleared(true);
        let previous = volume.replace_mask(all);
        self.commit_whole_mask(body, previous);
        self.status = format!("masked all of {}", self.active_body_name());
    }

    /// Push the entry a whole-mask change produced and put the picture back in
    /// step with it.
    fn commit_whole_mask(&mut self, body: NodeId, previous: MaskField) {
        let before = self.history.stats();
        self.history.push(Entry::new(vec![Change::WholeMask { body, mask: Box::new(previous) }]));
        self.record_history(before);
        self.unsaved = true;
        self.thumbs.geometry_changed(body);
        self.remesh_dirty();
    }

    /// The amount slider moved.
    ///
    /// **Absolute.** The first non-zero amount of a gesture takes the mask off
    /// the body and keeps it as a snapshot; every amount after that is applied
    /// to that same snapshot, so the result depends on where the slider IS and
    /// never on how it got there. Dragging back to zero moves the snapshot back
    /// onto the body bit for bit and ends the gesture with no entry at all,
    /// which is why "blur at 0.0 is a no-op" is literally true rather than
    /// approximately so.
    ///
    /// # What one step of the slider costs, said plainly
    ///
    /// A whole-body filter and a whole-body remesh, per step. The remesh is the
    /// larger half and the plan already names it: 71 ms on the reference dragon
    /// and roughly 475 ms at the 45,567 bricks the pool is sized for. The
    /// step is 0.05, so a full sweep is at most twenty of those. Two things
    /// keep it usable rather than one: the uniform fast path in
    /// [`hani_core::MaskField::filtered`] walks only the shell where
    /// protection actually changes, and a slider emits nothing when the snapped
    /// value has not moved. **If this becomes the complaint, the answer is the
    /// two-halves `iced::Task` increment 25a already specifies for the
    /// generators, not a coarser step.**
    pub(super) fn mask_amount_changed(&mut self, amount: f32) {
        let amount = amount.clamp(0.0, 1.0);
        self.mask_amount = amount;
        let body = self.doc.active();

        // A gesture belongs to one body and one filter. Either changing under
        // it ends it where it stands rather than re-applying one body's
        // snapshot onto another's mask.
        if self
            .mask_grab
            .as_ref()
            .is_some_and(|grab| grab.body != body || grab.filter != self.mask_filter)
        {
            self.commit_mask_filter();
        }

        if amount <= 0.0 {
            // Put the snapshot back, exactly as it was, and push nothing.
            if let Some(grab) = self.mask_grab.take()
                && let Some(volume) = self.doc.volume_mut(grab.body)
            {
                volume.replace_mask(grab.snapshot);
                self.remesh_dirty();
            }
            return;
        }

        let filter = self.mask_filter;
        if self.mask_grab.is_none() {
            if let Some(why) = self.refuse_the_filter(filter) {
                self.status = why;
                log::warn!("{}", self.status);
                self.mask_amount = 0.0;
                return;
            }
            let snapshot = self.doc.active_volume_mut().take_mask();
            self.mask_grab = Some(MaskGrab { body, filter, snapshot });
        } else {
            // Drop what the last change of this gesture left behind BEFORE the
            // next one is built, so the peak stays at two masks and not three.
            drop(self.doc.active_volume_mut().take_mask());
        }

        let Some(grab) = &self.mask_grab else {
            return;
        };
        let next = grab.snapshot.filtered(filter, amount);
        self.doc.active_volume_mut().replace_mask(next);
        self.remesh_dirty();
    }

    /// The amount slider was let go: one entry for the whole drag.
    ///
    /// The slider goes back to zero because the filter is absolute and the
    /// amount is a property of the gesture rather than of the document -- see
    /// the field's own documentation.
    pub(super) fn mask_amount_released(&mut self) {
        self.commit_mask_filter();
        self.mask_amount = 0.0;
    }

    /// Commit a live filter gesture, if there is one.
    pub(super) fn commit_mask_filter(&mut self) {
        let Some(grab) = self.mask_grab.take() else {
            return;
        };
        let verb = grab.filter.done();
        let body = grab.body;
        let name = self.body_name(body);
        self.commit_whole_mask(body, grab.snapshot);
        self.status = format!("{verb} the mask on {name}");
    }

    /// Commit a filter gesture whose slider is no longer there to release it.
    ///
    /// **On the frame tick, and two `Option` comparisons.** The amount slider
    /// lives in the right-click tool menu, and the menu can go away mid-drag --
    /// Escape closes it, and Escape is also how a user dismisses anything. The
    /// active body can change under it too. Either way the `MaskAmountReleased`
    /// that would have committed the drag is never coming, and what is left is
    /// a filtered mask with no history entry and no `unsaved`: real work that
    /// ctrl+Z cannot reach and that quitting would not even prompt about.
    ///
    /// Here rather than in each of the arms that closes a menu, because a list
    /// of those is a list that goes out of date silently -- the same argument
    /// `publish_visibility` makes about the eye.
    pub(super) fn settle_an_orphaned_mask_grab(&mut self) {
        let orphaned = self
            .mask_grab
            .as_ref()
            .is_some_and(|grab| self.menu.is_none() || grab.body != self.doc.active());
        if orphaned {
            self.commit_mask_filter();
            self.mask_amount = 0.0;
        }
    }

    /// Why a global filter would not fit under the memory ceiling.
    ///
    /// **Not optional, and not a tidy-up.** A filter needs the old mask while
    /// it writes the new one, and the history entry then holds the old one, so
    /// the peak is `field + 2 x mask`. A fully painted mask is 25% of its field
    /// -- 32,768 bytes a brick against 131,072 -- which makes that peak 1.5
    /// times the field: on the measured 0.0565 mm dragon, 6.22 GiB against a
    /// 6 GiB ceiling, from one button press. `History::trim` keeps a single
    /// over-budget entry by design, so the undo budget cannot save it either.
    ///
    /// **Clear, Invert and Mask All are deliberately NOT gated**, and that is
    /// the same paragraph of the plan read to its end rather than a narrowing
    /// of it: all three are O(1) in memory, none of them builds a second mask,
    /// and refusing the one verb that GIVES the memory back would trap a user
    /// with a mask they cannot remove at exactly the moment they need to.
    fn refuse_the_filter(&self, filter: MaskFilter) -> Option<String> {
        let mask = self.doc.active_volume().mask();
        let (why, _coarser) =
            mask.would_fit(mask.bytes(), self.doc_stats.resident_bytes, self.doc.voxel_size())?;
        Some(format!("could not {} the mask on {}: {why}", filter.verb(), self.active_body_name()))
    }

    // ------------------------------------------------- the generated masks

    /// The recipe a generator button means, with the sliders as they stand.
    fn recipe_for(&self, which: MaskGenerator) -> MaskRecipe {
        match which {
            MaskGenerator::Cavity => MaskRecipe::Cavity { feature_mm: self.mask_feature_mm },
            MaskGenerator::Smoothness => {
                MaskRecipe::Smoothness { feature_mm: self.mask_feature_mm }
            }
            MaskGenerator::Thickness => {
                MaskRecipe::Thickness { voxels: self.mask_thickness_voxels }
            }
        }
    }

    /// Build a whole mask out of the active body's geometry.
    ///
    /// # It runs on this thread, and the plan asked for two halves
    ///
    /// Increment 25a specifies an `iced::Task` in two halves like
    /// `ImportRequested` / `ImportLoaded`, with a progress tick. **It is not
    /// written that way and the reason is the document, not the effort.** An
    /// import builds a fresh volume from a path, so the future owns everything
    /// it touches; a generator reads the volume the document is holding, and
    /// there is no way to lend that across an await. The three ways out are all
    /// worse than the hitch: cloning the volume is a second copy of up to four
    /// gigabytes, moving it out and back leaves the body missing from the
    /// document mid-flight -- visible, and editable -- and an `Arc<Volume>`
    /// rewrites every mutating path in the crate.
    ///
    /// What it costs, measured on the reference dragon: **39.6 ms on 24 cores**,
    /// against **71.0 ms for the whole-document remesh that has to follow it**
    /// and that is already synchronous. So moving only the pass off-thread would
    /// hide a third of the pause and buy a mid-flight state that nothing else in
    /// this application has. Extrapolated to the 45,567-brick ceiling the pass
    /// is about 265 ms, and if that becomes the complaint the answer is to move
    /// the remesh and the pass together, not the pass alone.
    ///
    /// The elapsed time goes to stderr past [`hani_core::SLOW_SPLIT`], which
    /// is the same instrument `apply_split` uses and for the same reason: the
    /// status bar is already carrying the result, and the perf readout cannot
    /// see a frame that never came.
    pub(super) fn generate_mask(&mut self, which: MaskGenerator) {
        let body = self.doc.active();
        let recipe = self.recipe_for(which);
        let name = self.active_body_name();

        if let Some(why) = self.refuse_the_generator(recipe) {
            self.status = why;
            log::warn!("{}", self.status);
            return;
        }

        let started = web_time::Instant::now();
        let mask = self.doc.active_volume().generated_mask(recipe);
        let elapsed = started.elapsed();
        if mask.is_free() {
            // No entry, no `unsaved`, no remesh. A generator that found nothing
            // has not changed the document, and pushing an entry for it would
            // spend the undo budget on putting back what is already there.
            self.status = format!("{}: nothing on {name} matches", recipe.verb());
            return;
        }

        let previous = self.doc.active_volume_mut().replace_mask(mask);
        self.commit_whole_mask(body, previous);
        self.status = format!("{} on {name}", recipe.done());
        if elapsed > hani_core::SLOW_SPLIT {
            eprintln!("{} on {name} took {} ms", recipe.verb(), elapsed.as_millis());
        }
    }

    /// Why a generated mask would not fit under the memory ceiling.
    ///
    /// **The refusal is part of this feature rather than a nicety.** A
    /// hand-painted mask collapses to tiles almost everywhere; a generated one
    /// writes a value at every surface voxel, so the +25% of the mask
    /// arithmetic is what is actually paid -- and the thickness walk copies the
    /// field on top of that. Predicting it is
    /// `hani_core::Volume::generated_mask_demand`'s job and refusing on it is
    /// this one's.
    fn refuse_the_generator(&self, recipe: MaskRecipe) -> Option<String> {
        let volume = self.doc.active_volume();
        let extra = volume.generated_mask_demand(recipe);
        let (why, _coarser) =
            volume.mask().would_fit(extra, self.doc_stats.resident_bytes, self.doc.voxel_size())?;
        Some(format!("could not {} on {}: {why}", recipe.verb(), self.active_body_name()))
    }

    /// Protect the half of every visible body that a cut would have removed.
    ///
    /// **The plane is the cut's own and so is the gesture**: ctrl held at the
    /// end of a cut drag writes protection instead of taking material away.
    /// That is the whole of increment 25c's "no new gesture" -- the brick
    /// classification, the ray pair and the plane are all the ones `finish_cut`
    /// already had, and what changes is which field the half-space is written
    /// into.
    ///
    /// Across every DRAWN body, for the reason `Document::clip` gives: a cut is
    /// a line the user draws across what they can see, and a mask drawn the same
    /// way has to mean the same thing. One compound entry of N
    /// `Change::WholeMask`, so one ctrl+Z takes the whole gesture back.
    pub(super) fn mask_cutter(
        &mut self,
        cutters: &[&[hani_core::ClipPlane]],
        shape: cut::CutShape,
    ) {
        let feather_mm = HALFSPACE_FEATHER_VOXELS * self.doc.voxel_size();
        let drawn = self.drawn_nodes();
        let bodies: Vec<NodeId> = self
            .doc
            .nodes()
            .iter()
            .enumerate()
            .filter(|(index, _)| drawn.get(*index).copied().unwrap_or(false))
            .map(|(_, node)| node.id)
            .collect();

        let active = self.doc.active();
        let mut changes = Vec::new();
        let mut touched = 0usize;
        // Two facts the old status line confused, and the confusion was the
        // whole of its second defect. This loop walks every DRAWN body, while
        // `split_the_mask_off` takes `doc.active()` and nothing else -- so "the
        // body this sentence is about" and "the body the split would take" are
        // not the same body. It named the second while describing the first.
        let mut only: Option<NodeId> = None;
        let mut touched_active = false;
        for body in bodies {
            let Some(volume) = self.doc.volume_mut(body) else {
                continue;
            };
            // Straight to `cutter_mask` rather than through `MaskRecipe`, and
            // that is deliberate. `MaskRecipe` derives `Copy` and travels
            // inside `Message`, so a slice in it would need a lifetime, an
            // `Arc` would cost the `Copy`, and a fixed array would put several
            // hundred bytes into every message the application sends. The
            // recipe stays the single-plane one it always was.
            let mask = volume.cutters_mask(cutters, feather_mm);
            if mask.is_free() {
                // The plane missed this body entirely, so it keeps whatever it
                // was carrying. Replacing it with an empty mask would throw a
                // hand-painted one away for a line drawn somewhere else.
                continue;
            }
            let previous = volume.replace_mask(mask);
            changes.push(Change::WholeMask { body, mask: Box::new(previous) });
            self.thumbs.geometry_changed(body);
            touched += 1;
            // `then_some` rather than a first-write guard: this clears itself
            // on the second body, so `only` is safe to read anywhere rather
            // than only under a `touched == 1` arm somewhere further down.
            only = (touched == 1).then_some(body);
            touched_active |= body == active;
        }

        if changes.is_empty() {
            self.status = "the mask gesture missed the model".to_string();
            return;
        }
        let before = self.history.stats();
        self.history.push(Entry::new(changes));
        self.record_history(before);
        self.unsaved = true;
        self.remesh_dirty();
        // Names what was protected, and then names the thing to do next.
        // **Circling a lump and being told only that it is masked is half an
        // answer**: the reason to select rather than cut is to lift the lump
        // off as its own body, and a user who is not told that is available
        // will reach for the panel and hunt for it.
        //
        // Which is exactly what happened. This used to say "split it off from
        // the BODIES panel", and the BODIES panel has no such button -- its
        // verb row is five icons and its Split fires `Message::BodySplit`, the
        // connected-component split, a different operation. See `split_advice`.
        let what = match shape {
            cut::CutShape::Line => "that half",
            cut::CutShape::Curve => "inside the curve",
            cut::CutShape::Lasso => "inside the lasso",
        };
        let where_ = match only {
            Some(body) => self.body_name(body),
            None => format!("{touched} bodies"),
        };
        self.status = match shape {
            cut::CutShape::Line => format!("masked {what} of {where_}"),
            _ => {
                format!("masked {what} on {where_} — {}", split_advice(touched_active, touched))
            }
        };
    }

    /// Split the active body in two along its mask.
    ///
    /// No preview card, unlike `split_active_body`: a masked split always makes
    /// exactly two bodies, so the question a card exists to answer -- how many
    /// parts is this -- is already answered, and the tint on screen is a better
    /// preview than any card could be.
    ///
    /// The three lines after the split are the unit `apply_split`,
    /// `Hani::remove_body` and `Hani::apply_merge` all document: the source
    /// stops existing, its bricks are in nobody's dirty set, so
    /// `SharedFrame::forget_body` is what releases its slots and the
    /// whole-document remesh is the other half.
    pub(super) fn split_the_mask_off(&mut self) {
        let source = self.doc.active();
        let name = self.active_body_name();

        let visible = self
            .doc
            .index_of(source)
            .and_then(|index| self.shown.get(index).copied())
            .unwrap_or(true);
        if !visible {
            self.status = format!(
                "could not split {name}: it is not on screen, and its halves would not be either"
            );
            return;
        }
        if let Some(why) = self.doc.split_masked_guard(source) {
            self.status = format!("could not split {name}: {why}");
            return;
        }

        let started = web_time::Instant::now();
        let Some(outcome) = self.doc.split_masked(source) else {
            // The guard covers the two O(1) cases; this is the third, and it
            // needs the pass to find out: a mask that is neither empty nor
            // total can still sit entirely off the material.
            self.status = format!("the mask on {name} does not divide it");
            return;
        };
        self.shared.forget_body(source);
        self.doc.mark_everything_dirty();

        let before = self.history.stats();
        self.history.push(outcome.entry);
        self.unsaved = true;
        self.status = format!("split {name} along its mask");
        self.record_history(before);
        if started.elapsed() > hani_core::SLOW_SPLIT {
            eprintln!("masked split of {name} took {} ms", outcome.elapsed.as_millis());
        }

        self.refresh_model_radius();
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// Part the active body in two along a drawn line, with keys.
    ///
    /// The cut tool's own gesture with alt held: the plane is the one a cut
    /// would have used, and instead of a side going the body becomes two
    /// bodies with pegs on one and sockets in the other. See
    /// `hani_core::part`.
    ///
    /// **The ACTIVE body and no other**, where a cut crosses everything
    /// drawn. A cut is an eraser and parting is a decision about one part:
    /// what its two halves are called, where their keys go, what its
    /// neighbours do not get done to them. Every refusal names the body.
    ///
    /// The shape of everything after the plan is `split_the_mask_off`'s, for
    /// that function's reasons: the source's slots are released in the frame
    /// its halves are marked dirty, and the whole gesture is one undo entry.
    pub(super) fn part_along(&mut self, planes: &[hani_core::ClipPlane], shape: cut::CutShape) {
        let source = self.doc.active();
        let name = self.active_body_name();
        let (cut::CutShape::Line, [plane]) = (shape, planes) else {
            self.status = format!("not parted: parting {name} takes one straight line across it");
            return;
        };

        let visible = self
            .doc
            .index_of(source)
            .and_then(|index| self.shown.get(index).copied())
            .unwrap_or(true);
        if !visible {
            self.status = format!(
                "could not part {name}: it is not on screen, and its halves would not be either"
            );
            return;
        }
        if let Some(why) = self.doc.split_guard(source) {
            self.status = format!("could not part {name}: {why}");
            return;
        }
        let Some(volume) = self.doc.volume(source) else {
            return;
        };

        // The plan first, and read only: it is what says whether the line
        // meets the body at all, and what goes in the status line about the
        // faces that could not be keyed.
        let plan = volume.part_plan(*plane, self.key_spec);
        if !plan.cuts() {
            self.status = format!("not parted: the line misses {name}");
            return;
        }
        let summary = plan.summary();
        let Some(outcome) = self.doc.part(source, &plan) else {
            // The plan found a face and the cut left one side empty, which a
            // plane grazing the body can do.
            self.status = format!("not parted: the line only grazes {name}");
            return;
        };
        self.shared.forget_body(source);
        self.doc.mark_everything_dirty();

        let before = self.history.stats();
        self.history.push(outcome.entry);
        self.unsaved = true;
        self.status = format!("parted {name} in two: {summary}");
        self.record_history(before);

        self.refresh_model_radius();
        self.remesh_dirty();
        self.refresh_overlay();
    }
}

/// The mask TOOL: the mode, the gesture, and what one mask stroke costs.
///
/// Its own module because every test here is about the tool rather than about
/// the brush -- what a press means, which map it writes, and what the release
/// pushes onto history.
#[cfg(test)]
mod mask_tool_tests {
    use super::*;
    use crate::tablet::PenState;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    fn centre_of_viewport() -> Vector {
        Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
    }

    /// Pointer events routed through `update` rather than straight into
    /// `on_pointer`.
    ///
    /// The difference matters here and nowhere else in this file: the standing
    /// mask card is rebuilt in `publish_visibility`, which `update` runs after
    /// every message and which `on_pointer` never reaches. A test that poked
    /// the pointer handler directly would paint a mask and then find no card,
    /// which is a fact about the test harness rather than about the
    /// application.
    fn press(app: &mut Hani, at: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
    }

    fn drag_to(app: &mut Hani, at: Vector) {
        update(app, Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }));
    }

    fn release(app: &mut Hani) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short mask drag across the middle of the model.
    fn mask_drag(app: &mut Hani) {
        let centre = centre_of_viewport();
        press(app, centre);
        drag_to(app, Vector::new(centre.x + 20.0, centre.y));
        drag_to(app, Vector::new(centre.x + 40.0, centre.y));
        release(app);
    }

    /// Every field value of the active body, in a fixed order, for a bit-for-bit
    /// comparison across a gesture.
    fn field_of(app: &Hani) -> Vec<(hani_core::BrickCoord, Vec<f32>)> {
        let volume = app.doc.active_volume();
        let mut coords: Vec<_> = volume.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        coords
            .into_iter()
            .map(|coord| {
                let origin = coord.origin();
                let dim = hani_core::BRICK_DIM as i32;
                let values = (0..dim)
                    .flat_map(|z| {
                        (0..dim).flat_map(move |y| (0..dim).map(move |x| glam::IVec3::new(x, y, z)))
                    })
                    .map(|local| volume.sample_voxel(origin + local))
                    .collect();
                (coord, values)
            })
            .collect()
    }

    /// `m` enters and leaves, and the brush selection underneath is untouched.
    ///
    /// The second half is the one worth pinning. Mask is a TOOL and not an
    /// eighth brush, so entering it must not disturb which brush a user will be
    /// back on when they leave -- and the digits, `s`, `[` and `]` all keep
    /// working on the mask because it borrows the brush's radius and falloff.
    #[test]
    fn m_enters_and_leaves_mask_mode_and_leaves_the_brush_alone() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Clay));
        let radius = app.brush.radius;

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Mask, "m did not enter mask mode");
        assert_eq!(app.brush.kind, BrushKind::Clay, "entering mask mode changed the brush");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt, "pressing the live tool did not go back to sculpting");
        assert_eq!(app.brush.kind, BrushKind::Clay, "leaving mask mode changed the brush");
        assert_eq!(app.brush.radius, radius, "the mask tool kept the brush's radius");
    }

    /// `m` really is the key, decoded the same way every other shortcut is.
    #[test]
    fn the_m_key_decodes_to_the_mask_tool() {
        assert!(
            matches!(
                crate::viewport::shortcut("m", false, false, false),
                Some(Message::ToolChanged(Tool::Mask))
            ),
            "m does not reach the mask tool"
        );
        // Chorded it belongs to the toolkit, like everything else under a
        // modifier that this application has not claimed.
        assert!(crate::viewport::shortcut("m", true, false, false).is_none());
        assert!(crate::viewport::shortcut("m", false, false, true).is_none());
    }

    /// `w` reaches the transform gizmo, and only bare.
    ///
    /// ZBrush's, Blender's and Maya's key for Move, taken rather than invented.
    /// Chorded it belongs to the toolkit: `ctrl+w` closes a window nearly
    /// everywhere and this application has claimed neither form.
    #[test]
    fn the_w_key_decodes_to_the_transform_gizmo() {
        assert!(
            matches!(
                crate::viewport::shortcut("w", false, false, false),
                Some(Message::ToolChanged(Tool::Transform))
            ),
            "w does not reach the transform gizmo"
        );
        assert!(crate::viewport::shortcut("w", true, false, false).is_none());
        assert!(crate::viewport::shortcut("w", false, false, true).is_none());
    }

    /// `h` starts a peek, and only bare.
    ///
    /// ZBrush's own mask-visibility key, taken rather than invented. Chorded it
    /// belongs to the toolkit: `ctrl+h` is a browser and toolkit shortcut in
    /// several places and this application has claimed neither form.
    #[test]
    fn the_h_key_decodes_to_a_mask_peek() {
        assert!(
            matches!(
                crate::viewport::shortcut("h", false, false, false),
                Some(Message::MaskPeekStarted)
            ),
            "h does not start a peek"
        );
        assert!(crate::viewport::shortcut("h", true, false, false).is_none());
        assert!(crate::viewport::shortcut("h", false, false, true).is_none());
    }

    /// **A mask drag changes the mask and not one field value.**
    ///
    /// The whole of what the tool promises. A field that moved here is a mask
    /// tool that has quietly become a sculpt brush, which a user would find in
    /// an exported print rather than on screen.
    #[test]
    fn a_mask_drag_changes_the_mask_and_not_one_field_value() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        let before = field_of(&app);

        mask_drag(&mut app);

        assert!(app.doc.active_volume().mask_fill() > 0.0, "the drag painted no mask at all");
        assert_eq!(field_of(&app), before, "a mask drag moved the field");
    }

    /// **Ctrl lowers the mask with every one of the seven brushes selected.**
    ///
    /// This is the `is_directional` trap, and it is the reason mask mode
    /// computes its own direction. `stroke_direction` gates on
    /// `brush.kind.is_directional()`, which answers false for Smooth, Flatten
    /// and Move -- so a mask painter that reused it would MASK on ctrl+drag for
    /// three of the seven, with nothing on screen to say which three.
    #[test]
    fn ctrl_lowers_the_mask_with_every_brush_selected() {
        for kind in BrushKind::ALL {
            let mut app = app();
            update(&mut app, Message::BrushKindChanged(kind));
            update(&mut app, Message::ToolChanged(Tool::Mask));

            mask_drag(&mut app);
            let raised = app.doc.active_volume().mask_fill();
            assert!(raised > 0.0, "{kind}: the plain drag painted nothing");

            app.control = true;
            for _ in 0..6 {
                mask_drag(&mut app);
            }
            assert!(
                app.doc.active_volume().mask_fill() < raised,
                "{kind}: ctrl+drag did not lower the mask"
            );

            // ...and alt is its synonym, exactly as it is for the brush.
            app.control = false;
            app.alt = true;
            assert_eq!(app.mask_op(), MaskOp::Lower, "{kind}: alt is not ctrl's synonym");
        }
    }

    /// The eraser end of the stylus unmasks, and combines with the modifier the
    /// way it does everywhere else.
    #[test]
    fn the_stylus_eraser_lowers_the_mask() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.mask_op(), MaskOp::Raise);

        app.tablet.simulate(PenState {
            in_proximity: true,
            pressure: 1.0,
            eraser: true,
            tilt: glam::Vec2::ZERO,
        });
        assert_eq!(app.mask_op(), MaskOp::Lower, "the eraser end did not unmask");

        // The two combine rather than override: holding the modifier while
        // using the eraser gives back the additive gesture.
        app.control = true;
        assert_eq!(app.mask_op(), MaskOp::Raise);

        // And shift outranks both, because blur is a different operation rather
        // than a direction.
        app.shift = true;
        assert_eq!(app.mask_op(), MaskOp::Blur);
    }

    /// **A mask-only stroke sets `unsaved` and pushes exactly one history
    /// entry**, and undoing it puts the mask back and marks bricks dirty.
    ///
    /// It did neither before this increment: `end_stroke` recorded field bricks
    /// only, so a mask gesture produced no entry, never set `unsaved`, and did
    /// not even raise the discard prompt on the way out.
    #[test]
    fn a_mask_stroke_is_one_undoable_entry_that_marks_the_document_unsaved() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        let entries = app.history_stats.undo_entries;

        mask_drag(&mut app);

        let painted = app.doc.active_volume().mask_fill();
        assert!(painted > 0.0, "the fixture painted nothing");
        assert!(app.unsaved, "a mask stroke did not mark the document unsaved");
        assert_eq!(
            app.history_stats.undo_entries,
            entries + 1,
            "one mask gesture became {} entries",
            app.history_stats.undo_entries - entries
        );

        app.perf.dirty_bricks = 0;
        app.undo();
        assert_eq!(app.doc.active_volume().mask_fill(), 0.0, "undo did not take the mask off");
        assert!(app.perf.dirty_bricks > 0, "undoing a mask stroke remeshed nothing");

        app.redo();
        assert!(
            (app.doc.active_volume().mask_fill() - painted).abs() < 1.0e-9,
            "redo did not put the mask back"
        );
    }

    /// Escape leaves mask mode ON, and clears the cut.
    ///
    /// A cut is a pending destructive thing and Escape calls it off. A mask is
    /// expensive user work, and Escape is also the key that closes every menu,
    /// so a user dismissing a dropdown must not silently lose the mode they are
    /// painting in.
    #[test]
    fn escape_clears_the_cut_and_leaves_mask_mode_on() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        update(&mut app, Message::MenuClosed);
        assert_eq!(app.tool, Tool::Sculpt, "escape did not disarm the cut");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        update(&mut app, Message::MenuClosed);
        assert_eq!(app.tool, Tool::Mask, "escape threw away the mask mode");
    }

    /// An in-progress resize outranks BOTH modes, which is a change of ordering
    /// from the cut alone.
    ///
    /// A destructive one-shot may reasonably outrank a resize; an ongoing paint
    /// may not, and once the two collapsed into one match on the tool the merged
    /// arm had to take one of the two precedences. It takes the mask's.
    #[test]
    fn a_sizing_drag_outranks_both_modes() {
        for tool in [Tool::Cut, Tool::Mask] {
            let mut app = app();
            app.cursor = Some(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0));
            update(&mut app, Message::ToolChanged(tool));
            update(&mut app, Message::SizingStarted(SizingTarget::Radius));
            assert!(app.sizing.is_some(), "the fixture did not start a resize");

            press(&mut app, centre_of_viewport());
            assert_eq!(
                app.drag.map(|drag| drag.kind),
                Some(DragKind::Sizing),
                "{tool:?} stole a press that belonged to a resize"
            );
        }
    }

    /// The mask keeps its own strength, and the brush gets its own back.
    ///
    /// Draw's default is 0.15, which barely deposits; a user painting a mask
    /// drags the slider up to get a usable one. Sharing the number would make
    /// the very next Draw stroke after leaving mask mode a gouge at full
    /// strength on a scan they were carefully repairing.
    #[test]
    fn the_mask_remembers_its_own_strength_and_the_brush_keeps_its() {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert!((app.brush.strength - 0.15).abs() < 1.0e-6);

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert!(app.brush.strength > 0.9, "the mask arrived at {}", app.brush.strength);

        update(&mut app, Message::BrushStrengthChanged(0.6));
        // Choosing a different brush mid-mask must not disturb it: both sides of
        // the swap resolve to the mask's own slot.
        update(&mut app, Message::BrushKindChanged(BrushKind::Clay));
        assert!((app.brush.strength - 0.6).abs() < 1.0e-6, "the mask forgot its setting");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt);
        assert!(
            (app.brush.strength - 0.15).abs() < 1.0e-6,
            "clay came back at the mask's strength"
        );

        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert!((app.brush.strength - 0.6).abs() < 1.0e-6, "the mask did not get its own back");
    }

    /// The standing card is up whenever anything is masked, whatever tool is
    /// live, and it names the body.
    ///
    /// This is the mechanism that makes "a mask is active and nothing on screen
    /// says so" unreachable, so the assertion is about it being UNCONDITIONAL
    /// rather than about its wording.
    #[test]
    fn the_mask_card_stands_whatever_tool_is_live() {
        let mut app = app();
        assert!(app.mask_card.is_none(), "an unmasked document showed a mask card");

        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        let card = app.mask_card.clone().expect("a painted mask showed no card");
        let name = app.doc.node(app.doc.active()).expect("an active body").name.clone();
        assert!(
            card.headline.contains(&name),
            "the card does not name the body: {}",
            card.headline
        );
        assert!(card.headline.contains('%'), "the card carries no percentage: {}", card.headline);

        // Back to sculpting, and the card stays: the whole point is that it is
        // not scoped to the tool that made the mask.
        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt);
        assert!(app.mask_card.is_some(), "leaving mask mode took the card with it");

        // And it goes away when the mask does.
        update(&mut app, Message::Undo);
        assert!(app.mask_card.is_none(), "undoing the mask left its card behind");
    }

    /// **Switching the tint off leaves the card up, unchanged, and marks no
    /// brick dirty.** This is the whole of what makes decision 13 safe.
    ///
    /// A `show mask` toggle ships where this plan argued for a held-only peek
    /// key, on the reasoning that a tint which can be switched off will be left
    /// off. What closes the hole is that the toggle governs the TINT ONLY: the
    /// card is built from the document and consults neither the toggle nor the
    /// strength, so "a mask is active and nothing on screen says so" stays
    /// unreachable. If this test ever has to be relaxed, the toggle is the
    /// thing to delete rather than the assertion.
    ///
    /// The pixel half of the claim -- that a body with the tint off is
    /// pixel-identical to an unmasked one -- is in `hani-gpu`'s
    /// `offscreen.rs`, which is the only place a pixel exists.
    #[test]
    fn switching_the_tint_off_leaves_the_card_up_and_every_brick_alone() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);

        let card = app.mask_card.clone().expect("a painted mask showed no card");
        let fill = app.doc.active_volume().mask_fill();
        let revision = app.doc.active_volume().mask().revision();
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "the fixture left a remesh pending");
        assert_eq!(app.shared.mask_view().tint, DEFAULT_MASK_TINT, "the tint did not start on");

        update(&mut app, Message::ShowMaskToggled);
        assert_eq!(app.shared.mask_view().tint, 0.0, "the toggle did not reach the renderer");
        assert_eq!(app.mask_card, Some(card.clone()), "switching the tint off changed the card");
        assert_eq!(app.doc.active_volume().mask_fill(), fill, "the toggle changed protection");
        assert_eq!(app.doc.active_volume().mask().revision(), revision);
        assert_eq!(
            app.doc.active_volume().dirty_count(),
            0,
            "the toggle marked bricks for remesh, which it has no reason to: the tint is a uniform"
        );

        update(&mut app, Message::ShowMaskToggled);
        assert_eq!(app.shared.mask_view().tint, DEFAULT_MASK_TINT, "the tint did not come back");
        assert_eq!(app.mask_card, Some(card), "switching the tint back on changed the card");
    }

    /// The held peek key shows the mask whatever the toggle says, and puts it
    /// back exactly as it was.
    ///
    /// It is kept alongside the toggle rather than replaced by it because it is
    /// the way back for a user who switched the tint off and forgot -- so the
    /// case worth pinning is the one where the toggle is OFF.
    #[test]
    fn the_held_peek_key_shows_the_mask_whatever_the_toggle_says() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        update(&mut app, Message::ShowMaskToggled);
        update(&mut app, Message::MaskTintChanged(0.6));
        assert_eq!(app.shared.mask_view().tint, 0.0, "the fixture did not switch the tint off");

        update(&mut app, Message::MaskPeekStarted);
        assert_eq!(app.shared.mask_view().tint, 0.6, "the peek key showed nothing");
        // It repeats while it is held, which must be idempotent rather than a
        // strobe -- which is why the peek is two messages and not a toggle.
        update(&mut app, Message::MaskPeekStarted);
        assert_eq!(app.shared.mask_view().tint, 0.6);

        update(&mut app, Message::MaskPeekEnded);
        assert_eq!(app.shared.mask_view().tint, 0.0, "the peek key latched");
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "a peek marked bricks for remesh");
    }

    /// The tint strength is a view strength in 0..1, and nothing it does
    /// reaches protection.
    #[test]
    fn the_tint_strength_is_clamped_and_changes_no_protection() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        let fill = app.doc.active_volume().mask_fill();

        update(&mut app, Message::MaskTintChanged(4.0));
        assert_eq!(app.shared.mask_view().tint, 1.0);
        update(&mut app, Message::MaskTintChanged(-2.0));
        assert_eq!(app.shared.mask_view().tint, 0.0);
        update(&mut app, Message::MaskTintChanged(0.25));
        assert_eq!(app.shared.mask_view().tint, 0.25);

        assert_eq!(
            app.doc.active_volume().mask_fill(),
            fill,
            "the tint slider changed how much of the body is protected, which is the confusion \
             3D-Coat has to warn about in its own documentation"
        );
    }

    /// The polarity handed to the shader is read out of the ACTIVE body every
    /// time, not remembered beside it.
    ///
    /// Remembering it is how the two come to disagree, and what a disagreement
    /// looks like is protected material drawn as free -- the mask at its most
    /// misleading. Nothing in the application can set the polarity yet, so this
    /// sets it the way the increment that ships Invert will.
    #[test]
    fn the_published_polarity_is_the_active_bodys_own() {
        let mut app = app();
        assert!(!app.shared.mask_view().inverted);

        app.doc.active_volume_mut().mask_mut().set_inverted(true);
        update(&mut app, Message::Frame);
        assert!(app.shared.mask_view().inverted, "the flip never reached the renderer");

        app.doc.active_volume_mut().mask_mut().set_inverted(false);
        update(&mut app, Message::Frame);
        assert!(!app.shared.mask_view().inverted);
    }

    /// The card's percentage is CACHED and only recomputed when the mask moves.
    ///
    /// `view()` runs at display rate off `window::frames()`, so a frame that
    /// changed nothing must not pay for a pass over the mask.
    ///
    /// **Asserted with a sentinel, because comparing the card against itself
    /// cannot fail.** A rebuild writes the same words as the card it replaces,
    /// so `assert_eq!(card, card_before)` passes just as happily with the
    /// revision gate deleted and `mask_fill` running on every one of the sixty
    /// frames a second -- which is the entire cost the cache exists to avoid. A
    /// word only the test could have written is the one thing a recompute
    /// destroys.
    #[test]
    fn a_frame_that_changed_nothing_does_not_rebuild_the_mask_card() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        assert!(app.mask_card.is_some(), "a painted mask showed no card");

        let revision = app.doc.active_volume().mask().revision();
        app.mask_card.as_mut().expect("a painted mask showed no card").headline =
            "SENTINEL".to_string();
        for _ in 0..8 {
            update(&mut app, Message::Frame);
        }
        assert_eq!(
            app.doc.active_volume().mask().revision(),
            revision,
            "an idle frame moved the mask revision, so the cache can never hit"
        );
        assert_eq!(
            app.mask_card.as_ref().map(|card| card.headline.as_str()),
            Some("SENTINEL"),
            "eight idle frames rebuilt the card, so every frame pays for mask_fill"
        );
        assert_eq!(app.mask_card.as_ref().map(|card| card.revision), Some(revision));

        // And the gate is not so wide that a real change slips through it.
        mask_drag(&mut app);
        assert_ne!(
            app.mask_card.as_ref().map(|card| card.headline.as_str()),
            Some("SENTINEL"),
            "painting more mask left the card saying what it said before"
        );
    }

    /// A hidden masked body costs the frame tick nothing either.
    ///
    /// The count of masked bodies off screen is the one line on the card that
    /// goes stale without the mask moving, so it sits outside the revision key
    /// -- and that is exactly how a `format!` gets back onto the frame path.
    /// `publish_visibility` allocates nothing by contract and this runs at the
    /// bottom of it, so the count is compared as a number and only turned into
    /// words when it is a NEW number. Sentinels on both strings, because a
    /// rebuild of either one is the bug.
    #[test]
    fn hiding_a_masked_body_does_not_put_a_format_on_the_frame_path() {
        let mut app = app();
        let first = app.doc.active();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        mask_drag(&mut app);
        // The copy carries the mask, so this is two masked bodies in the same
        // place -- and hiding the original leaves the active one masked, which
        // is the arm where the count sits outside the revision key.
        update(&mut app, Message::BodyDuplicated);
        update(&mut app, Message::BodyVisibilityToggled(first));

        let card = app.mask_card.as_ref().expect("a hidden masked body showed no card");
        assert!(card.headline.ends_with('%'), "no percentage on the card: {}", card.headline);
        assert_eq!(card.off_screen, "+1 masked, hidden", "the hidden body was not counted");

        let card = app.mask_card.as_mut().expect("a hidden masked body showed no card");
        card.headline = "SENTINEL".to_string();
        card.off_screen = "SENTINEL".to_string();
        for _ in 0..8 {
            update(&mut app, Message::Frame);
        }
        let card = app.mask_card.as_ref().expect("eight frames took the card away");
        assert_eq!(card.headline, "SENTINEL", "an idle frame rebuilt the headline");
        assert_eq!(card.off_screen, "SENTINEL", "an idle frame rebuilt the hidden count");

        // A body with no mask of its own, with one hidden and one visible out
        // there: the card is nothing but the counts, both of them are said, and
        // it is cached on both.
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let card = app.mask_card.as_ref().expect("the hidden masked body stopped being named");
        assert_eq!(card.headline, "MASK — 1 masked, elsewhere");
        assert_eq!(card.off_screen, "+1 masked, hidden", "the hidden body stopped being counted");

        app.mask_card.as_mut().expect("a card").headline = "SENTINEL".to_string();
        for _ in 0..8 {
            update(&mut app, Message::Frame);
        }
        assert_eq!(
            app.mask_card.as_ref().map(|card| card.headline.as_str()),
            Some("SENTINEL"),
            "an idle frame rebuilt the whole card for a body that has no mask on it"
        );

        // Showing it again is a real change, and the counts have to move with
        // it -- the hidden line goes and the body joins the visible ones.
        // **The card does NOT go away**, which is increment 24's review
        // finding: it used to, leaving two masked bodies on screen with the
        // active body unmasked and nothing anywhere saying protection existed.
        update(&mut app, Message::BodyVisibilityToggled(first));
        let card = app.mask_card.as_ref().expect("two masked bodies on screen showed no card");
        assert_eq!(card.headline, "MASK — 2 masked, elsewhere");
        assert!(card.off_screen.is_empty(), "nothing is hidden now: {}", card.off_screen);
        assert!(
            !card.names_the_active_body(),
            "the card offered to clear a mask the active body does not have"
        );
    }

    /// A sculpt press onto fully protected material says so.
    ///
    /// Without this the mask's most confusing behaviour -- a stroke that does
    /// nothing because of protection painted minutes ago -- reads as a broken
    /// brush, and there is nothing under the cursor for the user to connect it
    /// to.
    #[test]
    fn a_sculpt_press_onto_a_protected_surface_says_the_mask_blocked_it() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(Tool::Mask));
        // Wide and repeated, so the surface under the cursor really does reach
        // full protection rather than the feathered rim's fraction of it.
        update(&mut app, Message::BrushRadiusChanged(12.0));
        for _ in 0..4 {
            mask_drag(&mut app);
        }
        update(&mut app, Message::ToolChanged(Tool::Mask));
        assert_eq!(app.tool, Tool::Sculpt, "the fixture did not go back to sculpting");

        app.status.clear();
        press(&mut app, centre_of_viewport());
        release(&mut app);
        assert!(
            app.status.contains("mask"),
            "a press onto protected material reported: {}",
            app.status
        );
    }
}

/// The whole-mask VERBS: Clear, Invert, Mask all, and the four absolute
/// filters.
///
/// Its own module rather than more of `mask_tool_tests`, because none of these
/// is a gesture in the viewport: they are buttons and a slider, and what they
/// have to answer for is memory and undo rather than what a press means.
#[cfg(test)]
mod mask_verb_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// An application with a real painted mask on its one body.
    fn masked() -> Hani {
        let mut app = app();
        let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        update(&mut app, Message::ToolChanged(Tool::Mask));
        update(&mut app, Message::BrushRadiusChanged(10.0));
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre,
                size: SIZE,
            }),
        );
        for step in 1..=3 {
            update(
                &mut app,
                Message::Pointer(PointerEvent::Moved {
                    position: Vector::new(centre.x + step as f32 * 15.0, centre.y),
                    size: SIZE,
                }),
            );
        }
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert!(app.doc.active_volume().mask_fill() > 0.0, "the fixture painted no mask");
        app
    }

    /// Every stored mask byte of the active body, for a bit-for-bit comparison.
    fn mask_of(app: &Hani) -> Vec<(hani_core::BrickCoord, Vec<u8>)> {
        let mask = app.doc.active_volume().mask();
        let mut coords: Vec<_> = mask.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        let dim = hani_core::BRICK_DIM as i32;
        coords
            .into_iter()
            .map(|coord| {
                let origin = coord.origin();
                let bytes = (0..dim)
                    .flat_map(|z| {
                        (0..dim).flat_map(move |y| (0..dim).map(move |x| glam::IVec3::new(x, y, z)))
                    })
                    .map(|local| mask.at(origin + local))
                    .collect();
                (coord, bytes)
            })
            .collect()
    }

    /// Clear takes the mask off, gives its bytes back, and one ctrl+Z puts it
    /// back exactly as it was.
    #[test]
    fn clearing_the_mask_frees_its_bytes_and_one_undo_restores_it_bit_for_bit() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;
        let masked_bytes = app.doc_stats.resident_bytes;

        update(&mut app, Message::MaskCleared);
        assert!(app.doc.active_volume().mask().is_free(), "Clear left protection behind");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "Clear was not one entry");
        assert!(app.unsaved, "Clear did not mark the document unsaved");
        assert!(
            app.doc_stats.resident_bytes < masked_bytes,
            "Clear kept the mask's bytes: {} against {masked_bytes}",
            app.doc_stats.resident_bytes
        );
        assert!(app.mask_card.is_none(), "the card outlived the mask it was about");

        update(&mut app, Message::Undo);
        assert_eq!(mask_of(&app), before, "undoing a Clear did not put the mask back");
        assert_eq!(app.doc_stats.resident_bytes, masked_bytes);
        assert!(app.mask_card.is_some(), "the card did not come back with the mask");
    }

    /// **Clear charges the reclaim allowance and not the stroke budget.**
    ///
    /// The map is moved into the entry rather than copied into it, so there is
    /// nothing for the 256 MB stroke budget to hold -- and charging it there
    /// would evict every stroke behind it the first time somebody cleared a
    /// mask off a large body.
    #[test]
    fn clearing_the_mask_costs_the_reclaim_allowance_and_not_the_stroke_budget() {
        let mut app = masked();
        let before = app.history.stats();
        update(&mut app, Message::MaskCleared);
        let after = app.history.stats();
        assert_eq!(after.bytes, before.bytes, "a moved mask was charged to the stroke budget");
        assert!(
            after.reclaim_bytes > before.reclaim_bytes,
            "the reclaim allowance is not counting the mask the entry is holding"
        );
    }

    /// **Invert twice is the mask that went in, and costs nothing either way.**
    ///
    /// Without the polarity bool this is a rewrite of every mask brick, which
    /// on a lightly masked 45,567-brick model is 1.04 GiB from one keystroke.
    #[test]
    fn inverting_twice_is_bit_identical_and_marks_no_bricks() {
        let mut app = masked();
        let before = mask_of(&app);
        let history_before = app.history.stats();
        // Drained rather than read off `perf`, which still holds whatever the
        // fixture's last remesh counted -- an Invert does not call one, so the
        // counter would pass this test with the claim reversed.
        let mut dirty = Vec::new();
        app.doc.take_dirty(&mut dirty);

        update(&mut app, Message::MaskInverted);
        assert!(app.doc.active_volume().mask().inverted());
        assert!(app.shared.mask_view().inverted, "the flip never reached the renderer");
        app.doc.take_dirty(&mut dirty);
        assert!(dirty.is_empty(), "an Invert marked {} bricks for a remesh", dirty.len());

        update(&mut app, Message::MaskInverted);
        assert!(!app.doc.active_volume().mask().inverted());
        assert_eq!(mask_of(&app), before, "two Inverts did not come back to the same mask");

        let after = app.history.stats();
        assert_eq!(
            after.undo_entries,
            history_before.undo_entries + 2,
            "two Inverts were not two entries"
        );
        assert_eq!(after.bytes, history_before.bytes, "an Invert cost the stroke budget bytes");
        assert_eq!(after.reclaim_bytes, history_before.reclaim_bytes);

        // And back out again through undo, which has to be just as cheap: a
        // remesh here is 475 ms at the brick count the pool is sized for.
        app.doc.take_dirty(&mut dirty);
        update(&mut app, Message::Undo);
        assert!(app.doc.active_volume().mask().inverted(), "undo did not flip it back");
        app.doc.take_dirty(&mut dirty);
        assert!(dirty.is_empty(), "undoing an Invert marked {} bricks", dirty.len());
    }

    /// Mask All over an empty map is one bool: no map, no bricks, no bytes.
    #[test]
    fn masking_all_over_an_empty_map_allocates_nothing() {
        let mut app = app();
        // `doc.totals()` and not `doc_stats`, which is written by `remesh_dirty`
        // and is still zero on an application nothing has remeshed yet.
        let before = app.doc.totals().resident_bytes;
        let history_before = app.history.stats();

        update(&mut app, Message::MaskAllApplied);
        assert!(app.doc.active_volume().mask().protects_everything());
        assert_eq!(app.doc.totals().resident_bytes, before, "Mask All allocated a map");
        assert_eq!(app.history.stats().bytes, history_before.bytes);
        // The entry's own struct is charged to the reclaim allowance and
        // nothing else: no map, no brick, and nothing that grows with the size
        // of the body it protects.
        let held = app.history.stats().reclaim_bytes - history_before.reclaim_bytes;
        assert!(held < 256, "Mask All over an empty map is holding {held} bytes");
        assert!(app.mask_card.is_some(), "a fully masked body showed no card");

        // Twice is refused rather than pushed as a second no-op entry.
        let entries = app.history.stats().undo_entries;
        update(&mut app, Message::MaskAllApplied);
        assert_eq!(app.history.stats().undo_entries, entries, "Mask All twice pushed two entries");
        assert!(app.status.contains("already"), "the refusal said: {}", app.status);
    }

    /// Mask All over a painted mask throws the map away and inverts, as ONE
    /// change, so undo puts both halves back together.
    #[test]
    fn masking_all_over_a_painted_mask_is_one_entry_that_undoes_whole() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAllApplied);
        assert!(app.doc.active_volume().mask().protects_everything());
        assert_eq!(app.history.stats().undo_entries, entries + 1, "Mask All was not one entry");

        update(&mut app, Message::Undo);
        assert!(!app.doc.active_volume().mask().inverted(), "undo left the polarity inverted");
        assert_eq!(mask_of(&app), before, "undo did not put the map back");
    }

    /// **A gesture whose slider went away is committed on the next frame.**
    ///
    /// The amount slider lives in the right-click menu, and Escape closes that
    /// menu -- so the release that commits a drag can simply never arrive. What
    /// would be left is a filtered mask with no history entry and no `unsaved`:
    /// work ctrl+Z cannot reach and that quitting would not prompt about.
    #[test]
    fn a_filter_gesture_orphaned_by_a_closing_menu_is_committed_on_the_next_frame() {
        let mut app = masked();
        app.menu = Some(Vec2::ZERO);
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(1.0));
        let filtered = mask_of(&app);

        // A frame DURING the drag must not commit it early, or every filter
        // becomes one entry per sixtieth of a second.
        update(&mut app, Message::Frame);
        assert!(app.mask_grab.is_some(), "a frame mid-drag ended the drag");
        assert_eq!(app.history.stats().undo_entries, entries, "a frame mid-drag pushed an entry");

        // The menu goes -- Escape, a press in the viewport, anything.
        app.menu = None;
        update(&mut app, Message::Frame);
        assert!(app.mask_grab.is_none(), "the drag is still open with nothing to release it");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "the orphaned drag was lost");
        assert_eq!(mask_of(&app), filtered, "committing the orphan changed what it committed");
        assert!(app.unsaved, "an orphaned drag left the document looking saved");
        assert_eq!(app.mask_amount, 0.0);

        update(&mut app, Message::Undo);
        assert_ne!(mask_of(&app), filtered, "the committed orphan is not undoable");
    }

    /// **Blur at 0.0 is a no-op: no change, and no entry.**
    ///
    /// Literally rather than approximately: dragging back to zero moves the
    /// snapshot itself back onto the body, so it is the same field object.
    #[test]
    fn a_filter_at_zero_changes_nothing_and_pushes_no_entry() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(0.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(mask_of(&app), before, "a filter at zero moved a value");
        assert_eq!(app.history.stats().undo_entries, entries, "a filter at zero pushed an entry");

        // And out and back within one drag is the same claim about the middle
        // of a gesture rather than the start of one.
        update(&mut app, Message::MaskAmountChanged(0.8));
        update(&mut app, Message::MaskAmountChanged(0.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(mask_of(&app), before, "dragging out and back did not land where it started");
        assert_eq!(
            app.history.stats().undo_entries,
            entries,
            "dragging out and back pushed an entry"
        );
    }

    /// **The standing card follows a drag rather than freezing on its first
    /// step**, which the mask's own revision counter cannot deliver on its own.
    ///
    /// Every step of an absolute filter is built from ONE snapshot, so each
    /// carries `snapshot.revision + 1` -- the same number for a grow at 0.3 and
    /// a grow at 0.9. The card is keyed on that number. `Volume::replace_mask`
    /// is what moves it past the mask leaving the body; delete that one line
    /// and this test is the only thing that notices.
    #[test]
    fn the_standing_card_follows_a_filter_drag_step_by_step() {
        let mut app = masked();
        update(&mut app, Message::MaskFilterChosen(MaskFilter::Grow));

        update(&mut app, Message::MaskAmountChanged(0.3));
        let light = app.mask_card.clone().expect("a masked body showed no card");
        let light_fill = app.doc.active_volume().mask_fill();
        assert_eq!(
            light.revision,
            app.doc.active_volume().mask().revision(),
            "the card is keyed on a revision the mask is not at"
        );

        update(&mut app, Message::MaskAmountChanged(0.9));
        let heavy = app.mask_card.clone().expect("a masked body showed no card");
        assert_ne!(
            app.doc.active_volume().mask_fill(),
            light_fill,
            "the fixture's two steps produce the same mask, so it proves nothing"
        );
        assert_ne!(
            light.revision, heavy.revision,
            "two steps of one drag share a revision, so the card can never update"
        );
        assert_eq!(
            heavy.revision,
            app.doc.active_volume().mask().revision(),
            "the card froze on the first step of the drag"
        );
    }

    /// **The whole drag is ONE entry, and the same amount twice from the same
    /// grab lands in the same place.**
    ///
    /// This is the absolute gesture. ZBrush's accumulative alternative has been
    /// its top masking complaint since 2007.
    #[test]
    fn blurring_at_one_twice_in_a_drag_equals_blurring_once_and_is_one_entry() {
        let mut app = masked();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(1.0));
        let once = mask_of(&app);
        update(&mut app, Message::MaskAmountChanged(1.0));
        assert_eq!(mask_of(&app), once, "the second application of the same amount moved it");
        update(&mut app, Message::MaskAmountReleased);

        assert_eq!(
            app.history.stats().undo_entries,
            entries + 1,
            "one drag became more than one entry"
        );
        assert_eq!(mask_of(&app), once, "the release changed the mask");
        assert_eq!(app.mask_amount, 0.0, "the slider did not go back to zero");
        assert!(app.unsaved);
        assert!(app.status.contains("blurred"), "the status said: {}", app.status);
    }

    /// A blur really blurs -- a softened mask has values the painted one does
    /// not -- and one undo takes the whole drag back off.
    #[test]
    fn a_blur_softens_the_mask_and_one_undo_takes_the_whole_drag_off() {
        let mut app = masked();
        let before = mask_of(&app);

        update(&mut app, Message::MaskAmountChanged(0.4));
        update(&mut app, Message::MaskAmountChanged(1.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_ne!(mask_of(&app), before, "the blur changed nothing at all");

        update(&mut app, Message::Undo);
        assert_eq!(mask_of(&app), before, "one undo did not take the whole drag off");
    }

    /// Choosing a different filter mid-drag commits the drag it interrupts
    /// rather than re-applying one filter's snapshot under another's name.
    #[test]
    fn choosing_another_filter_mid_drag_commits_the_drag_it_interrupts() {
        let mut app = masked();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskAmountChanged(1.0));
        update(&mut app, Message::MaskFilterChosen(MaskFilter::Grow));
        assert_eq!(app.history.stats().undo_entries, entries + 1, "the interrupted drag was lost");
        assert!(app.mask_grab.is_none(), "the grab outlived its filter");
        assert_eq!(app.mask_amount, 0.0, "the slider kept the old filter's amount");

        update(&mut app, Message::MaskAmountChanged(1.0));
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(app.history.stats().undo_entries, entries + 2);
        assert!(app.status.contains("grew"), "the status said: {}", app.status);
    }

    /// All four filters run, and each of them changes the mask.
    #[test]
    fn every_one_of_the_four_filters_changes_the_mask() {
        for filter in MaskFilter::ALL {
            let mut app = masked();
            let before = mask_of(&app);
            update(&mut app, Message::MaskFilterChosen(filter));
            update(&mut app, Message::MaskAmountChanged(1.0));
            update(&mut app, Message::MaskAmountReleased);
            assert_ne!(mask_of(&app), before, "{filter:?} did nothing");
        }
    }

    /// **The refusal fires before the walk, and the filter never runs.**
    ///
    /// The peak of a filter is `field + 2 x mask`, which on a fully painted
    /// mask is 1.5 times the field -- 6.22 GiB against a 6 GiB ceiling on the
    /// measured 0.0565 mm dragon, from one button press. `History::trim` keeps
    /// a single over-budget entry by design, so the undo budget cannot save it.
    ///
    /// The document is driven to the ceiling by writing `doc_stats`, which is
    /// the number the guard reads and the only way to reach a six-gigabyte
    /// document in a unit test.
    #[test]
    fn a_filter_that_would_not_fit_is_refused_and_never_runs() {
        let mut app = masked();
        let before = mask_of(&app);
        let entries = app.history.stats().undo_entries;
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize;

        update(&mut app, Message::MaskAmountChanged(1.0));
        assert_eq!(mask_of(&app), before, "the filter ran after the guard refused it");
        assert!(app.mask_grab.is_none(), "the refusal still took the mask off the body");
        assert_eq!(app.mask_amount, 0.0, "the slider stayed up after the refusal");
        assert_eq!(app.history.stats().undo_entries, entries, "a refused filter pushed an entry");
        assert!(
            app.status.contains("could not blur the mask") && app.status.contains("GB"),
            "the refusal has to name the verb and the cost: {}",
            app.status
        );

        // And a release after a refusal commits nothing.
        update(&mut app, Message::MaskAmountReleased);
        assert_eq!(app.history.stats().undo_entries, entries);
    }

    /// **Clear is NOT gated by the memory guard**, and that is deliberate: it
    /// is the one verb that gives the memory back, and refusing it at the
    /// ceiling would trap a user with a mask they cannot remove.
    #[test]
    fn clear_still_works_with_the_document_at_the_memory_ceiling() {
        let mut app = masked();
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize;
        update(&mut app, Message::MaskCleared);
        assert!(app.doc.active_volume().mask().is_free(), "Clear was refused at the ceiling");

        let mut app = masked();
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize;
        update(&mut app, Message::MaskInverted);
        assert!(app.doc.active_volume().mask().inverted(), "Invert was refused at the ceiling");
    }

    /// Clearing a body that carries no mask says so rather than pushing an
    /// entry that restores nothing.
    #[test]
    fn clearing_an_unmasked_body_refuses_by_name() {
        let mut app = app();
        let entries = app.history.stats().undo_entries;
        update(&mut app, Message::MaskCleared);
        assert_eq!(app.history.stats().undo_entries, entries, "Clear on nothing pushed an entry");
        assert!(app.status.contains("no mask"), "the refusal said: {}", app.status);
    }

    /// The two reflex buttons are on the card only when the percentage above
    /// them is about the body they would act on.
    #[test]
    fn the_cards_verbs_appear_only_when_it_names_the_active_body() {
        let mut app = masked();
        let card = app.mask_card.as_ref().expect("a painted mask showed no card");
        assert!(card.names_the_active_body(), "the card is about the body that was masked");

        // A second body, active and unmasked, with the masked one hidden: the
        // card is up for the hidden one and must not offer to clear this one.
        let masked_body = app.doc.active();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        update(&mut app, Message::BodyVisibilityToggled(masked_body));
        let card = app.mask_card.as_ref().expect("a hidden masked body showed no card");
        assert!(
            !card.names_the_active_body(),
            "the card offered its verbs on a body with no mask: {}",
            card.headline
        );
    }

    /// **A fully protected body VISIBLE on screen is announced even when it is
    /// not the active one.** Increment 24's review finding, inverted into an
    /// assertion.
    ///
    /// The shape of it, as the review reproduced it: add a Cube, press Mask
    /// all, click the other body in the BODIES panel. The active body carries
    /// no mask and the Cube is not hidden, so it was counted nowhere and the
    /// card was set to `None` -- a body that spares every cut and refuses every
    /// stroke, sitting in plain view, with nothing anywhere saying so. The
    /// first the user heard of it was the `bricks spared` status line after
    /// drawing a clip plane.
    ///
    /// The tint is the other half of the signal and it is fixed too -- see
    /// `MeshPool::draw`'s per-bucket polarity -- but the tint is a preference
    /// the user can switch off, and the card is the channel that cannot be. So
    /// this is the assertion that has to hold.
    #[test]
    fn a_fully_masked_body_on_screen_is_announced_even_when_it_is_not_active() {
        let mut app = app();
        let first = app.doc.active();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        assert_ne!(cube, first, "the primitive did not become the active body");

        update(&mut app, Message::MaskAllApplied);
        assert!(app.doc.active_volume().mask().protects_everything(), "Mask all did nothing");

        // Away to the other body, exactly as clicking its panel row does.
        update(&mut app, Message::BodySelected(first));
        assert_eq!(app.doc.active(), first);
        assert!(app.doc.active_volume().mask().is_free(), "the fixture masked the wrong body");
        assert!(
            app.shown.iter().all(|shown| *shown),
            "the fixture hid something, which is the case the card already covered"
        );

        let card = app.mask_card.as_ref().expect("a fully masked body on screen showed no card");
        assert_eq!(card.headline, "MASK — 1 masked, elsewhere");
        assert!(card.off_screen.is_empty(), "nothing is hidden: {}", card.off_screen);
        assert!(
            !card.names_the_active_body(),
            "the card offered to clear a mask the active body does not have"
        );

        // And it goes when the protection does, rather than standing on stale
        // counts: back to the Cube, Clear, and the document is free again.
        update(&mut app, Message::BodySelected(cube));
        update(&mut app, Message::MaskCleared);
        update(&mut app, Message::BodySelected(first));
        assert!(app.mask_card.is_none(), "the card outlived the mask it was counting");
    }

    /// **Every body that disagrees with the published polarity is named, and
    /// the set follows which body is active.**
    ///
    /// The rendering half of the same finding. `Uniforms::mask_inverted` is one
    /// word for the whole draw and carries the ACTIVE body's polarity, so the
    /// bodies that disagree with it have to be published beside it or they are
    /// drawn through it -- which is how a fully protected body came to be drawn
    /// with no tint. See [`hani_gpu::MaskPolarity`]; the pixels are pinned in
    /// `hani-gpu`'s `offscreen.rs`, which is the only place a pixel exists.
    ///
    /// The assertions are on `app.shared`, which is what the render callback
    /// actually reads, rather than on the application's own kept buffer.
    #[test]
    fn the_bodies_that_disagree_about_polarity_are_published_beside_the_word() {
        let mut app = app();
        let first = app.doc.active();
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();

        // A document where every body agrees costs the draw nothing.
        assert!(!app.shared.mask_view().inverted);
        assert!(
            app.shared.opposite_polarity_snapshot().is_empty(),
            "an unmasked document published a polarity exception"
        );

        update(&mut app, Message::MaskAllApplied);
        assert!(app.shared.mask_view().inverted, "the active body's polarity did not reach it");
        assert_eq!(
            app.shared.opposite_polarity_snapshot(),
            vec![first],
            "the body that is NOT masked was left to be drawn through the masked one's polarity"
        );

        // The word is the active body's, so choosing another body swaps which
        // side of the document is the exception.
        update(&mut app, Message::BodySelected(first));
        assert!(!app.shared.mask_view().inverted);
        assert_eq!(
            app.shared.opposite_polarity_snapshot(),
            vec![cube],
            "the fully masked body was left to be drawn as free"
        );

        // And undoing the Mask All puts the document back to agreeing.
        update(&mut app, Message::BodySelected(cube));
        update(&mut app, Message::Undo);
        assert!(!app.shared.mask_view().inverted, "undo did not put the polarity back");
        assert!(
            app.shared.opposite_polarity_snapshot().is_empty(),
            "the exception outlived the mask that caused it"
        );
    }
}

/// The GENERATED masks: cavity, smoothness, thickness, the half-space that
/// rides the cut's own drag, and the split that ends the arc.
///
/// Its own module rather than more of `mask_verb_tests`, because what these
/// have to answer for is different: a verb is O(1) and a generator is a pass
/// over the whole body, so the questions are whether it refuses before it runs,
/// whether it pushes an entry when it found nothing, and whether the one
/// gesture it borrows still does what it did.
#[cfg(test)]
mod mask_generator_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// An application with a real painted mask on its one body.
    ///
    /// The same shape `mask_verb_tests::masked` uses, and deliberately a second
    /// copy rather than a shared helper: that module's fixture is about the
    /// verbs and is free to change with them, and a split that quietly started
    /// testing a different mask would be very hard to see.
    fn masked() -> Hani {
        let mut app = app();
        let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        update(&mut app, Message::ToolChanged(Tool::Mask));
        update(&mut app, Message::BrushRadiusChanged(10.0));
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre,
                size: SIZE,
            }),
        );
        for step in 1..=3 {
            update(
                &mut app,
                Message::Pointer(PointerEvent::Moved {
                    position: Vector::new(centre.x + step as f32 * 15.0, centre.y),
                    size: SIZE,
                }),
            );
        }
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
        assert!(app.doc.active_volume().mask_fill() > 0.0, "the fixture painted no mask");
        update(&mut app, Message::ToolChanged(Tool::Sculpt));
        app
    }

    /// Every stored mask byte of the active body, for a bit-for-bit comparison.
    fn mask_of(app: &Hani) -> Vec<(hani_core::BrickCoord, Vec<u8>)> {
        let mask = app.doc.active_volume().mask();
        let mut coords: Vec<_> = mask.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        let dim = hani_core::BRICK_DIM as i32;
        coords
            .into_iter()
            .map(|coord| {
                let origin = coord.origin();
                let bytes = (0..dim)
                    .flat_map(|z| {
                        (0..dim).flat_map(move |y| (0..dim).map(move |x| glam::IVec3::new(x, y, z)))
                    })
                    .map(|local| mask.at(origin + local))
                    .collect();
                (coord, bytes)
            })
            .collect()
    }

    /// A checksum of the FIELD, so a generator that quietly wrote a distance is
    /// caught by the tests that say it must not.
    fn field_of(app: &Hani) -> u64 {
        let volume = app.doc.active_volume();
        let mut coords: Vec<hani_core::BrickCoord> = volume.brick_coords().collect();
        coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
        let dim = hani_core::BRICK_DIM as i32;
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for coord in coords {
            let origin = coord.origin();
            for z in 0..dim {
                for y in 0..dim {
                    for x in 0..dim {
                        let value = volume.sample_voxel(origin + glam::IVec3::new(x, y, z));
                        hash = (hash ^ u64::from(value.to_bits())).wrapping_mul(0x1000_0000_01b3);
                    }
                }
            }
        }
        hash
    }

    /// The default body is a 30 mm sphere, which is smooth everywhere at every
    /// feature size the slider offers -- so this is the generator fixture that
    /// needs no sculpting to produce a mask.
    #[test]
    fn a_smoothness_mask_protects_the_body_and_pushes_exactly_one_entry() {
        let mut app = app();
        let entries = app.history.stats().undo_entries;
        let field = field_of(&app);

        update(&mut app, Message::MaskGenerated(MaskGenerator::Smoothness));

        assert!(!app.doc.active_volume().mask().is_free(), "the smoothness mask protected nothing");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "a generator was not one entry");
        assert!(app.unsaved, "a generated mask did not mark the document unsaved");
        assert!(app.mask_card.is_some(), "a generated mask raised no overlay card");
        assert_eq!(field_of(&app), field, "a generator wrote to the FIELD");
    }

    /// The sign, from the outside. A solid sphere is convex everywhere, so
    /// asking for its cavities has to change nothing at all -- and changing
    /// nothing means no entry and no `unsaved`, not an entry that restores what
    /// is already there.
    #[test]
    fn a_cavity_mask_on_a_convex_sphere_finds_nothing_and_pushes_no_entry() {
        let mut app = app();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskGenerated(MaskGenerator::Cavity));

        assert!(app.doc.active_volume().mask().is_free(), "a convex sphere was masked as a cavity");
        assert_eq!(app.history.stats().undo_entries, entries, "an empty result pushed an entry");
        assert!(!app.unsaved, "an empty result marked the document unsaved");
        assert!(app.status.contains("nothing"), "the status does not say why: {}", app.status);
    }

    /// A generator REPLACES the mask, and the mask it replaced has to come back
    /// whole -- including its polarity, which a generator always resets.
    #[test]
    fn undoing_a_generated_mask_puts_the_hand_painted_one_back_bit_for_bit() {
        let mut app = masked();
        update(&mut app, Message::MaskInverted);
        let before = mask_of(&app);
        assert!(app.doc.active_volume().mask().inverted());

        update(&mut app, Message::MaskGenerated(MaskGenerator::Smoothness));
        assert!(
            !app.doc.active_volume().mask().inverted(),
            "a generated mask inherited the polarity it was asked to replace"
        );

        update(&mut app, Message::Undo);
        assert_eq!(mask_of(&app), before, "undo did not put the painted mask back");
        assert!(app.doc.active_volume().mask().inverted(), "undo did not put the polarity back");
    }

    /// Nothing thin about a 60 mm ball, and the honest answer to that is a
    /// status line rather than a mask over the whole thing.
    #[test]
    fn a_thickness_mask_on_a_solid_ball_selects_nothing() {
        let mut app = app();
        update(&mut app, Message::MaskGenerated(MaskGenerator::Thickness));
        assert!(app.doc.active_volume().mask().is_free(), "a 60 mm ball was called thin");
    }

    /// The two sliders are session state and both are clamped where they are
    /// received, because a message can arrive from anywhere and the thickness
    /// ceiling is a property of the narrow band rather than a preference.
    #[test]
    fn both_generator_sliders_are_clamped_where_the_message_lands() {
        let mut app = app();

        update(&mut app, Message::MaskThicknessChanged(400.0));
        assert_eq!(app.mask_thickness_voxels, hani_core::MAX_THICKNESS_VOXELS);
        update(&mut app, Message::MaskThicknessChanged(-4.0));
        assert_eq!(app.mask_thickness_voxels, 1, "the thinnest wall is one voxel, never zero");

        update(&mut app, Message::MaskFeatureChanged(-1.0));
        assert_eq!(app.mask_feature_mm, *MASK_FEATURE_RANGE_MM.start());
        update(&mut app, Message::MaskFeatureChanged(1000.0));
        assert_eq!(app.mask_feature_mm, *MASK_FEATURE_RANGE_MM.end());
    }

    /// **The refusal is part of the feature and not a nicety.** A generated
    /// mask writes a value at every surface voxel, so the +25% is what is
    /// actually paid -- and it has to be predicted rather than discovered by an
    /// allocator.
    ///
    /// The ceiling cannot be reached by building a document (six gigabytes of
    /// resident bricks is six gigabytes of real allocation), so what is moved
    /// here is the number the guard is asked about, which is the same thing
    /// `hani_core::split`'s own refusal test does with its guard.
    #[test]
    fn a_generated_mask_that_would_not_fit_is_refused_before_it_runs() {
        let mut app = app();
        // Everything but a few megabytes of the ceiling is already spoken for.
        app.doc_stats.resident_bytes = MAX_VOLUME_BYTES as usize - 4 * 1024 * 1024;
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::MaskGenerated(MaskGenerator::Smoothness));

        assert!(app.doc.active_volume().mask().is_free(), "the refusal did not stop the pass");
        assert_eq!(app.history.stats().undo_entries, entries, "a refusal pushed an entry");
        assert!(!app.unsaved, "a refusal marked the document unsaved");
        assert!(
            app.status.contains("GB of memory") && app.status.contains("mm"),
            "the refusal names neither the ceiling nor a voxel size that would fit: {}",
            app.status
        );
    }

    // --------------------------------------------- the half-space, and its control

    /// Ctrl at the end of a cut drag writes protection instead of taking
    /// material away: the same gesture, the same plane, a different field.
    #[test]
    fn a_ctrl_cut_drag_masks_the_half_instead_of_removing_it() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let field = field_of(&app);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.control = true;
        let middle_y = SIZE.y / 2.0;
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: Vector::new(SIZE.x * 0.1, middle_y),
                size: SIZE,
            }),
        );
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, middle_y),
            size: SIZE,
        });
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));

        assert_eq!(field_of(&app), field, "a ctrl cut removed material");
        assert!(!app.doc.active_volume().mask().is_free(), "a ctrl cut protected nothing");
        assert_ne!(app.tool, Tool::Cut, "the cut stayed armed after being used");
        assert!(app.mask_card.is_some(), "the half-space mask raised no overlay card");

        // The half a plain cut would have removed is the half that is protected,
        // and its opposite is free -- which is the sentence the doc comment
        // makes and the only part of it a number can hold up.
        let voxel = app.doc.voxel_size();
        let protection = |mm: f32| {
            let cell = (Vec3::new(0.0, mm, 0.0) / voxel).round().as_ivec3();
            app.doc.active_volume().mask().at(cell)
        };
        assert_ne!(protection(12.0) > 128, protection(-12.0) > 128, "both halves read the same");
    }

    /// The control for the test above, in the same module: without ctrl the cut
    /// still cuts, so the branch cannot be reached by a modifier that is stuck
    /// on.
    #[test]
    fn a_cut_drag_without_ctrl_still_removes_material() {
        let mut app = app();
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        let field = field_of(&app);

        update(&mut app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        let middle_y = SIZE.y / 2.0;
        update(
            &mut app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: Vector::new(SIZE.x * 0.1, middle_y),
                size: SIZE,
            }),
        );
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, middle_y),
            size: SIZE,
        });
        update(&mut app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));

        assert_ne!(field_of(&app), field, "a plain cut removed nothing");
        assert!(app.doc.active_volume().mask().is_free(), "a plain cut painted a mask");
    }

    // --------------------------------------------------- splitting the mask off

    #[test]
    fn splitting_off_the_mask_makes_two_bodies_and_one_undo_puts_the_source_back() {
        let mut app = masked();
        let source = app.doc.active();
        let bodies = app.doc.body_count();
        let entries = app.history.stats().undo_entries;

        update(&mut app, Message::BodySplitMasked);

        assert_eq!(app.doc.body_count(), bodies + 1, "a masked split did not make a second body");
        assert!(app.doc.index_of(source).is_none(), "the source was not consumed");
        assert_eq!(app.history.stats().undo_entries, entries + 1, "a split was not one entry");
        assert!(app.unsaved);
        assert!(
            app.doc.active_volume().mask().is_free(),
            "the half the user was left on arrived carrying a mask"
        );

        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), bodies, "one undo did not take both halves back");
        assert!(app.doc.index_of(source).is_some(), "undo did not restore the source row");
        assert!(
            !app.doc.active_volume().mask().is_free(),
            "undo restored the geometry but not the mask that chose the split"
        );
    }

    /// The refusal names the body, because a document with sixty-four rows in it
    /// makes "carries no mask" a question about which one.
    #[test]
    fn splitting_off_a_mask_that_is_not_there_is_refused_by_name() {
        let mut app = app();
        let bodies = app.doc.body_count();
        update(&mut app, Message::BodySplitMasked);
        assert_eq!(app.doc.body_count(), bodies, "an unmasked body was split anyway");
        assert!(app.status.contains("no mask"), "the refusal does not say why: {}", app.status);
        assert!(!app.unsaved, "a refusal marked the document unsaved");
    }
}
