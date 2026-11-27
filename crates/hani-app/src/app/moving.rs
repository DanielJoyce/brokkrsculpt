// SPDX-License-Identifier: AGPL-3.0-only

//! A body the transform gizmo is dragging, drawn where the drag has it.
//!
//! Found by the maintainer on 2026-11-27: "Poor feedback when using gizmo
//! with transform->inserted sub meshs, no kind of realtime update to see
//! them being rotated / moved". While a handle was held only a wireframe
//! box followed the pointer; the body itself stayed put until the release
//! rebuilt its field at the new placement (`Hani::rebake_gizmo`), which is
//! one resample a gesture and is kept that way.
//!
//! Now the body's mesh as it stands is drawn through the drag's map, on
//! the graphics card (`hani_gpu::spread`), every frame of the drag. What is
//! on screen while a handle is held was built at the placement pinned when
//! the button went down, so the map is the placement now after the pinned
//! one undone. On the release the field is rebuilt there and the map is
//! dropped.

use glam::Mat4;
use hani_core::NodeId;
use hani_core::similarity::Similarity;

use crate::gizmo::Gizmo;

/// The map the body under a held gizmo is drawn through, if a handle is
/// held.
pub(super) fn drawn(gizmo: Option<&Gizmo>) -> Option<(NodeId, Mat4)> {
    let gizmo = gizmo?;
    gizmo.grabbed?;
    let map = matrix(gizmo.placement) * matrix(gizmo.pinned).inverse();
    // A scale of nought has no inverse; the gizmo never lets one through,
    // and the body is drawn where it is if one ever does.
    map.is_finite().then_some((gizmo.body, map))
}

fn matrix(placement: Similarity) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        placement.scale,
        placement.rotation,
        placement.translation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    /// Held, a point of the mesh as it was built at the pinned placement
    /// is drawn where the placement now puts it; let go, nothing is.
    #[test]
    fn the_map_takes_the_pinned_placement_to_the_live_one() {
        let mut gizmo = Gizmo::new(NodeId(2), Vec3::splat(-1.0), Vec3::splat(1.0), 4.0);
        gizmo.pinned = Similarity {
            rotation: Quat::from_rotation_y(0.4),
            scale: Vec3::new(1.5, 1.0, 0.8),
            translation: Vec3::new(3.0, 0.0, -1.0),
        };
        gizmo.placement = gizmo.pinned.then(Similarity {
            rotation: Quat::from_rotation_x(0.7),
            scale: Vec3::splat(1.2),
            translation: Vec3::new(0.0, 2.0, 0.0),
        });
        assert!(drawn(Some(&gizmo)).is_none(), "drawn moved with no handle held");
        gizmo.grabbed = Some(crate::gizmo::Handle::Uniform);
        let (body, map) = drawn(Some(&gizmo)).expect("a held handle draws the body moved");
        assert_eq!(body, NodeId(2));
        for base in [Vec3::ZERO, Vec3::new(1.0, -0.5, 0.25), Vec3::new(-0.7, 0.9, 0.3)] {
            let shown = gizmo.pinned.transform_point(base);
            let want = gizmo.placement.transform_point(base);
            let got = map.transform_point3(shown);
            assert!(got.distance(want) < 1.0e-4, "{base}: drawn at {got}, wanted {want}");
        }
        // Pressed and not yet moved: drawn where it is.
        gizmo.placement = gizmo.pinned;
        let (_, map) = drawn(Some(&gizmo)).unwrap();
        assert!(map.abs_diff_eq(Mat4::IDENTITY, 1.0e-5));
        assert!(drawn(None).is_none());
    }
}
