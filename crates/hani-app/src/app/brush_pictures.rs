// SPDX-License-Identifier: AGPL-3.0-only

//! The user's own brush pictures: kept, listed, and named by a brush.
//!
//! Asked for by the maintainer on 2026-11-27: "Brushes should support
//! custom bitmaps as part of the brush designer." A picture could be read
//! for the stamp or across the tip before this, but it was held only while
//! it was in hand: a brush kept with it said "whichever picture is in hand"
//! (`presets.rs`), and the next picture read took its place.
//!
//! Now a picture taken in is kept, as the file it was, in a directory of
//! its own under the data directory, as a map taken in is (`maps.rs`). The
//! picture picker lists every one of them after the six that come with the
//! application, and a brush kept with one names it, `stamp:<name>` or
//! `tip:<name>`, so taking the brush up takes its picture up.
//!
//! Its name is its file's, without the extension, as a map's is, and the
//! same fixed-size name type ([`MapName`]). A name that is the name of a
//! picture that comes with the application, or `picture`, which a brush
//! file uses for "the one in hand", is refused.
//!
//! In a browser there is no directory: a picture taken in is held for the
//! session, and a brush that names it finds it until the page is left.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use hani_core::brush::alpha::{Alpha, BuiltIn};

use super::maps::MapName;

/// The directory the pictures are kept in, under the data directory.
const DIRECTORY: &str = "brush-pictures";

/// What a picture's file may be called after its name.
const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// The biggest file read as a brush picture, in bytes.
const MOST_PICTURE_FILE: usize = 64 * 1024 * 1024;

/// What a brush file calls "the picture in hand". Kept in step with
/// `presets::IN_HAND` by a test.
pub(super) const IN_HAND: &str = "picture";

/// Where the pictures are kept, if anywhere is known.
pub(super) fn directory() -> Option<PathBuf> {
    crate::paths::data_file(DIRECTORY)
}

/// The pictures that have been kept, by name, in order: those in
/// `directory`, and in a browser those taken in this session.
pub(super) fn kept(directory: Option<&Path>) -> Vec<MapName> {
    let mut names: Vec<MapName> = directory
        .and_then(|directory| std::fs::read_dir(directory).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_picture(path))
        .filter_map(|path| MapName::new(&path.file_stem()?.to_string_lossy()))
        .collect();
    names.extend(pictures().keys().copied());
    names.sort_by_key(|name| name.as_str().to_lowercase());
    names.dedup();
    names
}

fn is_picture(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known))
    })
}

/// A picture file's bytes as a stamp image: its brightness is its height.
///
/// Sixteen bits of grey where the file has them, because eight is 256
/// steps and a stamp a centimetre tall would have terraces 40 microns
/// high, which a resin printer prints.
pub(super) fn decode(bytes: &[u8]) -> Result<Alpha, String> {
    let picture = image::load_from_memory(bytes).map_err(|error| error.to_string())?.into_luma16();
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    let heights: Vec<f32> =
        picture.as_raw().iter().map(|level| f32::from(*level) / f32::from(u16::MAX)).collect();
    Alpha::from_heights(width, height, &heights).map_err(|error| error.to_string())
}

/// The name a picture file is kept under: its file's name without the
/// extension, as a name a file can have, and not one a brush file already
/// means something else by.
pub(super) fn name_for(file_name: &str) -> Result<MapName, String> {
    let stem = Path::new(file_name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = MapName::new(&stem)
        .ok_or_else(|| "that file has no name a picture can be kept under".to_string())?;
    let taken = name.as_str().eq_ignore_ascii_case(IN_HAND)
        || BuiltIn::ALL.iter().any(|kind| kind.label().eq_ignore_ascii_case(name.as_str()));
    if taken {
        return Err(format!(
            "{} is the name of a picture that comes with the application",
            name.as_str()
        ));
    }
    Ok(name)
}

/// Take a picture in from its file's name and bytes: read it, to know it is
/// one, and keep its file in `directory` (one of that name already there
/// is replaced). With no directory it is held for the session.
pub(super) fn take_in(
    file_name: &str,
    bytes: &[u8],
    directory: Option<&Path>,
) -> Result<(MapName, Arc<Alpha>), String> {
    if bytes.len() > MOST_PICTURE_FILE {
        return Err(format!("{file_name} is too big to be a brush picture"));
    }
    let name = name_for(file_name)?;
    let alpha = Arc::new(decode(bytes).map_err(|why| format!("{file_name}: {why}"))?);
    if let Some(directory) = directory {
        std::fs::create_dir_all(directory)
            .map_err(|why| format!("{}: {why}", directory.display()))?;
        // One file a name: a picture kept before under this name as a
        // JPEG and now as a PNG is replaced, not kept twice.
        for extension in EXTENSIONS {
            let _ = std::fs::remove_file(directory.join(format!("{}.{extension}", name.as_str())));
        }
        let extension = Path::new(file_name)
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
            .filter(|extension| EXTENSIONS.contains(&extension.as_str()))
            .unwrap_or_else(|| "png".to_string());
        let file = directory.join(format!("{}.{extension}", name.as_str()));
        std::fs::write(&file, bytes).map_err(|why| format!("{}: {why}", file.display()))?;
    }
    pictures().insert(name, alpha.clone());
    Ok((name, alpha))
}

/// The pictures read so far, by name.
fn pictures() -> std::sync::MutexGuard<'static, HashMap<MapName, Arc<Alpha>>> {
    static PICTURES: LazyLock<Mutex<HashMap<MapName, Arc<Alpha>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    PICTURES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A kept picture by name, read once.
pub(super) fn alpha(name: MapName) -> Result<Arc<Alpha>, String> {
    alpha_in(name, directory().as_deref())
}

/// [`alpha`], from a directory of the caller's choosing.
pub(super) fn alpha_in(name: MapName, directory: Option<&Path>) -> Result<Arc<Alpha>, String> {
    if let Some(held) = pictures().get(&name) {
        return Ok(held.clone());
    }
    let file = directory
        .into_iter()
        .flat_map(|directory| {
            EXTENSIONS.map(|extension| directory.join(format!("{}.{extension}", name.as_str())))
        })
        .find(|file| file.is_file())
        .ok_or_else(|| format!("there is no picture called {}", name.as_str()))?;
    let bytes = std::fs::read(&file).map_err(|why| format!("{}: {why}", file.display()))?;
    let alpha = Arc::new(decode(&bytes).map_err(|why| format!("{}: {why}", name.as_str()))?);
    pictures().insert(name, alpha.clone());
    Ok(alpha)
}

/// Forget the pictures read from files: they may have been changed by
/// hand. In a browser, where there are no files, they are all there is,
/// and are kept.
pub(super) fn forget_kept() {
    if directory().is_some() {
        pictures().clear();
    }
}

/// Ask for a picture and take it in. `None` when the dialog is dismissed.
pub(super) async fn pick() -> Option<Result<(MapName, Arc<Alpha>), String>> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Brush picture")
        .add_filter("Picture", &EXTENSIONS)
        .pick_file()
        .await?;
    let bytes = handle.read().await;
    Some(take_in(&handle.file_name(), &bytes, directory().as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir()
                .join(format!("hani-brush-pictures-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&directory);
            Self(directory)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A picture file: white in its top right quarter, black elsewhere.
    pub(crate) fn png() -> Vec<u8> {
        let picture = image::ImageBuffer::from_fn(32, 32, |x, y| {
            image::Luma([if x >= 16 && y < 16 { u16::MAX } else { 0u16 }])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageLuma16(picture)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("a png writes");
        bytes
    }

    #[test]
    fn a_picture_taken_in_is_kept_listed_and_found_again() {
        let scratch = Scratch::new("kept");
        let (name, alpha) =
            take_in("Brick wall.png", &png(), Some(&scratch.0)).expect("a PNG is a brush picture");
        assert_eq!(name.as_str(), "Brick wall");
        assert!(alpha.sample(0.5, 0.5) > 0.9, "the white quarter is not top right");
        assert!(scratch.0.join("Brick wall.png").is_file(), "the file was not kept");
        assert_eq!(kept(Some(&scratch.0)).iter().filter(|kept| **kept == name).count(), 1);

        // Read again from its file, as on the next start.
        pictures().remove(&name);
        let again = alpha_in(name, Some(&scratch.0)).expect("the kept picture reads");
        assert_eq!(*again, *alpha);

        // Taken in again as a JPEG of the same name, it is one file.
        let jpeg = {
            let picture = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
                8,
                8,
                image::Luma([200]),
            ));
            let mut bytes = Vec::new();
            picture
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
                .unwrap();
            bytes
        };
        take_in("Brick wall.jpg", &jpeg, Some(&scratch.0)).expect("a JPEG is a brush picture");
        assert!(!scratch.0.join("Brick wall.png").exists(), "the old file was kept beside it");
        assert!(scratch.0.join("Brick wall.jpg").is_file());
        pictures().remove(&name);
    }

    #[test]
    fn names_a_brush_file_means_something_else_by_are_refused() {
        for file in ["Rivet.png", "picture.png", "PICTURE.jpg", "!!!.png"] {
            assert!(name_for(file).is_err(), "{file} was taken");
        }
        assert_eq!(name_for("my wood grain.jpeg").unwrap().as_str(), "my wood grain");
        assert_eq!(IN_HAND, super::super::presets::IN_HAND);
    }

    #[test]
    fn what_is_not_a_picture_is_refused_and_not_kept() {
        let scratch = Scratch::new("refused");
        let refused = take_in("notes.png", b"this is not a picture", Some(&scratch.0));
        assert!(refused.is_err());
        assert!(!scratch.0.join("notes.png").exists());
        assert!(alpha_in(MapName::new("nothing here").unwrap(), Some(&scratch.0)).is_err());
    }
}
