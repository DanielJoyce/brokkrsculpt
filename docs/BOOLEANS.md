<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# Boolean groups

Asked for by the maintainer on 2026-11-28: *"I want hierarchy to support
nesting with groups and Boolean ops between nodes resulting in a groups and
group nodes can be transformed with the gizmo, and the Boolean ops can baked
in which collapses nodes and replaces them with the result. The Boolean is a
bit tricky because the order can matter in some cases ( union vs diff ) so
think about a way to show this and spend some time getting it right."*

## What a boolean group is

A folder in the bodies list is a **group**, as it has been: it holds rows and
changes nothing about them. A group can be made a **boolean group**. Then its
children are its operands, read **top to bottom**, and what the group shows
is one solid:

```text
result = child 1
result = result  op 2  child 2
result = result  op 3  child 3
...
```

Each child after the first carries one operation:

| Chip | Operation | What it does to the result so far |
|---|---|---|
| **+** | Add (union) | adds the child's solid |
| **−** | Cut (difference) | takes the child's solid away |
| **∩** | Keep (intersection) | keeps only what is also inside the child |

The first child is the **base**, and its chip says so: an operation on the
first row would have nothing to work on.

A child that is itself a group stands for its own result: a boolean group
for its fold, a plain group for the union of everything in it. So groups
nest, and the nesting is the brackets.

## Why the order is a fold, top to bottom

Order matters: `(A + B) − C` is not `A + (B − C)` when `C` overlaps `A`.
Three ways of showing that were weighed.

1. **A tree of binary nodes**, each with two slots, as a CSG tree is drawn
   in a textbook. Every bracket is explicit, but a chain of five cuts is
   five levels deep, past the eight the list allows, and the list stops
   reading as a list of parts.
2. **An operation per row with an unordered set**, as some modellers do:
   unions first, then differences. Simple, and wrong for exactly the case
   the maintainer named: a part added back inside a hole after the cut
   cannot be said.
3. **A left fold down the list**, as a modifier stack or a feature
   timeline reads. The order on screen is the order applied, and moving a
   row up or down is how the order is changed, with the drag the list
   already has. Brackets are a nested group.

The third is taken. What keeps it honest is that the order is shown, not
implied:

- the chips on the rows, read down the list;
- the group row's tooltip spells the fold out with every bracket, as
  `((Body 1 − Hole) + Boss)`, so what the rows mean is never a guess;
- a row moved within a boolean group changes the result at once, on the
  model, so the order is seen as well as read.

## In the list

- A folder row has a **∪** chip. Pressed, the folder combines its rows; lit,
  it does, and its tooltip spells the fold out with every bracket.
- A row inside a combining folder has a chip with its operation, **+**,
  **−** or **∩**, pressed in turn through the three; the first row says
  **Base**.
- In Transform, under the verbs, **+**, **−** and **∩** put the body in hand
  and the body below it into a new folder that combines them, the one below
  the base and the one in hand added, cut or kept. **Bake** shows while the
  body in hand is in a combining folder.
- A hidden row is left out of the fold, which is how a cut is tried with
  and without.

## On the model

- A combining folder's result is drawn in place of its rows, in their paint
  as a bake would leave it: the base's paint, and an added row's where that
  row has material; a cut or a kept face bare. Worked out at each vertex as
  a brick of the result is meshed (`result_paint.rs`), so nothing more is
  kept, and a stroke of paint on a row shows on the result as it lands.
- While the body in hand is inside it, the rows that cut or keep are drawn
  see-through, as the Others button draws bodies, so that what does the
  cutting can be seen, picked and sculpted: reddened for a cut, bluer for a
  keep (`hani_gpu::tint`).
- The result follows its rows as they change, a brick at a time: every body
  shares one lattice, so each operation is a pointwise `min` or `max` of two
  fields and a brick of the result depends on that brick of each row and
  nothing else. A stroke on a row, or a row moved with the gizmo, shows in
  the result as it lands. A change to the arrangement (a chip, a row moved
  in the list, an eye, an undo) builds every result again, on every core.

## Baking

**Bake** collapses a combining folder into one body that holds its result,
named as the folder was, where the folder was: the base, with each row
after it applied in turn, and nested folders collapsed first (a plain one
as the union of what it holds). The base keeps its paint and its mask; what
an added row brings is carried as a merge carries it, and a cut face is
bare. One undo puts the folder, its rows and the base's field back. A
folder with a hidden row is not baked: the row would be lost or left out,
and the status line says to show it or delete it.

## Saved

The arrangement is saved in a section of its own at the end of the file
(`HANBOOLS`): for each row that combines or does not add, its place in the
table and one byte. A document with none has no such section and is byte
for byte what it was. An older build reads a file with one as plain
folders, the section being past everything it knows. The results are not
saved; they are built again on load.

## Fields

`min(a, b)` for an add, `max(a, -b)` for a cut, `max(a, b)` for a keep, on
distances held in the narrow band. The cut and the keep are exact only near
the surface they make, which is all the band holds anywhere. The joins are
hard: there is no blend between rows.

## Moving a folder

With the Transform tool up, pressing a folder's row puts the gizmo on the
folder, about the middle of every body under it; pressing a body's row, or
picking a body on the model, puts it back on that body. A drag draws every
body under the folder, and a combining folder's result, through the drag's
map while the handle is held, and on the release each body is built again
at the whole placement from its field as it was when the gizmo armed, so
drags do not compound. Putting the gizmo away commits one undo entry for
the whole folder; Escape puts every body back. See `app/group_gizmo.rs`.

## Not done

- A varnish's finish on a row is not drawn on the result until it is
  baked.
- Picking a point on a result picks the row whose surface is there.
