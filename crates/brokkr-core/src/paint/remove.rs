// SPDX-License-Identifier: AGPL-3.0-only

//! Taking a paint out of a document.
//!
//! A voxel names its paint by number ([`super::PaintTable`]), so a paint
//! cannot simply leave the table: every paint after it would be called by
//! the number of the one before, and the model would change colour from
//! that paint on. Two things are done together, and the application
//! records both in one undo entry so that they come back together:
//!
//! 1. [`PaintTable::remove`] takes the paint out of the table.
//! 2. [`Volume::forget_paint`], on every body, bares the voxels that held
//!    it and moves every later paint's voxels down a number.
//!
//! # What is not renumbered
//!
//! A number past the end of the table is not a paint. It is a filament
//! slot, as every number was before the document had paints
//! ([`super::bake::Bake::flat_colour`]), and it keeps its number: filament
//! 6 does not become filament 5 because a paint was thrown away.
//!
//! # The limit
//!
//! The renumbering goes through [`Volume::edit_colour`], which is the
//! recorded, undoable way into the colour field, and which only opens
//! voxels in the narrow band: the ones a brush could have painted. A voxel
//! that was painted and has since been buried deeper than the band keeps
//! its old number. Nothing draws it and no export reads it. If the surface
//! is later carved back down to it, it shows as whichever paint has that
//! number then.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Paint, PaintTable};
use crate::body::Document;
use crate::undo::Change;
use crate::volume::Volume;

/// The number a voxel holds once paint `removed` has left a table that had
/// `paints_before` paints in it.
///
/// The paint itself goes to 0, bare. Paints after it move down one. A
/// number that was never a paint, 0 and anything past the end of the
/// table, is left alone.
pub fn renumbered(slot: u8, removed: u8, paints_before: u8) -> u8 {
    // A paint past the end of the table needs no test of its own: every
    // number that is a paint is less than it, and is left alone below.
    if removed == 0 || slot > paints_before {
        return slot;
    }
    match slot.cmp(&removed) {
        std::cmp::Ordering::Less => slot,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => slot - 1,
    }
}

impl PaintTable {
    /// Take the paint `slot` names out of the table and hand it back, or
    /// `None`, and nothing changed, when it names none.
    ///
    /// Every later paint is then named by one number less, which is only
    /// true of the model once [`Volume::forget_paint`] has been run on
    /// every body with the same `slot` and the length the table had BEFORE
    /// this call.
    pub fn remove(&mut self, slot: u8) -> Option<Paint> {
        let index = usize::from(slot).checked_sub(1)?;
        (index < self.paints.len()).then(|| self.paints.remove(index))
    }

    /// Roughly what the table costs to keep a copy of, for the undo
    /// history's accounting.
    pub fn bytes(&self) -> usize {
        size_of::<Self>()
            + self.paints.iter().map(|paint| size_of::<Paint>() + paint.name.len()).sum::<usize>()
    }
}

impl Volume {
    /// Renumber this body's paint for a table that paint `removed` has
    /// left, having held `paints_before` paints: see [`renumbered`].
    ///
    /// Returns how many voxels were bared. Protection is not consulted: a
    /// mask says what a brush may touch, and this is not a brush. A masked
    /// voxel left holding the old number would be the wrong paint.
    ///
    /// Recorded, when a stroke is being recorded, like any other edit of
    /// the colour field.
    pub fn forget_paint(&mut self, removed: u8, paints_before: u8) -> usize {
        let Some((low, high)) = self.world_bounds() else {
            return 0;
        };
        let (lo, hi) = self.voxel_bounds(low, high);
        let bared = AtomicUsize::new(0);
        self.edit_colour(lo, hi, |_, _, held, _| {
            let now = renumbered(held, removed, paints_before);
            if now == 0 && held != 0 {
                bared.fetch_add(1, Ordering::Relaxed);
            }
            now
        });
        bared.into_inner()
    }
}

/// What taking a paint out of a document did.
pub struct Removed {
    /// The paint that left.
    pub paint: Paint,
    /// How many voxels it was on, over every body. They are bare now.
    pub bared: usize,
    /// Everything that changed, to push as ONE undo entry: the table as it
    /// was, and the renumbered voxels of each body that had any.
    pub changes: Vec<Change>,
}

impl Document {
    /// Change the document's paints, and hand back the change that undoes
    /// it, or `None` when `edit` left the table as it was.
    ///
    /// For everything but removing a paint, which has voxels to renumber
    /// too: see [`Document::remove_paint`].
    pub fn change_paints(&mut self, edit: impl FnOnce(&mut PaintTable)) -> Option<Change> {
        let before = self.paints().clone();
        let mut after = before.clone();
        edit(&mut after);
        if after == before {
            return None;
        }
        self.replace_paints(after);
        Some(Change::Paints { body: self.active(), paints: Box::new(before) })
    }

    /// Take the paint `slot` names out of the document: out of the table,
    /// and off every body, hidden ones too, with every later paint's voxels
    /// moved down a number to go on naming it.
    ///
    /// `None`, and nothing changed, when `slot` names no paint. No stroke
    /// may be being recorded on any body: this records its own.
    pub fn remove_paint(&mut self, slot: u8) -> Option<Removed> {
        let before = self.paints().clone();
        let mut after = before.clone();
        let paint = after.remove(slot)?;
        let paints_before = before.len() as u8;
        let (mut changes, mut bared) = (Vec::new(), 0);
        for (body, volume) in self.bodies_mut() {
            debug_assert!(!volume.is_recording(), "a stroke is open on a body");
            volume.begin_stroke();
            bared += volume.forget_paint(slot, paints_before);
            if let Some(edit) = volume.end_stroke() {
                changes.push(Change::Bricks { body, edit });
            }
        }
        self.replace_paints(after);
        changes.push(Change::Paints { body: self.active(), paints: Box::new(before) });
        Some(Removed { paint, bared, changes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::{Entry, History, UndoOutcome};
    use glam::{IVec3, Vec3};

    fn table(count: u8) -> PaintTable {
        let mut table = PaintTable::default();
        for index in 0..count {
            table.add(Paint::flat(format!("paint {}", index + 1), [index, 0, 0]));
        }
        table
    }

    /// A ball in six bands of latitude, numbered 0 to 5 from the bottom.
    fn banded() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::ZERO, 12.0);
        volume.edit_colour(IVec3::splat(-20), IVec3::splat(20), |_, at, _, _| {
            ((at.y + 12.0) / 4.0).floor().clamp(0.0, 5.0) as u8
        });
        volume
    }

    /// How many voxels of the band hold each number.
    fn census(volume: &Volume) -> [usize; 8] {
        let mut counts = [0; 8];
        for z in -16..=16 {
            for y in -16..=16 {
                for x in -16..=16 {
                    let cell = IVec3::new(x, y, z);
                    if (volume.sample_voxel(cell)).abs() < 1.0 {
                        counts[usize::from(volume.colour().at(cell))] += 1;
                    }
                }
            }
        }
        counts
    }

    /// The paint removed is bare, the paints after it are one number down,
    /// and everything else is as it was: the numbers before it, bare, and
    /// the numbers that were never paints.
    #[test]
    fn a_removed_paints_voxels_are_bared_and_later_paints_move_down() {
        // Three paints, so 4 and 5 on the model are filament slots.
        let mut volume = banded();
        let before = census(&volume);
        assert!(before[..6].iter().all(|count| *count > 50), "{before:?}");

        let bared = volume.forget_paint(2, 3);
        let after = census(&volume);
        assert_eq!(after[0], before[0] + before[2], "what was paint 2 is not bare");
        assert_eq!(after[1], before[1], "paint 1 changed");
        assert_eq!(after[2], before[3], "paint 3 is not paint 2 now");
        assert_eq!(after[3], 0, "something is still called paint 3");
        assert_eq!((after[4], after[5]), (before[4], before[5]), "a filament slot moved");
        assert!(bared >= before[2], "{bared} voxels reported bared, {} seen", before[2]);

        // The rule itself, for every number there is.
        for slot in 0..=255_u8 {
            let wanted = match slot {
                2 => 0,
                3 => 2,
                other => other,
            };
            assert_eq!(renumbered(slot, 2, 3), wanted, "slot {slot}");
            // A paint that is not in the table is not removed.
            assert_eq!(renumbered(slot, 0, 3), slot);
            assert_eq!(renumbered(slot, 4, 3), slot);
        }
    }

    /// The last paint and the first go the same way, and a mask is not
    /// asked: a protected voxel is renumbered with the rest.
    #[test]
    fn the_first_and_the_last_paint_can_go_and_a_mask_does_not_stop_it() {
        let mut volume = banded();
        let before = census(&volume);
        volume.forget_paint(5, 5);
        let after = census(&volume);
        assert_eq!(after[0], before[0] + before[5]);
        assert_eq!(after[1..5], before[1..5]);

        let mut volume = banded();
        volume.edit_mask(IVec3::splat(-20), IVec3::splat(20), |_, _, _| crate::PROTECTED);
        volume.forget_paint(1, 5);
        let after = census(&volume);
        assert_eq!(after[0], before[0] + before[1], "a masked voxel kept the old number");
        assert_eq!(after[1..5], before[2..6]);
        assert_eq!(after[5], 0);
    }

    /// Recorded like a stroke: undoing it puts every number back.
    #[test]
    fn forgetting_a_paint_is_recorded_and_can_be_put_back() {
        let mut volume = banded();
        let before = census(&volume);
        volume.begin_stroke();
        volume.forget_paint(2, 5);
        let edit = volume.end_stroke().expect("the renumbering changed the field");
        assert_ne!(census(&volume), before);
        let redo = volume.apply_edit(edit);
        assert_eq!(census(&volume), before);
        volume.apply_edit(redo);
        assert_eq!(census(&volume)[5], 0);

        // A paint nothing holds changes nothing and records nothing.
        let mut plain = Volume::new(1.0);
        plain.seed_sphere(Vec3::ZERO, 12.0);
        plain.begin_stroke();
        assert_eq!(plain.forget_paint(1, 1), 0);
        assert!(plain.end_stroke().is_none());
    }

    /// A paint taken out of a document of two bodies leaves the table and
    /// both bodies, in one undo entry that puts all three back, and one
    /// redo that takes them out again.
    #[test]
    fn a_paint_leaves_every_body_and_comes_back_with_one_undo() {
        let mut doc = Document::from_volume(banded());
        let second = doc.add_body("Second", banded());
        let first = doc.nodes()[0].id;
        assert!(doc.change_paints(|paints| *paints = table(5)).is_some());
        let painted = doc.paints().clone();
        let before = census(doc.volume(first).expect("the first body"));
        let seen =
            |doc: &Document| [first, second].map(|body| census(doc.volume(body).expect("a body")));
        assert_eq!(seen(&doc), [before, before]);

        let removed = doc.remove_paint(3).expect("paint 3 is there");
        assert_eq!(removed.paint.name, "paint 3");
        // Every voxel of the band that held it, on both bodies: more than
        // the census counts, which looks only within a voxel of the surface.
        assert!(
            removed.bared >= 2 * before[3] && removed.bared.is_multiple_of(2),
            "{}",
            removed.bared
        );
        assert_eq!(doc.paints().len(), 4);
        assert_eq!(doc.paints().paint(3).map(|paint| paint.name.as_str()), Some("paint 4"));
        let mut gone = before;
        (gone[0], gone[3], gone[4], gone[5]) = (before[0] + before[3], before[4], before[5], 0);
        assert_eq!(seen(&doc), [gone, gone]);

        // One entry: both bodies and the table.
        assert_eq!(removed.changes.len(), 3);
        let mut history = History::default();
        history.push(Entry::new(removed.changes));
        let shown = vec![true; doc.nodes().len()];
        assert!(matches!(history.undo(&mut doc, &shown), UndoOutcome::Applied(_)));
        assert_eq!(*doc.paints(), painted);
        assert_eq!(seen(&doc), [before, before]);
        assert!(matches!(history.redo(&mut doc, &shown), UndoOutcome::Applied(_)));
        assert_eq!(doc.paints().len(), 4);
        assert_eq!(seen(&doc), [gone, gone]);
        assert!(matches!(history.undo(&mut doc, &shown), UndoOutcome::Applied(_)));
        assert_eq!(seen(&doc), [before, before]);

        // A hidden body is renumbered with the rest, and undoing that is
        // refused while it is hidden, as undoing a stroke on it would be:
        // nothing changes off screen.
        let removed = doc.remove_paint(1).expect("paint 1 is there");
        history.push(Entry::new(removed.changes));
        let hidden: Vec<bool> = doc.nodes().iter().map(|node| node.id != second).collect();
        assert_eq!(history.undo(&mut doc, &hidden), UndoOutcome::Refused(second));
        assert_eq!(doc.paints().len(), 4, "a refused undo changed the table");

        // A paint that is not there is not removed, and nothing changes.
        assert!(doc.remove_paint(0).is_none());
        assert!(doc.remove_paint(9).is_none());
        assert_eq!(doc.paints().len(), 4);
    }

    /// A change to the table that is no change records nothing, and one
    /// that is comes back with an undo, with no body touched.
    #[test]
    fn a_change_to_the_paints_is_one_undo_and_no_change_is_none() {
        let mut doc = Document::from_volume(banded());
        assert!(doc.change_paints(|_| {}).is_none());
        let change = doc.change_paints(|paints| *paints = table(2)).expect("a change");
        assert!(doc.change_paints(|paints| *paints = table(2)).is_none());
        let before = census(doc.volume(doc.active()).expect("the body"));

        let mut history = History::default();
        history.push(Entry::new(vec![change]));
        assert!(history.stats().bytes > 0, "a paint table weighs nothing in the history");
        let edited = doc.change_paints(|paints| {
            paints.paint_mut(2).expect("paint 2").base = [9, 9, 9];
        });
        history.push(Entry::new(vec![edited.expect("a change")]));
        assert_eq!(doc.paints().paint(2).expect("paint 2").base, [9, 9, 9]);

        // With the body hidden, too: the rack is on the panel whatever the
        // eye says, and a hidden body must not hold up the undo of a
        // change that touches no body.
        let shown = vec![false; doc.nodes().len()];
        assert_eq!(history.undo(&mut doc, &shown), UndoOutcome::Applied(doc.active()));
        assert_eq!(*doc.paints(), table(2));
        assert_eq!(history.undo(&mut doc, &shown), UndoOutcome::Applied(doc.active()));
        assert!(doc.paints().is_empty(), "the document's first paints were not undone");
        assert_eq!(history.redo(&mut doc, &shown), UndoOutcome::Applied(doc.active()));
        assert_eq!(history.redo(&mut doc, &shown), UndoOutcome::Applied(doc.active()));
        assert_eq!(doc.paints().paint(2).expect("paint 2").base, [9, 9, 9]);
        assert_eq!(census(doc.volume(doc.active()).expect("the body")), before);
    }

    /// The table gives up the paint asked for and only that one, and
    /// refuses a number that names none.
    #[test]
    fn the_table_gives_up_one_paint_and_keeps_the_order_of_the_rest() {
        let mut paints = table(4);
        let gone = paints.remove(2).expect("paint 2 is there");
        assert_eq!(gone.name, "paint 2");
        let names: Vec<&str> = paints.paints().iter().map(|paint| paint.name.as_str()).collect();
        assert_eq!(names, ["paint 1", "paint 3", "paint 4"]);
        assert_eq!(paints.paint(2).map(|paint| paint.name.as_str()), Some("paint 3"));

        let before = paints.clone();
        assert_eq!(paints.remove(0), None);
        assert_eq!(paints.remove(4), None);
        assert_eq!(paints, before);
        assert!(paints.bytes() > 3 * size_of::<Paint>());
        assert!(table(5).bytes() > paints.bytes());
    }
}
