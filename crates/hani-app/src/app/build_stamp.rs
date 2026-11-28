// SPDX-License-Identifier: AGPL-3.0-only

//! The build's identity: the commit it was made from, and its ordinal when
//! the release pipeline stamped one. Moved out of `app.rs` (issue 189).

/// The commit this was built from, when the build system supplied one.
///
/// Shown in About, and not only as a nicety: shipping binaries carries an AGPL
/// obligation to offer the corresponding source, and a commit is what ties an
/// artifact to it. `unknown` in a local build, which is honest.
pub fn build_commit() -> &'static str {
    option_env!("HANI_COMMIT").unwrap_or("unknown")
}

/// What the Help panel and the About line call this build.
///
/// `build 1023 (9535407)` for a release build; `build 9535407` for anything
/// else, because an unstamped build has no ordinal and inventing one would be
/// worse than omitting it.
pub fn build_label() -> String {
    match build_number() {
        Some(build) => format!("build {build} ({})", build_commit()),
        None => format!("build {}", build_commit()),
    }
}

/// The build ordinal, if this binary came from the release pipeline.
///
/// **A total order over builds, which [`build_commit`] cannot provide.** A git
/// SHA orders nothing, so a commit stamp can only answer "is the published
/// build different from mine". This answers "which is newer", which is what an
/// updater needs before it will replace anything.
///
/// `None` on every local build, every source build, and any build whose stamp
/// failed to parse. That is the load-bearing case rather than the fallback:
/// `None` is what makes the updater structurally inert on a developer's
/// machine, so a stray keypress cannot overwrite the binary they are working
/// on with a CI artefact. `release.yml` sets `HANI_BUILD` to
/// `1000 + github.run_number` and then reads this value back out of the binary
/// it just built, because the way this goes wrong -- a cached build script
/// serving the previous run's ordinal -- is otherwise silent.
///
/// Deliberately NOT saturating or defaulting on a bad parse: a stamp that says
/// `banana` must not compare as build zero, which would sort below every real
/// release for ever.
pub fn build_number() -> Option<u64> {
    option_env!("HANI_BUILD")?.trim().parse().ok()
}
