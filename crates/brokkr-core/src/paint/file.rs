// SPDX-License-Identifier: AGPL-3.0-only

//! Paint in a project file: one optional section at the file's tail.
//!
//! The paint of every body ([`super::field`]), written after everything
//! the container already holds, behind a tag of its own, as the paint
//! table is and for the reason `project.rs` gives for that: a document
//! with no paint is written byte for byte as it always was, a reader from
//! before paint stops where it always stopped and opens the sculpt, and a
//! file from before paint has nothing here to find.
//!
//! # The bytes
//!
//! ```text
//! u32   bodies            every body of the document, in its own order
//! for each body:
//!   u32 bricks
//!   for each brick, sorted by coordinate:
//!     i32 x 3             the brick's coordinate
//!     u8                  0: one paint throughout, then its 4 bytes
//!                         1: runs, then u32 run count and that many runs
//!                         2: every voxel, then 131,072 bytes
//!     a run is u16 length (never 0) and the 4 bytes it repeats
//! ```
//!
//! All little endian. Bricks are sorted so that writing one document twice
//! gives the same bytes, which the project file's own tests require of
//! everything in it.
//!
//! **Runs, and not a general compressor.** A painted brick is mostly bare:
//! only the voxels of the surface hold anything, and a base coat is one
//! value over all of those. Runs take the common case down by a factor of
//! ten or more with a decoder that is a dozen lines and cannot be made to
//! write more than a brick, which matters for bytes that come from a file.
//! A brick that runs would make BIGGER (a gradient over every voxel) is
//! written whole.
//!
//! # Reading is strict, and the caller is forgiving
//!
//! [`decode`] refuses anything that is not exactly a section this writer
//! could have written. What a refusal costs is the paint and never the
//! sculpt: `project.rs` has read the geometry in full before it looks here.

use glam::IVec3;

use super::field::{BARE, PaintBrick, PaintField, Rgba, stored};
use crate::brick::{BRICK_VOXELS, BrickCoord};

/// What marks the section, after the paint table's own tail if there is
/// one. The digit is the layout's version: a different layout is a
/// different tag, which an older reader skips.
pub(crate) const TAG: &[u8; 8] = b"BRKRGBA1";

const UNIFORM: u8 = 0;
const RUNS: u8 = 1;
const WHOLE: u8 = 2;

/// Why a section could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintFileError {
    /// The bytes stop before the section does.
    Truncated,
    /// The section is for a different number of bodies than the document
    /// has.
    Bodies { section: u32, document: usize },
    /// A brick is marked as a kind there is not.
    UnknownKind(u8),
    /// A brick's runs do not add up to a brick, or one of them is empty.
    BadRuns,
    /// There are bytes left over after the last body.
    Trailing,
}

impl std::fmt::Display for PaintFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaintFileError::Truncated => f.write_str("the paint is cut short"),
            PaintFileError::Bodies { section, document } => {
                write!(f, "the paint is for {section} bodies and the file holds {document}")
            }
            PaintFileError::UnknownKind(kind) => {
                write!(f, "a paint brick of unknown kind ({kind})")
            }
            PaintFileError::BadRuns => f.write_str("a paint brick's runs do not make a brick"),
            PaintFileError::Trailing => f.write_str("there is something after the last paint"),
        }
    }
}

/// A brick's voxels as runs of equal paint, or `None` when that would be
/// no smaller than writing them all.
fn runs_of(data: &[Rgba; BRICK_VOXELS]) -> Option<Vec<u8>> {
    let whole = BRICK_VOXELS * 4;
    let mut out: Vec<u8> = Vec::new();
    let mut count = 0_u32;
    let mut at = 0;
    while at < BRICK_VOXELS {
        let value = data[at];
        let mut length = 1;
        // A run cannot outgrow its length: a brick is 32,768 voxels and a
        // length counts to 65,535.
        while at + length < BRICK_VOXELS && data[at + length] == value {
            length += 1;
        }
        out.extend_from_slice(&(length as u16).to_le_bytes());
        out.extend_from_slice(&value);
        count += 1;
        at += length;
        if out.len() + 4 >= whole {
            return None;
        }
    }
    let mut framed = Vec::with_capacity(out.len() + 4);
    framed.extend_from_slice(&count.to_le_bytes());
    framed.extend_from_slice(&out);
    Some(framed)
}

/// The section for these bodies' paint, or `None` when none of them has
/// any, which is a document that gains no bytes.
pub(crate) fn encode<'a>(bodies: impl Iterator<Item = &'a PaintField> + Clone) -> Option<Vec<u8>> {
    if bodies.clone().all(PaintField::is_empty) {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&(bodies.clone().count() as u32).to_le_bytes());
    for paint in bodies {
        let mut coords: Vec<BrickCoord> = paint.bricks().keys().copied().collect();
        coords.sort_unstable();
        out.extend_from_slice(&(coords.len() as u32).to_le_bytes());
        for coord in coords {
            for component in coord.0.to_array() {
                out.extend_from_slice(&component.to_le_bytes());
            }
            match &paint.bricks()[&coord] {
                PaintBrick::Uniform(rgba) => {
                    out.push(UNIFORM);
                    out.extend_from_slice(rgba);
                }
                PaintBrick::Dense(data) => match runs_of(data) {
                    Some(runs) => {
                        out.push(RUNS);
                        out.extend_from_slice(&runs);
                    }
                    None => {
                        out.push(WHOLE);
                        out.extend_from_slice(data.as_flattened());
                    }
                },
            }
        }
    }
    Some(out)
}

/// Read a section back into one field for each of `bodies` bodies.
pub(crate) fn decode(bytes: &[u8], bodies: usize) -> Result<Vec<PaintField>, PaintFileError> {
    let mut rest = bytes;
    let mut take = |count: usize| -> Result<&[u8], PaintFileError> {
        if rest.len() < count {
            return Err(PaintFileError::Truncated);
        }
        let (head, tail) = rest.split_at(count);
        rest = tail;
        Ok(head)
    };
    let word = |bytes: &[u8]| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let rgba = |bytes: &[u8]| stored([bytes[0], bytes[1], bytes[2], bytes[3]]);

    let claimed = word(take(4)?);
    if claimed as usize != bodies {
        return Err(PaintFileError::Bodies { section: claimed, document: bodies });
    }
    let mut fields = Vec::with_capacity(bodies);
    for _ in 0..bodies {
        let mut field = PaintField::default();
        let bricks = word(take(4)?);
        for _ in 0..bricks {
            let at = take(12)?;
            let component = |index: usize| {
                i32::from_le_bytes([at[index], at[index + 1], at[index + 2], at[index + 3]])
            };
            let coord = BrickCoord(IVec3::new(component(0), component(4), component(8)));
            let brick = match take(1)?[0] {
                UNIFORM => PaintBrick::Uniform(rgba(take(4)?)),
                RUNS => {
                    let runs = word(take(4)?);
                    let mut brick = PaintBrick::dense_filled(BARE);
                    let data = brick.make_dense();
                    let mut at = 0;
                    for _ in 0..runs {
                        let run = take(6)?;
                        let length = usize::from(u16::from_le_bytes([run[0], run[1]]));
                        if length == 0 || at + length > BRICK_VOXELS {
                            return Err(PaintFileError::BadRuns);
                        }
                        data[at..at + length].fill(rgba(&run[2..]));
                        at += length;
                    }
                    if at != BRICK_VOXELS {
                        return Err(PaintFileError::BadRuns);
                    }
                    brick
                }
                WHOLE => {
                    let raw = take(BRICK_VOXELS * 4)?;
                    let mut brick = PaintBrick::dense_filled(BARE);
                    for (voxel, bytes) in brick.make_dense().iter_mut().zip(raw.as_chunks::<4>().0)
                    {
                        *voxel = stored(*bytes);
                    }
                    brick
                }
                other => return Err(PaintFileError::UnknownKind(other)),
            };
            field.insert(coord, brick);
        }
        // A brick of one value written as runs or whole, or a brick of
        // bare, is stored the way an edit would have left it.
        field.collapse();
        fields.push(field);
    }
    if !rest.is_empty() {
        return Err(PaintFileError::Trailing);
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::Volume;
    use glam::Vec3;

    /// A ball with a flat coat on one side and a different paint in every
    /// voxel on the other.
    fn painted() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::ZERO, 40.0);
        volume.edit_paint(IVec3::splat(-50), IVec3::splat(50), |cell, at, _, _| {
            if at.x > 4.0 {
                [200, 30, 20, 255]
            } else if at.x < -4.0 {
                [(cell.x & 255) as u8, (cell.y & 255) as u8, (cell.z & 255) as u8, 200]
            } else {
                BARE
            }
        });
        volume
    }

    fn kinds_in(section: &[u8]) -> [usize; 3] {
        // Walk the section as the reader does, counting each kind.
        let word = |at: usize| u32::from_le_bytes(section[at..at + 4].try_into().expect("four"));
        let mut at = 4;
        let mut kinds = [0; 3];
        for _ in 0..word(0) {
            let bricks = word(at);
            at += 4;
            for _ in 0..bricks {
                let kind = section[at + 12];
                kinds[usize::from(kind)] += 1;
                at += 13
                    + match kind {
                        UNIFORM => 4,
                        RUNS => 4 + 6 * word(at + 13) as usize,
                        _ => BRICK_VOXELS * 4,
                    };
            }
        }
        assert_eq!(at, section.len());
        kinds
    }

    /// The paint of several bodies comes back as it went in, bare bodies
    /// and all, and a document with no paint writes no section.
    #[test]
    fn paint_comes_back_from_its_section_as_it_went_in() {
        let one = painted();
        let bare = Volume::new(1.0);
        let mut other = Volume::new(1.0);
        other.seed_sphere(Vec3::new(5.0, 0.0, 0.0), 9.0);
        other.edit_paint(IVec3::splat(-20), IVec3::splat(30), |_, _, _, _| [1, 2, 3, 4]);

        let bodies = [one.paint(), bare.paint(), other.paint()];
        let section = encode(bodies.iter().copied()).expect("there is paint to write");
        // Every brick of a ball is mostly bare, so every one is runs, even
        // where each painted voxel differs from the next: a brick written
        // whole is `runs_hold_a_brick_exactly`'s to make.
        let kinds = kinds_in(&section);
        assert!(kinds[1] > 20, "the fixture wrote kinds {kinds:?}");
        assert!(
            section.len() < kinds[1] * BRICK_VOXELS * 4 / 2,
            "{} bytes for {kinds:?}",
            section.len()
        );

        let back = decode(&section, 3).expect("a section just written");
        for (field, body) in back.iter().zip([&one, &bare, &other]) {
            assert_eq!(field.is_empty(), body.paint().is_empty());
            let reach = 50;
            for z in (-reach..=reach).step_by(3) {
                for y in (-reach..=reach).step_by(3) {
                    for x in -reach..=reach {
                        let cell = IVec3::new(x, y, z);
                        assert_eq!(field.at(cell), body.paint().at(cell), "{cell}");
                    }
                }
            }
        }
        // The same bytes a second time, and from a field that holds the
        // same paint but was built in another order: a map's own order is
        // not the file's.
        assert_eq!(encode(bodies.iter().copied()).expect("paint"), section);
        let mut coords: Vec<BrickCoord> = one.paint().bricks().keys().copied().collect();
        coords.sort_unstable_by_key(|coord| std::cmp::Reverse(*coord));
        let mut rebuilt = PaintField::default();
        for coord in coords.iter().chain(coords.iter().rev()) {
            rebuilt.insert(*coord, one.paint().bricks()[coord].clone());
        }
        assert_eq!(
            encode(std::iter::once(&rebuilt)),
            encode(std::iter::once(one.paint())),
            "the bytes depend on the order the bricks were made in"
        );

        assert!(encode([bare.paint(), bare.paint()].into_iter()).is_none());
        assert!(encode(std::iter::empty()).is_none());
    }

    /// A brick written as runs is the brick, whatever its runs are: one
    /// value, which is one run since a length counts further than a brick
    /// is long, and a change at the very first voxel and the very last.
    #[test]
    fn runs_hold_a_brick_exactly() {
        let mut brick = PaintBrick::dense_filled([9, 8, 7, 6]);
        let data = brick.make_dense();
        // One value over 32,768 voxels is one run: a length counts to 65,535.
        let runs = runs_of(data).expect("one value is one run");
        assert_eq!(runs.len(), 4 + 6);
        data[BRICK_VOXELS - 1] = [1, 1, 1, 1];
        data[0] = [2, 2, 2, 2];
        let runs = runs_of(data).expect("three runs");
        assert_eq!(u32::from_le_bytes(runs[..4].try_into().expect("four")), 3);

        let mut field = PaintField::default();
        field.insert(BrickCoord::new(-3, 0, 7), brick.clone());
        let section = encode(std::iter::once(&field)).expect("paint");
        let back = decode(&section, 1).expect("a section just written");
        assert_eq!(back[0].brick(BrickCoord::new(-3, 0, 7)), Some(&brick));

        // A brick with a different paint in every voxel is written whole:
        // runs would be half as big again.
        for (index, voxel) in brick.make_dense().iter_mut().enumerate() {
            *voxel = [(index & 255) as u8, ((index >> 8) & 255) as u8, 3, 255];
        }
        assert!(runs_of(brick.make_dense()).is_none());
        let mut field = PaintField::default();
        field.insert(BrickCoord::new(0, 0, 0), brick.clone());
        let section = encode(std::iter::once(&field)).expect("paint");
        assert_eq!(section.len(), 4 + 4 + 13 + BRICK_VOXELS * 4);
        assert_eq!(
            decode(&section, 1).expect("whole")[0].brick(BrickCoord::new(0, 0, 0)),
            Some(&brick)
        );
    }

    /// Each way a section can be wrong is refused by name, and a section
    /// for another number of bodies is not read into this one.
    #[test]
    fn each_way_a_section_can_be_wrong_is_named() {
        let volume = painted();
        let section = encode(std::iter::once(volume.paint())).expect("paint");
        assert_eq!(
            decode(&section, 2).err(),
            Some(PaintFileError::Bodies { section: 1, document: 2 })
        );
        assert_eq!(decode(&section[..section.len() - 1], 1).err(), Some(PaintFileError::Truncated));
        assert_eq!(decode(&[], 1).err(), Some(PaintFileError::Truncated));
        let mut longer = section.clone();
        longer.push(0);
        assert_eq!(decode(&longer, 1).err(), Some(PaintFileError::Trailing));

        // One brick, by hand: its kind, and its runs.
        let brick = |kind: u8, body: &[u8]| {
            let mut bytes = vec![1, 0, 0, 0, 1, 0, 0, 0];
            bytes.extend_from_slice(&[0; 12]);
            bytes.push(kind);
            bytes.extend_from_slice(body);
            bytes
        };
        assert_eq!(decode(&brick(9, &[]), 1).err(), Some(PaintFileError::UnknownKind(9)));
        let run = |length: u16| {
            let mut bytes = length.to_le_bytes().to_vec();
            bytes.extend_from_slice(&[5, 5, 5, 5]);
            bytes
        };
        let runs = |lengths: &[u16]| {
            let mut bytes = (lengths.len() as u32).to_le_bytes().to_vec();
            for length in lengths {
                bytes.extend_from_slice(&run(*length));
            }
            brick(RUNS, &bytes)
        };
        assert!(decode(&runs(&[32768]), 1).is_ok(), "one run of a whole brick");
        // A brick written whole, with a voxel in it that has a colour and
        // no coverage: read as bare, as an edit would have stored it.
        let mut whole = Vec::with_capacity(BRICK_VOXELS * 4);
        for index in 0..BRICK_VOXELS {
            whole.extend_from_slice(&[(index & 255) as u8, 1, 2, 255]);
        }
        whole[..4].copy_from_slice(&[9, 9, 9, 0]);
        let fields = decode(&brick(WHOLE, &whole), 1).expect("a whole brick");
        assert_eq!(fields[0].at(IVec3::ZERO), BARE, "a colour with no coverage was kept");
        assert_eq!(fields[0].at(IVec3::X), [1, 1, 2, 255]);
        assert_eq!(
            decode(&runs(&[32767]), 1).err(),
            Some(PaintFileError::BadRuns),
            "a voxel short"
        );
        assert_eq!(
            decode(&runs(&[32768, 1]), 1).err(),
            Some(PaintFileError::BadRuns),
            "a voxel over"
        );
        assert_eq!(
            decode(&runs(&[0, 32768]), 1).err(),
            Some(PaintFileError::BadRuns),
            "an empty run"
        );
        assert_eq!(
            decode(&runs(&[65535]), 1).err(),
            Some(PaintFileError::BadRuns),
            "a run past the end"
        );
        // Paint with no coverage in a file is bare, as it is everywhere.
        let mut hollow = (1_u32).to_le_bytes().to_vec();
        hollow.extend_from_slice(&32768_u16.to_le_bytes());
        hollow.extend_from_slice(&[9, 9, 9, 0]);
        let read = decode(&brick(RUNS, &hollow), 1).expect("a brick of no coverage");
        assert!(read[0].is_empty(), "paint with no coverage was kept");
        for error in [
            PaintFileError::Truncated,
            PaintFileError::Bodies { section: 1, document: 2 },
            PaintFileError::UnknownKind(9),
            PaintFileError::BadRuns,
            PaintFileError::Trailing,
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    /// A painted document saved and opened has its paint, body for body.
    /// One with no paint is the file it always was, to the byte. And with
    /// a paint table as well, both tails come back.
    #[test]
    fn a_documents_paint_is_saved_with_it_and_costs_one_without_it_nothing() {
        use crate::body::Document;
        use crate::project::{ProjectState, read, write};

        let saved = |doc: &Document, state: &ProjectState| {
            let mut bytes = Vec::new();
            write(&mut bytes, doc, state).expect("writing to memory");
            bytes
        };
        let ends_with_paint = |bytes: &[u8], section: usize| {
            let at = bytes.len() - section - 16;
            &bytes[at..at + 8] == TAG
                && u64::from_le_bytes(bytes[at + 8..at + 16].try_into().expect("eight"))
                    == section as u64
        };

        // No paint: nothing of this in the file.
        let mut bare = Document::from_volume(painted());
        *bare.volume_mut(bare.active()).expect("the body").paint_mut() = PaintField::default();
        let plain = saved(&bare, &ProjectState::default());
        assert!(!plain.windows(8).any(|window| window == TAG), "an unpainted file names paint");

        // Two bodies, the second bare.
        let mut doc = Document::from_volume(painted());
        let first = doc.active();
        let mut ball = Volume::new(1.0);
        ball.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 9.0);
        let second = doc.add_body("Bare", ball);
        let bytes = saved(&doc, &ProjectState::default());
        let section =
            encode(doc.nodes().iter().filter_map(|node| node.volume()).map(Volume::paint))
                .expect("the first body is painted");
        assert!(ends_with_paint(&bytes, section.len()), "the paint is not the file's tail");
        // Everything before the tail is the file with no paint in it.
        let mut unpainted = Document::from_volume(painted());
        *unpainted.volume_mut(unpainted.active()).expect("the body").paint_mut() =
            PaintField::default();
        let mut ball = Volume::new(1.0);
        ball.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 9.0);
        unpainted.add_body("Bare", ball);
        assert_eq!(
            bytes[..bytes.len() - section.len() - 16],
            saved(&unpainted, &ProjectState::default())[..],
            "painting a body changed the file before its tail"
        );

        // A folder is a node with no body, and the paint is counted by
        // bodies: one beside the painted body costs it nothing.
        let mut grouped = Document::from_volume(painted());
        let inside = grouped.active();
        grouped.group(inside, "Group 1").expect("the group");
        let foldered = saved(&grouped, &ProjectState::default());
        let (back, _) = read(&mut &foldered[..]).expect("a file just written");
        assert!(back.nodes().len() > 1, "the fixture has no folder");
        let body = back.nodes().iter().find_map(|node| node.volume()).expect("the body");
        assert!(!body.paint().is_empty(), "a folder beside the body cost it its paint");

        let (back, _) = read(&mut &bytes[..]).expect("a file just written");
        let same = |one: &Volume, other: &Volume| {
            for z in (-50..=50).step_by(5) {
                for y in (-50..=50).step_by(5) {
                    for x in -50..=70 {
                        let cell = IVec3::new(x, y, z);
                        assert_eq!(one.paint().at(cell), other.paint().at(cell), "{cell}");
                    }
                }
            }
        };
        let ids: Vec<_> = back.nodes().iter().map(|node| node.id).collect();
        same(back.volume(ids[0]).expect("first"), doc.volume(first).expect("first"));
        assert!(!back.volume(ids[0]).expect("first").paint().is_empty());
        assert!(back.volume(ids[1]).expect("second").paint().is_empty());
        assert!(doc.volume(second).expect("second").paint().is_empty());

        // With a paint table too: its tail first, then this one, and both
        // read.
        let mut table = crate::paint::PaintTable::default();
        table.add(crate::paint::Paint::flat("bone", [222, 208, 170]));
        let state = ProjectState { paints: table.clone(), ..ProjectState::default() };
        let both = saved(&doc, &state);
        assert!(ends_with_paint(&both, section.len()));
        let (back, state_back) = read(&mut &both[..]).expect("a file just written");
        assert_eq!(state_back.paints, table);
        let first_back = back.nodes()[0].id;
        same(back.volume(first_back).expect("first"), doc.volume(first).expect("first"));
    }

    /// A damaged paint tail costs the paint and never the sculpt: cut
    /// short, its length wrong, or its bytes spoiled, the file opens with
    /// every body as it was sculpted and bare.
    #[test]
    fn a_damaged_paint_tail_costs_the_paint_and_never_the_sculpt() {
        use crate::body::Document;
        use crate::project::{ProjectState, read, write};

        let doc = Document::from_volume(painted());
        let mut bytes = Vec::new();
        write(&mut bytes, &doc, &ProjectState::default()).expect("writing to memory");
        let section = encode(std::iter::once(doc.volume(doc.active()).expect("the body").paint()))
            .expect("paint")
            .len();
        let tail = bytes.len() - section - 16;
        let opens_bare = |bytes: &[u8], what: &str| {
            let (back, _) = read(&mut &bytes[..]).unwrap_or_else(|why| panic!("{what}: {why}"));
            let volume = back.volume(back.active()).expect("the body");
            assert!(volume.paint().is_empty(), "{what}: paint was read anyway");
            assert_eq!(volume.sample_voxel(IVec3::new(40, 0, 0)), {
                let original = doc.volume(doc.active()).expect("the body");
                original.sample_voxel(IVec3::new(40, 0, 0))
            });
        };
        opens_bare(&bytes[..bytes.len() - 5], "cut short");
        opens_bare(&bytes[..tail + 12], "cut in its length");
        opens_bare(&bytes[..tail + 3], "cut in its tag");
        let mut wrong = bytes.clone();
        wrong[tail + 8..tail + 16].copy_from_slice(&u64::MAX.to_le_bytes());
        opens_bare(&wrong, "a length no file has");
        let mut wrong = bytes.clone();
        wrong[tail + 8..tail + 16].copy_from_slice(&(section as u64 + 9).to_le_bytes());
        opens_bare(&wrong, "a length past the end of the file");
        let mut wrong = bytes.clone();
        wrong[tail + 16] = 7;
        opens_bare(&wrong, "a section for seven bodies");
        let mut wrong = bytes.clone();
        wrong[tail] = b'X';
        opens_bare(&wrong, "a tag that is not this one");

        // Control: the file as written does open with its paint.
        let (back, _) = read(&mut &bytes[..]).expect("the file as written");
        assert!(!back.volume(back.active()).expect("the body").paint().is_empty());
    }

    /// The section is bytes from a file, so anything may be in it. Whatever
    /// is, reading it does not panic, and what is read is paint.
    #[test]
    fn a_damaged_section_gets_an_answer_and_never_a_panic() {
        let mut small = Volume::new(1.0);
        small.seed_sphere(Vec3::ZERO, 10.0);
        small.edit_paint(IVec3::splat(-14), IVec3::splat(14), |cell, at, _, _| {
            if at.y > 0.0 { [200, 30, 20, 255] } else { [(cell.x & 255) as u8, 7, 7, 99] }
        });
        let good = encode(std::iter::once(small.paint())).expect("paint");
        // The head of the section, where the counts and kinds are, takes
        // most of the damage: a flipped byte in the middle of a whole brick
        // is only another colour.
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move |below: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % below as u64) as usize
        };
        let (mut read, mut refused) = (0, 0);
        for round in 0..3000 {
            let mut bytes = good.clone();
            for _ in 0..1 + next(3) {
                let at =
                    if round % 2 == 0 { next(bytes.len().min(400)) } else { next(bytes.len()) };
                match next(4) {
                    0 => bytes[at] = next(256) as u8,
                    1 => bytes[at] ^= 1 << next(8),
                    2 => bytes.truncate(at),
                    _ => bytes.insert(at, next(256) as u8),
                }
                if bytes.is_empty() {
                    break;
                }
            }
            match decode(&bytes, 1) {
                Ok(fields) => {
                    assert_eq!(fields.len(), 1);
                    read += 1;
                }
                Err(_) => refused += 1,
            }
        }
        // Controls: some damage was survivable (it changed a colour) and
        // some was not, so both branches of the reader were walked.
        assert!(read > 200, "only {read} of 3000 damaged sections were read");
        assert!(refused > 200, "only {refused} of 3000 damaged sections were refused");
    }
}
