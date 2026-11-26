// SPDX-License-Identifier: AGPL-3.0-only

//! A texture: what a painted model looks like, as a picture on its unwrap.
//!
//! A colour a vertex ([`super::ply`]) is as fine as the mesh. Export a model
//! at a tenth of its triangles and a wash that pooled in a groove is a
//! smear across whichever big triangle now spans the groove. A texture is
//! as fine as its texels, whatever the mesh under it is: this is what lets a
//! file be small AND look like the model.
//!
//! # The model is the source, not the mesh
//!
//! Every texel under a triangle is a point on that triangle, and a
//! decimated triangle is not quite on the surface. So the point is stepped
//! onto the surface along the field's gradient, one step, and the colour is
//! asked for THERE ([`crate::paint::bake::Bake::surface_colour`]): the
//! paint the nearest voxel holds, shaded by the curvature of the field at
//! that spot. `docs/STUDIO-PLAN.md` decision D5: the field is the oracle.
//! Nothing here compares two meshes or casts a ray, and there is no cage to
//! set up and no "the rays missed" to debug.
//!
//! # Margins
//!
//! A program drawing a textured triangle reads a little past its edge, and
//! a smaller copy of the texture (a mip) averages texels across chart
//! borders. So once every texel under a triangle is baked, the colours at
//! each chart's edge are spread outward into the unwrap's margin, a ring a
//! pass. What is left is filled with one flat grey.

use glam::{Vec2, Vec3};
use rayon::prelude::*;

use super::ExportMesh;
use super::unwrap::Unwrap;
use crate::paint::bake::Bake;
use crate::volume::Volume;

/// A square picture, eight bits a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    /// Texels on a side.
    pub size: u32,
    /// Red, green, blue and alpha for each texel, a row at a time from the
    /// top. sRGB. Always opaque: a renderer that sees alpha in a base
    /// colour may take the model for glass.
    pub rgba: Vec<u8>,
}

/// The grey a texel is when nothing is near it. Mid grey: if it ever shows,
/// through a bad lookup in someone else's program, it is the least wrong
/// colour to see.
pub const BACKGROUND: [u8; 3] = [128, 128, 128];

/// How far a triangle's edge is pushed out when deciding which texel
/// centres it owns, in texels. Half a texel and a little: a texel whose
/// centre is just outside the edge is still half under the triangle, and
/// leaving it for the margin pass would put the neighbouring chart's worth
/// of blur on a seam.
const EDGE_SLACK: f32 = 0.55;

/// How far a texel's point may be stepped to reach the surface, in voxels.
/// A decimated triangle is within a voxel of it ([`super::decimate`]); a
/// step longer than this is a field that is not a distance near the point,
/// and the point is better left where it is.
const LONGEST_STEP: f32 = 2.0;

impl Unwrap {
    /// Bake `volume`'s paint onto this unwrap of `mesh`, which must be a
    /// mesh of that volume.
    ///
    /// `margin` is how many texels the edge colours are spread outward by,
    /// and should be the unwrap's own.
    pub fn baked(
        &self,
        mesh: &ExportMesh,
        volume: &Volume,
        bake: &Bake<'_>,
        margin: u32,
    ) -> Texture {
        self.baked_by(mesh, volume, margin, |point, _, _| bake.surface_colour(volume, point))
    }

    /// The surface's occlusion, roughness and metal, baked onto this unwrap
    /// as one picture the way glTF packs them: red how open the surface is
    /// (1 open to the sky, toward 0 shut in a recess), green the roughness
    /// and blue the metalness. The finish is the one a varnish laid, and
    /// where none is the matte non-metal every material here was before
    /// (stage 6 of `docs/PAINT-RACK.md`). The occlusion is the one a wash
    /// pools by ([`Volume::occlusion_at`]), so a viewer's ambient light
    /// darkens the recesses the paint already knows about.
    pub fn finish_baked(&self, mesh: &ExportMesh, volume: &Volume, margin: u32) -> Texture {
        let voxel_size = volume.voxel_size();
        self.baked_by(mesh, volume, margin, |point, _, _| {
            let voxel = (point / voxel_size).round().as_ivec3();
            let code = volume.finish_at(voxel);
            let finish =
                crate::paint::Finish::from_code(code).unwrap_or(crate::paint::Finish::Matte);
            let (rough, metal) = finish.roughness_and_metal();
            let open = 1.0 - volume.occlusion_at(voxel);
            [
                (open * 255.0).round() as u8,
                (rough * 255.0).round() as u8,
                (metal * 255.0).round() as u8,
            ]
        })
    }

    /// Bake whatever `colour` says of each point of the surface onto this
    /// unwrap of `mesh`, a mesh of `volume`. `colour` is also told which
    /// triangle the texel is under and where in it, as weights of its three
    /// corners that add to one, for a bake that needs the triangle's own
    /// frame (`super::normal_map`).
    pub(super) fn baked_by(
        &self,
        mesh: &ExportMesh,
        volume: &Volume,
        margin: u32,
        colour: impl Fn(Vec3, usize, [f32; 3]) -> [u8; 3] + Sync,
    ) -> Texture {
        let size = self.size as usize;
        let voxel_size = volume.voxel_size();

        // Every texel each triangle owns, worked out across every core and
        // written afterwards: the triangles of a seam own the same texels,
        // and two threads writing one texel is a race even when they would
        // write the same thing.
        let owned: Vec<Vec<(u32, [u8; 3])>> = self
            .corners
            .par_iter()
            .zip(mesh.triangles.par_iter())
            .enumerate()
            .map(|(index, (corners, triangle))| {
                let flat = corners.map(|point| self.points[point as usize]);
                let solid = triangle.map(|vertex| mesh.positions[vertex as usize]);
                let mut texels = Vec::new();
                let area = (flat[1] - flat[0]).perp_dot(flat[2] - flat[0]);
                if area.abs() <= f32::MIN_POSITIVE {
                    return texels;
                }
                let low = (flat[0].min(flat[1]).min(flat[2]) - EDGE_SLACK).floor().max(Vec2::ZERO);
                let high = (flat[0].max(flat[1]).max(flat[2]) + EDGE_SLACK).ceil();
                // How far inside each edge a point is, as a share of the
                // triangle: its barycentric coordinates. And how much of
                // one a texel's slack is, for each edge.
                let lengths = [
                    (flat[2] - flat[1]).length(),
                    (flat[0] - flat[2]).length(),
                    (flat[1] - flat[0]).length(),
                ];
                for y in low.y as usize..(high.y as usize).min(size) {
                    for x in low.x as usize..(high.x as usize).min(size) {
                        let centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let weights = [
                            (flat[1] - centre).perp_dot(flat[2] - centre) / area,
                            (flat[2] - centre).perp_dot(flat[0] - centre) / area,
                            (flat[0] - centre).perp_dot(flat[1] - centre) / area,
                        ];
                        let inside = (0..3)
                            .all(|edge| weights[edge] >= -EDGE_SLACK * lengths[edge] / area.abs());
                        if !inside {
                            continue;
                        }
                        // A texel just past an edge is coloured as the
                        // nearest point of the triangle.
                        let clamped = weights.map(|weight| weight.max(0.0));
                        let total = clamped[0] + clamped[1] + clamped[2];
                        let on_triangle =
                            (solid[0] * clamped[0] + solid[1] * clamped[1] + solid[2] * clamped[2])
                                / total;
                        let weights = clamped.map(|weight| weight / total);
                        let colour =
                            colour(onto_surface(volume, on_triangle, voxel_size), index, weights);
                        texels.push(((y * size + x) as u32, colour));
                    }
                }
                texels
            })
            .collect();

        let mut rgba = vec![0_u8; size * size * 4];
        for texels in &owned {
            for (index, colour) in texels {
                let at = *index as usize * 4;
                rgba[at..at + 3].copy_from_slice(colour);
                rgba[at + 3] = 255;
            }
        }
        spread(&mut rgba, size, margin);
        for texel in rgba.as_chunks_mut::<4>().0.iter_mut() {
            if texel[3] == 0 {
                texel[..3].copy_from_slice(&BACKGROUND);
                texel[3] = 255;
            }
        }
        Texture { size: self.size, rgba }
    }
}

/// A point near the surface, stepped onto it: one step down the field's
/// gradient by the distance the field reads there.
fn onto_surface(volume: &Volume, point: Vec3, voxel_size: f32) -> Vec3 {
    let distance = volume.sample_world(point);
    // Already on it, to within what a texel can show.
    if distance.abs() < 0.05 || distance.abs() > LONGEST_STEP {
        return point;
    }
    point - volume.gradient_world(point) * (distance * voxel_size)
}

/// Spread the baked texels' colours outward into the empty ones around
/// them, `rings` times, a texel further each time.
///
/// An empty texel takes the mean of whichever of its eight neighbours were
/// baked or filled on an EARLIER ring, so a ring grows evenly from the edge
/// it started at and does not smear along the direction it is scanned in.
fn spread(rgba: &mut [u8], size: usize, rings: u32) {
    for _ in 0..rings {
        let mut filled: Vec<(usize, [u8; 3])> = Vec::new();
        for y in 0..size {
            for x in 0..size {
                let index = y * size + x;
                if rgba[index * 4 + 3] != 0 {
                    continue;
                }
                let (mut sum, mut count) = ([0_u32; 3], 0_u32);
                for (dx, dy) in
                    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
                {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= size as i64 || ny >= size as i64 {
                        continue;
                    }
                    let neighbour = (ny as usize * size + nx as usize) * 4;
                    if rgba[neighbour + 3] != 0 {
                        for channel in 0..3 {
                            sum[channel] += u32::from(rgba[neighbour + channel]);
                        }
                        count += 1;
                    }
                }
                if count > 0 {
                    filled.push((
                        index,
                        [0, 1, 2].map(|channel| ((sum[channel] + count / 2) / count) as u8),
                    ));
                }
            }
        }
        if filled.is_empty() {
            break;
        }
        for (index, colour) in filled {
            rgba[index * 4..index * 4 + 3].copy_from_slice(&colour);
            rgba[index * 4 + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::{INSIDE, NARROW_BAND, OUTSIDE};
    use crate::clip::ClipPlane;
    use crate::export::decimate::DecimateSpec;
    use crate::export::unwrap::{UnwrapSpec, strictly_inside};
    use crate::paint::{Paint, PaintTable, Wash};
    use glam::IVec3;

    const RED: [u8; 3] = [200, 40, 30];
    const BLUE: [u8; 3] = [30, 60, 190];
    const CLAY: [u8; 3] = [150, 140, 130];

    fn two_paints() -> PaintTable {
        let mut table = PaintTable::default();
        table.add(Paint::flat("red", RED));
        table.add(Paint::flat("blue", BLUE));
        table
    }

    fn bake(table: &PaintTable) -> Bake<'_> {
        Bake { table, filaments: &[], unpainted: CLAY, crease_gain: 1.0 }
    }

    fn texel(texture: &Texture, x: usize, y: usize) -> [u8; 4] {
        let at = (y * texture.size as usize + x) * 4;
        texture.rgba[at..at + 4].try_into().unwrap()
    }

    /// A ball painted red above and blue below, with a band left bare
    /// round the middle.
    fn painted_ball() -> Volume {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 12.0);
        volume.edit_colour(IVec3::splat(-40), IVec3::splat(40), |_, at, _, _| {
            if at.z > 2.0 {
                1
            } else if at.z < -2.0 {
                2
            } else {
                0
            }
        });
        volume
    }

    /// Every texel under a triangle is the colour of the paint on the
    /// model at that point, every texel is opaque, and the picture is the
    /// size asked.
    #[test]
    fn every_texel_under_a_triangle_is_the_models_paint_there() {
        let volume = painted_ball();
        let (mesh, _) = volume.export_mesh();
        let spec = UnwrapSpec { size: 512, ..UnwrapSpec::default() };
        let unwrap = mesh.unwrapped(spec);
        let table = two_paints();
        let texture = unwrap.baked(&mesh, &volume, &bake(&table), spec.margin);
        assert_eq!(texture.size, 512);
        assert_eq!(texture.rgba.len(), 512 * 512 * 4);
        assert!(
            texture.rgba.as_chunks::<4>().0.iter().all(|texel| texel[3] == 255),
            "a texel is not opaque"
        );

        let (mut red, mut blue, mut bare) = (0, 0, 0);
        for (triangle, corners) in unwrap.corners.iter().enumerate() {
            let flat = corners.map(|point| unwrap.points[point as usize]);
            let solid = mesh.triangles[triangle].map(|vertex| mesh.positions[vertex as usize]);
            // Only triangles well inside one band: on a boundary a texel
            // may fairly be either.
            let heights = solid.map(|corner| corner.z);
            let (low, high) = (
                heights[0].min(heights[1]).min(heights[2]),
                heights[0].max(heights[1]).max(heights[2]),
            );
            let expected = if low > 3.0 {
                RED
            } else if high < -3.0 {
                BLUE
            } else if low > -1.0 && high < 1.0 {
                CLAY
            } else {
                continue;
            };
            let centre = (flat[0] + flat[1] + flat[2]) / 3.0;
            let (x, y) = (centre.x as usize, centre.y as usize);
            if !strictly_inside(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), flat) {
                // Smaller than a texel: nothing to say about it alone.
                continue;
            }
            let got = texel(&texture, x, y);
            assert_eq!(&got[..3], &expected, "triangle {triangle} at {centre}");
            match expected {
                RED => red += 1,
                BLUE => blue += 1,
                _ => bare += 1,
            }
        }
        assert!(red > 500 && blue > 500 && bare > 50, "{red} red, {blue} blue, {bare} bare");
    }

    /// The colours at a chart's edge are spread into the margin and no
    /// further, and what is left is the background.
    #[test]
    fn the_margin_takes_the_edge_colours_and_the_rest_is_background() {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 12.0);
        volume.edit_colour(IVec3::splat(-40), IVec3::splat(40), |_, _, _, _| 1);
        let (mesh, _) = volume.export_mesh();
        let table = two_paints();
        let spec = UnwrapSpec { size: 256, margin: 3, ..UnwrapSpec::default() };
        let unwrap = mesh.unwrapped(spec);

        let none = unwrap.baked(&mesh, &volume, &bake(&table), 0);
        let some = unwrap.baked(&mesh, &volume, &bake(&table), 3);
        let count = |texture: &Texture, colour: [u8; 3]| {
            texture.rgba.as_chunks::<4>().0.iter().filter(|texel| texel[..3] == colour).count()
        };
        // One paint on the whole body: every texel is that or background.
        for texture in [&none, &some] {
            assert_eq!(count(texture, RED) + count(texture, BACKGROUND), 256 * 256);
        }
        assert!(count(&none, RED) > 10_000, "the body covers {} texels", count(&none, RED));
        let grown = count(&some, RED) - count(&none, RED);
        assert!(grown > 1000, "three rings of margin added only {grown} texels");
        // No more than three texels from a baked one, in any direction.
        let baked = |x: i64, y: i64| {
            (0..256).contains(&x)
                && (0..256).contains(&y)
                && texel(&none, x as usize, y as usize)[..3] == RED
        };
        for y in 0..256_i64 {
            for x in 0..256_i64 {
                if texel(&some, x as usize, y as usize)[..3] != RED {
                    continue;
                }
                let near = (-3..=3).any(|dy| (-3..=3).any(|dx| baked(x + dx, y + dy)));
                assert!(near, "the margin reached {x} {y}, more than three texels from the body");
            }
        }
        assert!(count(&some, BACKGROUND) > 1000, "nothing of the atlas is background");
    }

    /// What a texture is for. A cube with a slot in it, a wash that pools
    /// in the slot's corners, and the mesh decimated until a few hundred
    /// triangles are left: the picture still has the wash in the corners,
    /// texel by texel, where a colour a vertex has almost no vertices to
    /// put it on.
    #[test]
    fn a_wash_survives_decimation_in_a_texture() {
        let half = 12.0;
        let mut volume = Volume::new(0.5);
        let reach = Vec3::splat(half + NARROW_BAND);
        volume.edit_box(-reach, reach, |position, _| {
            let outside = position.abs() - Vec3::splat(half);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / 0.5).clamp(INSIDE, OUTSIDE)
        });
        volume.clip_convex(&[
            ClipPlane { point: Vec3::new(-3.0, 0.0, 0.0), normal: Vec3::X },
            ClipPlane { point: Vec3::new(3.0, 0.0, 0.0), normal: -Vec3::X },
            ClipPlane { point: Vec3::new(0.0, 0.0, half - 5.0), normal: Vec3::Z },
        ]);
        volume.edit_colour(IVec3::splat(-40), IVec3::splat(40), |_, _, _, _| 1);
        let bone = [222, 208, 170];
        let mut table = PaintTable::default();
        table.add(Paint {
            wash: Wash { colour: [70, 30, 20], stain: 0.0, flow: 2.0, edge: 0.0 },
            ..Paint::flat("bone", bone)
        });

        let (dense, _) = volume.export_mesh();
        let (mesh, _, report) = volume.export_mesh_decimated(DecimateSpec::within(0.25));
        assert!(
            mesh.triangles.len() * 20 < dense.triangles.len(),
            "the fixture is not much decimated: {}",
            report.summary()
        );
        let spec = UnwrapSpec { size: 1024, ..UnwrapSpec::default() };
        let unwrap = mesh.unwrapped(spec);
        let texture = unwrap.baked(&mesh, &volume, &bake(&table), spec.margin);

        // Texels that are the plain base coat, and texels a good deal
        // darker than it: the flats and the corners. Not counting the
        // background, which is darker than bone too: counted, it was
        // every dark texel there was, and this test passed with the wash
        // switched off.
        let luma = |texel: &[u8]| {
            (2126 * i32::from(texel[0]) + 7152 * i32::from(texel[1]) + 722 * i32::from(texel[2]))
                / 10000
        };
        let base = luma(&bone);
        let flats =
            texture.rgba.as_chunks::<4>().0.iter().filter(|texel| texel[..3] == bone).count();
        let pooled = texture
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|texel| texel[..3] != BACKGROUND && luma(&texel[..]) < base - 60)
            .count();
        assert!(flats > 100_000, "only {flats} texels are the base coat");
        assert!(pooled > 5_000, "only {pooled} texels hold the wash");

        // The control: the same wash as a colour a vertex, on the same
        // decimated mesh, lands on a handful of vertices at best.
        let colours = bake(&table).vertex_colours(&volume, &mesh);
        let dark_vertices = colours.iter().filter(|colour| luma(&colour[..]) < base - 60).count();
        assert!(
            dark_vertices * 10 < pooled,
            "{dark_vertices} vertices hold the wash against {pooled} texels: the control shows \
             nothing"
        );
    }

    /// The occlusion, roughness and metal picture: on a cube with a slot
    /// cut in its top, the slot's floor is baked more shut in (less red)
    /// than the open top, and with no varnish every texel is the matte
    /// non-metal in green and blue.
    #[test]
    fn the_slot_floor_is_baked_shut_in_and_the_top_open() {
        let half = 12.0;
        let mut volume = Volume::new(0.5);
        let reach = Vec3::splat(half + NARROW_BAND);
        volume.edit_box(-reach, reach, |position, _| {
            let outside = position.abs() - Vec3::splat(half);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / 0.5).clamp(INSIDE, OUTSIDE)
        });
        volume.clip_convex(&[
            ClipPlane { point: Vec3::new(-2.0, 0.0, 0.0), normal: Vec3::X },
            ClipPlane { point: Vec3::new(2.0, 0.0, 0.0), normal: -Vec3::X },
            ClipPlane { point: Vec3::new(0.0, 0.0, half - 5.0), normal: Vec3::Z },
        ]);
        let (mesh, _, _) = volume.export_mesh_decimated(DecimateSpec::within(0.25));
        let spec = UnwrapSpec { size: 1024, ..UnwrapSpec::default() };
        let unwrap = mesh.unwrapped(spec);
        let packed = unwrap.finish_baked(&mesh, &volume, spec.margin);
        let (mut floor, mut top) = (Vec::new(), Vec::new());
        for (triangle, corners) in unwrap.corners.iter().enumerate() {
            let flat = corners.map(|point| unwrap.points[point as usize]);
            let solid = mesh.triangles[triangle].map(|vertex| mesh.positions[vertex as usize]);
            let middle = (solid[0] + solid[1] + solid[2]) / 3.0;
            let centre = (flat[0] + flat[1] + flat[2]) / 3.0;
            let (x, y) = (centre.x as usize, centre.y as usize);
            if !strictly_inside(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), flat) {
                continue;
            }
            let got = texel(&packed, x, y);
            assert_eq!(got[1..3], [230, 0], "not a matte non-metal at {middle}");
            if (middle.z - (half - 5.0)).abs() < 0.3 && middle.x.abs() < 1.0 {
                floor.push(got[0]);
            } else if (middle.z - half).abs() < 0.3 && middle.x.abs() > 6.0 {
                top.push(got[0]);
            }
        }
        assert!(!floor.is_empty() && !top.is_empty(), "{} floor, {} top", floor.len(), top.len());
        let mean =
            |values: &[u8]| values.iter().map(|v| f32::from(*v)).sum::<f32>() / values.len() as f32;
        let (floor, top) = (mean(&floor), mean(&top));
        assert!(
            floor + 40.0 < top,
            "the slot's floor ({floor}) is not darker than the top ({top})"
        );
    }

    /// A texel is coloured from ITS point of its triangle. On a cube
    /// decimated down to a few big triangles and painted in stripes a
    /// millimetre wide, a point picked off-centre in each triangle has the
    /// stripe of that point in the picture, and not the stripe of the point
    /// some other mixing of the three corners would give.
    #[test]
    fn a_texel_is_coloured_from_its_own_point_of_a_big_triangle() {
        let half = 10.0;
        let mut volume = Volume::new(0.5);
        let reach = Vec3::splat(half + NARROW_BAND);
        volume.edit_box(-reach, reach, |position, _| {
            let outside = position.abs() - Vec3::splat(half);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / 0.5).clamp(INSIDE, OUTSIDE)
        });
        // Decimated BEFORE it is painted. The decimator keeps every vertex
        // on a boundary between two paints, rightly, and under stripes a
        // millimetre wide that is all of them. The geometry is the same
        // either way, and a texture is baked from the model and not from
        // the mesh's own idea of its paint.
        let (mesh, _, report) = volume.export_mesh_decimated(DecimateSpec::within(0.2));
        assert!(mesh.triangles.len() < 4000, "the cube is not decimated: {}", report.summary());
        // Stripes across every face, running at an angle to all of them.
        volume.edit_colour(IVec3::splat(-40), IVec3::splat(40), |_, at, _, _| {
            if ((at.x + 2.0 * at.y + 3.0 * at.z) / 2.0).floor() as i64 % 2 == 0 { 1 } else { 2 }
        });
        let table = two_paints();
        let spec = UnwrapSpec { size: 1024, ..UnwrapSpec::default() };
        let unwrap = mesh.unwrapped(spec);
        let texture = unwrap.baked(&mesh, &volume, &bake(&table), spec.margin);

        let (mut right, mut wrong, mut big) = (0, 0, 0);
        for (triangle, corners) in unwrap.corners.iter().enumerate() {
            let flat = corners.map(|point| unwrap.points[point as usize]);
            let solid = mesh.triangles[triangle].map(|vertex| mesh.positions[vertex as usize]);
            let texels = (flat[1] - flat[0]).perp_dot(flat[2] - flat[0]).abs() * 0.5;
            if texels < 400.0 {
                continue;
            }
            big += 1;
            // Several points, none of them the middle, and each given by
            // three different shares so that no two corners can be swapped
            // unnoticed.
            for shares in [[0.6, 0.3, 0.1], [0.15, 0.25, 0.6], [0.2, 0.7, 0.1]] {
                let place = flat[0] * shares[0] + flat[1] * shares[1] + flat[2] * shares[2];
                let point = solid[0] * shares[0] + solid[1] * shares[1] + solid[2] * shares[2];
                let expected =
                    bake(&table).surface_colour(&volume, onto_surface(&volume, point, 0.5));
                let got = texel(&texture, place.x as usize, place.y as usize);
                if got[..3] == expected {
                    right += 1;
                } else {
                    wrong += 1;
                }
            }
        }
        assert!(big > 10, "only {big} triangles are big enough to tell");
        // Not every one: a point within half a texel of a stripe's edge can
        // fairly be either colour.
        assert!(
            right > (right + wrong) * 9 / 10,
            "{right} of {} points have their own stripe in the picture",
            right + wrong
        );
    }

    /// A texel on a decimated triangle is coloured from the surface under
    /// it and not from where the flat triangle happens to be.
    #[test]
    fn a_point_off_the_surface_is_stepped_onto_it() {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 12.0);
        for off in [0.6_f32, -0.6, 0.3] {
            let direction = Vec3::new(0.3, -0.5, 0.8).normalize();
            let stepped = onto_surface(&volume, direction * (12.0 + off), 0.5);
            assert!(
                (stepped.length() - 12.0).abs() < 0.05,
                "a point {off} mm off the ball was stepped to {} from its centre",
                stepped.length()
            );
        }
        // Too far to be a step: left where it is.
        let far = Vec3::new(0.0, 0.0, 30.0);
        assert_eq!(onto_surface(&volume, far, 0.5), far);
    }
}
