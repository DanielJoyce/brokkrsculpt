// SPDX-License-Identifier: AGPL-3.0-only

//! The bug report: what is collected, what is stripped out of it, and the
//! issue it becomes on GitHub.
//!
//! It was sent to TinkerAtlas, as SindriCAD's is; that was retired on
//! 2026-11-27 at the maintainer's word (*"Retire tinkeratlas bug report"*),
//! with the TinkerAtlas account it could be signed with. A report is now an
//! issue on this repository: the dialog opens GitHub's new-issue page with
//! the title and the body in it, and puts the whole body on the clipboard,
//! because a link can carry only so much (see [`Report::link`]). Nothing
//! leaves the machine until the user presses GitHub's own button.
//!
//! # The privacy rule
//!
//! `Hani::diagnostics` was written as something the user reads and pastes
//! themselves, and its doc says so: "the user can read exactly what they are
//! sending before they send it". The same text goes through
//! [`redact_user_paths`] first, and the dialog shows the finished body before
//! anything is opened. An issue is public, which is the more reason for it.

/// Longest first line that becomes the report's title.
///
/// Counted in `char`s and not bytes: cutting a multi-byte character in half
/// would not survive the trip as valid UTF-8.
const TITLE_CHARS: usize = 120;

/// Ordinary breadcrumbs kept, most recent last.
const MAX_TAIL_CRUMBS: usize = 20;
/// Sticky facts kept. See [`trim_breadcrumbs`].
const MAX_STICKY_CRUMBS: usize = 24;

/// Replace the segment after a home-directory marker with `[REDACTED]`.
///
/// Ported verbatim from SindriCAD, whose tests came with it. It handles the
/// Windows drive form, the UNC form with the host preserved, and both unix
/// forms, and a segment ends at a separator, whitespace or a quote — which is
/// what makes it work on log lines rather than only on bare paths.
///
/// It is deliberately not clever. It knows about home directories and nothing
/// else: not emails, not tokens, not hostnames, not the name of a file that
/// happens to sit outside a home directory. Anything collected here has to be
/// worth sending on that basis, rather than on the basis that this will catch
/// what should not have been collected.
pub(crate) fn redact_user_paths(text: &str) -> String {
    fn redact_after(hay: &str, needle_lower: &str, separators: &[char]) -> String {
        let mut out = String::with_capacity(hay.len());
        let lower = hay.to_lowercase();
        let mut i = 0;
        while let Some(rel) = lower[i..].find(needle_lower) {
            let start = i + rel + needle_lower.len();
            out.push_str(&hay[i..start]);
            let rest = &hay[start..];
            let end = rest
                .find(|c: char| {
                    separators.contains(&c) || c.is_whitespace() || c == '"' || c == '\''
                })
                .unwrap_or(rest.len());
            if end > 0 {
                out.push_str("[REDACTED]");
            }
            i = start + end;
        }
        out.push_str(&hay[i..]);
        out
    }

    let text = redact_after(text, "\\users\\", &['\\', '/']);
    let text = redact_after(&text, "/home/", &['/', '\\']);
    redact_after(&text, "/users/", &['/', '\\'])
}

/// True when a crumb carries the `HH:MM:SS ` prefix an ordinary one gets.
fn is_timestamped(crumb: &str) -> bool {
    let bytes = crumb.as_bytes();
    bytes.len() >= 9
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b':'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && bytes[5] == b':'
        && bytes[6].is_ascii_digit()
        && bytes[7].is_ascii_digit()
        && bytes[8] == b' '
}

/// Keep the last twenty ordinary crumbs, but never at the cost of the sticky
/// facts recorded at startup.
///
/// Ported along with the bug it fixes, which is worth repeating because the
/// obvious implementation has it. SindriCAD's was a plain "keep the last
/// twenty", and the facts captured once at startup — the device inventory, the
/// graphics adapter — sit at the front of the trail, so that dropped exactly
/// them the moment the session did anything. In other words it dropped them in
/// every session busy enough to be worth reporting, paying the privacy cost of
/// collecting an inventory and delivering none of its diagnostic value.
///
/// **The timestamp is the discriminator.** `crumb` always prefixes one and
/// `sticky_fact` never does, and a sticky fact that gained one would silently
/// become droppable again. See `breadcrumbs.rs`, which owns the other half of
/// that contract.
pub(crate) fn trim_breadcrumbs(crumbs: &[String]) -> Vec<String> {
    let (ordinary, sticky): (Vec<&String>, Vec<&String>) =
        crumbs.iter().partition(|crumb| is_timestamped(crumb));
    sticky
        .into_iter()
        .take(MAX_STICKY_CRUMBS)
        .chain(ordinary.into_iter().rev().take(MAX_TAIL_CRUMBS).rev())
        .cloned()
        .collect()
}

/// One report: the issue's title and its body.
///
/// Assembled and redacted before it is shown to the user, so what the dialog
/// displays is what the issue says rather than a description of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    pub title: String,
    /// What the user wrote, redacted.
    pub description: String,
    /// The diagnostics and the session's trail, redacted, one line a fact;
    /// empty when the user left them out.
    pub detail: Vec<String>,
}

/// The longest new-issue link opened. GitHub turns away a link much past
/// eight thousand characters, and some browsers cut one sooner.
const MOST_LINK_CHARS: usize = 7000;

impl Report {
    /// Build a report from what the user typed and what the application knows.
    ///
    /// Every free-text field is redacted here rather than at the call sites, so
    /// there is one place to look when asking what leaves the machine.
    pub fn new(
        description: &str,
        diagnostics: &str,
        breadcrumbs: &[String],
        version: &str,
        renderer: &str,
    ) -> Self {
        let description = redact_user_paths(description.trim());
        let first_line = description.lines().next().unwrap_or("Bug report").to_string();
        let title: String = first_line.chars().take(TITLE_CHARS).collect();
        let mut detail = Vec::new();
        if !diagnostics.trim().is_empty() || !breadcrumbs.is_empty() {
            detail.push(format!("HaniSculpt {version}"));
            detail.push(format!(
                "os: {} {} {}",
                std::env::consts::OS,
                os_build(),
                std::env::consts::ARCH
            ));
            if !renderer.is_empty() {
                detail.push(format!("graphics: {}", redact_user_paths(renderer)));
            }
            detail.extend(redact_user_paths(diagnostics).lines().map(str::to_string));
            if !breadcrumbs.is_empty() {
                detail.push("session:".to_string());
                detail.extend(
                    trim_breadcrumbs(breadcrumbs)
                        .into_iter()
                        .map(|crumb| format!("  {}", redact_user_paths(&crumb))),
                );
            }
        }
        Self { title, description, detail }
    }

    /// The issue's body: the description, and the detail folded away under
    /// it, the first `lines` of it.
    fn body_with(&self, lines: usize) -> String {
        if self.detail.is_empty() {
            return self.description.clone();
        }
        let shown = lines.min(self.detail.len());
        let mut detail = self.detail[..shown].join("\n");
        if shown < self.detail.len() {
            detail.push_str("\n(cut short here: the whole report is on the clipboard)");
        }
        format!(
            "{}\n\n<details><summary>Diagnostics</summary>\n\n```text\n{detail}\n```\n\n</details>",
            self.description
        )
    }

    /// The whole body, for the clipboard and the dialog.
    pub fn body(&self) -> String {
        self.body_with(usize::MAX)
    }

    /// GitHub's new-issue page with the title and as much of the body as a
    /// link can carry, and whether any was left out. Whole lines of the detail
    /// go from the end first, so the block stays closed and readable; a
    /// description too long by itself is cut last.
    pub fn link(&self) -> (String, bool) {
        let full = crate::links::new_issue(&self.title, &self.body());
        if full.len() <= MOST_LINK_CHARS {
            return (full, false);
        }
        let mut lines = self.detail.len();
        while lines > 0 {
            lines -= 1;
            let link = crate::links::new_issue(&self.title, &self.body_with(lines));
            if link.len() <= MOST_LINK_CHARS {
                return (link, true);
            }
        }
        let mut description: String = self.description.clone();
        loop {
            let cut =
                Report { description: description.clone(), detail: Vec::new(), ..self.clone() };
            let link = crate::links::new_issue(
                &self.title,
                &format!(
                    "{}\n\n(cut short here: the whole report is on the clipboard)",
                    cut.description
                ),
            );
            if link.len() <= MOST_LINK_CHARS || description.is_empty() {
                return (link, true);
            }
            let keep = description.chars().count() * 3 / 4;
            description = description.chars().take(keep).collect();
        }
    }
}

/// Which OS build this is, for the reports where the platform is the story.
///
/// SindriCAD added this after a report it could not diagnose: the only platform
/// facts it collected were "linux" and "x86_64", neither of which says which
/// desktop or driver stack is installed.
///
/// **All three now, and the other two matter MORE than Linux.** This returned
/// an empty string off Linux and called that honest, which it was while Linux
/// was the only platform anyone shipped. It stopped being adequate the day
/// Windows and macOS builds went out: those are the two nobody here can
/// reproduce anything on, so a report from them arriving without an OS version
/// is a report that cannot be chased. macOS in particular is where the version
/// IS the question -- Sequoia changed what Gatekeeper does to an unsigned app,
/// and the Metal divergence in the masking tint may well be version specific.
///
/// Read from files, never by shelling out, so this cannot hang on a slow
/// process or hand a report an error message from a shell.
pub(crate) fn os_build() -> String {
    // Linux: the distribution, which names the desktop and driver stack.
    if cfg!(target_os = "linux")
        && let Ok(text) = std::fs::read_to_string("/etc/os-release")
    {
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                return value.trim_matches('"').to_string();
            }
        }
    }

    // macOS: the same plist `sw_vers` reads, scanned the same way `os-release`
    // is rather than parsed as XML. The two keys sit on the line after their
    // own `<key>`, which is the whole of the format worth knowing here.
    if cfg!(target_os = "macos")
        && let Ok(text) =
            std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
    {
        let value_after = |key: &str| -> Option<String> {
            let at = text.find(&format!("<key>{key}</key>"))?;
            let rest = &text[at..];
            let open = rest.find("<string>")? + "<string>".len();
            let close = rest[open..].find("</string>")?;
            Some(rest[open..open + close].to_string())
        };
        if let Some(version) = value_after("ProductVersion") {
            let build = value_after("ProductBuildVersion").unwrap_or_default();
            return if build.is_empty() {
                format!("macOS {version}")
            } else {
                format!("macOS {version} ({build})")
            };
        }
    }

    // Windows: `OS` and `OSVERSION` are not set, and the registry needs an API
    // call this crate has no other reason to make. The environment does carry
    // the processor identifier and the edition is not worth a dependency, so
    // this reports what it can rather than nothing -- an empty string here was
    // the actual gap.
    if cfg!(target_os = "windows") {
        let build = std::env::var("OS").unwrap_or_default();
        let arch = std::env::consts::ARCH;
        return if build.is_empty() {
            format!("Windows ({arch})")
        } else {
            format!("{build} ({arch})")
        };
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SindriCAD's own cases, brought over unchanged. Each of these is a shape
    /// that turned up in a real field report.
    #[test]
    fn user_paths_are_redacted_in_every_form_a_log_line_takes() {
        // Windows drive, and the exact shape a spawn line has.
        assert_eq!(
            redact_user_paths(r"spawn: C:\Users\jrabi\AppData\Local\HaniSculpt\app.exe"),
            r"spawn: C:\Users\[REDACTED]\AppData\Local\HaniSculpt\app.exe"
        );
        // UNC, where the HOST is not a username and has to survive.
        assert_eq!(
            redact_user_paths(r"\\HOST\Users\someone\share"),
            r"\\HOST\Users\[REDACTED]\share"
        );
        // Both unix forms, including a name that is not this machine's.
        assert_eq!(
            redact_user_paths("path /home/alice/proj/x.stl and /Users/bob/y"),
            "path /home/[REDACTED]/proj/x.stl and /Users/[REDACTED]/y"
        );
        // A segment ends at a quote, not only at a separator.
        assert_eq!(
            redact_user_paths(r#"log at "C:\Users\name" end"#),
            r#"log at "C:\Users\[REDACTED]" end"#
        );
        // Text with nothing to redact comes back byte identical.
        assert_eq!(redact_user_paths("no paths here"), "no paths here");
    }

    /// The case this application will actually hit: its own status line.
    #[test]
    fn the_status_line_this_application_writes_is_redacted() {
        assert_eq!(
            redact_user_paths("saved /home/maker/Downloads/sculpt2.hani"),
            "saved /home/[REDACTED]/Downloads/sculpt2.hani"
        );
    }

    #[test]
    fn sticky_facts_survive_a_busy_session_and_lead_the_trail() {
        // The regression this function exists for: a plain "keep the last
        // twenty" drops exactly the startup facts, and only once the session
        // has been busy enough to be worth reporting.
        let mut crumbs = vec![
            "[wgpu] AMD Radeon RX 6900 XT, Vulkan".to_string(),
            "[tablet] no tablet found".to_string(),
        ];
        for index in 0..60 {
            crumbs.push(format!("12:00:0{} noise {index}", index % 10));
        }

        let out = trim_breadcrumbs(&crumbs);
        assert!(out[0].starts_with("[wgpu]"), "sticky facts must lead: {out:?}");
        assert!(out.iter().any(|c| c.contains("Radeon")), "the adapter was dropped");
        assert!(out.iter().any(|c| c.contains("noise 59")), "the newest crumb was dropped");
        assert!(!out.iter().any(|c| c.contains("noise 0")), "an old crumb survived");
        assert_eq!(out.len(), 2 + MAX_TAIL_CRUMBS);
    }

    #[test]
    fn a_short_plain_trail_is_left_alone() {
        let crumbs: Vec<String> =
            (0..5).map(|index| format!("12:00:0{index} step {index}")).collect();
        assert_eq!(trim_breadcrumbs(&crumbs), crumbs);
    }

    #[test]
    fn sticky_facts_are_themselves_bounded() {
        let crumbs: Vec<String> = (0..100).map(|index| format!("[fact] {index}")).collect();
        assert_eq!(trim_breadcrumbs(&crumbs).len(), MAX_STICKY_CRUMBS);
    }

    #[test]
    fn a_title_is_cut_by_characters_and_not_by_bytes() {
        // Cutting by bytes would split a multi-byte character. Every one of
        // these is four bytes.
        let description = "🔨".repeat(200);
        let report = Report::new(&description, "", &[], "0.0.1", "");
        assert_eq!(report.title.chars().count(), TITLE_CHARS);
        assert!(report.title.chars().all(|c| c == '🔨'));
    }

    #[test]
    fn the_body_carries_the_diagnostics_folded_away_and_both_are_redacted() {
        let report = Report::new(
            "crashed after saving to /home/maker/x.hani",
            "last message: saved /home/maker/x.hani",
            &["12:00:01 saved".to_string()],
            "0.0.1",
            "Radeon (Vulkan)",
        );
        let body = report.body();
        assert!(!body.contains("/home/maker"), "a home directory reached the body");
        assert!(body.starts_with("crashed after saving to"), "{body}");
        assert!(body.contains("<details><summary>Diagnostics</summary>"));
        assert!(body.contains("last message:") && body.contains("graphics: Radeon"), "{body}");
        assert!(body.contains("session:\n  12:00:01 saved"), "{body}");
        assert!(body.trim_end().ends_with("</details>"));
        assert!(report.title.contains("[REDACTED]"), "the title is taken from redacted text");
        let without = Report::new("it went black", "", &[], "0.0.1", "Radeon");
        assert_eq!(without.body(), "it went black", "detail was attached when left out");
    }

    /// A report too long for a link keeps the description and the first
    /// lines of the detail, says it was cut, and still closes the block; and
    /// the link is one the application opens.
    #[test]
    fn a_long_report_is_cut_to_fit_a_link_from_the_end_of_the_detail() {
        let diagnostics: String =
            (0..2000).map(|n| format!("fact {n}: {}\n", "x".repeat(20))).collect();
        let report = Report::new("the brush stuck", &diagnostics, &[], "0.0.1", "");
        let (link, cut) = report.link();
        assert!(cut, "a report this long was not cut");
        assert!(link.len() <= MOST_LINK_CHARS, "{}", link.len());
        assert!(crate::links::may_be_opened(&link));
        assert!(link.contains("the%20brush%20stuck"));
        assert!(link.contains("fact%200%3A"), "the first facts went");
        assert!(!link.contains("fact%201999"), "the last facts stayed");
        assert!(link.contains("%3C%2Fdetails%3E"), "the block was left open");
        let (short, cut) = Report::new("short", "one fact", &[], "0.0.1", "").link();
        assert!(!cut && short.contains("one%20fact"));
        let (huge, cut) = Report::new(&"word ".repeat(5000), "", &[], "0.0.1", "").link();
        assert!(cut && huge.len() <= MOST_LINK_CHARS);
    }
}
