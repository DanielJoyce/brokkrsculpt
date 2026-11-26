#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Make HaniSculpt's brand pictures from the three originals.

    python3 scripts/brand-assets.py

Reads `assets/brand/haniko/*.jpg` and writes beside them:

* `hanisculpt-icon-{512,256,128,64,32}.png`: the badge, Haniko in a ring,
  with everything outside the ring transparent and the cream inside kept.
  The application's icon.
* `haniko.png`: Haniko standing, the background transparent.
* `hanisculpt-lockup.png`: Haniko and the name, the background transparent,
  cropped to what is drawn, and the whole name in the orange of SCULPT so
  that it reads on light and dark alike (the original's HANI is near black).

The originals are JPEGs on an off-white ground. Everything in them is
outlined in black, so the ground only ever meets black: a pixel is ground
when it is within a small distance of the ground's colour, and the pixels
just beside the ground are a blend of black and ground, whose opacity is how
much darker than the ground they are.
"""

from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent.parent / "assets" / "brand" / "haniko"

# How far from the ground's colour a pixel may be and still be ground, in
# 0..255 per channel (Euclidean). JPEG noise on the originals is under 20.
GROUND = 40.0


def ground_colour(rgb: np.ndarray) -> np.ndarray:
    """The ground's colour: the median of the four edges."""
    edges = np.concatenate([rgb[0], rgb[-1], rgb[:, 0], rgb[:, -1]])
    return np.median(edges, axis=0)


def keyed(path: Path, outside_only: bool) -> Image.Image:
    """`path` with its ground made transparent.

    `outside_only` keeps ground that is enclosed (the inside of the badge's
    ring) and takes only what is connected to the picture's edge.
    """
    rgb = np.asarray(Image.open(path).convert("RGB")).astype(np.float32)
    bg = ground_colour(rgb)
    ground = np.linalg.norm(rgb - bg, axis=2) < GROUND
    if outside_only:
        # A copy: an image made over a NumPy buffer is read only, and the
        # fill would land nowhere.
        mask = Image.fromarray(np.where(ground, 255, 0).astype(np.uint8)).copy()
        for corner in [(0, 0), (mask.width - 1, 0), (0, mask.height - 1),
                       (mask.width - 1, mask.height - 1)]:
            ImageDraw.floodfill(mask, corner, 128)
        ground = np.asarray(mask) == 128

    # The blend band: two pixels either side of the ground's edge.
    near = np.asarray(
        Image.fromarray(np.where(ground, 255, 0).astype(np.uint8)).filter(ImageFilter.MaxFilter(5))
    ) > 0
    band = near & ~ground

    alpha = np.ones(ground.shape, np.float32)
    alpha[ground] = 0.0
    # Black over ground: opacity is how much darker than the ground it is.
    lum = rgb.mean(axis=2)
    band_alpha = np.clip(1.0 - lum / bg.mean(), 0.0, 1.0)
    alpha[band] = band_alpha[band]

    out = rgb.copy()
    # Unblend the band from the ground, so its colour is the ink's.
    a = np.maximum(alpha[..., None], 1e-3)
    unblended = np.clip((rgb - (1.0 - a) * bg) / a, 0, 255)
    out[band] = unblended[band]
    out[ground] = 0
    rgba = np.dstack([out, alpha * 255.0]).round().astype(np.uint8)
    return Image.fromarray(rgba, "RGBA")


def cropped(image: Image.Image, margin: int) -> Image.Image:
    """Cropped to what is not transparent, with `margin` pixels round it."""
    left, top, right, bottom = image.getchannel("A").getbbox()
    return image.crop((max(left - margin, 0), max(top - margin, 0),
                       min(right + margin, image.width), min(bottom + margin, image.height)))


def orange_lettered(path: Path) -> Image.Image:
    """The lockup keyed, with every letter in the orange of SCULPT.

    The original's HANI is near black, which vanishes on a dark background;
    in one orange the lockup reads on light and dark alike. Haniko is keyed
    as the other pictures are. The letters are found right of the widest
    empty run of columns between him and them, and have no black outline,
    so each letter pixel's opacity is how far it is from the ground toward
    whichever ink it is nearer, black or orange, and its colour is the orange.
    """
    image = keyed(path, outside_only=False)
    rgb = np.asarray(Image.open(path).convert("RGB")).astype(np.float32)
    bg = ground_colour(rgb)
    ground = np.linalg.norm(rgb - bg, axis=2) < GROUND
    width = rgb.shape[1]

    # The gap between Haniko and the letters: the longest run of empty
    # columns in the middle of the picture, over the rows the letters take
    # (those with ink in the right fifth). Over every row there is none:
    # Haniko's raised arm overhangs the H.
    rows = (~ground[:, width * 4 // 5 :]).any(axis=1)
    empty = ~(~ground[rows]).any(axis=0)
    best, run, split = 0, 0, width // 2
    for x in range(width // 5, width * 3 // 5):
        run = run + 1 if empty[x] else 0
        if run > best:
            best, split = run, x - run // 2

    # The letters' rows, with a few pixels for their soft edges: Haniko's
    # arm is above them and keeps its own colours.
    found = np.flatnonzero(rows)
    top, bottom = max(found[0] - 6, 0), min(found[-1] + 7, rgb.shape[0])
    letters = rgb[top:bottom, split:]
    orange_pixels = letters[(letters[..., 0] - letters[..., 2] > 120) & (letters[..., 0] > 180)]
    orange = np.median(orange_pixels, axis=0)
    dark = np.array([30.0, 20.0, 20.0])

    def toward(ink: np.ndarray) -> np.ndarray:
        line = bg - ink
        return np.clip(((bg - letters) @ line) / (line @ line), 0.0, 1.0)

    alpha = np.maximum(toward(orange), toward(dark))
    alpha[ground[top:bottom, split:]] = 0.0
    out = np.asarray(image).copy()
    out[top:bottom, split:, :3] = orange.round().astype(np.uint8)
    out[top:bottom, split:, 3] = (alpha * 255.0).round().astype(np.uint8)
    return Image.fromarray(out, "RGBA")


def main() -> None:
    badge = cropped(keyed(HERE / "haniko-badge.jpg", outside_only=True), 8)
    side = max(badge.size)
    square = Image.new("RGBA", (side, side))
    square.paste(badge, ((side - badge.width) // 2, (side - badge.height) // 2))
    for size in (512, 256, 128, 64, 32):
        square.resize((size, size), Image.LANCZOS).save(HERE / f"hanisculpt-icon-{size}.png")

    standing = cropped(keyed(HERE / "haniko.jpg", outside_only=False), 16)
    standing.thumbnail((1024, 1024), Image.LANCZOS)
    standing.save(HERE / "haniko.png")

    lockup = cropped(orange_lettered(HERE / "hanisculpt-lockup.jpg"), 24)
    lockup.thumbnail((1200, 1200), Image.LANCZOS)
    lockup.save(HERE / "hanisculpt-lockup.png")

if __name__ == "__main__":
    main()
