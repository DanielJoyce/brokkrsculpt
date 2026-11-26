<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# How it works

A broad view of what happens underneath. None of it needs knowing to sculpt,
but it explains why the tools behave as they do.

## The model is a field, not a mesh

The space round the model is divided into a fine grid of tiny cubes, *voxels*,
a quarter of a millimetre across to begin with. Each voxel holds one number:
how far it is from the surface, negative inside the material and positive
outside. The surface is where that number crosses zero. This is called a
*signed distance field*.

Only the voxels near the surface need a real distance; everything deep inside
is simply "inside" and everything far outside is "outside". So the grid is
stored in blocks of 32 by 32 by 32 voxels, *bricks*, and only the bricks the
surface passes through hold numbers at all. A brick wholly inside or outside is
one number, and empty space costs nothing. That is what lets a fine grid fit in
memory.

Because the model is solid, it cannot have a hole, a gap or a face turned
inside out. Whatever you do to it, it stays a closed, printable solid.

## A brush changes the field

A brush stroke is a run of *stamps* along the path of the pointer. Each stamp
changes the numbers in the voxels under it:

- **Draw** moves the surface out along its normal, by reading each voxel's
  value from a little further in.
- **Inflate** and **Deflate** shift every value up or down, so the surface
  swells or shrinks along its own normals.
- **Smooth** blends each value toward its neighbours.
- **Clay**, **Flatten** and **Scrape** pull the surface toward a plane under
  the brush.
- **Pinch** and **Widen** read values from further from, or nearer to, the
  line of the stroke, so whatever is under it gets narrower or wider and keeps
  its height.
- **Move** takes a copy of the field when you press and drags that region with
  the pointer, so the shape travels without being rebuilt.

Brushes read values from shifted places rather than adding to them, so they
cannot invent detail that was not there or grow a rough crust as strokes pile
up. The *mask* is a second field of protection, one value a voxel, that scales
how much a brush changes each place.

## From the field to the screen

The field is turned into triangles brick by brick: wherever the distance
changes sign between neighbouring voxels, the surface is placed between them.
Only the bricks a stroke touched are remeshed, on every core of the processor,
and their triangles go to the graphics card, which draws them with lighting
and shading. That is why a stroke on a large model still draws at once.

## Paint lives in the voxels too

Paint is stored on the voxels at the surface, layer by layer, each layer with
its own blend and opacity. A wash reads how shut in each place is and pools in
the recesses; a drybrush reads how much each place stands out and catches only
the edges. Because paint belongs to the material, it moves when the surface is
moved and is cut when the model is cut.

## Getting it out

- **Exporting** meshes the whole field at full detail, then can thin the mesh
  for a smaller file while keeping the shape within a fraction of a voxel.
- **3MF** carries the colours a slicer can use. **OBJ with a picture** and
  **GLB** lay the mesh flat (*unwrapping*) and bake the paint into a texture,
  and GLB adds the recesses' shade, the varnish's shine and a map of the fine
  detail, so a thinned mesh is lit as the full one is.
- **Parting** cuts a body in two with a peg on one side and a socket in the
  other, sized by the clearance your printer needs.

## Undo and files

Before a stroke changes a brick, a copy of the brick as it was is kept, packed
small, so undoing puts back exactly the bricks a stroke touched. A `.hani` file
holds the bricks, the mask, the paint and the history of the session.

## In a browser

The same program also builds for the web and runs on WebGPU. There it works on
one core, and opening and saving files is not built yet.
