// SPDX-License-Identifier: AGPL-3.0-only

//! Wavefront OBJ with a texture: the mesh, a place on a picture for every
//! corner of every triangle, and a material file that names the picture.
//!
//! Three files travel together and all three are plain: `model.obj`,
//! `model.mtl`, and a PNG for each body. It is the oldest way there is of
//! handing a textured model to another program, and for that reason the
//! one most of them read.
//!
//! This is a file of its own and not more of [`super::obj`], which is
//! upstream's and is left as it is. The two agree on everything they share:
//! millimetres, Z up, indices from one and from the top of the file.
//!
//! The picture itself is not written here. A PNG is a file format with a
//! compressor in it, this crate has neither, and the application that calls
//! this already has both.

use std::io::{self, Write};

use super::ExportMesh;
use super::unwrap::Unwrap;
use crate::orientation::to_print_space;

/// One body to write: its mesh, where each triangle is on its picture, and
/// the picture's file name as the material file should give it.
#[derive(Debug, Clone, Copy)]
pub struct Textured<'a> {
    pub name: &'a str,
    pub mesh: &'a ExportMesh,
    pub unwrap: &'a Unwrap,
    /// The picture's file name, with no directory: it is looked for beside
    /// the material file.
    pub picture: &'a str,
}

/// The name a body's material goes by in both files.
fn material(index: usize) -> String {
    format!("body_{}", index + 1)
}

/// Write the bodies as one OBJ that takes its materials from `materials`,
/// the material file's name with no directory.
///
/// A texture coordinate in an OBJ runs from the BOTTOM left of the picture,
/// where an unwrap's points run from the top left, so `v` is turned over on
/// the way out. Getting that wrong draws every chart with the right shape
/// and the wrong contents, which looks like a bad bake and is not one.
pub fn write_obj(bodies: &[Textured<'_>], materials: &str, out: &mut impl Write) -> io::Result<()> {
    for (index, body) in bodies.iter().enumerate() {
        if body.unwrap.corners.len() != body.mesh.triangles.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("body {index} was unwrapped from another mesh"),
            ));
        }
    }
    let mut out = io::BufWriter::new(out);
    writeln!(out, "# Exported by BrokkrSculpt")?;
    writeln!(out, "# Units are millimetres")?;
    writeln!(out, "mtllib {materials}")?;

    let (mut vertices, mut places) = (0_u32, 0_u32);
    for (index, body) in bodies.iter().enumerate() {
        let (mesh, unwrap) = (body.mesh, body.unwrap);
        writeln!(out, "o {}", body.name)?;
        for position in &mesh.positions {
            let position = to_print_space(*position);
            writeln!(out, "v {} {} {}", position.x, position.y, position.z)?;
        }
        let size = unwrap.size as f32;
        for point in &unwrap.points {
            writeln!(out, "vt {} {}", point.x / size, 1.0 - point.y / size)?;
        }
        let has_normals = mesh.normals.len() == mesh.positions.len();
        if has_normals {
            for normal in &mesh.normals {
                let normal = to_print_space(*normal);
                writeln!(out, "vn {} {} {}", normal.x, normal.y, normal.z)?;
            }
        }
        writeln!(out, "usemtl {}", material(index))?;
        for (triangle, corners) in mesh.triangles.iter().zip(&unwrap.corners) {
            write!(out, "f")?;
            for (vertex, place) in triangle.iter().zip(corners) {
                let (v, vt) = (vertex + vertices + 1, place + places + 1);
                if has_normals {
                    write!(out, " {v}/{vt}/{v}")?;
                } else {
                    write!(out, " {v}/{vt}")?;
                }
            }
            writeln!(out)?;
        }
        vertices += mesh.positions.len() as u32;
        places += unwrap.points.len() as u32;
    }
    out.flush()
}

/// Write the material file: one material a body, each white and matte with
/// its picture as its colour.
pub fn write_mtl(bodies: &[Textured<'_>], out: &mut impl Write) -> io::Result<()> {
    writeln!(out, "# Exported by BrokkrSculpt")?;
    for (index, body) in bodies.iter().enumerate() {
        writeln!(out, "newmtl {}", material(index))?;
        // White, so the picture is multiplied by one and is the colour.
        writeln!(out, "Ka 1 1 1")?;
        writeln!(out, "Kd 1 1 1")?;
        writeln!(out, "Ks 0 0 0")?;
        writeln!(out, "illum 1")?;
        writeln!(out, "map_Kd {}", body.picture)?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::unwrap::UnwrapSpec;
    use crate::volume::Volume;
    use glam::{Vec2, Vec3};

    fn ball(radius: f32) -> ExportMesh {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::ZERO, radius);
        volume.export_mesh().0
    }

    /// What an OBJ holds, read back a line at a time.
    #[derive(Default)]
    struct Read {
        positions: Vec<Vec3>,
        places: Vec<Vec2>,
        normals: usize,
        /// For each face, three `(v, vt, vn)`, all counted from one.
        faces: Vec<[(usize, usize, Option<usize>); 3]>,
        objects: Vec<String>,
        materials: Vec<String>,
        library: Option<String>,
    }

    fn read(text: &str) -> Read {
        let mut read = Read::default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let number = |word: Option<&str>| word.unwrap().parse::<f32>().unwrap();
            match words.next() {
                Some("v") => read.positions.push(Vec3::new(
                    number(words.next()),
                    number(words.next()),
                    number(words.next()),
                )),
                Some("vt") => {
                    read.places.push(Vec2::new(number(words.next()), number(words.next())))
                }
                Some("vn") => read.normals += 1,
                Some("o") => read.objects.push(line[2..].to_string()),
                Some("usemtl") => read.materials.push(words.next().unwrap().to_string()),
                Some("mtllib") => read.library = Some(words.next().unwrap().to_string()),
                Some("f") => {
                    let corners: Vec<(usize, usize, Option<usize>)> = words
                        .map(|corner| {
                            let mut parts = corner.split('/');
                            (
                                parts.next().unwrap().parse().unwrap(),
                                parts.next().unwrap().parse().unwrap(),
                                parts.next().map(|part| part.parse().unwrap()),
                            )
                        })
                        .collect();
                    read.faces.push(corners.try_into().expect("a face is a triangle"));
                }
                _ => {}
            }
        }
        read
    }

    /// Two bodies come back as two objects with a material each, every
    /// face's corners on the right vertices and at the right places on the
    /// picture, the second body's counted on from the first's.
    #[test]
    fn two_textured_bodies_read_back_with_their_own_places_on_their_own_pictures() {
        let (first, second) = (ball(8.0), ball(5.0));
        let spec = UnwrapSpec { size: 256, ..UnwrapSpec::default() };
        let (first_unwrap, second_unwrap) = (first.unwrapped(spec), second.unwrapped(spec));
        let bodies = [
            Textured { name: "Head", mesh: &first, unwrap: &first_unwrap, picture: "m_1.png" },
            Textured {
                name: "Left hand",
                mesh: &second,
                unwrap: &second_unwrap,
                picture: "m_2.png",
            },
        ];
        let mut bytes = Vec::new();
        write_obj(&bodies, "m.mtl", &mut bytes).expect("writing to memory");
        let back = read(std::str::from_utf8(&bytes).expect("an OBJ is text"));

        assert_eq!(back.library.as_deref(), Some("m.mtl"));
        assert_eq!(back.objects, ["Head", "Left hand"]);
        assert_eq!(back.materials, ["body_1", "body_2"]);
        assert_eq!(back.positions.len(), first.positions.len() + second.positions.len());
        assert_eq!(back.places.len(), first_unwrap.points.len() + second_unwrap.points.len());
        assert_eq!(back.normals, back.positions.len());
        assert_eq!(back.faces.len(), first.triangles.len() + second.triangles.len());
        assert!(
            back.places
                .iter()
                .all(|place| (0.0..=1.0).contains(&place.x) && (0.0..=1.0).contains(&place.y))
        );

        let mut face = 0;
        let (mut vertices, mut places) = (0, 0);
        for (mesh, unwrap) in [(&first, &first_unwrap), (&second, &second_unwrap)] {
            for (triangle, corners) in mesh.triangles.iter().zip(&unwrap.corners) {
                for corner in 0..3 {
                    let (v, vt, vn) = back.faces[face][corner];
                    assert_eq!(v - 1, triangle[corner] as usize + vertices, "face {face}");
                    assert_eq!(vt - 1, corners[corner] as usize + places, "face {face}");
                    assert_eq!(vn, Some(v), "a normal is its vertex's");
                    // The position is the mesh's, turned to Z up, and the
                    // place is the unwrap's with `v` counted from the
                    // bottom.
                    let position = to_print_space(mesh.positions[triangle[corner] as usize]);
                    assert!((back.positions[v - 1] - position).length() < 1.0e-4);
                    let point = unwrap.points[corners[corner] as usize];
                    let place = Vec2::new(point.x / 256.0, 1.0 - point.y / 256.0);
                    assert!((back.places[vt - 1] - place).length() < 1.0e-5, "face {face}");
                }
                face += 1;
            }
            vertices += mesh.positions.len();
            places += unwrap.points.len();
        }
        // The control for "turned over": a point near the top of the atlas
        // is near 1 in the file, and the unwrap has one.
        let top = first_unwrap.points.iter().map(|point| point.y).fold(f32::MAX, f32::min);
        assert!(top < 32.0, "the fixture has nothing near the top of its atlas");
        assert!(back.places.iter().any(|place| place.y > 0.85));

        let mut bytes = Vec::new();
        write_mtl(&bodies, &mut bytes).expect("writing to memory");
        let materials = String::from_utf8(bytes).expect("a material file is text");
        let named: Vec<&str> =
            materials.lines().filter(|line| line.starts_with("newmtl")).collect();
        assert_eq!(named, ["newmtl body_1", "newmtl body_2"]);
        let pictures: Vec<&str> =
            materials.lines().filter(|line| line.starts_with("map_Kd")).collect();
        assert_eq!(pictures, ["map_Kd m_1.png", "map_Kd m_2.png"]);
        assert_eq!(materials.matches("Kd 1 1 1").count(), 2, "a material is not white");
    }

    /// An unwrap of some other mesh is refused before a byte is written.
    #[test]
    fn an_unwrap_of_another_mesh_is_refused_and_nothing_is_written() {
        let (one, other) = (ball(8.0), ball(5.0));
        let unwrap = other.unwrapped(UnwrapSpec { size: 128, ..UnwrapSpec::default() });
        let mut bytes = Vec::new();
        let refused = write_obj(
            &[Textured { name: "x", mesh: &one, unwrap: &unwrap, picture: "x.png" }],
            "x.mtl",
            &mut bytes,
        );
        assert_eq!(refused.expect_err("a mismatched unwrap").kind(), io::ErrorKind::InvalidInput);
        assert!(bytes.is_empty());
    }

    /// A mesh with no normals writes faces without them and stays a file.
    #[test]
    fn a_mesh_without_normals_writes_faces_without_them() {
        let mesh = ExportMesh { normals: Vec::new(), ..ball(5.0) };
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 128, ..UnwrapSpec::default() });
        let mut bytes = Vec::new();
        write_obj(
            &[Textured { name: "bare", mesh: &mesh, unwrap: &unwrap, picture: "b.png" }],
            "b.mtl",
            &mut bytes,
        )
        .unwrap();
        let back = read(std::str::from_utf8(&bytes).unwrap());
        assert_eq!(back.normals, 0);
        assert_eq!(back.faces.len(), mesh.triangles.len());
        assert!(back.faces.iter().all(|face| face.iter().all(|corner| corner.2.is_none())));
    }
}
