# The paint rack, and painting a figure

A design, asked for by the maintainer on 2026-11-25. Built so far, each
with an "as built" part where its design is: stage 1, layers (end of
section 1); stage 2, the index and racks (end of section 3); stage 3,
brush sizes and the airbrush (section 5); stage 4, the sponge and the
one-click coats (end of section 7); stage 5, stamps and decals (end of
section 6); and stage 6, varnish (end of section 8). It covers:

- a searchable index of real hobby paints, from the
  [miniature-paints](https://github.com/Arcturus5404/miniature-paints)
  tables;
- racks: sets of paints a user chooses, swaps between and adds colours
  of their own to;
- layers, from the start: every coat, decal and varnish on a layer the
  user orders, hides, blends and redoes;
- the tools for laying paint: brush sizes, an airbrush, a sponge that
  works the way a sponge does;
- stamps and decals, dabbed on and then painted over or washed;
- three one-click coats: a base coat, a zenithal, and a spongechop;
- varnish: gloss, satin and matte.

Each stage at the end is one pull request. The open questions are listed
last.

**Decided by the maintainer, 2026-11-25:** spongechop is the slapchop done
with a sponge; layers are built first, properly, and everything else goes
on them; stamps and decals are dabbed on and can then be painted over or
washed.

## What there is today

The Paint mode already has a colour field and a set of touches. This
design builds on them rather than beside them.

- **A colour per voxel.** `hani-core/src/paint/field.rs`: four bytes a
  voxel through the narrow band, sRGB red, green and blue and a coverage,
  beside the distance field. It is one layer.
- **Slot recipes.** `hani-core/src/paint.rs`: up to 63 paints, each a
  base colour, a wash (pigment, stain, flow into creases, run off edges),
  a highlight, a zenithal amount and a `Finish` (Matte, Satin, Gloss,
  Metallic; saved but "not drawn yet"). These paint the one-byte slot
  field, which is what a filament printer reads.
- **Touches.** `hani-core/src/paint/touch.rs`: Loaded, Drybrush (catch,
  streak, bristle, runs out), Sponge (density, speck size, more on edges),
  Wash and Fill. Modes Paint, Glaze, Erase. Coat the body, Pick from
  model, an opacity, a soft edge.
- **The surface's shape is known.** The cavity (how concave the surface
  is at a point) is computed for washes and for the creases view, and the
  normal for the overhang view. A one-click coat reads both.

## 1. Layers

Paint is in layers from the first stage, and everything else in this
design is laid on one: a coat, a stroke, a decal, a varnish.

### What a layer is

A body has a stack of layers, bottom to top. Each layer has:

- a **name** ("Zenithal", "Spongechop", "Contrast: Blood Angels Red",
  "Decal: Aquila");
- a **field**: four bytes a voxel through the narrow band, sRGB colour and
  coverage, exactly the field `paint/field.rs` holds now. A layer is that
  field, more than once;
- a **blend**, which says how it lies over what is under it (below);
- an **opacity**, 0 to 1, over the whole layer;
- **visible** and **locked**.

The **active layer** is where strokes and dabs go. A one-click coat makes
a new layer above the active one, named for the coat, so it can be redone
or hidden alone. A decal makes a decal layer.

### Blends, and why the paint's kind lives on the layer

The point of layers done properly is that a layer above reads what is
under it at the moment it is drawn, not when it was painted. Change the
spongechop and the contrast over it changes with it, as on a real model.
So the blends that depend on what is under them are worked out when the
stack is put together, not baked into the layer:

| Blend | What it does | Used by |
|---|---|---|
| Normal | Covers what is under it by its coverage | Paint, metallic, primer, base coat, the zenithal, the spongechop's colours, a decal |
| Multiply | Darkens what is under it by its colour | A wash's tint, shading |
| Wash | Multiply, thicker in recesses and thinner on edges, by the cavity | Washes |
| Contrast | Tints by the value of what is under it: strong over light, nearly gone over dark; pools in recesses, thins on edges | Contrast and speed paints |
| Glaze | A film of the colour's hue over everything, that gives back half the lightness it takes: the hue shows over white, and over a colour the two mix as light through a film. A glaze stroke's share is its **Keeps light** setting | Stains, inks, glazes |
| Screen | Lightens | Glows, a highlight |

A paint's kind (section 4) picks the layer's blend when a coat is made or
a paint chosen, and the user can change it.

### Putting the stack together

The renderer and the exports keep reading one colour field, as they do
now: the **composite**, which is the stack put together, brick by brick.
A stroke or a change to a layer (its blend, opacity, visibility, order)
marks the bricks it touched, and only those are put together again. So
the renderer is unchanged, the frame cost of layers is the bricks a
stroke touches, and an export bakes the composite as it bakes the one
field today.

### What comes with them

- **Undo.** A stroke records the bricks of the layer it changed, as it
  records the paint field's bricks now, with the layer named. Adding,
  deleting, reordering, merging and a change of blend or opacity are
  undo entries of their own.
- **Sculpting under paint.** Every layer is carried as the paint field is
  now: Move and Bulk carry every layer with the surface; a field brush
  leaves every layer on its voxels. Issue 82 applies to all of them.
- **The file.** The project file holds the stack. A document from before
  opens with its paint as one layer, "Paint".
- **Merging.** Merge down, and flatten (the whole stack into one layer),
  which is also what makes a stack light again.
- **Memory.** A layer costs what the paint field costs now, four bytes a
  voxel in the band, only where it has paint. A figure with eight layers
  has eight of those at most. The stack is limited (16 a body to start,
  and a setting with the other limits of issue 74), and the panel says
  how much a layer holds.

### The layers panel

In Paint's settings tab, always open: the stack, top first, each with its
eye, its name (double click to rename), its blend and opacity, and a lock.
Drag to reorder. Under it: new layer, duplicate, delete, merge down,
flatten. Pressing a layer makes it active.

### Stage 1, as built (2026-11-25)

**The stack.** `hani-core/src/paint/layers.rs`. The active layer is the
volume's own paint field, so every tool, touch, stroke record and undo
that wrote the paint before writes the active layer now, unchanged. The
other layers sit beside it. A body with one plain layer (Normal, full
opacity, shown) is exactly what a body was before: no composite, no
extra bytes in the file, nothing extra in a frame. Sixteen layers a body.

**The composite.** Put together in `Volume::drain_dirty`, brick by brick,
in the bricks anything marked dirty, before they are meshed; the mesher
reads it. Arithmetic, on the shader's own `surface * (1 - a) + p` (sRGB,
as the field stores it): for each shown layer, bottom up, with `k` its
coverage times its opacity times, for a Wash or a Contrast, how much it
pools at that voxel, the colour under it is `CLAY * (1 - a) + p`, the
layer's blend turns its colour and that into a target, and then
`p = (1 - k) p + k target`, `a = a + k (1 - a)`. How much a voxel is a
crease is the field's Laplacian there, clamped to -1..=1, and is read
only where a shown layer's blend reads the shape. The clay is the
exports' (150, 140, 130).

**What carries every layer.** Move (each layer locked and dragged with
the material, one undo for all of them), Bulk, a shift, a warp, a turn, a
resample, a copy, a split (each half keeps its own share of each layer).
A merge of two bodies takes the other body's stack, flattened, into this
one's active layer: two stacks do not line up layer for layer.

**Undo.** A stroke records the bricks of each layer it changed, by the
layer's id. A change to the stack (add, duplicate, delete, move, merge,
flatten, a name, blend, opacity, eye or lock) is one entry holding the
whole stack before it. A drag of the opacity, or a name typed, is one
entry however many messages it is, by a revision count the history
keeps.

**The file.** A section of its own, `BRKLAYR1`, after the paint's
(`paint/layer_file.rs`), written only where a body has more than one
plain layer. The paint section before it holds each body's paint as it
is seen, so a reader from before layers opens a layered file looking as
it did, as one layer. An older file opens as one layer, "Paint".

**What reads which.** The renderer, the exports (PLY colours, the
texture bake, the 3MF filament choice) and Pick from model read what is
seen. A Fill runs by the likeness of the active layer's paint.

**The panel.** LAYERS, at the top of Paint's Settings tab, with a
heading that does not fold. The stack top first: an eye, the name (the
active layer's is a field to type in; another's is a button that makes
it active), its blend, and a lock. Under it: new, duplicate, up, down,
merge down, flatten, delete, then the active layer's blend and opacity.
A hidden or locked layer takes no paint, and the status line says so.

**Tested.** Properties: whatever sequence of adds, duplicates, deletes,
moves, merges, activations, opacities, paint and flattens, the stack
stays a stack (one to sixteen layers, the active one in it, each id
once) and putting back each state before a change in reverse gives back
the first; a stack whose blends do not read the shape, flattened, is
what was seen, and a layer moved up and back leaves it as it was; a
normal layer at full coverage hides what is under it; a multiply or wash
never lightens and a screen never darkens; a contrast is lighter over a
lighter undercoat; a wash pools deeper in a crease. Worked fixtures: a
stroke goes to the active layer and its undo comes off that layer only;
a stroke on every layer undoes on every layer; what is seen is the stack
put together, and hiding a layer shows what is under it; a shifted body
and both halves of a split keep every layer; a change to the stack
undoes and redoes through the history; a stack saves and opens, a file
cut before the section opens as the seen paint, and a flattened document
writes no section; a damaged section is refused at every length; Move
carries both layers of a two-layer body and one undo takes both back.
The application, by its messages: a coat goes on the active layer and
each undoes; a locked or hidden layer takes no paint; an opacity drag
and a typed name are one undo each; up, down, merge and flatten, and
their undos; a mask holds paint off a layer. On the screen: a red base,
a glazed blue layer over it, and the eye taking the blue off again.

**Not done in this stage.**

- Drag to reorder: Up and Down buttons instead.
- Double click to rename: the active layer's name is a field instead.
- Each row's own opacity: the active layer's is under the list.
- How much a layer holds, on the panel; and the limit of sixteen as a
  setting with the other limits of issue 74.
- A merge down or a flatten keeps a Wash or a Contrast at no crease: the
  pooling is lost, as merging a live effect in a painting program loses
  it.
- A merge of two bodies brings the other's stack flattened, not as
  layers.
- What the composite costs a frame is not measured: the build machine
  has no graphics card. By arithmetic it is a pass over the 32,768 voxels
  of each dirty brick for each shown layer with paint there.
- Painting by filament slot is not on layers (open question 8).

## 2. The paint index

### The data

The miniature-paints repository is MIT licensed (copyright 2022 Rick
Fleuren), so it can be bundled with credit. It is one markdown table a
brand: name, a product code for some brands, the product line ("Set"),
and the colour as red, green, blue and hex.

Counted from a clone on 2026-11-25: **11,443 rows from 34 brands**, every
row with a readable colour. 1,534 of them are Pantone and RAL, which are
colour references and not paints; that leaves **9,909 hobby paints**.

The index is generated at build time from the tables into a compact file
shipped with the application (a script under `scripts/`, rerun when the
tables change; the tables are not fetched at run time). Each entry:
brand, line, name, code, sRGB, kind (below), and flags (airbrush ready,
spray, discontinued, triad role).

### Kinds

The user asked for washes, stains, regular paints, metallics and
contrast or speed paints. The tables do not say which is which; the
product line usually does, and the name the rest of the time. A first
pass of the rules, run over every row:

| Kind | How it is told | Count |
|---|---|---|
| Paint (opaque) | Everything not below | 7,049 |
| Air (opaque, thinned for an airbrush) | Line says Air, Model Air, Game Air, Airbrush | 911, and 65 metallic |
| Metallic | Line says Metal, Metallic, Alchemy, Liquid Gold, Pearl, Glitter, Chameleon; or the name says gold, silver, steel, iron, bronze, copper, brass, chrome, alloy, mithril, pewter and the like, less a list of exceptions (Steel Legion Drab, Ironjawz Yellow, Golden Yellow, Pearl Skin...) | 790 |
| Contrast / speed | Line says Contrast, Speedpaint, Xpress, Instant Colors, One-coat | 274 |
| Wash | Line says Wash, Shade, Quickshade, Tone, Dipping Ink, Wash FX; or the name says wash | 219 |
| Spray | Line says Spray, Aerosol | 216, and 46 metallic |
| Primer | Line says Primer, Undercoat | 110 |
| Stain (ink, glaze, transparent) | Line says Ink, Glaze, Transparent, Candy, Inktensity, Intensity, Clear | 106 |
| Effect | Technical, Weathering FX, Soil Works, Special FX, texture | 92 |
| Dry (for drybrushing) | Citadel's Dry line | 30, and 1 metallic |
| Reference | Pantone, RAL | 1,534 |

Two traps the first pass fell into, and the rules now avoid:

- **Foundry's "Shade" is not a wash.** The Foundry Paint System sells
  three opaque paints a colour: Shade, Base and Light. Read as washes, 114
  opaque paints were misfiled. The index keeps Foundry's triad role as a
  flag instead, which is useful: it is a ready-made dark, middle and light
  for a spongechop or a zenithal.
- **"(blue Shade)" is a pigment.** Golden's and Liquitex's "Phthalo Green
  (Blue Shade)" names a variant of the pigment, not a wash. Only "wash" in
  a name counts.

The rules are a heuristic and will be wrong somewhere. The index carries
an **override table** (brand, line, name, kind) for the cases found, and
the user can change a paint's kind in their own rack. The classifier is
`scripts/paint_index.py`, committed with this design.

### Search

- **By name, brand, line and code**, forgiving: words in any order,
  prefixes, small misspellings. "khorne" finds Khorne Red; "70.950" finds
  Vallejo Black.
- **By kind and brand**, as filters beside the search field.
- **By colour.** Pick a colour (a swatch, a hex code, or the eyedropper
  on the model) and the index lists the nearest paints by a perceptual
  distance (OKLab, or CIEDE2000; see the questions), with the distance
  shown. This is also "what do I own that is closest to this".
- **Equivalents.** From any paint, its nearest equivalents in the other
  brands.

## 3. Racks

A rack is a set of paints a user controls: a name, an ordered list of
paints, and which of them are on the keys 1 to 8.

- **Racks are the user's.** Create, rename, duplicate, delete. Several
  are kept ("My Citadel shelf", "Necron army", "Skin tones") and one is
  the **rack in hand**: P in Paint brings it up at the pointer, keys 1 to
  8 pick from it, and its swatches fill Paint's library.
- **Swap racks** from a list at the top of the library, or from the P
  palette, in one click.
- **Add paints** by searching the index and pressing add, or by dragging
  a result onto the rack. Reorder by dragging. Remove from the rack,
  which leaves the index alone.
- **Custom paints.** A colour of the user's own: a name, a colour (hue,
  saturation and value, hex, or picked off the model), a kind (the same
  kinds as the index), and a finish. Saved in the rack it was made in,
  and copyable to others. A custom paint can be made from an index paint
  ("like Retributor Armour, but warmer").
- **Kept as files**, one a rack, in the data directory beside the
  brushes, so a rack can be handed to someone. A document keeps the
  paints it used (the colour and the kind, with the name), so a model
  opens with its paints on a machine that has none of the racks.
- **The keys 1 to 8** pick the rack's first eight, or whichever the user
  put on them (`app/keys.rs`, extended to Paint).

The slot recipes stay for filament printing. A rack paint can be turned
into a recipe (base colour, and for a wash or contrast the wash
settings) when a model is to be printed in filaments.

### Stage 2, as built (2026-11-25)

**The index.** `assets/paints/index.tsv`, 11,443 lines and 660 KB,
generated by `scripts/paint_index.py` from commit `22791a6` of the
tables, with their MIT licence beside it and a README crediting them.
It is built into the application and read the first time it is asked
for (`crates/hani-app/src/paint_index.rs`). The override table is
`scripts/paint_overrides.tsv`; it starts with three Citadel metallics the
rules filed as paint because their names do not say metal: Retributor
Armour, Screaming Bell and Ironbreaker. An airbrush or spray metallic is
filed as Metallic with its `a` or `s` flag.

**Open questions 1 and 2, taken this way.** Pantone and RAL are in the
index as the kind Reference, searched only when that kind is chosen.
Colour distance is OKLab's: the reference, CIEDE2000, costs several times
as much a pair, for no change in which paint comes first that matters
here.

**Search.** By name: words in any order, each the start of a word of the
name, brand or line, or part of a code ("70.950" is Vallejo Black), or
from four letters a word one slip away (a letter wrong, missing, extra or
two swapped: "khrone" finds Khorne Red). An exact name first, then a name
that starts with what was typed, then all the words in the name, then the
rest. By colour: the paints nearest the colour in hand, with the distance
shown in hundredths. A kind and a brand narrow either. Equivalents in
other brands are the colour search after a paint is tried: there is no
separate list.

**Racks.** `crates/hani-app/src/app/rack.rs`. Made, renamed,
duplicated, deleted and swapped at the top of Paint's library, which is
now the rack: its swatches (the first eight with their key on the
tooltip), the chosen paint with Up, Down and take off, Add paints (the
search, each result tried by pressing its name and put on by its plus),
and New colour (the colour in hand, a name and a kind; since moved into
the paint designer, below). Each rack is a
file, `racks/<name>.rack` in the data directory; which is in hand is
`rack.conf` in the settings. A first run has a Starter rack of twelve
Citadel paints, a wash and a contrast among them. The fixed range of
swatches the colour brush had is gone; the rack is in its place.

**Keys and P.** In Paint, painting in colour, the keys 1 to 8 pick the
rack's first eight, and P brings up the rack over the touches. Painting
by filament slot, the keys are the touches as before.

**The kind chooses the blend.** Choosing a paint takes its colour, and
if its kind wants another blend than the active layer's (a wash, Wash; a
stain, Glaze; a contrast, Contrast; anything else, Normal), an empty
active layer takes that blend, and one with paint on it gets a new layer
of that blend over it, named for the paint. One undo entry.

**Tested.** Properties: a search for any paint's exact name finds a
paint of that name first; the nearest paint to any paint's colour is no
distance from it and each after is no nearer; distance is symmetric and
none from itself; any rack written to its file reads back as it was.
Worked: the whole index reads and has the design's counts; the design's
examples ("khorne", "70.950", "nuln oil", a slip, words out of order)
find their paints; references only when asked for; a kind and a brand
narrow; racks kept as files come back, a rename is one file, a delete
is gone; the starter rack is whole. The application by its messages:
key 3 picks the third paint; a wash makes an empty layer a wash and a
painted one gets a wash layer named for it, which is one undo; racks
made, filled, a paint on once, a colour of the user's own by its name,
ordered, emptied, swapped, duplicated and deleted down to one; a paint
tried is not put on. On the screen: the rack, the search by name and by
colour, and a wash's tooltip and the layer turning to Wash.

**Not done in this stage.**

- Dragging a result onto the rack, and dragging to reorder: buttons.
- A custom paint's finish, and making one from an index paint by
  changing it ("like Retributor Armour, but warmer"): the colour in hand
  is named instead, which can be an index paint's after trying it.
- Copying a custom paint to another rack, except by duplicating the rack.
- A document keeping the paints it used.
- The keys put on other paints than the rack's first eight.
- Turning a rack paint into a filament recipe.
- A user changing a paint's kind in their rack.

## 4. How each kind of paint lays

The kind is not only a label: it decides how a paint goes on, and which
blend its layer takes (section 1). The user can still change either.

| Kind | Coverage | Into recesses | Off edges | Finish |
|---|---|---|---|---|
| Paint, Air | Opaque, builds in one or two coats | No | No | Matte |
| Metallic | Opaque | No | No | Metallic |
| Wash | Thin over flats | Pools in creases, by the cavity | Runs off | Satin (most washes dry with a sheen) |
| Stain | Even tint over everything, what is under it shows | Slightly | No | Gloss to satin |
| Contrast / speed | Tints by what is under it: bright over a light undercoat, nearly gone over dark | Pools strongly | Thins on edges, so the undercoat shows as a highlight | Satin |
| Primer | Opaque, one coat | No | No | Matte |
| Effect | As paint, flagged | | | As made |

The contrast blend is what makes a spongechop or a zenithal under it
worth doing: its colour multiplies the value of what is under it.

## 5. Tools

The Paint mode's rail becomes: **Brush, Airbrush, Sponge, Decal, Fill,
Mask, Measure**, with Brush the default (painting is still what Paint
does when nothing else is chosen).

### Brush sizes

Named presets with real sizes, in millimetres on the model, which a
painter will know by their brush: **Detail** 0.3, **Fine** 0.6,
**Standard** 1.2, **Base** 2.5, **Large** 5. Each also keeps a falloff
and a flow; a user can change them and add their own, as Sculpt's brushes
are kept. The drybrush has its own three: small, medium, large.

### Airbrush

Paint sprayed rather than laid:

- **A cone from the view.** Each frame the button is held, paint lands
  on what the cone from the pointer reaches, more at the middle (a soft,
  Gaussian spot), less where the surface turns away from the spray.
- **Builds with time.** Holding it still darkens a spot; a quick pass
  barely tints. Flow per second, and pen pressure as flow.
- **Distance.** The spot's size follows how far the surface is from the
  camera's near plane, as moving an airbrush away widens its spray.
- **Overspray**, a faint wide halo, can be turned off.
- Respects the mask, so a stencil is the mask.

### Stage 3, as built (2026-11-25)

**Sizes.** A row under Radius in Paint's Brush section, and on the P
palette: Detail 0.3, Fine 0.6, Standard 1.2, Base 2.5 and Large 5, and
with the drybrush in hand its own Small 1.5, Medium 3 and Large 6. The
numbers are the brush's **radius** in millimetres, the number the Radius
control already shows, so a size and the slider agree. The one the radius
is, is lit. A size sets the radius and nothing else: the falloff and
flow kept with each, and sizes of the user's own, are not done.

**The airbrush** is a sixth touch, beside Loaded, Wash, Drybrush, Sponge
and Fill, in colour only (`crates/hani-core/src/paint/spray.rs`,
`crates/hani-app/src/app/airbrush.rs`). The rail is not changed in this
stage: the touches are where a way of laying paint is chosen today, and
moving them all onto the rail is one change, not six.

- Each frame the button is held, and at each move of the pointer, it
  sprays for the time since it last did, at the pointer, along the view:
  a Gaussian spot round the line from the camera through the pointer,
  less where the surface turns away, nothing on a surface behind the one
  it meets. So a spot held still fills in and a quick pass barely tints.
  Between two moves further apart than half a spot, the time is shared
  out along the way: sprayed only at each event, a quick sweep left a row
  of dots on the screen.
- **Flow**, in coats a second at the middle of the spot (0.1 to 4, 1 to
  start), times the pen's pressure.
- **Distance**: the spot is the brush's radius where the surface is as
  far from the camera as the point the camera turns about, and wider in
  proportion further away (a quarter to four times).
- **Overspray**, on to start: a halo a twentieth as strong reaching twice
  as far.
- The mask holds it off, so a stencil is the mask; a hidden or locked
  layer refuses it; the whole hold is one undo.

**Tested.** Properties: the spot is strongest at its middle and never
stronger further out. Worked: held still it lays more than a quick pass;
the far side of a ball takes nothing; the mask is a stencil; overspray
reaches further. The application by its messages: a press lays paint,
held it builds, let go it stops, and the hold is one undo; the middle of
a sweep wider than the spot takes paint; painting by slot has no
airbrush; a size sets the radius. On the screen: a spot held two and a
half seconds, a slow sweep, a quick one, and the size row.

**Not done in this stage.** The rail of tools (Brush, Airbrush, Sponge,
Decal, Fill); sizes of the user's own, with a falloff and a flow each;
the airbrush's own spot shape beyond the Gaussian; a measured cost of a
spray on a large spot (the build machine has no graphics card; by
arithmetic a spot of radius r looks at a box of (2.5 r / voxel)³ voxels
with overspray on, about 110,000 for a 3 mm spot at 0.25 mm voxels).

### Sponge, as a sponge works

Today's sponge lays specks over the brush's disc. A sponge on a figure
does more than that, and the technique this design follows is the
spongechop, which is the slapchop done with a sponge:

1. a dark undercoat everywhere;
2. a mid value dabbed on with a torn foam sponge, blotted nearly dry, so
   it catches the raised areas and the tops of forms and misses the
   recesses;
3. a light value dabbed the same way, on less of the surface: the
   highest, most upward facing parts;
4. a translucent contrast paint over all of it, which takes its light and
   dark from what is under it.

So the reworked sponge:

- **Prefers what stands up.** A speck lands with a chance that grows
  with how convex the surface is and how high it is above its
  surroundings, and falls in recesses (the cavity). A "reach" setting
  says how far into a recess it may go.
- **Prefers what faces the light**, optionally: a direction, as the
  zenithal has, so dabs land more on top faces.
- **Builds with dabs.** Each dab adds specks; the same spot dabbed again
  fills in, and the edge between covered and bare stays broken.
- **Speck size and density** as now, and a pattern: fine foam, coarse
  foam, torn edge.

## 6. Stamps and decals

A picture dabbed onto the model, as a waterslide decal or a stamp is on a
real one, and then painted over or washed like anything else.

- **Dabbed on.** Press where it goes, drag to size and turn it, as
  Sculpt's stamps are placed; it is projected along the view onto the
  surface, and wraps the surface it lands on. Each dab is one undo entry.
- **On a decal layer.** A dab goes on the active layer if it is a decal
  layer, or makes one ("Decal: Aquila") above the active layer. Layers
  above it paint over it; a wash layer above it runs into its recesses
  and stains it; a varnish over it hides its edge, as matte varnish over
  a glossy decal does.
- **Two kinds of picture:**
  - a **decal** keeps its own colours (a picture with transparency:
    insignia, numbers, freehand, eyes, tattoos);
  - a **stamp** is a stencil laid in the paint in hand: dark is paint,
    light is none, grey is part (hazard stripes, camouflage, runes,
    patterns).
- **Its edge**: hard, as a decal's film; or soft, faded over a share of
  its size.
- **A decal has a finish**: a fresh decal is glossy, as a real one is,
  until a varnish layer over it says otherwise.
- **Where pictures come from**: a set that comes with the application, a
  folder of the user's own, and any picture taken in, kept as the brushes
  are.
- **The mask** limits a dab, so a decal can be kept off a part.

### Stage 5, as built (2026-11-25)

A seventh touch, **Decal**, in colour only (`paint/decal.rs` in the core,
`app/decal.rs`). The touches are laid out three, two and two: four to a
row clipped "Wash", and three, three and one left Decal alone on a line.

- **Dabbed on.** Press where it goes; drag away to size it (the picture
  is twice as wide as the drag, at the depth of the point pressed) and
  round to turn it. It is dabbed again at every move, over the paint as
  it was before the stroke, and a voxel the last dab reached that this
  one does not is given that paint back, so the drag shows it as it will
  be and what is left is one dab. A click dabs it at the brush's size,
  unturned.
- **Projected along the view.** The picture lies square to the view
  through the point pressed and is carried onto the surface it meets,
  within half its width in front or behind, fading out as the surface
  turns from the view.
- **On a decal layer.** The active layer if its name starts "Decal", or
  a new one over it, "Decal: Star" and so on. The new layer and the dab
  are one undo entry (`LayersAdded` and the stroke in one `Entry`, which
  undoes in reverse).
- **Decal or stamp.** Any picture can go on as either: a decal in its
  own colours by its own alpha, a stamp as a stencil in the paint in
  hand, dark being paint. Choosing a built-in picture chooses which it is
  made for.
- **Its edge**: *Soft edge*, a share of its size, none to start.
- **Pictures**: six that come with the application, drawn in code: three
  decals (Star, Eye, Roundel) and three stamps (Hazard, Camo, Dots); and
  *Picture…*, which takes in any PNG or JPEG and keeps a copy in
  `decals/` in the data directory, read back at every start.
- **The mask** keeps a dab off, as it keeps a brush off.

**Tested.** Properties: a soft edge only ever fades, and a full turn
lands every point where it was. Worked: a decal lays its own colour on
the surface it meets and not on the back of the ball or outside the
picture; a stamp lays the paint in hand where the picture is dark; a
smaller dab in the same stroke gives back what a larger one took, and
the dab is one undo; the mask keeps a dab off; every built-in picture has
something in it, the stamps both dark and light; the star has five
points. The application by its messages: a dab makes "Decal: Star", a
longer drag covers more, the layer and the dab are one undo, and a
second dab goes on the same layer; a built-in stamp is chosen as a
stamp; a picture taken in is chosen; the edge is held to its range. On
the screen: a star dragged larger and turned, which showed four points
where five were drawn (the angle's fraction kept its sign; fixed, and
tested), and a hazard stamp in red.

**Not done in this stage.** A decal's own finish (glossy until varnish),
which waits for varnish; a decal layer as a kind of its own rather than
a name; the pictures shown as pictures on their buttons; removing a
picture taken in, except from the directory; turning by a second press
rather than the drag's direction.

### After use: a decal previewed while it is dragged (2026-11-25)

The maintainer found a decal slow as it grew, and asked whether it could
be drawn as a decal before it is pressed into the layer. It was dabbed
again at every move of the drag, and a dab asked every voxel of the cube
round the picture, solid and empty alike, with the trilinear gradient:
the cost grew with the cube of its size.

**Drawn, then laid.** While the button is down the dab is drawn over the
model and not laid: a grid of 48 by 48 over the picture, each corner cast
along the view onto the surface and lifted a voxel toward the eye, each
cell in the colour the dab would lay there (`Hani::decal_preview`).
When the button comes up it is laid once, on its decal layer, in the
stroke whose entry the layer joins (`Hani::set_the_decal_down`, called
from the end of a stroke).

**A dab a brick at a time.** `Volume::dab` passes over a brick that holds
no surface whole, works out the rest across every core, and takes the
normal by six lookups, as the airbrush does.

| A decal across | One move, before | One move, after | Laid on release, after |
|---|---|---|---|
| 4.5 mm | 3.4 ms | 2.8 ms | 4.9 ms |
| 11 mm | 15.0 ms | 7.3 ms | 9.1 ms |
| 21 mm | 57.6 ms | 10.8 ms | 12.9 ms |
| 41 mm | 261 ms | 15.5 ms | 27.4 ms |

Measured by `what_a_decal_costs` (application, release build, ignored),
on the build machine, which has no graphics card: what one adds to draw
the preview was not measured.

Tests: the dab and its layer are still one undo, and a dab dragged three
times as far is larger; a point of the picture put on the plane comes
back as the same point (a property, which the preview is laid out by).
On the screen: a star dragged out, drawn while the button was held and
laid when it came up, its points within a few pixels of the preview's.

Not done: the preview is drawn on the surface as it is, so it does not
show a soft edge's fade or what the mask holds off; it is coarser than
the dab, 48 cells across whatever its size.

### After use: what a drybrush would catch, shown under the brush (2026-11-25)

The maintainer asked for drybrushes to "show how high you are above a
surface so you can see what parts you are gonna hit", and then for an
indicator rather than a full preview if a preview cost too much.

A drybrush here is not held at a height: it catches what stands up into it
by as much as its *Catch* says, from the sharpest edges at the least to
every raised form at the most (`paint/touch.rs`, `Drybrush::amount`). A
ring drawn at a height would show a rule the brush does not follow. So
what is shown is its answer: with the Drybrush touch, the pointer over
the model and no button down, each point of the surface under the ring
that a stroke would leave paint on is tinted in the ring's orange, the
more it would take the stronger, asked as a stroke asks it with a full
load and no direction of travel (`Volume::drybrush_would_catch`,
`app/dry_preview.rs`). Moving *Catch* moves what is tinted.

It is not too dear for a pointer move: `what_the_catch_costs` (release,
ignored) on the edge of a box, 0.13 ms at a 2 mm brush, 1.1 ms at 5 mm,
5.7 ms at 10 mm, on the build machine, which has no graphics card.

Tests: on a cube the edge shows and the middle of a face does not, and
more catch never shows less; in the application, over a box's edge
something is tinted and more catch tints no less, over the middle of a
face nothing is, and with another touch nothing is. On the screen: a box
inserted on the ball, its top edge tinted under the ring and its faces
not.

Not done: the bristles' streaks and the side a stroke travels toward are
not in what is shown (it is the middle of both), and a smooth ball, which
stands up nowhere, shows nothing, as a drybrush catches nothing there.

## 7. One-click coats

Each is a small card (its few settings and a live preview on the model)
with Apply, which is one undo entry. Each works on the active body, or on
the masked part of it, or on every visible body.

### Base coat

The body, all of it, in one paint: what Coat the body does now, given a
paint from the rack and the paint's kind (a primer is a primer), on a
new layer "Base coat" at the bottom of the stack unless one is chosen.

### Zenithal

A primer value from a light direction:

- **Where the light comes from.** A sun dragged on a small dome over the
  model, or "from above", or "from the view as it is now". Default:
  straight down, slightly toward the front of the model.
- **Two or three primers:** dark (default black) for what faces away,
  light (default white) for what faces the light, and an optional middle
  (grey) between.
- **How it spreads.** A spray cone angle (narrow: hard top light; wide:
  soft), and whether other parts of the model **shadow** each other (an
  arm over a chest leaves the chest darker under it), which is a ray
  toward the light from each surface point.

The value at a point is how much it faces the light, through the cone's
falloff, times the shadow, through the dark, middle and light stops. It
is laid as its own layer, "Zenithal", so it can be painted over or
redone, and a contrast layer over it reads its value.

### Spongechop

The four steps of the sponge above, in one press:

- **Three colours:** dark (recesses), middle and light (default black,
  grey, white; or a Foundry triad, or any three from the rack).
- **The weight at each point** is a mix of three things the engine
  knows: how high it stands over its surroundings, how much it faces the
  light (the zenithal's direction), and how far out of a recess it is
  (the cavity). Recesses stay dark; the higher, more upward parts get
  more of the light.
- **Sponged, not smooth.** The middle and light are laid as specks with
  the sponge's pattern, thresholded on the weight, so the gradient is
  broken the way a real sponge leaves it.
- **Then a contrast, optionally:** pick one from the rack and it goes on
  as the fourth step.
- **On layers:** the dark, the middle and the light are a layer each
  ("Spongechop: dark" and so on) and the contrast a layer of its own with
  the contrast blend, so changing the light afterwards brightens the
  contrast over it.

### Stage 4, as built (2026-11-25)

**The sponge** (`paint/touch.rs`) gained three settings beside covers,
speck and more on edges: **Into recesses** (how far into a crease it
reaches: at none a full crease takes nothing, at all a recess takes what
a flat does; a third to start), **Toward the light** (how much a face
turned from the light is passed over; none to start), and the **foam**:
Fine, Coarse (specks twice the size), Torn (the same noise read where
a coarser noise pushes it, so the share caught is unchanged and the
edges are ragged), or Smooth (no specks: every point takes the share it
is owed as a thinner coat, and the Speck slider goes; added 2026-11-27).
The drybrush's **bristle marks** switch, off, sets Streaks to none and
hides Streaks and Bristle: the paint goes on wherever the shape says. The light is the coats' light. Each dab is a new
seed, as before, so dabbing again fills in.

**The coats** (`paint/coats.rs`, `app/coats.rs`), in a COATS block under
LAYERS in Paint's settings. The light is set once for all of them:
*From above* (75 degrees up, toward the front), *From the view* (toward
the camera now), or by *Light round* and *Light up* in degrees. The
dragged sun on a dome is not done. Each coat's colours are swatches that
take the colour in hand when pressed, so they come from the rack.

- **Base coat**: the colour in hand over all of the body, on a new layer
  "Base coat" put at the bottom of the stack.
- **Zenithal**: a new layer "Zenithal" over the active one. A point's
  value is how squarely it faces the light, `(n . l + 1) / 2`, raised to
  `90 / cone` (a narrow cone is a hard top light), through the dark,
  optional middle and light primers. **Shadows** is a box, off to start
  (open question 3): a ray toward the light from each point, stepping by
  the field's own distance, and a point something stands in front of
  keeps a quarter of its light.
- **Spongechop**: four layers over the active one, "Spongechop: dark"
  (a full coat), "middle" and "light" (sponged), and "contrast" with the
  Contrast blend if the box is ticked. A point's weight is 0.45 of how
  much it faces the light, 0.3 of how high it stands along the light over
  the body's reach, and 0.25 of how far out of a crease it is (from the
  field's Laplacian). The middle covers a share rising over weights 0.3
  to 0.6, the light over 0.6 to 0.85, each thresholded on the sponge's
  own speck noise with a soft rim, so the gradient is broken as a sponge
  leaves it. The sponge in hand is the one used. Each press is a new
  speckle.

Every coat respects the mask, and is one undo.

**Undo, made smaller.** A coat over a whole figure adds layers covering
all of it, and stage 1's undo kept the whole stack for every change: on
the application's starting ball three coats outgrew the history's
budget and pushed the first off, which a test caught. Now a change that
only adds layers (a coat, a new layer, a duplicate, a paint's new layer)
keeps their ids (`Change::LayersAdded`; undone, their paint goes to the
redo side as `Change::LayersTaken`), and one that changes only settings
or order (eye, lock, blend, opacity, name, up, down) keeps the settings
(`Change::LayerSettings`), which also ended a clone of every layer's
paint at each step of an opacity drag. Merge down, delete and flatten,
which throw paint away, still keep the whole stack.

**Tested.** Properties: the sponge never takes less where the surface
stands up more or faces the light more, a recess takes what a flat does
at full reach and nothing at none, and every foam covers its density on
a flat; a zenithal is never darker facing the light more, nor with a
wider cone; a spongechop never earns a recess more light than the peak
beside it, and its light never covers more than its middle; layers
arranged and arranged back are as they were. Worked: a base coat at the
bottom covering the body; a zenithal light on top, dark below, held off
by the mask; shadows darken the top of a ball under another; a
spongechop's layers, dark everywhere, its light far more on top; layers
taken out and put back, and taking every real layer leaves the implicit
one. The application by its messages: each coat is its layers and one
undo; a coat's swatch takes the colour in hand; the light from the view
is toward the camera and the sponge takes it; a zenithal is seen light
on top and dark below. On the screen: the block, a zenithal (dark seen
from below), and a spongechop on the starting ball.

**Not done in this stage.** The sun dragged on a dome; a live preview
before Apply; a coat on every visible body at once (it is the active
body, and the mask for a part of it); a Foundry triad or three paints
picked for the spongechop in one go (each swatch takes the colour in
hand); the spongechop's height "over its surroundings" is height along
the light over the whole body, not over a neighbourhood; what a zenithal
with shadows costs on a large model is not measured (by arithmetic, a
ray of up to the body's length over the light, stepped by the field's
distance but at least a voxel, for each voxel of the band).

## 8. Varnish

A finish over paint: **gloss, satin or matte**, brushed on like paint or
coated over the whole body in one click.

- **A varnish layer** holds a finish per voxel and no colour: matte,
  satin or gloss. The finish of a point is the top visible varnish that
  covers it; under no varnish it is the paint's own (matte for most,
  metallic for a metallic, glossy for a fresh decal). A varnish layer can
  be brushed, or coated over the whole body in one click.
- **Drawn.** The viewport's shading takes a roughness from the finish (a
  wide, dim highlight for matte, a tight bright one for gloss) and a
  metalness for metallic. `Finish` is already saved on a slot recipe and
  "not drawn yet"; this is where it is drawn.
- **Exported.** The GLB and OBJ exports carry a roughness and metalness
  map beside the colour (the bake of stage S5 has the colour; this adds
  the rest), so a render elsewhere shows the varnish.
- Typical use, which the defaults follow: matte over everything, gloss on
  eyes, gems, wet things and blood.

### Stage 6, as built (2026-11-25)

**A varnish is a blend.** `Blend::Varnish`, the seventh, on any layer:
it lays no colour (the composite passes over it), and its paint's red is
a finish's code, `Finish::code`: 1 matte, 2 satin, 3 gloss, 4 metallic,
nought none. The stack puts a second field together beside the colour,
the finish: at each voxel, the top shown varnish with at least half a
coat there. A merge down refuses a pair with a varnish in it; a flatten
keeps every varnish as a layer over the paint put together. A reader
from before this stage refuses a layers section with a varnish in it and
opens the paint as it was seen.

**Laid.** COATS has *Varnish*: Matte, Satin or Gloss, and Apply, a new
layer "Varnish: Gloss" and so on over all of the body, one undo. On a
Varnish layer the brush and every touch lay the finish chosen there,
not the colour in hand, so a varnish is brushed on as paint is (gloss on
eyes and gems, matte over the rest).

**Drawn.** Each vertex carries the finish at its cell in byte 3 of the
attribute buffer, which was spare. The shader adds a highlight from a
light above and left of the camera, as tight and bright as the finish is
smooth (`Finish::roughness_and_metal`: 0.9, 0.5, 0.15 and 0.35 rough;
metallic is metal), and a metal's highlight in its own colour with less
of it lit flat. Drawn flat, a varnish changes nothing.

**Exported.** A GLB of a varnished body carries a second picture, its
roughness and metal as glTF packs them (green and blue), named by its
material as `metallicRoughnessTexture` with factors of one. A body with
no varnish is the matte non-metal it was. The bake is the base colour's
(`Unwrap::baked_by`), reading the finish at each texel's point.

**Tested.** A varnish coat lays a finish and no colour, hidden it is no
finish, and a flatten keeps it; a GLB with a roughness picture names it
and its bytes come back, and a body after it with none still reads back
whole; the application's GLB of a gloss-varnished ball has the gloss's
roughness over its texels (read with serde_json and the image crate);
the app's varnish coat is a layer and one undo, and a stroke on a
Varnish layer lays the finish; rendered on llvmpipe, a gloss varnish
puts a brighter highlight on the ball than the clay has, changes no
shape, and drawn flat changes nothing. On the screen: the ball before
and after a gloss coat.

**Not done in this stage.** A paint's own finish where no varnish is
(metallic for a metallic, glossy for a fresh decal); a slot paint's
saved finish drawn; the OBJ+PNG export's roughness (its MTL has no map
for it that every reader takes); a varnish hiding a decal's edge, which
needs the decal's own gloss first.

### After use: pooling, effects by kind, and metal (2026-11-25)

The maintainer used the six stages and found that washes and contrasts
did not pool, that a paint's kind should already choose how it goes on,
and that metallics did not look metallic.

**Pooling by occlusion** (`paint/occlusion.rs`). A Wash or Contrast layer
pooled by the field's Laplacian at a voxel, which sees a crease a voxel
or two wide. Now it pools by how shut in the surface is: six rays from
just off the surface across the hemisphere it faces, stepped out at 0.5,
1, 2 and 4 mm, a ray that meets solid counting the more the nearer. The
Laplacian is kept for an edge, which the occlusion cannot tell from a
flat. It is worked out only where a pooling layer has paint, when the
stack is put together. A test: a dark wash over a light coat on two balls
side by side is seen darker where they meet than on top.

**The touch is the effect.** A seventh way of putting paint on,
*Contrast*, beside *Wash*. Both lay paint where the brush goes; choosing
either gives the active layer that blend (an empty layer takes it, one
with paint gets a new layer over it), and the blend does the pooling, so
it is not done twice. Choosing a paint chooses its touch: a wash
*Wash*, a contrast *Contrast*, an air or spray paint the *Airbrush*, a
dry paint the *Drybrush*. Any touch can be chosen after: a contrast
drybrushed stays on its contrast layer. A plain paint keeps the touch in
hand, unless that was a wash's or a contrast's.

**Metal.** A layer can be *metal*, kept in the file as a flag. Its paint
has the metallic finish where it is the top paint with half a coat or
more; a plain coat over it takes the finish away and a varnish over that
gives its own. A metallic paint from the rack goes on a metal layer. The
shader draws metal as mostly reflection: dark where it faces away from
the light above, bright where it catches it, a hard highlight in its own
colour and a thin rim; a render test measures a metallic gold ball's
spread of light from its darkest to its brightest at more than half as
much again as the same gold as paint, with a highlight that is gold and
not white.

### After use: pooling as strong as you want it (2026-11-25)

The maintainer found the pooling too faint for a contrast or a speed
paint to work: they "pull in and around details", and a wash does not
cover high spots as well as a speed paint. Asked for: an occlusion
intensity slider for the paint engine.

**Pooling**, a number in the LAYERS block from 0 to 3, is how strongly a
Wash or Contrast layer pools by the occlusion: the occlusion is
multiplied by it before it is a crease (`occlusion::crease_of`). It is on
every body (`Volume::set_pooling`, which puts the pooling layers together
again), a body made later is given it on the next frame, and it is kept
as a setting, `paint.conf`, when the slider is let go
(`app/pooling.rs`). It was 1 before; it is 1.5 unless set.

**Deeper in a recess.** Pooled paint is more paint: a wash or a contrast
in a crease is now darker by up to seven tenths of a second coat of
itself, by how shut in it is, as well as covering more. A wash still
leaves about a third of itself on a flat and a contrast nine tenths.

The paint pictures follow the pooling in hand, to a tenth.

Tests: with more pooling a wash is darker along the line from the neck
between two balls to the top of one, at none only the edge of the top
tells them apart (`pooling_more_darkens_the_recess_more`); a wash and a
contrast are deeper in a crease and a plain paint is not; the slider sets
every body, and a body added after is given it on the next frame. On
the screen: *Pooling* under the layer's opacity; a strip of a wash and a
contrast at 0, 1.5 and 3.

Not done: the reach of the occlusion (four steps out to 4 mm) is not on
the slider; pooling is not kept in the project file, so a document
exported on two machines with different settings differs.

### After use: the paint designer, and a picture of each paint (2026-11-25)

Asked for by the maintainer: a paint should be shown as a brush is, on a
textured ball slapchopped dark in its recesses and near white on its
highlights, and the colour's sliders, a mixer and the paint's effect
should be in a designer card, as a brush has its block.

**A paint's picture** (`app/paint_preview.rs`). The ball is a real body:
a sphere with twenty-two shallow bumps of varied size joined over the
face it shows, laid on a spiral so they are even. It is ray-marched once
a size, and each pixel keeps the way it faces and how shut in it is (the
same `crease_at` a Wash or Contrast layer pools by). Its undercoat is a
slapchop of those: a dark grey coat, and near white where the surface
faces up, faces the light and stands out of a recess. A paint's picture
is each pixel's undercoat with the paint laid over it by its kind's
blend, by the stack's own arithmetic (`layers::over_undercoat`), and lit;
a metal is lit as the viewport lights one. So a wash is seen pooling in
the recesses and leaving the highlights, a contrast taking its light from
what is under it, a metal reflecting. Each paint's picture is made the
first time it is drawn and kept. They are the rack's swatches, the chosen
paint's, and the designer's large one. Eight big bumps, the first ball,
read as a cluster of berries at the rack's size.

**Contrast, stronger on a flat.** A contrast layer laid three quarters
of a coat on a flat; over the near white of the picture's highlights that
read as a pastel. It is now nine tenths on a flat, a little more in a
recess and less on an edge.

**The designer** (`app/designer.rs`). A card over the window, opened by
*New paint* under the rack (from the colour in hand), by *Edit* on the
chosen paint, or by pressing the colour in hand's swatch. It holds the
picture and the touch the kind puts it on with; the paint's name and
kind; its colour as a swatch, a hex field and three channels; the
machine's filament colours to start from; and a mixer: one of the rack's
paints mixed in, in linear light, by as much as is set. *Save* puts the
paint on the rack, in place of the one it was opened on or at the end,
and takes it up, which chooses its touch and suits the layer. The panel
keeps what is used while painting: the rack, the touches, the colour in
hand with *Pick from model*, the opacity. The three channels and the
filaments that were on it are in the designer, and *New colour* under the
rack is gone into it.

Tests: a paint designed from the colour in hand goes on the rack by its
name, kind and colour and is taken up (a contrast with the Contrast
touch); one with no name is refused; a rack paint changed is changed in
its place; a mix lies between its two colours, for any two (a property);
the ball covers about a disc, has a dark recess and a near white
highlight; a wash's picture spreads more light than a paint's, and a
metal's more than the same colour as paint.

Not done: a finish (matte, satin, gloss) of a paint's own, which is
still a varnish layer's; mixing more than two; a paint made from an index
paint ("like this one, but warmer") other than by trying it and pressing
the swatch in hand; a hue and saturation picker.

## 9. Where it goes in the window

- **Paint's library tab** is the rack: the rack's name and a list to swap
  racks, its swatches (each a picture of the paint on a slapchopped
  ball, with the keys 1 to 8 on them), Add paints (which opens the index
  search), and New paint, which opens the paint designer.
- **P** brings the rack in hand up at the pointer.
- **Paint's settings tab**: the layers panel (section 1), always open at
  the top; then the tool in hand's settings (size preset, flow, sponge
  pattern, airbrush spray) and the paint in hand (kind, finish).
- **The one-click coats** are buttons in a COATS section of the settings
  tab, and in a Paint menu: Base coat, Zenithal, Spongechop, Varnish.
- **The rail**: Brush, Airbrush, Sponge, Decal, Fill, Mask, Measure.

## 10. Stages

1. **Layers.** The stack on every body (name, blend, opacity, visible,
   locked, active), the composite brick by brick, undo per layer, the file
   (an old document opens as one layer), Move and Bulk carrying every
   layer, the layers panel, merge down and flatten. Today's paint and
   touches paint the active layer.
2. **Index and racks.** The generated index (with the classifier and the
   override table), search by name and colour, racks (make, swap, add,
   reorder, files), custom paints, keys 1 to 8 on the rack, the kind
   choosing the blend.
3. **Brush sizes and the airbrush.**
4. **The sponge reworked, and the one-click coats**: base coat, zenithal,
   spongechop, each on its own layers.
5. **Stamps and decals**, on decal layers.
6. **Varnish**: varnish layers, the finish drawn, and the exports'
   roughness and metalness.

How each is tested, as the rest of the studio is: properties where a
rule holds for any input (a zenithal never makes a surface facing the
light darker than one facing away; a spongechop never puts more of the
light in a recess than on the peak next to it; a search for a paint's
exact name finds it first; the nearest paint to a paint's own colour is
itself; a contrast layer over a lighter layer comes out lighter; moving
a layer up and down and back leaves the composite as it was; a stack
flattened is the composite), the application driven by its messages, and
a look on the screen.

## Open questions

1. **Pantone and RAL**: in the index as colour references (searchable,
   not paints), or left out?
2. **Colour distance for search**: OKLab (cheap, good) or CIEDE2000 (the
   reference, heavier)?
3. **Zenithal shadowing**: a ray toward the light from every surface
   point costs time on a big model; on by default, or a box to tick?
4. **Contrast paints**: is "tints by the value under it, pools in
   recesses, thins on edges" close enough, or should a contrast's
   behaviour be measured from swatches of real ones?
5. **Racks and filaments**: should a rack also say which filament each
   paint prints in, for a printer that prints colour?
6. **Decals that come with it**: which pictures?
7. **Layers across bodies**: is a layer one body's, or can a layer
   ("Varnish: matte") cover several bodies at once?
8. **The slot recipes**: answered by the maintainer on 2026-11-25:
   painting by filament slot is removed. An old document's slots are laid
   into its colour paint when it opens (`paint/old_slots.rs`).

## Sources

- The paint tables: [Arcturus5404/miniature-paints](https://github.com/Arcturus5404/miniature-paints), MIT.
- Sponging on miniatures: [Goonhammer, Hobby 101: Sponge Painting](https://goonhammer.com/hobby-101-sponge-painting), [FineScale, chip paint with a sponge and microcell foam](https://finescale.com/how-to/articles/2022/05/chip-paint-with-a-sponge-and-microcell-foam/), [Age of Miniatures, stippling](https://ageofminiatures.com/stippling-technique-for-miniatures/).
- Slapchop: [Loot Studios](https://blog.lootstudios.com/paint/slapchop-and-how-to-paint-an-army-of-minis-really-fast/), [Empire of Minis](https://empireofminis.com/blogs/miniature-painting-and-basing/slapchop-painting-technique-the-fastest-way-to-get-your-miniatures-table-ready), [Goonhammer, one-coat painting](https://www.goonhammer.com/hobby-101-one-coat-painting-speed-paints-contrast-and-more).
- Zenithal: [Composite Paint, zenithal priming explained](https://compositepaint.com/learn/zenithal-priming-explained/), [Age of Miniatures, zenithal highlights](https://ageofminiatures.com/zenithal-highlight-techniques-for-miniatures/).
- Varnish: [Goonhammer, how to use varnish](https://goonhammer.com/hobby-101-how-to-use-varnish-with-miniatures), [Age of Miniatures, matte, gloss and satin compared](https://ageofminiatures.com/best-varnish-for-miniatures/).
