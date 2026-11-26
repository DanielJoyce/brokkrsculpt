// SPDX-License-Identifier: AGPL-3.0-only

//! A normal map: the sculpt's own surface direction, as a picture, so that a
//! mesh thinned to a few thousand triangles is still lit as the model is.
//!
//! # The frame is written, not assumed
//!
//! A normal map holds each texel's normal in the frame of the triangle under
//! it: along the tangent, along the bitangent, and along the mesh's normal.
//! glTF builds that frame from a vertex's `NORMAL` and `TANGENT`, the
//! bitangent being `cross(normal, tangent.xyz) * tangent.w`. A file that
//! leaves `TANGENT` out has its frame made up by the viewer, by an algorithm
//! (MikkTSpace) that this bake would have to match exactly. So the tangents
//! are worked out here ([`tangents`]) and written into the file beside the
//! normals, and the bake uses the very same ones, interpolated across each
//! triangle as a viewer interpolates them: what the viewer undoes is exactly
//! what was done.
//!
//! A tangent runs the way the picture's `u` increases across the triangle,
//! and its `w` is the sign that makes the bitangent run the way `v`
//! increases, as glTF reads it (from the picture's top left, down).
//!
//! # The surface is the field's
//!
//! Each texel's point is stepped onto the surface as every bake here does,
//! and its normal is the field's gradient there: the sculpt, not the thinned
//! mesh. Where the two agree the texel is the flat `(0.5, 0.5, 1)` blue.

use std::collections::HashMap;

use glam::{Vec3, Vec4};

use super::ExportMesh;
use super::texture::Texture;
use super::unwrap::Unwrap;
use crate::volume::Volume;

/// The unit normal each position of `mesh` is drawn with: its own, or
/// where it came without them the normals of the triangles round it, the
/// bigger the triangle the more it counts.
pub fn vertex_normals(mesh: &ExportMesh) -> Vec<Vec3> {
    if mesh.normals.len() == mesh.positions.len() {
        return mesh.normals.iter().map(|normal| normal.normalize_or(Vec3::Y)).collect();
    }
    let mut normals = vec![Vec3::ZERO; mesh.positions.len()];
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|vertex| mesh.positions[vertex as usize]);
        let area = (b - a).cross(c - a);
        for vertex in triangle {
            normals[*vertex as usize] += area;
        }
    }
    normals.into_iter().map(|normal| normal.normalize_or(Vec3::Y)).collect()
}

/// A tangent for each pairing of a position with a place on the picture,
/// which is what a vertex of a glTF is: unit length, square to the
/// position's normal, running the way `u` does, with in `w` the sign that
/// makes `cross(normal, tangent) * w` run the way `v` does.
pub fn tangents(mesh: &ExportMesh, unwrap: &Unwrap, normals: &[Vec3]) -> HashMap<(u32, u32), Vec4> {
    // The ways `u` and `v` run across each triangle, added up at each
    // corner: bigger triangles count for more, as the sums are not made
    // unit length first.
    let mut sums: HashMap<(u32, u32), (Vec3, Vec3)> = HashMap::new();
    for (triangle, corners) in mesh.triangles.iter().zip(&unwrap.corners) {
        let [p0, p1, p2] = triangle.map(|vertex| mesh.positions[vertex as usize]);
        let [t0, t1, t2] = corners.map(|place| unwrap.points[place as usize]);
        let (e1, e2) = (p1 - p0, p2 - p0);
        let (d1, d2) = (t1 - t0, t2 - t0);
        let det = d1.x * d2.y - d2.x * d1.y;
        if det.abs() <= f32::MIN_POSITIVE {
            continue;
        }
        let along_u = (e1 * d2.y - e2 * d1.y) / det;
        let along_v = (e2 * d1.x - e1 * d2.x) / det;
        for (vertex, place) in triangle.iter().zip(corners) {
            let sum = sums.entry((*vertex, *place)).or_insert((Vec3::ZERO, Vec3::ZERO));
            sum.0 += along_u;
            sum.1 += along_v;
        }
    }
    let mut out = HashMap::with_capacity(sums.len());
    for (triangle, corners) in mesh.triangles.iter().zip(&unwrap.corners) {
        for (vertex, place) in triangle.iter().zip(corners) {
            let key = (*vertex, *place);
            if out.contains_key(&key) {
                continue;
            }
            let normal = normals[*vertex as usize];
            let (along_u, along_v) = sums.get(&key).copied().unwrap_or((Vec3::ZERO, Vec3::ZERO));
            let tangent = (along_u - normal * normal.dot(along_u))
                .try_normalize()
                .unwrap_or_else(|| normal.any_orthonormal_vector());
            let w = if normal.cross(tangent).dot(along_v) < 0.0 { -1.0 } else { 1.0 };
            out.insert(key, tangent.extend(w));
        }
    }
    out
}

/// A normal from the frame `normal`, `tangent` (with its sign in `w`), as a
/// texel: each of its three parts from -1..=1 to 0..=255.
fn encoded(surface: Vec3, normal: Vec3, tangent: Vec4) -> [u8; 3] {
    let bitangent = normal.cross(tangent.truncate()) * tangent.w;
    let local =
        Vec3::new(surface.dot(tangent.truncate()), surface.dot(bitangent), surface.dot(normal));
    local.to_array().map(|part| ((part.clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0).round() as u8)
}

impl Unwrap {
    /// The surface's normal, baked onto this unwrap of `mesh` in the frame
    /// [`tangents`] gives it, which is the frame a glTF of this mesh and
    /// this unwrap is written with.
    pub fn normals_baked(&self, mesh: &ExportMesh, volume: &Volume, margin: u32) -> Texture {
        let normals = vertex_normals(mesh);
        let tangents = tangents(mesh, self, &normals);
        self.baked_by(mesh, volume, margin, |point, triangle, weights| {
            let vertices = mesh.triangles[triangle];
            let places = self.corners[triangle];
            let (mut normal, mut tangent, mut sign) = (Vec3::ZERO, Vec3::ZERO, 1.0);
            for corner in 0..3 {
                normal += normals[vertices[corner] as usize] * weights[corner];
                let at = tangents
                    .get(&(vertices[corner], places[corner]))
                    .copied()
                    .unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0));
                tangent += at.truncate() * weights[corner];
                sign = at.w;
            }
            let normal = normal.normalize_or(Vec3::Y);
            let tangent = (tangent - normal * normal.dot(tangent))
                .try_normalize()
                .unwrap_or_else(|| normal.any_orthonormal_vector());
            let surface = volume.gradient_world(point).normalize_or(normal);
            encoded(surface, normal, tangent.extend(sign))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::unwrap::UnwrapSpec;
    use glam::Vec2;

    /// Every tangent is unit length and square to its normal, and with its
    /// sign the frame runs the way the picture does: along a triangle, the
    /// tangent the way `u` grows and the bitangent the way `v` grows.
    #[test]
    fn the_tangents_run_the_way_the_picture_does() {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        let (mesh, _) = volume.export_mesh();
        let unwrap = mesh.unwrapped(UnwrapSpec { size: 512, ..UnwrapSpec::default() });
        let normals = vertex_normals(&mesh);
        let tangents = tangents(&mesh, &unwrap, &normals);
        let mut checked = 0;
        for (triangle, corners) in mesh.triangles.iter().zip(&unwrap.corners) {
            let [p0, p1, p2] = triangle.map(|vertex| mesh.positions[vertex as usize]);
            let [t0, t1, t2] = corners.map(|place| unwrap.points[place as usize]);
            let (e1, e2, d1, d2) = (p1 - p0, p2 - p0, t1 - t0, t2 - t0);
            let det = d1.x * d2.y - d2.x * d1.y;
            if det.abs() < 1e-3 {
                continue;
            }
            let along_u = (e1 * d2.y - e2 * d1.y) / det;
            let along_v = (e2 * d1.x - e1 * d2.x) / det;
            let (vertex, place) = (triangle[0], corners[0]);
            let normal = normals[vertex as usize];
            let tangent = tangents[&(vertex, place)];
            assert!((tangent.truncate().length() - 1.0).abs() < 1e-3);
            assert!(tangent.truncate().dot(normal).abs() < 1e-3);
            let bitangent = normal.cross(tangent.truncate()) * tangent.w;
            assert!(tangent.truncate().dot(along_u) > 0.0, "the tangent runs against u");
            assert!(bitangent.dot(along_v) > 0.0, "the bitangent runs against v");
            checked += 1;
        }
        assert!(checked > 1000, "only {checked} triangles were checked");
    }

    /// A ball rippled finer than a thinned mesh's triangles: the picture
    /// leans the way each ripple does, and decoded through the same frame a
    /// viewer builds, a texel gives back the field's own normal at its
    /// point.
    #[test]
    fn a_texel_decodes_to_the_fields_normal_through_its_frame() {
        use crate::brick::{INSIDE, OUTSIDE};
        let mut volume = Volume::new(0.25);
        let reach = Vec3::splat(13.0);
        volume.edit_box(-reach, reach, |position, _| {
            let ripple = 0.35 * (position.x * 1.5).sin() * (position.y * 1.5).sin();
            ((position.length() - 10.0 - ripple) / 0.25).clamp(INSIDE, OUTSIDE)
        });
        let (mesh, _, _) =
            volume.export_mesh_decimated(crate::export::decimate::DecimateSpec::within(0.5));
        let spec = UnwrapSpec { size: 512, ..UnwrapSpec::default() };
        let unwrap = mesh.unwrapped(spec);
        let picture = unwrap.normals_baked(&mesh, &volume, spec.margin);
        let normals = vertex_normals(&mesh);
        let tangents = tangents(&mesh, &unwrap, &normals);
        let (mut checked, mut leaning) = (0, 0);
        for (index, (triangle, corners)) in mesh.triangles.iter().zip(&unwrap.corners).enumerate() {
            let flat = corners.map(|place| unwrap.points[place as usize]);
            let centre = (flat[0] + flat[1] + flat[2]) / 3.0;
            let (x, y) = (centre.x as usize, centre.y as usize);
            let texel_centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let area = (flat[1] - flat[0]).perp_dot(flat[2] - flat[0]);
            let weights = [
                (flat[1] - texel_centre).perp_dot(flat[2] - texel_centre) / area,
                (flat[2] - texel_centre).perp_dot(flat[0] - texel_centre) / area,
                (flat[0] - texel_centre).perp_dot(flat[1] - texel_centre) / area,
            ];
            if weights.iter().any(|weight| *weight < 0.05) {
                continue;
            }
            // The frame a viewer builds at this texel.
            let (mut normal, mut tangent) = (Vec3::ZERO, Vec3::ZERO);
            let mut sign = 1.0;
            let mut point = Vec3::ZERO;
            for corner in 0..3 {
                normal += normals[triangle[corner] as usize] * weights[corner];
                let at = tangents[&(triangle[corner], corners[corner])];
                tangent += at.truncate() * weights[corner];
                sign = at.w;
                point += mesh.positions[triangle[corner] as usize] * weights[corner];
            }
            let normal = normal.normalize();
            let tangent = (tangent - normal * normal.dot(tangent)).normalize();
            let bitangent = normal.cross(tangent) * sign;
            let at = (y * picture.size as usize + x) * 4;
            let decode = |byte: u8| f32::from(byte) / 255.0 * 2.0 - 1.0;
            let texel = &picture.rgba[at..at + 3];
            let decoded = (tangent * decode(texel[0])
                + bitangent * decode(texel[1])
                + normal * decode(texel[2]))
            .normalize();
            // The field's normal where the point steps onto the surface.
            let distance = volume.sample_world(point);
            let on = point - volume.gradient_world(point) * (distance * volume.voxel_size());
            let truth = volume.gradient_world(on).normalize();
            assert!(decoded.dot(truth) > 0.995, "triangle {index}: {decoded} against {truth}");
            checked += 1;
            // The thinned mesh's own normal is off the truth here: the
            // picture is carrying something.
            leaning += usize::from(normal.dot(truth) < 0.999);
        }
        assert!(checked > 100, "only {checked} texels were checked");
        assert!(leaning * 4 > checked, "only {leaning} of {checked} texels lean: nothing to carry");
    }
}
