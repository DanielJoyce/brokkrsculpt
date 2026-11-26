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

## The cut sized to what it cuts (2026-11-26)

The maintainer's answers to the open questions: a ring over both legs
**splits both**; the cut "should be the size that allows it to cut" (not
the ring's circle); and a ring with X on "should only cut one leg" (it
does not mirror).

**Fitted to the islands** (`Volume::fit_to_islands`). The disc's plane
crosses the solid in islands, each a cross-section. Each island with a
cell inside the ring is followed out, three ring radii at most; one that
runs further, or has less than two fifths of itself inside the ring, is
not a piece the ring is round. The cut is the islands that qualify, whole,
two voxels round, and nothing else in the plane: a leg a little wider than
the ring is cut whole, a torso the same plane crosses beside it is not, and
two legs are two islands, both cut. A piece of two real parts is split into
a body each in the same undo entry (`Entry::followed_by`, and `split.rs`'s
own split); a speck is not a part and makes no body.

**The narrowest cut round the part** (`Volume::propose_cut`). Directions
tried: the solid's three axes under the ring, the surface's normal where it
was pressed, the view, and the way the piece grows out of the body (from
the middle of the solid within three radii toward the ring's middle, where
that is clear). Each is slid a quarter radius at a time, a radius either
way, and fitted. A slice qualifies as a **neck** (no wider than the slices
beside it) or a **base** (the last that fits before the body, with the
slices the other way running out to a tip), and not as a **graze** (the
body past it on both sides), and only when it is through the ring's middle
or has the middle on the piece's side. Of those, the one taking the most
piece through the least section, a little against sliding far or slanting
off the way out of the body, is proposed. What led there, each seen on the
screen or in a test: the long axis alone put a diagonal over a knob; the
smallest section took a sliver off a knob's tip, then a cap off its flank,
then a strip down a leg's flank; a dome blended onto a ball has no neck at
all; and the smallest section that qualified was still a sliver off a
dome's flank. A lone ball has no neck and no base: a ring over it proposes
nothing, and halving it is the line cut's.

Tests: a ring round both legs severs both, two bodies and the figure, one
undo; a ring smaller than a leg cuts that leg whole and only it; a slab the
plane crosses beside a rod, reaching into the ring and far past it, is left
out; a knob on a stalk is cut round its neck; a dome blended on is cut at
its base; a wide ring pressed on one leg cuts across both. On the screen: a
dome on the ball, the band round its base, the dome split off as one body.

## Stage 2, as built (2026-11-26)

**The ring the handles move** (`app/ring_cut.rs`). A cut proposed is kept
as a ring: its middle, its normal (turned toward the piece, the side with
less solid within three radii), an axis across it, and the radius the ring
was drawn at. The fitted cut and its band are worked out from it again on
every move of a handle, and a ring that fits nothing is drawn in red with
no band, *Split in two* waiting.

**Four handles**, small squares facing the eye, each lifted toward the eye
along its own line of sight (on the rim they sit where it meets the
surface, which hid them; lifted along one shared direction, those off to
the side of the view slid off the rim, both seen on the screen):

- on the normal's axis, past the rim: slides the cut along it;
- on the rim at the axis across: tilts it about the other axis;
- on the rim a quarter turn on: tilts it about the first;
- on the rim's far side: widens or narrows it.

A press within twelve pixels of a handle takes it; anywhere else starts a
new ring. A drag moves the handle by how far the pointer went along the
way the handle moves on the screen, measured from where it was taken, so
the result does not depend on how many pointer events arrived.

**See-through** (`brokkr-gpu`, `sculpt.wgsl`). Three new vectors in the
uniforms: a plane, a sphere, and an amount. A fragment in front of the
plane and within the sphere is drawn as it always was; any other has that
share of its pixels screened away by a 4x4 ordered dither. No sorting, the
depth buffer still right, and the band and cut face show through what is
screened. The application keeps the piece's side, from two voxels behind
the cut to four radii out, and the rest screened by *See-through* on the
bar, 60 % unless set; with no cut proposed, or another tool, it is off.

Tests: a render test (llvmpipe) holds that off changes no pixel, that
fully screened past a plane through a ball draws the kept half as before
and the other not at all, and that half screened draws about half; the
handles, dragged through the pointer as a user drags them, slide the cut
along its normal and not round it, turn it, and widen it, and it still
fits; the see-through view is published while a cut is proposed and off
when it is dropped; a cut turned to fit nothing is not split and says so.
On the screen: a dome on the ball, ringed, the ball screened and the dome
solid, the ring and its four handles, and the cut slid out along its axis
with the band following.

Not done: a handle hidden behind the piece cannot be taken until the view
is turned; the see-through view keeps a sphere of four radii, which on a
long limb leaves the far end screened.

## Stage 3, as built (2026-11-26): Part with pegs

**Keys on the cut's face** (`Volume::plan_keys`). A parting's plan on the
cut's plane (`part.rs`, every rule of it: the widest point of each island,
a second key where there is room, the wall round and under each), and of
its keys those within the fitted cut, so a cross-section the plane makes
elsewhere gets none. Planned each time the cut is fitted, so moving the cut
plans them again.

**Severed and keyed** (`Document::sever_keyed`). The sever, then the keys
between the two bodies by the parting's own loop, lifted out of
`Volume::parted` into `part::put_keys`: a peg added to the body behind the
plane and its socket, the peg grown by the clearance, taken out of the one
in front (a pin hole or magnet pocket out of both). One undo entry.

**In the application.** Each key drawn on the cut face, its rim and a dot
at its middle; a press on the dot drags the key across the face, to where
the pointer's ray meets the cut's plane, and only within the cut. *Part
with pegs* beside *Split in two*, waiting while the cut fits nothing or has
no room for a key. The key's size and kind are the parting's, set in Print.
The piece is now screened too, at half the see-through: the keys are on the
cut face, inside the piece, and drawn solid it hid them (seen on the
screen).

Tests: an arm severed with keys has the peg on the body behind the cut and
its socket in the arm, keys only within the cut; in the application, keys
planned on a cut that fits, one dragged through the pointer moves across
the face and stays on it, Part with pegs leaves the peg's tip in exactly one
body, one undo; the render test holds the kept part screened by its own
share. On the screen: a dome ringed, the cut face and its key seen through
the piece, Part with pegs making two bodies.

Not done: a key dragged is not checked against the wall the plan checks
for, so one pulled to the face's edge can break out; a key moved by hand is
planned away again when the cut is moved.

## Open questions

None open.
