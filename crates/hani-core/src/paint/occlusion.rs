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
//!
//! How strongly a body pools is its own setting ([`Volume::set_pooling`]),
//! which the application sets on every body from one number in Paint's
//! LAYERS block, asked for by the maintainer on 2026-11-25 ("we just need
//! a AO intensity slider for the paint engine").

use glam::{IVec3, Vec3};

use crate::volume::Volume;

/// How strongly a wash or a contrast pools, unless it is told otherwise:
/// what the occlusion is multiplied by before it is a crease. At 1, the
/// first value, the maintainer found the pooling too faint for a contrast
/// paint to work ("The effect of AO needs to be higher").
pub const POOLING: f32 = 1.5;

/// How strongly it may be set to pool, from none to twice the usual.
pub const POOLING_RANGE: std::ops::RangeInclusive<f32> = 0.0..=3.0;

/// The crease a point is, -1 a full edge to 1 shut in all round, from how
/// shut in it is (`occlusion`, 0..=1), the field's own edge there (`edge`,
/// -1..=0) and how strongly the paint pools (`pooling`).
pub fn crease_of(occlusion: f32, edge: f32, pooling: f32) -> f32 {
    (occlusion * 1.6 * pooling.max(0.0) + edge.min(0.0)).clamp(-1.0, 1.0)
}

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
        crease_of(self.occlusion_at(voxel), self.edge_at(voxel), self.pooling())
    }

    /// How much of an edge the surface is at `voxel`, -1 a full edge to 0
    /// none: the field's Laplacian there, which sees what stands out a
    /// voxel or two and not a recess.
    pub fn edge_at(&self, voxel: IVec3) -> f32 {
        let sample = |offset: IVec3| self.sample_voxel(voxel + offset);
        crate::paint::layers::crease_from(
            sample(IVec3::ZERO),
            [
                sample(IVec3::X),
                sample(-IVec3::X),
                sample(IVec3::Y),
                sample(-IVec3::Y),
                sample(IVec3::Z),
                sample(-IVec3::Z),
            ],
        )
        .min(0.0)
    }

    /// How strongly a wash or a contrast pools on this body.
    pub fn pooling(&self) -> f32 {
        self.pooling
    }

    /// Pool a wash or a contrast this strongly, within [`POOLING_RANGE`]:
    /// every brick a pooling layer has paint in is put together again.
    pub fn set_pooling(&mut self, pooling: f32) {
        let pooling = pooling.clamp(*POOLING_RANGE.start(), *POOLING_RANGE.end());
        if pooling == self.pooling {
            return;
        }
        self.pooling = pooling;
        if self.layers_read_the_shape() {
            self.restack();
        }
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

    /// The more a body pools, the darker a wash is at the neck against the
    /// top of a ball; at none, only the edge the top is tells them apart.
    /// Setting it puts the stack together again.
    #[test]
    fn pooling_more_darkens_the_recess_more() {
        use crate::paint::layers::Blend;
        let mut volume = two_balls();
        let (lo, hi) = (IVec3::splat(-60), IVec3::splat(60));
        volume.edit_paint(lo, hi, |_, _, _, _| [230, 230, 230, 255]);
        volume.add_layer("Wash", Blend::Wash).expect("room");
        volume.edit_paint(lo, hi, |_, _, _, _| [60, 40, 20, 255]);
        let on_surface = |volume: &Volume, from: Vec3, way: Vec3| {
            let mut at = from;
            while volume.sample_world(at) < 0.0 {
                at += way * 0.05;
            }
            (at / volume.voxel_size()).round().as_ivec3()
        };
        // Across the surface from the neck out to the top of one ball: the
        // neck is shut in at any strength, and how far out of it the wash
        // still pools is what the strength changes.
        let line: Vec<IVec3> = (0..=18)
            .map(|step| on_surface(&volume, Vec3::new(step as f32 * 0.2, 0.0, 0.0), Vec3::Y))
            .collect();
        let top = *line.last().expect("a line");
        let mut darker = Vec::new();
        for pooling in [0.0, 1.0, POOLING, 3.0] {
            volume.set_pooling(pooling);
            let mut dirty = Vec::new();
            volume.take_dirty(&mut dirty);
            let lightness = |voxel: IVec3| {
                let [r, g, b, _] = volume.display_paint().at(voxel);
                i32::from(r) + i32::from(g) + i32::from(b)
            };
            darker.push(line.iter().map(|voxel| lightness(top) - lightness(*voxel)).sum::<i32>());
        }
        assert!(
            darker.windows(2).all(|pair| pair[1] > pair[0]),
            "the wash is not darker along the line with more pooling: {darker:?}"
        );
        assert_eq!(volume.pooling(), 3.0);
        volume.set_pooling(9.0);
        assert_eq!(volume.pooling(), *POOLING_RANGE.end());
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
