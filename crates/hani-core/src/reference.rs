// SPDX-License-Identifier: AGPL-3.0-only

//! Reference pictures on the three axis planes, and the floor: what a
//! sculpt is modelled against, kept with it in the `.hani` file.
//!
//! A picture is held as the bytes of the file it came from (PNG or JPEG),
//! never decoded here: `hani-core` knows nothing of pictures, and the file
//! keeps what was taken in rather than a larger raw copy. The application
//! decodes it to draw it.
//!
//! # In the file
//!
//! An optional tagged section after the paint layers, [`TAG`], present only
//! when there is a picture or the floor is on, so a document without either
//! is written byte for byte as before. Read on the paint table's terms: a
//! section cut short, too long or not understood leaves the document with no
//! references and the sculpt as it was read.

use std::sync::Arc;

/// What marks the section.
pub const TAG: &[u8; 8] = b"HANREFER";

/// The section's own layout version.
const VERSION: u8 = 1;

/// The most bytes one picture may be, in the file and as taken in.
///
/// A ceiling against a damaged length rather than a limit anyone should
/// meet: a 4096 pixel square JPEG is a few megabytes.
pub const MAX_PICTURE_BYTES: usize = 64 * 1024 * 1024;

/// The most bytes a picture's name may be.
const MAX_NAME_BYTES: usize = 255;

/// One of the three planes a picture can stand on, named for the view that
/// looks straight at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plane {
    /// Faces +Z, the Front view: the picture spans X across and Y up.
    Front,
    /// Faces +X, the Right view: the picture spans -Z across and Y up.
    Side,
    /// Faces +Y, the Top view: the picture spans X across and -Z up.
    Top,
}

impl Plane {
    pub const ALL: [Plane; 3] = [Plane::Front, Plane::Side, Plane::Top];

    pub fn index(self) -> usize {
        match self {
            Plane::Front => 0,
            Plane::Side => 1,
            Plane::Top => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Plane::Front => "Front",
            Plane::Side => "Side",
            Plane::Top => "Top",
        }
    }

    /// The axis the plane faces along, as an index into a vector.
    pub fn normal_axis(self) -> usize {
        match self {
            Plane::Front => 2,
            Plane::Side => 0,
            Plane::Top => 1,
        }
    }

    /// The plane's normal, out towards the view that faces it.
    pub fn normal(self) -> glam::Vec3 {
        match self {
            Plane::Front => glam::Vec3::Z,
            Plane::Side => glam::Vec3::X,
            Plane::Top => glam::Vec3::Y,
        }
    }

    /// The world directions of the picture's rightward and upward edges, as
    /// seen from the view that faces the plane.
    pub fn across_and_up(self) -> (glam::Vec3, glam::Vec3) {
        match self {
            Plane::Front => (glam::Vec3::X, glam::Vec3::Y),
            Plane::Side => (glam::Vec3::NEG_Z, glam::Vec3::Y),
            Plane::Top => (glam::Vec3::X, glam::Vec3::NEG_Z),
        }
    }

    fn from_byte(byte: u8) -> Option<Plane> {
        Plane::ALL.get(usize::from(byte)).copied()
    }
}

/// One picture on one plane.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    /// The file's name, without its folder, to show in the panel.
    pub name: String,
    /// The file's bytes, PNG or JPEG, as they were taken in.
    pub picture: Arc<[u8]>,
    /// Where the picture's middle is on its plane, in millimetres along the
    /// plane's across and up directions ([`Plane::across_and_up`]).
    pub centre: [f32; 2],
    /// Where the plane stands along its normal, in millimetres.
    pub depth: f32,
    /// How wide the picture is drawn, in millimetres. Its height follows from
    /// its own proportions.
    pub width: f32,
    /// 0 is invisible, 1 is opaque.
    pub opacity: f32,
    pub shown: bool,
    /// Drawn mirrored left to right.
    pub flipped: bool,
}

impl Reference {
    /// A picture as first taken in: in the middle, at the depth and width
    /// given, half see-through.
    pub fn new(name: String, picture: Arc<[u8]>, depth: f32, width: f32) -> Self {
        Self {
            name,
            picture,
            centre: [0.0, 0.0],
            depth,
            width,
            opacity: 0.5,
            shown: true,
            flipped: false,
        }
    }
}

/// Every picture, one a plane at most, and whether the floor is drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct References {
    pub planes: [Option<Reference>; 3],
    pub floor: bool,
}

impl References {
    pub fn on(&self, plane: Plane) -> Option<&Reference> {
        self.planes[plane.index()].as_ref()
    }

    pub fn on_mut(&mut self, plane: Plane) -> Option<&mut Reference> {
        self.planes[plane.index()].as_mut()
    }

    /// Nothing to write: no picture and no floor.
    pub fn is_empty(&self) -> bool {
        !self.floor && self.planes.iter().all(Option::is_none)
    }

    /// The section's bytes, or `None` when there is nothing to write.
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.is_empty() {
            return None;
        }
        let mut out = vec![VERSION, u8::from(self.floor)];
        let held: Vec<(Plane, &Reference)> = Plane::ALL
            .into_iter()
            .filter_map(|plane| self.on(plane).map(|reference| (plane, reference)))
            .collect();
        out.push(held.len() as u8);
        for (plane, reference) in held {
            out.push(plane.index() as u8);
            out.push(u8::from(reference.shown) | (u8::from(reference.flipped) << 1));
            for value in [
                reference.centre[0],
                reference.centre[1],
                reference.depth,
                reference.width,
                reference.opacity,
            ] {
                out.extend_from_slice(&value.to_le_bytes());
            }
            let name = name_that_fits(&reference.name);
            out.push(name.len() as u8);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(reference.picture.len() as u32).to_le_bytes());
            out.extend_from_slice(&reference.picture);
        }
        Some(out)
    }

    /// Read a section's bytes back, or `None` when they are not one.
    pub fn decode(bytes: &[u8]) -> Option<References> {
        let mut at = Cursor { bytes, at: 0 };
        if at.byte()? != VERSION {
            return None;
        }
        let mut references = References { floor: at.byte()? & 1 != 0, ..References::default() };
        let count = at.byte()?;
        if usize::from(count) > Plane::ALL.len() {
            return None;
        }
        for _ in 0..count {
            let plane = Plane::from_byte(at.byte()?)?;
            let flags = at.byte()?;
            let mut values = [0.0f32; 5];
            for value in &mut values {
                *value = at.f32()?;
            }
            let [across, up, depth, width, opacity] = values;
            let name_length = usize::from(at.byte()?);
            let name = String::from_utf8_lossy(at.take(name_length)?).into_owned();
            let picture_length = at.u32()? as usize;
            if picture_length > MAX_PICTURE_BYTES {
                return None;
            }
            let picture: Arc<[u8]> = Arc::from(at.take(picture_length)?);
            // Repaired rather than refused, as a view is: a number nobody
            // would miss is no reason to lose the picture.
            let finite =
                |value: f32, otherwise: f32| if value.is_finite() { value } else { otherwise };
            references.planes[plane.index()] = Some(Reference {
                name,
                picture,
                centre: [finite(across, 0.0), finite(up, 0.0)],
                depth: finite(depth, 0.0),
                width: finite(width, 100.0).max(0.01),
                opacity: finite(opacity, 0.5).clamp(0.0, 1.0),
                shown: flags & 1 != 0,
                flipped: flags & 2 != 0,
            });
        }
        (at.at == bytes.len()).then_some(references)
    }
}

/// A name cut to [`MAX_NAME_BYTES`] on a character boundary.
fn name_that_fits(name: &str) -> &str {
    let mut end = name.len().min(MAX_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(count)?;
        let taken = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(taken)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take(1).map(|taken| taken[0])
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|taken| u32::from_le_bytes(taken.try_into().expect("four bytes")))
    }

    fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn a_reference(seed: u8) -> Reference {
        Reference {
            name: format!("front-{seed}.png"),
            picture: Arc::from(vec![seed; 37 + usize::from(seed)]),
            centre: [1.5, -2.25],
            depth: -40.0,
            width: 120.0,
            opacity: 0.35,
            shown: seed.is_multiple_of(2),
            flipped: seed.is_multiple_of(3),
        }
    }

    #[test]
    fn nothing_to_say_writes_no_section() {
        assert_eq!(References::default().encode(), None);
    }

    #[test]
    fn the_floor_alone_is_a_section() {
        let floor = References { floor: true, ..References::default() };
        let bytes = floor.encode().expect("the floor on is something to keep");
        assert_eq!(References::decode(&bytes), Some(floor));
    }

    #[test]
    fn every_plane_comes_back_as_it_went() {
        let references = References {
            planes: [Some(a_reference(1)), Some(a_reference(2)), Some(a_reference(3))],
            floor: true,
        };
        let bytes = references.encode().unwrap();
        assert_eq!(References::decode(&bytes), Some(references));
    }

    #[test]
    fn a_section_cut_short_or_running_on_is_not_taken() {
        let references = References { planes: [None, Some(a_reference(4)), None], floor: false };
        let bytes = references.encode().unwrap();
        for end in 0..bytes.len() {
            assert_eq!(References::decode(&bytes[..end]), None, "cut at {end} was taken");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(References::decode(&longer), None);
    }

    #[test]
    fn nonsense_numbers_are_repaired_rather_than_refused() {
        let mut reference = a_reference(5);
        reference.depth = f32::NAN;
        reference.opacity = 7.0;
        reference.width = -3.0;
        let references = References { planes: [Some(reference), None, None], floor: false };
        let back = References::decode(&references.encode().unwrap()).unwrap();
        let back = back.on(Plane::Front).unwrap();
        assert_eq!(back.depth, 0.0);
        assert_eq!(back.opacity, 1.0);
        assert!(back.width > 0.0);
    }

    #[test]
    fn a_long_name_is_cut_on_a_character() {
        let mut reference = a_reference(6);
        reference.name = "é".repeat(400);
        let references = References { planes: [None, None, Some(reference)], floor: false };
        let back = References::decode(&references.encode().unwrap()).unwrap();
        let name = &back.on(Plane::Top).unwrap().name;
        assert!(name.len() <= MAX_NAME_BYTES && name.chars().all(|c| c == 'é'));
    }

    #[test]
    fn each_plane_faces_its_view_and_its_edges_are_square_to_it() {
        for plane in Plane::ALL {
            let (across, up) = plane.across_and_up();
            assert_eq!(across.dot(plane.normal()), 0.0);
            assert_eq!(up.dot(plane.normal()), 0.0);
            // Across, up and out make a right-handed frame, as a screen's
            // right, up and towards the viewer do.
            assert_eq!(across.cross(up), plane.normal(), "{plane:?} is seen mirrored");
            assert_eq!(plane.normal()[plane.normal_axis()], 1.0);
        }
    }

    proptest! {
        #[test]
        fn damaged_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
            let _ = References::decode(&bytes);
        }

        #[test]
        fn any_references_round_trip(
            floor in any::<bool>(),
            held in proptest::collection::vec(any::<bool>(), 3),
            depth in -500.0f32..500.0,
            width in 0.1f32..900.0,
            opacity in 0.0f32..=1.0,
            picture in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let mut references = References { floor, ..References::default() };
            for (plane, on) in Plane::ALL.into_iter().zip(held) {
                if on {
                    let mut reference = Reference::new(
                        format!("{plane:?}"),
                        Arc::from(picture.clone()),
                        depth,
                        width,
                    );
                    reference.opacity = opacity;
                    references.planes[plane.index()] = Some(reference);
                }
            }
            match references.encode() {
                None => prop_assert!(references.is_empty()),
                Some(bytes) => prop_assert_eq!(References::decode(&bytes), Some(references)),
            }
        }
    }
}
