<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# Lighting and materials

How the model is lit on screen, and what it looks made of where it is not
painted. None of this changes the model or its paint: it is how you look at
it, the same for every body, in every workspace.

**View > Lighting** opens the *LIGHTING* section, at the top of the dock's
second tab; the same menu entry shuts it. Its heading folds it like any
other. Everything in it is kept between sessions.

## The presets

Each is one press, and replaces the lights there are. Change any number
afterwards and the lights are your own: no preset is lit until you choose
one again.

- **Studio** is the light the viewport has always had: a key from the upper
  left and a soft fill from the lower right, both with the camera, so the
  model looks the same however you turn it. The one to sculpt by.
- **Lightbox** is a light tent: a key high on the front left, a soft fill
  low on the right and a rim from behind, all fixed round the model. Turn
  the view and the highlights slide over the surface, which is what shows a
  gloss from a matte, and where a surface is smooth or lumpy.
- **Overhead** is one strong light from nearly straight above and a cool
  sky over a dark ground. The tops of forms are bright and the undersides
  in shade: the light a miniature is usually painted for, and the one to
  judge a zenithal or edge highlights by.
- **Sky** is no light at all, only the sky above and the ground below.
  Soft and even, with no highlights from a lamp; a gloss shows the horizon
  in it.

## A light

Up to four. The numbered buttons pick which light's settings are shown;
*Add* puts in another, *Remove* takes the shown one out.

- **On** turns it off without losing where it was.
- **With the camera**: ticked, the light is fixed to your view, as if
  mounted on the camera, and turning round the model never moves its
  highlight on the screen. Unticked, it is fixed in place round the model,
  like a lamp on the desk: turn the view and you see the model from the
  lit side or the dark side, and the highlights move over it.
- **Round** is where it is round the model in degrees: 0 in front, 90 to
  the right, 180 behind, 270 to the left. For a light with the camera,
  "in front" is toward you; for a fixed one it is the side the camera
  starts on.
- **Up** is how high it is: 0 level, 90 straight above, -90 straight
  below.
- **Strength**: 1 is a full light.
- **Colour**: the swatch opens its red, green and blue; or type a hex
  colour.
- **Shape**, folded, has three more. *Wrap* lets the light reach past
  side-on, as a large soft light does. *Falloff* is how fast it fades
  toward side-on, higher fading sooner. *Highlight* is how much shine it
  puts on a surface: 0 lights the form and leaves no highlight, as a fill
  light should.

## The global light

The light from all round: the **Sky** colour lights what faces up, the
**Ground** colour what faces down, and half of each what faces sideways.
**Strength** sets how much of it there is. A gloss, a metal and chrome
reflect it, so a brighter sky makes them brighter.

## The unpainted material

What every body looks made of where it is bare. Paint covers it.

- **Clay**: the warm grey the viewport has always had, a little cool in the
  shade. The default.
- **Polystyrene**: the plastic of a kit, smooth with a soft broad shine. A
  mid grey until you set another colour.
- **Green stuff**: mixed epoxy putty, blue-green, nearly matte and a little
  waxy. Its colour is its own.
- **Pewter**: a dull warm grey metal, fairly rough, so its form reads by
  what it reflects more than by shade. Its colour is its own.
- **Resin**: a printed or cast resin that lets a little light in, so its
  shadows are soft and its thin edges glow when lit from behind. Milky white
  until you set another colour; the three swatches under it are milky
  white, grey and a light tan, the usual resins.

Polystyrene and resin each keep their own colour.

## Reset

*Reset* puts back Studio and the clay, and the two colours.

## Where it is kept

In `lighting.conf`, in the settings folder (on Linux `~/.config/hanisculpt`).
A file that cannot be read is taken as the defaults. In a browser nothing is
kept.

The paint's picture in the paint designer and on the rack is lit by its own
light, not by these.
