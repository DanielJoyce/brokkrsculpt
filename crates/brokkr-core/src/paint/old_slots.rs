// SPDX-License-Identifier: AGPL-3.0-only

//! A document painted by filament slot, opened now that painting by slot
//! is gone: each slot it painted is laid into the colour paint, as the
//! colour it showed, and the slots and their recipes are let go.
//!
//! The maintainer removed filament painting on 2026-11-25. The slot field
//! and the paint table are still read from a file, for this and nothing
//! else. A slot named in the document's paint table shows its recipe's
//! base colour; any other shows its filament's colour. Where the body
//! already has colour paint, that paint stays, as it was drawn over the
//! slot.

use glam::IVec3;

use crate::body::{Document, NodeId};
use crate::colour::ColourField;
use crate::paint::PaintTable;

impl Document {
    /// Lay every body's slot paint into its colour paint, in the colour
    /// each slot showed, and let the slots and the paint table go. `true`
    /// when anything was laid. `filaments` is the colour of each slot from
    /// 1, the machine's.
    pub fn lay_slots_into_paint(&mut self, filaments: &[[u8; 3]]) -> bool {
        let table = self.replace_paints(PaintTable::default());
        let colour_of = |slot: u8| -> Option<[u8; 3]> {
            if slot == 0 {
                return None;
            }
            if let Some(paint) = table.paint(slot) {
                return Some(paint.base);
            }
            filaments.get(usize::from(slot) - 1).copied().or(Some([128, 128, 128]))
        };
        let ids: Vec<NodeId> =
            self.nodes().iter().filter(|node| node.is_body()).map(|node| node.id).collect();
        let mut laid = false;
        for id in ids {
            let Some(volume) = self.volume_mut(id) else {
                continue;
            };
            if volume.colour().is_empty() {
                continue;
            }
            let slots = std::mem::take(volume.colour_mut());
            let coords: Vec<_> = volume.brick_coords().collect();
            for coord in coords {
                volume.edit_paint(coord.origin(), coord.max_voxel(), |voxel: IVec3, _, held, _| {
                    if held[3] != 0 {
                        return held;
                    }
                    match colour_of(slots.at(voxel)) {
                        Some([r, g, b]) => [r, g, b, 255],
                        None => held,
                    }
                });
            }
            *volume.colour_mut() = ColourField::default();
            laid = true;
        }
        laid
    }
}

#[cfg(test)]
mod tests {
    use crate::body::Document;
    use crate::paint::{Paint, PaintTable};
    use crate::volume::Volume;
    use glam::{IVec3, Vec3};

    /// Slots become colour paint in the colour they showed: a recipe's
    /// base where the table names one, the filament's otherwise. Paint
    /// already there stays. The slots and the table are let go.
    #[test]
    fn slots_become_the_colour_they_showed() {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        let (lo, hi) = (IVec3::splat(-30), IVec3::splat(30));
        volume.edit_colour(lo, hi, |_, at, _, _| if at.x > 0.0 { 1 } else { 2 });
        volume.edit_paint(lo, hi, |_, at, held, _| if at.y > 6.0 { [9, 9, 9, 255] } else { held });
        let mut doc = Document::from_volume(volume);
        let mut table = PaintTable::default();
        table.add(Paint::flat("Red", [200, 10, 10]));
        doc.replace_paints(table);
        assert!(doc.lay_slots_into_paint(&[[1, 2, 3], [40, 200, 40]]));
        let volume = doc.active_volume();
        assert!(volume.colour().is_empty(), "the slots were kept");
        assert!(doc.paints().is_empty(), "the table was kept");
        assert_eq!(
            volume.paint().at(IVec3::new(20, 0, 0)),
            [200, 10, 10, 255],
            "slot 1 is the recipe"
        );
        assert_eq!(
            volume.paint().at(IVec3::new(-20, 0, 0)),
            [40, 200, 40, 255],
            "slot 2 is the filament"
        );
        assert_eq!(
            volume.paint().at(IVec3::new(0, 20, 0)),
            [9, 9, 9, 255],
            "paint already there went"
        );
        // A document with no slots is left as it is.
        let mut plain = Document::from_volume(Volume::new(0.5));
        assert!(!plain.lay_slots_into_paint(&[]));
    }
}
