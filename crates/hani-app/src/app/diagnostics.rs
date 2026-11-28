// SPDX-License-Identifier: AGPL-3.0-only

//! What a bug report carries: the status line's trail, the facts of the
//! session, and the diagnostics. Moved out of `app.rs` (issue 189).

use super::*;

impl Hani {
    /// Where exported files go.
    ///
    /// A fixed directory rather than a file dialog: Iced has no file picker and
    /// pulling one in is a dependency and a portal setup for something a first
    /// version can do without. The path is shown in the interface so it is never
    /// a guess.
    /// Everything worth pasting into a bug report.
    ///
    /// Deliberately a string on the clipboard rather than an upload: there is no
    /// server, no endpoint, no telemetry and no consent question. The user can
    /// read exactly what they are sending before they send it.
    ///
    /// The session type is here because it is the single most likely cause of a
    /// bad report: running on XWayland puts the window outside the X screen's
    /// bounds and the compositor stops requesting frames, which reads as a 1 fps
    /// bug and is not one.
    /// Note the status line in the trail if it has changed since last frame.
    ///
    /// Watching one string beat a `set_status` helper through two dozen call
    /// sites: this cannot be forgotten by the next person to add one, and the
    /// per-frame cost is a string comparison that allocates only when something
    /// actually happened.
    ///
    /// The consequence to know about is that a status set and replaced within
    /// a single frame is not recorded. Nothing here does that -- a status is
    /// something a user reads -- and the alternative was a trail with holes in
    /// it nobody would notice.
    pub(super) fn record_status_change(&mut self) {
        if self.status == self.crumbed_status {
            return;
        }
        self.crumbed_status.clone_from(&self.status);
        self.breadcrumbs.crumb(&self.status);
    }

    /// Record the things that are true of this session rather than the things
    /// that happened in it.
    ///
    /// Called once the renderer and the input devices have been looked at, so
    /// there is something to say. These are what make a report from a machine
    /// nobody here owns answerable: which graphics stack, which session type,
    /// whether the devices the application steers with were found at all.
    pub(super) fn record_session_facts(&mut self) {
        self.breadcrumbs.sticky_fact(&format!("[wgpu] {}", self.shared.adapter_summary()));
        self.breadcrumbs.sticky_fact(&format!(
            "[session] {} on {}",
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into()),
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into()),
        ));
        self.breadcrumbs.sticky_fact(&format!("[tablet] {}", self.tablet.describe()));
        self.breadcrumbs
            .sticky_fact(&format!("[spacemouse] {}", self.spacemouse.diagnosis().explain()));
    }

    /// Build the report the dialog would send, or `None` when there is nothing
    /// to send.
    ///
    /// One function for the preview, the clipboard and the upload, so what is
    /// shown is what is sent. A description of the payload that is assembled
    /// separately from the payload is a description that goes stale.
    pub(crate) fn assemble_report(&self) -> Option<crate::report::Report> {
        let draft = self.bug_report.as_ref()?;
        let description = draft.description.text();
        if description.trim().is_empty() {
            return None;
        }
        let (diagnostics, trail) = if draft.with_detail {
            (self.diagnostics(), self.breadcrumbs.all())
        } else {
            (String::new(), Vec::new())
        };
        Some(crate::report::Report::new(
            &description,
            &diagnostics,
            &trail,
            &format!("{} ({})", env!("CARGO_PKG_VERSION"), build_commit()),
            &self.shared.adapter_summary(),
        ))
    }

    pub(crate) fn diagnostics(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(out, "HaniSculpt {} ({})", env!("CARGO_PKG_VERSION"), build_commit());
        // **The build ordinal and the updater's own state, in every report.**
        // Costs two lines and turns every future "it stopped updating" into a
        // one-message diagnosis instead of a conversation: which build they are
        // on, whether it came from the updater, and when the channel last
        // answered. A commit stamp alone cannot say any of that, because a SHA
        // has no order.
        let _ = writeln!(
            out,
            "build: {}",
            build_number()
                .map_or_else(|| "none (not a release build)".to_string(), |n| n.to_string())
        );
        let _ = writeln!(out, "updates: {}", crate::update::outcome_line());
        // **The update history, verbatim, and this is the route by which a
        // failed update reaches a maintainer at all.** A Windows release build
        // has no console, so before the journal existed a refused install left
        // no trace anywhere once the window was shut. A user cannot be asked to
        // go and find a file; Help > Report a bug is the button they already
        // press. A machine that has never updated has no history and gets no
        // heading, rather than a heading over nothing.
        let history = crate::update::journal::for_report();
        if !history.trim().is_empty() {
            let _ = writeln!(out, "update history:\n{}", history.trim_end());
        }
        // **The OS line, and it is not the same question on each platform.**
        // `XDG_SESSION_TYPE` and `XDG_CURRENT_DESKTOP` answer "Wayland or X11,
        // and which compositor", which is the first thing to know about a
        // Linux graphics report. They are unset everywhere else, so this used
        // to print "unknown on unknown" on Windows and macOS -- noise shaped
        // like data, in the one place a reader is scanning for facts. Off
        // Linux the version is the useful thing instead, and `report`'s
        // `os_build` already knows it.
        if cfg!(target_os = "linux") {
            let _ = writeln!(
                out,
                "session: {} on {}",
                std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into()),
                std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into()),
            );
        } else {
            let _ = writeln!(out, "os: {}", crate::report::os_build());
        }
        let _ = writeln!(out, "wgpu: {}", self.shared.adapter_summary());
        let _ = writeln!(
            out,
            "model: voxel {:.3} mm, {} dense + {} uniform bricks, {:.1} MB",
            self.doc.voxel_size(),
            self.doc_stats.dense_bricks,
            self.doc_stats.uniform_bricks,
            self.doc_stats.resident_bytes as f64 / (1024.0 * 1024.0),
        );
        let pool = self.shared.stats();
        let _ = writeln!(
            out,
            "view: {} triangles, {} drawn / {} culled / {} hidden, {:.1} fps",
            pool.triangles,
            pool.drawn,
            pool.culled,
            pool.hidden,
            if self.perf.average_frame_ms() > 0.0 {
                1000.0 / self.perf.average_frame_ms()
            } else {
                0.0
            },
        );
        let _ = writeln!(out, "tablet: {}", self.tablet.describe());
        let _ = writeln!(out, "spacemouse: {}", self.spacemouse.diagnosis().explain());
        if !self.status.is_empty() {
            let _ = writeln!(out, "last message: {}", self.status);
        }
        let trail = self.breadcrumbs.all();
        if !trail.is_empty() {
            let _ = writeln!(out, "\ntrail:");
            for crumb in crate::report::trim_breadcrumbs(&trail) {
                let _ = writeln!(out, "  {crumb}");
            }
        }
        out
    }
}
