# Workspaces, and one kit of controls

The design for the window's panels, menus and controls, and for moving
between sculpting, kitbashing, painting and getting a print ready. Written
from a review of the application as it stood on 2026-11-25: screenshots of
the running window under Xvfb and an inventory of `crates/hani-app/src`.

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
| U4 Workspaces | **Done**, 2026-11-25 | The switcher in the title bar, Ctrl and 1 to 4, Tab back; for each workspace a rail, a bar, a library and settings; what each remembers |
| U5 Each workspace its own | **Part done**, 2026-11-25 | A drawn falloff (issue 78) and a drawn pressure curve (issue 76); what the core does to paint under a sculpt stroke, written up (issue 82). Kitbash's placement (issue 77) is not done |

### U1: the kit

`crates/hani-app/src/ui.rs` holds one function a kind of control.

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
  which `Hani::update` unwraps before anything else, so the guards that
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
neither. So 7 and 8 picked Gather and Spread, and Move and Paint are
on the rail. (On 2026-11-26 Gather became Pinch, taking 5 where
upstream's Pinch stood, Spread became Widen at 7, and 8 is Scrape.) Making Move a brush of the palette is a change to the
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

### U4: the workspaces

`app/workspace.rs`. The switcher is in the middle of the title bar
(Sculpt, Kitbash, Paint, Print; Ctrl and 1 to 4; Tab goes back to the
one before). The right-hand panel is the dock now, 276 wide where it was
258: two tabs, the workspace's library (named for what is in it:
Brushes, Parts, Paints, Export) and its Settings, and the bodies list
at its foot in every workspace.

| Workspace | Rail | Library | Settings |
|---|---|---|---|
| Sculpt | Brush, Move, Mask, Transform, Cut, Measure | The palette's Sculpt and Lines and stamps groups, and the user's own | BRUSH, STROKE, PATTERN, FORM, WORKBENCH, DETAIL |
| Kitbash | Place, Transform, Cut, Measure | The palette's Add group; a copy of a body; maps taken in | ADD (how what is in hand meets the surface), DETAIL |
| Paint | Paint, Mask, Measure | The paint: modes, touches, colour, the machine's filaments | BRUSH (size and edge), STROKE, FILAMENT |
| Print | Measure, Cut, Transform | Export: detail, overhangs, formats, textures, parting keys | DETAIL, FILAMENT |

The bar follows the lit tool: a brush's size, weight and edge; for
Place, the soft join and how far the part is held off; for Measure, the
cut and the gizmo, the mirror planes alone.

A switch keeps the document as it was and each workspace as it was
left. On the way out, a workspace's lit tool, its tab, its open
sections and, for Kitbash, the part in hand are kept, and the part and
a measurement are put down so neither is armed under another
workspace's tools. On the way in, the workspace's tool is taken up
again: Brush and the palette's brush that was in hand, Place and the
part, Paint, or Measure, which is what Print opens with, so that a drag
in Print does not sculpt.

The keys 1 to 8 pick the open workspace's library: the Sculpt group's
first eight in Sculpt, the Add group's in Kitbash (numbered on their
tiles), the five touches in Paint, nothing in Print.

HISTORY and Reset sphere left the dock: undo and redo are the Edit menu
and their keys, and File, New is a fresh sphere. The palette's "At the
pointer" button went too, at the maintainer's word on 2026-11-25: it was
a second way to the `b` key.

Tests: `no_switch_changes_the_document` (a property: any run of
switches and Tabs keeps the camera, the mirror planes, the active body,
the body count, the undo history and the field at four points, and
lights only a tool the open workspace's rail has),
`a_workspace_is_found_as_it_was_left`,
`kitbash_holds_what_it_adds_and_only_there`,
`the_number_keys_follow_the_workspace`, `ctrl_and_a_digit_is_a_workspace`,
`tab_goes_back_and_a_card_holds_it`. `the_falloff_offered_follows_the_tool`
covers Measure and Place.

On a screen: each workspace's rail, bar and library; Settings; the
switcher. The bar in Print first showed the paint brush's numbers, left
from Paint, and in Kitbash the brush's; it follows the lit tool now.

Not done in U4:

- The user's own brushes are shown in Sculpt only, adding ones too.
- The `b` palette at the pointer shows the open workspace's groups; in
  Paint and Print it shows Sculpt's.
- Each workspace's scroll position is not kept.
- Tab cannot be seen working under Xvfb, which passes no keys; it is
  tested through the key message.

### U5: drawn curves, and paint under a sculpt stroke

- **The falloff, drawn** (`curve.rs`), in BRUSH's settings: a
  cross-section from rim to rim, the weight a stamp gives at each
  distance from its middle, worked out by `Profile::weight`, the
  function the brush itself uses. It follows the five shapes and the
  flat middle as they are changed. For the mask and for a colour it is
  upstream's curve. This is the second of issue 78's three asks; the
  first (a cross-section of a surface before and after a stamp, which is
  what tells Clay from Draw) and the third (a curve that can be edited)
  are not done.
- **The pressure curve, drawn**, in Preferences under the pen: pressure
  along, the strength a stroke is given up, by `tablet::shape`, the
  pen's own function. The first of issue 76's asks is a curve that can
  be drawn; this one can only be looked at, and it is still one curve
  for every brush.
- **A map's caption** in Kitbash's settings is the status bar's prompt
  now, like the shapes'.

Tests: `a_cross_section_is_the_same_either_side` (a property over the
five shapes and any flat middle: the drawing is symmetric, full in the
middle and nothing at the rims) and `a_pressure_curve_is_the_pens` (a
property: what is drawn at each pressure is what the pen gives).

**Paint under a sculpt stroke**, the question the maintainer left to be
found out (decision 5 above). Both kinds of paint are stored per voxel,
through the narrow band. A field brush, an insert and detail gain leave
the paint on its voxels; Move and Bulk carry it with the surface. So a
surface that moves less than about three voxels keeps its colour,
surface grown further out is bare, paint carved away stays buried and
shows again if the surface comes back to it, and undo restores paint
with the field. That is kept as it is, and issue 82 has the details,
the file and line of each, and what wants changing (new surface taking
the paint it grew from, buried paint not coming back).

Not done in U5, and so not done in this design:

- Kitbash's placement (issue 77): a height set during the drag, a
  preview, a lent mesh seated by its underside, the lift in millimetres.
  Seating by the underside needs a decision on which way is down for a
  body that is copied and turned.
- An editable falloff curve and a per-brush pressure curve (issues 78
  and 76), and the before-and-after cross-section of 78.
- A status bar warning when a stroke grows bare surface on a painted
  body (issue 82).
- The palette cannot be reordered; scroll positions are not kept per
  workspace; the `b` palette in Paint and Print shows Sculpt's groups.

## R1: four modes, each its own (2026-11-25)

The maintainer looked at U4 and changed the design: the modes are to be
separated more fully, with a palette each and shortcuts that change by
mode. Decided in conversation on 2026-11-25:

- **Sculpt, Transform, Paint, Print** (Ctrl and 1 to 4). Kitbash is
  Transform: the bodies, the hierarchy and the parts, and moving them.
- **P** brings up the open mode's palette at the pointer: Sculpt's
  brushes, Transform's parts, Paint's touches; Print has none. It was
  `b` for the one palette there was.
- **Shapes split by result**: a shape added in Sculpt is joined into the
  body (since 2026-11-28 placed first, and joined when done: see
  `app/placed.rs`) and is a brush of Sculpt's palette (its group is "Add,
  baked in");
  the same shape in Transform is a part, a body of its own. The mode
  says which, whichever way it was chosen, and the Baked in / Its own
  body toggle is gone.
- **The cut splits**: a cut in Sculpt cuts material away, and never
  parts; Print's cut parts the body in two with keys, Alt or not.
- **The bodies list always shows**, at the foot of every mode's dock,
  with no fold, so which part a stroke, a coat, a move or a cut goes to
  is chosen in any mode. Adding, deleting, duplicating, merging and
  splitting bodies are Transform's.
- **Move stays on Sculpt's rail. Paint has no Paint button**: painting
  is what Paint does when neither the mask nor a measurement is up.
- **Keys by mode**: M masks in Sculpt and Paint; C cuts in Sculpt and
  parts in Print; W is Transform's gizmo; Ctrl G groups in Transform. A
  key that means nothing in the open mode does nothing.

| Mode | Rail | Palette and library | Settings |
|---|---|---|---|
| Sculpt | Brush, Move, Mask, Cut, Measure | Sculpt, Lines and stamps, Add (baked in), the user's own | BRUSH, STROKE, PATTERN, FORM, WORKBENCH, DETAIL; with a shape in hand, ADD and DETAIL |
| Transform | Gizmo, Place, Measure | Parts: the Add group, a copy of a body, maps taken in | ADD, DETAIL |
| Paint | Mask, Measure | The paint: modes, touches, colour, filaments | BRUSH, STROKE, FILAMENT |
| Print | Cut, Measure | Export, parting keys, fit test | DETAIL, FILAMENT |

Transform opens on the gizmo, and Place takes up the part last in hand
(a ball the first time). A part taken up from the palette or by a key
puts the gizmo down so the drag sets it down; the gizmo puts the part
down and Place takes the same one up again. In Transform and Print a
drag never strokes the field: when their tool puts itself down (a cut is
one shot, the gizmo can be refused) the mode's first tool is taken up,
or measuring.

Tests: `transform_holds_its_part_and_only_there`,
`the_mode_says_baked_in_or_its_own_body`, `keys_and_the_cut_follow_the_mode`,
and the property `no_switch_changes_the_document` now holds the lit tool
to the mode's tools. The three parting tests run in Print. The palette
test presses P. A kept brush that adds, taken up in Sculpt, is baked in
whatever it was kept as, and its test says so.

Not done in R1: the paint rack (to be designed); dragging rows of the
bodies list is not limited to Transform, only its verb row is.

## R2: the keys 1 to 8 are the user's (2026-11-25)

`app/keys.rs`. Right click a tile of Sculpt's or Transform's palette and
a row comes up over it: "Put Clay strips on a key", the keys 1 to 8 (the
one it is on lit) and a way out. Press a key and the brush is on it, in
that mode only; the tile shows the number. A brush is on one key at
most: one already on a key and put on another swaps with the brush that
was there; one on no key takes the key from the one there, which stays
on the palette. A key nobody has set picks the palette's own place, so
nothing changes until someone sets one.

A key names its brush rather than its place, so a brush added before it
on the palette does not move a key. The keys are kept in `keys.conf` in
the settings directory, a line a key: `sculpt.2 = Clay strips`. A line
that cannot be read is left out and its key is the palette's.

Tests: `unset_keys_are_the_palettes_places`; the property
`a_brush_is_on_one_key_and_the_keys_keep` (any run of puts leaves each
brush on one key at most and the last put on each key, and the file
reads back as written); `a_bad_line_is_left_out`; and through the
application `a_brush_put_on_a_key_is_picked_by_it` (the key picks it,
two keyed brushes swap, Transform's keys are its own).

On a screen: the row over the palette on a right click, a brush put on
2, its badge and the status line, and `keys.conf` as written.

Not done: Paint's keys are its five touches and cannot be set; a brush
of the user's renamed or forgotten leaves its key to the palette's
place.

## R3: Explode and show only the active body (2026-11-26)

Asked for by the maintainer: *"the three d hierarchy needs a "explode"
button which seperates all the parts temporarily to make it easier to pick
on ( unexplode on pick ) and a quick button to hide / show all parts but
the current active one."*

`app/explode.rs`, and `hani_gpu::spread`. Two buttons over the bodies
list, in every mode, each lit while on.

**Explode** draws every drawn body apart. The largest, by the volume of
its box, stays where it is; every other is moved along the line from the
largest's middle through its own, to twice its distance or to where the
two boxes' half-widths and a gap clear each other, whichever is further.
A part centred on the largest has no line of its own and is sent across
the view as it was when the button was pressed, each such part a
different way round, so none hides behind the largest. Nothing moves but
the picture: each body's offset is a per-instance vertex attribute the
sculpt shader adds to where it draws the vertex, read from a small buffer
by the instance number the body's draws are issued with (entry 0 is zero,
and is what a body not apart and every thumbnail use). The frustum test
shifts the brick's box by the same offset. The pick marches each body
with the ray moved back by its offset, so what is pressed is what is
drawn. A left press on a part chooses it and brings everything back; a
press on nothing does nothing; escape and the button bring them back too.
While apart there is no brush ring and no gizmo (both would be drawn
where the body is, not where it is drawn); a move in hand when the button
is pressed is kept, as choosing another tool keeps it, and the gizmo
comes back on whatever is picked.

**Show only the active body** is solo on the active body, from the
header rather than the row: pressed again, or escape, everything is
shown as it was.

Tests: `a_spread_body_is_drawn_moved_by_its_offset` (offscreen: another
body's offset moves nothing, this body's moves its picture across the
screen, and none puts it back exactly);
`explode_draws_the_parts_apart_and_moves_no_field` (the largest stays,
the other is clear of it along the line between them, no undo entry);
`a_press_on_a_part_apart_selects_it_and_brings_them_back` (and carves
nothing); `a_part_inside_the_anchor_is_sent_across_the_view_and_clear_of_it`;
`the_gizmo_goes_while_apart_and_comes_back_on_the_part_picked`;
`one_body_does_not_explode`; `solo_active_shows_the_active_body_alone`.

On a screen: a ball, a cube and a cylinder side by side in Transform,
opened out along the row; the gizmo gone while apart; a click on the cube
where it was drawn chose it, brought the three back and put the gizmo on
the cube. Show only the active body drew the cube alone, SOLO in the
header, and again drew all three.

Each body's box is measured from its bricks when Explode is pressed, not
read from the document's cache: the cache only grows, and a piece just
split off a ball kept the whole ball's box, was taken for the largest part,
and pushed the ball away (seen on the screen;
`explode_after_a_split_moves_the_piece_and_not_the_body`).

Apart, the view is framed to hold every part, facing the way it faced and
only ever moving out, and together again it is put back as it was
(`apart_every_part_is_in_view_and_the_view_comes_back`): a model the view
held whole had opened past its edges.

Not done: a part's offset is worked out from its box, so two parts on
the same side of the largest at the same distance can still overlap each
other; folders are not exploded as a group.

### Travelling, and the others see-through (2026-11-26)

Asked for by the maintainer: *"Animate the explode, when clicking on a part
in explode view that unexplodes the view and selects that part. Add a
button next to hide all other parts to make them translucent and
unselectable instead. And when its on, selecting a part in the hierarchy
then makes that part active and all others translucent."* A click on a
part already chose it and brought the parts back; what was missing was
the travel.

**The parts travel** apart and back over 320 ms, eased, and the view with
them between where it was and the view framing them apart (both chosen
on a screen, not measured). The layout is worked out whole and drawn
scaled by how far the travel has got, so the pick, which marches each
body moved back by its drawn offset, still hits what is drawn. On the
way back nothing is where it ends up, so a press then is taken and does
nothing; the gizmo comes back once they are together. Pressed again on
the way back, they turn round where they are, the same way.

**The others see-through** is a third button. Every body but the active
one is screened by the ring cut's 4x4 dither, 70 per cent of its pixels
discarded (chosen on a screen), so what is behind shows with no sorting
and the depth buffer still right; the share rides in the fourth float of
the body's entry in the Explode buffer, which was padding. The pick
leaves those bodies out (`Hani::pick_mask`), so they cannot be chosen,
hovered or carved through the viewport, and a ray passes through them to
what is behind. Choosing another row makes that one solid and the rest
see-through, since the rule is "all but the active one". Solo hides what
this shows through, so turning either on turns the other off; escape
leaves it.

Tests: `the_parts_travel_apart_and_a_press_on_the_way_back_does_nothing`
(the bead part of the way out, further, then all the way, and a press on it
on the way back selects nothing);
`the_others_see_through_cannot_be_picked_and_follow_the_active_body`
(screened and not picked, following the row, one or the other with
solo, escape); offscreen in `hani-gpu`, a body three quarters screened
draws 0.25 of its pixels within 0.05 (llvmpipe). The tests above settle
the travel before they look.

On a screen, under Xvfb: a ball added on the starting body in Transform;
see-through on drew the ball as glass over the solid body; choosing the
ball's row made it solid and the body glass; Explode moved the ball out
and pulled the view back, and pressed again brought it in over a run of
frames. Llvmpipe draws a frame in about a fifth of a second, so the
travel took longer there than it says.
