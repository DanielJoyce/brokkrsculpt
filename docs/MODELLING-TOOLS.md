<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# Modelling tools: shell, extract, deform, revolve

Asked for by the maintainer on 2026-11-26, in these words: *"Implement
shell, which takes an implicit surface and makes it hollow with some
specified wall thickness. The current surface becomes the outer surface.
Implement extract where you mask on a surface and the masked area is
extracted by thickening it a certain distance. At the same time users can
offset it and choose how much of that distance goes inward and penetrates
the existing surface. The extracted surface becomes a new object in the
tree. Implement deformation modifiers such as twist, curve, etc. Implement
revolve which shows a vertical line on the screen and the user can draw a
curve or squiggle on either side and the curve is rotated around the line
to produce a solid object."*

Not in the first plan. Built in one pull request, the four together,
because shell and extract share a new piece of the engine and all four
are the same kind of whole-body operation with one button.

| Tool | Where | What it does | Engine |
|---|---|---|---|
| Shell | Sculpt, FORM, *Shell* row | Hollows the active body to a wall; its surface is the outside of the wall | `hani_core::shell` |
| Extract | Sculpt, EXTRACT | The masked part of the surface thickened into a new body below the active one | `hani_core::extract` |
| Deform | Transform, DEFORM | *Twist*, *Bend* or *Taper* the active body along X, Y or Z | `hani_core::deform` |
| Revolve | Transform, REVOLVE | *Draw a profile*: an upright axis on the screen, a line drawn beside it, spun into a new body | `hani_core::revolve` |

Each is one undo entry. Shell and deform hold the body as it was; extract
and revolve add a row that undo takes away.

## The distance past the band (`reach.rs`)

The field is a distance only within three voxels of the surface; past that
it is a saturated sign. A two millimetre wall at a quarter millimetre is
eight voxels in, so the shell needs a distance the field does not hold.

`hani_core::reach` finds it sparsely. Every band voxel within one voxel
of the surface gives the surface point nearest it,
`x - d * grad d / |grad d|^2`. Each point's ownership then spreads outward
through the lattice, nearest first, a voxel keeping whichever point its 26
neighbours offer is nearest: the propagated closest point transform. The
storage is a dense brick of point numbers wherever the reach goes, and
nothing is stored past the limit the caller names.

Measured by `a_ball_reads_its_own_radius_deep_inside`: on a 10 mm ball at
0.25 mm, one to three millimetres in along five directions, the worst
error against the true depth is under a quarter of a voxel (the test's
bound; the reading is not printed).

## Shell

The wall is `max(d, depth - wall)`: inside the body and nearer the surface
than the wall. `d` is the stored field, untouched, so the outside surface,
its paint, its layers and its mask are exactly as they were. The inner
wall is the offset surface: a groove outside is a rounded ridge inside, and
a part thinner than two walls stays solid, so nowhere is thinner than
asked.

Measured by `a_ball_becomes_a_wall_of_the_thickness_asked`: a 10 mm ball
shelled to 2 mm crosses at 8 mm and 10 mm along three directions, each
within 0.1 mm. A property test holds the inner wall to within 0.1 mm of the
wall asked for walls from 1 to 4 mm.

Refused: a wall thinner than one voxel, which cannot be meshed as two
surfaces.

## Extract

`s` is the signed distance to the whole surface. The plate is the slab
`inner <= s <= outer`, with

- `outer = offset + (1 - inward) * thickness`
- `inner = offset - inward * thickness`

so *Inward* is the share of the thickness that goes into the body, nought
to one, and *Offset* lifts the whole plate off the surface (or sinks it).
The plate stops at the mask's edge: a point is in its footprint when the
nearest surface to it is masked, and the side wall stands square to the
surface there. A voxel counts as masked at half protection or more, so a
feathered edge is cut at the middle of its feather.

The new body is named after the source with " plate", goes directly below
it in the tree at its depth, and becomes the active body. The source is not
changed.

## Deform

Each deformer is a warp, `new(p) = old(inverse(p))`, so surface detail rides
along, as in the form tools. The frame is the body's surface bounds: a
centre, and along the chosen axis a base and a length. `t` runs from nought
at the base to one at the far end.

- **Twist** turns each slice about the axis by `amount * t` degrees, so
  *Amount* is the turn at the far end.
- **Bend** curls the axis into an arc of *Amount* degrees, the base
  staying put, toward X for a body along Y or Z and toward Y for one along
  X. At most half a turn.
- **Taper** scales each slice across the axis by `1 + (amount - 1) * t`:
  *Amount* is the far end's size, and the base keeps its own.

Paint and paint layers go with the surface, voxel by voxel. The field is
redistanced after, because a warp that stretches leaves the gradient off
one. Each kind keeps its own amount.

A property test holds every deformer's inverse to its forward map within
a thousandth of a millimetre over a 10 mm box, for bends up to 85 degrees.
A bend tighter than the body is thick folds it through its own centre of
curvature, which no inverse undoes.

## Revolve

*Draw a profile* puts a dashed upright line on the viewport through the
point the view turns about. A left drag draws a line; its mirror is drawn
faint on the other side of the axis, which is the half of the solid not
being drawn. On release the line is spun round the axis into a body named
*Revolved*, and the tool is put away. Escape puts it away without making
anything. The other buttons still turn and pan the view while it is out.

In the world the axis is the camera's up through the target, and the line
lies in the plane through the target facing the camera, so what is drawn
is made at the size it was drawn, and turning the view first chooses which
way the axis lies.

Only how far a point is from the axis matters, not which side, so a line
drawn on either side, or crossing the axis, spins into the same solid as
its fold onto one side. The solid's meridian section is the profile, a
flat cap across the axis at each end, and the profile mirrored: a closed
outline symmetric about the axis, whose distance is exactly the solid's
distance in that half plane. Inside is by the even-odd rule, so a line that
crosses itself still makes a solid. The outline's distance is worked out
once on a grid of half a voxel and read bilinearly, so the cost per voxel
does not grow with the number of points drawn.

## Not done

- **Shell has no drain hole.** A resin print of a closed shell traps
  liquid. Cut one with the ring cut or a brush.
- **The reach runs on one thread**; the write-back is parallel. Its time on
  a large body is not measured here (the build machine has no screen to
  time a real model on, and no figure is quoted).
- **Extract carries no paint** onto the plate, and its side wall is square,
  not bevelled.
- **Deform is applied by a button**, not live: no preview while the amount
  is changed, and no modifier stack kept to change later. Undo is the way
  back. The mask is not carried by a deform.
- **Revolve's line is one line.** A hole through the middle is drawn as a
  line that goes up one side of the wall and down the other. The line is
  not smoothed beyond thinning to a voxel's spacing.
- **The plate's edge shows small steps** along the mask's boundary, about
  a voxel high, because which side of the edge a point is on is decided by
  the surface voxels the mask covers. The Smooth brush takes them off.

## On a screen

Looked at under Xvfb at 1280 by 900 on the starting ball, 2026-11-26,
with the debug build (optimised, with debug information) on this
four-core machine with no graphics card:

- *Taper* at 0.5 along Y made the ball an egg, 1,279 ms by the status
  line. *Bend* at 45 degrees then leaned it toward +X, 1,458 ms.
- *Draw a profile* put the dashed axis through the view's target. A
  squiggle drawn right of it was drawn with its mirror faint on the left,
  and on release became *Revolved*, a vase-like body round the axis, 42 ms.
  Xvfb delivered only 14 pointer positions for the whole stroke, so its
  sides were faceted; a real pointer gives far more.
- A band of mask painted across the vase's middle, *Extract* at 1 mm made
  *Revolved plate* directly below it in the list, a band standing on the
  vase's surface.
- *Shell* was not looked at: from outside a hollowed body looks the same.
  The tests read the wall through the middle.
