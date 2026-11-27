// SPDX-License-Identifier: AGPL-3.0-only

//! Vector displacement maps: read from files, kept, and made shapes.
//!
//! `docs/STUDIO-PLAN.md` section 18, stage C10. The core knows what a map
//! is and how one is set down ([`hani_core::solid::VectorMap`],
//! `solid/relief.rs`), and nothing about files. This module is the rest:
//!
//! * **Reading one.** The files other programs write are OpenEXR, three
//!   32-bit numbers a pixel ([`read_file`]). Red is the displacement
//!   across the square, green up it, and blue out of it, each in
//!   half-widths of the square: what Blender's vector displacement
//!   brushes are baked as.
//! * **Keeping them.** A map taken in is copied into the `maps`
//!   directory of the data directory, beside `brushes`, under a name of
//!   no more than [`LONGEST_NAME`] bytes ([`take_in`]). A brush names its
//!   map by that name, so a brush file and its map can both be handed to
//!   somebody else.
//! * **Making them shapes.** A map is made a shape once, when it is first
//!   wanted, and the shape is kept for as long as the application runs
//!   ([`relief`]): it takes a fifth of a second in the build the tests
//!   run in (measured: `what_a_stamp_costs` in the core).

// A picked file has no path in a browser. See `web.rs`.
#[cfg(target_arch = "wasm32")]
use crate::web::NoPath;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use hani_core::solid::{BuiltInMap, MAX_MAP_SIZE, Relief, VectorMap};

/// The directory of the data directory that maps are kept in.
const DIRECTORY: &str = "maps";

/// What a map's file is called after its name.
const EXTENSION: &str = "exr";

/// The longest a map's name is, in bytes.
pub(super) const LONGEST_NAME: usize = 32;

/// The biggest file read as a map, in bytes. A map of 4096 pixels a side
/// with nothing squeezed out of it is 201 megabytes (arithmetic: three
/// 4-byte numbers a pixel), and bigger than that is not a brush.
const MOST_MAP_FILE: u64 = 512 * 1024 * 1024;

/// The name of a map that was taken in: its file's name, without the
/// extension. A fixed size, so that what names a map can be copied about
/// as freely as the name of a shape.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapName {
    bytes: [u8; LONGEST_NAME],
    length: u8,
}

impl MapName {
    /// `text` as a name a file can have: its letters, digits, spaces,
    /// dashes and underscores, and no more of them than fit. `None` when
    /// nothing is left.
    pub(super) fn new(text: &str) -> Option<Self> {
        let mut bytes = [0; LONGEST_NAME];
        let mut length = 0;
        let kept = text
            .trim()
            .chars()
            .filter(|letter| letter.is_alphanumeric() || matches!(letter, ' ' | '-' | '_'));
        for letter in kept {
            if length + letter.len_utf8() > LONGEST_NAME {
                break;
            }
            letter.encode_utf8(&mut bytes[length..]);
            length += letter.len_utf8();
        }
        (length > 0).then_some(Self { bytes, length: length as u8 })
    }

    pub(super) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.length as usize]).unwrap_or("map")
    }
}

impl std::fmt::Debug for MapName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapName({:?})", self.as_str())
    }
}

/// Which map: one that comes with the application, or one taken in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapOf {
    BuiltIn(BuiltInMap),
    Named(MapName),
}

impl MapOf {
    pub(super) fn label(&self) -> &str {
        match self {
            MapOf::BuiltIn(kind) => kind.label(),
            MapOf::Named(name) => name.as_str(),
        }
    }

    /// The map a brush file's `map:` names: one that comes with the
    /// application if it is called that, and one taken in if not.
    pub(super) fn named(text: &str) -> Option<Self> {
        BuiltInMap::ALL
            .into_iter()
            .find(|kind| kind.label().eq_ignore_ascii_case(text.trim()))
            .map(MapOf::BuiltIn)
            .or_else(|| MapName::new(text).map(MapOf::Named))
    }
}

/// Where maps are kept, if anywhere is known.
pub(super) fn directory() -> Option<PathBuf> {
    crate::paths::data_file(DIRECTORY)
}

/// The maps that have been taken in, by name, in order.
pub(super) fn kept(directory: Option<&Path>) -> Vec<MapName> {
    let Some(entries) = directory.and_then(|directory| std::fs::read_dir(directory).ok()) else {
        return Vec::new();
    };
    let mut names: Vec<MapName> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION))
        })
        .filter_map(|path| MapName::new(&path.file_stem()?.to_string_lossy()))
        .collect();
    names.sort_by_key(|name| name.as_str().to_lowercase());
    names.dedup();
    names
}

/// Read a map from an OpenEXR file: its first layer with red, green and
/// blue in it, at no more than [`MAX_MAP_SIZE`] pixels a side (a bigger
/// one is read every so many pixels and not all of them).
pub(super) fn read_file(path: &Path) -> Result<VectorMap, String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a file".to_string());
    let size = std::fs::metadata(path).map_err(|why| format!("{name}: {why}"))?.len();
    if size > MOST_MAP_FILE {
        return Err(format!("{name} is too big to be a brush's map: {size} bytes"));
    }
    let bytes = std::fs::read(path).map_err(|why| format!("{name}: {why}"))?;
    read_bytes(&bytes).map_err(|why| format!("{name}: {why}"))
}

pub(super) fn read_bytes(bytes: &[u8]) -> Result<VectorMap, String> {
    // Our own reader (`openexr.rs`), then every `step`th pixel each way, so
    // a map is at most `MAX_MAP_SIZE` across.
    let picture = crate::openexr::read(bytes)
        .map_err(|why| format!("not an OpenEXR picture with red, green and blue in it ({why})"))?;
    let step = picture.width.max(picture.height).div_ceil(MAX_MAP_SIZE).max(1);
    let (width, height) = (picture.width.div_ceil(step), picture.height.div_ceil(step));
    let mut vectors = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            vectors.push(picture.pixels[y * step * picture.width + x * step]);
        }
    }
    VectorMap::from_vectors(width, height, &vectors).map_err(|why| why.to_string())
}

/// Take a map in: read it, to know that it is one, and copy its file
/// into `directory` under a name that will do. One of that name already
/// there is replaced.
pub(super) fn take_in(path: &Path, directory: &Path) -> Result<MapName, String> {
    let name = path
        .file_stem()
        .and_then(|stem| MapName::new(&stem.to_string_lossy()))
        .ok_or_else(|| "that file has no name a map can be kept under".to_string())?;
    if BuiltInMap::ALL.iter().any(|kind| kind.label().eq_ignore_ascii_case(name.as_str())) {
        return Err(format!(
            "{} is the name of a map that comes with the application",
            name.as_str()
        ));
    }
    let map = read_file(path)?;
    let shape = Relief::of(&map).map_err(|why| format!("{}: {why}", name.as_str()))?;
    std::fs::create_dir_all(directory).map_err(|why| format!("{}: {why}", directory.display()))?;
    let kept = file_of(directory, name);
    if kept != path {
        std::fs::copy(path, &kept).map_err(|why| format!("{}: {why}", kept.display()))?;
    }
    shapes().insert(MapOf::Named(name), Arc::new(shape));
    Ok(name)
}

fn file_of(directory: &Path, name: MapName) -> PathBuf {
    directory.join(format!("{}.{EXTENSION}", name.as_str()))
}

/// The shapes made so far.
fn shapes() -> std::sync::MutexGuard<'static, HashMap<MapOf, Arc<Relief>>> {
    static SHAPES: LazyLock<Mutex<HashMap<MapOf, Arc<Relief>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    SHAPES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The shape of a map, made the first time it is asked for. A map that
/// was taken in is read from the maps directory.
pub(super) fn relief(map: MapOf) -> Result<Arc<Relief>, String> {
    relief_in(map, directory().as_deref())
}

pub(super) fn relief_in(map: MapOf, directory: Option<&Path>) -> Result<Arc<Relief>, String> {
    if let Some(shape) = shapes().get(&map) {
        return Ok(shape.clone());
    }
    let read = match map {
        MapOf::BuiltIn(kind) => kind.map(),
        MapOf::Named(name) => {
            let directory =
                directory.ok_or_else(|| "there is nowhere maps are kept".to_string())?;
            let path = file_of(directory, name);
            if !path.is_file() {
                return Err(format!("the map {} is not in {}", name.as_str(), directory.display()));
            }
            read_file(&path)?
        }
    };
    let shape = Arc::new(Relief::of(&read).map_err(|why| format!("{}: {why}", map.label()))?);
    shapes().insert(map, shape.clone());
    Ok(shape)
}

/// Forget the shapes of the maps that were taken in: their files may
/// have been changed by hand.
pub(super) fn forget_kept() {
    shapes().retain(|map, _| matches!(map, MapOf::BuiltIn(_)));
}

/// Ask for a map file and take it in. `None` when the dialog is
/// dismissed.
pub(super) async fn pick() -> Option<Result<MapName, String>> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Vector displacement map")
        .add_filter("OpenEXR", &["exr"])
        .pick_file()
        .await?;
    let Some(directory) = directory() else {
        return Some(Err("there is nowhere to keep a map".to_string()));
    };
    Some(take_in(handle.path(), &directory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// An OpenEXR file of `width` by `height`, each pixel from `pixel`.
    pub(crate) fn exr_of(
        width: usize,
        height: usize,
        pixel: impl Fn(usize, usize) -> (f32, f32, f32) + Sync,
    ) -> Vec<u8> {
        use exr::prelude::WritableImage;
        let image = exr::prelude::Image::from_channels(
            (width, height),
            exr::prelude::SpecificChannels::rgb(|at: exr::math::Vec2<usize>| pixel(at.x(), at.y())),
        );
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write().non_parallel().to_buffered(&mut bytes).expect("writing");
        bytes.into_inner()
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let directory =
                std::env::temp_dir().join(format!("hani-maps-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&directory);
            std::fs::create_dir_all(&directory).expect("a scratch directory");
            Self(directory)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A map file is read as it was written: red across, green up, blue
    /// out, the first row of the picture the top of the square. One that
    /// is bigger than a map may be is read smaller, and what is not a
    /// map file is said.
    #[test]
    fn a_map_is_read_from_an_openexr_file() {
        // A bump that leans: out by 0.3 in the middle, and across by the
        // pixel's own height up the picture, so that top and bottom differ.
        let bump = |x: usize, y: usize, size: usize| {
            let (u, v) = (
                x as f32 / (size - 1) as f32 * 2.0 - 1.0,
                1.0 - y as f32 / (size - 1) as f32 * 2.0,
            );
            let dome = (1.0 - (u * u + v * v) / 0.49).max(0.0);
            (0.2 * v * dome, 0.0, 0.3 * dome)
        };
        let map = read_bytes(&exr_of(65, 65, |x, y| bump(x, y, 65))).expect("a map");
        assert_eq!(map.size(), 65);
        assert!((map.sample(0.0, 0.0).z - 0.3).abs() < 0.01);
        let (high, low) = (map.sample(0.0, 0.4), map.sample(0.0, -0.4));
        assert!(
            high.x > 0.02 && low.x < -0.02,
            "the top of the picture is not the top of the square"
        );
        assert_eq!(map.sample(1.0, 1.0), Vec3::ZERO);

        let big = read_bytes(&exr_of(1100, 1100, |x, y| bump(x, y, 1100))).expect("a big map");
        assert!(big.size() <= MAX_MAP_SIZE && big.size() >= MAX_MAP_SIZE / 2, "{}", big.size());
        assert!((big.sample(0.0, 0.0).z - 0.3).abs() < 0.02);

        assert!(read_bytes(b"not a picture at all").is_err());
        assert!(read_bytes(&exr_of(2, 2, |_, _| (0.0, 0.0, 1.0))).is_err());
    }

    /// A map taken in is kept under its file's name, is there to be read
    /// again, and is refused if it is no map or would hide one that
    /// comes with the application.
    #[test]
    fn a_map_taken_in_is_kept() {
        let scratch = Scratch::new("kept");
        let (from, maps) = (scratch.0.join("downloads"), scratch.0.join("maps"));
        std::fs::create_dir_all(&from).expect("a directory");
        let bytes = exr_of(33, 33, |x, y| {
            let (u, v) = (x as f32 / 16.0 - 1.0, y as f32 / 16.0 - 1.0);
            (0.0, 0.0, 0.25 * (1.0 - (u * u + v * v) / 0.5).max(0.0))
        });
        let file = from.join("My ear (left)!.exr");
        std::fs::write(&file, &bytes).expect("writing");
        assert!(kept(Some(&maps)).is_empty());
        let name = take_in(&file, &maps).expect("a map taken in");
        assert_eq!(name.as_str(), "My ear left");
        assert_eq!(kept(Some(&maps)), vec![name]);
        assert!(maps.join("My ear left.exr").is_file());
        forget_kept();
        let shape = relief_in(MapOf::Named(name), Some(&maps)).expect("its shape");
        assert!((shape.span().1 - 0.25).abs() < 0.02);

        let stud = from.join("stud.exr");
        std::fs::write(&stud, &bytes).expect("writing");
        assert!(take_in(&stud, &maps).is_err(), "a map hid one that comes with the application");
        let flat = from.join("flat.exr");
        std::fs::write(&flat, exr_of(9, 9, |_, _| (0.0, 0.0, 0.0))).expect("writing");
        assert!(take_in(&flat, &maps).is_err(), "a map that displaces nothing was taken in");
        let text = from.join("notes.exr");
        std::fs::write(&text, "not a picture").expect("writing");
        assert!(take_in(&text, &maps).is_err());
        assert_eq!(kept(Some(&maps)).len(), 1);

        let gone = MapOf::Named(MapName::new("never there").expect("a name"));
        assert!(relief_in(gone, Some(&maps)).is_err());
    }

    /// A name is what a file can be called, no longer than fits, and a
    /// brush file's `map:` names a map that comes with the application
    /// before one that was taken in.
    #[test]
    fn a_map_has_a_name() {
        assert_eq!(MapName::new("  Dragon_scale-2  ").expect("a name").as_str(), "Dragon_scale-2");
        assert_eq!(MapName::new("a/b\\c:d").expect("a name").as_str(), "abcd");
        assert!(MapName::new(" ../ ").is_none());
        let long = MapName::new(&"é".repeat(40)).expect("a name");
        assert_eq!(long.as_str().chars().count(), LONGEST_NAME / 2);
        assert_eq!(MapOf::named("horn"), Some(MapOf::BuiltIn(BuiltInMap::Horn)));
        assert_eq!(MapOf::named("Ear").map(|map| map.label().to_string()), Some("Ear".to_string()));
        assert_eq!(MapOf::named("!!"), None);
    }
}
