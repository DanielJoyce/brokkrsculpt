# The ring cut: drag a ring round what to cut, then split or part it

Asked for by the maintainer on 2026-11-26:

> instead of drawing an arc across the entire model can I drag a circle over
> it instead? Like stretching a rubberband and then choosing cut? That way I
> don't have to worry about a simple 2d screen space line cutting through
> more of the model than I want, like a arm and accidentally cutting the
> torso as well. If once I get the circle approximately where I want it it
> would then be cool if I could adjust the contour further (like around
> collar of a shirt) and then even adjust the cutting plane and where the peg
> will be placed. Maybe by adjusting part transparency so you can see.

The maintainer's answers, the same day:

- **The first plane runs across the limb**: the long axis of the material
  inside the ring, and the plane square to it through the ring's middle.
- **A flat plane first**, moved and tilted by handles; a curved cut, shaped
  by dragging points on the contour, is a later stage.
- **The choices are Split in two and Part with pegs**, and **the line cut
  stays** beside the ring, for cutting straight through everything on
  purpose.
- **While adjusting, the rest of the body is ghosted**: the piece being cut
  stays solid, everything else is see-through by a slider, and the cut face
  and the pegs are drawn on top.

Then, the same day, on stage 1 as it was being built: "More like drag a
rubber band on the surface. Or at least the disc projects down to a band
for finer manipulation", and "if you use the disc for the initial
positioning people can just rotate it as needed". So **the band on the
surface is what is shown**, where the disc would part the model, and the
disc is what is positioned and turned: stage 2's handles turn the disc
and the band follows, and stage 4 drags points of the band itself.

## What is wrong with the line

The cut tool reads a drag as a line, a curve or a lasso on the screen and
carries it along the view as a prism (`app/cut.rs`). A line is a plane
through the whole model, so a line across an arm hanging beside a torso
crosses the torso too. Parting, which keeps both sides and keys them, takes
only that one straight line (`part_along` in `app.rs`). Nothing in either
says "this piece and not its neighbour".

## The gesture

1. **Drag out a ring.** With the Cut tool's *Ring* way chosen, the press is
   the ring's middle and the drag its radius, a rubber band on the screen.
2. **A cut is proposed.** When the button comes up, the material under the
   ring is looked at: the point under the middle and the solid round it
   within the ring's radius. Its long axis (the largest spread of that
   solid, by principal components) is the limb's, and the plane is square to
   it through the middle of the material under the ring. The cut is a disc:
   that plane, within the ring's radius of its middle.
3. **The contour is drawn**: where the disc meets the surface, the line the
   cut will leave on the model.
4. **Adjust** (stage 2): the disc's rim drawn with handles to turn it, a
   handle to slide it along its normal, its edge to widen or narrow it, the
   band following each change; the rest of the body ghosted.
5. **Choose**: *Split in two* or *Part with pegs* (stage 3), or Escape to
   drop it.

## The engine: a disc that severs

A plane cut on a distance field is `max(field, plane)` and closes its own
face (`clip.rs`). A disc is a plane that stops: it separates a piece only
if it goes all the way round the piece. So:

- **Which side is the piece.** The solid is walked voxel by voxel from both
  faces of the disc at once, never across the disc. If the two walks meet,
  the disc does not go all the way round, and nothing is cut: the status
  line says so. Otherwise the walk that finishes first is the piece, which
  is the smaller side: the arm, not the figure.
- **The two bodies.** Each is the source with the other's solid raised to
  outside, as `split.rs` makes its parts, and within a few voxels of the
  disc `max(field, ±plane)`, so that each has a flat closed face exactly on
  the plane and nothing else of the body is touched.
- **Pegs** (stage 3) are `part.rs`'s keys, planned on the disc's face only:
  a peg is `min` into one side and its socket, the same solid grown by the
  clearance, `max` out of the other.

The walk costs in proportion to the smaller piece, since it stops when that
side is done. Solid bricks stored as one number are taken whole.

## Stages

1. **The ring and a split.** The *Ring* way in the cut tool, the ring drawn
   while dragged, the plane across the limb, the contour drawn, *Split in
   two* and Escape. The line cut stays as it is. Why the cut is slow is
   measured here, before anything is changed for it.
2. **Adjusting, and the ghost.** Handles for the plane's offset, tilt and
   the disc's radius; the rest of the body drawn see-through, by a slider.
3. **Part with pegs.** The keys planned on the disc's face, drawn, and
   draggable on it before the parting is made.
4. **A curved contour.** Points on the contour that can be dragged along
   the surface, and a cut surface through them instead of the plane.

Each is tested as the rest of the studio is: properties where a rule holds
for any input (a split's two bodies together are the source; a disc that
does not go round cuts nothing; the piece is the smaller side), the
application driven by its messages and pointer events, and a look on the
screen.

## Stage 1, as built (2026-11-26)

**The engine** (`brokkr-core/src/sever.rs`). `Volume::disc_across` takes
the point under the ring's middle, the view's direction and the ring's
radius, walks the view's line through the solid to its far side, samples
the solid within the radius of the middle on a grid of about forty a
side, and takes its principal axis as the disc's normal. `Volume::sever`
walks the solid from both faces of the disc, taking turns; a voxel step
across the disc is not taken; the two meeting is *not round*, one side
with no seed is *misses*, and the walk that runs out first is the piece.
`Document::sever` puts the piece and the rest in the source's place as
*A* and *B*, the piece active, one undo entry.

**The walk, made fast enough.** The first walk took a voxel at a time
through the map and took 1.5 s to sever the ball the application opens
with through its middle (the worst case: two equal halves), 76 s at a
quarter of the voxel. A brick solid throughout and away from the disc is
now walked whole, and a brick with surface in it is flooded through its
own array, with the disc tested only in bricks near it. Through the
application, `what_a_split_costs` (release, ignored), the ball split in
half: 0.55 s at 0.25 mm (564 bricks), 9.3 s at 0.0625 mm (18 250
bricks), of which about a third is building the two bodies, each a copy
of the source as a parting's halves are. A limb, the case the ring is for,
is a small side, and its walk stops when the limb is done.

**The application** (`app/ring_cut.rs`). *Ring* and *Line* on the cut
tool's bar, Ring first. Press and drag a ring, drawn square to the camera
round the press; let go, and the disc is found once at the scale of the
point pressed and again at the scale of its own middle (at the front's
scale, a ring wider than a ball was a disc inside it, out of sight: seen
on the screen). What is drawn is the band: where the disc meets the
surface, traced by marching squares on a grid of 96 across the disc, as
a ribbon five pixels wide lying on the surface. *Split in two* and *Drop*
on the bar; Escape drops it. A refusal says so on the status line, which a
shape chosen in the palette had been covering whatever tool was in hand;
that prompt now shows only with the sculpting tool in hand. The tests of
the line cut choose *Line* when they arm it.

Tests: a disc across an arm severs it, the smaller side, the two together
the figure within a face's rounding, both printable; the same for any tilt
up to a third of a right angle and anywhere along the arm (a property); a
disc too small cuts nothing and one in empty space misses; a ring round
the arm proposes a disc square to it; a body severed is two bodies and one
undo; in the application, a small ring over the ball proposes a disc
inside it, no band, and refuses to split; a ring wider than the ball
splits it in two and undoes; a ring round an arm splits it off. On the
screen: the ring while dragged, the disc round a ball, a ball split into
*A* and *B*, a band over a knob on the ball, and the refusal readable.

Not done in stage 1: handles to turn and slide the disc (stage 2); the
ghost; a knob, barely longer than it is wide, gets a disc across its top
rather than round its neck, which turning it will put right; the split's
cost at a fine voxel when both sides are large.

## Open questions

1. A ring over two limbs at once (both legs): two pieces, or refused?
2. The ring on the screen and the disc in the model can disagree on a
   foreshortened limb; should the ring's radius be the disc's, or should the
   disc grow to the limb's cross-section?
3. Mirror: should a ring cut with X on cut the mirrored limb too, as the
   line cut does?
