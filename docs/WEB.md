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
  picked there has no path, and every reader here takes one. `web.rs` gives
  it an empty path, which every reader turns into "could not read".
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

- **Opening, saving, importing and exporting files do not work** in the
  browser: the dialogs open, and what is picked cannot be read or written by
  path. Reading the picked file's bytes and saving by download is the next
  step.
- **One core.** Threads in a browser need `SharedArrayBuffer`, which needs
  the page served with cross-origin isolation headers, which GitHub Pages
  cannot set, and a nightly compiler for the atomics. Strokes on a large
  model will be slower than on the desktop.
- **WebGPU only.** There is no WebGL fallback: the renderer's storage buffers
  and limits are WebGPU's.
- **No settings kept** between visits, and no autosave.
- **The scale bar's label is missing**: it is drawn in a monospace face the
  application does not ship, and a browser offers it none.
- **Lines on the surface have no depth bias** in a browser (see above), so a
  ring lying exactly on the surface may break into dashes.
- *Check for updates* is shown and does nothing there.
- **Not tried on a real graphics card**, nor in Firefox or Safari.
