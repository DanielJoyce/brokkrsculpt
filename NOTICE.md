<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# Third-Party Notices

HaniSculpt is built on Rust crates. This file records their licensing and
satisfies their attribution requirements.

**Nothing here imposes a copyleft obligation on HaniSculpt.** The project is
AGPL-3.0-only by its own choice, not because a dependency forced it. Every one
of the **240 crates** compiled into a Linux build is permissively licensed
(MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, 0BSD, Unlicense, CC0-1.0 or
Unicode-3.0) but one, the `contracts` macro, which is MPL-2.0 and runs only
while compiling (below). There is no GPL, no LGPL and no EUPL in the graph. That is worth stating plainly because it is unusual: it
means nobody taking this code on inherits a linking question, a written offer,
or a relinking obligation from us.

Full license texts are not vendored here. Each crate ships its own under
`~/.cargo/registry/`, and every project below is linked to its source.

## How this was compiled, and how to re-check it

From `cargo metadata`, filtered to `x86_64-unknown-linux-gnu`, walking the
`normal` and `build` dependency edges from the three workspace crates —
`hani-core`, `hani-gpu` and `hani-app` — and **excluding
`dev-dependencies`**, which are used to test the software and are not
distributed in a build of it. Re-run it whenever `Cargo.lock` changes.

Counts by declared license expression, over those 246 crates (counted on
2026-11-27 with `cargo tree -e normal,build --target x86_64-unknown-linux-gnu
-f '{p}|{l}'` from each workspace crate, `hani-evdev` included, workspace
crates left out):

| Crates | Expression |
| ---: | --- |
| 110 | `MIT OR Apache-2.0` |
| 66 | `MIT` |
| 12 | `Apache-2.0 OR MIT` |
| 9 | `Apache-2.0` |
| 8 | `MIT/Apache-2.0` |
| 8 | `MIT OR Apache-2.0 OR Zlib` |
| 4 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| 3 | `Zlib OR Apache-2.0 OR MIT` |
| 3 | `Zlib` |
| 3 | `BSD-2-Clause` |
| 2 | `Unlicense OR MIT` |
| 2 | `ISC` |
| 2 | `BSD-3-Clause OR Apache-2.0` |
| 2 | `BSD-3-Clause` |
| 2 | `BSD-2-Clause OR Apache-2.0 OR MIT` |
| 2 | `Apache-2.0/MIT` |
| 1 each | `MPL-2.0`, `MIT OR Zlib OR Apache-2.0`, `CC0-1.0`, `Apache-2.0 OR GPL-2.0-only`, `Apache-2.0 AND MIT`, `Apache-2.0 AND ISC`, `0BSD OR MIT OR Apache-2.0`, `(MIT OR Apache-2.0) AND Unicode-3.0` |

## The one line that mentions the GPL, and why it is not a problem

**`self_cell` 1.3.0** — `Apache-2.0 OR GPL-2.0-only` —
<https://github.com/Voultapher/self_cell>. This is a *disjunction*: a
downstream user picks one. **HaniSculpt takes it under Apache-2.0**, so no
GPL-2.0 term applies to this project or to anything built from it. It is
recorded here rather than left for someone to find in a license scan and worry
about. It arrives transitively through the text stack — `iced` → `iced_wgpu` →
`cryoglyph` → `cosmic-text` → `self_cell` — not by direct choice.

**`contracts`** 0.6 (MPL-2.0, <https://gitlab.com/karroffel/contracts>) is
the one crate under a weak copyleft. It is a procedural macro: it runs inside
the compiler, writes the `debug_requires` and `debug_ensures` checks into this
project's own functions, and is not itself linked into the program. MPL-2.0 is
a per-file licence, its obligations attach to its own source files, and those
are neither modified nor distributed here. Earlier counts in this file left it
out because they walked only the crates linked into the binary.

Two other expressions are conjunctions rather than choices, so *both* terms
apply and both are permissive: **`ring`** (`Apache-2.0 AND ISC`) and
**`unicode-ident`** (`(MIT OR Apache-2.0) AND Unicode-3.0`).

## The dependencies that carry the project

Direct dependencies of the workspace, and the few transitive ones large enough
to be worth naming:

### Interface and rendering

- **iced** and **iced_wgpu** 0.14 — MIT —
  <https://github.com/iced-rs/iced>. The application shell and the `shader`
  widget the viewport is drawn into.
- **wgpu** 27 and **naga** — MIT OR Apache-2.0 —
  <https://github.com/gfx-rs/wgpu>. Vulkan is the backend in practice.
- **winit** — Apache-2.0 — <https://github.com/rust-windowing/winit>. Reached
  through iced; the application does not use it directly.
- **cosmic-text** — MIT OR Apache-2.0 —
  <https://github.com/pop-os/cosmic-text>. Text shaping, via iced.
- **tiny-skia** and **tiny-skia-path** — BSD-3-Clause —
  <https://github.com/RazrFalcon/tiny-skia>. On Linux only, where winit's
  Wayland window decorations (`sctk-adwaita`) are drawn with it. iced's own
  software renderer is no longer built.
- **miniz_oxide** — MIT OR Zlib OR Apache-2.0 —
  <https://github.com/Frommi/miniz_oxide>. Inflating the ZIP blocks of an
  OpenEXR picture, the file a vector displacement map comes in. The OpenEXR
  reader itself is this project's own (`hani-app/src/openexr.rs`) since
  2026-11-27. **exr** and **lebe** (BSD-3-Clause,
  <https://github.com/johannesvollmer/exrs>) are now a `dev-dependency` only:
  the reader's tests read what exr writes.
- **arboard** — MIT OR Apache-2.0 — <https://github.com/1Password/arboard>.
  The clipboard, for the snip's picture and the report's text; it brings the
  `image` crate's PNG path and `moxcms` and `pxfm` (BSD-3-Clause OR
  Apache-2.0).
- **bytemuck** — Zlib OR Apache-2.0 OR MIT —
  <https://github.com/Lokathor/bytemuck>. Taken under Apache-2.0. Getting Rust
  structs safely to the GPU as bytes.
- **glam** — MIT OR Apache-2.0 — <https://github.com/bitshifter/glam-rs>.
  Vectors and matrices.

### Engine

- **fast-surface-nets** — MIT OR Apache-2.0 —
  <https://github.com/bonsairobo/fast-surface-nets-rs>. **No longer in a
  build**, since 2026-11-24: it was the isosurface extraction each brick's
  mesh came out of, and is now a `dev-dependency`, what this project's own
  mesher (`hani-core/src/mesh/nets.rs`) is tested against bit for bit. Its
  method and its arithmetic are what that mesher follows, which is why it is
  still named here. It took `ndshape` and a second copy of `glam` out of the
  build with it, which took the count of that day from 312 to 309.
- **rayon** — MIT OR Apache-2.0 — <https://github.com/rayon-rs/rayon>. The
  parallel path in `edit_voxels`.
- **rustc-hash** — Apache-2.0 OR MIT —
  <https://github.com/rust-lang/rustc-hash>. The sparse brick map's hasher.

### Files

- **roxmltree** 0.20 — MIT OR Apache-2.0 —
  <https://github.com/RazrFalcon/roxmltree>. 3MF is XML.
- **yazi** 0.2.1 — Apache-2.0 OR MIT — <https://github.com/dfrg/yazi>. Raw
  DEFLATE; a 3MF is a ZIP and the ZIP container is read by hand.
- **serde_json** — MIT OR Apache-2.0 — <https://github.com/serde-rs/json>.
- **rfd** 0.17.2 — MIT — <https://github.com/PolyMeilex/rfd>. Native file
  dialogs. The version is pinned for dependency cost, not compatibility.

### Devices

- **hani-evdev**, this project's own (`crates/hani-evdev`), over **libc**
  (MIT OR Apache-2.0, <https://github.com/rust-lang/libc>). Stylus pressure
  and tilt, and the SpaceMouse, read below the display server. It replaced
  the `evdev` crate (Apache-2.0 OR MIT, <https://github.com/cmr/evdev>) on
  2026-11-27, and keeps its names for what it covers.
- **octotablet** (MIT, <https://github.com/Fuzzyzilla/octotablet>), at
  commit `1b0a14e`. The pen from the compositor under Wayland: position,
  pressure, tilt, which end and which button, over the tablet protocol. With
  it **enum_dispatch** (MIT OR Apache-2.0), **strum** and **strum_macros**
  (MIT), **heck** and **rustversion** (MIT OR Apache-2.0).

### Network

There is no TLS library in the application. The updater fetches over HTTPS
with the system's own `curl`, run as a program; the printer is spoken to over
plain HTTP on the local network with the standard library; a bug report is a
link opened in the browser. `ureq`, `rustls`, `rustls-webpki` and
`webpki-roots` left the build on 2026-11-27.

- **ring** — Apache-2.0 AND ISC — <https://github.com/briansmith/ring>.
  SHA-256 and signature checks on a downloaded update. Both terms apply; both
  are permissive. Contains code derived from BoringSSL, which carries its own
  permissive notices in the crate's own `LICENSE`. It brings **untrusted**
  (ISC).

### Logging

- **log** and **env_logger** — MIT OR Apache-2.0 — <https://github.com/rust-lang/log>
  and <https://github.com/rust-cli/env_logger>.
- **pollster** — Apache-2.0/MIT — <https://github.com/zesterer/pollster>.

## Assets

**Nothing here is third-party.** Every image and mark in this repository was
made for it, and there are no third-party fonts, icons, textures or sample
models to attribute.

- `docs/images/*.jpg` — screenshots of the application, taken from it.
- `assets/brand/haniko/` — Haniko, HaniSculpt's mascot: the badge, the
  mascot and the lockup, given by the maintainer, and the PNGs
  `scripts/brand-assets.py` makes from them. See `docs/HANIKO.md`.
- `assets/icons/*.svg` — the user-interface icon set, **generated** from
  `crates/hani-app/src/icon.rs`, which is where the drawings live. Original
  work, drawn for this project. They follow the same house style as SindriCAD's
  icons — a 24 by 24 grid, 1.6 stroke, round caps, `currentColor` throughout —
  because that is the same author's set too; no third-party icon library is
  used, vendored or traced.

**No font files are shipped.** The lockups name a font *stack* and fall back
through the system's own faces, so nothing here embeds a typeface. Converting
the wordmark to outlines before it goes anywhere it must render identically is
noted in the files themselves.

The clay matcap the model is shaded with is *generated in code*
(`hani-gpu/src/matcap.rs`) rather than shipped, which keeps the largest asset
the application needs out of the repository entirely — that was the reasoning
before any of the above existed, and it still holds for anything the renderer
consumes.

**The paint index is third-party data.** `assets/paints/index.tsv` is
generated from the tables of
[miniature-paints](https://github.com/Arcturus5404/miniature-paints),
copyright 2022 Rick Fleuren, under the MIT licence, whose text is beside
it as `assets/paints/LICENSE-miniature-paints`. The kinds in it are this
project's own (`scripts/paint_index.py`). The paint names and brands are
the makers' trademarks, used to name their products.
