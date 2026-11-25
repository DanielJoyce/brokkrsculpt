# Workspaces, and one kit of controls

The design for the window's panels, menus and controls, and for moving
between sculpting, kitbashing, painting and getting a print ready. Written
from a review of the application as it stood on 2026-11-25: screenshots of
the running window under Xvfb and an inventory of `crates/brokkr-app/src`.

## What was found

- One right-hand panel, 258 px wide, held every setting in eighteen blocks
  in one order, whatever was being done. While painting, the sculpt
  palette, FORM and PATTERN were still there above and below the paint.
- Five ways of laying out a slider: a label above at 11 px, a label above
  at 10 px, a label with a typed field (the right-click menu), label,
  slider and value on one line (the colour), and a slider with an Apply
  button beside it.
- Two kinds of heading that looked alike: seven folded, nine did not.
  "What it does" folded with a text triangle.
- Three places to pick a brush: the strip of eight, the palette and the
  `b` popup. Falloff was five shapes and a flat middle in the panel and
  four shapes in the right-click menu.
- No tooltips. Every icon-only button (the body verbs, the eye, solo, the
  pattern row of the right-click menu, the window controls) was unnamed.
- Buttons in three text sizes, a dozen literal paddings beside the
  spacing scale, and a disabled button drawn exactly as an enabled one.
- Inter named as the font and not shipped, so the face was whatever the
  machine had.

## The design

Every workspace has the same six regions, and only what fills them
changes:

1. **Title bar**: menus (File, Edit, View, Help), the document's name,
   the workspace switcher, undo and redo, the window controls.
2. **Context bar** over the viewport: what is changed every few strokes
   for the tool in hand (radius, strength, falloff, pressure, mirror).
   The right-click popup is the same controls in a floating card, built
   by the same function.
3. **Tool rail** on the left: the kinds of tool in this workspace, as
   icons named on hover. Brushes are not on it; they are in the library.
4. **Viewport**: the navigation cube, saved views, the scale bar, frame
   stats and the bug button.
5. **Dock** on the right: the workspace's library and its inspector (the
   full settings of what is chosen), with the bodies list at its foot in
   every workspace.
6. **Status bar**: the result of an action, a prompt such as Measure's,
   a warning, and the voxel size, triangles and bodies.

| Workspace | Rail | Library | Inspector |
|---|---|---|---|
| Sculpt | Brush, Mask, Transform, Cut, Measure | The palette's sculpt brushes and lines and stamps, and the user's own | Brush, Falloff, Stroke, Tip, What it does, Pattern, Whole body |
| Kitbash | Place, Transform, Cut, Measure | Shapes, displacement maps, copies of bodies, meshes | Placement, Join, Cable, Transform |
| Paint | Brush, Pick colour, Mask, Fill, Measure | Touches, colours, the machine's filaments | Colour, the touch's settings, Falloff, Stroke, Pressure |
| Print | Inspect, Cut, Parting keys, Measure | Export formats, texture size, open in the slicer | Size, Overhangs, Parting keys and a fit test, Filament slots |

A workspace changes what the panels offer and what a drag on the model
does. It does not change the document: the camera, the bodies, which is
chosen, the mask, the mirror planes and the one undo history carry over
every switch. Each workspace keeps its own last tool, open sections and
scroll position, so Sculpt, then Paint, then Sculpt again lands where it
was left. Ctrl and 1 to 4 pick a workspace, Tab goes back to the last.

## Decided by the maintainer, 2026-11-25

- **Four workspaces**: Sculpt, Kitbash, Paint and Print.
- **Print is a workspace**, not an export dialog.
- **Keys 1 to 8 pick the first eight of the open workspace's library**,
  numbered on its tiles. In Sculpt the order starts with today's eight.
- **No key hints in the status bar.** What a key or a held modifier does
  is in tooltips and in Help, Keyboard shortcuts (issue 69).
- **Paint under a sculpt stroke**: find out what the core does today,
  keep that, and write it up with a ticket if it wants changing.

## Stages

One pull request each, into `feature/studio`.

| Stage | State | What it is |
|---|---|---|
| U1 The kit | **Done**, 2026-11-25 | `ui.rs`: a folding section, a number, a choice of a few, a button, a tooltip. Every panel block under a heading that folds; every slider a number; every icon-only button named on hover; Inter shipped |
| U2 Menus, status bar, Preferences | **Done**, 2026-11-25 | Edit and View menus, a status bar at the foot, a Preferences card (the pen, the SpaceMouse, updates), a Keyboard shortcuts card on `?`, and every grey caption out of the panel |
| U3 Context bar and one popup | **Done**, 2026-11-25 | The bar over the viewport and the right-click card from the same functions; a rail of tool kinds; keys 1 to 8 pick the palette's first eight, numbered on their tiles |
| U4 Workspaces | Not begun | The switcher, a library and inspector for each, what each remembers |
| U5 Each workspace its own | Not begun | Paint's filaments, Kitbash's placement (issue 77), Print, a drawn falloff (issue 78) and pressure curve (issue 76) |

### U1: the kit

`crates/brokkr-app/src/ui.rs` holds one function a kind of control.

- **`section` and `heading`**: a caret and a title in capitals, which
  folds. Every block of the panel is one now. BRUSHES, WORKBENCH, ADD,
  BRUSH, STROKE, FORM and HISTORY did not fold before; FORM starts shut,
  since it works on the whole body and is reached for now and then.
  `disclosure` is the smaller fold inside a block ("What it does").
- **`number`**: a label and a field that takes typing on one line, the
  slider under them. What is typed is a draft, held by a key the caller
  names, until enter; enter sends what the slider would have sent for
  that number, held to the slider's range, and a draft that is not a
  number is let go. Both the slider and enter send `Message::FieldSet`,
  which `Brokkr::update` unwraps before anything else, so the guards that
  look at a message (the rename, the gizmo, the workbench, the welcome
  card) meet the message it carries and never the wrapper. Every slider
  in the panel is one of these; the right-click menu's radius and
  strength fields are its own until U3 rebuilds that menu.
- **`segmented`**: two to five choices in a row, each given a share of
  the row by the length of its word. In equal shares "Smooth" and
  "Cylinder" ran past their buttons' edges, seen on the screen.
- **`button_secondary`, `toggle`**: one text size for
  every control's words (`theme::LABEL_SIZE`, 11 px) where there were
  three, and padding from named constants (`theme::PAD_*`).
- **`tip`**: a name on hover. Every button that is only
  an icon has one, and a test reads every source file and fails on a
  `button(icon::icon(` not inside a `crate::ui::tip(`.

Theme: a disabled button is drawn dim (it was drawn exactly as an enabled
one, so Redo with nothing to redo looked live). Inter Regular and SemiBold 4.0 are
in `assets/fonts` with their licence (SIL Open Font License 1.1) and
bundled by `main.rs`.

Tests: `a_number_is_read_past_its_unit_and_its_label`,
`what_a_field_shows_reads_back` (a property: whatever a field shows,
with two places and a unit, reads back as that value),
`a_typed_number_is_what_the_slider_would_send`,
`a_draft_lasts_until_its_number_is_set` (through the application), and
`every_button_that_is_only_an_icon_is_named_on_hover`.

On a screen, under Xvfb at 1280 by 900: the panel scrolled top to bottom
with the Draw brush and with the Paint brush, and the right-click menu
with Paint. That is where the clipped words were found.

Not done in U1:

- The grey captions that explain keys ("ctrl or alt drag removes", the
  cheat sheet at the foot of the panel, the export path) are still
  there. They go in U2, with Help, Keyboard shortcuts to take them.
- The right-click menu's radius and strength still use their own typed
  fields; its falloff row is the kit's.
- Typing in a field cannot be seen working under Xvfb, which passes no
  keys; the draft and the submit are tested through messages.
- An icon in a disabled button keeps its colour: an icon is drawn at a
  colour fixed when it is built, and cannot follow the button's state.
- Nothing moved between blocks, and nothing was taken out.

### U2: menus, a status bar, Preferences

- **Edit**: Undo and Redo with their keys (greyed when there is nothing to
  do), Invert the mask, Clear the mask, Preferences.
- **View**: Paint, Creases, Overhangs, Mask, Pictures in the bodies list,
  Frame stats, as ticks; Frame the body. Show paint and show creases left
  FILAMENT, and the Thumbnails tick left BODIES, for this menu. The
  overhangs tick stays in EXPORT as well, where its angle is.
- **Help**: Keyboard shortcuts first.
- **The status bar**, at the foot of the window (`app/cards.rs`): what the
  last action did, in red when it failed, and three readings: the voxel
  size, the triangles drawn, the bodies. The status line left the title
  bar and the export block, where it was shown twice at once. While a
  gesture waits for its clicks the bar says what to do instead: Measure's
  two points, the insert in hand, the stamp in hand. Those were captions
  beside the Measure button and under ADD and STROKE.
- **Preferences** (Edit menu): the pen, the SpaceMouse, the update check
  and the welcome screen on startup. PEN and SPACEMOUSE left the panel.
  No key: Ctrl and comma already hides the active body.
- **Keyboard shortcuts** (Help menu, or `?`): every key and held modifier
  on one card, in two columns. The cheat sheet at the foot of the panel
  went, and so did every caption that said what a key or a modifier does.
- **Captions out**, the rest of them: what ctrl does, S and U, the
  pattern's, the export path, the workbench's two, FORM's, the stamp's,
  the plane's sides, a brush of nothing, the wash's, the fill's, the
  overhang's, Parting keys' key, the timeline's, the SpaceMouse
  settings' file path. Where one said something a user needs, it is the
  control's tooltip: Radius and Strength name S and U, Picture across the
  tip says what bright means, FORM's Apply says it works on the whole
  body less the mask, Parting keys says Cut with Alt, the timeline says
  click, drag and right click.

Tests: `the_cards_of_the_menus_open_hold_the_keys_and_close` (each card
takes its menu away, holds the keys, and goes on Escape and on Close),
`question_mark_is_the_shortcuts_card`, `every_letter_on_the_card_is_a_key`
(every letter the card names decodes to a key, chords with Ctrl held),
`the_devices_are_not_in_the_panel`.

On a screen: the four menus, each under its word; the shortcuts card,
which a click outside does not close; the Preferences card; the status
bar; the panel top to bottom with no caption left in it.

Not done in U2:

- The settings system of issue 74 (limits as settings) is not built.
  Preferences is where it will go.
- Help has no page for what the shortcuts card cannot say (issue 69).
- A menu is placed under its word by a measured step, not by where iced
  lays the word out.
- The scale bar's "at the middle of the view" stays: it says what the
  reading is of, not how to use it.

### U3: the context bar, one popup, the rail

- **The context bar** (`app/context.rs`), over the viewport: the tool's
  name, its radius, its strength (or a colour's opacity), its falloff (a
  profile's five shapes for a brush on the field, upstream's four curves
  for the mask and for a colour, nothing for a filament slot), and the
  mirror planes. The cut and the gizmo show their name and the mirror
  planes only. A number on the bar is the kit's number laid on one line:
  label, a short slider, the field. The row wraps when the window is
  narrower than it.
- **The right-click menu** is built from the same functions: the same
  radius and strength numbers (the same drafts: typing in one shows in
  the other), the same falloff choice. Its own typed fields, which moved
  the value on every keystroke that parsed, are gone; like every number
  now, the value moves on enter. Before this the menu offered four
  falloffs where the panel offered five shapes and a flat middle.
- **The rail** (`app/rail.rs`) replaced the strip of brushes: Brush (the
  palette's brush in hand; its icon is that brush's kind), Move and Paint
  (upstream's two that are not brushes of the palette), Mask, Transform,
  Cut. Each keeps a word under its icon, and for the three modes it is a
  state, as on the strip: "armed", "blur", "free". Brush, after Move or
  Paint, brings back the palette's brush that was in hand.
- **Keys 1 to 8** pick the palette's first eight, which carry their
  number in a corner of their picture. The palette's Sculpt group starts
  with upstream's six in upstream's order, so 1 to 6 pick what they
  always picked.

**Where this is not what was decided.** The decision was that the keys
pick the library's first eight and that in Sculpt the order starts with
today's eight. Today's seventh and eighth were Move and Paint, and
neither is a brush of the palette: the palette holds brushes on the
field, with a picture of a stroke and a profile, and Move and Paint are
neither. So 7 and 8 pick Gather and Spread now, and Move and Paint are
on the rail. Making Move a brush of the palette is a change to the
brush model and was not made.

Tests: `each_rail_button_lights_itself_and_brush_comes_back` (every pair
of rail buttons, and a mode pressed twice lights the brush under it);
`the_falloff_offered_follows_the_tool`;
`the_digits_select_the_first_eight_brushes_of_the_palette`;
`the_first_six_keys_are_upstreams_six`;
`a_typed_radius_arrives_on_enter_and_still_clamps`, which replaces the
menu's own field test. A test of the studio typed a strength through the
menu's field and now sends what the field sends.

On a screen: the bar with Draw, with the mask and with Paint; the rail;
the numbered tiles; the right-click menu with Paint. The bar wrapped the
mirror planes onto a second line at 1280 wide, and was tightened until
it did not.

Not done in U3:

- The palette cannot be reordered, so the keys cannot be re-pointed.
- The WORKBENCH is not on the bar (the design had it there).
- The panel's BRUSH block still has the radius and strength as well as
  the bar; the inspector of U4 decides what stays.
- While shift is held the rail's Brush button says "smooth", but the
  palette does not light Smooth as the strip did.
