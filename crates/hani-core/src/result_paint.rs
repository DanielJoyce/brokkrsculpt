// SPDX-License-Identifier: AGPL-3.0-only

//! A boolean group's result drawn in its rows' paint, as a bake would leave
//! it, rather than in clay.
//!
//! Asked for by the maintainer on 2026-11-28, of the boolean groups'
//! not-done list: "continue on those gaps". A result is a field and
//! nothing else (`combine.rs`), so it meshed bare. Its paint is worked out
//! here, at the vertices of each brick as it is meshed, from the rows: the
//! base's paint, then each added row's where that row has material, which
//! is what [`crate::Document::bake_boolean`] does to the voxels (a merge's
//! `union_colour_from` and `union_paint_from`); a cut or a kept face is
//! bare, as a bake leaves it. A nested combining folder stands for its own
//! result's paint, worked out the same way; a plain folder for the union of
//! its shown children's.
//!
//! Nothing is stored: the result's field is built and kept as before, and
//! its paint read from the rows each time a brick of it is meshed, which is
//! as often as the rows change.
//!
//! What `body.rs` holds for this: the mesher's lookup hands a result here
//! (`mesh_one`).

use glam::IVec3;

use crate::body::{Document, NodeId};
use crate::brick::BrickCoord;
use crate::combine::BoolOp;
use crate::mesh::{BrickMesh, MeshScratch};
use crate::paint::field::Rgba;

/// What a row paints at each cell, and whether it has material there.
struct RowPaint {
    slots: Vec<u8>,
    rgba: Vec<Rgba>,
    material: Vec<bool>,
}

impl RowPaint {
    fn bare(cells: usize) -> Self {
        Self { slots: vec![0; cells], rgba: vec![[0; 4]; cells], material: vec![false; cells] }
    }

    /// Where `row` has material, its paint over this.
    fn add(&mut self, row: RowPaint) {
        for index in 0..self.material.len() {
            if row.material[index] {
                self.slots[index] = row.slots[index];
                self.rgba[index] = row.rgba[index];
                self.material[index] = true;
            }
        }
    }
}

impl Document {
    /// Mesh one brick of a combining folder's result, painted from its rows.
    pub(crate) fn mesh_result(
        &self,
        folder: NodeId,
        coord: BrickCoord,
        scratch: &mut MeshScratch,
        out: &mut BrickMesh,
    ) {
        let Some(result) = self.boolean_result(folder) else {
            out.clear();
            return;
        };
        result.mesh_brick(coord, scratch, out);
        if out.vertices.is_empty() {
            return;
        }
        // The slots first, as `Volume::mesh_brick` lays them, so the cut at a
        // boundary between two slots is made; then the paint, at every
        // vertex the cut left.
        out.colour = self.row_paint(folder, coord, &out.cells).slots;
        out.paint.clear();
        crate::mesh::split_paint_boundaries(out);
        out.paint = self.row_paint(folder, coord, &out.cells).rgba;
        debug_assert_eq!(out.paint.len(), out.vertices.len());
    }

    /// What a row paints at each of `cells`: a body its own paint, a
    /// combining folder the fold of its rows', a plain folder the union of
    /// its shown children's.
    fn row_paint(&self, id: NodeId, coord: BrickCoord, cells: &[IVec3]) -> RowPaint {
        let Some(node) = self.node(id) else {
            return RowPaint::bare(cells.len());
        };
        if let Some(volume) = node.volume() {
            let mut slots = Vec::with_capacity(cells.len());
            volume.colour().slots_at_cells(coord, cells, &mut slots);
            let mut rgba = Vec::with_capacity(cells.len());
            volume.display_paint().rgba_blended_at_cells(coord, cells, &mut rgba);
            let material = cells.iter().map(|cell| volume.has_material_at(*cell)).collect();
            return RowPaint { slots, rgba, material };
        }
        let rows: Vec<(NodeId, BoolOp)> = if self.combines(id) {
            self.operands(id)
        } else {
            self.children_of(id)
                .into_iter()
                .filter(|child| self.node(*child).is_some_and(|node| node.visible))
                .map(|child| (child, BoolOp::Add))
                .collect()
        };
        let mut painted = RowPaint::bare(cells.len());
        for (index, (row, op)) in rows.into_iter().enumerate() {
            match (index, op) {
                // The base: its paint, all of it.
                (0, _) => painted = self.row_paint(row, coord, cells),
                (_, BoolOp::Add) => painted.add(self.row_paint(row, coord, cells)),
                // A cut or kept face is bare: what was painted stays where
                // the base still is.
                (_, BoolOp::Cut | BoolOp::Keep) => {}
            }
        }
        painted
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::combine::BoolState;
    use crate::volume::Volume;

    fn ball(centre: Vec3, radius: f32) -> Volume {
        crate::primitive::build(crate::PrimitiveKind::Sphere, 0.5, centre, radius)
    }

    /// A paint laid over the whole of a body.
    fn painted(mut volume: Volume, rgba: Rgba) -> Volume {
        volume.edit_paint(IVec3::splat(-200), IVec3::splat(200), |_, _, _, _| rgba);
        volume
    }

    /// A red ball, a blue ball added at its side, a bare one cutting the
    /// other side.
    fn group() -> (Document, NodeId) {
        let mut doc = Document::new(0.5);
        let first = doc.active();
        doc.replace_volume(first, painted(ball(Vec3::ZERO, 6.0), [200, 0, 0, 255]));
        let boss = painted(ball(Vec3::new(6.0, 0.0, 0.0), 4.0), [0, 0, 200, 255]);
        let second = doc.add_body("Boss", boss);
        let third = doc.add_body("Hole", ball(Vec3::new(-6.0, 0.0, 0.0), 3.0));
        let (folder, _) = doc.group(first, "Pair").expect("room for a folder");
        doc.move_to_folder(second, Some(folder)).expect("moved");
        doc.move_to_folder(third, Some(folder)).expect("moved");
        doc.set_bool_state(folder, BoolState { combines: true, ..BoolState::default() });
        doc.set_bool_state(third, BoolState { op: BoolOp::Cut, ..BoolState::default() });
        let mut dirty = Vec::new();
        doc.refresh_booleans(&mut dirty);
        (doc, folder)
    }

    /// The paint at the vertex nearest a point, over the bricks that
    /// `mesh` meshes.
    fn nearest(
        coords: &[BrickCoord],
        at: Vec3,
        mesh: impl Fn(BrickCoord, &mut MeshScratch, &mut BrickMesh),
    ) -> Rgba {
        let mut scratch = MeshScratch::default();
        let mut best = (f32::MAX, [0; 4]);
        for coord in coords {
            let mut out = BrickMesh::default();
            mesh(*coord, &mut scratch, &mut out);
            assert_eq!(out.paint.len(), out.vertices.len());
            assert_eq!(out.colour.len(), out.vertices.len());
            for (vertex, paint) in out.vertices.iter().zip(&out.paint) {
                let distance = Vec3::from(vertex.position).distance(at);
                if distance < best.0 {
                    best = (distance, *paint);
                }
            }
        }
        best.1
    }

    /// The base's paint where it shows, an added row's where it shows, a
    /// cut face bare; and the bake leaves the same paint where it shows.
    #[test]
    fn a_result_is_painted_as_its_bake_would_be() {
        let (mut doc, folder) = group();
        let coords: Vec<BrickCoord> =
            doc.boolean_result(folder).expect("a result").brick_coords().collect();
        let live = |at| {
            nearest(&coords, at, |coord, scratch, out| doc.mesh_result(folder, coord, scratch, out))
        };
        let top = Vec3::new(0.0, 6.0, 0.0);
        let far = Vec3::new(10.0, 0.0, 0.0);
        let (base, boss) = (live(top), live(far));
        assert!(base[0] > 150 && base[2] < 50, "the base's top: {base:?}");
        assert!(boss[2] > 150 && boss[0] < 50, "the added ball's far side: {boss:?}");
        let face = live(Vec3::new(-3.0, 0.0, 0.0));
        assert!(face[3] < 50, "the cut face inside the base: {face:?}");

        let _ = doc.bake_boolean(folder).expect("baked");
        let baked = doc.volume(doc.active()).expect("the baked body");
        let coords: Vec<BrickCoord> = baked.brick_coords().collect();
        for (at, before) in [(top, base), (far, boss)] {
            let after =
                nearest(&coords, at, |coord, scratch, out| baked.mesh_brick(coord, scratch, out));
            assert_eq!(after, before, "the bake and the live result differ at {at}");
        }
    }
}
