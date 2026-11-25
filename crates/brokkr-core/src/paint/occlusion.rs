// SPDX-License-Identifier: AGPL-3.0-only

//! How shut in the surface is at a voxel: the ambient occlusion a wash or
//! a contrast pools by.
//!
//! The first pooling read the field's Laplacian at the voxel, which sees a
//! crease a voxel or two wide and nothing broader: the maintainer found
//! that washes and contrasts did not pool in the recesses of a figure.
//! This asks the surface the way a real wash finds it: from a point just
//! off the surface, six rays across the hemisphere it faces (one straight
//! out, five leaning round it), each stepped out to a few millimetres;
//! a ray that meets solid is shut in, the sooner the more. The answer is
//! 0 for a point open to the sky and 1 for one shut in all round.
//!
//! The field's sign is right everywhere, out of the narrow band too, so a
//! step far from the surface still says solid or not. Edges are not this
//! function's: an edge is as open as a flat, and is told apart by the
//! Laplacian, which [`Volume::crease_at`] puts beside it.

use glam::{IVec3, Vec3};

use crate::volume::Volume;

/// How far a ray is stepped, in millimetres, each step twice the last.
const FIRST_STEP_MM: f32 = 0.5;
const STEPS: usize = 4;

impl Volume {
    /// How shut in the surface is at `voxel`, 0 open to 1 shut in all
    /// round. See the module documentation.
    pub fn occlusion_at(&self, voxel: IVec3) -> f32 {
        let sample = |at: IVec3| self.sample_voxel(at);
        let normal = Vec3::new(
            sample(voxel + IVec3::X) - sample(voxel - IVec3::X),
            sample(voxel + IVec3::Y) - sample(voxel - IVec3::Y),
            sample(voxel + IVec3::Z) - sample(voxel - IVec3::Z),
        )
        .normalize_or_zero();
        if normal == Vec3::ZERO {
            return 0.0;
        }
        // Just off the surface, whichever side of it the voxel is.
        let start = voxel.as_vec3() - normal * sample(voxel) + normal * 1.0;
        let (across, along) = normal.any_orthonormal_pair();
        let first = (FIRST_STEP_MM / self.voxel_size()).max(1.0);
        let mut shut = 0.0;
        let rays = 6;
        for ray in 0..rays {
            let direction = if ray == 0 {
                normal
            } else {
                let turn = (ray - 1) as f32 * std::f32::consts::TAU / 5.0;
                (normal * 0.55 + (across * turn.cos() + along * turn.sin()) * 0.85).normalize()
            };
            let mut step = first;
            for at in 0..STEPS {
                let point = start + direction * step;
                if sample(point.round().as_ivec3()) < 0.0 {
                    // Shut in near is shut in more.
                    shut += 1.0 - at as f32 / STEPS as f32 * 0.6;
                    break;
                }
                step *= 2.0;
            }
        }
        (shut / rays as f32).clamp(0.0, 1.0)
    }

    /// How much of a crease the surface is at `voxel`, -1 for a full edge
    /// to 1 for a point shut in all round: the occlusion for a recess, and
    /// the Laplacian for an edge, which the occlusion cannot tell from a
    /// flat. What a wash and a contrast pool by.
    pub fn crease_at(&self, voxel: IVec3) -> f32 {
        let sample = |offset: IVec3| self.sample_voxel(voxel + offset);
        let local = crate::paint::layers::crease_from(
            sample(IVec3::ZERO),
            [
                sample(IVec3::X),
                sample(-IVec3::X),
                sample(IVec3::Y),
                sample(-IVec3::Y),
                sample(IVec3::Z),
                sample(-IVec3::Z),
            ],
        );
        (self.occlusion_at(voxel) * 1.6 + local.min(0.0)).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two balls side by side: where they meet is shut in, and the far side
    /// of either is open.
    fn two_balls() -> Volume {
        let mut volume = Volume::new(0.25);
        for centre in [Vec3::new(-3.6, 0.0, 0.0), Vec3::new(3.6, 0.0, 0.0)] {
            let mut one = Volume::new(0.25);
            one.seed_sphere(centre, 4.0);
            volume.begin_stroke();
            volume.union_from(&one);
            let _ = volume.end_stroke();
        }
        volume
    }

    /// The surface near where two balls meet is far more shut in than the
    /// top of either, which is open to the sky.
    #[test]
    fn where_two_balls_meet_is_shut_in() {
        let volume = two_balls();
        let on_surface = |from: Vec3, way: Vec3| {
            let mut at = from;
            while volume.sample_world(at) < 0.0 {
                at += way * 0.05;
            }
            (at / volume.voxel_size()).round().as_ivec3()
        };
        let neck = on_surface(Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
        let top = on_surface(Vec3::new(3.6, 0.0, 0.0), Vec3::Y);
        let (shut, open) = (volume.occlusion_at(neck), volume.occlusion_at(top));
        assert!(shut > 0.3, "the neck is only {shut} shut in");
        assert!(open < 0.1, "the top of a ball is {open} shut in");
        assert!(volume.crease_at(neck) > volume.crease_at(top));
    }

    /// A dark wash over a light coat is seen darker where the two balls
    /// meet than on top of either: it pools by the occlusion.
    #[test]
    fn a_wash_pools_where_the_balls_meet() {
        use crate::paint::layers::Blend;
        let mut volume = two_balls();
        let (lo, hi) = (IVec3::splat(-60), IVec3::splat(60));
        volume.edit_paint(lo, hi, |_, _, _, _| [230, 230, 230, 255]);
        volume.add_layer("Wash", Blend::Wash).expect("room");
        volume.edit_paint(lo, hi, |_, _, _, _| [20, 15, 10, 255]);
        let mut dirty = Vec::new();
        volume.take_dirty(&mut dirty);
        let on_surface = |from: Vec3, way: Vec3| {
            let mut at = from;
            while volume.sample_world(at) < 0.0 {
                at += way * 0.05;
            }
            (at / volume.voxel_size()).round().as_ivec3()
        };
        let lightness = |voxel: IVec3| {
            let [r, g, b, _] = volume.display_paint().at(voxel);
            u32::from(r) + u32::from(g) + u32::from(b)
        };
        let neck = lightness(on_surface(Vec3::ZERO, Vec3::Y));
        let top = lightness(on_surface(Vec3::new(3.6, 0.0, 0.0), Vec3::Y));
        assert!(neck + 120 < top, "the wash is {neck} at the neck and {top} on top");
    }

    /// A single ball is open everywhere.
    #[test]
    fn a_ball_is_open_all_round() {
        let mut volume = Volume::new(0.25);
        volume.seed_sphere(Vec3::ZERO, 4.0);
        for voxel in [IVec3::new(16, 0, 0), IVec3::new(0, -16, 0), IVec3::new(0, 0, 16)] {
            assert!(volume.occlusion_at(voxel) < 0.1, "{voxel}");
        }
    }
}
