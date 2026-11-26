<!-- SPDX-License-Identifier: AGPL-3.0-only -->

<p align="center"><img src="../assets/brand/haniko/haniko.png" alt="Haniko" width="240"></p>

# Haniko — Official Mascot Bible for HaniSculpt

## Character Sheet

* **Name:** Haniko (はにこ)
* **Title:** Patron Spirit of Undone Vertices & Happy Accidents
* **Origin:** Kofun-period kiln mound (digitized during a routine 3D photogrammetry scan gone slightly sideways)
* **Height:** 14 cm on a desk; infinitely scalable in viewport units
* **Material:** Unfired terracotta clay with a mild specular sheen and a hollow, non-manifold interior
* **Signature Pose:** One hand planted firmly on hip in quiet judgment; the other raised in a permanent, slightly wobbly wave
* **Expression:** Perpetual wide-eyed, open-mouthed astonishment—whether you just nailed a clean quad topology pass or accidentally subdivided a mesh to 14 million triangles
* **Likes:** Smooth clay buildup brushes, X-axis symmetry, well-baked normal maps, warm ambient occlusion, and frequent auto-saves
* **Dislikes:** Pinched poles, stray floating vertices, forgetting to save before boolean operations, and flat shading on organic curves

---

## Backstory

Fifteen hundred years ago, a sleepy apprentice potter in the Kofun period shaped Haniko out of river clay to stand guard around a royal burial mound. Unlike the stoic warriors and noble horses fired beside him, Haniko came out of the kiln with uneven arms, stubby little feet, and an expression of existential surprise—so the potter tucked him behind a pine tree and forgot him.

Centuries later, a digital preservation team set up a laser scanner to archive the mound's artifacts. A stray sunbeam, a glitch in the point-cloud solver, and a sudden breeze through the trees pulled Haniko's hollow silhouette straight into the mesh buffer. Instead of resting quietly in a museum archive, he wandered across the local network and took up residence inside **HaniSculpt**.

Now, Haniko lives in the corner of the viewport as an unblinking studio companion. Because he remembers what it feels like to be pinched, rolled, and smoothed by real thumbs, he treats every digital brush stroke like genuine pottery.

---

## In-Character UI Lines & Micro-Copy

*Note: Haniko communicates through subtle body language, expressive stares, and brief, dryly enthusiastic studio notes.*

### Empty Canvas / New Project
* *"A fresh lump of digital clay! Try not to poke a hole straight through the back this time."*
* *"I have stood completely still for 1,500 years. You can definitely survive blocking out the primary shapes before adding pores."*
* *"Step 1: Add sphere. Step 2: Squish it until it looks like a potato. Step 3: Trust the process."*

### Loading & Remeshing Screens
* *"Kneading the polygons... please keep your hands inside the viewport."*
* *"Smoothing out the lumps—unlike my apprentice potter, I actually care about even density."*
* *"Rebuilding topology. Hold still, this tickles."*
* *"Baking ambient occlusion into all the hard-to-reach crevices..."*

### Helpful Tips & Symmetry Nudges
* *"That left ear is looking great. Too bad symmetry is turned off and the right side is still a smooth egg."*
* *"Zoom out! If the silhouette doesn't read from across the room, tiny wrinkles won't save it."*
* *"Ctrl+Z is a gift from the kiln gods. Use it freely."*
* *"Non-manifold geometry detected. Even I'm hollow on the inside, and I wouldn't try to 3D print that."*

### High-Poly Warnings & Crash Recovery
* *"18 million triangles?! My terracotta heart can only take so much."*
* *"Whoa—that boolean operation collapsed the timeline. Good thing I stashed a backup mesh behind the pine tree."*
* *"Oops, we dropped the clay on the floor. Restoring your last auto-save..."*

### Export & Save Confirmations
* *"Fired and glazed! Your mesh is ready for the outside world."*
* *"Saved safely. Now go stretch your wrists—mine are stuck like this permanently."*
## The pictures

The originals are in `assets/brand/haniko/` as JPEGs: `haniko-badge.jpg` (the app's icon), `haniko.jpg` and `hanisculpt-lockup.jpg` (the welcome screen). `scripts/brand-assets.py` makes the PNGs beside them: the icon at five sizes with everything outside the ring transparent, Haniko and the lockup with the background transparent, the lockup's whole name in the orange of SCULPT so it reads on light and dark alike.
