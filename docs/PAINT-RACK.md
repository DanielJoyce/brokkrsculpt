# The paint rack, and painting a figure

A design, asked for by the maintainer on 2026-11-25 and not yet built. It
covers:

- a searchable index of real hobby paints, from the
  [miniature-paints](https://github.com/Arcturus5404/miniature-paints)
  tables;
- racks: sets of paints a user chooses, swaps between and adds colours
  of their own to;
- the tools for laying paint: brush sizes, an airbrush, a sponge that
  works the way a sponge does, and stamps;
- three one-click coats: a base coat, a zenithal, and a sponge chop;
- varnish: gloss, satin and matte;
- layers, later.

Each stage at the end is one pull request. Nothing here is decided until
the maintainer has read it; the open questions are listed last.

## What there is today

The Paint mode already has a colour field and a set of touches. This
design builds on them rather than beside them.

- **A colour per voxel.** `brokkr-core/src/paint/field.rs`: four bytes a
  voxel through the narrow band, sRGB red, green and blue and a coverage,
  beside the distance field. It is one layer.
- **Slot recipes.** `brokkr-core/src/paint.rs`: up to 63 paints, each a
  base colour, a wash (pigment, stain, flow into creases, run off edges),
  a highlight, a zenithal amount and a `Finish` (Matte, Satin, Gloss,
  Metallic; saved but "not drawn yet"). These paint the one-byte slot
  field, which is what a filament printer reads.
- **Touches.** `brokkr-core/src/paint/touch.rs`: Loaded, Drybrush (catch,
  streak, bristle, runs out), Sponge (density, speck size, more on edges),
  Wash and Fill. Modes Paint, Glaze, Erase. Coat the body, Pick from
  model, an opacity, a soft edge.
- **The surface's shape is known.** The cavity (how concave the surface
  is at a point) is computed for washes and for the creases view, and the
  normal for the overhang view. A one-click coat reads both.

## 1. The paint index

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
  for a sponge chop or a zenithal.
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

## 2. Racks

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

## 3. How each kind of paint lays

The kind is not only a label: it decides how a paint goes on. Each is a
setting of the touches and wash that exist, chosen when the paint is
picked, which the user can still change.

| Kind | Coverage | Into recesses | Off edges | Finish |
|---|---|---|---|---|
| Paint, Air | Opaque, builds in one or two coats | No | No | Matte |
| Metallic | Opaque | No | No | Metallic |
| Wash | Thin over flats | Pools in creases, by the cavity | Runs off | Satin (most washes dry with a sheen) |
| Stain | Even tint over everything, what is under it shows | Slightly | No | Gloss to satin |
| Contrast / speed | Tints by what is under it: bright over a light undercoat, nearly gone over dark | Pools strongly | Thins on edges, so the undercoat shows as a highlight | Satin |
| Primer | Opaque, one coat | No | No | Matte |
| Effect | As paint, flagged | | | As made |

The contrast rule is what makes a sponge chop or a zenithal under it
worth doing: its colour multiplies the value of what is under it.

## 4. Tools

The Paint mode's rail becomes: **Brush, Airbrush, Sponge, Stamp, Fill,
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

### Sponge, as a sponge works

Today's sponge lays specks over the brush's disc. A sponge on a figure
does more than that, and the technique this design follows is the
sponge version of the slapchop:

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

### Stamps

A picture laid onto the surface in the paint in hand: eyes, numbers,
hazard stripes, freehand patterns, runes, and any picture the user
takes in. Placed the way Sculpt's stamps are (press where it goes, drag
to size and turn it), projected along the view. The picture is a
stencil: where it is dark, paint; where it is light, none; grey, part.
It goes through the mask.

## 5. One-click coats

Each is a small card (its few settings and a live preview on the model)
with Apply, which is one undo entry. Each works on the active body, or on
the masked part of it, or on every visible body.

### Base coat

The body, all of it, in one paint: what Coat the body does now, given a
paint from the rack and the paint's kind (a primer is a primer).

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
is laid as paint, so it can be painted over, and a contrast over it
reads its value.

### Sponge chop

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

## 6. Varnish

A finish over paint: **gloss, satin or matte**, brushed on like paint or
coated over the whole body in one click.

- **A finish per voxel**, two bits beside the colour: matte, satin,
  gloss, and metallic (which comes from a metallic paint and is not a
  varnish). Varnish changes the finish; it does not change the colour.
- **Drawn.** The viewport's shading takes a roughness from the finish (a
  wide, dim highlight for matte, a tight bright one for gloss) and a
  metalness for metallic. `Finish` is already saved on a slot recipe and
  "not drawn yet"; this is where it is drawn.
- **Exported.** The GLB and OBJ exports carry a roughness and metalness
  map beside the colour (the bake of stage S5 has the colour; this adds
  the rest), so a render elsewhere shows the varnish.
- Typical use, which the defaults follow: matte over everything, gloss on
  eyes, gems, wet things and blood.

## 7. Layers (later)

Paint in layers the user orders, hides, and sets the opacity of:

- each layer a colour field like today's (four bytes a voxel through the
  band), on one body;
- a blend per layer: normal, multiply (for a shade), screen (for a glow);
- a layer per one-click coat by default ("Zenithal", "Sponge chop",
  "Contrast"), so one can be redone without the rest;
- merged down, or flattened for export and for printing.

The cost is a colour field a layer; a figure with five layers is five
times today's paint memory, which is why it waits and why layers can be
merged.

## 8. Where it goes in the window

- **Paint's library tab** is the rack: the rack's name and a list to swap
  racks, its swatches (with the keys 1 to 8 on them), Add paints (which
  opens the index search), and New colour.
- **P** brings the rack in hand up at the pointer.
- **Paint's settings tab**: the tool in hand's settings (size preset,
  flow, sponge pattern, airbrush spray), and the paint in hand (kind,
  finish).
- **The one-click coats** are buttons in a COATS section of the settings
  tab, and in a Paint menu: Base coat, Zenithal, Sponge chop, Varnish.
- **The rail**: Brush, Airbrush, Sponge, Stamp, Fill, Mask, Measure.

## 9. Stages

1. **Index and racks.** The generated index (with the classifier and the
   override table), search by name and colour, racks (make, swap, add,
   reorder, files), custom paints, keys 1 to 8 on the rack, the kind
   setting how a paint lays.
2. **Brush sizes and the airbrush.**
3. **The sponge reworked, and the one-click coats**: base coat, zenithal,
   sponge chop.
4. **Stamps.**
5. **Varnish**: the finish field, drawn and exported.
6. **Layers.**

How each is tested, as the rest of the studio is: properties where a
rule holds for any input (a zenithal never makes a surface facing the
light darker than one facing away; a sponge chop never puts more of the
light in a recess than on the peak next to it; a search for a paint's
exact name finds it first; the nearest paint to a paint's own colour is
itself), the application driven by its messages, and a look on the
screen.

## Open questions

1. **Pantone and RAL**: in the index as colour references (searchable,
   not paints), or left out?
2. **Colour distance for search**: OKLab (cheap, good) or CIEDE2000 (the
   reference, heavier)?
3. **Zenithal shadowing**: a ray toward the light from every surface
   point costs time on a big model; on by default, or a box to tick?
4. **"Sponge chop"**: no source found uses the term. This design reads it
   as the slapchop done with a sponge (dark, then middle and light
   sponged onto the raised and upward parts, then a contrast). Is that
   what was meant?
5. **Contrast paints**: is "multiplies the value under it, pools in
   recesses, thins on edges" close enough, or should a contrast's
   behaviour be measured from swatches of real ones?
6. **Racks and filaments**: should a rack also say which filament each
   paint prints in, for a printer that prints colour, as the slot
   recipes do?
7. **Stamps library**: which pictures come with the application?

## Sources

- The paint tables: [Arcturus5404/miniature-paints](https://github.com/Arcturus5404/miniature-paints), MIT.
- Sponging on miniatures: [Goonhammer, Hobby 101: Sponge Painting](https://goonhammer.com/hobby-101-sponge-painting), [FineScale, chip paint with a sponge and microcell foam](https://finescale.com/how-to/articles/2022/05/chip-paint-with-a-sponge-and-microcell-foam/), [Age of Miniatures, stippling](https://ageofminiatures.com/stippling-technique-for-miniatures/).
- Slapchop: [Loot Studios](https://blog.lootstudios.com/paint/slapchop-and-how-to-paint-an-army-of-minis-really-fast/), [Empire of Minis](https://empireofminis.com/blogs/miniature-painting-and-basing/slapchop-painting-technique-the-fastest-way-to-get-your-miniatures-table-ready), [Goonhammer, one-coat painting](https://www.goonhammer.com/hobby-101-one-coat-painting-speed-paints-contrast-and-more).
- Zenithal: [Composite Paint, zenithal priming explained](https://compositepaint.com/learn/zenithal-priming-explained/), [Age of Miniatures, zenithal highlights](https://ageofminiatures.com/zenithal-highlight-techniques-for-miniatures/).
- Varnish: [Goonhammer, how to use varnish](https://goonhammer.com/hobby-101-how-to-use-varnish-with-miniatures), [Age of Miniatures, matte, gloss and satin compared](https://ageofminiatures.com/best-varnish-for-miniatures/).
