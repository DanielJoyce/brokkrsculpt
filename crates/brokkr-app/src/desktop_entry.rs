// SPDX-License-Identifier: AGPL-3.0-only

//! Haniko's badge as the launcher and window icon on a Linux desktop, with
//! nothing installed by hand.
//!
//! On Wayland a window cannot hand its compositor an icon. The compositor
//! takes the window's app id, here `hanisculpt`, finds the `.desktop` file of
//! that name in the data directories, and shows the icon it names from the
//! icon theme. A build run from `target/` or unpacked from an archive has
//! neither, so it showed the compositor's default icon: the maintainer saw
//! exactly that on 2026-11-26.
//!
//! So at startup this writes, for the current user only:
//!
//! * `$XDG_DATA_HOME/icons/hicolor/{128x128,256x256}/apps/hanisculpt.png`, the
//!   badge;
//! * `$XDG_DATA_HOME/applications/hanisculpt.desktop`, whose `Exec` is this
//!   binary, wherever it is.
//!
//! Each is written only when it is missing or differs, so a launch normally
//! writes nothing. A `.desktop` file that this did not write (one a package or
//! the user installed) carries no `X-HaniSculpt-Written` line and is left
//! alone. Failures are logged and otherwise ignored: an icon is not worth a
//! refusal to start.
//!
//! The first launch after it is written may still show the default icon,
//! until the desktop notices the new file; every launch after shows Haniko.

use std::path::{Path, PathBuf};

/// The window's app id, which is also the name the desktop file and the
/// icon go by. `main.rs` gives it to the window.
pub const APP_ID: &str = "hanisculpt";

/// The line that marks a desktop file as this module's to rewrite.
const WRITTEN: &str = "X-HaniSculpt-Written=true";

/// The desktop file for a binary at `exe`.
fn entry_text(exe: &Path) -> String {
    // Quoted, with the characters the Desktop Entry spec reserves inside
    // quotes escaped, so a path with a space in it still launches.
    let mut quoted = String::from("\"");
    for character in exe.to_string_lossy().chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=HaniSculpt\n\
         GenericName=3D Sculpting\n\
         Comment=Voxel sculpting for 3D printing\n\
         Exec={quoted}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Graphics;3DGraphics;Engineering;\n\
         Keywords=sculpt;voxel;3D;print;mesh;STL;3MF;\n\
         StartupWMClass={APP_ID}\n\
         {WRITTEN}\n"
    )
}

/// Write `bytes` to `path` unless it already holds them. Whether it wrote.
fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    if std::fs::read(path).is_ok_and(|held| held == bytes) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(true)
}

/// Put the icons and the desktop file under `data_home` for a binary at
/// `exe`. Returns the paths it wrote.
fn install_into(data_home: &Path, exe: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut wrote = Vec::new();
    for (size, png) in [(128, crate::logo::BADGE), (256, crate::logo::ICON)] {
        let path = data_home
            .join("icons/hicolor")
            .join(format!("{size}x{size}"))
            .join("apps")
            .join(format!("{APP_ID}.png"));
        if write_if_changed(&path, png)? {
            wrote.push(path);
        }
    }
    let entry = data_home.join("applications").join(format!("{APP_ID}.desktop"));
    let someone_elses =
        std::fs::read_to_string(&entry).is_ok_and(|held| !held.lines().any(|line| line == WRITTEN));
    if !someone_elses && write_if_changed(&entry, entry_text(exe).as_bytes())? {
        wrote.push(entry);
    }
    Ok(wrote)
}

/// Install for the current user, at startup. Linux only; a no-op without a
/// data directory or a path to this binary.
pub fn install() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let (Some(data_home), Ok(exe)) = (crate::paths::xdg_data_home(), std::env::current_exe())
    else {
        return;
    };
    match install_into(&data_home, &exe) {
        Ok(wrote) if !wrote.is_empty() => {
            log::info!("installed the desktop entry and icon: {wrote:?}");
        }
        Ok(_) => {}
        Err(why) => log::warn!("could not install the desktop entry: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("hanisculpt-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn the_icon_and_entry_go_in_once_and_then_are_left() {
        let home = scratch("once");
        let exe = Path::new("/opt/Hani Sculpt/brokkrsculpt");
        let first = install_into(&home, exe).expect("writable");
        assert_eq!(first.len(), 3, "two icons and the entry: {first:?}");
        let entry = std::fs::read_to_string(home.join("applications/hanisculpt.desktop")).unwrap();
        assert!(entry.contains("Exec=\"/opt/Hani Sculpt/brokkrsculpt\""), "{entry}");
        assert!(entry.contains("Icon=hanisculpt\n"));
        assert!(entry.contains("StartupWMClass=hanisculpt\n"));
        let icon = std::fs::read(home.join("icons/hicolor/256x256/apps/hanisculpt.png")).unwrap();
        assert_eq!(icon, crate::logo::ICON);
        assert!(
            install_into(&home, exe).expect("writable").is_empty(),
            "a second launch writes nothing"
        );
        // A move of the binary rewrites the entry and nothing else.
        let moved = install_into(&home, Path::new("/usr/bin/brokkrsculpt")).expect("writable");
        assert_eq!(moved, vec![home.join("applications/hanisculpt.desktop")]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_entry_someone_else_installed_is_left_alone() {
        let home = scratch("theirs");
        let entry = home.join("applications/hanisculpt.desktop");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, "[Desktop Entry]\nName=Packaged\n").unwrap();
        install_into(&home, Path::new("/tmp/brokkrsculpt")).expect("writable");
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), "[Desktop Entry]\nName=Packaged\n");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_quote_or_a_dollar_in_the_path_is_escaped() {
        let text = entry_text(Path::new("/a\"b$c/x"));
        assert!(text.contains("Exec=\"/a\\\"b\\$c/x\"\n"), "{text}");
    }
}
