// SPDX-License-Identifier: AGPL-3.0-only

//! The application's own life: the window title, what it listens to, the
//! actions that would discard work and the prompt that guards them, and an
//! update's install and restart. Moved out of `app.rs` (issue 189).

use super::*;

impl Hani {
    /// What the sculpt is called, for the window title and the header.
    ///
    /// One function so the two cannot drift: the header used to derive this
    /// itself.
    pub(crate) fn document_name(&self) -> String {
        match &self.project_path {
            Some(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            None => "untitled".to_string(),
        }
    }

    /// The window title, which is where the unsaved marker is most visible.
    ///
    /// iced re-evaluates this every frame through `program::with_title`, so the
    /// star appears the moment a stroke lands with nothing else to wire.
    pub fn title(&self) -> String {
        let star = if self.unsaved { "*" } else { "" };
        format!("{}{star} — HaniSculpt", self.document_name())
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            // Drives the frame rate readout and keeps the viewport presenting
            // while a stroke is in flight.
            iced::window::frames().map(|_| Message::Frame),
            // The window manager's close button. `main` sets
            // `exit_on_close_request(false)`, so this is the ONLY thing that
            // ends the application -- if this subscription is ever dropped the
            // window becomes unclosable.
            iced::window::close_requests().map(Message::CloseRequested),
            // Keyboard shortcuts, on events the widget tree IGNORED. They
            // lived in the shader widget for a year, captured window-wide,
            // "because the shader already receives every event" -- and that
            // capture stole 1-7, s, u, x, y and z from every text field in
            // the application, because the shader traverses before the panel.
            // Filtering on Ignored is what makes them focus-aware: a focused
            // text input consumes its keystrokes, and a shortcut fires only
            // when nothing else wanted the key.
            //
            // What that callback must NOT do is decide what a key means.
            // `listen_with` takes a bare `fn` pointer, not a closure, so there
            // is no `self` there and no way to get one: whether a modal is up,
            // whether a gesture is in flight, whether the document can even
            // take the change are all invisible from inside it. It forwards
            // the key and `on_key` decides. Reaching for a static or a global
            // to dodge that is how the two halves drift apart.
            //
            // It is `key_event`, a named function rather than an inline
            // closure, purely so the tests can call it -- see the tests on it
            // for what silently breaks when it is wrong.
            iced::event::listen_with(key_event),
        ])
    }

    /// Run an action that discards the document, now that it has been allowed.
    pub(super) fn run_pending(&mut self, action: PendingAction) -> Task<Message> {
        match action {
            PendingAction::NewSculpt => {
                self.reset_sculpt();
                Task::none()
            }
            PendingAction::Open => Task::perform(pick_project_to_open(), Message::OpenChosen),
            PendingAction::OpenRecent(path) => {
                self.open_project(&path);
                Task::none()
            }
            PendingAction::RecoverAutosave => {
                self.recover_autosave();
                Task::none()
            }
            PendingAction::OpenStored(name) => {
                self.fetch_kept(name, crate::message::AfterFetch::Open)
            }
            PendingAction::Import => Task::perform(pick_mesh_to_import(), Message::ImportChosen),
            // Destroying the last window is what ends a non-daemon iced
            // application (`iced_winit` sends `Control::Exit` once the window
            // manager is empty), so this really does quit.
            // In a browser there is nothing to quit to: closing the drawing
            // surface leaves a dead page. A tab closes itself; a hosting
            // webview is told, with the leave warning taken down, since what
            // was not saved has just been saved or let go. See
            // `web::tell_host`.
            PendingAction::Quit(_) if cfg!(target_arch = "wasm32") => {
                crate::web::warn_on_leaving(false);
                crate::web::tell_host("close");
                Task::none()
            }
            PendingAction::Quit(id) => iced::window::close(id),
            PendingAction::Restart(id) => self.install_and_restart(id),
        }
    }

    /// Swap the binary and relaunch, or say why not.
    ///
    /// Everything before the `rename` inside `apply::install` is
    /// non-destructive, so every refusal here leaves the install exactly as it
    /// was. Runs on the interface thread deliberately: the download already
    /// happened, so what is left is a handful of filesystem operations, and
    /// doing them in a task would mean the window could be closed underneath a
    /// half-finished swap.
    /// Swap the binary and relaunch, or say why not.
    ///
    /// **Off the interface thread, and that is not an optimisation.** An earlier
    /// revision did all of this inline, reasoning that after the download only
    /// "a handful of filesystem operations" remain. True on Linux. On Windows
    /// the sequence opens the staged file for exclusive access first and retries
    /// for up to sixty seconds, which is where antivirus interference is
    /// absorbed -- so inline meant the window froze solid for up to a minute
    /// with nothing drawn and nothing said. A Windows tester clicked install and
    /// reported that nothing happened, which is exactly what that looks like.
    ///
    /// Everything before the rename is non-destructive, so a refusal here leaves
    /// the install exactly as it was.
    pub(super) fn install_and_restart(&mut self, id: iced::window::Id) -> Task<Message> {
        // **First, above everything, including the clear below.** A second
        // press while a swap is running must change nothing at all: clearing
        // the note first and then returning would wipe "Installing build N…"
        // off the card and leave a running install saying nothing, which is the
        // "clicked install, nothing happened" report this whole path exists to
        // stop producing.
        if self.installing_update {
            return Task::none();
        }
        // A new attempt, so last attempt's refusal stops being the answer.
        self.update_failure = None;
        let Some(staged) = self.downloaded_update.clone() else {
            self.update_says(
                crate::update::journal::Step::Install,
                "there is no downloaded update to install".to_string(),
            );
            return Task::none();
        };
        let Some(offer) = self.offer.clone() else {
            // **Reachable, and it used to return in silence.** Turning update
            // checking off clears `self.offer` but leaves `downloaded_update`
            // alone, and the Help entry that starts an install is gated on the
            // download alone -- so a user who downloads, unticks Check for
            // updates, then presses Install lands exactly here. With the clear
            // above and nothing said, the card would go blank and the press
            // would look like a dead button.
            self.update_says(
                crate::update::journal::Step::Install,
                "the update was withdrawn -- switch checking back on and look again".to_string(),
            );
            return Task::none();
        };
        let Some(target) = self.exe.clone() else {
            self.update_says(
                crate::update::journal::Step::Install,
                "this copy cannot update itself".to_string(),
            );
            return Task::none();
        };
        self.installing_update = true;
        // Said before the work starts, because the work can take a minute and a
        // button that does nothing visible is indistinguishable from a broken
        // one -- which is how this was reported.
        self.status = format!("installing build {}…", offer.build);

        let running_build = build_number();
        let running_commit = build_commit();
        let resume = self.project_path.clone();
        Task::perform(
            async move {
                use crate::update::journal::{self, Step};
                // Every refusal below is written down where it is raised, and
                // not back in the message arm: only here is it known whether
                // the swap or the relaunch after it was the thing that refused,
                // and that distinction is the point of `Step::Relaunch`.
                let build = Some(offer.build);
                let refused_at = |step: Step, why: String| {
                    journal::failed(step, build, &why);
                    why
                };
                // Never waited on: another copy mid-apply means this one says so
                // rather than blocking behind someone else's 33 MB.
                let lock = crate::update::apply::Lock::take()
                    .map_err(|why| refused_at(Step::Install, why.to_string()))?;
                crate::update::apply::gates(&target, running_build, running_commit)
                    .map_err(|why| refused_at(Step::Install, why.to_string()))?;
                // What we are reverting TO, recorded before anything moves.
                let previous = crate::update::apply::sha256_of(&target.path)
                    .map_err(|why| refused_at(Step::Install, why.to_string()))?;
                crate::update::apply::install(&target, &staged, &previous)
                    .map_err(|why| refused_at(Step::Install, why.to_string()))?;
                // **The swap is done, so the record goes down before anything
                // that follows can fail.** What follows moves the floor and
                // spawns a process, and a log that said only "relaunch failed"
                // would leave a maintainer unable to tell whether the new build
                // is at the target path -- which is the first thing they need.
                journal::ok(Step::Install, build);
                // **The floor rises here and nowhere else.** Only a completed
                // apply may move it, so merely being shown a manifest cannot.
                // After the swap rather than before: a floor raised for an update
                // that then failed would refuse the manifest that could fix it.
                crate::update::record_applied(offer.seq, offer.key_epoch, offer.build);
                // `resume` is the channel that survives the restart -- argv would
                // not, because a rollback can spawn an OLDER binary that has
                // never heard of the flag.
                if let Some(marker) = crate::update::apply::marker_path(&target) {
                    let _ = crate::update::apply::write_marker(
                        &marker,
                        &crate::update::apply::Pending {
                            build: offer.build,
                            attempt: 0,
                            resume,
                            path: target.path.clone(),
                        },
                    );
                }
                // Released before the spawn: the child opens its own handle and
                // would otherwise race the parent's exit and be refused its own
                // first state write.
                lock.release();
                // **Spawn first; the caller exits only if this worked.** A user
                // who clicked restart and got a closed window with nothing
                // coming back has no terminal and no way to tell whether their
                // install survived.
                //
                // Retried, because nothing re-waits for antivirus after the new
                // file lands at the target path. The attempt count rides along
                // in the message and in the log: it is what tells a permanent
                // block apart from the transient hold this assumes, and until
                // one of those two is confirmed the count is worth more than
                // the retry is.
                crate::update::apply::relaunch(&target.path).map_err(|refused| {
                    let tries = if refused.tries == 1 {
                        "one try".to_string()
                    } else {
                        format!("{} tries", refused.tries)
                    };
                    let why = format!(
                        "build {} is installed, but starting it failed after {tries} ({}) -- this \
                         window is still the old build; close it and start HaniSculpt again",
                        offer.build, refused.error
                    );
                    refused_at(Step::Relaunch, why)
                })?;
                Ok(offer.build)
            },
            move |outcome: Result<u64, String>| Message::UpdateInstalled(id, outcome),
        )
    }

    /// Do `action`, or ask first if there is work to lose.
    pub(super) fn guard(&mut self, action: PendingAction) -> Task<Message> {
        if self.would_lose_work(&action) {
            // The prompt is about to draw over the viewport, and a menu left
            // open would sit on top of it.
            self.top_menu = None;
            self.menu = None;
            self.confirm = Some(action);
            return Task::none();
        }
        self.run_pending(action)
    }

    /// Whether carrying `action` out would cost the user something they cannot
    /// get back with one gesture.
    ///
    /// `unsaved` for everything, and for an import **the body count as well,
    /// whether or not anything is unsaved**. Import replaces every body with
    /// the mesh it reads. With a saved five-body project open, `unsaved` is
    /// false, so it used to discard all five having asked nothing: no dialog,
    /// no prompt, nothing in the status line. The file on disk survives, which
    /// is the only reason this is a prompt and not a refusal — reassembling
    /// five bodies by hand is not a recovery a user should have to make because
    /// a menu item did more than its name said.
    ///
    /// Open is deliberately NOT guarded the same way: it puts a document the
    /// user picked in place of this one, which is what its name promises.
    fn would_lose_work(&self, action: &PendingAction) -> bool {
        self.unsaved || (matches!(action, PendingAction::Import) && self.doc.body_count() > 1)
    }

    /// Act on the answer to the unsaved-work prompt.
    pub(super) fn answer_confirm(&mut self, choice: ConfirmChoice) -> Task<Message> {
        match choice {
            ConfirmChoice::Cancel => {
                self.confirm = None;
                Task::none()
            }
            ConfirmChoice::Discard => match self.confirm.take() {
                Some(action) => self.run_pending(action),
                None => Task::none(),
            },
            // In a browser, kept there. See `app/shelf.rs`.
            #[cfg(target_arch = "wasm32")]
            ConfirmChoice::Save => self.shelf_save(crate::message::AfterSave::Continue),
            #[cfg(not(target_arch = "wasm32"))]
            ConfirmChoice::Save => match self.project_path.clone() {
                // It has a file already, so this is a plain write and the
                // answer is known immediately.
                Some(path) => {
                    self.save_project(&path);
                    if self.unsaved {
                        // Failed. `status` says why; the prompt stays up so the
                        // user can still choose to discard or cancel rather
                        // than losing the work to a write that did not happen.
                        return Task::none();
                    }
                    match self.confirm.take() {
                        Some(action) => self.run_pending(action),
                        None => Task::none(),
                    }
                }
                // No file yet, so ask for one. The prompt stays up until the
                // dialog comes back, which is what `SavedThenContinue` settles.
                None => Task::perform(pick_project_to_save(), Message::SavedThenContinue),
            },
        }
    }

    /// Report an update refusal to BOTH places a user might be looking, and
    /// write it down where it survives the window closing.
    ///
    /// **One writer, so a new update outcome cannot reach one surface and miss
    /// the others.** The status line is the general one and is enough whenever
    /// the welcome screen is down; `update_failure` is what the welcome card
    /// draws, because that card covers the status line and carries an install
    /// button of its own; the journal is what is still there tomorrow, and the
    /// only thing a Windows release build leaves behind at all. Anything that
    /// assigns `self.status` directly for an update outcome is a bug of the
    /// shape this exists to prevent.
    pub(super) fn update_says(&mut self, step: crate::update::journal::Step, why: String) {
        let build = self.offer.as_ref().map(|offer| offer.build);
        self.update_says_of(step, build, why);
    }

    /// [`Hani::update_says`] about a stated build.
    ///
    /// The revert path needs this: the build it is about comes from
    /// `crash_revert` -- the build being rolled back FROM -- and not from
    /// `self.offer`, which is whatever an update check last advertised and is
    /// an unrelated number. Recording the offer there put a failed revert and
    /// the successful revert beside it under two different builds.
    pub(super) fn update_says_of(
        &mut self,
        step: crate::update::journal::Step,
        build: Option<u64>,
        why: String,
    ) {
        crate::update::journal::failed(step, build, &why);
        self.update_shows(why);
    }

    /// Show an update outcome that has ALREADY been written down.
    ///
    /// Separate from [`Hani::update_says`] because the failures raised inside
    /// the install task record themselves: only there is it known whether the
    /// swap or the relaunch after it was the thing that refused, and a record
    /// written back here would have to guess from the wording.
    pub(super) fn update_shows(&mut self, why: String) {
        self.update_failure = Some(why.clone());
        self.status = why;
    }
}
