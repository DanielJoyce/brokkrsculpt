// SPDX-License-Identifier: AGPL-3.0-only

//! The document and its files: a new sculpt, the crash net, the view a file
//! keeps, open, import, save and its check, export and the slicer. Moved
//! out of `app.rs` (issue 189).

use super::*;

impl Hani {
    /// Throw the sculpt away and start again from a fresh sphere.
    ///
    /// Extracted from the old `ResetSphere` arm so the menu and the panel button
    /// share one path rather than drifting.
    pub(super) fn reset_sculpt(&mut self) {
        let mut volume = Volume::new(self.doc.voxel_size());
        volume.seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM);
        volume.mark_everything_dirty();
        self.doc = Document::from_volume(volume);
        // History refers to bricks of the volume that just went away, so keeping
        // it would let undo splice pieces of the discarded model into this one.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.camera = OrbitCamera::framing(Vec3::ZERO, MODEL_RADIUS_MM);
        // `framing` leaves the default lattice behind, and the floor is a
        // number of voxels, so it has to be told the real one.
        self.refresh_camera_lattice();
        self.project_path = None;
        // A new sculpt starts with no pictures and no floor. See
        // `app/references.rs`.
        self.adopt_references(hani_core::reference::References::default());
        self.unsaved = false;
        self.status = String::new();
        self.publish_camera();
        self.rebuild_everything();
        self.refresh_detail_advice();
        self.refresh_overlay();
    }

    /// How long between crash-net writes.
    ///
    /// Two minutes was a compromise against the write itself, when the write
    /// was on the thread that draws: often enough to be worth having and
    /// rare enough that the hitch was not something you sculpted against.
    /// The write is off that thread now (`app/autosave.rs`) and this could
    /// be shorter. It has been left, because a write still has to find the
    /// document holding still for as long as it takes.
    const AUTOSAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(120);

    /// How long the pointer must have been still before the crash net is
    /// begun.
    ///
    /// This was here to hide a hitch: the write was on the thread that
    /// draws, measured at **1.55 GB/s**, so a 159 MB model took 100 ms and the
    /// 671 MB autosave seen from a real scan session about 430 ms -- some
    /// twenty six dropped frames, every two minutes, in the middle of
    /// sculpting. Waiting for a pause put the hitch where it was not seen.
    ///
    /// There is no hitch to hide now: the encoding is a piece a frame and the
    /// file is written by another thread. See `app/autosave.rs`. The pause is
    /// kept for a different reason: a write in pieces is given up the moment
    /// the document is wanted, so one begun while the hand is still moving is
    /// work thrown away.
    const AUTOSAVE_IDLE_GAP: std::time::Duration = std::time::Duration::from_secs(3);

    /// Where the crash net lives.
    ///
    /// `$XDG_STATE_HOME`, not the config directory: this is recoverable state
    /// rather than settings, and it must never sit next to the user's own
    /// files where it could be mistaken for one.
    pub(crate) fn default_autosave_path() -> Option<std::path::PathBuf> {
        crate::paths::state_file("autosave.hani")
    }

    /// Write the crash net if it is due.
    ///
    /// Driven from the frame tick rather than a timer, because iced's `time`
    /// module is empty under this feature set -- the defaults here include
    /// `thread-pool`, not `tokio` or `smol`, so `iced::time::every` does not
    /// exist to be called.
    ///
    /// Known limitation: `window::frames()` stops arriving when the compositor
    /// stops asking for frames, so autosave stalls on a minimised or fully
    /// occluded window. That is the acceptable direction -- the case worth
    /// protecting is a crash during active sculpting, and during active
    /// sculpting there are frames. Revisit if iced gains a timer here.
    pub(super) fn maybe_autosave(&mut self) {
        // One in flight goes on a piece, whatever else is so: the checks
        // below are about beginning one. See `app/autosave.rs`.
        if self.autosave.in_flight() {
            self.advance_autosave();
            return;
        }
        // The crash net is of the document, and on the workbench what is
        // in the document's place is not it.
        if self.on_the_bench() {
            return;
        }
        if !self.unsaved
            // Never mid-stroke: the document has to hold still while it is
            // written.
            || self.stroke.is_active()
            || self.drag.is_some()
            || self.last_autosave.elapsed() < Self::AUTOSAVE_INTERVAL
            // And not while the pointer is still moving. See the constant.
            || self.last_activity.elapsed() < Self::AUTOSAVE_IDLE_GAP
        {
            return;
        }
        if self.begin_autosave() {
            self.advance_autosave();
        } else {
            // So that a write that cannot begin is not tried again on every
            // frame after it. One that begins and is given up is another
            // matter: it is tried again at the next pause.
            self.last_autosave = Instant::now();
        }
    }

    /// The session state a `.hani` file carries alongside the field.
    ///
    /// Shared by the explicit save and the autosave so the two cannot record
    /// different things.
    pub(super) fn project_state(&self) -> hani_core::ProjectState {
        hani_core::ProjectState {
            view: self.current_view(),
            paints: self.doc.paints().clone(),
            // See `app/references.rs`.
            references: self.references.clone(),
            // See `app/symmetry_frame.rs`.
            radial: hani_core::Radial::default(),
        }
    }

    /// What a `.hani` file stores beside the sculpt, so a reopen comes back to
    /// the same working setup.
    pub(super) fn current_view(&self) -> hani_core::View {
        hani_core::View {
            camera_target: self.camera.target,
            camera_distance: self.camera.distance,
            camera_yaw: self.camera.yaw,
            camera_pitch: self.camera.pitch,
            camera_roll: self.camera.roll,
            brush_radius: self.brush.radius,
            brush_strength: self.brush.strength,
            mirror: MirrorAxis::ALL.map(|axis| self.symmetry_settings().axis(axis)),
        }
    }

    /// Put the camera, brush and mirror planes back the way a view records
    /// them, clamping every number to the range the interface offers.
    ///
    /// Clamped rather than trusted for the same reason `open_project` clamps:
    /// a view can come from a file, and the radius ceiling has already moved
    /// once. A saved 12 mm brush is fine; a saved 40 mm one from some future
    /// build must not put the slider somewhere it cannot be dragged back from.
    ///
    /// The mirror planes are put back and then **swept**, for the same reason
    /// choosing a body sweeps them: a file can name a mirror the body it was
    /// saved with does not straddle -- it was saved from a build without the
    /// refusal, or with a different body active -- and a restored mirror
    /// carves twins into empty space exactly as an enabled one does. The sweep
    /// costs one measurement of the field, at a file open or a key press,
    /// which is a user action; see
    /// [`Hani::refuse_mirrors_the_body_does_not_straddle`].
    pub(super) fn apply_view(&mut self, view: &hani_core::View) {
        // **The near and far planes come from the body being restored, never
        // from the default sphere, and getting that wrong poisons the file it
        // is later saved to.**
        //
        // This used to spread `framing(Vec3::ZERO, MODEL_RADIUS_MM)` -- the
        // 30 mm starting ball -- which set `near` to 0.03 and therefore pinned
        // `zoom_by`'s floor at `near * 10.0` = 0.3 mm whatever was actually in
        // the file. On a 133 mm model the floor should be 1.33 mm, so the first
        // scroll inwards drove the camera to 0.3 and left it there: zooming in
        // did nothing, and zooming out moved 12% of 0.3 mm, which is 0.036 mm a
        // notch. Pan and orbit both scale with `distance`, so THE SPACEMOUSE,
        // THE SCROLL WHEEL AND THE NAVIGATION CUBE ALL LOOKED DEAD AT ONCE --
        // which is the shape of the report, and the reason it read as a broken
        // file rather than a broken clamp. Saving then wrote 0.3 into the
        // container, so the damage outlived the session that caused it.
        let radius =
            self.doc.active_volume().content_radius().unwrap_or(MODEL_RADIUS_MM).max(1.0e-3);
        let centre = self
            .doc
            .active_volume()
            .world_bounds()
            .map_or(Vec3::ZERO, |(low, high)| (low + high) * 0.5);
        let framed = OrbitCamera::framing(centre, radius);

        let mut camera = OrbitCamera {
            target: view.camera_target,
            distance: view.camera_distance,
            yaw: view.camera_yaw,
            pitch: view.camera_pitch,
            roll: view.camera_roll,
            ..framed
        };
        // **The admissible range is asked of a camera carrying the document's
        // own lattice and extent, which is the pair the live camera will be
        // given the moment this restore lands.**
        //
        // `framing` sets neither: it takes a radius and leaves `voxel_size` at
        // the 0.25 mm default, so `framed.min_distance()` was 0.5 mm for every
        // document ever opened while the live floor is
        // `doc.voxel_size() * 2.0`. Measured: resample to 0.05, scroll in, the
        // camera legitimately reaches 0.1 -- and reopening threw that framing
        // away and re-framed to 158 mm, with a status line telling the user
        // their file was wrong. Every lattice but 0.25 was affected, and the
        // suite could not see it because `VOXEL_SIZE_MM` happens to equal the
        // camera default.
        //
        // The ceiling had the same shape for a second reason: it came from the
        // ACTIVE body's content radius while the live one comes from the
        // document ball, so 200 notches of scroll-out parked the camera at
        // 6906 -- legal, the live clamp put it there -- and the loader called
        // 5520 the limit and rejected it. This is the same "the loader and the
        // live camera must not disagree about the same word" the target test
        // below is built on; it simply was not applied to the distance.
        camera.set_lattice(self.doc.voxel_size(), self.document_ball().1);

        // **A file written before that fix carries the bad numbers, so the
        // restore has to be able to recover one rather than merely stop making
        // them.** Two ways a saved view can be unusable, and both are checked
        // against the body rather than against a constant:
        //
        // The target may sit outside the model's bounding sphere, in which case
        // the camera is aimed at empty space and no amount of orbiting brings
        // the model back -- orbiting turns about the target, so the model stays
        // exactly as far off screen as it was. The real file that prompted this
        // had a target 435 mm from a model of radius 133 mm.
        //
        // And the distance may be outside what `zoom_by` will now admit, which
        // is what a floor computed from the wrong radius leaves behind.
        //
        // **The target test is [`Hani::document_ball`] and not the active
        // body's own sphere, so that the loader and the live camera cannot
        // disagree about the same word.** [`Hani::contain_target`] holds the
        // camera inside that ball on every mutation; if the restore accepted a
        // smaller one, a target the live camera reaches legitimately -- panned
        // across a two body document, say -- would be refused on the way back
        // in and the user's framing thrown away for no reason. The union spans
        // the gaps between bodies, which for a camera bound is the safe
        // direction to be loose in.
        //
        // The angles are kept in both cases. They are the part of a saved view
        // that is still meaningful when the rest of it is not, and re-framing
        // without them would spin a model the user had deliberately turned.
        let (ball_centre, ball_radius) = self.document_ball();
        let lost_target =
            !camera.target.is_finite() || camera.target.distance(ball_centre) > ball_radius;
        let lost_distance = !camera.distance.is_finite()
            || camera.distance < camera.min_distance()
            || camera.distance > camera.max_distance();
        if lost_target {
            camera.target = framed.target;
        }
        if lost_distance {
            // Clamped for the same reason `frame_active` clamps: a body smaller
            // than a couple of voxels frames at a distance under its own floor.
            camera.distance = framed.distance.clamp(camera.min_distance(), camera.max_distance());
        }
        if lost_target || lost_distance {
            // Said out loud rather than fixed silently: the view really is not
            // the one the file asked for, and a user who had framed a detail
            // deliberately should be told why it moved.
            self.status = format!(
                "{} — the saved view could not see the model, so it was re-framed",
                self.status
            );
        }

        self.camera = camera;
        self.refresh_camera_lattice();
        self.brush.radius = view.brush_radius.clamp(MIN_RADIUS_MM, self.max_radius());
        self.brush.strength = view.brush_strength.clamp(MIN_STRENGTH, MAX_STRENGTH);
        // The planes are each body's own and kept with it; a file from
        // before kept them here, for every body. See `app/symmetry_frame.rs`.
        self.adopt_session_mirrors(view.mirror);
        self.refuse_mirrors_the_body_does_not_straddle("turned off");
    }

    /// Whether there is a crash net to offer.
    pub(crate) fn has_autosave(&self) -> bool {
        self.autosave_file.as_ref().is_some_and(|path| path.is_file())
    }

    /// Move an autosave this build cannot read out of the way, so the next one
    /// cannot destroy it.
    ///
    /// **The updater supports downgrade, and that is what makes this
    /// necessary.** Publishing a manifest naming an older build is how a bad
    /// release is walked back, and `apply.rs` states outright that the build
    /// reading a file can be older than the build that wrote it. Compose that
    /// with the autosave: a newer build writes a crash net in a field encoding
    /// the older one refuses, the user is told an autosave is waiting, File >
    /// Recover fails with a version message, they shrug and start sculpting --
    /// and 120 seconds later the older build overwrites the only copy of their
    /// work with an empty sphere. Nothing in the write path notices that the
    /// file it is about to replace is one it could not read.
    ///
    /// Probed with [`read_outline`], which reads a header and a node table
    /// rather than a document, so the check costs a few hundred bytes on a file
    /// that may be a gigabyte. Any error moves it aside, not only a version
    /// one: a truncated or corrupt crash net is equally worth keeping, since
    /// it is the only copy of something and this application cannot say what a
    /// later build or a hex editor will make of it.
    ///
    /// Named by the first free number rather than by a timestamp, so a test can
    /// assert the name it lands on.
    pub(super) fn preserve_unreadable_autosave(&mut self) -> Option<std::path::PathBuf> {
        let path = self.autosave_file.clone()?;
        let file = std::fs::File::open(&path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        if hani_core::project::read_outline(&mut reader).is_ok() {
            return None;
        }
        drop(reader);

        // A bounded search: a directory already holding this many refused crash
        // nets is a bigger problem than the naming, and spinning here would be
        // worse than giving up.
        for attempt in 1..100 {
            let aside = path.with_file_name(format!("autosave-unreadable-{attempt}.hani"));
            if aside.exists() {
                continue;
            }
            return match std::fs::rename(&path, &aside) {
                Ok(()) => {
                    log::warn!("kept an unreadable autosave as {}", aside.display());
                    Some(aside)
                }
                Err(error) => {
                    log::error!("could not move {} aside: {error}", path.display());
                    None
                }
            };
        }
        None
    }

    /// Load the crash net, and deliberately do not adopt its path.
    ///
    /// `open_project` would leave `project_path` pointing at the autosave file,
    /// and the next plain Save would then write the user's work back into the
    /// crash net -- a file under `$XDG_STATE_HOME` that they never chose and
    /// that the next successful save deletes. So the recovered document is
    /// deliberately homeless and unsaved: Save behaves as Save As, which is
    /// what makes them name it somewhere real.
    pub(super) fn recover_autosave(&mut self) {
        let Some(path) = self.autosave_file.clone() else {
            return;
        };
        self.open_project(&path);
        if self.status.contains("could not") {
            return;
        }
        self.project_path = None;
        self.unsaved = true;
        // `open_project` records whatever it opened. The autosave is a crash
        // net, not a document, and has no business in the recent list.
        self.recent.forget(&path);
        self.status = "recovered the autosave — save it somewhere".to_string();
    }

    pub(super) fn open_project(&mut self, path: &std::path::Path) {
        // A file picked in a browser is held by name. See `files.rs`.
        let mut reader = match crate::files::open(path) {
            Ok(file) => file,
            Err(error) => {
                self.status = format!("could not open {}: {error}", path.display());
                // A file that has been moved or deleted since it was last used
                // should stop being offered.
                self.recent.forget(path);
                return;
            }
        };
        let (doc, state) = match hani_core::project::read(&mut reader) {
            Ok(loaded) => loaded,
            Err(error) => {
                // The message says what was expected as well as what was found,
                // because "a file from a different build" and "a partial
                // download" look identical without it.
                self.status = format!("could not read {}: {error}", path.display());
                return;
            }
        };

        self.doc = doc;
        // The mask card is keyed on `(body, mask revision)`, and a mask read
        // from a file has the revision the READER left on it -- which counts
        // the bricks it inserted. Two projects whose active body shares an id
        // and a mask brick count would then hit each other's cache and the
        // card would stand there naming the wrong percentage. Nothing of the
        // old document survives an open, this included.
        self.mask_card = None;
        // Before `apply_view`, so that a mirror plane the loaded body does not
        // straddle wins the status line: which file was opened is visible in
        // the title and the recent list, and a mirror plane going off is not
        // visible anywhere else. Same ordering, and same reason, as
        // `select_body`.
        self.status = format!("opened {}", path.display());
        self.apply_view(&state.view);
        // A file from before each body had its own radial copies. See
        // `app/symmetry_frame.rs`.
        self.adopt_session_radial(state.radial);
        self.doc.replace_paints(state.paints);
        // The pictures and the floor. See `app/references.rs`.
        self.adopt_references(state.references);
        // Painted by filament slot, from before slot painting was taken
        // out: its slots laid into its colour paint. See
        // `hani_core::paint::old_slots`.
        self.lay_old_slots();

        // History belongs to the model that just went away.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.project_path = Some(path.to_path_buf());
        self.unsaved = false;
        self.recent.record(path);
        self.publish_camera();
        self.rebuild_everything();
        self.refresh_detail_advice();
        self.refresh_overlay();
    }

    /// Swap an imported model in, modelled on `open_project`.
    ///
    /// Three things here are individually silent when wrong.
    ///
    /// `project_path` is cleared, where `open_project` sets it. An import must
    /// not adopt the mesh's path, or the next plain Save would write a `.hani`
    /// container straight over the user's `.stl` with no dialog and no warning.
    /// This is the most damaging mistake available in this function.
    ///
    /// `mark_everything_dirty` is done by `voxelise` itself, exactly as
    /// `project::read` does it, which is why neither this nor `open_project`
    /// appears to need it.
    ///
    /// The camera is framed on `MODEL_RADIUS_MM` like every other call site,
    /// not on the model's own measured radius: the bound rounds out to whole
    /// bricks, so for a centred model it over-reports by up to about 1.7 times
    /// plus padding and the camera sits visibly too far back.
    pub(super) fn adopt_import(&mut self, imported: crate::message::Imported) {
        self.doc = Document::from_volume(imported.volume);
        self.camera = OrbitCamera::framing(Vec3::ZERO, MODEL_RADIUS_MM);
        // `framing` leaves the default lattice behind, and the floor is a
        // number of voxels, so it has to be told the real one.
        self.refresh_camera_lattice();
        // **The brush is in millimetres and the model just changed size.** The
        // 3 mm default is a twentieth of the 60 mm starting ball, which is a
        // brush; against a 200 mm import it is a pin, and against a model that
        // arrived small it covered the whole thing. Neither is a radius anyone
        // chose, and both read as the controls being broken rather than as a
        // slider needing a drag. Held at the same twentieth so the brush means
        // the same thing whatever was opened.
        let longest = imported.report.longest_mm;
        if longest.is_finite() && longest > 0.0 {
            self.brush.radius = (longest / 20.0).clamp(MIN_RADIUS_MM, self.max_radius());
        }
        self.history.clear();
        self.history_stats = self.history.stats();
        self.project_path = None;
        // An import is unsaved work by definition: there is no `.hani` file
        // holding it, and quitting now would lose the whole thing.
        self.unsaved = true;
        self.status = format!(
            "imported {}: {}, {:.0} ms",
            imported
                .source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| imported.source.display().to_string()),
            imported.report.summary(),
            imported.elapsed_ms,
        );
        log::info!("{}", self.status);
        // Nothing in an STL, OBJ or 3MF says which axis is up, and the two
        // conventions in the world disagree -- so a mesh that arrived lying
        // down is a normal outcome rather than a corrupt file. Ask; do not
        // turn it silently, because the guess can be wrong and a model turned
        // without being asked about is one the user cannot account for.
        //
        // Filtered against `Up`, so the common case of a print-ready file that
        // was already standing raises nothing.
        self.orient_prompt = imported.resting_up.filter(|up| *up != hani_core::Facing::Up);
        self.publish_camera();
        self.rebuild_everything();
        self.refresh_detail_advice();
        self.refresh_overlay();
    }

    /// Save the document, through a temporary file and a rename.
    ///
    /// **Two things here used to be wrong, and both got worse with every body
    /// added.**
    ///
    /// It opened the user's real file with `File::create`, which truncates it
    /// to zero before a byte of the new document is written. The crash net
    /// already had a temp-and-rename and the document did not, so the *net* was
    /// better protected than the thing it protects -- and at the measured
    /// 1.55 GB/s a three gigabyte document is about two seconds of the user's
    /// file existing as a truncated stub. A failed save now leaves the previous
    /// file exactly as it was.
    ///
    /// And it cleared `unsaved` and deleted the crash net **on the writer's
    /// word**. So the file is read back before either happens: its header and
    /// node table only, a few hundred bytes and no geometry, compared against
    /// the document that was meant to go into it. On a mismatch the temporary
    /// is thrown away, the previous file is untouched, `unsaved` stays set, the
    /// autosave survives, and the status says "could not" -- which is what
    /// `panel.rs` colours red.
    pub(super) fn save_project(&mut self, path: &std::path::Path) {
        let state = self.project_state();

        // Beside the target rather than in a temp directory, so the rename is
        // within one filesystem and therefore atomic.
        let temporary = path.with_extension("hani.tmp");
        let file = match std::fs::File::create(&temporary) {
            Ok(file) => file,
            Err(error) => {
                self.status = format!("could not write {}: {error}", path.display());
                return;
            }
        };
        let mut writer = std::io::BufWriter::new(file);
        if let Err(error) = hani_core::project::write(&mut writer, &self.doc, &state) {
            self.status = format!("could not write {}: {error}", path.display());
            std::fs::remove_file(&temporary).ok();
            return;
        }
        // `project::write` ends with its own flush and reports a failure there
        // as an error, so nothing is left in the buffer by the time this
        // returns. The drop is to CLOSE the file before it is reopened for the
        // check below.
        drop(writer);

        if let Err(problem) = self.verify_written(&temporary) {
            self.status = format!("could not write {}: {problem}", path.display());
            std::fs::remove_file(&temporary).ok();
            return;
        }

        if let Err(error) = std::fs::rename(&temporary, path) {
            self.status = format!("could not replace {}: {error}", path.display());
            std::fs::remove_file(&temporary).ok();
            return;
        }

        self.project_path = Some(path.to_path_buf());
        // Only here. Every failure arm above leaves the flag set, or a failed
        // write would let the next quit go through silently.
        self.unsaved = false;
        self.recent.record(path);
        // The work is safely in a file the user chose, so the crash net has
        // nothing left to protect and a stale one would only offer to restore
        // something older than what is on disk.
        self.clear_autosave();
        self.status = format!("saved {}", path.display());
    }

    /// Read back what was just written, far enough to know it is this document.
    ///
    /// The header and the node table, which is a few hundred bytes whatever the
    /// sculpt weighs -- the geometry is deliberately not re-read, because doing
    /// that on the draw thread would double the cost of every save.
    ///
    /// What it catches is the class the old code could not: a write that
    /// reported success and produced a file this build will not open, or opens
    /// as a different document. That is not hypothetical -- the format grew a
    /// node table this release, and the first thing to exercise it in the wild
    /// is the autosave, unwatched, every two minutes.
    ///
    /// The check itself is pinned by
    /// `the_save_verification_notices_a_file_that_is_not_this_document`, and
    /// the *gate* in `save_project` that calls it -- a separate thing, and the
    /// one that can be refactored away without a test noticing -- by
    /// `a_save_that_cannot_read_its_own_output_back_keeps_the_previous_file`.
    /// If you change the shape of the failure here, that second test is the one
    /// to look at: it drives this through a temporary that reads back empty.
    pub(super) fn verify_written(&self, path: &std::path::Path) -> Result<(), String> {
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        self.check_written(&mut std::io::BufReader::new(file))
    }

    /// The check of [`Self::verify_written`], on bytes from anywhere: in a
    /// browser they are checked in memory. See `app/shelf.rs`.
    pub(super) fn check_written(&self, reader: &mut impl std::io::Read) -> Result<(), String> {
        let outline =
            hani_core::project::read_outline(reader).map_err(|error| error.to_string())?;
        if outline.nodes != self.doc.node_count() || outline.bodies != self.doc.body_count() {
            return Err(format!(
                "the file came back with {} rows and {} bodies, and the document has {} and {}",
                outline.nodes,
                outline.bodies,
                self.doc.node_count(),
                self.doc.body_count()
            ));
        }
        if outline.voxel_size != self.doc.voxel_size() {
            return Err(format!(
                "the file came back at a {} mm voxel, and the document is at {} mm",
                outline.voxel_size,
                self.doc.voxel_size()
            ));
        }
        Ok(())
    }

    /// Weld every visible body and write them all out to a chosen path.
    ///
    /// Refuses to write anything that would not print. A slicer given a mesh
    /// with holes either rejects it or fills it wrong, and finding that out
    /// after a failed print is worse than being told here.
    ///
    /// **The refusal happens before `File::create`, and that ordering is the
    /// point.** `File::create` truncates, so a verdict taken after it would
    /// leave the user with a zero-byte file where their last good export was.
    /// Everything is welded and validated into memory, the verdict is taken,
    /// and only then is a file opened.
    ///
    /// **The status line names how many bodies were omitted, unconditionally
    /// -- including when the answer is none.** The eye is one bit in a
    /// forty-byte node record with no integrity check of any kind, and it is
    /// the bit that decides whether a part reaches the printer: a single
    /// flipped bit is a perfectly legal value that loads without error. The
    /// brick stream is defended by a checked distance decode and the node table
    /// is defended by nothing, so this line is the whole of that defence and it
    /// costs one format string. Making it conditional -- "only say it when
    /// something was hidden" -- would mean silence is ambiguous between
    /// "nothing was hidden" and "the count was never computed", which is
    /// exactly the reading it exists to prevent.
    ///
    /// `saved_visibility` and never `display_visibility`: a view mode must not
    /// decide what reaches a print.
    pub(super) fn export(&mut self, format: ExportFormat, path: &std::path::Path) {
        let started = Instant::now();
        let mut visible = Vec::new();
        self.doc.saved_visibility(&mut visible);
        // Decimated or not, what comes back is welded, validated and judged
        // by the same verdict below. A decimated body that would not print is
        // handed back at full density by the decimator itself, and says so.
        let (mut bodies, decimated) = match self.export_detail.tolerance_mm() {
            None => (self.doc.export_bodies(&visible), None),
            Some(tolerance_mm) => {
                let spec = hani_core::export::decimate::DecimateSpec::within(tolerance_mm);
                let (bodies, reports) = self.doc.export_bodies_decimated(&visible, spec);
                let total = hani_core::export::decimate::DecimateReport::summed(&reports);
                (bodies, Some(total))
            }
        };
        let omitted = self.doc.body_count().saturating_sub(bodies.len());

        // Per body, never over the union: a document whose second body is
        // empty sums to a printable report while half the print is missing.
        if let Err(why) = hani_core::export::document_verdict(&bodies) {
            self.status = format!("not exported, {why}");
            log::error!("refusing to export: {why}");
            return;
        }

        // Paints are worked out on screen and stored nowhere, and a file has
        // to hold something: a colour a vertex for a PLY, a filament a
        // triangle for a 3MF. See `app/paints.rs`.
        let colours = self.bake_for(format, &mut bodies);

        let summary =
            hani_core::MeshReport::summed(bodies.iter().map(|(_, _, report)| *report)).summary();
        let parts: Vec<(&str, &hani_core::ExportMesh)> =
            bodies.iter().map(|(meta, mesh, _)| (meta.name.as_str(), mesh)).collect();

        let filaments = self.palette.as_filaments();
        // Read once, here, rather than inside the writer: both of these walk the
        // user's OrcaSlicer presets off disk, which is not something an export
        // writer should be doing per body.
        let placement =
            hani_core::export::threemf::Placement { plate_centre: crate::slicer::plate_centre() };
        let settings = crate::slicer::project_settings_body(&self.palette);
        let write = || -> std::io::Result<()> {
            // In a browser, a download. See `files.rs`.
            let file = crate::files::create(path)?;
            let mut file = std::io::BufWriter::new(file);
            match format {
                ExportFormat::Stl => hani_core::export::stl::write_all(&parts, &mut file)?,
                ExportFormat::Obj => hani_core::export::obj::write_all(&parts, &mut file)?,
                // The 3MF carries the user's own filament slots, sits on their
                // bed, and names the presets their slicer already has -- none of
                // which the geometry knows anything about.
                ExportFormat::ThreeMf => hani_core::export::threemf::write_project(
                    &parts,
                    &hani_core::export::threemf::Project {
                        filaments: filaments.clone(),
                        placement,
                        settings: settings.as_deref(),
                    },
                    &mut file,
                )?,
                ExportFormat::Textured => self.write_textured(&bodies, path, &mut file)?,
                ExportFormat::Glb => self.write_glb(&bodies, &mut file)?,
                ExportFormat::Ply => {
                    let coloured: Vec<hani_core::export::ply::Coloured<'_>> = bodies
                        .iter()
                        .zip(&colours)
                        .map(|((_, mesh, _), colours)| hani_core::export::ply::Coloured {
                            mesh,
                            colours,
                        })
                        .collect();
                    hani_core::export::ply::write_all(&coloured, &mut file)?;
                }
            }
            std::io::Write::flush(&mut file)
        };

        match write() {
            Ok(()) => {
                let bytes = crate::files::size(path);
                // STL has nowhere to put an object boundary, so several bodies
                // arrive in a slicer as one part. Said here rather than
                // discovered after slicing.
                let fused = matches!(format, ExportFormat::Stl) && parts.len() > 1;
                // No silent fallback. Without the user's presets the file still
                // opens, but OrcaSlicer invents a machine named after it and
                // displaces whatever printer they had selected -- which looks
                // like the application broke their slicer rather than like a
                // setting that could not be read.
                let unbound = matches!(format, ExportFormat::ThreeMf) && settings.is_none();
                // What the decimation did, in the decimator's own words, or
                // nothing at all for a full export: the line an existing
                // export writes is unchanged.
                let decimation =
                    decimated.map(|report| format!("; {}", report.summary())).unwrap_or_default();
                // And how much of it overhangs, when the overhang view is
                // on and so somebody is asking. See `app/overhang.rs`.
                let decimation = format!("{decimation}{}", self.overhang_summary(&bodies));
                self.status = format!(
                    "exported {} of {} bodies; {omitted} hidden{} -- {summary}{decimation} to {} \
                     ({:.1} MB, {:.0} ms){}",
                    parts.len(),
                    self.doc.body_count(),
                    if fused { " (STL fuses them into one part)" } else { "" },
                    path.display(),
                    bytes as f64 / (1024.0 * 1024.0),
                    started.elapsed().as_secs_f64() * 1000.0,
                    if unbound {
                        " — no printer named: pick your machine in OrcaSlicer, then export again"
                    } else {
                        ""
                    }
                );
                log::info!("{}", self.status);
            }
            Err(error) => {
                self.status = format!("could not write {}: {error}", path.display());
                log::error!("{}", self.status);
            }
        }
    }

    /// Export the sculpt to a staging file and open it in OrcaSlicer.
    ///
    /// A staging path under the state directory rather than a dialog: this is a
    /// handoff and not a save, the file is the slicer's input rather than the
    /// user's document, and asking where to put something they will not look
    /// for again is a question with no useful answer.
    ///
    /// 3MF rather than STL, because it carries units. An STL does not, so a
    /// slicer has to guess millimetres, and every slicer guessing right is a
    /// convention rather than a guarantee.
    ///
    /// Runs on the update thread, like `export` already does. The export itself
    /// is the slow half and it is the same work the Export buttons do.
    pub(super) fn hand_to_slicer(&mut self) {
        let Some(slicer) = crate::slicer::find() else {
            self.status = "could not find OrcaSlicer — install it and try again".to_string();
            return;
        };
        let Some(staged) = crate::paths::state_file("handoff.3mf") else {
            self.status = "could not work out where to put the file".to_string();
            return;
        };
        if let Some(parent) = staged.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.status = format!("could not make {}: {error}", parent.display());
            return;
        }

        self.export(ExportFormat::ThreeMf, &staged);
        // `export` refuses a model that would not print, and says so in the
        // status. Handing the slicer a file that was never written would
        // replace that message with a less useful one.
        if !staged.is_file() || self.status.contains("not exported") {
            return;
        }

        match crate::slicer::open(&slicer, &staged) {
            Ok(()) => {
                self.status = format!("opened in {}", slicer.display());
            }
            Err(why) => self.status = format!("could not open the slicer: {why}"),
        }
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    /// `update` returns a `Task` now. Tests do not run the iced runtime, so
    /// there is nothing to hand it to and dropping it is correct — but it must
    /// be dropped deliberately rather than by `#[allow]`, or a test that should
    /// have driven a dialog would pass silently.
    fn update(app: &mut Hani, message: Message) {
        let task = app.update(message);
        drop(task);
    }

    use hani_core::BrickCoord;

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// The starting ball has to straddle every mirror plane equally, or turning
    /// on symmetry works against a model that was never centred.
    ///
    /// It always has -- but "I measured it once" is not a guarantee, and the
    /// defaults it depends on (`MODEL_RADIUS_MM`, `VOXEL_SIZE_MM`, and the fact
    /// that voxel index 0 sits exactly on the origin) are three separate things
    /// a future change could move. This is what makes it permanent.
    #[test]
    fn the_diagnostics_carry_what_a_bug_report_needs() {
        let app = app();
        let report = app.diagnostics();
        for expected in ["HaniSculpt", "model:", "view:", "tablet:", "spacemouse:"] {
            assert!(report.contains(expected), "diagnostics are missing {expected}:\n{report}");
        }
        // The platform line is one or the other, never neither. `session:`
        // answers "Wayland or X11, and which compositor", which only Linux can
        // be asked; everywhere else the OS version is the useful fact and
        // `os:` carries it. Asserting the pair rather than one name is what
        // stops a future platform silently reporting no platform at all --
        // this test demanded `session:` on every OS and passed for a year
        // because only one OS was built.
        let platform = if cfg!(target_os = "linux") { "session:" } else { "os:" };
        assert!(
            report.contains(platform),
            "diagnostics name no platform; expected {platform}:\n{report}"
        );
        // The commit is what ties a binary to its source, which is an AGPL
        // obligation rather than a nicety.
        assert!(report.contains(build_commit()));
    }

    #[test]
    fn the_default_model_is_centred_on_every_mirror_plane() {
        let app = app();
        let (mesh, _) = app.doc.active_volume().export_mesh();
        assert!(!mesh.positions.is_empty());

        let mut low = Vec3::splat(f32::MAX);
        let mut high = Vec3::splat(f32::MIN);
        for position in &mesh.positions {
            low = low.min(*position);
            high = high.max(*position);
        }

        // Well under a voxel: the lattice is symmetric about the origin, so the
        // only error available is f32 rounding.
        let tolerance = VOXEL_SIZE_MM * 0.01;
        let midpoint = (low + high) * 0.5;
        for (axis, offset) in [("x", midpoint.x), ("y", midpoint.y), ("z", midpoint.z)] {
            assert!(offset.abs() < tolerance, "the model is off centre on {axis} by {offset} mm");
        }

        // And it reaches the same distance either way along each axis, which is
        // what "equal sides of the centreline" actually means.
        for (axis, lo, hi) in [("x", low.x, high.x), ("y", low.y, high.y), ("z", low.z, high.z)] {
            assert!(
                (hi + lo).abs() < tolerance,
                "the model reaches {hi} one way on {axis} and {lo} the other"
            );
        }

        // The field itself, in mirrored pairs, not just the mesh it produced.
        for step in 0..24 {
            let probe = Vec3::new(
                MODEL_RADIUS_MM - 1.0 + step as f32 * 0.05,
                step as f32 * 0.37,
                step as f32 * -0.21,
            );
            for flip in
                [Vec3::new(-1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, 1.0), Vec3::new(1.0, 1.0, -1.0)]
            {
                let here = app.doc.active_volume().sample_world(probe);
                let mirrored = app.doc.active_volume().sample_world(probe * flip);
                assert!(
                    (here - mirrored).abs() < 1.0e-4,
                    "the field disagrees across {flip:?}: {here} against {mirrored}"
                );
            }
        }
    }

    #[test]
    fn the_default_model_exports_watertight() {
        // The most basic promise of a printing tool: what it opens with can be
        // printed.
        let app = app();
        let (_, report) = app.doc.active_volume().export_mesh();
        assert!(
            report.is_printable(),
            "the model the application starts with does not print: {}",
            report.summary()
        );
    }

    #[test]
    fn a_sculpted_model_exports_watertight() {
        let mut app = app();
        app.viewport_size = Vec2::new(800.0, 600.0);
        app.brush.strength = 0.7;
        for offset in 0..6 {
            app.on_pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: iced::Vector::new(400.0 + offset as f32 * 8.0, 300.0),
                size: iced::Vector::new(800.0, 600.0),
            });
            app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        }

        let (_, report) = app.doc.active_volume().export_mesh();
        assert!(report.is_printable(), "{}", report.summary());
    }

    #[test]
    fn resampling_finer_increases_detail_and_keeps_it_printable() {
        let mut app = app();
        let (_, before) = app.doc.active_volume().export_mesh();
        let coarse_voxel = app.doc.voxel_size();

        update(&mut app, Message::Resample(coarse_voxel / 2.0));

        assert!((app.doc.voxel_size() - coarse_voxel / 2.0).abs() < 1.0e-6);
        let (_, after) = app.doc.active_volume().export_mesh();
        assert!(
            after.triangles > before.triangles * 2,
            "finer voxels should give far more triangles: {} against {}",
            after.triangles,
            before.triangles
        );
        assert!(after.is_printable(), "{}", after.summary());
        assert!(app.status.contains("resampled"), "the interface should say what happened");
    }

    #[test]
    fn resampling_will_not_go_past_the_limits() {
        // Memory grows as the inverse cube of the voxel size, so the bounds stop
        // a click walking the model past what the mesh pool has been measured to
        // hold. Checked on the clamp rather than by resampling, because actually
        // building a model at the limit is a gigabyte of work.
        assert_eq!(Hani::clamped_voxel_size(0.0000001), FINEST_VOXEL_MM);
        assert_eq!(Hani::clamped_voxel_size(1000.0), COARSEST_VOXEL_MM);
        assert_eq!(Hani::clamped_voxel_size(0.25), 0.25);

        // And the interface offers a step that lands inside the limits. Three
        // halvings from the default are allowed and four are not: 0.25 halved
        // three times is 0.03125, which is the finest rung of the ladder and
        // the one resin work needs. It was two halvings until 2026-08-21, when
        // the floor came down from 0.06 to 0.03 for exactly that rung.
        const _: () = assert!(FINEST_VOXEL_MM > VOXEL_SIZE_MM / 16.0);
        const _: () = assert!(FINEST_VOXEL_MM < VOXEL_SIZE_MM / 4.0);
    }

    #[test]
    fn resampling_clears_history_and_schedules_a_full_remesh() {
        let mut app = app();
        app.viewport_size = Vec2::new(800.0, 600.0);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: iced::Vector::new(400.0, 300.0),
            size: iced::Vector::new(800.0, 600.0),
        });
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        assert!(app.history.can_undo());

        let old_coords: Vec<BrickCoord> = app.doc.active_volume().brick_coords().collect();
        let finer = app.doc.voxel_size() / 2.0;
        update(&mut app, Message::Resample(finer));

        assert!(
            !app.history.can_undo(),
            "history refers to bricks at the old resolution and must be dropped"
        );
        assert!(app.perf.dirty_bricks > old_coords.len(), "the whole model has to be remeshed");
    }

    #[test]
    fn resampling_to_the_current_size_does_nothing() {
        let mut app = app();
        let before = app.doc.active_volume().brick_count();
        let same = app.doc.voxel_size();
        update(&mut app, Message::Resample(same));
        assert_eq!(app.doc.active_volume().brick_count(), before);
        assert!(app.status.is_empty(), "nothing happened, so nothing should be reported");
    }

    // --- the N-body export -------------------------------------------------

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("hani-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// An application holding `count` small spheres, laid out along X so they
    /// do not touch.
    ///
    /// A coarse lattice and a small radius on purpose: this is about how many
    /// bodies reach a file, and twelve default spheres at 0.25 mm would spend
    /// the whole test welding.
    fn app_with_bodies(count: usize) -> Hani {
        let mut app = app();
        let mut first = Volume::new(1.0);
        first.seed_sphere(Vec3::ZERO, 8.0);
        let mut doc = Document::from_volume(first);
        for index in 1..count {
            let mut volume = Volume::new(1.0);
            volume.seed_sphere(Vec3::new(index as f32 * 48.0, 0.0, 0.0), 8.0);
            doc.add_body(format!("Body {}", index + 1), volume);
        }
        app.doc = doc;
        app.rebuild_everything();
        app
    }

    /// Turn one row's eye off through the document's own snapshot path, which
    /// is what the panel will do.
    fn hide(app: &mut Hani, at: usize) {
        let id = app.doc.nodes()[at].id;
        let mut meta = app.doc.meta(id).expect("the row is in the document");
        meta.visible = false;
        app.doc.set_meta(&meta);
    }

    /// **Twelve bodies with one hidden export eleven, and the status says so in
    /// those words.**
    ///
    /// Decisions 4 and 10 both rest on this string. The eye is one bit in a
    /// forty-byte node record with no checksum over it, and it is the bit that
    /// decides whether a part reaches the printer -- a flipped bit is a legal
    /// value that loads without any error at all. The brick stream has a
    /// checked distance decode defending it; the node table has nothing. Naming
    /// the count is the whole of that defence, which is why it is asserted
    /// rather than assumed.
    #[test]
    fn a_hidden_body_is_omitted_and_the_status_names_how_many() {
        let directory = scratch("export-hidden");
        let path = directory.join("twelve.obj");
        let mut app = app_with_bodies(12);
        hide(&mut app, 4);

        app.export(ExportFormat::Obj, &path);

        assert!(
            app.status.contains("exported 11 of 12 bodies; 1 hidden"),
            "the omitted count is not in the status: {}",
            app.status
        );
        let written = std::fs::read_to_string(&path).expect("the file should be there");
        let objects: Vec<&str> = written.lines().filter(|line| line.starts_with("o ")).collect();
        assert_eq!(objects.len(), 11, "eleven bodies should have reached the file: {objects:?}");
        assert!(
            !objects.contains(&"o Body 5"),
            "the hidden body reached the file anyway: {objects:?}"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// **And a document with nothing hidden still names the count.**
    ///
    /// Split out from the test above because it is the case a conditional
    /// message would get wrong, and a conditional message is the obvious
    /// "improvement" somebody makes later: if the line only appears when
    /// something was hidden, silence means either "nothing was hidden" or "the
    /// count was never worked out", and those are the two readings the count
    /// exists to keep apart.
    /// **The export detail reaches the file**, and a full export is still
    /// exactly what it was.
    ///
    /// Two halves, because the setting has a default that must change
    /// nothing: at Full the file and the status line are what they were
    /// before the setting existed, and at anything else the file is smaller,
    /// still passes the verdict, and the status line says what was done to it
    /// in millimetres.
    #[test]
    fn the_export_detail_decimates_the_file_and_full_leaves_it_alone() {
        let directory = scratch("export-detail");
        let full = directory.join("full.stl");
        let draft = directory.join("draft.stl");
        let mut app = app();

        assert_eq!(
            app.export_detail,
            ExportDetail::Full,
            "a fresh application should not decimate"
        );
        app.export(ExportFormat::Stl, &full);
        assert!(app.status.contains("exported 1 of 1 bodies"), "{}", app.status);
        assert!(
            !app.status.contains("within"),
            "a full export's status line mentions a tolerance: {}",
            app.status
        );

        update(&mut app, Message::ExportDetailChanged(ExportDetail::Draft));
        assert_eq!(app.export_detail, ExportDetail::Draft, "the setting did not arrive");
        app.export(ExportFormat::Stl, &draft);
        assert!(app.status.contains("exported 1 of 1 bodies"), "{}", app.status);
        assert!(
            app.status.contains("triangles, within") && app.status.contains("mm of the sculpt"),
            "a decimated export's status line does not say what was done: {}",
            app.status
        );

        let full_bytes = std::fs::metadata(&full).expect("the full export was written").len();
        let draft_bytes = std::fs::metadata(&draft).expect("the draft export was written").len();
        assert!(
            draft_bytes * 4 < full_bytes,
            "the draft is {draft_bytes} bytes against {full_bytes}: it was not decimated"
        );
        // A binary STL is 84 bytes and then 50 a triangle, so the size is the
        // triangle count and a decimated file still has to be a whole number
        // of them.
        assert_eq!((draft_bytes - 84) % 50, 0, "the draft is not a well formed binary STL");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_document_with_nothing_hidden_still_names_the_count() {
        let directory = scratch("export-none-hidden");
        let path = directory.join("one.stl");
        let mut app = app();

        app.export(ExportFormat::Stl, &path);

        assert!(
            app.status.contains("exported 1 of 1 bodies; 0 hidden"),
            "the count is missing from a plain single-body export: {}",
            app.status
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    /// **An export while soloed writes every body the EYE is showing, including
    /// the ones solo is hiding, and names only the eye-hidden count.**
    ///
    /// This is the line where a view mode would be at its most dangerous. Solo
    /// is a way of looking at a document; export is what reaches a printer. If
    /// the two ever share a visibility answer, a user who soloed one part to
    /// inspect it and then pressed Export gets a file with eleven parts missing
    /// and a status line that agrees with them -- and the failure is discovered
    /// after the print. `export` reads `saved_visibility`, and `saved_visibility`
    /// takes no solo parameter at all, so this is a test of a call that has no
    /// other form.
    #[test]
    fn an_export_while_soloed_still_writes_every_body_the_eye_shows() {
        let directory = scratch("export-soloed");
        let path = directory.join("twelve.obj");
        let mut app = app_with_bodies(12);
        hide(&mut app, 4);

        // Solo something else entirely, so eleven of the twelve rows are not
        // drawn and only ONE of them is hidden by an eye.
        let soloed = app.doc.nodes()[7].id;
        drop(app.update(Message::SoloEntered(soloed)));
        assert_eq!(app.solo, Some(soloed), "the fixture never entered solo");
        let mut drawn = Vec::new();
        app.doc.display_visibility(app.solo, &mut drawn);
        assert_eq!(
            drawn.iter().filter(|shown| **shown).count(),
            1,
            "the fixture is not actually hiding anything on screen"
        );

        app.export(ExportFormat::Obj, &path);

        assert!(
            app.status.contains("exported 11 of 12 bodies; 1 hidden"),
            "solo reached the export, or the count now includes it: {}",
            app.status
        );
        let written = std::fs::read_to_string(&path).expect("the file should be there");
        let objects: Vec<&str> = written.lines().filter(|line| line.starts_with("o ")).collect();
        assert_eq!(objects.len(), 11, "solo dropped ten parts out of the print: {objects:?}");
        assert!(
            !objects.contains(&"o Body 5"),
            "the eye-hidden body reached the file anyway: {objects:?}"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A single body's STL is byte for byte what the single-mesh writer
    /// produces, end to end through the application.
    ///
    /// **What this proves on its own is narrower than its name suggests**, and
    /// worth stating so nobody reads it as the byte pin it is not. Both sides
    /// end in `stl::write_all`, so a change to the writer moves them together
    /// and this test cannot see it. What it does see is everything the
    /// application wraps around the writer -- `export_bodies`, the per-body
    /// weld, `document_verdict`, the file creation -- adding nothing to a
    /// one-body file. The bytes themselves are pinned in `hani-core` by
    /// `export::stl::tests::a_known_mesh_writes_the_bytes_committed_in_the_golden`
    /// against a committed fixture. The two together are what pins the file a
    /// user receives; neither one does it alone.
    ///
    /// The shipped, verified property: an STL out of this build has been opened
    /// in a slicer and printed. STL carries no object names, so it is the one
    /// format where the N-body path can be identical rather than merely
    /// equivalent -- OBJ and 3MF now carry the body's name where they used to
    /// carry the literal `HaniSculpt`, which is the only byte that moves.
    #[test]
    fn a_single_body_export_is_byte_identical_to_the_single_mesh_writer() {
        let directory = scratch("export-identical");
        let path = directory.join("one.stl");
        let mut app = app();

        app.export(ExportFormat::Stl, &path);
        let written = std::fs::read(&path).expect("the file should be there");

        let (mesh, report) = app.doc.active_volume().export_mesh();
        assert!(report.is_printable());
        let mut expected = Vec::new();
        hani_core::export::stl::write(&mesh, &mut expected).unwrap();
        assert_eq!(written, expected, "the N-body path changed a one-body STL");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The user's own filament slots reach the exported package.
    ///
    /// **This is the whole of what increment 1 is for.** The `paint_color`
    /// writer and its `Filaments` shipped complete and unreferenced: nothing
    /// outside `export/threemf.rs` mentioned either, so every 3MF a user ever
    /// received declared the four built-in colours regardless of what was in
    /// their machine. The assertion is a substring rather than a parsed zip
    /// because the writer stores its entries uncompressed, so the settings part
    /// is literally in the file -- and reaching for a zip reader to check one
    /// string would be the more fragile of the two.
    #[test]
    fn the_users_own_filaments_reach_the_exported_package() {
        let directory = scratch("export-palette");
        let path = directory.join("painted.3mf");
        let mut app = app();
        app.palette.slots[0].colour = "#123456".to_string();
        app.palette.slots[1].material = "PETG".to_string();

        app.export(ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();

        assert!(
            written.contains("\"#123456\""),
            "the export declared the built-in colours instead of the user's"
        );
        assert!(written.contains("\"PETG\""), "the material did not travel");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A user who never touches the palette gets exactly the file they got
    /// before, byte for byte.
    ///
    /// The default palette is DERIVED from `Filaments::default()` rather than
    /// restating it, and this is what holds that true: if the two ever drift,
    /// every existing export silently changes and this fails.
    #[test]
    fn an_untouched_palette_exports_the_same_bytes_as_before() {
        let directory = scratch("export-palette-default");
        let path = directory.join("plain.3mf");
        let mut app = app();

        app.export(ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();

        // The default palette must still be the writer's own defaults. Asserted
        // on the settings part rather than on the whole file's bytes, because
        // the placement transform is read from the developer's own OrcaSlicer
        // presets -- a byte comparison here would pass or fail depending on
        // which printer the machine running the suite happens to have selected.
        for colour in hani_core::export::threemf::Filaments::default().colours {
            assert!(
                written.contains(&format!("\"{colour}\"")),
                "the default palette and the writer's own defaults have drifted apart: \
                 {colour} is missing"
            );
        }

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The exported model sits ON the bed, not half under it.
    ///
    /// A 3MF opened as a project is placed exactly where the file says, and
    /// with no transform that was the plate's front-left corner with half the
    /// sculpt below z = 0 -- reported from OrcaSlicer against a real export on
    /// 2026-09-02. The centring half needs the user's plate size and is absent
    /// on a machine with no OrcaSlicer; the drop onto the bed is unconditional
    /// and is what this pins.
    #[test]
    fn an_exported_model_sits_on_the_bed_rather_than_half_under_it() {
        let directory = scratch("export-on-bed");
        let path = directory.join("placed.3mf");
        let mut app = app();

        app.export(ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();

        let item = written
            .lines()
            .find(|line| line.contains("<item "))
            .expect("the build has to reference the object")
            .to_string();
        let transform = item
            .split_once("transform=\"")
            .map(|(_, rest)| rest.split('"').next().unwrap_or_default().to_string())
            .expect("a default sphere straddles z = 0, so it must carry a transform");
        let numbers: Vec<f32> =
            transform.split_whitespace().filter_map(|word| word.parse().ok()).collect();
        assert_eq!(numbers.len(), 12, "a 3MF item transform is 12 numbers: {transform}");

        // The lowest point of the sphere, in print space, plus the lift the
        // transform applies, has to land on the bed.
        let lifted = numbers[11];
        let mut lowest = f32::INFINITY;
        for line in written.lines().filter(|line| line.contains("<vertex ")) {
            if let Some((_, rest)) = line.split_once("z=\"")
                && let Ok(z) = rest.split('"').next().unwrap_or_default().parse::<f32>()
            {
                lowest = lowest.min(z);
            }
        }
        assert!(lowest.is_finite(), "no vertices in the exported file");
        assert!(
            (lowest + lifted).abs() < 1.0e-3,
            "the model's lowest point lands at {} instead of on the bed",
            lowest + lifted
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// Several bodies in one STL are named as fused, because the format has
    /// nowhere to put an object boundary and a slicer will load them as one
    /// part. Better said here than discovered after slicing.
    #[test]
    fn an_stl_of_several_bodies_says_they_arrive_as_one_part() {
        let directory = scratch("export-stl-fused");
        let path = directory.join("three.stl");
        let mut app = app_with_bodies(3);

        app.export(ExportFormat::Stl, &path);
        assert!(app.status.contains("exported 3 of 3 bodies"), "{}", app.status);
        assert!(
            app.status.contains("STL fuses them into one part"),
            "an STL of three bodies did not say what it did with them: {}",
            app.status
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    /// **A body that would not print refuses the whole export, and refuses it
    /// before the file is opened.**
    ///
    /// `File::create` truncates, so a verdict taken after it destroys the file
    /// the user was about to print -- and the natural per-body refactor is
    /// exactly the one that moves the check down past the open. The previous
    /// export was left in place here, unread and unchanged.
    #[test]
    fn an_unprintable_body_refuses_before_the_file_is_opened() {
        let directory = scratch("export-refusal");
        let path = directory.join("keepme.obj");
        std::fs::write(&path, b"the previous export, which must survive").unwrap();

        let mut app = app_with_bodies(2);
        // An empty body prints nothing, and the sum over the document would
        // still read as watertight -- see `document_verdict`.
        let empty = app.doc.nodes()[1].id;
        *app.doc.volume_mut(empty).expect("a body") = Volume::new(1.0);

        app.export(ExportFormat::Obj, &path);

        assert!(app.status.starts_with("not exported"), "reported: {}", app.status);
        assert!(app.status.contains("Body 2"), "the refusal must name the body: {}", app.status);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"the previous export, which must survive",
            "the refusal opened and truncated the file before deciding"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// **The detail guard measures the whole document, not the active body.**
    ///
    /// It never did: `too_fine_for_the_pool` reads `doc_stats`, which is a sum
    /// over every body, and the numbers below are what makes the difference
    /// visible. Two bodies of 0.8 GiB each is 1.6 GiB, and one halving of the
    /// voxel quadruples a shell -- 6.4 GiB, past the ceiling. Either body ALONE
    /// would come to 3.2 GiB and be admitted. A guard that asked the active
    /// body would step straight past the ceiling with the second body's bricks
    /// unaccounted for, and the first thing to report it would be the machine
    /// swapping.
    #[test]
    fn a_two_body_document_over_the_ceiling_refuses_the_finer_step() {
        let mut app = app_with_bodies(2);
        assert_eq!(app.doc.body_count(), 2);

        // `doc_stats` really is the sum, measured rather than assumed.
        let per_body: usize =
            app.doc.bodies().map(|(_, volume)| volume.stats().resident_bytes).sum();
        assert_eq!(app.doc_stats.resident_bytes, per_body);
        assert!(
            app.doc_stats.resident_bytes > app.doc.active_volume().stats().resident_bytes,
            "the fixture's two bodies must not cost the same as one"
        );

        let each = 0.8 * 1024.0 * 1024.0 * 1024.0;
        assert!(each * 4.0 < MAX_VOLUME_BYTES, "either body alone has to be admitted");
        assert!(each * 2.0 * 4.0 > MAX_VOLUME_BYTES, "the pair has to be refused");
        app.doc_stats.resident_bytes = (each * 2.0) as usize;

        let before = app.doc.voxel_size();
        update(&mut app, Message::Resample(before / 2.0));

        assert!(app.doc.voxel_size() > before / 2.0, "the halving should not have been allowed");
        assert!(
            app.status.contains("finest the mesh pool holds"),
            "the cap should be reported: {}",
            app.status
        );
    }
}
