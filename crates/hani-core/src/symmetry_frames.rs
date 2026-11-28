// SPDX-License-Identifier: AGPL-3.0-only

//! Symmetry that belongs to a body: which mirror planes are on, how many
//! radial copies round which axis, and the frame the planes and the axis
//! are in, carried with the body when the gizmo moves it.
//!
//! Asked for by the maintainer on 2026-11-28: "when you transform a node
//! with the gizmo the symmetry transforms with it (ie symmetry is local to
//! the object not global)", and then, of the settings, which were still one
//! for the session: "Each body should have its own settings. Yes. Just like
//! zbrush". A body starts with its symmetry off and the room's frame; a
//! body made from another takes that one's whole symmetry.
//!
//! A change to the settings is an undo step, [`Change::Symmetry`], asked
//! for by the maintainer: "Make symmetry setting changes undoable too"; so
//! is a move of the frame, [`Change::SymmetryFrame`], which goes with the
//! body's field. Only bodies whose symmetry is not off in the
//! room's frame are held, so a document that never used it holds none and
//! saves no byte for it.
//!
//! The file keeps every body's symmetry in a section of its own after the
//! boolean groups, `HANSYMM2`. The section this replaced, `HANSYMMS`, kept
//! the frames and one radial setting for the session (the planes were in
//! the file's view); it is still read, and the application lays a file's
//! session settings on every body. An older build stops reading before
//! either, as before every section it does not know.
//!
//! What `body.rs` holds for this: one field on [`Document`] and its value in
//! the two constructors. `undo.rs` has two changes, [`Change::SymmetryFrame`]
//! and [`Change::Symmetry`], and `project.rs` writes and reads the section.

use glam::{Quat, Vec3};
use rustc_hash::FxHashMap;

use crate::body::{Document, NodeId};
use crate::brush::{MirrorAxis, Symmetry, SymmetryFrame};
use crate::undo::Change;

/// The tag of the section that held the frames and the session's radial
/// copies, before each body had its own settings. Read, never written.
pub const TAG: &[u8; 8] = b"HANSYMMS";

/// The tag of the section that holds every body's symmetry.
pub const TAG2: &[u8; 8] = b"HANSYMM2";

/// A row of it: its place in the table (u32 LE), the planes (a bit each, X
/// first), the radial copies, the radial axis, and the frame as seven f32 LE.
const ROW2_BYTES: usize = 4 + 3 + 7 * 4;

/// The most that section can hold: a count and a row for every node.
pub const MOST_SECTION2_BYTES: u64 = 4 + ROW2_BYTES as u64 * crate::body::MAX_NODES as u64;

/// A row: its place in the table (u32 LE), the origin and the turn as seven
/// f32 LE.
const ROW_BYTES: usize = 4 + 7 * 4;

/// The most a section can hold: the radial copies and their axis, a count,
/// and a row for every node.
pub const MOST_SECTION_BYTES: u64 = 2 + 4 + ROW_BYTES as u64 * crate::body::MAX_NODES as u64;

/// Each body's symmetry, where it is not off in the room's frame.
#[derive(Debug, Clone, Default)]
pub struct Frames {
    frames: FxHashMap<NodeId, Symmetry>,
}

/// Whether a symmetry is the one a body starts with. The radial axis
/// counts though the copies are off: it is the choice the next copies take.
fn is_default(symmetry: &Symmetry) -> bool {
    symmetry.is_off()
        && symmetry.frame() == SymmetryFrame::IDENTITY
        && symmetry.radial_axis() == Symmetry::OFF.radial_axis()
}

/// The radial copies as the replaced section kept them for the session: how
/// many, the stroke's own among them (one is off), and round which axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Radial {
    pub copies: u8,
    pub axis: MirrorAxis,
}

impl Default for Radial {
    fn default() -> Self {
        Self { copies: 1, axis: MirrorAxis::Y }
    }
}

impl Radial {
    /// The copies a symmetry lays.
    pub fn of(symmetry: Symmetry) -> Self {
        Self { copies: symmetry.radial(), axis: symmetry.radial_axis() }
    }
}

impl Document {
    /// A body's symmetry: off, in the room's frame, until it is set.
    pub fn symmetry(&self, id: NodeId) -> Symmetry {
        self.frames.frames.get(&id).copied().unwrap_or(Symmetry::OFF)
    }

    /// A body's frame: the room's until the gizmo has moved it.
    pub fn symmetry_frame(&self, id: NodeId) -> SymmetryFrame {
        self.symmetry(id).frame()
    }

    /// Set a body's planes and radial copies from `settings`, keeping its
    /// frame, and answer the change that puts them back: `None` when nothing
    /// changes or the row is not a body here. A caller that keeps the change
    /// makes it an undo step; one that drops it (a plane turned off because
    /// the body no longer reaches it) does not.
    pub fn set_symmetry_settings(&mut self, id: NodeId, settings: Symmetry) -> Option<Change> {
        if !self.node(id).is_some_and(|node| node.is_body()) {
            return None;
        }
        let before = self.symmetry(id);
        let after = settings.in_frame(before.frame());
        if before == after {
            return None;
        }
        self.put_symmetry(id, after);
        Some(Change::Symmetry { id, symmetry: before })
    }

    /// A body's whole symmetry, frame and all, as it comes from a file or
    /// from the body a new one was made from. Not an undo step.
    pub fn put_symmetry(&mut self, id: NodeId, symmetry: Symmetry) {
        if is_default(&symmetry) {
            self.frames.frames.remove(&id);
        } else {
            self.frames.frames.insert(id, symmetry);
        }
    }

    /// Set a body's frame, and the change that puts it back; `None` when
    /// nothing changes or the row is not a body here.
    pub fn set_symmetry_frame(&mut self, id: NodeId, frame: SymmetryFrame) -> Option<Change> {
        if !self.node(id)?.is_body() {
            return None;
        }
        let before = self.symmetry_frame(id);
        if before == frame {
            return None;
        }
        self.put_symmetry_frame(id, frame);
        Some(Change::SymmetryFrame { id, frame: before })
    }

    /// Put a frame on a row and answer the one it had. The room's frame is
    /// held as no entry at all.
    pub(crate) fn put_symmetry_frame(&mut self, id: NodeId, frame: SymmetryFrame) -> SymmetryFrame {
        let before = self.symmetry(id);
        self.put_symmetry(id, before.in_frame(frame));
        before.frame()
    }
}

/// The replaced section for a document, kept to test that it is still
/// read: the radial copies (a byte for the count and one for the axis),
/// then for each body whose frame is not the room's, its place in the table
/// and the frame. `None` where the radial copies are off
/// and every frame is the room's, so that such a document gains no byte.
#[cfg(test)]
pub(crate) fn encode(doc: &Document, radial: Radial) -> Option<Vec<u8>> {
    let rows: Vec<(u32, SymmetryFrame)> = doc
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let frame = doc.symmetry_frame(node.id);
            (frame != SymmetryFrame::IDENTITY).then_some((index as u32, frame))
        })
        .collect();
    if rows.is_empty() && radial.copies <= 1 {
        return None;
    }
    let mut out = Vec::with_capacity(6 + rows.len() * ROW_BYTES);
    out.push(radial.copies);
    out.push(MirrorAxis::ALL.iter().position(|axis| *axis == radial.axis).unwrap_or(1) as u8);
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for (index, frame) in rows {
        out.extend_from_slice(&index.to_le_bytes());
        let numbers = [
            frame.origin.x,
            frame.origin.y,
            frame.origin.z,
            frame.rotation.x,
            frame.rotation.y,
            frame.rotation.z,
            frame.rotation.w,
        ];
        for number in numbers {
            out.extend_from_slice(&number.to_le_bytes());
        }
    }
    Some(out)
}

/// The radial copies and the frames a section holds, each frame by its place
/// in the table. What cannot be a frame (a number that is not finite, a turn
/// that is not near a unit quaternion) is left the room's rather than
/// refusing the file: a frame is where the planes are, not the sculpt.
pub(crate) fn decode(bytes: &[u8], rows: usize) -> (Radial, Vec<(usize, SymmetryFrame)>) {
    let mut radial = Radial::default();
    let Some((&[copies, axis], rest)) = bytes.split_first_chunk::<2>() else {
        return (radial, Vec::new());
    };
    radial.copies = copies.clamp(1, Symmetry::MAX_RADIAL);
    radial.axis = MirrorAxis::ALL.get(usize::from(axis)).copied().unwrap_or(MirrorAxis::Y);
    let Some((count, mut rest)) = rest.split_first_chunk::<4>() else {
        return (radial, Vec::new());
    };
    let count = u32::from_le_bytes(*count) as usize;
    let mut out = Vec::new();
    for _ in 0..count.min(crate::body::MAX_NODES) {
        let Some((row, after)) = rest.split_first_chunk::<ROW_BYTES>() else {
            break;
        };
        rest = after;
        let index = u32::from_le_bytes([row[0], row[1], row[2], row[3]]) as usize;
        let number = |at: usize| {
            let at = 4 + at * 4;
            f32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]])
        };
        let origin = Vec3::new(number(0), number(1), number(2));
        let rotation = Quat::from_xyzw(number(3), number(4), number(5), number(6));
        let unit = (rotation.length() - 1.0).abs() < 1.0e-3;
        if index < rows && origin.is_finite() && rotation.is_finite() && unit {
            out.push((index, SymmetryFrame { origin, rotation: rotation.normalize() }));
        }
    }
    (radial, out)
}

/// The section for a document: for each body whose symmetry is not off in
/// the room's frame, its place in the table and its symmetry. `None` where
/// there is none, so that such a document gains no byte.
pub(crate) fn encode2(doc: &Document) -> Option<Vec<u8>> {
    let rows: Vec<(u32, Symmetry)> = doc
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let symmetry = doc.symmetry(node.id);
            (!is_default(&symmetry)).then_some((index as u32, symmetry))
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(4 + rows.len() * ROW2_BYTES);
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for (index, symmetry) in rows {
        out.extend_from_slice(&index.to_le_bytes());
        let planes = MirrorAxis::ALL
            .into_iter()
            .enumerate()
            .fold(0u8, |bits, (bit, axis)| bits | (u8::from(symmetry.axis(axis)) << bit));
        out.push(planes);
        out.push(symmetry.radial());
        out.push(axis_code(symmetry.radial_axis()));
        let frame = symmetry.frame();
        let numbers = [
            frame.origin.x,
            frame.origin.y,
            frame.origin.z,
            frame.rotation.x,
            frame.rotation.y,
            frame.rotation.z,
            frame.rotation.w,
        ];
        for number in numbers {
            out.extend_from_slice(&number.to_le_bytes());
        }
    }
    Some(out)
}

/// Every body's symmetry the section holds, by its place in the table. A
/// frame that cannot be one is left the room's, as [`decode`] leaves it,
/// and the settings beside it kept: they are only switches.
pub(crate) fn decode2(bytes: &[u8], rows: usize) -> Vec<(usize, Symmetry)> {
    let Some((count, mut rest)) = bytes.split_first_chunk::<4>() else {
        return Vec::new();
    };
    let count = u32::from_le_bytes(*count) as usize;
    let mut out = Vec::new();
    for _ in 0..count.min(crate::body::MAX_NODES) {
        let Some((row, after)) = rest.split_first_chunk::<ROW2_BYTES>() else {
            break;
        };
        rest = after;
        let index = u32::from_le_bytes([row[0], row[1], row[2], row[3]]) as usize;
        if index >= rows {
            continue;
        }
        let (planes, copies, axis) = (row[4], row[5], row[6]);
        let number = |at: usize| {
            let at = 7 + at * 4;
            f32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]])
        };
        let origin = Vec3::new(number(0), number(1), number(2));
        let rotation = Quat::from_xyzw(number(3), number(4), number(5), number(6));
        let unit = (rotation.length() - 1.0).abs() < 1.0e-3;
        let frame = if origin.is_finite() && rotation.is_finite() && unit {
            SymmetryFrame { origin, rotation: rotation.normalize() }
        } else {
            SymmetryFrame::IDENTITY
        };
        let axis = MirrorAxis::ALL.get(usize::from(axis)).copied().unwrap_or(MirrorAxis::Y);
        let symmetry = MirrorAxis::ALL
            .into_iter()
            .enumerate()
            .fold(Symmetry::OFF, |set, (bit, plane)| set.with_axis(plane, planes & (1 << bit) != 0))
            .with_radial(copies, axis)
            .in_frame(frame);
        out.push((index, symmetry));
    }
    out
}

/// An axis as the file keeps it: 0, 1 or 2 for X, Y or Z.
fn axis_code(axis: MirrorAxis) -> u8 {
    MirrorAxis::ALL.iter().position(|each| *each == axis).unwrap_or(1) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::similarity::Similarity;

    fn turned() -> SymmetryFrame {
        SymmetryFrame {
            origin: Vec3::new(4.0, -2.0, 7.5),
            rotation: Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.3),
        }
    }

    /// A body starts in the room's frame, a frame set is held and undone,
    /// and the room's frame set again holds nothing.
    #[test]
    fn a_frame_is_set_undone_and_the_rooms_holds_nothing() {
        let mut doc = Document::new(0.5);
        let body = doc.active();
        assert_eq!(doc.symmetry_frame(body), SymmetryFrame::IDENTITY);
        let mut history = crate::undo::History::new(crate::DEFAULT_HISTORY_BUDGET);
        let change = doc.set_symmetry_frame(body, turned()).expect("a change");
        history.push(crate::undo::Entry::new(vec![change]));
        assert_eq!(doc.symmetry_frame(body), turned());
        assert!(doc.set_symmetry_frame(body, turned()).is_none(), "nothing changed");
        let visible = vec![true; doc.node_count()];
        let _ = history.undo(&mut doc, &visible);
        assert_eq!(doc.symmetry_frame(body), SymmetryFrame::IDENTITY);
        assert!(doc.frames.frames.is_empty(), "the room's frame is held as nothing");
        let _ = history.redo(&mut doc, &visible);
        assert_eq!(doc.symmetry_frame(body), turned());
    }

    /// A frame moved by a placement carries its origin as a point does and
    /// adds the turn; the scale has no say.
    #[test]
    fn a_frame_moves_with_its_placement() {
        let placement = Similarity {
            rotation: Quat::from_rotation_z(0.4),
            scale: Vec3::splat(2.0),
            translation: Vec3::new(1.0, 2.0, 3.0),
        };
        let moved = turned().moved(placement);
        assert!(moved.origin.distance(placement.transform_point(turned().origin)) < 1.0e-5);
        let expected = placement.rotation * turned().rotation;
        assert!(moved.rotation.angle_between(expected) < 1.0e-5);
        // A plane through the frame still passes through the moved body's
        // point that was on it.
        let on_plane = turned().origin + turned().rotation * Vec3::new(0.0, 3.0, -1.0);
        let local = moved.to_local(placement.transform_point(on_plane));
        assert!(local.x.abs() < 1.0e-4, "the point left the X plane: {local}");
    }

    /// A whole file keeps each body's own symmetry, settings and frame, and
    /// a file whose bodies have none has no section for it.
    #[test]
    fn a_file_keeps_each_bodys_symmetry() {
        let mut doc = Document::new(0.5);
        let first = doc.active();
        let second = doc.add_body("Second", crate::Volume::new(0.5));
        let mirrored = Symmetry::X.with_axis(MirrorAxis::Z, true).in_frame(turned());
        let radial = Symmetry::OFF.with_radial(6, MirrorAxis::Z);
        doc.put_symmetry(first, mirrored);
        doc.put_symmetry(second, radial);
        let mut bytes = Vec::new();
        crate::project::write(&mut bytes, &doc, &Default::default()).expect("written");
        let (back, _) = crate::project::read(&mut bytes.as_slice()).expect("read");
        let ids: Vec<NodeId> = back.nodes().iter().map(|node| node.id).collect();
        let read = back.symmetry(ids[0]);
        assert!(read.axis(MirrorAxis::X) && read.axis(MirrorAxis::Z) && !read.axis(MirrorAxis::Y));
        assert_eq!(read.radial(), 1);
        assert!(read.frame().origin.distance(turned().origin) < 1.0e-6, "{read:?}");
        assert!(read.frame().rotation.angle_between(turned().rotation) < 1.0e-5);
        let read = back.symmetry(ids[1]);
        assert_eq!((read.radial(), read.radial_axis()), (6, MirrorAxis::Z));
        assert!(!read.mirrors_any());

        let plain = Document::new(0.5);
        let mut bytes = Vec::new();
        crate::project::write(&mut bytes, &plain, &Default::default()).expect("written");
        assert!(!bytes.windows(TAG2.len()).any(|window| window == TAG2), "a section for nothing");
        let (back, _) = crate::project::read(&mut bytes.as_slice()).expect("read");
        assert!(back.symmetry(back.active()).is_off());
    }

    /// A file from before each body had its own settings still reads: its
    /// frames onto their bodies and its radial copies into the state, for
    /// the application to lay on every body.
    #[test]
    fn the_replaced_section_is_still_read() {
        let mut doc = Document::new(0.5);
        let body = doc.active();
        let mut bytes = Vec::new();
        crate::project::write(&mut bytes, &doc, &Default::default()).expect("written");
        doc.set_symmetry_frame(body, turned());
        let section =
            encode(&doc, Radial { copies: 5, axis: MirrorAxis::X }).expect("frames and copies");
        bytes.extend_from_slice(TAG);
        bytes.extend_from_slice(&(section.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&section);
        let (back, state) = crate::project::read(&mut bytes.as_slice()).expect("read");
        assert_eq!(state.radial, Radial { copies: 5, axis: MirrorAxis::X });
        let frame = back.symmetry_frame(back.active());
        assert!(frame.origin.distance(turned().origin) < 1.0e-6, "{frame:?}");
    }

    /// Settings set on a body keep its frame, are undone and redone without
    /// moving it, are refused on a folder, and a body set back to off in the
    /// room's frame holds nothing.
    #[test]
    fn settings_keep_the_frame_and_off_holds_nothing() {
        let mut doc = Document::new(0.5);
        let body = doc.active();
        doc.set_symmetry_frame(body, turned());
        let mut history = crate::undo::History::new(crate::DEFAULT_HISTORY_BUDGET);
        let change = doc.set_symmetry_settings(body, Symmetry::X).expect("a change");
        history.push(crate::undo::Entry::new(vec![change]));
        assert!(doc.symmetry(body).axis(MirrorAxis::X));
        assert_eq!(doc.symmetry_frame(body), turned(), "the settings moved the frame");
        assert!(doc.set_symmetry_settings(body, Symmetry::X).is_none(), "nothing changed");
        let visible = vec![true; doc.node_count()];
        let _ = history.undo(&mut doc, &visible);
        assert!(doc.symmetry(body).is_off(), "the undo left the plane on");
        assert_eq!(doc.symmetry_frame(body), turned(), "the undo moved the frame");
        let _ = history.redo(&mut doc, &visible);
        assert!(doc.symmetry(body).axis(MirrorAxis::X), "the redo did not put it back");
        let (folder, _) = doc.group(body, "Folder").expect("room for a folder");
        assert!(doc.set_symmetry_settings(folder, Symmetry::X).is_none(), "a folder took settings");
        doc.set_symmetry_settings(body, Symmetry::OFF);
        doc.set_symmetry_frame(body, SymmetryFrame::IDENTITY);
        assert!(doc.frames.frames.is_empty());
    }

    /// The section round trips the radial copies and every frame, says
    /// nothing for a document with neither, and leaves a frame that cannot
    /// be one the room's.
    #[test]
    fn the_section_round_trips_and_is_absent_when_there_is_nothing() {
        let mut doc = Document::new(0.5);
        let body = doc.active();
        assert!(encode(&doc, Radial::default()).is_none());
        let radial = Radial { copies: 6, axis: MirrorAxis::Z };
        let bytes = encode(&doc, radial).expect("the copies are on");
        assert_eq!(decode(&bytes, 1), (radial, Vec::new()));
        doc.set_symmetry_frame(body, turned());
        let bytes = encode(&doc, Radial::default()).expect("a frame is set");
        let (back, frames) = decode(&bytes, 1);
        assert_eq!(back, Radial::default());
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, 0);
        assert!(frames[0].1.origin.distance(turned().origin) < 1.0e-6);
        assert!(frames[0].1.rotation.angle_between(turned().rotation) < 1.0e-5);
        // Out of the table, and not a unit turn: both left out.
        assert!(decode(&bytes, 0).1.is_empty());
        let mut broken = bytes.clone();
        let w = 2 + 4 + 4 + 6 * 4;
        broken[w..w + 4].copy_from_slice(&3.0f32.to_le_bytes());
        assert!(decode(&broken, 1).1.is_empty());
        // Cut short: what is whole is read, the rest is not.
        assert!(decode(&bytes[..bytes.len() - 1], 1).1.is_empty());
        assert_eq!(decode(&[], 1), (Radial::default(), Vec::new()));
    }
}
