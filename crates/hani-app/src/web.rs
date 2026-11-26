// SPDX-License-Identifier: AGPL-3.0-only

//! What the application is missing in a browser, named in one place.
//!
//! The web build is the same application compiled for `wasm32`. Most of it
//! needs nothing: the field, the brushes, the meshing and the renderer are
//! the same code. What a browser does not have is a file system the
//! application can name paths in, so a file picked in a dialog has no path.
//! Here that reads as an empty one, which every reader turns into "could not
//! read", rather than a hole in the build. See `docs/WEB.md`.

/// A picked file's path, which in a browser it does not have.
///
/// Named `path` so the call sites read the same on every target: natively
/// `rfd::FileHandle::path` is an inherent method and wins; in a browser
/// there is none, and this answers.
#[cfg(target_arch = "wasm32")]
pub trait NoPath {
    fn path(&self) -> &std::path::Path;
}

#[cfg(target_arch = "wasm32")]
impl NoPath for rfd::FileHandle {
    fn path(&self) -> &std::path::Path {
        std::path::Path::new("")
    }
}

/// Send a panic's message to the browser's console. Natively nothing: the
/// default hook already prints it.
pub fn console_panics() {
    #[cfg(target_arch = "wasm32")]
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&info.to_string()));
    }));
}
