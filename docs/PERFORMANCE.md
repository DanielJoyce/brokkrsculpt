# Performance: how it is measured, and what was found

The survey that began this is in the tracking ticket,
[DanielJoyce/hanisculpt#62](https://github.com/DanielJoyce/hanisculpt/issues/62),
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
HANI_PHASE_LOG=phases.csv hanisculpt
```

One comma-separated line for every frame a stroke is down in, with a heading
line. For reading a hitch back afterwards.

### A stroke recorded and replayed

```bash
HANI_RECORD=stroke.txt hanisculpt
```

Sculpt, then quit. The file holds the viewport's pointer events, where the
frames fell between them, and at each press of the left button the camera and
the brush in hand by name, size and strength. It is lines of text and can be
trimmed by hand.

```bash
HANI_REPLAY=stroke.txt HANI_REPLAY_PROJECT=model.hani hanisculpt
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
cargo test --release -p hani-app -- --ignored --nocapture what_a_stroke_costs
```

A ring drawn on the ball the application opens with, and on the same ball at a
quarter of the voxel size (4610 dense bricks), through the application's own
pointer path. No renderer is involved, so the upload and draw rows are empty.

### The core's own

`cargo bench -p hani-core --bench budget` times stamps and the remesh on one
fixture against budgets set on upstream's machine, and exits non-zero over
budget, which on this machine it does. `cargo bench -p hani-gpu` times the
sculpt pass on a generated ball.

### A paint stamp for a profiler

```bash
cargo run --release -p hani-core --example paint_profile
```

Paint stamps of 2, 5 and 10 mm round a ball at a sixteenth of a millimetre,
each remeshed: milliseconds a stamp for the stamp and for the remesh, and
how many bricks were remeshed.

### A stroke for a profiler

```bash
cargo build --profile bench -p hani-core --example stroke_profile
target/release/examples/stroke_profile draw 48 2
RAYON_NUM_THREADS=1 valgrind --tool=callgrind target/release/examples/stroke_profile draw 48 1
```

A ring of stamps round a ball of 4610 dense bricks with a remesh after each:
the brush, its radius in voxels, and how many times round. The brush is one
of upstream's six or one of the studio's by the name the palette has it
under (gather, wear, ridge, scrape, fill, polish, crease, bead, band). It
prints milliseconds a stamp for the edit and for the remesh. The benches say how long
a stamp takes and not where in it the time goes, and this is the smallest
program that spends its time the way a stroke does, so that a profiler has
something to look at. Everything in the core's section below was found with
it under callgrind, which counts instructions and not time: a count is exact
and repeatable where a time on this machine is not, and it says nothing of
cache misses.

### A shape dragged out, for a profiler

```bash
cargo build --profile bench -p hani-core --example insert_profile
target/release/examples/insert_profile ball 40 3 2
RAYON_NUM_THREADS=1 valgrind --tool=callgrind target/release/examples/insert_profile ball 40 3 1
```

What a brush that adds does on every move of the pointer: the body put back
as it was at the press and the shape joined to it at the size the drag now
gives it, 48 moves to a drag, with a remesh after each. The form (ball, box,
cylinder, cone, ring), how many voxels from its middle to its edge at the
end of the drag, how many voxels soft the join is, how many drags, and
`cut` to take the shape out. It prints milliseconds a move, and a sum of the
field about the shape as the last move left it: the same sum before and
after a change to the join is the same field, to the bit.

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
another thread (`app/autosave.rs`, `hani-core/src/project/pieces.rs`). A
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

**The mesher** (`hani-core/src/mesh/nets.rs`). The crate read the eight
corners of every one of a brick's 35,937 cells to find the thousand or two the
surface is in. This finds them first: one pass makes a row of sign bits for
each line of the apron (four samples at a time on x86-64), and the cells of a
line with a sign change in them are four rows and a few shifts. What it makes
is what the crate makes, bit for bit and in the same order, and tests hold it
to the crate on balls, planes at any angle, noise, noughts of both signs, and
aprons the surface only grazes. The crate is a dev-dependency now and three
crates fewer are in a build.

**The sample** (`hani-core/src/region.rs`). On x86-64 before SSE4.1, which
is what a release is built for, `floor` is a call into the compiler's
arithmetic library, and the trilinear read made three for every voxel. It is
now a cast and a correction, with `floor`'s answer to the bit, the sign of a
nought included, which twenty thousand random bit patterns and the edge cases
are held to.

**The voxels that cannot change** (`hani-core/src/volume/settled.rs`). A
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

### A shape's set-down, the apron, and the brush's ball

The 50 to 110 ms of one set-down, profiled
(`what_setting_a_shape_down_costs` under callgrind, on one core). Of the
gesture's instructions: putting the body back as it was, a third; asking the
shape how far it is from every voxel near it, a quarter; the soft join's wide
part, a fifth; the remesh, an eighth.

- **The body is put back by the brick** (`Volume::restore_before`). It was a
  copy of the whole box as it was before the stroke and then every voxel of
  the box compared with the copy. The stroke's own record holds each brick it
  has changed, as it was, and only those: each is copied back.
- **The shape is asked of a block before it is asked of each voxel**
  (`Join::add_narrow`, `Volume::edit_voxels_leaving`). A brick near the shape
  is taken as 512 blocks of four voxels a side, and a block whose middle puts
  all of it out of the join's reach is passed over. Only for a solid that
  says its distance is steady, which the shapes, a cable's runs and a lent
  body are and a draped map is not.
- **The wide join reads the body by the line** where it asked for each voxel
  by its coordinate.

A ball dragged out over 48 moves, one to a frame, on the opening ball at half
the voxel size:

| | edit over the drag | a set-down, median |
|---|---|---|
| Before | 2508 ms | 50.6 ms |
| The body put back by the brick | 1809 ms | 36.2 ms |
| And the shape asked of blocks, the wide join by lines | 1327 ms | 26.6 ms |

Two more, in the stroke of every brush:

- **The apron is gathered by strides** (`Volume::gather_apron`). An apron is
  3468 lines from 27 bricks, most of them one sample long, and each line
  worked out what kind of brick it came from and where it went. Both are now
  settled once for each brick's part. It had as many instructions as the
  surface nets it feeds.
- **A line of voxels is narrowed to the brush's ball** before the brush is
  asked anything (`Ball` in `volume/settled.rs`). Every brush begins by
  asking whether the voxel is in its ball, and of the cube round a ball nearly
  half is not. The narrowing is a voxel and a half wider than the ball and in
  `f64`, so that which side of the ball's skin a voxel is on is still the
  brush's to say.

`stroke_profile` on one core, milliseconds a stamp, from the table above to
now: Draw at 48 voxels, edit 13.1 to 11.3 and remesh 4.2 to 3.0; Smooth at 48,
edit 6.2 to 5.5 and remesh 3.7 to 2.9; Draw at 12, remesh 0.81 to 0.60, edit
as it was. On four cores the differences are inside the noise of single runs.

Tests: 160 random joins (every form, a cable's run, a lent body, each in a
mirror, added and cut, hard and soft) leave the same field and the same undo
with blocks and with every voxel asked; a span never leaves out a voxel a
brush's own test would let in, for 400 random balls at six voxel sizes; and
the tests that run every brush with skipping on and off now hold the ball
too.

Not done: a set-down was not profiled again after these, so what its 27 ms
are now made of is not known; by the first profile's shares it should be the
wide join's spreading of the distance. And the brush loop by rows (issue 52).

### The Draw brush, a run of voxels at a time (issue 52, begun)

With the mesher and the library's `floor` out of the way, about half of a
Draw stroke's instructions were the brush's arithmetic, asked about one voxel
at a time, and the trilinear read it ends in.

An edit can now be offered a run of voxels along X (`VoxelEdit::run`,
`Volume::edit_voxels_by_runs`): the voxels of a line that are all to be
visited, under one freedom from the mask. The Draw brush takes them four at a
time (`brush/runs.rs`), with the operations every x86-64 has: how far each is
from the brush's middle, the falloff of that, where to read from, which cell
that is and how far across it, and the seven blends of the cell's eight
corners. Fetching the corners is still a voxel at a time, as four pairs of
neighbours.

**What a run writes is what the closure writes, to the bit.** Every operation
is the closure's, in the closure's order, done to four numbers where the
closure does it to one. Nothing is reordered and nothing is fused. With the
brush's shortcuts off no run is offered, so the tests that run every brush
both ways compare the two, and so do the tests that hold the brush engine's
Draw to the classic one, since the engine is still a voxel at a time.

What was tried on the way, by instructions of `stroke_profile draw 48` on one
core, in thousands of millions:

| | |
|---|---|
| Before | 7.08 |
| The loop reshaped to offer runs, and nothing taking them | 7.18 |
| The weight four at a time, the read as it was | 6.14 |
| And the read's blends four at a time, the corners fetched through `cell` | 6.10 |
| And the corners fetched as pairs | 5.22 |

The fourth row is the one to notice: making the blends wide and then filling
their eight inputs one number at a time through a function that returns an
array cost as much as it saved. Reading each pair of neighbours as one and
dealing the four voxels' pairs out is what made it pay.

Milliseconds a stamp, `stroke_profile`, runs of the same session, since the
machine was a tenth slower that day than when the table above was made: Draw
at 48 voxels on one core, edit about 12.5 to 8.0 and 8.5. On four cores Draw
at 48 is 2.8 where it has been 4.1 to 4.8, and Draw at 128 is 23.6 where it
was 36.8, and 59.7 at the start.

Not done then, and done since (the next section): any brush but Draw, and
the compound brushes that most of the palette is made of.

### Every brush, a run of voxels at a time (issue 52)

The run that was Draw's is now one kernel for every brush on the field
(`brush/runs.rs`): the studio's compound brush, with whatever amounts of its
actions a brush has, and upstream's six, each of which is one action of it.
`compound.rs` has held one action to the classic brush of that name bit for
bit since it was written, so the classic brushes are handed the same kernel
and no second one was written for them.

Four voxels at a time: the weight under any profile, with a plateau or
without; where a push, a pinch and a gather read from, and the read; the
offset an inflate adds; the mean of the six neighbours a smooth blends
toward, which away from the rim of the copy are six loads of four; the blend
toward a plane, the side of it kept, and the mask let in after. A voxel on
the brush's axis, which a pinch has nowhere to draw toward, is left out of
the read as the closure leaves it out.

Milliseconds a stamp of 48 voxels' radius, the edit alone, on one core:
`stroke_profile <brush> 48 2`, single runs, before and after in one session.
The studio's brushes are named as the palette names them.

| | Before | After |
|---|---|---|
| Draw | 9.2 | 9.2 |
| Inflate | 6.6 | 4.8 |
| Smooth | 7.0 | 5.0 |
| Pinch | 14.6 | 8.5 |
| Flatten | 4.8 | 2.7 |
| Clay | 6.2 | 3.3 |
| Gather | 16.3 | 10.2 |
| Wear down | 16.0 | 9.2 |
| Ridge | 16.4 | 10.5 |
| Scrape | 7.7 | 3.0 |
| Fill | 6.9 | 3.1 |
| Polish | 13.1 | 5.9 |
| Crease | 16.3 | 8.5 |
| Bead | 17.4 | 8.8 |
| Band | 16.3 | 9.1 |

Draw is where it was, which is what was hoped of it: the kernel that does
everything costs it nothing over the one that did only Draw. By instructions,
which do not move from run to run (`valgrind --tool=callgrind`, one round of
48 stamps and their remeshes, the ball's making included, in thousands of
millions): Inflate 5.57 to 4.21, Gather 7.36 to 5.56, Polish 9.37 to 4.71.

**What a run writes is what the closure writes, to the bit**, as before, and
now compared as bits and not as numbers: upstream's six under each falloff,
both ways, at two voxel sizes; twelve studio brushes, among them every one
of the palette's that is amounts of its own and two that are every action at
once, under each shape of profile with a plateau and without; and 48 brushes
of random amounts, profile, strength and voxel size. Each test also counts
the voxels done by runs, so that a comparison of the closure with itself
cannot pass for one.

One thing is not held to the bit, because the language does not hold it:
which of a nought and a minus nought `f32::min` and `f32::max` return.
Levelling to one side takes the lesser or the greater of a voxel's value and
its blend, and where those are noughts of opposite sign either may come
back, from the closure as from the run. No test has met the case.

Checked by breaking it: the six neighbours added in another order, the
gather's projection along the stroke left out, the wrong side of a plane
kept and a pinch read on the axis each fail a test. Multiplying an
inflate's three factors in another order does not, and should not: one of
them is one or minus one, and the product is the same bits either way.

On a screen: five strokes on the starting ball with Crease, Bead, Inflate,
Pinch and Polish, by the same pointer moves, once with the runs and once
with a switch that offers none (made for the look and not kept). The two
pictures of the window are the same in every pixel. That is the software
renderer of a machine with no graphics card, which is enough to compare two
fields by.

Not done:

- **A brush that lays one layer a stroke** (Layer, Clay strips, Stitches) is
  a loop of its own (`compound/layer.rs`), a voxel at a time.
- **Not with a pattern, not with a picture across the tip**, and not where
  the mask varies within a brick. Each is the closure as it was.
- **Only x86-64.** Elsewhere no run is offered and the brush is as it was.
  The same four-at-a-time arithmetic exists on Apple silicon and was not
  written: this machine cannot run it.
- **The brush engine** (`brush/engine.rs`) is a voxel at a time. Nothing in
  the application stamps with it.
- **On every core** the gain was not measured brush by brush.

### A shape's set-down, profiled at last, and a crash it found

The set-down of a shape being dragged out was the slowest thing measured
here and had not been profiled since it was last changed. There is now a
program to profile it with (`examples/insert_profile.rs`, in the instruments
above), and what it showed first was that a soft join is everything: a ball
of 40 voxels joined hard cost 7.2 ms a move on one core, joined 3 voxels soft
29.7, and 8 voxels soft 59.8. Under callgrind 42% of the whole gesture's
instructions were one function, `widen`, which works the body's distance out
further than the field keeps it, on one core.

What was done, in the order the profile asked for it:

- **`widen` offers each point once.** A voxel is put on a front once for each
  point of the surface it is ever handed, and when two of those fall in the
  same half voxel it came round again holding what it held the first time and
  offered it to its 26 neighbours again, to no effect: half of all the
  offering there was. It is now marked as having offered what it holds.
- **And asks its neighbours three at a time.** The three along x at each of
  the nine places about a voxel, as four floats, with the operations every
  x86-64 has, in the order they were asked one by one. A voxel on a front
  carries where it is in y and z, which was three divisions to work out
  each time it was taken off.
- **The list of where to spread from** is a mark a voxel and not a set of
  coordinates, the search for it looks at sixteen voxels at a time before it
  looks at one, what each voxel is to hold is worked out a line at a time
  and not through a call a voxel, and the write visits only the voxels that
  change.
- **A shape's box is the shape's.** Every form said it was inside the ball
  round its box, 1.733 across where a ball is 1: five times the voxels,
  copied and looked through on every move. A ball now says it is a ball, and
  the others the box that holds their own box once it is turned.
- **A voxel that cannot change is not asked.** Adding moves a voxel toward
  solid and no other way, so a voxel that is solid already stays as it is
  whatever the shape says, and a brick that is all solid is passed over: the
  half of a shape that is inside the body it is set on was being made dense,
  gone through and put back. Taking out, the same of empty.
- **Nor is a block deep inside the shape.** Where the shape's own distance is
  past the end of the band, the join comes to that end whatever the body
  held. A big shape is mostly such blocks, and each of their voxels asked the
  shape how far off it was to be told so.
- **The voxel loop steps over a run of voxels it is not to visit** in one
  step, where it took them one at a time. This is every edit's loop; for a
  brush it came to a hundredth of a stroke's instructions or less (Inflate
  4.213 thousand million to 4.188, Gather 5.557 to 5.491, Polish no change).

Milliseconds a move, `insert_profile <form> <voxels> <soft> 1`, the join
alone, on every core, single runs of one session:

| | Before | After |
|---|---|---|
| Ball, 40 voxels, 3 soft | 18.7 | 8.8 |
| Ball, 40 voxels, 8 soft | 48.3 | 20.8 |
| Ball, 25 voxels, 2.5 soft | 11.0 | 5.2 |
| Box, 30 voxels, 3 soft | 19.1 | 8.2 |
| Cylinder, 30 voxels, 5 soft | 20.7 | 9.5 |
| Cone, 30 voxels, 4 soft | 12.4 | 5.6 |
| Ring, 30 voxels, 3 soft | 12.5 | 6.2 |
| Ball taken out, 40 voxels, 3 soft | 18.5 | 8.2 |
| Box taken out, 30 voxels, 6 soft | 27.7 | 12.5 |
| Ring taken out, 30 voxels, 8 soft | 29.7 | 12.7 |
| Ball, 70 voxels, 3 soft | 31.1 | 19.6 |

On one core, two drags: the ball of 40 joined 3 soft 29.7 to 13.0, hard 7.2
to 3.5, 8 soft 59.8 to 25.3; a box of 40 joined 3 soft 47.3 to 19.4. By
instructions, the ball of 40 joined 3 soft for one drag with its remeshes:
14.84 thousand million to 7.44.

In the application (`what_setting_a_shape_down_costs`, a ball dragged out in
48 moves, a move a frame), the edit phase a frame: median 31.7 ms to 16.6,
and 1532 ms to 974 for the whole drag. The slowest frames are another
matter, and the next paragraph is why: the worst was 63.4 ms before and 52.1
and 74.5 in two runs after.

**The last row of the table, and the slowest frames, are not like for
like.** A soft join is given up, and the shape left joined 2 voxels soft,
when the box it would be worked out in is over 12 million voxels. With every
form's box the ball round its box, that was a ball past 57 voxels joined 3
soft; with a ball's box a ball's it is past 98 (arithmetic, from the box and
the room about it). So a ball between those sizes
now gets the soft join it asked for where it was quietly joined hard, which
is right and costs what a soft join costs. What it costs at that size is
`widen` again, and not its arithmetic: the box the body's distance is
worked out in is the box round a ring on the body's surface, nearly all of
it nowhere near the ring, and it is filled, copied and gone through whole.

**What the join writes.** Each change but one leaves the field as it was to
the bit, held by tests: `widen` against itself as it was, kept in the tests,
on 96 random fields in boxes of every shape, twice in the same working
space; the join with its shortcuts and with every voxel asked, 160 random
joins of every form, a cable's run and a lent body, added and taken out,
with the same undo; and eleven drags of `insert_profile`, whose sum of the
field is the same number before and after for every form but the ball. The
one is the ball's box. The body's distance is worked out in a box of its
own, in coordinates that begin where the box does, and a box that begins
somewhere else rounds differently in the last place: a ball of 40 joined 3
soft differs at 43 voxels of 11 million, by two millionths of a voxel at
most (measured, by writing both fields out and comparing them).

**The crash.** Profiling a big ball stopped the profile: a ball more than
113 voxels from its middle to its edge had a box of more voxels than a copy
of the field may hold, and the core answers that by panicking. That is not
the profile's fault. At the voxel the application opens with that is a ball
of 28 mm, and at the finest it has, 3.4 mm (arithmetic: 113 voxels of 0.25
and of 0.03 mm). A test that drags a ball that far by pointer events, at
half the opening voxel, took the application down on the code as it was. The join now copies the body
a slab of the box at a time, each small enough, and what a voxel comes to
hold does not depend on which slab it is in: held to the bit by a test that
takes the same join in slabs a voxel deep, six deep and forty. With that and
the shortcuts above, a ball of 200 voxels is 64 ms a move where one of 113
could not be made.

On a screen, under Xvfb: a ball dragged out of the starting ball to about a
third of its width, and a box beside it. Both are set down with their soft
joins and nothing in the log. The big ball at a fine voxel was not driven on
the screen; the test that drags one by pointer events is what holds it.

Not done:

- **The soft join's box.** It should be the ring and not the box round the
  ring: worked out in pieces, each a part of the ring with room about it.
  That would also let it use every core, which `widen` does not, and move
  the size at which a soft join is given up, which is a limit on time that
  the time no longer justifies. The pieces would not round as the whole
  does, so it is a change to what is written, in the last place.
- **No limit on how big a shape can be dragged out.** It no longer panics,
  and its cost grows with its size: a ball of 400 voxels is some eight
  thousand bricks of solid, which is a gigabyte (arithmetic). A brush is held
  to 100 voxels by upstream for this reason. A limit on a shape is a limit
  on what can be made, and was left for the maintainer to set.
- **A brick wholly inside a shape** is still made dense and written a voxel
  at a time, to end as one value throughout. It could be made that value at
  once.
- **A shape's distance is asked a voxel at a time** through a call that
  cannot be seen through, for the voxels near its surface. A line of them at
  a time, as the brushes are now done, is the same idea and was not built.
- **Only x86-64** has the three at a time. Elsewhere `widen` asks one
  neighbour at a time, and still offers each point once.

### A shape previewed coarse, and the airbrush and the layers across every core

Asked for by the maintainer on 2026-11-25: inserting was slow, a large paint
brush and above all the airbrush were slow, and could a gesture be tried
against something coarser and applied to the real thing. Measured in release
on the machine above, by two measurements run by hand:
`what_a_move_of_an_insert_costs` (`app/adding.rs`) and `what_painting_costs`
(`app/airbrush.rs`, with `HANI_LAYERED=1` for a stack of five layers, a
spongechop over the paint). Both time the application's own handling of the
messages, so they include the remesh's CPU half and not the GPU's.

**A shape dragged out is previewed, and set down once.** Every move used to
put the body back and join the shape again. Now a move samples the shape's
own distance on a grid 40 cells across and meshes that into the overlay,
translucent, and the join is done when the button comes up
(`app/insert_preview.rs`). The preview does not show the soft join.

| One move of a drag | Before | After |
|---|---|---|
| Ball, 15 voxels from its middle | 6.7 ms | 6.8 ms |
| Ball, 42 voxels | 21.0 ms | 5.9 ms |
| Ball, 81 voxels | 53.8 ms | 5.8 ms |
| Box, 81 voxels | 48.6 ms | 4.8 ms |

**The airbrush a brick at a time, across every core.** Its first version
asked every voxel of the box round the spot, solid and empty space alike,
and took the normal by the trilinear gradient. Now bricks that cannot hold
paint are passed over whole, the normal is six lookups, and the bricks are
worked out on every core (`paint/spray.rs`).

| One frame of the airbrush | Before | After |
|---|---|---|
| 2 mm spot | 1.6 ms | 0.8 ms |
| 5 mm spot | 7.6 ms | 1.9 ms |
| 10 mm spot | 40.2 ms, then 36.8 and 35.1 on reruns | 6.4 ms |

**The layers put together across every core.** Each dirty brick of the
composite was put together one after another; now they are worked out in
parallel and stored after (`Volume::put_the_layers_together`).

| One move of a loaded brush, over five layers | Before | After |
|---|---|---|
| 2 mm | 3.8 ms | 2.4 ms |
| 5 mm | 6.6 ms | 4.6 ms |
| 10 mm | 15.6 ms | 10.3 ms |

Over one plain layer the brush is 1.8, 3.7 and 7.7 ms at those sizes, and
was not changed. What a graphics card adds to any of these, the upload and
the draw of the bricks remeshed, was not measured: there is none here.

**What a stroke deposits (2026-11-25), a cost and not a gain.** A stroke
now keeps two numbers a voxel, the most any stamp gave and what it has
deposited, so that its passes add up where they overlap
(`paint/stroke.rs`). One move of a loaded brush over one plain layer, two
runs of each build back to back, on a machine whose runs of one build
differed by up to 2 ms:

| One move of a loaded brush, one layer | Before | After |
|---|---|---|
| 2 mm | 2.2, 3.2 ms | 2.8, 3.8 ms |
| 5 mm | 5.3, 6.3 ms | 6.5, 5.5 ms |
| 10 mm | 10.8, 12.9 ms | 13.9, 15.4 ms |

About a fifth more at 10 mm, by the means of the two. The airbrush does
not take this path and was not changed.

**A decal drawn while it is dragged, and laid once (2026-11-25).** It
was dabbed again at every move, and each dab asked every voxel of the
cube round it. Now a move draws it on a grid cast onto the surface, and
the dab, which passes over bricks with no surface and works across every
core, is laid when the button comes up (`app/decal.rs`,
`paint/decal.rs`). `what_a_decal_costs`, release build:

| A decal across | One move, before | One move, after | Laid on release |
|---|---|---|---|
| 4.5 mm | 3.4 ms | 2.8 ms | 4.9 ms |
| 11 mm | 15.0 ms | 7.3 ms | 9.1 ms |
| 21 mm | 57.6 ms | 10.8 ms | 12.9 ms |
| 41 mm | 261 ms | 15.5 ms | 27.4 ms |

**And a crash.** A drag toward the horizon of the surface pressed on asked
for a shape with no end, and a test that dragged 3,000 pixels past the model
was killed by the kernel for memory. A shape is now held to 160 voxels from
its middle, twice the largest brush (`MAX_INSERT_VOXELS`). Since 2026-11-28
the bound comes from the memory there is: see `app/placed.rs`.

## A pass over the cores and the graphics card (2026-11-26)

Asked by the maintainer: are we using all our cores, is there a way to test
how many a given PC should use, and are we doing all we can on the graphics
card. Rayon reads `RAYON_NUM_THREADS`, so each instrument was run with 1, 2
and 4 threads. Milliseconds; the sculpting rows are medians a frame of
`what_a_stroke_costs` on the ball at a sixteenth of a millimetre.

| Before this pass | 1 thread | 2 | 4 | 4 against 1 |
|---|---|---|---|---|
| Sculpting stroke, the edit | 8.77 | 4.52 | 2.46 | 3.6 times |
| Sculpting stroke, the remesh | 4.71 | 2.25 | 1.35 | 3.5 times |
| Paint brush, 10 mm, a move | 12.8 | 13.4 | 9.8 | 1.3 times |
| Airbrush, 10 mm, a frame | 12.8 | 8.7 | 8.6 | 1.5 times |
| Decal dragged, 41 mm, a move | 17.0 | 17.2 | 17.3 | none |
| Decal laid, 41 mm | 68.7 | 40.4 | 30.5 | 2.3 times |

Sculpting used the cores well. Painting did not: the paint brush wrote its
bricks one after another on one thread, the airbrush and the decal worked
out what to lay across cores and then wrote it on one, and the decal's
preview cast its rays on one.

**Paint written across cores** (`Volume::edit_paint_across_cores`,
`paint/field.rs`). The sculpting edit's way: the paint bricks a stamp
reaches are lifted out of the field, each written on whichever core with
its own state (a stroke's coverage of it, a spray's amounts), and put back
with the undo record. Each brick is written by one hand from what it held,
so what it holds afterwards is what the serial edit leaves, to the bit: a
property test holds the two to it for any box, in a stroke and out of one,
undo included, and every stroke test that compares to the bit still holds.
The drybrush's and the sponge's cache of the surface's curvature, which was
in cells, is in atomics, so that a stamp that reads it can be shared out;
two cores that work out the same cell at once get the same number. The
stroke, the airbrush and the decal all take this path.

**The decal's preview cast across cores.** 2401 casts along the view.

**A paint edit marks only what it changed.** It marked the box round
everything it changed, and every brick in that box was remeshed. Measured
with `paint_profile` (below), 10 mm, four threads: 762 bricks remeshed a
stamp before, 458 after, and the remesh 20.7 ms before and 16.4 after.

**A stamp passes over a brick it cannot reach.** Nothing further than the
radius times the square root of two from the path is reached (a radius
across, a radius deep), so a brick whose middle is further than that and
half its diagonal is not opened. It did not change these numbers, the
corners of the box round a stamp on a ball holding little surface, and it
is kept: it costs a subtraction a brick and the tests hold it to the bit.

| After | 1 thread | 2 | 4 | 4 against 1 |
|---|---|---|---|---|
| Paint brush, 10 mm, a move | 11.3 | 8.3 | 5.1 | 2.2 times |
| Airbrush, 10 mm, a frame | 12.5 | 6.7 | 4.2 | 3.0 times |
| Decal dragged, 41 mm, a move | 18.4 | | 6.1 | 3.0 times |
| Decal laid, 41 mm | 75.7 | 43.5 | 22.5 | 3.4 times |

### How many threads a machine should use

Unless told otherwise the pool has a worker for every thread the machine
has. Nothing held one back for the window, and nothing needs to: the
window's thread waits while the pool works. What cannot be known in advance
is where more threads stop paying: a second thread on a core that has one
(hyper-threading) gives far less than a core, and a small efficiency core
less than a large one. So it is measured.

`hani_core::calibrate` times a stroke's kind of work (a ring of
sculpting stamps, each remeshed, and a ring of paint stamps, on a ball at
a sixteenth of a millimetre) in a pool of 1, 2, 4 and so on up to every
thread, after a warm-up, and recommends the fewest threads within three
parts in a hundred of the fastest.

```bash
cargo run --release -p hani-core --example cores
```

On the build machine, twice: 1 thread 822 and 827 ms, 2 threads 490 and
467, 4 threads 251 and 267; four recommended. In the application,
*Measure* in Preferences runs it off the window's thread and sets *Worker
threads* from it; on the same machine, a debug build drawing its own window
on the CPU, it found 1042, 645 and 604 ms, a flatter curve than the bare
core's, which is what measuring where the work is done is for. *Worker
threads* is `cores.conf` in the settings, and the pool is sized from it
once, as the application starts (`app/cores.rs`).

### The graphics card

It draws, and does nothing else: there are no compute shaders. Every edit,
the meshing, the layers put together and the occlusion are on the CPU, and
now spread across it. What could move, in the order it would be worth
doing:

1. **Paint looked up by the card, not carried by the mesh.** A paint stroke
   changes no shape, yet the surface is meshed again to carry its colours
   to the vertices. `paint_profile`, four threads, a 10 mm stamp at a
   sixteenth of a millimetre: 40.7 ms of stamp and 16.4 ms of remesh;
   2 mm, 2.9 and 2.1. A paint field the fragment shader reads, a brick at
   a time, would drop that remesh and sharpen the paint, which is now as
   fine as the mesh's vertices. Its cost is memory on the card: a dense
   brick of paint is 128 KB, and a fully painted figure at that voxel is
   thousands of them, so it wants the band packed and not whole bricks.
   A piece of work in its own right; not begun.
2. **The edits and the meshing as compute** (issue 59). Not attempted,
   for the reasons in the section below, which stand: the CPU paths are
   held to the bit by their tests, they now scale with the cores, and a
   compute path here would run on the CPU and say nothing about a card.
3. **Timing what the card does.** iced makes the device with no optional
   features, so there is no timestamp query, and the card's share of a
   frame is seen only as the interval between frames in the replay.

Nothing in this section was measured on a graphics card; the build
machine has none.

### Paint carried onto a sculpted surface (2026-11-27)

New work in a sculpting stroke over paint: after each pointer event's
stamps, each voxel that has come into the band since the stroke began and
holds no paint takes the paint along its normal (`paint/carry.rs`). Measured
with `what_carrying_paint_costs` (`app/paint_carry.rs`, run with `--ignored
--nocapture`) on the build machine, the dev profile, which is optimized
here: 60 pointer events of a Draw stroke over the default ball coated red,
the time in the carry against the event's whole handling, which includes
one carry.

| Brush radius | First version | Saturated bricks passed over | Whole event, after |
|---|---|---|---|
| 3 mm | 0.23 ms | 0.18 ms | 1.03 ms |
| 10 mm | 1.26 ms | 0.63 ms | 1.95 ms |

The first version read every voxel of a stamp's box through the volume's
brick map; the second passes over a brick that is saturated throughout,
which has no voxel in the band and is most of the box, before asking after
its paint, and reads a dense brick's voxels directly. A body with no paint
costs the carry one look at each layer's field and nothing more.

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
  what was left in the stamp, the brush's arithmetic a voxel at a time, has
  since been taken four voxels at a time on the CPU (issue 52) with the
  bit-for-bit tests still holding. A compute path cannot keep those, and
  what it may differ by is still the maintainer's to decide.
