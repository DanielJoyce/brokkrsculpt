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

/// Hand bytes to the browser as a download of the file `name`. Natively
/// nothing: files are written where they belong. See `files.rs`.
#[cfg(target_arch = "wasm32")]
pub fn download(name: &str, bytes: &[u8]) {
    glue::hani_download(name, bytes);
}

/// Open a link in a new tab. See `links.rs`, which decides which links.
#[cfg(target_arch = "wasm32")]
pub fn open_tab(link: &str) -> Result<(), String> {
    glue::hani_open_tab(link);
    Ok(())
}

/// A sculpt kept in the browser's own storage, as the list of them shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub name: String,
    /// When it was saved, in the browser's own words for the user's locale.
    pub when: String,
    pub bytes: u64,
}

/// The sculpts kept in the browser, newest first.
pub async fn stored() -> Result<Vec<Stored>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let listed =
            wasm_bindgen_futures::JsFuture::from(glue::hani_store_list()).await.map_err(said)?;
        let listed: js_sys::Array = listed.dyn_into().map_err(said)?;
        let field = |row: &wasm_bindgen::JsValue, key: &str| {
            js_sys::Reflect::get(row, &wasm_bindgen::JsValue::from_str(key)).unwrap_or_default()
        };
        Ok(listed
            .iter()
            .map(|row| Stored {
                name: field(&row, "name").as_string().unwrap_or_default(),
                when: field(&row, "when").as_string().unwrap_or_default(),
                bytes: field(&row, "size").as_f64().unwrap_or(0.0) as u64,
            })
            .collect())
    }
    #[cfg(not(target_arch = "wasm32"))]
    Err(NOT_HERE.to_string())
}

/// Keep a sculpt's bytes in the browser under `name`, over any of that name.
pub async fn store(name: String, bytes: Vec<u8>) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::JsFuture::from(glue::hani_store_put(&name, &bytes))
            .await
            .map(drop)
            .map_err(said)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        drop((name, bytes));
        Err(NOT_HERE.to_string())
    }
}

/// A sculpt's bytes from the browser, or `None` if none has that name.
pub async fn fetch(name: String) -> Result<Option<Vec<u8>>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let found = wasm_bindgen_futures::JsFuture::from(glue::hani_store_get(&name))
            .await
            .map_err(said)?;
        if found.is_null() || found.is_undefined() {
            return Ok(None);
        }
        Ok(Some(js_sys::Uint8Array::new(&found).to_vec()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        drop(name);
        Err(NOT_HERE.to_string())
    }
}

/// Take a sculpt out of the browser's storage.
pub async fn forget(name: String) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::JsFuture::from(glue::hani_store_delete(&name))
            .await
            .map(drop)
            .map_err(said)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        drop(name);
        Err(NOT_HERE.to_string())
    }
}

/// Ask the browser to warn before the page is left, or stop asking. On
/// while there is work not saved. Natively nothing: the application asks
/// itself.
pub fn warn_on_leaving(warn: bool) {
    #[cfg(target_arch = "wasm32")]
    glue::hani_warn_on_leaving(warn);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = warn;
}

/// Whether the window's own buttons (minimise, maximise, close) are drawn.
///
/// On the desktop, always. In a browser tab, never: the tab is the window,
/// and closing the drawing surface from inside it left a page that drew
/// nothing. A page hosted in a webview, which has no tab, asks for them:
/// `?window-buttons=1` in its address, or `window.hanisculpt =
/// { windowButtons: true }` set before the module loads. Pressed there,
/// they tell the host ([`tell_host`]) and leave the page alone. Asked once.
/// See `docs/WEB.md`.
pub fn window_buttons() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        static ASKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ASKED.get_or_init(glue::hani_window_buttons)
    }
    #[cfg(not(target_arch = "wasm32"))]
    true
}

/// Tell the page's host a window button was pressed: `close` (once any
/// unsaved work has been saved or let go), `minimise` or `maximise`. Every
/// way a webview listens is tried; see the glue below and `docs/WEB.md`.
/// Natively nothing: the application has its own window.
pub fn tell_host(action: &str) {
    #[cfg(target_arch = "wasm32")]
    glue::hani_tell_host(action);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = action;
}

/// A name for a sculpt saved before it was given one: the date and time.
pub fn fresh_name() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let now = js_sys::Date::new_0();
        format!(
            "Sculpt {}-{:02}-{:02} {:02}.{:02}",
            now.get_full_year(),
            now.get_month() + 1,
            now.get_date(),
            now.get_hours(),
            now.get_minutes()
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    "Sculpt".to_string()
}

#[cfg(not(target_arch = "wasm32"))]
const NOT_HERE: &str = "there is no browser storage here";

/// What a refused promise said, as text.
#[cfg(target_arch = "wasm32")]
fn said(error: wasm_bindgen::JsValue) -> String {
    js_sys::Reflect::get(&error, &wasm_bindgen::JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_else(|| "the browser refused".to_string())
}

/// The browser's side: a download, and IndexedDB, which keeps bytes of any
/// size where `localStorage` keeps a few megabytes of text. Two object
/// stores, so that listing the sculpts reads their names and not their
/// bytes. Compiled into the module, so the page needs nothing of its own.
#[cfg(target_arch = "wasm32")]
mod glue {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
const DB = "hanisculpt";
function opened() {
  return new Promise((done, failed) => {
    const asked = indexedDB.open(DB, 1);
    asked.onupgradeneeded = () => {
      asked.result.createObjectStore("about", { keyPath: "name" });
      asked.result.createObjectStore("bytes");
    };
    asked.onsuccess = () => done(asked.result);
    asked.onerror = () => failed(asked.error);
  });
}
function within(mode, work) {
  return opened().then((db) => new Promise((done, failed) => {
    const t = db.transaction(["about", "bytes"], mode);
    let answer;
    work(t, (value) => { answer = value; });
    t.oncomplete = () => { db.close(); done(answer); };
    t.onerror = () => { db.close(); failed(t.error); };
    t.onabort = () => { db.close(); failed(t.error || new Error("the browser stopped the save")); };
  }));
}
export function hani_store_put(name, bytes) {
  const copy = bytes.slice();
  return within("readwrite", (t) => {
    t.objectStore("about").put({ name, saved: Date.now(), size: copy.length });
    t.objectStore("bytes").put(copy, name);
  });
}
export function hani_store_get(name) {
  return within("readonly", (t, answer) => {
    const asked = t.objectStore("bytes").get(name);
    asked.onsuccess = () => answer(asked.result === undefined ? null : asked.result);
  });
}
export function hani_store_delete(name) {
  return within("readwrite", (t) => {
    t.objectStore("about").delete(name);
    t.objectStore("bytes").delete(name);
  });
}
export function hani_store_list() {
  return within("readonly", (t, answer) => {
    const asked = t.objectStore("about").getAll();
    asked.onsuccess = () => answer(asked.result
      .sort((a, b) => b.saved - a.saved)
      .map((row) => ({
        name: row.name,
        size: row.size,
        when: new Date(row.saved).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }),
      })));
  });
}
export function hani_download(name, bytes) {
  const url = URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}
export function hani_open_tab(link) {
  window.open(link, "_blank", "noopener");
}
export function hani_window_buttons() {
  const asked = new URLSearchParams(window.location.search).get("window-buttons");
  if (asked !== null) { return asked === "" || asked === "1" || asked === "true"; }
  const host = globalThis.hanisculpt;
  return !!(host && host.windowButtons);
}
export function hani_tell_host(action) {
  const message = { source: "hanisculpt", action };
  const host = globalThis.hanisculpt;
  try { if (host && typeof host.onWindow === "function") { host.onWindow(action); } } catch (_) {}
  try { if (window.chrome && window.chrome.webview) { window.chrome.webview.postMessage(message); } } catch (_) {}
  try {
    const handlers = window.webkit && window.webkit.messageHandlers;
    if (handlers && handlers.hanisculpt) { handlers.hanisculpt.postMessage(message); }
  } catch (_) {}
  try { if (window.parent && window.parent !== window) { window.parent.postMessage(message, "*"); } } catch (_) {}
  window.dispatchEvent(new CustomEvent("hanisculpt-window", { detail: action }));
}
function leaving(event) { event.preventDefault(); event.returnValue = ""; }
export function hani_warn_on_leaving(warn) {
  window.removeEventListener("beforeunload", leaving);
  if (warn) { window.addEventListener("beforeunload", leaving); }
}
"#)]
    extern "C" {
        pub fn hani_store_put(name: &str, bytes: &[u8]) -> js_sys::Promise;
        pub fn hani_store_get(name: &str) -> js_sys::Promise;
        pub fn hani_store_delete(name: &str) -> js_sys::Promise;
        pub fn hani_store_list() -> js_sys::Promise;
        pub fn hani_download(name: &str, bytes: &[u8]);
        pub fn hani_warn_on_leaving(warn: bool);
        pub fn hani_open_tab(link: &str);
        pub fn hani_window_buttons() -> bool;
        pub fn hani_tell_host(action: &str);
    }
}
