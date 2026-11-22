// SPDX-License-Identifier: AGPL-3.0-only

//! A painted body as something a printer can use: a colour at every vertex,
//! or a filament for every paint.
//!
//! A paint is worked out per pixel on the screen and is not stored anywhere
//! ([`crate::paint`]). A file has to hold something, and there are two things
//! a file can hold, for two kinds of machine.
//!
//! * **A full colour process** (binder jet, material jet, a colour print
//!   service) takes a colour at every point. [`Bake::vertex_colours`] works
//!   out the colour of each vertex of an export mesh with [`Paint::shade`],
//!   the CPU copy of the arithmetic the shader does, from the same crease
//!   signal at the same gain. What is exported is what was on screen, with
//!   the viewport's light left out.
//! * **A filament printer** takes a filament for every triangle, and has a
//!   handful of them. [`Bake::filament_slots`] sends each paint to the
//!   filament it names, or to the loaded filament nearest its colour when it
//!   names none, and hands back slots the 3MF writer already understands.
//!   A wash does not survive this, and cannot: the machine has four colours.
//!
//! Neither changes the document. Both are worked out for one export and
//! thrown away.

use glam::Vec3;

use super::{Paint, PaintTable, srgb};
use crate::export::ExportMesh;
use crate::volume::Volume;

/// What a bake needs to know that the document does not hold.
#[derive(Debug, Clone, Copy)]
pub struct Bake<'a> {
    pub table: &'a PaintTable,
    /// The colour of each filament slot on the machine, sRGB, slot 1 first.
    pub filaments: &'a [[u8; 3]],
    /// The colour of a surface nobody painted, sRGB.
    pub unpainted: [u8; 3],
    /// How many units of the crease signal a full crease is: the viewport's
    /// gain ([`crate::signals`]), so a wash pools in the file where it pooled
    /// on screen.
    pub crease_gain: f32,
}

impl Bake<'_> {
    /// The paint a slot names, if the document has paints and this is one.
    fn paint(&self, slot: u8) -> Option<&Paint> {
        self.table.paint(slot)
    }

    /// The colour of a slot on a flat, sRGB: what a rack or a legend shows.
    ///
    /// A slot the table has no paint for is a filament slot, as it was
    /// before paints existed, and is its filament's colour. Slot 0, and a
    /// slot that is neither, is the unpainted colour.
    pub fn flat_colour(&self, slot: u8) -> [u8; 3] {
        self.colour(slot, 0.0)
    }

    fn colour(&self, slot: u8, crease: f32) -> [u8; 3] {
        if let Some(paint) = self.paint(slot) {
            return srgb(paint.shade(crease));
        }
        usize::from(slot)
            .checked_sub(1)
            .and_then(|index| self.filaments.get(index))
            .copied()
            .unwrap_or(self.unpainted)
    }

    /// One sRGB colour for each vertex of `mesh`, which must be a mesh of
    /// `volume`: the paint its slot names, shaded by how creased the surface
    /// is at it.
    ///
    /// A mesh with no slots at all is unpainted throughout. The crease is
    /// read from the field at the vertex and not carried by the mesh, so
    /// this is as good for a decimated mesh as for a dense one, though a
    /// decimated mesh has fewer vertices to hold a wash in.
    pub fn vertex_colours(&self, volume: &Volume, mesh: &ExportMesh) -> Vec<[u8; 3]> {
        if mesh.slots.len() != mesh.positions.len() {
            return vec![self.unpainted; mesh.positions.len()];
        }
        let voxel_size = volume.voxel_size();
        mesh.positions
            .iter()
            .zip(&mesh.slots)
            .map(|(position, slot)| {
                // Only a paint with something to pool or to catch reads the
                // surface. The lookup is a hash a sample and thirty-two
                // samples, and most vertices of most models are a flat
                // paint or none.
                let crease = match self.paint(*slot) {
                    Some(paint) if !(paint.wash.is_none() && paint.highlight.is_none()) => {
                        crease_at(volume, *position, voxel_size) * self.crease_gain
                    }
                    _ => 0.0,
                };
                self.colour(*slot, crease)
            })
            .collect()
    }

    /// The filament a slot prints in, 1-based, or 0 for none.
    ///
    /// A paint that names a filament the machine has prints in it. One that
    /// names none, or names one the machine does not have, prints in the
    /// loaded filament nearest the paint's colour on a flat. A slot with no
    /// paint is a filament slot already and is left as it is.
    pub fn filament_of(&self, slot: u8) -> u8 {
        let Some(paint) = self.paint(slot) else {
            return slot;
        };
        if paint.filament != 0 && usize::from(paint.filament) <= self.filaments.len() {
            return paint.filament;
        }
        nearest(srgb(paint.shade(0.0)), self.filaments)
    }

    /// `mesh.slots` with every paint sent to its filament: what the 3MF
    /// writer is handed in place of the paint numbers.
    ///
    /// Empty when nothing is left assigned, on [`ExportMesh::slots`]' own
    /// rule that an unassigned mesh carries no slots at all.
    pub fn filament_slots(&self, mesh: &ExportMesh) -> Vec<u8> {
        let mut by_slot = [0_u8; 256];
        for (slot, filament) in by_slot.iter_mut().enumerate() {
            *filament = self.filament_of(slot as u8);
        }
        let slots: Vec<u8> = mesh.slots.iter().map(|slot| by_slot[usize::from(*slot)]).collect();
        if slots.iter().all(|slot| *slot == 0) { Vec::new() } else { slots }
    }
}

/// The crease signal at a world position, in 1/voxel, positive in a recess
/// and 0 where it cannot be measured: the number a vertex's cavity byte
/// carries to the shader, read here from the field.
fn crease_at(volume: &Volume, position: Vec3, voxel_size: f32) -> f32 {
    let cell = (position / voxel_size).floor().as_ivec3();
    volume.cavity_at_cell(cell).unwrap_or(0.0)
}

/// The 1-based index of the filament nearest a colour, or 0 when there are
/// no filaments.
///
/// Nearest by the "redmean" distance, a weighted distance in sRGB that
/// follows the eye better than a plain one and needs no colour space
/// (Thiadmer Riemersma, "Colour metric", compuphase.com). Good enough to
/// tell which of four spools a paint is closest to, which is all it is
/// asked. On a tie the lower slot wins, so the answer does not depend on
/// the order anything was iterated in.
pub fn nearest(colour: [u8; 3], filaments: &[[u8; 3]]) -> u8 {
    let distance = |other: &[u8; 3]| {
        let mean_red = (i32::from(colour[0]) + i32::from(other[0])) / 2;
        let red = i32::from(colour[0]) - i32::from(other[0]);
        let green = i32::from(colour[1]) - i32::from(other[1]);
        let blue = i32::from(colour[2]) - i32::from(other[2]);
        (((512 + mean_red) * red * red) >> 8)
            + 4 * green * green
            + (((767 - mean_red) * blue * blue) >> 8)
    };
    filaments
        .iter()
        .take(usize::from(u8::MAX))
        .enumerate()
        .min_by_key(|(index, filament)| (distance(filament), *index))
        .map_or(0, |(index, _)| index as u8 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::{INSIDE, NARROW_BAND, OUTSIDE};
    use crate::clip::ClipPlane;
    use crate::paint::{Highlight, Wash};
    use glam::IVec3;

    const VOXEL: f32 = 0.5;
    const HALF: f32 = 12.0;
    const BONE: [u8; 3] = [222, 208, 170];
    const CLAY: [u8; 3] = [150, 140, 130];
    const FILAMENTS: [[u8; 3]; 3] = [[230, 230, 230], [20, 20, 20], [200, 30, 30]];

    /// A cube with a square slot cut down the middle of its +z face: flats,
    /// convex edges, and two concave corners along the slot's floor.
    fn slotted_cube() -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(HALF + NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(HALF);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / VOXEL).clamp(INSIDE, OUTSIDE)
        });
        volume.clip_convex(&[
            ClipPlane { point: Vec3::new(-3.0, 0.0, 0.0), normal: Vec3::X },
            ClipPlane { point: Vec3::new(3.0, 0.0, 0.0), normal: -Vec3::X },
            ClipPlane { point: Vec3::new(0.0, 0.0, HALF - 5.0), normal: Vec3::Z },
        ]);
        volume
    }

    fn painted(slot: impl Fn(Vec3) -> u8) -> (Volume, ExportMesh) {
        let mut volume = slotted_cube();
        volume.edit_colour(IVec3::splat(-40), IVec3::splat(40), |_, at, _, _| slot(at));
        let (mesh, report) = volume.export_mesh();
        assert!(report.is_printable(), "the fixture does not weld shut");
        (volume, mesh)
    }

    fn table_of(paint: Paint) -> PaintTable {
        let mut table = PaintTable::default();
        table.add(paint).expect("an empty table took no paint");
        table
    }

    fn bake(table: &PaintTable) -> Bake<'_> {
        Bake { table, filaments: &FILAMENTS, unpainted: CLAY, crease_gain: 1.0 }
    }

    fn luma(colour: [u8; 3]) -> i32 {
        (2126 * i32::from(colour[0]) + 7152 * i32::from(colour[1]) + 722 * i32::from(colour[2]))
            / 10000
    }

    /// A plain paint is its colour at every vertex, an unpainted vertex is
    /// clay, and a body with no slots at all is clay throughout.
    #[test]
    fn a_plain_paint_bakes_to_its_colour_and_bare_clay_to_clay() {
        let table = table_of(Paint::flat("bone", BONE));
        let (volume, mesh) = painted(|at| u8::from(at.x < 0.0));
        let colours = bake(&table).vertex_colours(&volume, &mesh);
        assert_eq!(colours.len(), mesh.positions.len());
        let (mut bone, mut clay) = (0, 0);
        for ((colour, slot), position) in colours.iter().zip(&mesh.slots).zip(&mesh.positions) {
            match slot {
                1 => {
                    assert_eq!(*colour, BONE, "a painted vertex at {position}");
                    bone += 1;
                }
                _ => {
                    assert_eq!(*colour, CLAY, "an unpainted vertex at {position}");
                    clay += 1;
                }
            }
        }
        assert!(bone > 1000 && clay > 1000, "the fixture is {bone} painted and {clay} not");

        let (volume, mesh) = painted(|_| 0);
        assert!(mesh.slots.is_empty(), "an unpainted body carried slots");
        let colours = bake(&table).vertex_colours(&volume, &mesh);
        assert_eq!(colours.len(), mesh.positions.len());
        assert!(colours.iter().all(|colour| *colour == CLAY));
    }

    /// A wash bakes into the creases and a highlight onto the edges, and a
    /// flat is the base coat exactly. This is the screen's picture in a
    /// file: the same three places the pixel tests read.
    #[test]
    fn a_wash_bakes_into_the_creases_and_a_highlight_onto_the_edges() {
        let paint = Paint {
            wash: Wash { colour: [70, 30, 20], stain: 0.0, flow: 2.0, edge: 0.0 },
            highlight: Highlight { colour: [255, 255, 255], amount: 1.0, start: 0.2 },
            ..Paint::flat("bone", BONE)
        };
        let table = table_of(paint);
        let (volume, mesh) = painted(|_| 1);
        let colours = bake(&table).vertex_colours(&volume, &mesh);
        let near = |point: Vec3, within: f32| -> Vec<[u8; 3]> {
            mesh.positions
                .iter()
                .zip(&colours)
                .filter(|(at, _)| (**at - point).length() < within)
                .map(|(_, colour)| *colour)
                .collect()
        };
        // The middle of the big flat beside the slot.
        let flat = near(Vec3::new(-8.0, 0.0, HALF), 2.0);
        assert!(flat.len() > 20, "only {} vertices on the flat", flat.len());
        assert!(flat.iter().all(|colour| *colour == BONE), "the flat is not the base coat");
        // Where the slot's floor meets its wall.
        let crease = near(Vec3::new(-3.0, 0.0, HALF - 5.0), 0.6);
        let darkest = crease.iter().map(|c| luma(*c)).min().expect("no vertex in the crease");
        assert!(
            darkest < luma(BONE) - 40,
            "the darkest vertex in the crease is {darkest} against a base of {}",
            luma(BONE)
        );
        // The cube's own edge between the +x and +z faces.
        let edge = near(Vec3::new(HALF, 0.0, HALF), 0.6);
        let lightest = edge.iter().map(|c| luma(*c)).max().expect("no vertex on the edge");
        assert!(
            lightest > luma(BONE) + 20,
            "the lightest vertex on the edge is {lightest} against a base of {}",
            luma(BONE)
        );

        // Each half by itself. A paint with only a wash still reads the
        // surface, and so does one with only a highlight: the bake skips the
        // lookup for a paint with NEITHER, and that test is easy to get
        // backwards.
        let only = |paint: Paint| {
            let table = table_of(paint);
            let colours = bake(&table).vertex_colours(&volume, &mesh);
            let lumas = |point: Vec3| -> Vec<i32> {
                mesh.positions
                    .iter()
                    .zip(&colours)
                    .filter(|(at, _)| (**at - point).length() < 0.6)
                    .map(|(_, colour)| luma(*colour))
                    .collect()
            };
            (
                lumas(Vec3::new(-3.0, 0.0, HALF - 5.0)).into_iter().min().unwrap(),
                lumas(Vec3::new(HALF, 0.0, HALF)).into_iter().max().unwrap(),
            )
        };
        let washed = Paint {
            wash: Wash { colour: [70, 30, 20], stain: 0.0, flow: 2.0, edge: 0.0 },
            ..Paint::flat("bone", BONE)
        };
        let (in_crease, on_edge) = only(washed);
        assert!(in_crease < luma(BONE) - 40, "a paint with only a wash did not bake it");
        assert_eq!(on_edge, luma(BONE), "a wash with no run-off changed the edge");
        let edged = Paint {
            highlight: Highlight { colour: [255, 255, 255], amount: 1.0, start: 0.2 },
            ..Paint::flat("bone", BONE)
        };
        let (in_crease, on_edge) = only(edged);
        assert!(on_edge > luma(BONE) + 20, "a paint with only a highlight did not bake it");
        assert_eq!(in_crease, luma(BONE), "a highlight landed in the crease");

        // With no gain there is no crease and no edge anywhere: the control
        // for both, and the proof that the gain is read.
        let flattened = Bake { crease_gain: 0.0, ..bake(&table) }.vertex_colours(&volume, &mesh);
        assert!(flattened.iter().all(|colour| *colour == BONE), "a wash baked with no gain");
    }

    /// The bake is the engine's own arithmetic at the vertex's own cell:
    /// every vertex is `Paint::shade` of the field's curvature there, at the
    /// gain given. Nothing in between can drift.
    #[test]
    fn every_vertex_is_the_engines_shade_of_the_fields_curvature_there() {
        let paint = Paint {
            wash: Wash::contrast([30, 60, 150]),
            highlight: Highlight { colour: [250, 240, 200], amount: 0.7, start: 0.1 },
            ..Paint::flat("primer", [235, 235, 235])
        };
        let table = table_of(paint.clone());
        let (volume, mesh) = painted(|_| 1);
        let gain = 0.6;
        let colours = Bake { crease_gain: gain, ..bake(&table) }.vertex_colours(&volume, &mesh);
        let mut curved = 0;
        for (position, colour) in mesh.positions.iter().zip(&colours) {
            let cell = (*position / VOXEL).floor().as_ivec3();
            let cavity = volume.cavity_at_cell(cell).unwrap_or(0.0);
            assert_eq!(*colour, srgb(paint.shade(cavity * gain)), "at {position}");
            curved += usize::from(cavity.abs() > 0.2);
        }
        assert!(curved > 200, "only {curved} vertices are anywhere curved");
    }

    /// A slot the table has no paint for is a filament slot and bakes to its
    /// filament's colour; one the machine has no filament for is clay.
    #[test]
    fn a_slot_with_no_paint_bakes_to_its_filament() {
        let table = table_of(Paint::flat("bone", BONE));
        let (volume, mesh) = painted(|at| if at.x < 0.0 { 3 } else { 9 });
        let colours = bake(&table).vertex_colours(&volume, &mesh);
        for (colour, slot) in colours.iter().zip(&mesh.slots) {
            match slot {
                3 => assert_eq!(*colour, FILAMENTS[2]),
                9 => assert_eq!(*colour, CLAY),
                other => panic!("the fixture painted slot {other}"),
            }
        }
        let bake = bake(&table);
        assert_eq!(bake.flat_colour(0), CLAY);
        assert_eq!(bake.flat_colour(1), BONE);
        assert_eq!(bake.flat_colour(2), FILAMENTS[1]);

        // And with no paints at all, every slot is its filament.
        let none = PaintTable::default();
        let colours = Bake { table: &none, ..bake }.vertex_colours(&volume, &mesh);
        assert!(colours.iter().zip(&mesh.slots).all(|(colour, slot)| match slot {
            3 => *colour == FILAMENTS[2],
            _ => *colour == CLAY,
        }));
    }

    /// Which spool a paint prints from.
    #[test]
    fn a_paint_prints_in_the_filament_it_names_or_the_nearest() {
        let mut table = PaintTable::default();
        // 1: names filament 2, though it is nearly white.
        table.add(Paint { filament: 2, ..Paint::flat("named", [240, 240, 240]) });
        // 2: names none, and is dark.
        table.add(Paint::flat("dark", [40, 35, 30]));
        // 3: names a filament the machine does not have, and is red.
        table.add(Paint { filament: 9, ..Paint::flat("red", [180, 40, 40]) });
        // 4: names none, and its base is pale, but it is a contrast paint
        // that comes out red on a flat. The flat is what a spool has to match.
        table.add(Paint {
            wash: Wash::contrast([190, 20, 20]),
            ..Paint::flat("c", [235, 230, 220])
        });
        let bake = bake(&table);
        assert_eq!(bake.filament_of(1), 2, "a named filament was overridden");
        assert_eq!(bake.filament_of(2), 2, "a dark paint did not go to the black spool");
        assert_eq!(bake.filament_of(3), 3, "a filament the machine lacks was not replaced");
        assert_eq!(bake.filament_of(4), 3, "a contrast paint was matched by its undercoat");
        assert_eq!(bake.filament_of(0), 0, "unpainted was given a filament");
        assert_eq!(bake.filament_of(7), 7, "a slot with no paint is a filament slot already");

        // A machine with nothing loaded prints nothing in colour.
        let empty = Bake { filaments: &[], ..bake };
        assert_eq!(empty.filament_of(2), 0);
        assert_eq!(empty.filament_of(1), 0, "a named filament the machine lacks was kept");
    }

    /// The slots a 3MF is handed: every paint sent to its spool, vertex for
    /// vertex, and nothing at all when nothing is left assigned.
    #[test]
    fn the_slots_handed_to_a_printer_are_filaments() {
        let mut table = PaintTable::default();
        table.add(Paint { filament: 3, ..Paint::flat("a", BONE) });
        table.add(Paint { filament: 1, ..Paint::flat("b", BONE) });
        let (_, mesh) = painted(|at| if at.x < 0.0 { 1 } else { 2 });
        let slots = bake(&table).filament_slots(&mesh);
        assert_eq!(slots.len(), mesh.slots.len());
        let (mut threes, mut ones) = (0, 0);
        for (filament, paint) in slots.iter().zip(&mesh.slots) {
            match paint {
                1 => {
                    assert_eq!(*filament, 3);
                    threes += 1;
                }
                2 => {
                    assert_eq!(*filament, 1);
                    ones += 1;
                }
                other => panic!("the fixture painted slot {other}"),
            }
        }
        assert!(threes > 1000 && ones > 1000);

        // No filaments loaded: every paint goes to none, and a mesh with
        // nothing assigned carries no slots, on `ExportMesh::slots`' rule.
        let mut unnamed = PaintTable::default();
        unnamed.add(Paint::flat("a", BONE));
        unnamed.add(Paint::flat("b", BONE));
        let nothing = Bake { table: &unnamed, filaments: &[], ..bake(&unnamed) };
        assert!(nothing.filament_slots(&mesh).is_empty());
    }

    #[test]
    fn the_nearest_filament_is_the_one_an_eye_would_pick() {
        let spools = [[255, 255, 255], [0, 0, 0], [200, 30, 30], [30, 60, 200], [240, 200, 40]];
        assert_eq!(nearest([250, 250, 245], &spools), 1);
        assert_eq!(nearest([30, 25, 20], &spools), 2);
        assert_eq!(nearest([150, 20, 20], &spools), 3);
        assert_eq!(nearest([60, 90, 220], &spools), 4);
        assert_eq!(nearest([230, 180, 60], &spools), 5);
        // Green counts for most, as it does to an eye: a green is nearer a
        // paler green than a grey of the same red and blue.
        assert_eq!(nearest([100, 200, 100], &[[100, 100, 100], [160, 200, 160]]), 2);
        assert_eq!(nearest([1, 2, 3], &[]), 0, "no filaments, and one was picked");
        // A tie goes to the lower slot, whichever order they are in.
        assert_eq!(nearest([100, 100, 100], &[[90, 100, 100], [90, 100, 100]]), 1);
        assert_eq!(nearest([100, 100, 100], &[[0, 0, 0], [0, 0, 0], [255, 255, 255]]), 1);
        assert_eq!(nearest([100, 100, 100], &[[255, 255, 255], [0, 0, 0], [0, 0, 0]]), 2);
    }
}
