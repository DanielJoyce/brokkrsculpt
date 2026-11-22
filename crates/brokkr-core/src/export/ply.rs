// SPDX-License-Identifier: AGPL-3.0-only

//! PLY with a colour at every vertex.
//!
//! The one format here that carries what a painted model looks like, as
//! opposed to which filament prints each part of it ([`super::threemf`]).
//! It is the plainest thing a full colour print service or another program
//! will take: a list of points, each with a colour, and a list of triangles.
//! No texture, no material file beside it, no unwrap.
//!
//! Binary, little endian, and the layout MeshLab writes, because that is the
//! layout every reader has been tested against:
//!
//! ```text
//! vertex:  float x y z,  float nx ny nz,  uchar red green blue
//! face:    list uchar int vertex_indices
//! ```
//!
//! The colours are sRGB, which is what a PLY's `uchar red` means to
//! everything that opens one, and which is the reason
//! [`crate::paint::Paint::shade`]'s linear answer goes through
//! [`crate::paint::srgb`] before it gets here.
//!
//! A colour belongs to a vertex and is blended across each triangle by
//! whatever draws the file. So the edge between two paints is one triangle
//! wide in a PLY, where in a 3MF it is exact. One triangle is a voxel, and a
//! voxel is finer than a colour printer resolves. The vertices could be
//! doubled along the boundary to keep it sharp, and are not: that would open
//! the mesh along every paint line, and a print service checks that a mesh
//! is closed before it looks at anything else.
//!
//! Coordinates and normals are turned to Z-up on the way out, as in every
//! other writer. See [`crate::orientation`].

use std::io::{self, Write};

use super::ExportMesh;
use crate::orientation::to_print_space;

/// One body and the colour of each of its vertices.
#[derive(Debug, Clone, Copy)]
pub struct Coloured<'a> {
    pub mesh: &'a ExportMesh,
    /// sRGB, one for each of `mesh.positions`, in order.
    pub colours: &'a [[u8; 3]],
}

/// Bytes one vertex takes: six floats and three bytes.
const VERTEX_BYTES: usize = 6 * 4 + 3;
/// Bytes one triangle takes: a count of one byte and three indices.
const FACE_BYTES: usize = 1 + 3 * 4;

fn header(vertices: usize, faces: usize) -> String {
    format!(
        "ply\n\
         format binary_little_endian 1.0\n\
         comment Exported by BrokkrSculpt\n\
         comment Units are millimetres\n\
         element vertex {vertices}\n\
         property float x\n\
         property float y\n\
         property float z\n\
         property float nx\n\
         property float ny\n\
         property float nz\n\
         property uchar red\n\
         property uchar green\n\
         property uchar blue\n\
         element face {faces}\n\
         property list uchar int vertex_indices\n\
         end_header\n"
    )
}

/// How many bytes [`write_all`] will write for these bodies.
pub fn size_of_all(bodies: &[Coloured<'_>]) -> usize {
    let vertices: usize = bodies.iter().map(|body| body.mesh.positions.len()).sum();
    let faces: usize = bodies.iter().map(|body| body.mesh.triangles.len()).sum();
    header(vertices, faces).len() + vertices * VERTEX_BYTES + faces * FACE_BYTES
}

/// Write several bodies as one PLY.
///
/// A PLY is one mesh with no names in it, so the bodies are laid end to end
/// and each one's triangles carry a running offset, for the reason the OBJ
/// writer's do. Their vertices are never merged with each other's.
///
/// Refused, before a byte is written, when a body does not have exactly one
/// colour a vertex, or when there are more vertices than a PLY's 32 bit
/// signed index can name. A normal that is missing is written as zero, which
/// is what a reader takes "no normal" to be.
pub fn write_all(bodies: &[Coloured<'_>], out: &mut impl Write) -> io::Result<()> {
    let mut vertices = 0_usize;
    let mut faces = 0_usize;
    for (index, body) in bodies.iter().enumerate() {
        if body.colours.len() != body.mesh.positions.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "body {index} has {} vertices and {} colours",
                    body.mesh.positions.len(),
                    body.colours.len()
                ),
            ));
        }
        vertices += body.mesh.positions.len();
        faces += body.mesh.triangles.len();
    }
    if vertices > i32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{vertices} vertices is more than a PLY can index"),
        ));
    }

    let mut out = io::BufWriter::new(out);
    out.write_all(header(vertices, faces).as_bytes())?;

    for body in bodies {
        let mesh = body.mesh;
        let has_normals = mesh.normals.len() == mesh.positions.len();
        for (index, position) in mesh.positions.iter().enumerate() {
            let position = to_print_space(*position);
            let normal =
                if has_normals { to_print_space(mesh.normals[index]) } else { glam::Vec3::ZERO };
            for value in [position.x, position.y, position.z, normal.x, normal.y, normal.z] {
                out.write_all(&value.to_le_bytes())?;
            }
            out.write_all(&body.colours[index])?;
        }
    }
    let mut offset = 0_u32;
    for body in bodies {
        for triangle in &body.mesh.triangles {
            out.write_all(&[3])?;
            for corner in triangle {
                // Checked above to fit: the sum of every body's vertices is
                // no more than `i32::MAX`.
                out.write_all(&((corner + offset) as i32).to_le_bytes())?;
            }
        }
        offset += body.mesh.positions.len() as u32;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// What a PLY written here holds, read back the way a stranger's reader
    /// would: the header a line at a time, then the bytes it describes.
    struct Read {
        vertices: Vec<(Vec3, Vec3, [u8; 3])>,
        faces: Vec<[i32; 3]>,
        leftover: usize,
    }

    fn read(bytes: &[u8]) -> Read {
        let end = b"end_header\n";
        let split = bytes
            .windows(end.len())
            .position(|window| window == end)
            .expect("the file has no end_header")
            + end.len();
        let header = std::str::from_utf8(&bytes[..split]).expect("the header is not text");
        let mut lines = header.lines();
        assert_eq!(lines.next(), Some("ply"), "the first line of a PLY is `ply`");
        assert_eq!(lines.next(), Some("format binary_little_endian 1.0"));
        let count = |element: &str| -> usize {
            header
                .lines()
                .find_map(|line| line.strip_prefix(&format!("element {element} ")))
                .unwrap_or_else(|| panic!("no element {element}"))
                .parse()
                .expect("an element count is a number")
        };
        // The properties, in order, are the layout. A reader goes by these
        // lines and by nothing else.
        let properties: Vec<&str> =
            header.lines().filter(|line| line.starts_with("property ")).collect();
        assert_eq!(
            properties,
            [
                "property float x",
                "property float y",
                "property float z",
                "property float nx",
                "property float ny",
                "property float nz",
                "property uchar red",
                "property uchar green",
                "property uchar blue",
                "property list uchar int vertex_indices",
            ]
        );

        let mut rest = &bytes[split..];
        let float = |rest: &mut &[u8]| {
            let value = f32::from_le_bytes(rest[..4].try_into().unwrap());
            *rest = &rest[4..];
            value
        };
        let mut vertices = Vec::new();
        for _ in 0..count("vertex") {
            let position = Vec3::new(float(&mut rest), float(&mut rest), float(&mut rest));
            let normal = Vec3::new(float(&mut rest), float(&mut rest), float(&mut rest));
            let colour = [rest[0], rest[1], rest[2]];
            rest = &rest[3..];
            vertices.push((position, normal, colour));
        }
        let mut faces = Vec::new();
        for _ in 0..count("face") {
            assert_eq!(rest[0], 3, "a face that is not a triangle");
            rest = &rest[1..];
            let mut corners = [0_i32; 3];
            for corner in &mut corners {
                *corner = i32::from_le_bytes(rest[..4].try_into().unwrap());
                rest = &rest[4..];
            }
            faces.push(corners);
        }
        Read { vertices, faces, leftover: rest.len() }
    }

    fn triangle(at: f32) -> ExportMesh {
        ExportMesh {
            positions: vec![
                Vec3::new(at, 1.0, 2.0),
                Vec3::new(at + 1.0, 1.0, 2.0),
                Vec3::new(at, 2.0, 2.0),
            ],
            normals: vec![Vec3::Z, Vec3::Z, Vec3::Y],
            triangles: vec![[0, 1, 2]],
            slots: Vec::new(),
        }
    }

    /// Two bodies come back as one mesh: every vertex with its own colour in
    /// its own place, turned to Z-up, and the second body's triangle on the
    /// second body's vertices.
    #[test]
    fn two_bodies_read_back_with_their_colours_and_their_own_vertices() {
        let (first, second) = (triangle(0.0), triangle(10.0));
        let first_colours = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
        let second_colours = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
        let bodies = [
            Coloured { mesh: &first, colours: &first_colours },
            Coloured { mesh: &second, colours: &second_colours },
        ];
        let mut bytes = Vec::new();
        write_all(&bodies, &mut bytes).expect("writing to memory");
        assert_eq!(bytes.len(), size_of_all(&bodies), "the size was predicted wrongly");

        let back = read(&bytes);
        assert_eq!(back.leftover, 0, "there are bytes after the last face");
        assert_eq!(back.vertices.len(), 6);
        assert_eq!(back.faces, [[0, 1, 2], [3, 4, 5]], "the second body's indices are not offset");
        for (index, (position, normal, colour)) in back.vertices.iter().enumerate() {
            let (mesh, colours, local) = if index < 3 {
                (&first, &first_colours, index)
            } else {
                (&second, &second_colours, index - 3)
            };
            assert_eq!(*position, to_print_space(mesh.positions[local]), "vertex {index}");
            assert_eq!(*normal, to_print_space(mesh.normals[local]), "normal {index}");
            assert_eq!(*colour, colours[local], "colour {index}");
        }
        // The turn is a turn: sculpt up is print up, and the control for
        // "turned" is that the two are not the same vector to begin with.
        assert_eq!(back.vertices[2].1, Vec3::Z, "sculpt +Y did not become print +Z");
        assert_ne!(to_print_space(Vec3::Y), Vec3::Y);
    }

    /// A body without one colour a vertex is refused before anything is
    /// written, so a half file is never left where a whole one was asked
    /// for.
    #[test]
    fn a_body_with_the_wrong_number_of_colours_is_refused_and_nothing_is_written() {
        let mesh = triangle(0.0);
        for colours in [&[][..], &[[0, 0, 0]; 2][..], &[[0, 0, 0]; 4][..]] {
            let mut bytes = Vec::new();
            let error = write_all(&[Coloured { mesh: &mesh, colours }], &mut bytes)
                .expect_err("a colour count that does not match was accepted");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(bytes.is_empty(), "{} bytes were written before the refusal", bytes.len());
        }
    }

    /// Nothing at all is still a file a reader accepts, and a mesh with no
    /// normals writes zeros where they would be and keeps its layout.
    #[test]
    fn an_empty_export_and_a_mesh_without_normals_are_both_whole_files() {
        let mut bytes = Vec::new();
        write_all(&[], &mut bytes).expect("writing to memory");
        let back = read(&bytes);
        assert!(back.vertices.is_empty() && back.faces.is_empty() && back.leftover == 0);

        let bare = ExportMesh { normals: Vec::new(), ..triangle(0.0) };
        let colours = [[9, 9, 9]; 3];
        let mut bytes = Vec::new();
        write_all(&[Coloured { mesh: &bare, colours: &colours }], &mut bytes).unwrap();
        let back = read(&bytes);
        assert_eq!(back.leftover, 0);
        assert!(back.vertices.iter().all(|(_, normal, _)| *normal == Vec3::ZERO));
        assert_eq!(back.vertices[1].0, to_print_space(bare.positions[1]));
        assert_eq!(back.faces, [[0, 1, 2]]);
    }
}
