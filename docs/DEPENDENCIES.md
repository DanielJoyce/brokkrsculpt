<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# Dependencies: what we build, and what we could stop building

Asked for by the maintainer on 2026-11-26, after `arboard` came in with
Snip and report: *"investigate reducing dependencies. See what we can pare
if we impl some things ourselves or maybe just vendor them ( turn them into
a crate ) and strip out what we don't need. Make some proposals"*. Tracked
in [DanielJoyce/hanisculpt#133](https://github.com/DanielJoyce/hanisculpt/issues/133).

These were proposals. On 2026-11-27 the maintainer said: *"Retire
tinkeratlas bug report. Do 1 and 2 and 4 and 5"*. What has been done, and
measured after, is under **Done** at the end.

## How it was measured

Every number below is a count of distinct packages (name and version) that
`cargo tree -p hani-app -e no-dev --target <target>` lists, measured on
2026-11-26 against the lockfile at `7c3bfab`. "Distinct packages" is what
gets compiled, so it is the number that build time and the supply chain
both follow. The cost of one dependency was measured two ways:

- `cargo tree --prune <crate>`, which leaves out what only that crate
  brings;
- and, for anything feature-related or where two of ours share crates, by
  editing a copy of the manifests in a scratch directory and counting again.
  `--prune` cannot see features, so the second is the one that counts.

Today:

| Target | Packages |
|---|---|
| Linux x86_64 | 340 |
| Windows x86_64 | 259 |
| macOS arm64 | 254 |
| Browser (wasm32) | 188 |

Not measured: compile time and binary size per crate (`cargo bloat` and
`cargo build --timings` would answer those, and the build machine has
neither installed); Windows and macOS were counted, not built.

## The proposals, largest first

Savings are Linux packages unless said; each was measured on its own on top
of proposal 1, so they add up only roughly. Proposals 1 to 7 and both of 9
together were also measured: **Linux 340 to 222, browser 188 to 152.**

| # | Proposal | Linux | Browser | Work | Risk |
|---|---|---|---|---|---|
| 1 | iced without the defaults we do not use | -57 | -8 | a manifest line | low |
| 2 | `env_logger` without its default features | -13 | -13 | a manifest line | none |
| 3 | arboard without Wayland's data-control | -6 | 0 | a manifest line | low |
| 4 | The updater and bug report without `ureq` | -13 to -17 | 0 | small to medium | medium |
| 5 | Our own evdev layer over `libc` | -7 | 0 | medium | medium |
| 6 | Our own OpenEXR reader | -7 | -4 | medium | medium |
| 7 | arboard replaced | -7 | 0 | small or medium | low |
| 8 | Our own logger | -2 more than 2 | -2 | small | none |
| 9 | No `serde_json`; no Markdown | -3; -4 | -4; -5 | medium; medium | low |

### 1. iced without the defaults we do not use (-57)

`iced = "0.14"` in the workspace takes iced's default features, and three of
them are unused here:

- **`linux-theme-detection`, 41 packages** on Linux (a D-Bus stack, to ask
  the desktop whether it is in dark mode). The application never asks:
  `main.rs` sets `.theme(app_theme)`, which returns the one theme it has.
- **`tiny-skia`, 5 on Linux and 7 in a browser**: the software renderer iced
  falls back to without a GPU. The viewport is a wgpu shader and draws
  nothing without one, so the fallback starts a window that cannot sculpt.
  In a browser it was worse: the fallback failed on the canvas and the page
  now checks for an adapter before starting (`docs/WEB.md`). Without it, a
  machine with no adapter gets iced's error instead of a blank viewport.
- **`debug`, 8 on Linux** (`iced_devtools`). Turned on in `hani-app`'s
  manifest with no comment and nothing in the source referring to it.

Proposed: `default-features = false` on the workspace entry, and in
`hani-app` the features actually used: `wgpu`, `x11`, `wayland`,
`thread-pool`, `crisp`, `web-colors`, `advanced`, `canvas`,
`image-without-codecs`, `markdown`. Measured: Linux 340 to 283, browser
188 to 180. To check on a screen: Wayland and X11 sessions, and the window
on a machine with Vulkan only through llvmpipe.

### 2. `env_logger` without its default features (-13, and -13 in a browser)

`env_logger` with its defaults brings regex (four crates), terminal colour
detection (seven) and a date library (two) to print log lines. Without them
`RUST_LOG=info` and `RUST_LOG=hani_app=debug` still work; what goes is
regex filters in `RUST_LOG`, colour and the timestamp's formatting.
Measured: Linux 283 to 270, browser 180 to 167 (the browser builds it and
does not start it; moving it to the native-only table saves the rest).

### 3. arboard without Wayland's data-control (-6)

Six of arboard's seven are its `wayland-data-control` feature
(`wl-clipboard-rs`, `tree_magic_mini`, `os_pipe`, `petgraph`, `fixedbitset`,
`nom`). Without it, a picture is put on the clipboard through X11, which a
Wayland desktop serves to its windows through XWayland; a Wayland session
with no XWayland loses the picture and keeps the issue link. Measured:
Linux 283 to 277. (The ticket said `image-data` turns on TIFF everywhere:
it does on macOS only, measured, where arboard writes a picture as TIFF.)

### 4. The updater and bug report without `ureq` (-13 to -17)

`ureq` is 13 packages (rustls, its root certificates and the HTTP parts),
and with `ring` and `minisign-verify` 17. It has three users:

- **The bug report** posts to `tinkeratlas.com` (`report.rs`), with the
  TinkerAtlas account (`account.rs`). The maintainer took the TinkerAtlas
  links off the welcome screen on 2026-11-26, and Snip and report now files
  issues on GitHub. **Decision for the maintainer: retire the TinkerAtlas
  report?** If so, `report.rs`'s network half and `account.rs` go.
- **The printer check** talks to Moonraker on the local network over plain
  HTTP. A GET over `std::net::TcpStream` is about fifty lines and no crate.
- **The updater** downloads from GitHub releases over HTTPS, which needs TLS.
  Either keep `ureq` for this alone, or run the system's `curl` (shipped with
  Windows 10 and later, macOS and nearly every Linux), which takes TLS and
  proxies off our hands. The download is still checked against its
  minisign signature and SHA-256 before anything is run, so `curl` is not
  trusted with more than `ureq` is now. `ring` and `minisign-verify` stay for
  that check (the manifest forbids hand-rolling SHA-256, rightly).

With all three: -13 (ureq's share) on every desktop. Also retires the
`~3.3` bound the manifest holds `ureq` on, and with it a second `base64`.

### 5. Our own evdev layer (-7)

`evdev` brings `nix` and `bitvec` (with `funty`, `radium`, `tap`, `wyz`).
What the tablet and SpaceMouse readers use of it is small: open a device,
ask its capabilities (`EVIOCGBIT`, `EVIOCGABS`, `EVIOCGNAME`), read
`input_event`s, and in the tests make a virtual device through `uinput`.
That is a few hundred lines of `ioctl` over `libc`, which is already in the
graph. It is unsafe code on a path that only real hardware exercises, so it
wants the device tests (`--features uinput`) run on a machine with
`/dev/uinput` before it goes in. Vendoring `evdev` and stripping it was
looked at and is worse: the bitvec use is spread through its types, and a
fork is a fork to keep up.

### 6. Our own OpenEXR reader (-7, and -4 in a browser)

`exr` reads displacement maps (`app/maps.rs`): one layer, three channels.
Its tree is `half`, `lebe`, `bit_field`, `zune-inflate`, `smallvec`, and
`multiversion` with its macros. A reader for scanline images in half or
float, uncompressed, RLE, ZIPS or ZIP, is a few hundred lines on
`miniz_oxide`, which PNG already brings. The cost is files it would refuse:
PIZ, PXR24, B44 and DWA compression, and tiled images. Which of those
ZBrush, Mudbox and Blender write for a vector displacement map should be
checked on real files before this is done; if PIZ is common, keep `exr`.

### 7. arboard replaced (-7)

If proposal 3 is not enough: either write the picture to the clipboard
ourselves through `x11rb` (in the graph already, from iced's clipboard) and
the Win32 and AppKit calls the window crates link, or drop the clipboard
picture: *Open an issue* saves the PNG and says where, and the user drags it
into the issue. The second is an hour; the first is a module per platform.

### 8. Our own logger (-2 beyond proposal 2)

A `log::Log` that prints level, target and message, filtered by a level in
`RUST_LOG`, is about forty lines. Over proposal 2 it saves `env_logger` and
`env_filter` only. Worth it only if proposal 2's output is not enough.

### 9. Not recommended now

- **`serde_json`** (-3): read for the slicer's presets, the paints, the
  printer's answer and the account. A JSON reader of our own is a parser to get
  right for three crates.
- **Help's Markdown** (-4, -5 in a browser): `pulldown-cmark` and a little.
  A renderer of the subset the pages use is a couple of hundred lines; the
  pages would then be held to that subset.
- **Vendoring iced, wgpu or cosmic-text**: most of the graph is theirs, and
  keeping a fork in step with them is more work than everything above.
- **wgpu without its OpenGL backend**: iced's text renderer (`cryoglyph`)
  turns on wgpu's defaults, so it is not ours to turn off, and the GL
  backend is the fallback on machines with no Vulkan.

## Duplicate versions, and whether upgrading folds them

Asked on 2026-11-26: *"Let's see if we can unify those disparate versions
too. Maybe we just need to upgrade"*. Mostly it does not, and the reason is
one crate: **iced 0.14.0 is the newest iced there is** (crates.io, checked
2026-11-26), and it holds winit 0.30, wgpu 27 and cosmic-text 0.15. Our wgpu
has to be iced's (the viewport shares its device), so ours cannot move ahead
of it either.

Tried in a scratch copy:

- **`cargo update`**, every semver-compatible upgrade at once: 505 packages
  to 498, and still 38 crates at two or more versions. It folds nothing, adds
  a second `miniz_oxide` (flate2's newest moved to 0.9 while png and exr are
  on 0.8), and takes in some releases days old. Not done.
- **`ureq` 3.3 to 3.4.2**: folds `base64` 0.22 into the 0.23 the rest use.
  Done, on 2026-11-26.

Who holds each duplicate, from the lockfile after that change:

| Crates | Versions | Held by | Ours to fold? |
|---|---|---|---|
| `smithay-client-toolkit`, `calloop`, `calloop-wayland-source`, `rustix`, `linux-raw-sys`, `thiserror` | 0.19/0.20, 0.13/0.14, ..., 0.38/1, 1/2 | winit 0.30 against iced's clipboard and the newer Wayland crates | no, waits on iced |
| `objc2` and eight `objc2-*`, `block2` | 0.5/0.6 | winit 0.30 and iced's macOS clipboard against rfd, arboard, softbuffer | no, waits on iced |
| `windows`, `windows-core` and four more | 0.58/0.62 | wgpu-hal against `mundy` (iced's theme detection) | **yes: proposal 1 removes `mundy`** |
| `windows-sys` | 0.52/0.59/0.61 | ring and winit; rustix; the rest | no |
| `glam` | 0.25/0.29/0.33 | iced; `fast-surface-nets` (a dev-dependency); ours | 0.29 only, by dropping the dev-dependency, in test builds |
| `skrifa`, `read-fonts`, `font-types` | two each | cosmic-text against swash | no |
| `hashbrown`, `foldhash` | 0.15/0.16/0.17 | gpu-descriptor and petgraph; wgpu; indexmap | petgraph goes with proposal 3; the rest no |
| `rustc-hash` | 1/2 | naga, wgpu-core, cosmic-text against iced and ours | no: iced is on 2 as well |
| `getrandom` | 0.2/0.4 | `ring` against the rest | only by dropping `ring` |
| `core-foundation`, `core-graphics-types` | two each | winit against wgpu-hal and ours | no |
| `bitflags`, `redox_syscall`, `jni-sys`, `syn` | | winit's macOS and Redox crates; Android only; proc macros split between syn 2 and 3 | no |

So: one fold now, six more (Windows only) with proposal 1, and the rest
when iced moves past winit 0.30. When an iced release after 0.14 is out and
has aged, upgrading to it is the change that folds the most; its git
`master` is not a release to ship on.

## Suggested order

1. Proposals 1, 2 and 3 together: three manifest changes, -76 on Linux,
   gated and looked at on a screen. Linux 340 to 264, Windows 259 to 227,
   macOS 254 to 223, browser 188 to 167 (measured).
2. The maintainer's call on the TinkerAtlas bug report, then proposal 4.
   (`ureq` itself moved to 3.4.2 on 2026-11-26; see above.)
3. Proposals 5 and 6 when there is hardware and sample files to test them
   with.

## Done

**Proposals 1 and 2**, 2026-11-27. iced's defaults are off in the
workspace and `hani-app` names the ten features it uses; `env_logger` has
no default features and is a native-only dependency. Measured after, with
the same `cargo tree` count: Linux 340 to 269, Windows 259 to 226, macOS
254 to 222, browser 188 to 165; the lockfile 505 packages to 431, none
added. Seen: the desktop window on X11 (Xvfb) and the web version in
Chromium both start and draw as before. Not seen: a native Wayland
session, and a machine with no GPU adapter, which now gets iced's error
where it got a window that could not draw the model.

## Not done

Compile time and binary size per crate were not measured. The Windows and macOS numbers are counts from the
lockfile, not builds.
