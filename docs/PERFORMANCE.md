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

Added to as the tickets are worked through. Runs of the same code on this
machine differ by up to about fifteen parts in a hundred (the edit row below
moved by that much between two runs with no change to the edit), so a
difference smaller than that is not a result.

### The autosave, off the thread that draws (issue 49)

The autosave was one call on the thread that draws. Upstream's own figure was
1.55 GB/s: 100 ms for a 159 MB model and about 430 ms for a 671 MB one, every
two minutes.

It is now encoded a piece of about 4 MB between two frames and written by
another thread (`app/autosave.rs`, `brokkr-core/src/project/pieces.rs`). A
write in pieces is given up the moment the document is wanted, and begun again
at the next pause.

`what_an_autosave_costs`, a 327 MB document:

| | on the thread that draws |
|---|---|
| The whole write in one call, as it was called | 193 ms in one frame |
| In 83 pieces | 0.87 ms a frame, 2.45 ms at the worst, 72 ms in all |

The one call above is of the new encoder, which copies a dense brick's bytes
whole where the old one wrote them a value at a time. The old one was not
timed here. *Save* and *Save as* are still one call, and are faster by
whatever that is worth; they also read the file back to check it, which this
did not change.

### Pointer moves handled once a frame (issue 48)

Moves are kept as they arrive and handled together before the next frame:
every move's stamps, in order, then one remesh, one pick for the ring and one
ring. Anything else that arrives first (a release, a key) handles them first,
so nothing is reordered.

`what_a_stroke_costs`, 4610 dense bricks, 3 mm brush, median milliseconds a
frame, before and after:

| | mesh | ring | whole frame | bricks meshed in the stroke |
|---|---|---|---|---|
| Two events a frame | 2.6 to 2.2 | 0.32 to 0.15 | 9.2 to 8.3 | 5932 and 5932 |
| Eight events a frame | 7.6 to 2.7 | 1.15 to 0.17 | 29.1 to 20.1 | 5932 to 2653 |

At two events a frame the ring is the only row that moved by more than the
noise: most moves of that recording lay no stamp, so there is seldom a second
remesh in a frame to save. At eight, the remesh is done once for what eight
moves dirtied, and the bricks meshed over the stroke fall by more than half.

The upload cap the ticket also asked for was not built. In the replay run in
the application, a stroke's frames uploaded 8 bricks and 0.4 MB each in 0.2 to
0.7 ms (on llvmpipe, where an upload is a copy). The frames that upload
hundreds of megabytes are the ones after an open or a resample, which have
just spent seconds meshing on the same thread. A cap would spread those and
would have to keep the renderer's forget and reset ordering right across
frames, for a hitch nobody has reported.

### No walk of the whole document per pointer event (issue 47)

After every remesh the document's totals and the active body's radius were
taken again, and each is a walk of every brick. They are now taken when a
stroke ends, and at once for anything that is not a stroke. The `after` row
of the table went from 0.17 ms a frame on 4610 bricks to nothing. It grows
with the document and not with the brush, so it was worth more than that
figure on a larger model, which was not measured. The readout behind the Info
toggle shows a stroke's brick counts when the stroke ends and not during it.
