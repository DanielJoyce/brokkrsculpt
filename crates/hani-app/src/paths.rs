// SPDX-License-Identifier: AGPL-3.0-only

//! Where the application keeps its own files.
//!
//! Three kinds of file, in three places, and the operating system is asked
//! where each belongs through the `dir_spec` crate:
//!
//! | | What | Linux | macOS | Windows |
//! |---|---|---|---|---|
//! | **Settings** | Chosen once and expected to survive a reinstall: the printer, the slicer, the puck's buttons | `~/.config` | `~/Library/Application Support` | `%APPDATA%` |
//! | **Data** | Made by the user and worth sharing: brushes | `~/.local/share` | `~/Library/Application Support` | `%APPDATA%` |
//! | **State** | Recovered from, not set: the autosave, the recent files | `~/.local/state` | `~/Library/Application Support` | `%LOCALAPPDATA%` |
//!
//! each under a directory named for the application. `XDG_CONFIG_HOME`,
//! `XDG_DATA_HOME` and `XDG_STATE_HOME` move them, on every platform, when
//! they are set and absolute. A relative value is ignored: it would resolve
//! against whatever directory the application happened to be started from,
//! which is not the same from one session to the next and is exactly when
//! these files are read.
//!
//! This module did all of that by hand before it asked `dir_spec`, and to
//! the same answers on Linux and Windows. What changed on macOS is that the
//! three variables are honoured there too.
//!
//! # The one thing the crate is not asked
//!
//! Settings and state must be different directories: an autosave sitting
//! beside the settings reads as a document the user chose to keep. On macOS
//! the platform's answer to all three questions is the same directory, so
//! state and data each go in a directory of their own inside it there, and
//! anywhere else an environment makes two of them the same.

// In a browser there are no folders: every directory is `None`, so a
// setting is not kept and a file is not looked for, the same as a machine
// with no home directory. Shadows the crate, which is native only. See
// `docs/WEB.md`.
#[cfg(target_arch = "wasm32")]
mod dir_spec {
    use std::path::PathBuf;
    pub fn config_home() -> Option<PathBuf> {
        None
    }
    pub fn state_home() -> Option<PathBuf> {
        None
    }
    pub fn data_home() -> Option<PathBuf> {
        None
    }
}

use std::path::PathBuf;

/// Everything the application writes lives under a directory of this name.
const APPLICATION: &str = "hanisculpt";

/// Where settings live. See the table at the top of this module.
fn config_directory() -> Option<PathBuf> {
    Some(dir_spec::config_home()?.join(APPLICATION))
}

/// The user's data directory itself (`$XDG_DATA_HOME`, `~/.local/share`),
/// not the application's folder in it: where desktop entries and icons go.
/// See `desktop_entry.rs`.
pub fn xdg_data_home() -> Option<PathBuf> {
    dir_spec::data_home()
}

/// What [`APPLICATION`] was before the name was HaniSculpt.
const LEGACY_APPLICATION: &str = "brokkrsculpt";

/// Move the settings, brushes and autosave a BrokkrSculpt build kept into
/// HaniSculpt's folders, once: each old folder is renamed to the new name
/// when the new one does not exist yet, and an autosave left under the old
/// extension takes the new one. Called at startup before anything reads a
/// setting. A failure is logged and leaves the old folder where it was.
pub fn adopt_legacy_folders() {
    for base in [dir_spec::config_home(), dir_spec::state_home(), dir_spec::data_home()]
        .into_iter()
        .flatten()
    {
        adopt_in(&base);
    }
    if let (Some(old), Some(new)) = (state_file("autosave.brokkr"), state_file("autosave.hani"))
        && old.exists()
        && !new.exists()
        && let Err(why) = std::fs::rename(&old, &new)
    {
        log::warn!("could not rename {}: {why}", old.display());
    }
}

/// The folder move under one base directory. Whether it moved one.
fn adopt_in(base: &std::path::Path) -> bool {
    let old = base.join(LEGACY_APPLICATION);
    let new = base.join(APPLICATION);
    if !old.is_dir() || new.exists() {
        return false;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => true,
        Err(why) => {
            log::warn!("could not move {} to {}: {why}", old.display(), new.display());
            false
        }
    }
}

/// `base`, in the application's directory, and in `apart` inside that when
/// it would otherwise be the settings' own directory.
fn apart_from_config(base: Option<PathBuf>, apart: &str) -> Option<PathBuf> {
    let directory = base?.join(APPLICATION);
    if Some(&directory) == config_directory().as_ref() {
        return Some(directory.join(apart));
    }
    Some(directory)
}

/// Where recoverable state lives.
///
/// **Never the same directory as [`config_directory`].** That invariant is
/// what `the_three_kinds_are_three_places` pins.
fn state_directory() -> Option<PathBuf> {
    apart_from_config(dir_spec::state_home(), "State")
}

/// Where what the user made lives: brushes, today.
fn data_directory() -> Option<PathBuf> {
    apart_from_config(dir_spec::data_home(), "Data")
}

/// A settings file under [`config_directory`].
///
/// For things the user chose and would expect to survive a reinstall.
pub fn config_file(name: &str) -> Option<PathBuf> {
    Some(config_directory()?.join(name))
}

/// A file or a directory of the user's own making under
/// [`data_directory`].
///
/// For things worth copying to another machine or handing to someone else:
/// a brush is the first of them.
pub fn data_file(name: &str) -> Option<PathBuf> {
    Some(data_directory()?.join(name))
}

/// A state file under [`state_directory`].
///
/// For things the application recovers from rather than things the user set.
/// The autosave lives here and not in the config directory precisely so it is
/// never mistaken for a document.
pub fn state_file(name: &str) -> Option<PathBuf> {
    Some(state_directory()?.join(name))
}

/// The `key = value` pairs in one of this application's flat config files.
///
/// Blank lines and `#` comments are skipped, a line with no `=` is skipped, and
/// both halves are trimmed. What a key MEANS, and what to do about one that is
/// unknown or unparseable, is the caller's business and differs per file --
/// `printer.rs` falls back to a default port, `spacemouse.rs` leaves the
/// binding alone, `welcome.rs` treats anything but `false` as yes.
///
/// **Extracted at the third copy and not before.** `printer.rs` and
/// `spacemouse.rs` each grew this loop independently and each documented that
/// it matched the other; `welcome.rs` made three, which is where duplication
/// stops being cheaper than an abstraction. Only the scanning is shared: a
/// "config file" type that also owned the defaults would have to know all three
/// schemas, which is the wrong abstraction rather than a missing one.
pub fn entries(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim()))
    })
}

/// An absolute path, spelled the way the platform running the tests spells one.
///
/// **`/thing` is not absolute on Windows.** It has no drive letter, so
/// `is_absolute` is false and every fixture that builds a path that way is
/// quietly testing the rejection branch instead of the one it meant. Ten tests
/// across three modules failed exactly that way the first time this crate was
/// built for Windows.
#[cfg(test)]
pub(crate) fn absolute(rest: &str) -> PathBuf {
    if cfg!(target_os = "windows") {
        PathBuf::from(format!(r"C:\{}", rest.replace('/', r"\")))
    } else {
        PathBuf::from(format!("/{rest}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check a hand-rolled copy is most likely to drop, and the one
    /// the crate is trusted with now. A relative value resolves against
    /// the working directory, which is not stable across sessions.
    #[test]
    fn a_relative_environment_value_is_ignored_in_favour_of_home() {
        // Asserted through the predicate rather than by setting the variable,
        // because the environment is process-wide and the test harness runs
        // these in parallel.
        assert!(!PathBuf::from("relative/path").is_absolute());
        assert!(absolute("absolute/path").is_absolute());
        // And of the crate itself, whatever this machine's environment is:
        // what it hands back is never relative.
        for directory in [dir_spec::config_home(), dir_spec::data_home(), dir_spec::state_home()]
            .into_iter()
            .flatten()
        {
            assert!(directory.is_absolute(), "{} is not absolute", directory.display());
        }
    }

    #[test]
    fn every_kind_lands_under_the_application_directory() {
        for built in [config_file("thing"), data_file("thing"), state_file("thing")] {
            let Some(path) = built else {
                continue;
            };
            assert!(
                path.to_string_lossy().contains(APPLICATION),
                "{} is not under the application directory",
                path.display()
            );
            assert!(path.ends_with("thing"));
            assert!(path.is_absolute(), "{} is not absolute", path.display());
        }
    }

    /// Settings, data and state must not collide: an autosave sitting next
    /// to the settings would read as a document the user chose to keep,
    /// and a brush beside them as a setting.
    #[test]
    fn the_three_kinds_are_three_places() {
        let (config, data, state) = (config_file("x"), data_file("x"), state_file("x"));
        if let (Some(config), Some(state)) = (&config, &state) {
            assert_ne!(config, state);
        }
        if let (Some(config), Some(data)) = (&config, &data) {
            assert_ne!(config, data);
        }
        if let (Some(data), Some(state)) = (&data, &state) {
            assert_ne!(data, state);
        }
    }

    /// Where two kinds would be one directory, the second gets a directory
    /// of its own inside it: macOS, where the platform has one answer for
    /// all three, and any environment that points two variables at one
    /// place.
    #[test]
    fn a_kind_that_would_share_the_settings_directory_is_put_inside_it() {
        let Some(config) = config_directory() else {
            return;
        };
        let shared = config.parent().map(PathBuf::from);
        assert_eq!(apart_from_config(shared, "State"), Some(config.join("State")));
        let elsewhere = absolute("somewhere/else");
        assert_eq!(
            apart_from_config(Some(elsewhere.clone()), "State"),
            Some(elsewhere.join(APPLICATION))
        );
        assert_eq!(apart_from_config(None, "State"), None);
    }

    /// On Linux, where this was developed and the paths are known: the
    /// directories the specification names, when the environment names no
    /// others.
    #[test]
    #[cfg(target_os = "linux")]
    fn on_linux_they_are_where_the_specification_says() {
        let unset = |key: &str| std::env::var_os(key).is_none();
        let Some(home) = dir_spec::home() else {
            return;
        };
        if unset("XDG_CONFIG_HOME") {
            assert_eq!(config_file("x"), Some(home.join(".config/hanisculpt/x")));
        }
        if unset("XDG_DATA_HOME") {
            assert_eq!(data_file("x"), Some(home.join(".local/share/hanisculpt/x")));
        }
        if unset("XDG_STATE_HOME") {
            assert_eq!(state_file("x"), Some(home.join(".local/state/hanisculpt/x")));
        }
    }

    /// A BrokkrSculpt folder is taken over once, and a HaniSculpt folder
    /// that already exists is never overwritten.
    #[test]
    fn the_old_folder_moves_once_and_never_over_a_new_one() {
        let base = std::env::temp_dir().join(format!("hani-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(LEGACY_APPLICATION)).unwrap();
        std::fs::write(base.join(LEGACY_APPLICATION).join("settings.conf"), "a = 1\n").unwrap();
        assert!(adopt_in(&base));
        assert!(base.join(APPLICATION).join("settings.conf").exists());
        assert!(!base.join(LEGACY_APPLICATION).exists());
        std::fs::create_dir_all(base.join(LEGACY_APPLICATION)).unwrap();
        assert!(!adopt_in(&base), "a second old folder does not replace the new one");
        assert!(base.join(APPLICATION).join("settings.conf").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
