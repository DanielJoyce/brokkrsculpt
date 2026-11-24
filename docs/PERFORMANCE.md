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

### A stroke for a profiler

```bash
cargo build --profile bench -p brokkr-core --example stroke_profile
target/release/examples/stroke_profile draw 48 2
RAYON_NUM_THREADS=1 valgrind --tool=callgrind target/release/examples/stroke_profile draw 48 1
```

A ring of stamps round a ball of 4610 dense bricks with a remesh after each:
the brush, its radius in voxels, and how many times round. It prints
milliseconds a stamp for the edit and for the remesh. The benches say how long
a stamp takes and not where in it the time goes, and this is the smallest
program that spends its time the way a stroke does, so that a profiler has
something to look at. Everything in the core's section below was found with
it under callgrind, which counts instructions and not time: a count is exact
and repeatable where a time on this machine is not, and it says nothing of
cache misses.

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

### The core: the mesher, the sample, and the voxels that cannot change

One stage, found by profiling `stroke_profile draw 48` on one core. Before any
of it the instructions of that stroke were, in parts of a hundred: the
`fast-surface-nets` crate 22, the trilinear sample 21, a software `floor`
called from it 7, the brush's own arithmetic 28. After, the stroke is 7.9
thousand million instructions where it was 14.4.

`stroke_profile`, milliseconds a stamp, before and after, radius in voxels.
Runs of the same code differ by a tenth or more here; where two runs were
made both are given.

| | edit | remesh | bricks remeshed a stamp |
|---|---|---|---|
| Draw 48, four cores | 6.1 and 6.3 to 4.1 and 4.5 | 2.3 and 3.0 to 2.0 and 2.0 | 68.7 to 65.2 |
| Pinch 48 | 7.3 to 4.0 | 3.0 to 1.7 | 68.7 to 65.7 |
| Smooth 48 | 2.6 to 2.3 | 2.0 to 1.8 | 68.7 to 65.7 |
| Inflate 48 | 2.2 to 2.1 | 3.0 to 1.8 | 68.7 to 66.7 |
| Clay 48 | 1.9 to 2.1 | 2.9 to 1.8 | 68.7 to 66.1 |
| Draw 128 | 59.7 to 39.8 | 13.3 to 7.9 | 735.8 to 392.6 |
| Draw 12 | 0.47 to 0.47 | 1.02 to 1.04 | 6.3 and 6.3 |
| Draw 48, one core | 19.8 to 13.1 | 6.9 to 4.2 | |
| Smooth 48, one core | 7.9 to 6.2 | 6.9 to 3.7 | |
| Draw 12, one core | 0.46 to 0.62 | 1.16 to 0.81 | |

Inflate and Clay did not get faster to edit, and were not expected to: they
read nothing but the voxel they write. The small brush did not either, in
either direction by more than the noise (its one-core edit read 0.44 on
another run): it is below the size the voxel skip is switched on at. What it
pays for is the remesh, and of that the surface nets are now a small part and
gathering each brick's apron is the larger.

**The mesher** (`brokkr-core/src/mesh/nets.rs`). The crate read the eight
corners of every one of a brick's 35,937 cells to find the thousand or two the
surface is in. This finds them first: one pass makes a row of sign bits for
each line of the apron (four samples at a time on x86-64), and the cells of a
line with a sign change in them are four rows and a few shifts. What it makes
is what the crate makes, bit for bit and in the same order, and tests hold it
to the crate on balls, planes at any angle, noise, noughts of both signs, and
aprons the surface only grazes. The crate is a dev-dependency now and three
crates fewer are in a build.

**The sample** (`brokkr-core/src/region.rs`). On x86-64 before SSE4.1, which
is what a release is built for, `floor` is a call into the compiler's
arithmetic library, and the trilinear read made three for every voxel. It is
now a cast and a correction, with `floor`'s answer to the bit, the sign of a
nought included, which twenty thousand random bit patterns and the edge cases
are held to.

**The voxels that cannot change** (`brokkr-core/src/volume/settled.rs`). A
stamp has always skipped a brick that holds one value throughout, where the
brush leaves that value alone. A brick the surface passes through holds the
band, six voxels of thirty-two, and the same two values either side of it. So
the claim is carried down to the voxel: one that holds an end of the band,
with nothing else within the brush's reach of it, is not visited. Found with a
bit a voxel and shifts, for a few thousand operations a brick. With the skip
on and off, every classic brush, every operator of the brush engine and the
compound brushes write the same field, the same undo entry and leave the same
bricks to remesh.

**Only what changed is remeshed** (issue 53). A stamp marked its whole box
dirty. It marks the bricks a voxel changed in, each with the neighbours whose
aprons read it. For a brush three bricks across that is 65 bricks where it was
69, because nearly every brick of so small a box changes; at 128 voxels it is
393 where it was 736.

And through the application, `what_a_stroke_costs` on 4610 dense bricks,
median milliseconds a frame: as it was before any of this work, after the
frame's path was put right (the three sections above), and after this.

| | at the start | after the frame's path | after the core |
|---|---|---|---|
| The ball it opens with, 3 mm brush | 1.2 | 1.1 | 0.9 |
| 3 mm brush, two events a frame | 9.2 | 8.3 | 5.0 |
| of which the edit | 6.0 | 5.6 | 3.3 |
| of which the remesh | 2.6 | 2.2 | 1.4 |
| 3 mm brush, eight events a frame | 29.1 | 20.1 | 12.5 |
| 8 mm brush, on the frames that stamp (95th) | 59.5 | 48.9 | 30.1 |
| Bricks meshed in the 8 mm stroke | 16681 | 16681 | 11105 |

Tried and dropped:

- **Not promoting a brick until a voxel changes** (issue 55). An edit begins
  on each brick it visits by copying it for undo and, where it holds one
  value, giving it 128 KB, and takes both back if nothing changed. Doing
  neither until the first change was built two ways. The first asked in one
  loop and wrote in another, and the brush's arithmetic, now called from three
  places, was no longer built into any of them: a Draw stamp went from 18.9 to
  28.8 ms on one core. The second was one loop and no better (29.4). And the
  saving it was for was not there: the 128 KB fills that a profile of a small
  brush showed at thirteen parts in a hundred were the fixture's ball being
  made, not the stamps. Reverted.
- **The newer instruction set** (issue 51). The ticket measured a tenth off
  every stamp and a sixth off the brushes that resample from building for
  x86-64-v3. Most of that was `floor` becoming an instruction. With the
  software `floor` gone, the same comparison on `stroke_profile` is 12.7
  against 13.1 ms for Draw on one core and 5.8 against 6.2 for Smooth, single
  runs, inside the noise on four cores. Two downloads or a dispatch at run time
  are not worth that, and neither was built.

Not done in the core:

- **The brush loop by rows** (issue 52). The brush's arithmetic is still
  called a voxel at a time, and with the mesher and the `floor` gone it is
  what is left: about half of a Draw stroke's instructions between it and the
  sample. Rewriting every brush's closure to work a row at a time is the next
  thing, and the bit-for-bit tests are what would hold it.
- **A sampler that remembers its brick** (issue 54), for the picks and the
  inserts. The picks were half a part in a hundred of a stroke.
- **The apron gather**, which is now the larger part of a small brush's
  remesh.

### Studio gestures set down once a frame (issue 60)

A shape, a body, a map, a hung cable and a single stamp are each set down
whole from where the press was and where the pointer is now: every move put
the last one back and joined again. With moves handled together (issue 48)
only the last of a frame is worth that, and the others are now passed over.
A cable laid along the model is made of every point the pointer passed and
still takes every move, as does the Move brush, which re-anchors as it goes.

The table a replay prints and the phase log now count how many times a
gesture was set down again.

`what_setting_a_shape_down_costs`: a ball dragged out over 48 moves, on the
opening ball at half the voxel size, single runs.

| | times set down | edit over the whole drag | a frame, median |
|---|---|---|---|
| A move to a frame | 48 | 2508 ms | 56 ms |
| Eight moves to a frame | 6 | 383 ms | 71 ms |

Before this the eight-to-a-frame drag set the ball down 48 times too, eight
in each frame: by the first row's figures about 400 ms a frame, which was not
measured as such.

**What that table also says is that one set-down is 50 to 110 ms here**, at
any rate of moves. That is the gesture itself: the body put back as it was,
the shape joined, and the soft join's fillet, which is on by default at three
voxels. It is the next thing to look at in the studio's own code, and it is
not done.

Not done: the gestures are timed whole, as part of `edit`, and not broken
down into the before-state put back, the join and the wide join, which is
what would say where those 50 to 110 ms go.

## The renderer: looked at, and not built

Nothing below was changed, because nothing below can be judged on a machine
that draws on its CPU. The replay is there to judge it with.

- **Redraw on change** (issue 50). The window redraws every frame because the
  frame tick is also the application's clock: it is what polls the SpaceMouse
  and the pen, advances a camera flight and the timeline, begins and advances
  the autosave, and asks for thumbnails. iced's timer is not in the feature
  set this is built with, which upstream's own comment on the autosave says.
  Stopping the frames when nothing has changed needs each of those to wake the
  application itself first. That is a piece of work in its own right, and
  what it buys is power and heat on an idle window, not speed.
- **Fewer draw calls** (issue 56). The pool draws a brick a call. Drawing many
  in one needs either a device feature iced does not ask for, or runs of
  bricks that are neighbours in the index buffer, with the padding between
  them made harmless. Upstream measured 1.13 ms for 5435 bricks; the ticket's
  9.5 ms at the pool's design size is that figure multiplied, not measured.
- **Smaller vertices and 16-bit indices** (issue 57). Indices are made
  absolute when a brick is uploaded, so one has to hold a position in the
  whole pool and cannot be 16 bits until that is undone. Why they are absolute
  is not written down in the pool; the comment that pointed at a reason now
  says so.
- **Overdraw** (issue 58). Back faces are drawn on purpose: the pipeline's
  comment is that sculpting means looking into cavities and through openings
  that are mid cut, and that culling belongs with a mesh that is reliably
  closed. Left as it is.
- **The stamp and the remesh as GPU compute** (issue 59). Not attempted. A
  compute shader would run here, on the CPU, and say nothing about a graphics
  card. The case for it is also weaker than it was: on one core a Draw stamp
  at 48 voxels takes a third less time and its remesh two fifths less, and
  what is left in the stamp
  is the brush's arithmetic a voxel at a time, which issue 52 would take
  further on the CPU with the bit-for-bit tests still holding. A compute path
  cannot keep those, and what it may differ by is still the maintainer's to
  decide.
