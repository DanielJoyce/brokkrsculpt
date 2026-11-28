<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# What HaniSculpt is

HaniSculpt is a digital sculpting application for people who **print what they
make**. You start from a ball of clay, a scan or a model you downloaded, push
it around with brushes until it looks right, paint it if you like, and send it
to your slicer or another program.

## What makes it different

Most sculpting programs move the points of a mesh, a skin of triangles. Stretch
that skin far enough and it tears, folds through itself or leaves holes, and a
model with holes will not print.

HaniSculpt sculpts **solid material**. Every point in space knows whether it is
inside the model or outside it. A brush changes which points are inside, and
the surface you see is simply where inside meets outside. So the model is
always closed and always printable, however hard you push it, and you never
have to repair it before a print.

## What you can do with it

- **Sculpt** with a palette of brushes: draw, clay, smooth, inflate and
  deflate, pinch and widen, flatten, scrape, crease, ridges and beads, rows of
  rivets and scales, and pictures pressed into the surface.
- **Add shapes**: balls, boxes, cylinders, cones, rings, cables and pipes, set
  down on the model by a drag, joined in or kept as parts of their own.
- **Model** with whole-body tools: hollow a body to a wall (*Shell*), lift a
  masked area into a plate of its own (*Extract*), twist, bend or taper a body
  (*Deform*), and spin a drawn line round an axis into a solid (*Revolve*).
- **Arrange parts**: several bodies in one document, in folders, moved, turned
  and scaled with a gizmo, split, merged, cut and parted with pegs and sockets
  so a big model prints in pieces that fit.
- **Paint** with real hobby paints from a rack of over nine thousand: base
  coats, washes that run into the recesses, drybrushing that catches the
  edges, sponging, an airbrush, decals, varnish, and layers.
- **Print**: export STL, OBJ, 3MF or PLY at full detail or thinned, see where
  the model overhangs, print a fit test for your printer's clearances, and
  export OBJ with a picture or GLB with the paint baked to textures.

## Where to go next

- [Getting around](help:getting-around): the workspaces, the panels and the
  keys.
- [How it works](help:how-it-works): what happens underneath, in broad terms.
- [Lighting and materials](help:lighting): the lights on the model, and what
  it looks made of where it is not painted.
- [The source](https://github.com/DanielJoyce/hanisculpt), on GitHub: the code,
  the releases, and where to report a problem.

HaniSculpt began as a fork of BrokkrSculpt, by MakerViking, and grew from
there. Its mascot is Haniko, a little terracotta figure who wandered in from a
photogrammetry scan.
