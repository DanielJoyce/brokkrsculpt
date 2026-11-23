// SPDX-License-Identifier: AGPL-3.0-only

//! GLB: a textured model in one file.
//!
//! Binary glTF 2.0. Where an OBJ with a texture is three files that have to
//! stay together ([`super::textured`]), a GLB is one, with the mesh and the
//! pictures inside it, and it is what web viewers, game engines and most
//! full colour print services take first.
//!
//! # What is in the file
//!
//! A twelve byte header, a chunk of JSON that says what everything is, and
//! a chunk of bytes that is everything: for each body its positions,
//! normals, places on its picture and triangle indices, and then the
//! picture itself, as the PNG the caller made. Each chunk is padded to a
//! multiple of four bytes, the JSON with spaces and the bytes with zeros,
//! as the format requires.
//!
//! # Three things this format means that the others here do not
//!
//! * **Metres.** Everything else this application writes is millimetres. A
//!   glTF is metres, by its specification and not by convention, so every
//!   position is divided by a thousand on the way out. A model written in
//!   millimetres opens a thousand times too big and nothing says why.
//! * **Y is up.** So is the sculpt, which makes this the one writer that
//!   does not turn the model: the others turn it to Z up for a printer.
//! * **A vertex is one position, one normal and one place on the picture.**
//!   A vertex of the mesh on the edge of two charts has two places, so it
//!   is written twice. The triangles still meet there; they just do not
//!   share the vertex.
//!
//! The JSON is written by hand. It is forty lines of fixed shape with a
//! handful of numbers and one kind of string in it, the bodies' names,
//! which are escaped.
//!
//! # What it does not carry yet
//!
//! One picture a body, its base colour. A paint's finish (how matt or how
//! metallic it is) is in the picture only as far as it changed the colour:
//! every material here is the same matt non-metal. A roughness and metal
//! picture is the next map to bake, and this is where it would be named.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, Write};

use glam::Vec3;

use super::ExportMesh;
use super::unwrap::Unwrap;

/// One body to write: its mesh, where each triangle is on its picture, and
/// the picture, already encoded as a PNG.
#[derive(Debug, Clone, Copy)]
pub struct Body<'a> {
    pub name: &'a str,
    pub mesh: &'a ExportMesh,
    pub unwrap: &'a Unwrap,
    pub png: &'a [u8],
}

/// Millimetres to metres.
const TO_METRES: f32 = 0.001;

/// A piece of the binary chunk, as the JSON describes it.
struct View {
    offset: usize,
    length: usize,
    /// 34962 for vertex data, 34963 for indices, none for a picture.
    target: Option<u32>,
}

/// A string as JSON: quoted, with the characters JSON cannot hold as they
/// are written the way it wants them.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            // A line end or a tab among them: JSON has shorter ways to
            // write those and no need of them.
            other if (other as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", other as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A normal for each position of a mesh that came without them: the
/// normals of the triangles round it, the bigger the triangle the more it
/// counts. Not yet made unit length.
fn faced(mesh: &ExportMesh) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; mesh.positions.len()];
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|vertex| mesh.positions[vertex as usize]);
        let area = (b - a).cross(c - a);
        for vertex in triangle {
            normals[*vertex as usize] += area;
        }
    }
    normals
}

/// Write the bodies as one GLB.
///
/// A body with no triangles is left out: the format has no way to say "a
/// mesh of nothing". Refused, before a byte is written, when there is then
/// nothing to write, when a body's unwrap is of another mesh, when a body
/// has no picture or a position that is not a number, or when the file
/// would be past the four gigabytes its own header can count.
pub fn write(bodies: &[Body<'_>], out: &mut impl Write) -> io::Result<()> {
    let refuse = |why: String| Err(io::Error::new(io::ErrorKind::InvalidInput, why));
    let bodies: Vec<&Body<'_>> =
        bodies.iter().filter(|body| !body.mesh.triangles.is_empty()).collect();
    if bodies.is_empty() {
        return refuse("there is nothing to write".to_string());
    }
    for body in &bodies {
        let name = body.name;
        if body.unwrap.corners.len() != body.mesh.triangles.len() {
            return refuse(format!("{name} was unwrapped from another mesh"));
        }
        if body.png.is_empty() {
            return refuse(format!("{name} has no picture"));
        }
        if !body.mesh.positions.iter().all(|position| position.is_finite()) {
            return refuse(format!("{name} has a position that is not a number"));
        }
    }

    let mut bin: Vec<u8> = Vec::new();
    let mut views: Vec<View> = Vec::new();
    // For each body: its vertex count, index count, and the least and most
    // of its positions, which the format requires be stated.
    let mut shapes: Vec<(usize, usize, Vec3, Vec3)> = Vec::new();
    let mut view = |bin: &mut Vec<u8>, bytes: &[u8], target: Option<u32>| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(View { offset: bin.len(), length: bytes.len(), target });
        bin.extend_from_slice(bytes);
    };

    for body in &bodies {
        let (mesh, unwrap) = (body.mesh, body.unwrap);
        let facing: Cow<'_, [Vec3]> = if mesh.normals.len() == mesh.positions.len() {
            Cow::Borrowed(&mesh.normals)
        } else {
            Cow::Owned(faced(mesh))
        };
        let size = unwrap.size as f32;
        // One vertex for each pairing of a position with a place.
        let mut made: HashMap<(u32, u32), u32> = HashMap::new();
        let (mut positions, mut normals, mut places, mut indices) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut count = 0_u32;
        for (triangle, corners) in mesh.triangles.iter().zip(&unwrap.corners) {
            for (vertex, place) in triangle.iter().zip(corners) {
                let index = *made.entry((*vertex, *place)).or_insert_with(|| {
                    let position = mesh.positions[*vertex as usize] * TO_METRES;
                    low = low.min(position);
                    high = high.max(position);
                    for value in position.to_array() {
                        positions.extend_from_slice(&value.to_le_bytes());
                    }
                    // The format wants unit normals and says so.
                    let normal = facing[*vertex as usize].normalize_or(Vec3::Y);
                    for value in normal.to_array() {
                        normals.extend_from_slice(&value.to_le_bytes());
                    }
                    // A glTF's picture is read from its top left, as an
                    // unwrap's points are: nothing is turned over.
                    let at = unwrap.points[*place as usize] / size;
                    for value in [at.x, at.y] {
                        places.extend_from_slice(&value.to_le_bytes());
                    }
                    count += 1;
                    count - 1
                });
                indices.extend_from_slice(&index.to_le_bytes());
            }
        }
        view(&mut bin, &positions, Some(34962));
        view(&mut bin, &normals, Some(34962));
        view(&mut bin, &places, Some(34962));
        view(&mut bin, &indices, Some(34963));
        view(&mut bin, body.png, None);
        shapes.push((count as usize, mesh.triangles.len() * 3, low, high));
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    // The JSON. Five views a body, in the order they were made: positions,
    // normals, places, indices, picture. Four accessors a body, for the
    // first four.
    let list = |items: Vec<String>| items.join(",");
    let json = format!(
        concat!(
            "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"BrokkrSculpt\"}},",
            "\"scene\":0,\"scenes\":[{{\"nodes\":[{nodes}]}}],",
            "\"nodes\":[{node_list}],",
            "\"meshes\":[{meshes}],",
            "\"materials\":[{materials}],",
            "\"textures\":[{textures}],",
            "\"images\":[{images}],",
            "\"samplers\":[{{\"magFilter\":9729,\"minFilter\":9987,\"wrapS\":33071,\"wrapT\":33071}}],",
            "\"accessors\":[{accessors}],",
            "\"bufferViews\":[{buffer_views}],",
            "\"buffers\":[{{\"byteLength\":{bytes}}}]}}"
        ),
        nodes = list((0..bodies.len()).map(|index| index.to_string()).collect()),
        node_list = list(
            bodies
                .iter()
                .enumerate()
                .map(|(index, body)| format!("{{\"name\":{},\"mesh\":{index}}}", quoted(body.name)))
                .collect()
        ),
        meshes = list(
            (0..bodies.len())
                .map(|index| {
                    let first = index * 4;
                    format!(
                        "{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{},\"NORMAL\":{},\
                         \"TEXCOORD_0\":{}}},\"indices\":{},\"material\":{index}}}]}}",
                        first,
                        first + 1,
                        first + 2,
                        first + 3
                    )
                })
                .collect()
        ),
        // Not metal, and not shiny: a painted model, lit by whatever opens
        // it.
        materials = list(
            (0..bodies.len())
                .map(|index| {
                    format!(
                        "{{\"pbrMetallicRoughness\":{{\"baseColorTexture\":{{\"index\":{index}}},\
                         \"metallicFactor\":0,\"roughnessFactor\":0.9}}}}"
                    )
                })
                .collect()
        ),
        textures = list(
            (0..bodies.len())
                .map(|index| format!("{{\"sampler\":0,\"source\":{index}}}"))
                .collect()
        ),
        images = list(
            (0..bodies.len())
                .map(|index| {
                    format!("{{\"bufferView\":{},\"mimeType\":\"image/png\"}}", index * 5 + 4)
                })
                .collect()
        ),
        accessors = list(
            shapes
                .iter()
                .enumerate()
                .flat_map(|(index, (vertices, corners, low, high))| {
                    let view = index * 5;
                    [
                        format!(
                            "{{\"bufferView\":{view},\"componentType\":5126,\"count\":{vertices},\
                             \"type\":\"VEC3\",\"min\":[{},{},{}],\"max\":[{},{},{}]}}",
                            low.x, low.y, low.z, high.x, high.y, high.z
                        ),
                        format!(
                            "{{\"bufferView\":{},\"componentType\":5126,\"count\":{vertices},\
                             \"type\":\"VEC3\"}}",
                            view + 1
                        ),
                        format!(
                            "{{\"bufferView\":{},\"componentType\":5126,\"count\":{vertices},\
                             \"type\":\"VEC2\"}}",
                            view + 2
                        ),
                        format!(
                            "{{\"bufferView\":{},\"componentType\":5125,\"count\":{corners},\
                             \"type\":\"SCALAR\"}}",
                            view + 3
                        ),
                    ]
                })
                .collect()
        ),
        buffer_views = list(
            views
                .iter()
                .map(|view| match view.target {
                    Some(target) => format!(
                        "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{},\"target\":{target}}}",
                        view.offset, view.length
                    ),
                    None => format!(
                        "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{}}}",
                        view.offset, view.length
                    ),
                })
                .collect()
        ),
        bytes = bin.len(),
    );
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }

    let total = 12 + 8 + json.len() + 8 + bin.len();
    let Ok(total) = u32::try_from(total) else {
        return refuse(format!("{total} bytes is more than a GLB can be"));
    };
    out.write_all(b"glTF")?;
    out.write_all(&2_u32.to_le_bytes())?;
    out.write_all(&total.to_le_bytes())?;
    out.write_all(&(json.len() as u32).to_le_bytes())?;
    out.write_all(b"JSON")?;
    out.write_all(&json)?;
    out.write_all(&(bin.len() as u32).to_le_bytes())?;
    out.write_all(b"BIN\0")?;
    out.write_all(&bin)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::unwrap::UnwrapSpec;
    use crate::volume::Volume;
    use glam::Vec2;

    fn ball(radius: f32) -> ExportMesh {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::ZERO, radius);
        volume.export_mesh().0
    }

    /// JSON, read back. Small, strict, and only for these tests: it is not
    /// what proves the JSON is JSON (the application's test reads a GLB
    /// with a real parser for that), it is what lets these tests follow
    /// the file's own indices to its bytes.
    #[derive(Debug, Clone, PartialEq)]
    enum Json {
        Null,
        Bool(bool),
        Number(f64),
        Text(String),
        List(Vec<Json>),
        Object(Vec<(String, Json)>),
    }

    impl Json {
        fn find(&self, key: &str) -> Option<&Json> {
            match self {
                Json::Object(members) => {
                    members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
                }
                _ => None,
            }
        }

        fn get(&self, key: &str) -> &Json {
            self.find(key).unwrap_or_else(|| panic!("no {key} in {self:?}"))
        }

        fn list(&self) -> &[Json] {
            match self {
                Json::List(items) => items,
                other => panic!("{other:?} is not a list"),
            }
        }

        fn number(&self) -> f64 {
            match self {
                Json::Number(number) => *number,
                other => panic!("{other:?} is not a number"),
            }
        }

        fn index(&self) -> usize {
            let number = self.number();
            assert!(number >= 0.0 && number.fract() == 0.0, "{number} is not an index");
            number as usize
        }

        fn text(&self) -> &str {
            match self {
                Json::Text(text) => text,
                other => panic!("{other:?} is not a string"),
            }
        }
    }

    struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl Reader<'_> {
        fn space(&mut self) {
            while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
                self.at += 1;
            }
        }

        fn take(&mut self, byte: u8) {
            self.space();
            assert_eq!(
                self.bytes[self.at] as char, byte as char,
                "at byte {} of the JSON",
                self.at
            );
            self.at += 1;
        }

        fn word(&mut self, word: &str, value: Json) -> Json {
            assert_eq!(&self.bytes[self.at..self.at + word.len()], word.as_bytes());
            self.at += word.len();
            value
        }

        fn string(&mut self) -> String {
            self.take(b'"');
            let mut out = Vec::new();
            loop {
                let byte = self.bytes[self.at];
                self.at += 1;
                match byte {
                    b'"' => break,
                    b'\\' => {
                        let escaped = self.bytes[self.at];
                        self.at += 1;
                        match escaped {
                            b'"' | b'\\' | b'/' => out.push(escaped),
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'u' => {
                                let hex = std::str::from_utf8(&self.bytes[self.at..self.at + 4])
                                    .expect("four hex digits");
                                let code = u32::from_str_radix(hex, 16).expect("four hex digits");
                                self.at += 4;
                                let character = char::from_u32(code).expect("a character");
                                out.extend_from_slice(character.to_string().as_bytes());
                            }
                            other => panic!("\\{} is not an escape", other as char),
                        }
                    }
                    control if control < 0x20 => panic!("a bare control character in a string"),
                    other => out.push(other),
                }
            }
            String::from_utf8(out).expect("JSON is UTF-8")
        }

        fn value(&mut self) -> Json {
            self.space();
            match self.bytes[self.at] {
                b'{' => {
                    self.at += 1;
                    let mut members = Vec::new();
                    self.space();
                    if self.bytes[self.at] == b'}' {
                        self.at += 1;
                        return Json::Object(members);
                    }
                    loop {
                        self.space();
                        let name = self.string();
                        self.take(b':');
                        members.push((name, self.value()));
                        self.space();
                        if self.bytes[self.at] == b'}' {
                            self.at += 1;
                            return Json::Object(members);
                        }
                        self.take(b',');
                    }
                }
                b'[' => {
                    self.at += 1;
                    let mut items = Vec::new();
                    self.space();
                    if self.bytes[self.at] == b']' {
                        self.at += 1;
                        return Json::List(items);
                    }
                    loop {
                        items.push(self.value());
                        self.space();
                        if self.bytes[self.at] == b']' {
                            self.at += 1;
                            return Json::List(items);
                        }
                        self.take(b',');
                    }
                }
                b'"' => Json::Text(self.string()),
                b't' => self.word("true", Json::Bool(true)),
                b'f' => self.word("false", Json::Bool(false)),
                b'n' => self.word("null", Json::Null),
                _ => {
                    let from = self.at;
                    while self.at < self.bytes.len()
                        && matches!(
                            self.bytes[self.at],
                            b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                        )
                    {
                        self.at += 1;
                    }
                    let text = std::str::from_utf8(&self.bytes[from..self.at]).expect("ASCII");
                    Json::Number(text.parse().unwrap_or_else(|_| panic!("{text:?} is no number")))
                }
            }
        }
    }

    /// A GLB taken apart: its JSON and its bytes, with the framing checked
    /// on the way.
    struct Glb {
        json: Json,
        bin: Vec<u8>,
    }

    fn u32_at(bytes: &[u8], at: usize) -> usize {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes")) as usize
    }

    fn open(bytes: &[u8]) -> Glb {
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(u32_at(bytes, 4), 2, "the version");
        assert_eq!(u32_at(bytes, 8), bytes.len(), "the header counts the whole file");
        let json_length = u32_at(bytes, 12);
        assert_eq!(&bytes[16..20], b"JSON");
        assert_eq!(json_length % 4, 0, "the JSON is padded to four bytes");
        let json_bytes = &bytes[20..20 + json_length];
        let after = 20 + json_length;
        let bin_length = u32_at(bytes, after);
        assert_eq!(&bytes[after + 4..after + 8], b"BIN\0");
        assert_eq!(bin_length % 4, 0, "the bytes are padded to four");
        assert_eq!(after + 8 + bin_length, bytes.len(), "and nothing follows them");
        let mut reader = Reader { bytes: json_bytes, at: 0 };
        let json = reader.value();
        assert!(
            json_bytes[reader.at..].iter().all(|byte| *byte == b' '),
            "only spaces pad the JSON"
        );
        let bin = bytes[after + 8..].to_vec();
        assert_eq!(json.get("buffers").list().len(), 1);
        assert_eq!(json.get("buffers").list()[0].get("byteLength").index(), bin.len());
        Glb { json, bin }
    }

    impl Glb {
        /// The bytes a view names, and what it says they are for.
        fn view(&self, index: usize) -> (&[u8], Option<usize>) {
            let view = &self.json.get("bufferViews").list()[index];
            assert_eq!(view.get("buffer").index(), 0);
            let (offset, length) = (view.get("byteOffset").index(), view.get("byteLength").index());
            assert!(length > 0, "a view of nothing is not allowed");
            assert!(offset + length <= self.bin.len(), "view {index} runs off the end");
            (&self.bin[offset..offset + length], view.find("target").map(Json::index))
        }

        /// An accessor's numbers, as floats or as indices, checked against
        /// what it says of itself.
        fn accessor(&self, index: usize, kind: &str, component: usize, target: usize) -> &[u8] {
            let accessor = &self.json.get("accessors").list()[index];
            assert_eq!(accessor.get("type").text(), kind, "accessor {index}");
            assert_eq!(accessor.get("componentType").index(), component, "accessor {index}");
            let view = accessor.get("bufferView").index();
            let (bytes, to) = self.view(view);
            assert_eq!(to, Some(target), "accessor {index}'s view is for the wrong thing");
            let offset = self.json.get("bufferViews").list()[view].get("byteOffset").index();
            assert_eq!(offset % 4, 0, "accessor {index}'s numbers start on a four byte boundary");
            let width = match kind {
                "VEC3" => 12,
                "VEC2" => 8,
                _ => 4,
            };
            assert_eq!(bytes.len(), accessor.get("count").index() * width, "accessor {index}");
            bytes
        }

        fn floats(&self, index: usize, kind: &str) -> Vec<f32> {
            self.accessor(index, kind, 5126, 34962)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|four| f32::from_le_bytes(*four))
                .collect()
        }

        /// One body, followed from the scene down: its name, its vertices
        /// (position, normal, place), its triangles, and its picture's
        /// bytes.
        fn body(&self, which: usize) -> Read {
            let scene = &self.json.get("scenes").list()[self.json.get("scene").index()];
            let node = &self.json.get("nodes").list()[scene.get("nodes").list()[which].index()];
            let mesh = &self.json.get("meshes").list()[node.get("mesh").index()];
            assert_eq!(mesh.get("primitives").list().len(), 1);
            let primitive = &mesh.get("primitives").list()[0];
            let attributes = primitive.get("attributes");
            let position = attributes.get("POSITION").index();
            let positions: Vec<Vec3> = self
                .floats(position, "VEC3")
                .as_chunks::<3>()
                .0
                .iter()
                .map(|three| Vec3::new(three[0], three[1], three[2]))
                .collect();
            let stated = &self.json.get("accessors").list()[position];
            let corner = |key: &str| {
                let numbers = stated.get(key).list();
                Vec3::new(
                    numbers[0].number() as f32,
                    numbers[1].number() as f32,
                    numbers[2].number() as f32,
                )
            };
            let normals: Vec<Vec3> = self
                .floats(attributes.get("NORMAL").index(), "VEC3")
                .as_chunks::<3>()
                .0
                .iter()
                .map(|three| Vec3::new(three[0], three[1], three[2]))
                .collect();
            let places: Vec<Vec2> = self
                .floats(attributes.get("TEXCOORD_0").index(), "VEC2")
                .as_chunks::<2>()
                .0
                .iter()
                .map(|two| Vec2::new(two[0], two[1]))
                .collect();
            let indices: Vec<u32> = self
                .accessor(primitive.get("indices").index(), "SCALAR", 5125, 34963)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|four| u32::from_le_bytes(*four))
                .collect();
            let material = &self.json.get("materials").list()[primitive.get("material").index()];
            let texture = material.get("pbrMetallicRoughness").get("baseColorTexture");
            let texture = &self.json.get("textures").list()[texture.get("index").index()];
            assert_eq!(texture.get("sampler").index(), 0);
            let image = &self.json.get("images").list()[texture.get("source").index()];
            assert_eq!(image.get("mimeType").text(), "image/png");
            let (picture, target) = self.view(image.get("bufferView").index());
            assert_eq!(target, None, "a picture is neither vertices nor indices");
            Read {
                name: node.get("name").text().to_string(),
                positions,
                low: corner("min"),
                high: corner("max"),
                normals,
                places,
                indices,
                picture: picture.to_vec(),
            }
        }
    }

    struct Read {
        name: String,
        positions: Vec<Vec3>,
        low: Vec3,
        high: Vec3,
        normals: Vec<Vec3>,
        places: Vec<Vec2>,
        indices: Vec<u32>,
        picture: Vec<u8>,
    }

    /// Everything a body went in with comes back, by following the file's
    /// own indices: each triangle's corners at their positions, in metres,
    /// and at their places on the picture.
    fn assert_reads_back(read: &Read, mesh: &ExportMesh, unwrap: &Unwrap, png: &[u8]) {
        assert_eq!(read.picture, png, "the picture's bytes");
        assert_eq!(read.normals.len(), read.positions.len());
        assert_eq!(read.places.len(), read.positions.len());
        assert_eq!(read.indices.len(), mesh.triangles.len() * 3);
        let size = unwrap.size as f32;
        for (triangle, (vertices, places)) in mesh.triangles.iter().zip(&unwrap.corners).enumerate()
        {
            for corner in 0..3 {
                let index = read.indices[triangle * 3 + corner] as usize;
                assert!(index < read.positions.len(), "an index past the vertices");
                let wanted = mesh.positions[vertices[corner] as usize] * 0.001;
                assert_eq!(read.positions[index], wanted, "triangle {triangle} corner {corner}");
                let wanted = unwrap.points[places[corner] as usize] / size;
                assert_eq!(read.places[index], wanted, "triangle {triangle} corner {corner}");
            }
        }
        let low = read.positions.iter().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
        let high = read.positions.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
        assert_eq!((read.low, read.high), (low, high), "the stated least and most");
        for normal in &read.normals {
            assert!((normal.length() - 1.0).abs() < 1e-4, "a normal {normal} is not unit");
        }
        for place in &read.places {
            assert!(place.min_element() >= 0.0 && place.max_element() <= 1.0, "{place}");
        }
    }

    /// Two bodies come back as two nodes with a mesh, a material and a
    /// picture each, none of them the other's. The pictures are given odd
    /// lengths so that the padding between pieces is exercised.
    #[test]
    fn two_bodies_read_back_with_their_own_meshes_and_their_own_pictures() {
        let (first, second) = (ball(8.0), ball(5.0));
        let spec = UnwrapSpec { size: 256, ..UnwrapSpec::default() };
        let (first_unwrap, second_unwrap) = (first.unwrapped(spec), second.unwrapped(spec));
        let (first_png, second_png) = (b"picture".as_slice(), b"other".as_slice());
        let bodies = [
            Body { name: "Head", mesh: &first, unwrap: &first_unwrap, png: first_png },
            Body { name: "Left hand", mesh: &second, unwrap: &second_unwrap, png: second_png },
        ];
        let mut bytes = Vec::new();
        write(&bodies, &mut bytes).expect("writing to memory");
        let glb = open(&bytes);
        assert_eq!(glb.json.get("asset").get("version").text(), "2.0");
        assert_eq!(glb.json.get("scenes").list()[0].get("nodes").list().len(), 2);
        for list in ["nodes", "meshes", "materials", "textures", "images"] {
            assert_eq!(glb.json.get(list).list().len(), 2, "{list}");
        }
        assert_eq!(glb.json.get("accessors").list().len(), 8);
        assert_eq!(glb.json.get("bufferViews").list().len(), 10);

        let (head, hand) = (glb.body(0), glb.body(1));
        assert_eq!((head.name.as_str(), hand.name.as_str()), ("Head", "Left hand"));
        assert_reads_back(&head, &first, &first_unwrap, first_png);
        assert_reads_back(&hand, &second, &second_unwrap, second_png);

        // Control: the two are different enough that reading one for the
        // other would have failed above.
        assert_ne!(head.positions.len(), hand.positions.len());
    }

    /// Millimetres in, metres out, and nothing turned: a ball 8 mm in
    /// radius is 0.008 across each way from its middle, and the vertex
    /// that was highest in Y still is.
    #[test]
    fn a_body_is_written_in_metres_and_the_way_up_it_was_sculpted() {
        let mut mesh = ball(8.0);
        // Not a ball any more: taller than it is wide, so that a turn
        // would show.
        for position in &mut mesh.positions {
            position.y *= 2.0;
        }
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 256, ..UnwrapSpec::default() });
        let mut bytes = Vec::new();
        write(&[Body { name: "Egg", mesh: &mesh, unwrap: &unwrap, png: b"png" }], &mut bytes)
            .expect("writing to memory");
        let read = open(&bytes).body(0);
        assert!((read.high.x - 0.008).abs() < 0.0005, "{}", read.high);
        assert!((read.high.y - 0.016).abs() < 0.0005, "{}", read.high);
        assert!((read.high.z - 0.008).abs() < 0.0005, "{}", read.high);
        assert!((read.low.y + 0.016).abs() < 0.0005, "{}", read.low);
    }

    /// A vertex on the edge of two charts has two places on the picture
    /// and is written once for each, and no more often than that.
    #[test]
    fn a_vertex_is_written_once_for_each_place_it_has() {
        let mesh = ball(8.0);
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 256, ..UnwrapSpec::default() });
        let mut pairs = std::collections::HashSet::new();
        for (vertices, places) in mesh.triangles.iter().zip(&unwrap.corners) {
            for corner in 0..3 {
                pairs.insert((vertices[corner], places[corner]));
            }
        }
        let mut bytes = Vec::new();
        write(&[Body { name: "Ball", mesh: &mesh, unwrap: &unwrap, png: b"png" }], &mut bytes)
            .expect("writing to memory");
        let read = open(&bytes).body(0);
        assert_eq!(read.positions.len(), pairs.len());
        // Control: a ball has seams, so this is more than its vertices and
        // the test is of something.
        assert!(pairs.len() > mesh.positions.len(), "the ball was unwrapped as one chart");
    }

    /// A mesh that came without normals gets them from its triangles, and
    /// on a ball they point out.
    #[test]
    fn a_mesh_without_normals_is_given_them_from_its_triangles() {
        let mesh = ExportMesh { normals: Vec::new(), ..ball(8.0) };
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 256, ..UnwrapSpec::default() });
        let mut bytes = Vec::new();
        write(&[Body { name: "Ball", mesh: &mesh, unwrap: &unwrap, png: b"png" }], &mut bytes)
            .expect("writing to memory");
        let read = open(&bytes).body(0);
        assert_reads_back(&read, &mesh, &unwrap, b"png");
        for (position, normal) in read.positions.iter().zip(&read.normals) {
            let out = position.normalize();
            assert!(normal.dot(out) > 0.9, "the normal {normal} at {position} does not point out");
        }
    }

    /// The normals a mesh came with are the ones written, and not ones
    /// worked out again from its triangles: they were averaged from the
    /// model, which knows its own surface better than the mesh does.
    #[test]
    fn the_normals_a_mesh_came_with_are_the_ones_written() {
        let mut mesh = ball(8.0);
        // Normals no triangle would give, so the two cannot be confused.
        let tilted = Vec3::new(0.6, 0.0, 0.8);
        mesh.normals = vec![tilted; mesh.positions.len()];
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 256, ..UnwrapSpec::default() });
        let mut bytes = Vec::new();
        write(&[Body { name: "Ball", mesh: &mesh, unwrap: &unwrap, png: b"png" }], &mut bytes)
            .expect("writing to memory");
        let read = open(&bytes).body(0);
        assert!(read.normals.iter().all(|normal| *normal == tilted));
        assert!(!read.normals.is_empty());
    }

    /// A body's name goes into the JSON whatever is in it.
    #[test]
    fn a_name_with_quotes_and_line_ends_in_it_is_still_json() {
        let mesh = ball(5.0);
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 128, ..UnwrapSpec::default() });
        let name = "The \"left\" hand\\arm\n\ttab \u{1} and Brokkr's own: Brokkr \u{00e9}\u{4e16}";
        let mut bytes = Vec::new();
        write(&[Body { name, mesh: &mesh, unwrap: &unwrap, png: b"png" }], &mut bytes)
            .expect("writing to memory");
        let json = &bytes[20..20 + u32_at(&bytes, 12)];
        assert!(
            json.iter().all(|byte| *byte >= 0x20),
            "a control character went into the JSON as it was"
        );
        assert_eq!(open(&bytes).body(0).name, name);
    }

    /// A body with nothing in it is left out and the rest are written.
    #[test]
    fn a_body_of_nothing_is_left_out() {
        let mesh = ball(5.0);
        let spec = UnwrapSpec { size: 128, ..UnwrapSpec::default() };
        let unwrap = mesh.unwrapped(spec);
        let nothing = ExportMesh::default();
        let no_unwrap = nothing.unwrapped(spec);
        let bodies = [
            Body { name: "Nothing", mesh: &nothing, unwrap: &no_unwrap, png: b"png" },
            Body { name: "Ball", mesh: &mesh, unwrap: &unwrap, png: b"ball" },
        ];
        let mut bytes = Vec::new();
        write(&bodies, &mut bytes).expect("writing to memory");
        let glb = open(&bytes);
        assert_eq!(glb.json.get("nodes").list().len(), 1);
        let read = glb.body(0);
        assert_eq!(read.name, "Ball");
        assert_reads_back(&read, &mesh, &unwrap, b"ball");
    }

    /// What cannot be a GLB is refused before anything is written.
    #[test]
    fn what_cannot_be_written_is_refused_with_nothing_written() {
        let (mesh, other) = (ball(5.0), ball(6.0));
        let spec = UnwrapSpec { size: 128, ..UnwrapSpec::default() };
        let (unwrap, other_unwrap) = (mesh.unwrapped(spec), other.unwrapped(spec));
        let nothing = ExportMesh::default();
        let no_unwrap = nothing.unwrapped(spec);
        let mut broken = mesh.clone();
        broken.positions[3].y = f32::NAN;

        let good = Body { name: "Ball", mesh: &mesh, unwrap: &unwrap, png: b"png" };
        let refused: [(&str, Vec<Body<'_>>); 5] = [
            ("no bodies", Vec::new()),
            ("only an empty body", vec![Body { mesh: &nothing, unwrap: &no_unwrap, ..good }]),
            ("another mesh's unwrap", vec![good, Body { unwrap: &other_unwrap, ..good }]),
            ("no picture", vec![good, Body { png: b"", ..good }]),
            ("a position that is not a number", vec![Body { mesh: &broken, ..good }]),
        ];
        for (what, bodies) in refused {
            let mut bytes = Vec::new();
            let error = write(&bodies, &mut bytes).expect_err(what);
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{what}");
            assert!(bytes.is_empty(), "{what}: {} bytes were written first", bytes.len());
        }
        // Control: the good body on its own is written.
        let mut bytes = Vec::new();
        write(&[good], &mut bytes).expect("writing to memory");
        assert!(!bytes.is_empty());
    }
}
