// SPDX-License-Identifier: AGPL-3.0-only

//! Where a file is read from and written to.
//!
//! On the desktop that is the file system, by path. A browser has no paths:
//! a file the user picks arrives as a name and its bytes, and a file the
//! application writes can only be handed back as a download. So the readers
//! and writers that open, import and export go through here, and a path is
//! what they are given on every target:
//!
//! - A picked file's bytes are **held** under its name ([`hold`]), and the
//!   name goes on as the path. [`open`] and [`read_mesh`] look there before
//!   the file system, and take the bytes out, so a picked file is read once.
//! - In a browser, [`create`] gives a writer that collects the bytes and
//!   hands them to the browser as a download of that name when it is
//!   flushed. On the desktop it is `File::create`.
//!
//! The store of sculpts kept inside the browser is `app/shelf.rs`. See
//! `docs/WEB.md`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Picked files not yet read, by name. A few at most, and usually one.
static HELD: Mutex<Vec<(PathBuf, Vec<u8>)>> = Mutex::new(Vec::new());

/// Keep a picked file's bytes under its name until it is read, and return
/// the name to pass on as its path. A second file of the same name replaces
/// the first.
pub fn hold(name: &str, bytes: Vec<u8>) -> PathBuf {
    let path = PathBuf::from(name);
    let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    held.retain(|(existing, _)| existing != &path);
    held.push((path.clone(), bytes));
    path
}

/// Take a held file's bytes out, if `path` names one.
fn take(path: &Path) -> Option<Vec<u8>> {
    let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let at = held.iter().position(|(existing, _)| existing == path)?;
    Some(held.swap_remove(at).1)
}

/// Open a file to read: a held one if `path` names it, else the file on disk.
pub fn open(path: &Path) -> std::io::Result<Box<dyn Read>> {
    if let Some(bytes) = take(path) {
        return Ok(Box::new(std::io::Cursor::new(bytes)));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(Box::new(std::io::BufReader::new(std::fs::File::open(path)?)))
    }
    #[cfg(target_arch = "wasm32")]
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "pick the file again"))
}

/// Read a mesh to import: a held one if `path` names it, else from disk.
pub fn read_mesh(path: &Path) -> Result<hani_core::ExportMesh, hani_core::import::ImportError> {
    if let Some(bytes) = take(path) {
        return hani_core::import::read_named(path, &bytes);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        hani_core::import::read_path(path)
    }
    #[cfg(target_arch = "wasm32")]
    Err(hani_core::import::ImportError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "pick the file again",
    )))
}

/// Create a file to write. In a browser, a download of that file's name,
/// which starts when the writer is flushed.
pub fn create(path: &Path) -> std::io::Result<Box<dyn Write>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(Box::new(std::fs::File::create(path)?))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sculpt".to_string());
        Ok(Box::new(Download { name, bytes: Vec::new() }))
    }
}

/// How many bytes a file just written holds. In a browser, those of the
/// last download of that name.
pub fn size(path: &Path) -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::metadata(path).map(|data| data.len()).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
        let sizes = SIZES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        sizes.iter().rev().find(|(of, _)| Some(of) == name.as_ref()).map_or(0, |(_, size)| *size)
    }
}

/// The sizes of the downloads made, by name, for [`size`].
#[cfg(target_arch = "wasm32")]
static SIZES: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

/// A file written in a browser: its bytes, handed over as a download when
/// flushed. A writer dropped without a flush, as one is when a writer stops
/// at an error, downloads nothing.
#[cfg(target_arch = "wasm32")]
struct Download {
    name: String,
    bytes: Vec<u8>,
}

#[cfg(target_arch = "wasm32")]
impl Write for Download {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.bytes.is_empty() {
            return Ok(());
        }
        let bytes = std::mem::take(&mut self.bytes);
        crate::web::download(&self.name, &bytes);
        let mut sizes = SIZES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        sizes.retain(|(of, _)| of != &self.name);
        sizes.push((self.name.clone(), bytes.len() as u64));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_file_is_read_once_by_its_name() {
        let path = hold("held-once.hani", b"the bytes".to_vec());
        assert_eq!(path, PathBuf::from("held-once.hani"));
        let mut read = String::new();
        open(&path).unwrap().read_to_string(&mut read).unwrap();
        assert_eq!(read, "the bytes");
        // Taken out by the read: a second open goes to the disk, where there
        // is no such file.
        assert!(open(&path).is_err());
    }

    #[test]
    fn a_second_file_of_the_same_name_replaces_the_first() {
        hold("held-twice.hani", b"first".to_vec());
        let path = hold("held-twice.hani", b"second".to_vec());
        let mut read = String::new();
        open(&path).unwrap().read_to_string(&mut read).unwrap();
        assert_eq!(read, "second");
        assert!(open(&path).is_err());
    }

    #[test]
    fn a_held_mesh_is_read_by_the_extension_of_its_name() {
        let ascii = b"solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\n\
                      vertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let path = hold("held-mesh.stl", ascii.to_vec());
        let mesh = read_mesh(&path).unwrap();
        assert_eq!(mesh.triangles.len(), 1);
    }
}
