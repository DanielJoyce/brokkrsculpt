// SPDX-License-Identifier: AGPL-3.0-only

//! A project written a piece at a time.
//!
//! [`super::write`] used to be one call that did not return until the whole
//! document was out, and the application called it from the thread that
//! draws: its own figure for an autosave of a large scan was 430 ms, which is
//! twenty six frames gone every two minutes. A document cannot be handed to
//! another thread to write, because its bricks are owned and not shared, and
//! a copy made to hand over costs what the write did.
//!
//! So the write is cut up. [`Pieces`] puts out the same bytes [`super::write`]
//! does, a few megabytes a call, into a buffer. The thread that draws makes a
//! piece between two frames and hands the buffer to a thread that writes, and
//! no frame waits for more than the copying of one piece.
//!
//! **The document must not change between the first piece and the last.**
//! Nothing here can check that: the caller abandons the write if it does.
//! [`super::write`] is this, run to its end in one call, so there is one
//! encoder and the two cannot come to disagree.
//!
//! A dense brick goes out as one copy of its bytes where the machine is
//! little endian, which is what the format is. It used to go out a value at a
//! time through a buffered writer whatever the machine.

use super::{
    BrickCoord, ColourBrick, Document, FIELD_VERSION_WITH_COLOUR, FIELD_VERSION_WITH_MASK,
    MASK_FLAG_INVERTED, MaskBrick, ProjectState, Result, TAG_COLOUR_DENSE, TAG_COLOUR_UNIFORM,
    TAG_DENSE, TAG_MASK_DENSE, TAG_MASK_UNIFORM, TAG_UNIFORM, Volume,
};
use crate::brick::Brick;

/// About how many bytes [`super::write`] makes at a time.
pub(super) const WHOLE_PIECE: usize = 4 << 20;

/// Which of a body's three streams is going out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stream {
    Bricks,
    Mask,
    Colour,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Head,
    /// A stream of the body at `Pieces::body`. `opened` once its count is
    /// out and its coordinates are sorted.
    Body {
        stream: Stream,
        opened: bool,
    },
    Tail,
    Done,
}

/// A write in progress.
#[derive(Debug)]
pub struct Pieces {
    field: u16,
    stage: Stage,
    /// Which body, counted along [`Document::bodies`].
    body: usize,
    /// The coordinates of the stream going out, sorted, and how many of them
    /// are out.
    coords: Vec<BrickCoord>,
    at: usize,
}

impl Pieces {
    /// Begin a write of `doc`. Refuses, before a byte is made, a document the
    /// reader would refuse.
    pub fn begin(doc: &Document) -> Result<Pieces> {
        super::refuse_what_could_not_be_read_back(doc)?;
        Ok(Pieces {
            field: super::lowest_field_version(doc),
            stage: Stage::Head,
            body: 0,
            coords: Vec::new(),
            at: 0,
        })
    }

    /// Whether every byte has been made.
    pub fn done(&self) -> bool {
        self.stage == Stage::Done
    }

    /// Add the next piece to `out`: whole records until `out` holds at least
    /// `budget` bytes, or the end. Returns whether that was the end.
    ///
    /// `doc` and `state` must be the ones every earlier piece was made from,
    /// unchanged.
    pub fn next(
        &mut self,
        out: &mut Vec<u8>,
        doc: &Document,
        state: &ProjectState,
        budget: usize,
    ) -> Result<bool> {
        while out.len() < budget.max(1) {
            match self.stage {
                Stage::Head => {
                    super::write_head(out, doc, state, self.field)?;
                    self.stage = Stage::Body { stream: Stream::Bricks, opened: false };
                }
                Stage::Body { stream, opened } => {
                    let Some((_, volume)) = doc.bodies().nth(self.body) else {
                        self.stage = Stage::Tail;
                        continue;
                    };
                    if !self.wanted(stream) {
                        self.after(stream);
                        continue;
                    }
                    if !opened {
                        self.open(out, volume, stream)?;
                        self.stage = Stage::Body { stream, opened: true };
                        continue;
                    }
                    let Some(coord) = self.coords.get(self.at).copied() else {
                        self.after(stream);
                        continue;
                    };
                    self.at += 1;
                    record(out, volume, stream, coord);
                }
                Stage::Tail => {
                    super::write_tail(out, doc, state)?;
                    self.stage = Stage::Done;
                }
                Stage::Done => break,
            }
        }
        Ok(self.done())
    }

    /// Whether this file's field version has the stream at all.
    fn wanted(&self, stream: Stream) -> bool {
        match stream {
            Stream::Bricks => true,
            Stream::Mask => self.field >= FIELD_VERSION_WITH_MASK,
            Stream::Colour => self.field >= FIELD_VERSION_WITH_COLOUR,
        }
    }

    /// What follows a stream: the body's next, or the next body's first.
    fn after(&mut self, stream: Stream) {
        self.coords.clear();
        self.at = 0;
        let next = match stream {
            Stream::Bricks => Stream::Mask,
            Stream::Mask => Stream::Colour,
            Stream::Colour => {
                self.body += 1;
                Stream::Bricks
            }
        };
        self.stage = Stage::Body { stream: next, opened: false };
    }

    /// A stream's heading: its flags where it has any, and how many records
    /// follow. The records' coordinates are sorted here, because the order a
    /// hash map gives them in is not the same twice and the bytes have to be.
    fn open(&mut self, out: &mut Vec<u8>, volume: &Volume, stream: Stream) -> Result<()> {
        self.coords.clear();
        self.at = 0;
        match stream {
            Stream::Bricks => self.coords.extend(volume.brick_coords()),
            Stream::Mask => {
                let mask = volume.mask();
                out.push(if mask.inverted() { MASK_FLAG_INVERTED } else { 0 });
                self.coords.extend(mask.brick_coords());
            }
            Stream::Colour => self.coords.extend(volume.colour().bricks().keys().copied()),
        }
        self.coords.sort_unstable();
        // The count is of the coordinates just taken, every one of which is a
        // key of the map it came from: a count that ran ahead of its records
        // would misparse the stream after this one.
        super::write_u64(out, self.coords.len() as u64)
    }
}

/// One record: where the brick is, what kind it is, and its values.
fn record(out: &mut Vec<u8>, volume: &Volume, stream: Stream, coord: BrickCoord) {
    for component in coord.0.to_array() {
        out.extend_from_slice(&component.to_le_bytes());
    }
    match stream {
        Stream::Bricks => match volume.brick(coord) {
            Some(Brick::Dense(values)) => {
                out.push(TAG_DENSE);
                distances(out, values.as_slice());
            }
            Some(Brick::Uniform(value)) => {
                out.push(TAG_UNIFORM);
                out.extend_from_slice(&value.to_le_bytes());
            }
            // Counted and then gone: only a document changed under the write
            // can do it, which the caller has promised not to. What an
            // absent brick reads as, so that the count still holds.
            None => {
                out.push(TAG_UNIFORM);
                out.extend_from_slice(&crate::brick::OUTSIDE.to_le_bytes());
            }
        },
        Stream::Mask => match volume.mask().brick(coord) {
            // One byte a voxel, so there is no byte order to be careful of.
            Some(MaskBrick::Dense(values)) => {
                out.push(TAG_MASK_DENSE);
                out.extend_from_slice(values.as_slice());
            }
            Some(MaskBrick::Uniform(value)) => out.extend_from_slice(&[TAG_MASK_UNIFORM, *value]),
            None => out.extend_from_slice(&[TAG_MASK_UNIFORM, 0]),
        },
        Stream::Colour => match volume.colour().bricks().get(&coord) {
            Some(ColourBrick::Dense(slots)) => {
                out.push(TAG_COLOUR_DENSE);
                out.extend_from_slice(slots.as_slice());
            }
            Some(ColourBrick::Uniform(slot)) => {
                out.extend_from_slice(&[TAG_COLOUR_UNIFORM, *slot]);
            }
            None => out.extend_from_slice(&[TAG_COLOUR_UNIFORM, 0]),
        },
    }
}

/// A dense brick's values, little endian: the file's order whatever the
/// machine's.
fn distances(out: &mut Vec<u8>, values: &[f32]) {
    if cfg!(target_endian = "little") {
        out.extend_from_slice(bytemuck::cast_slice(values));
    } else {
        out.reserve(values.len() * 4);
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        ColourField, MaskField, lowest_field_version, write, write_f32, write_head, write_tail,
        write_u64,
    };
    use super::*;
    use crate::brick::BRICK_DIM;
    use glam::{IVec3, Vec3};
    use proptest::prelude::*;
    use std::io::Write;

    /// Two bodies, one of them masked in part, so that the file has more
    /// than one stream with records in it.
    fn document() -> Document {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 14.0);
        let mut doc = Document::from_volume(volume);
        let mut other = Volume::new(0.5);
        other.seed_sphere(Vec3::new(30.0, 4.0, -3.0), 9.0);
        let mask = other.mask_mut();
        // A brick masked whole, which is kept as one value, and half of the
        // next, which is not.
        let origin = BrickCoord::new(1, 0, 0).origin();
        let side = BRICK_DIM as i32;
        for index in 0..side * side * side {
            let cell = IVec3::new(index % side, index / side % side, index / (side * side));
            mask.write(origin + cell, 160);
            if cell.x < side / 2 {
                mask.write(origin + IVec3::new(side, 0, 0) + cell, 90);
            }
        }
        mask.collapse();
        doc.add_body("another", other);
        doc
    }

    /// The whole file by the encoder there was before this one: a body's
    /// streams one after another, and a dense brick a value at a time. Kept
    /// here as what the pieces are held to, since [`write`] is now made of
    /// them and comparing the two would compare a thing with itself.
    fn whole(doc: &Document, state: &ProjectState) -> Vec<u8> {
        let mut out = Vec::new();
        let field = lowest_field_version(doc);
        write_head(&mut out, doc, state, field).unwrap();
        for (_, volume) in doc.bodies() {
            old_bricks(&mut out, volume).unwrap();
            if field >= FIELD_VERSION_WITH_MASK {
                old_mask(&mut out, volume.mask()).unwrap();
            }
            if field >= FIELD_VERSION_WITH_COLOUR {
                old_colour(&mut out, volume.colour()).unwrap();
            }
        }
        write_tail(&mut out, doc, state).unwrap();
        out
    }

    fn old_bricks(out: &mut impl Write, volume: &Volume) -> Result<()> {
        let mut coords: Vec<BrickCoord> = volume.brick_coords().collect();
        coords.sort_unstable();
        let pairs: Vec<(BrickCoord, &Brick)> = coords
            .into_iter()
            .filter_map(|coord| volume.brick(coord).map(|brick| (coord, brick)))
            .collect();

        write_u64(out, pairs.len() as u64)?;
        for (coord, brick) in pairs {
            for component in coord.0.to_array() {
                out.write_all(&component.to_le_bytes())?;
            }
            match brick {
                Brick::Uniform(value) => {
                    out.write_all(&[TAG_UNIFORM])?;
                    write_f32(out, *value)?;
                }
                Brick::Dense(values) => {
                    out.write_all(&[TAG_DENSE])?;
                    // One value at a time rather than a cast of the whole array:
                    // little endian has to be explicit, or a file written on one
                    // architecture is silently wrong on another.
                    for value in values.iter() {
                        write_f32(out, *value)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn old_mask(out: &mut impl Write, mask: &MaskField) -> Result<()> {
        let mut flags = 0u8;
        if mask.inverted() {
            flags |= MASK_FLAG_INVERTED;
        }
        out.write_all(&[flags])?;

        let mut coords: Vec<BrickCoord> = mask.brick_coords().collect();
        coords.sort_unstable();
        let pairs: Vec<(BrickCoord, &MaskBrick)> = coords
            .into_iter()
            .filter_map(|coord| mask.brick(coord).map(|brick| (coord, brick)))
            .collect();

        write_u64(out, pairs.len() as u64)?;
        for (coord, brick) in pairs {
            for component in coord.0.to_array() {
                out.write_all(&component.to_le_bytes())?;
            }
            match brick {
                MaskBrick::Uniform(value) => out.write_all(&[TAG_MASK_UNIFORM, *value])?,
                MaskBrick::Dense(values) => {
                    // One byte per voxel, so unlike the field there is no
                    // endianness to be explicit about and the array goes out whole.
                    out.write_all(&[TAG_MASK_DENSE])?;
                    out.write_all(values.as_slice())?;
                }
            }
        }
        Ok(())
    }

    fn old_colour(out: &mut impl Write, colour: &ColourField) -> Result<()> {
        let mut coords: Vec<BrickCoord> = colour.bricks().keys().copied().collect();
        coords.sort_unstable();

        write_u64(out, coords.len() as u64)?;
        for coord in coords {
            let brick = &colour.bricks()[&coord];
            for component in coord.0.to_array() {
                out.write_all(&component.to_le_bytes())?;
            }
            match brick {
                ColourBrick::Uniform(slot) => out.write_all(&[TAG_COLOUR_UNIFORM, *slot])?,
                ColourBrick::Dense(slots) => {
                    out.write_all(&[TAG_COLOUR_DENSE])?;
                    out.write_all(slots.as_slice())?;
                }
            }
        }
        Ok(())
    }

    /// And [`write`] is those bytes too.
    #[test]
    fn the_whole_write_is_what_it_was() {
        let doc = document();
        let state = ProjectState::default();
        let mut bytes = Vec::new();
        write(&mut bytes, &doc, &state).expect("a document that can be written");
        assert!(bytes == whole(&doc, &state), "the write in one call changed its bytes");
    }

    fn in_pieces(doc: &Document, state: &ProjectState, budget: usize) -> (Vec<u8>, usize) {
        let mut pieces = Pieces::begin(doc).expect("a document that can be written");
        let mut bytes = Vec::new();
        let mut piece = Vec::new();
        let mut count = 0;
        loop {
            piece.clear();
            let done = pieces.next(&mut piece, doc, state, budget).expect("a piece");
            bytes.extend_from_slice(&piece);
            count += 1;
            if done {
                assert!(pieces.done());
                break;
            }
            assert!(piece.len() >= budget.max(1), "a piece short of its budget before the end");
        }
        (bytes, count)
    }

    /// A dense brick's values are in the file little endian, value by value,
    /// which is what the reader takes them as.
    #[test]
    fn a_dense_brick_goes_out_value_by_value_little_endian() {
        let values = [1.5f32, -3.0, 0.25, f32::MIN_POSITIVE];
        let mut out = Vec::new();
        distances(&mut out, &values);
        let by_value: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
        assert_eq!(out, by_value);
    }

    #[test]
    fn what_is_written_in_pieces_reads_back() {
        let doc = document();
        let state = ProjectState::default();
        let (bytes, count) = in_pieces(&doc, &state, 64 * 1024);
        assert!(count > 8, "a document of megabytes went out in {count} pieces");
        let (read, _) = super::super::read(&mut bytes.as_slice()).expect("it reads back");
        assert_eq!(read.node_count(), doc.node_count());
        for ((_, written), (_, back)) in doc.bodies().zip(read.bodies()) {
            crate::testing::assert_same_field(written, back, "a body written in pieces");
            assert_eq!(written.mask().brick_coords().count(), back.mask().brick_coords().count());
        }
        let masked = read.bodies().nth(1).expect("the second body").1.mask();
        assert_eq!(masked.brick_coords().count(), 2, "the mask's two bricks came back");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// However it is cut, a write in pieces is the bytes of the write in
        /// one go.
        #[test]
        fn pieces_of_any_size_are_the_whole(budget in 0usize..600_000) {
            let doc = document();
            let state = ProjectState::default();
            let (bytes, _) = in_pieces(&doc, &state, budget);
            let same = bytes == whole(&doc, &state);
            prop_assert!(same, "pieces of {budget} bytes are not the whole");
        }
    }
}
