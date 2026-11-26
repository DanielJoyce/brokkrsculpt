// SPDX-License-Identifier: AGPL-3.0-only

//! The one link the application opens in a browser, and the rule that
//! decides which links it may open.
//!
//! This was `articles.rs`, which also fetched TinkerAtlas's articles for the
//! welcome screen and carried its Join and Visit buttons. Those went on
//! 2026-11-26 at the maintainer's word: *"Remove the links to the external
//! website."* What is left is the release page the updater's button leads
//! to, the repository, which the welcome screen and Help link to (asked for
//! on 2026-11-26), and the guard every link goes through.

/// Where a user is sent to get a build.
///
/// Here rather than in the updater for the reason above: this is one of the
/// prefixes [`may_be_opened`] admits, and a URL that lives apart from the rule
/// admitting it is a URL that will one day stop being admitted. It was in
/// `update_check.rs` until the updater replaced that module, and moving it was
/// not optional -- the allowlist and its test both read it, so deleting the old
/// module without moving this first does not compile.
pub const RELEASE_PAGE: &str = "https://github.com/DanielJoyce/hanisculpt/releases/tag/beta";

/// The project's own repository, linked from the welcome screen and from
/// Help.
pub const REPOSITORY: &str = "https://github.com/DanielJoyce/hanisculpt";

/// GitHub's page for a new issue on it, which Snip and report opens with a
/// title and a body in its query (`app/snip.rs`).
pub const NEW_ISSUE: &str = "https://github.com/DanielJoyce/hanisculpt/issues/new";

/// Whether a link may be handed to the user's browser.
///
/// **Split out of [`open_in_browser`] so acceptance can be tested.** That one
/// spawns a process the moment it approves, so a test proving a URL is allowed
/// would open a browser on whoever ran the suite -- which is why the existing
/// test only ever checks refusals. This is the same question with no side
/// effect.
///
/// # The rule, which is an invariant and not a heuristic
///
/// A link is admitted when it is exactly [`RELEASE_PAGE`] or
/// [`REPOSITORY`], and nothing else is. The welcome screen's links to TinkerAtlas, its articles and its
/// sign-in were taken out on 2026-11-26 at the maintainer's word (*"Remove
/// the links to the external website"*), and the prefix that admitted that
/// site went with them. A link added later goes in as an exact page, or as a
/// prefix ending in `/`, which is what stops `https://site.com` admitting
/// `https://site.com.example.net/`.
pub(crate) fn may_be_opened(link: &str) -> bool {
    link == RELEASE_PAGE
        || link == REPOSITORY
        || link.strip_prefix(NEW_ISSUE).is_some_and(|query| query.starts_with('?'))
}

/// Hand a link to whatever the desktop opens links with.
///
/// The platform's own opener rather than a crate: `slicer.rs` already
/// spawns a process the same way, and a browser-opening dependency would be a
/// second one to audit for the sake of one command.
pub fn open_in_browser(link: &str) -> Result<(), String> {
    if !may_be_opened(link) {
        return Err("that link is not one this application opens".to_string());
    }
    // In a browser, a new tab of it. See `web.rs`.
    #[cfg(target_arch = "wasm32")]
    return crate::web::open_tab(link);
    #[cfg(not(target_arch = "wasm32"))]
    open_on_desktop(link)
}

#[cfg(not(target_arch = "wasm32"))]
fn open_on_desktop(link: &str) -> Result<(), String> {
    // **One of these three, and no shell.** `xdg-open` exists only on
    // Linux; macOS has `open`; Windows has no single-word equivalent, and
    // `cmd /C start` would put a SHELL between us and a URL.
    // `rundll32 url.dll,FileProtocolHandler` is what `start` ends up calling
    // anyway, and it takes the argument directly.
    let (program, leading) = if cfg!(target_os = "macos") {
        ("open", None)
    } else if cfg!(target_os = "windows") {
        ("rundll32.exe", Some("url.dll,FileProtocolHandler"))
    } else {
        ("xdg-open", None)
    };
    let mut command = std::process::Command::new(program);
    if let Some(first) = leading {
        command.arg(first);
    }
    command
        .arg(link)
        .spawn()
        .map(|_| ())
        .map_err(|why| format!("nothing would open the link ({why})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The update button's page and the repository are admitted, and
    /// nothing else is: not the rest of GitHub, and not the site the welcome
    /// screen used to link to.
    #[test]
    fn only_the_release_page_and_the_repository_may_be_opened() {
        assert!(may_be_opened(RELEASE_PAGE), "the update button would be refused: {RELEASE_PAGE}");
        assert!(may_be_opened(REPOSITORY), "the repository link would be refused");
        assert!(!may_be_opened("https://github.com/DanielJoyce/hanisculpt.example.net"));
        assert!(!may_be_opened("https://github.com/DanielJoyce/hanisculpt/../other"));
        assert!(may_be_opened(&format!("{NEW_ISSUE}?title=a&body=b")));
        assert!(!may_be_opened(&format!("{NEW_ISSUE}s/../../x")));
        assert!(!may_be_opened("https://github.com/someone/else/releases/tag/beta"));
        assert!(!may_be_opened("https://tinkeratlas.com/"));
        assert!(open_in_browser("https://evil.example/phish").is_err());
    }
}
