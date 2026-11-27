<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# Getting around

## The four workspaces

The switch in the middle of the title bar chooses what you are doing. Each
workspace keeps its own tools, panels and keys, and remembers how you left it.
**Ctrl** with **1** to **4** switches between them, and **Tab** goes back to the
last one.

- **Sculpt** is for shaping: the brushes, masking, cutting, the form tools
  Bulk, Detail and Shell, and Extract.
- **Transform** is for parts: adding shapes as bodies of their own, the move,
  turn and scale gizmo, Deform and Revolve. A part follows the gizmo while you
  drag it, and is rebuilt where you let go.
- **Paint** is for colour: the rack of paints, the ways of putting paint on,
  the one-press coats, and layers. Sculpting over paint afterwards keeps it on
  the surface the brush makes.
- **Print** is for getting it out: export formats, detail, overhangs, parting
  keys and the fit test.

Rest the pointer on a button to see what it does.

## The window

- **The tool rail**, down the left, picks the kind of tool: a brush, Move,
  Mask, Cut, Measure, depending on the workspace.
- **The context bar**, along the top of the viewport, holds the settings you
  change most for the tool in hand: radius, strength, falloff, mirroring.
- **The panel**, on the right, has the palette or library of the workspace,
  its settings in blocks that fold, and the list of bodies at the foot.
- **The status bar**, along the bottom, says what the last action did.
- **Help, About HaniSculpt** says which version and build this is, where it
  runs, which graphics adapter it draws with and what it was built with:
  what to give in a bug report.

## Moving round the model

- **Middle or right drag** turns the view, the same with **Shift** held pans
  it, and the **wheel** zooms about the surface under the pointer.
- **On a touch screen**, one finger on the model is the tool and one finger
  off it turns the view; two fingers slide the view as they move and zoom it
  as they spread or pinch. A tap is a click.
- The **cube** in the corner shows which way you are looking. Click a face to
  look straight at it.
- **F** frames the active body.

## Sculpting

- **Left drag** on the model makes a stroke with the brush in hand.
- **1** to **8** pick the first eight brushes of the palette, and **P** brings
  the whole palette up under the pointer.
- **Shift** held smooths. **Ctrl** or **Alt** held carves instead of adding.
- **[** and **]** change the brush size, and dragging with **S** or **U** held
  changes the size or the strength.
- **X**, **Y** and **Z** mirror strokes across a plane.
- **Radial copies**, under Symmetry in the Settings tab, lays a stroke
  again at each even turn round the axis chosen beside it, X, Y or Z,
  through the middle of the room, or wherever the body has since been
  moved. Two to sixteen copies, with the mirror
  planes as well if they are on.
- The planes and the axis belong to the body: moved or turned with the
  Transform tool, they go with it, while the handle is still held. A new
  shape's planes go through its own middle, and a copy, a shell or an
  extract keeps those of the body it came from. **Recentre**, under
  Symmetry, puts the body's back through the middle of the room, unturned.
- Radial copies round an axis that misses the body are refused, as a
  mirror plane that misses it is.
- **M** picks the mask: masked parts are protected from brushes, and a softly
  painted mask protects softly.
- **Right click** brings up the tool's own settings where the pointer is.
- A brush changed from the one on the palette says so beside its name.
  *Save* keeps the change in it, *Save as new…* keeps it under a new name,
  and *Revert* puts the brush back as it was kept.
- A brush can lay a picture across its tip, or press one as a stamp. *Add a
  picture…* takes in a PNG or JPEG of your own: bright is high. It is kept
  and listed with the others from then on, and a brush you keep with it in
  hand takes it up again when you choose the brush.

## Bodies

The list at the foot of the panel holds every body in the document. Click a
row to make it the active body, which is the one tools work on. The eye hides
and shows a row, and rows can be dragged into folders. Above the list,
*Explode* spreads the parts apart so a small one can be picked, and the button
beside it shows only the active body.

### Boolean groups

A folder can **combine** its rows into one solid: press the **∪** chip on
its row. Read the rows top to bottom. The first is the base; each row
after it is **added** (**+**), **cut** (**−**) or **kept** where it
meets what is above (**∩**), by the chip on its row, which a press turns
through the three. The order matters: a row moved up or down changes the
result at once. A folder inside a combining folder is a bracket, standing
for its own result. Hover the **∪** chip to see the whole fold written out,
as `((Body 1 − Hole) + Boss)`.

With the Transform tool up, press a folder's row to move, turn or scale
everything in it at once with the gizmo; press a body's row to go back to
moving that body alone.

In Transform, **+**, **−** and **∩** under the list put the active body and
the body below it into a new combining folder. While you work on a row of
one, the rows that cut or keep are drawn see-through so you can see and
sculpt them. A hidden row is left out. **Bake** collapses the folder into
one body that holds the result; Undo puts it all back. See
`docs/BOOLEANS.md`.

## Reference pictures and the floor

REFERENCES, in Sculpt and Transform, holds a picture for each of the front,
side and top planes. Choose the plane, then *Load picture…* (PNG or JPEG).
It arrives as tall as the model, centred on it and standing behind it as
that view sees it. Opacity, width, across, up and depth move it; *Flip*
mirrors it; *Shown* hides it without losing it. The model is always drawn
over the pictures, wherever they stand. *Floor* draws a grid under the
lowest point of the model, a line every round number of millimetres.

## Inserts: which way up, and how deep

With a shape or a body in hand to add, ADD shows it on the surface it will
go into: up the picture is out of the surface, and the orange plane is the
surface. Drag the picture to turn it, right drag to look round it. *+X* to
*-Z* stand that axis of it straight out of the surface, and *Reset* puts it
back. *Held off* moves it up out of the surface or down into it; what is
under the plane will be inside the model.

In Sculpt, a shape you drag out is **placed**: it sits beside the body in a
folder that combines them (see Boolean groups above), added, or cut where
Ctrl was held, and you can still move, turn or scale it with the Transform
tool. It is joined into the body when you set down the next one or choose
another brush, or with Bake. Undo after that puts it back to placed.

## Interface size

Edit, *Preferences…*, *Interface size*. *Auto* makes everything larger on a
screen with many lines, by the monitor the window is on. Or untick it, drag
*Size*, look at the *Preview*, and press *Apply*. *Reset to auto* goes back.

## Undo, saving and files

- **Ctrl+Z** undoes a whole stroke or action at a time, and **Ctrl+Shift+Z**
  redoes it.
- A sculpt is saved as one `.hani` file holding everything: the bodies, the
  mask, the paint and its layers, and the reference pictures. Older `.brokkr`
  files open too.
- Your work is saved aside now and then while you sculpt, so a crash costs
  little.
- In a browser, **Save** keeps the sculpt in the browser, and *Save to
  computer* downloads it. The browser asks before you leave with work not
  saved.

**?** on the keyboard shows every key on one card.

**Ctrl ?** snips a part of the window to report: drag across it, draw on it,
say what should be different, and open an issue on GitHub with the picture on
the clipboard to paste in.
