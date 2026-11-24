# Performance: how it is measured, and what was found

The survey that began this is in the tracking ticket,
[DanielJoyce/brokkrsculpt#62](https://github.com/DanielJoyce/brokkrsculpt/issues/62),
and the tickets it links. This document is where the instruments are
described and where each change's before and after is written down.

On whose files these are: the plan's decision D6 is to add and not modify
upstream's files. The maintainer set that aside for this work on 2026-11-24
(*"The code is ours, so do what needs to be done"*). New instruments are
still modules of their own; the changes they justify are made where the cost
is.

## What every number here was measured on

Unless a row says otherwise: a shared virtual machine with 4 cores of a
2.1 GHz Xeon, no graphics card, drawing on llvmpipe (a CPU rasteriser). It is
several times slower than the machine upstream's README figures come from, and
it is noisy, so a figure here is for comparing with the figure beside it and
not with anything else. **Nothing in this document was measured on a graphics
card.** The replay below is how that is to be done.

## The instruments

### Phases in the Info readout

The readout behind the Info toggle has two lines of phase timings: the last
frame, and the worst of each phase over the last second. `app/phases.rs`.

| Phase | What is in it |
|---|---|
| pick | Ray marches: the surface under the pointer, for a stroke, for the ring, for a choice of body |
| edit | Writing the field: the stamps of the frame's pointer events, or a studio gesture |
| mesh | Meshing the bricks an edit dirtied and handing them to the renderer |
| after | What is worked out after a remesh: the document's totals and its radius |
| ring | The ring and the other lines drawn over the model |
| end | Ending a stroke: its undo entry |
| view | Building the interface |
| upload | The renderer's `prepare`: the frame's meshes into the pool, with how many bricks and bytes |
| draw | The renderer encoding its passes: culling and draw calls. The CPU's side only |

A phase's time is its own. An edit asks for a pick and ends in a remesh, and
each is charged to its own column, so a frame's columns add up to the time
spent.

How long the graphics card then takes over a frame is not among them. iced
makes the device with no optional features, so there is no timestamp query to
ask. The interval between frames is the evidence for that, and it is in the
replay's table.

### A line a frame

```bash
BROKKR_PHASE_LOG=phases.csv brokkrsculpt
```

One comma-separated line for every frame a stroke is down in, with a heading
line. For reading a hitch back afterwards.

### A stroke recorded and replayed

```bash
BROKKR_RECORD=stroke.txt brokkrsculpt
```

Sculpt, then quit. The file holds the viewport's pointer events, where the
frames fell between them, and at each press of the left button the camera and
the brush in hand by name, size and strength. It is lines of text and can be
trimmed by hand.

```bash
BROKKR_REPLAY=stroke.txt BROKKR_REPLAY_PROJECT=model.brokkr brokkrsculpt
```

Opens the project (or the ball, with no project named), waits ninety frames
for the model to be meshed and drawn, plays the recording a frame's worth of
events a frame, prints a table by phase with the adapter's name, and quits.

Not in a recording: the keyboard, the panel, the pen's pressure, and the
document. A brush with settings of its own that is not on the palette comes
back as the brush it goes by. `app/replay.rs`.

**This is the measurement to run on a machine with a graphics card**, before
and after any of the renderer's tickets.

### A stroke with no window

```bash
cargo test --release -p brokkr-app -- --ignored --nocapture what_a_stroke_costs
```

A ring drawn on the ball the application opens with, and on the same ball at a
quarter of the voxel size (4610 dense bricks), through the application's own
pointer path. No renderer is involved, so the upload and draw rows are empty.

### The core's own

`cargo bench -p brokkr-core --bench budget` times stamps and the remesh on one
fixture against budgets set on upstream's machine, and exits non-zero over
budget, which on this machine it does. `cargo bench -p brokkr-gpu` times the
sculpt pass on a generated ball.

## Where a stroke's time goes, before any change

`what_a_stroke_costs` at the commit that added it. 242 pointer events, two to a
frame except where said. Milliseconds a frame, median.

| | pick | edit | mesh | after | ring | whole frame | 95th |
|---|---|---|---|---|---|---|---|
| The ball it opens with (320 dense bricks), 3 mm brush | 0.02 | 0.35 | 0.64 | 0.003 | 0.17 | 1.2 | 1.7 |
| A quarter the voxel (4610 dense bricks), 3 mm brush | 0.03 | 6.0 | 2.6 | 0.17 | 0.32 | 9.2 | 11.1 |
| The same, eight events a frame | 0.11 | 19.7 | 7.6 | 0.46 | 1.15 | 29.1 | 34.1 |
| The same, 8 mm brush, on the frames that stamp (95th) | 0.04 | 47.7 | 11.2 | 0.24 | 0.79 | 59.5 | 59.5 |

What it says:

- **The edit and the remesh are the stroke.** Between them, 93 to 96 parts in
  a hundred of the time on the larger model.
- **The picks are not.** Half a part in a hundred. Two ray marches a move was
  one of the survey's findings and is true, and is not where the time is.
- **Eight events a frame cost eight frames' work.** Nothing is saved by their
  arriving together: each is stamped, remeshed and ringed on its own.
- 5932 bricks were meshed for 86 stamps, 69 a stamp, for a brush three bricks
  across.

## What each change did

Added to as the tickets are worked through.
