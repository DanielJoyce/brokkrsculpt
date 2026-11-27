<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# The web version

Asked for by the maintainer on 2026-11-26: *"See if you can get a wasm build
working and get this thing hosted on a GitHub page. Some kind of branch that
when it's merged to it builds and publishes a gh page."*

The web version is the same application, compiled for `wasm32-unknown-unknown`
and drawn through WebGPU. The field, the brushes, the meshing, the renderer
and the panels are the same code as on the desktop.

## Publishing it

The `pages` branch is the trigger. Whatever is merged or pushed into it is
built by `.github/workflows/pages.yml` and published to the repository's
GitHub Pages site. To publish what is on `main`, fast-forward `pages` to
it (`git push origin origin/main:refs/heads/pages`), or open a pull
request from `main` into `pages` and merge it.

**One setting, once, by someone with admin rights on the repository:**
Settings, Pages, *Build and deployment*, Source: **GitHub Actions**. Until
that is set the build succeeds and the publish step fails, saying Pages is
not enabled. The workflow cannot turn it on itself: the token a workflow
runs with is not allowed to.

## Building it by hand

```bash
rustup target add wasm32-unknown-unknown
```

```bash
cargo install wasm-bindgen-cli --version 0.2.127 --locked
```

```bash
scripts/web-build.sh site
```

```bash
python3 -m http.server -d site 8080
```

`scripts/web-build.sh` reads the `wasm-bindgen` version out of `Cargo.lock`
and refuses a tool that does not match it, because a mismatch builds and
then fails in the browser. The page is `web/index.html`; it says so plainly
when the browser has no WebGPU.

## What it needed

- **No sockets and no executables in a browser.** The updater, the bug
  report's request and the printer's request are native only: their crates
  (`ureq`, `ring`, `minisign-verify`) are dependencies only off `wasm32`, and
  each function that used them has a browser twin that refuses with a reason.
  The updater reads as inert, as it does on any build that is not a release.
- **No folders.** `dir_spec` does not build for a browser; `paths.rs` answers
  `None` for every folder there, so a setting is not kept and nothing is
  looked for, as on a machine with no home directory.
- **No paths for picked files.** `rfd`'s dialogs work in a browser, but a file
  picked there has no path, and every reader here takes one. Opening,
  importing and exporting go through `files.rs` instead, below. The other
  pickers (decal and stamp pictures, brush files, maps, the fit test) still
  give an empty path there, which their readers turn into "could not read".
- **No clock in `std`.** `std::time::Instant::now()` panics on
  `wasm32-unknown-unknown`. Every `Instant` in the three crates is now
  `web_time::Instant`, which is the standard library's own on every other
  target, so the desktop build is unchanged. The three `SystemTime::now()`
  stamps the same.
- **No threads.** Rayon's global pool falls back to the calling thread when a
  browser refuses to make one, so every parallel loop runs, on one core.
  Every thread the application starts itself already handles being refused.

## Seen running

On 2026-11-26, the site from `scripts/web-build.sh` served locally and
opened in the headless Chromium this container has, with WebGPU on
SwiftShader (`--enable-unsafe-webgpu --use-angle=vulkan
--enable-features=Vulkan`; with `--use-webgpu-adapter=swiftshader` instead,
iced's own staging buffer failed to map, which is that adapter's and was not
pursued):

- The welcome screen, the panels and the starting ball drew, with nothing in
  the console.
- A Draw stroke across the ball raised a ridge, and the title went to
  `untitled*`.
- The wasm file is 13 MB, unoptimised (`wasm-opt` is not run).

Three things the build needed that only showed in a browser: `env_logger`
stamps its lines with the standard clock and panicked on the first log line,
so it is not started there; a panic printed only "unreachable" until `web.rs`
sent its message to the console; and the overlay's line pipeline had a depth
bias, which WebGPU refuses on lines, so it has none there.

## Opening and saving

Asked for by the maintainer on 2026-11-26: *"we need to figure out load and
save so people can save their work to their computer. Initially it could
just be to the browser storage but letting them load and save from their PC
is critical too."* Both are there.

- **From the computer.** *Open…* and *Import and replace…* open the
  browser's own file chooser (rfd clicks a hidden file input while the press
  that asked still counts as the user's, and shows its own small card when
  it no longer does). The picked file's bytes are held under its name in
  `files.rs`, and the name goes on as the path, so `open_project` and the
  import read them as they read a file on disk. A mesh is read by
  `hani_core::import::read_named`, which is `read_path` without the disk.
- **To the computer.** *Save to computer* in the File menu, and *Download*
  on a kept sculpt, hand the `.hani` bytes to the browser as a download. So
  does every export: `files::create` is, in a browser, a writer that
  collects the bytes and downloads them under the file's name when it is
  flushed. An OBJ with a texture is three downloads, the OBJ, its material
  file and its picture.
- **In the browser.** *Save* keeps the sculpt in the browser's IndexedDB
  under its name, and *Save As…* and *Sculpts in this browser…* bring up a
  card that names it and lists what is kept, newest first, with its date and
  size, to Open, Download or Delete (a second press deletes). IndexedDB and
  not `localStorage`, which holds a few megabytes of text: the starting ball
  is 40 MB. Names and sizes are in one object store and the bytes in
  another, so the list does not read the bytes. The JavaScript is compiled
  into the module (`web.rs`), so the page needs nothing of its own.
- **The unsaved-work prompt's Save** keeps the sculpt in the browser, under
  the date and time if it has no name yet, and goes on to what was asked
  only when the browser says it has the bytes. A refusal (a full disk, a
  private window) leaves the work unsaved and the prompt up.
- **Leaving the page** with work not saved makes the browser ask first.

Seen in headless Chromium on 2026-11-26, the site from
`scripts/web-build.sh`: the starting ball kept (41,948,819 bytes in
IndexedDB), listed, opened again, downloaded from the list and by *Save to
computer* (the same 41,948,819 bytes), exported as an STL (27,127,884
bytes, "watertight"), that `.hani` opened through *Open…* and that STL
imported through *Import and replace…* (542,556 triangles, 5.6 s on one
core), and the browser asking before the page was left after a stroke and
not before it.

## The window's buttons, and a page in a webview

In a browser tab the title bar has no Minimise, Maximise or Close: the
tab is the window and has its own, and the leave warning (above) covers
unsaved work. They were there until 2026-11-27, when the maintainer found
that Close, then Discard, left the page drawing nothing: it had closed
the drawing surface inside the page, which is not a window that can be
closed.

A page hosted in a webview (a desktop shell around the web build, with no
tab) wants them, and asks:

- `?window-buttons=1` in the page's address, or
- `window.hanisculpt = { windowButtons: true }` set by the host before
  the module loads (a script the webview injects at document start).

Then the three buttons are drawn, and pressing one tells the host and
leaves the page as it is. Close asks about unsaved work first, as on the
desktop, and tells the host only once it has been saved or let go (the
leave warning is taken down at the same moment, so the host can close
the page without the browser asking again). The message is
`{ source: "hanisculpt", action }`, `action` being `close`, `minimise` or
`maximise`, sent every way a webview listens:

| Host | How it hears |
|---|---|
| Any | `window.hanisculpt.onWindow(action)`, if the host set that function |
| WebView2 (Windows) | `window.chrome.webview.postMessage(message)` |
| WKWebView (macOS, iOS) | `window.webkit.messageHandlers.hanisculpt.postMessage(message)` |
| A parent page (iframe) | `window.parent.postMessage(message, "*")` |
| Any | a `hanisculpt-window` event on `window`, `detail` the action |

What the host does with it is the host's: close its window, minimise it,
maximise it. Dragging the title bar to move the window is not sent.

## Fonts

A browser has no system fonts: there are only the faces the application
bundles. Shaping text in a family or a style with no face in it panics in
cosmic-text ("no default font found"), which is how switching Help to a
page with an italic in it stopped the web version (reported by the
maintainer on 2026-11-26; seen in headless Chromium, and found by cutting
the page down until it stopped). The help pages now draw in Inter
throughout, Inter Italic is bundled for their emphasis, and in a browser
`theme::MONO` is Inter, which also brought back the scale bar's label.

## A browser with WebGPU and no adapter

Reported by the maintainer on 2026-11-26 from the published page: Chrome
had `navigator.gpu` and no adapter for it ("No available adapters"; Chrome
turns WebGPU off and on by itself on some systems). iced then fell back to
its software renderer, which failed on the same canvas: *"A canvas context
other than `CanvasRenderingContext2d` was already created"*. The page now
asks for an adapter itself before it starts the application, and with none
it says so and says what to turn on, with Haniko. Seen in headless Chromium
both ways: with WebGPU turned off the message, with it on the application.
The maintainer then had it running in Firefox once `dom.webgpu.enabled` was
on in `about:config`, which the message now says too.

## Lint

The browser build compiles paths the desktop does not, so it is linted on
its own: `cargo clippy --workspace --target wasm32-unknown-unknown` under
`-D warnings`, in CI and in the gate, and the Pages and release builds run
with `-D warnings`. In a browser the updater's machinery is unreachable (its
entry points refuse there), so `update.rs` and `update/apply.rs` allow dead
code on that target only; natively every item in them is used.

## Not done

- **Pictures, brush files, maps and the fit test** are not read or written
  in a browser yet: their pickers give no bytes.
- **No autosave** in a browser, and no settings kept between visits. The
  warning on leaving is what there is.
- **One core.** Threads in a browser need `SharedArrayBuffer`, which needs
  the page served with cross-origin isolation headers, which GitHub Pages
  cannot set, and a nightly compiler for the atomics. Strokes on a large
  model will be slower than on the desktop.
- **WebGPU only.** There is no WebGL fallback: the renderer's storage buffers
  and limits are WebGPU's.
- **Numbers in a browser are in Inter**, not monospace, so a changing
  column of them can shift by a pixel.
- **Lines on the surface have no depth bias** in a browser (see above), so a
  ring lying exactly on the surface may break into dashes.
- *Check for updates* is shown and does nothing there.
- **Not tried on a real graphics card**, nor in Firefox or Safari.
