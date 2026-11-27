// SPDX-License-Identifier: AGPL-3.0-only

//! Symmetry that belongs to a body: each body's own frame for the mirror
//! planes and the radial axis, carried with it when the gizmo moves it.
//!
//! Asked for by the maintainer on 2026-11-28: "when you transform a node
//! with the gizmo the symmetry transforms with it (ie symmetry is local to
//! the object not global)". Which planes are on, and how many radial copies
//! round which axis, stay one setting for the session, as they were; where
//! the planes and the axis are is the body's. A body starts with the room's
//! frame, through the middle of the room and unturned, so nothing changes
//! for a body that has never been moved.
//!
//! Only frames that are not the room's are held, so a document whose bodies
//! were never moved holds none and saves no byte for them.
//!
//! The file keeps the frames and the radial copies in a section of its own,
//! after the boolean groups. An older build stops reading before it, as it
//! does before every section it does not know, and opens the file with every
//! frame the room's and the radial copies off.
//!
//! What `body.rs` holds for this: one field on [`Document`] and its value in
//! the two constructors. `undo.rs` has one change,
//! [`Change::SymmetryFrame`], and `project.rs` writes and reads the section.

use glam::{Quat, Vec3};
use rustc_hash::FxHashMap;

use crate::body::{Document, NodeId};
use crate::brush::{MirrorAxis, Symmetry, SymmetryFrame};
use crate::undo::Change;

/// The tag of the file's section that holds the frames.
pub const TAG: &[u8; 8] = b"HANSYMMS";

/// A row: its place in the table (u32 LE), the origin and the turn as seven
/// f32 LE.
const ROW_BYTES: usize = 4 + 7 * 4;

/// The most a section can hold: the radial copies and their axis, a count,
/// and a row for every node.
pub const MOST_SECTION_BYTES: u64 = 2 + 4 + ROW_BYTES as u64 * crate::body::MAX_NODES as u64;

/// The frames that are not the room's, by body.
#[derive(Debug, Clone, Default)]
pub struct Frames {
    frames: FxHashMap<NodeId, SymmetryFrame>,
}

/// The radial copies as a file keeps them: how many, the stroke's own among
/// them (one is off), and round which axis.
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
    /// A body's frame: the room's until the gizmo has moved it.
    pub fn symmetry_frame(&self, id: NodeId) -> SymmetryFrame {
        self.frames.frames.get(&id).copied().unwrap_or_default()
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
        let before = if frame == SymmetryFrame::IDENTITY {
            self.frames.frames.remove(&id)
        } else {
            self.frames.frames.insert(id, frame)
        };
        before.unwrap_or_default()
    }
}

/// The section for a document: the radial copies (a byte for the count and
/// one for the axis), then for each body whose frame is not the room's, its
/// place in the table and the frame. `None` where the radial copies are off
/// and every frame is the room's, so that such a document gains no byte.
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

    /// A whole file keeps a moved body's frame and the radial copies, and a
    /// file with neither reads back with the room's frame and no copies.
    #[test]
    fn a_file_keeps_the_frames_and_the_copies() {
        let mut doc = Document::new(0.5);
        let body = doc.active();
        let state = crate::project::ProjectState {
            radial: Radial { copies: 5, axis: MirrorAxis::X },
            ..Default::default()
        };
        doc.set_symmetry_frame(body, turned());
        let mut bytes = Vec::new();
        crate::project::write(&mut bytes, &doc, &state).expect("written");
        let (back, read) = crate::project::read(&mut bytes.as_slice()).expect("read");
        assert_eq!(read.radial, state.radial);
        let frame = back.symmetry_frame(back.active());
        assert!(frame.origin.distance(turned().origin) < 1.0e-6, "{frame:?}");

        let plain = Document::new(0.5);
        let mut bytes = Vec::new();
        crate::project::write(&mut bytes, &plain, &Default::default()).expect("written");
        assert!(!bytes.windows(TAG.len()).any(|window| window == TAG), "a section for nothing");
        let (back, read) = crate::project::read(&mut bytes.as_slice()).expect("read");
        assert_eq!(read.radial, Radial::default());
        assert_eq!(back.symmetry_frame(back.active()), SymmetryFrame::IDENTITY);
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
