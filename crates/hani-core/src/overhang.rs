// SPDX-License-Identifier: AGPL-3.0-only

//! Overhangs: where a model, printed as it stands, is printed over nothing.
//!
//! A printer that lays plastic in layers can lean a wall outward only so
//! far before the next layer has nothing under it. Past that the slicer
//! adds support, which costs time and material and leaves a scar where it
//! is broken off. Whether a sculpt needs it, and where, is decided while
//! sculpting: by a chin, an arm, the underside of a cloak. This module is
//! the rule for "where", so that the viewport can show it while there is
//! still time to do something about it, and an export can say how much.
//!
//! # The rule
//!
//! A surface overhangs when it faces down more steeply than the printer
//! manages. The lean is measured the way slicers and printer makers quote
//! it: in degrees **from upright**. A wall is 0, a ceiling is 90, and the
//! figure usually given for filament is 45.
//!
//! A surface leaning `a` from upright and facing down has a normal whose
//! downward part is `sin(a)`. So the test is one comparison, and it is the
//! same comparison in the shader (`sculpt.wgsl`), which is handed
//! [`limit`] and not the angle:
//!
//! ```text
//! -normal.y > sin(steepest)
//! ```
//!
//! Up is +Y, as it is everywhere in a sculpt
//! ([`crate::orientation`]): the model is judged as it stands in the
//! viewport, which is how the export writes it for the slicer. A model
//! turned over in the slicer is a different print.
//!
//! # The plate
//!
//! The underside of a model that rests on the bed faces straight down and
//! is the one such surface that needs no support at all. So nothing counts
//! within one first layer ([`FIRST_LAYER_MM`]) of the lowest point. Only
//! that: the underside of a ball is a real overhang a hair above the point
//! it stands on, and is shown as one.

use glam::{IVec3, Vec3};

use crate::export::ExportMesh;
use crate::volume::Volume;

/// How much of the bottom of a model is the bed's to hold up: one first
/// layer. 0.3 mm is at the thick end of what a filament printer's first
/// layer is, and a resin printer's is far thinner, so this errs toward
/// calling less of the model an overhang, by a third of a millimetre.
pub const FIRST_LAYER_MM: f32 = 0.3;

/// The steepest lean the view offers to test against, in degrees from
/// upright, and the gentlest. Past 80 nearly nothing is an overhang;
/// under 20 nearly everything facing down is.
pub const STEEPEST_RANGE: std::ops::RangeInclusive<f32> = 20.0..=80.0;

/// What a filament printer is usually said to manage.
pub const USUAL_STEEPEST: f32 = 45.0;

/// The number a normal's downward part is compared with, for a printer that
/// manages a lean of `steepest_degrees` from upright.
pub fn limit(steepest_degrees: f32) -> f32 {
    steepest_degrees.clamp(0.0, 90.0).to_radians().sin()
}

/// The height nothing counts below: a first layer above the lowest point.
pub fn plate(lowest: f32) -> f32 {
    lowest + FIRST_LAYER_MM
}

/// Whether a surface facing `normal`, at `height`, is an overhang.
pub fn overhangs(normal: Vec3, height: f32, limit: f32, plate: f32) -> bool {
    height > plate && -normal.normalize_or_zero().y > limit
}

/// How much of a mesh overhangs.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Overhangs {
    /// The area that overhangs, in the mesh's units squared.
    pub area: f32,
    /// The area of the whole mesh.
    pub total: f32,
}

impl Overhangs {
    /// The two added to another body's.
    pub fn and(self, other: Self) -> Self {
        Self { area: self.area + other.area, total: self.total + other.total }
    }

    /// The share of the surface that overhangs, 0 to 1.
    pub fn share(&self) -> f32 {
        if self.total > 0.0 { self.area / self.total } else { 0.0 }
    }

    /// One line for a status bar, for a mesh in millimetres.
    pub fn summary(&self, steepest_degrees: f32) -> String {
        if self.area <= 0.0 {
            return format!("nothing overhangs past {steepest_degrees:.0} degrees");
        }
        format!(
            "{:.1} cm² overhangs past {steepest_degrees:.0} degrees, {:.0}% of the surface",
            self.area / 100.0,
            self.share() * 100.0
        )
    }
}

impl Volume {
    /// The height of the lowest point of the surface, or `None` when there
    /// is no surface.
    ///
    /// [`Volume::surface_bounds`] is not this. It bounds the voxels that
    /// are near the surface, which reach up to three voxels under it, and a
    /// plate put three voxels too low is under the whole model: every flat
    /// underside resting on the bed would be called an overhang. (That was
    /// built first, and a test of where the plate is found it.)
    ///
    /// So the lowest layer of those voxels is read for how far each is from
    /// the surface. A voxel is never nearer the surface than it is under
    /// the lowest point of it, and the one straight under that point is
    /// exactly that far, so the least distance in the layer is the distance
    /// from the layer up to the lowest point. (Any layer under that point
    /// and near enough to it gives the same answer: a mutant that read the
    /// next layer up survived the tests, and was right to.)
    ///
    /// The cost is `surface_bounds`' own, a scan of every dense brick, and
    /// one layer of lookups on top: a user-action cost, not a frame's.
    pub fn lowest_point(&self) -> Option<f32> {
        let (low, high) = self.surface_bounds()?;
        let size = self.voxel_size();
        let cell = |world: f32| (world / size).round() as i32;
        let layer = cell(low.y);
        let nearest = (cell(low.x)..=cell(high.x))
            .flat_map(|x| (cell(low.z)..=cell(high.z)).map(move |z| IVec3::new(x, layer, z)))
            // A voxel far from any surface reads as far outside, which is
            // never the least of a layer that holds a voxel near one.
            .map(|voxel| self.sample_voxel(voxel))
            .reduce(f32::min)?;
        Some(layer as f32 * size + nearest * size)
    }
}

impl ExportMesh {
    /// The lowest point of the mesh, or `None` when it has none.
    pub fn lowest(&self) -> Option<f32> {
        self.positions.iter().map(|position| position.y).reduce(f32::min)
    }

    /// How much of this mesh overhangs, a triangle at a time: each one by
    /// the way it faces and the height of its middle.
    ///
    /// `limit` is [`limit`]'s and `plate` is [`plate`]'s. The plate is
    /// handed in and not read off this mesh because a document's is the
    /// lowest point of all its bodies, and this is one of them.
    pub fn overhangs(&self, limit: f32, plate: f32) -> Overhangs {
        let mut found = Overhangs::default();
        for triangle in &self.triangles {
            let [a, b, c] = triangle.map(|vertex| self.positions[vertex as usize]);
            let facing = (b - a).cross(c - a);
            let area = facing.length() / 2.0;
            found.total += area;
            if overhangs(facing, (a.y + b.y + c.y) / 3.0, limit, plate) {
                found.area += area;
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::Volume;

    fn ball(radius: f32) -> ExportMesh {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, radius);
        volume.export_mesh().0
    }

    /// The rule, on surfaces whose lean is known: a wall is never an
    /// overhang, a ceiling always is, and a surface leaning past the limit
    /// is one only when it faces DOWN.
    #[test]
    fn a_surface_overhangs_when_it_leans_past_the_limit_and_faces_down() {
        // A surface leaning `lean` degrees from upright, facing down or up.
        let facing = |lean: f32, down: bool| {
            let lean = lean.to_radians();
            Vec3::new(lean.cos(), if down { -lean.sin() } else { lean.sin() }, 0.0)
        };
        let at_45 = limit(45.0);
        assert!((at_45 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        for (lean, expected) in
            [(0.0, false), (30.0, false), (44.0, false), (46.0, true), (90.0, true)]
        {
            assert_eq!(overhangs(facing(lean, true), 5.0, at_45, 0.0), expected, "{lean} degrees");
            assert!(!overhangs(facing(lean, false), 5.0, at_45, 0.0), "{lean} degrees, facing up");
        }
        // A printer that manages more calls less of it an overhang.
        assert!(overhangs(facing(50.0, true), 5.0, limit(45.0), 0.0));
        assert!(!overhangs(facing(50.0, true), 5.0, limit(60.0), 0.0));
        // Not on the plate, however it faces, and a normal of any length
        // is the same normal.
        assert!(!overhangs(Vec3::NEG_Y, 0.2, at_45, plate(0.0)));
        assert!(overhangs(Vec3::NEG_Y, 0.4, at_45, plate(0.0)));
        assert!(overhangs(Vec3::NEG_Y * 40.0, 5.0, at_45, 0.0));
        assert!(!overhangs(Vec3::ZERO, 5.0, at_45, 0.0), "a normal of nothing faces nowhere");
        // The limit is kept to the angles there are.
        assert_eq!((limit(-10.0), limit(400.0)), (0.0, 1.0));
    }

    /// A ball has an overhang whose size is known: the cap below the
    /// latitude where it leans past the limit, which is the share
    /// `(1 - sin(steepest)) / 2` of the sphere.
    #[test]
    fn a_balls_overhang_is_the_cap_under_it() {
        let mesh = ball(20.0);
        let lowest = mesh.lowest().expect("a ball has a lowest point");
        assert!((lowest + 20.0).abs() < 0.3, "the ball's lowest point is at {lowest}");
        for steepest in [30.0_f32, 45.0, 60.0] {
            let found = mesh.overhangs(limit(steepest), plate(lowest));
            let expected = (1.0 - steepest.to_radians().sin()) / 2.0;
            assert!(
                (found.share() - expected).abs() < 0.01,
                "past {steepest} degrees {:.3} of a ball overhangs, and {expected:.3} should",
                found.share()
            );
        }
        let sphere = 4.0 * std::f32::consts::PI * 20.0 * 20.0;
        let found = mesh.overhangs(limit(45.0), plate(lowest));
        assert!((found.total / sphere - 1.0).abs() < 0.02, "the ball's area is {}", found.total);

        // The plate takes nothing off but what stands on it. Raised to the
        // ball's middle, only the half of the cap above the middle is
        // left, and that is none of it.
        let none = mesh.overhangs(limit(45.0), 0.0);
        assert_eq!(none.area, 0.0, "an overhang above the middle of a ball");
        // Control: with no plate at all the very bottom counts too. What
        // the plate takes off is the cap one first layer tall, whose area
        // is the ball's girth times that height.
        let whole = mesh.overhangs(limit(45.0), f32::MIN);
        let cap = 2.0 * std::f32::consts::PI * 20.0 * FIRST_LAYER_MM;
        let taken = whole.area - found.area;
        assert!((taken / cap - 1.0).abs() < 0.2, "the plate took {taken} mm² off, not {cap}");
    }

    /// A box standing on the plate has no overhang: its sides are upright
    /// and its underside is on the bed. Lifted off the plate, its
    /// underside is all of it.
    #[test]
    fn a_box_on_the_plate_has_no_overhang_and_one_in_the_air_has_its_underside() {
        let mut volume = Volume::new(0.5);
        let half = Vec3::new(10.0, 6.0, 8.0);
        let reach = half + Vec3::splat(crate::brick::NARROW_BAND);
        volume.edit_box(-reach, reach, |position, _| {
            let outside = position.abs() - half;
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / 0.5).clamp(crate::brick::INSIDE, crate::brick::OUTSIDE)
        });
        let mesh = volume.export_mesh().0;
        let lowest = mesh.lowest().expect("a box has a lowest point");
        let standing = mesh.overhangs(limit(45.0), plate(lowest));
        // Its rounded lower edges lean past 45 degrees for a voxel or so.
        assert!(standing.share() < 0.03, "{:.3} of a box on the plate overhangs", standing.share());

        let in_the_air = mesh.overhangs(limit(45.0), plate(lowest - 10.0));
        let underside = (2.0 * half.x) * (2.0 * half.z);
        assert!(
            (in_the_air.area / underside - 1.0).abs() < 0.1,
            "the underside of a box in the air is {} mm² and {} overhangs",
            underside,
            in_the_air.area
        );
        assert_eq!(in_the_air.total, standing.total);
    }

    /// The lowest point of the surface is found to a fraction of a voxel,
    /// where the bounds of the voxels near it are several voxels under it.
    #[test]
    fn the_lowest_point_of_the_surface_is_found_and_not_the_voxels_under_it() {
        for (voxel, radius) in [(0.5, 20.0), (0.25, 9.0), (1.0, 17.3)] {
            let mut volume = Volume::new(voxel);
            volume.seed_sphere(Vec3::new(3.0, 1.5, -2.0), radius);
            let lowest = volume.lowest_point().expect("a ball has a lowest point");
            let wanted = 1.5 - radius;
            assert!(
                (lowest - wanted).abs() < 0.15 * voxel,
                "at a voxel of {voxel} mm the lowest point of a ball is {lowest}, not {wanted}"
            );
            // Control: the bounds this is not are further off than the
            // first layer the plate allows for, which is why this exists.
            let (low, _) = volume.surface_bounds().expect("a ball has bounds");
            assert!(
                wanted - low.y > 1.5 * voxel,
                "the bounds are only {} mm under",
                wanted - low.y
            );
            // And the mesh, which is the other way to ask, agrees.
            let mesh = volume.export_mesh().0;
            let meshed = mesh.lowest().expect("the mesh has a lowest point");
            assert!((meshed - lowest).abs() < 0.3 * voxel, "{meshed} against {lowest}");
        }
        assert_eq!(Volume::new(0.5).lowest_point(), None);
    }

    /// The line for the status bar says how much, in the units a person
    /// thinks of a print in, and says so when there is none.
    #[test]
    fn the_summary_says_how_much_or_that_there_is_none() {
        let some = Overhangs { area: 1234.0, total: 12340.0 };
        assert_eq!(some.summary(45.0), "12.3 cm² overhangs past 45 degrees, 10% of the surface");
        assert_eq!(Overhangs::default().summary(60.0), "nothing overhangs past 60 degrees");
        assert_eq!(Overhangs::default().share(), 0.0);
        let both = some.and(Overhangs { area: 6.0, total: 60.0 });
        assert_eq!((both.area, both.total), (1240.0, 12400.0));
        assert_eq!(ExportMesh::default().lowest(), None);
        assert_eq!(ExportMesh::default().overhangs(0.5, 0.0), Overhangs::default());
    }
}
