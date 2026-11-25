// SPDX-License-Identifier: AGPL-3.0-only

//! The airbrush: paint sprayed along the view rather than laid by a
//! stamp. Stage 3 of `docs/PAINT-RACK.md`, section 5.
//!
//! One call is one moment of spraying. The application calls it every
//! frame the button is held, with how much paint that frame's time and
//! the pen's pressure let through, so paint builds where the nozzle
//! dwells and a quick pass barely tints.
//!
//! A voxel takes `amount * spot * facing * free` of the pigment, where:
//!
//! - `spot` is a Gaussian on the voxel's distance from the spray's axis,
//!   the line from the nozzle through the point aimed at, with a standard
//!   deviation of half the spot's radius; and, with overspray on, a faint
//!   wide halo added, a twentieth as strong with twice the reach;
//! - `facing` is how squarely the surface there faces the nozzle: the dot
//!   of its normal with the way back along the spray, nothing for a
//!   surface turned away;
//! - `free` is what the mask leaves free, so a stencil is the mask.
//!
//! Only voxels within a spot's radius of the point aimed at, along the
//! axis, are reached: the surface the spray meets, and not a surface
//! further back along the same line that a real spray could not reach
//! through it. Paint is laid as the colour brush lays it,
//! [`crate::paint::stroke::laid`], so a Glaze or an Erase sprays too.

use glam::{IVec3, Vec3};
use rustc_hash::FxHashMap;

use super::field::PaintField;
use super::stroke::{Pigment, laid};
use crate::mask::PROTECTED;
use crate::volume::Volume;

/// What is in the airbrush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spray {
    pub pigment: Pigment,
    /// The spot's radius on the surface, world units: where the spray is
    /// about an eighth as strong as at its middle.
    pub radius: f32,
    /// A faint wide halo round the spot.
    pub overspray: bool,
}

impl Spray {
    /// How far from the axis the spray reaches at all.
    fn reach(&self) -> f32 {
        if self.overspray { self.radius * 2.0 } else { self.radius * 1.25 }
    }

    /// How strong the spray is at `off` from its axis, 0..=1.
    pub fn spot(&self, off: f32) -> f32 {
        let sigma = self.radius * 0.5;
        let core = (-(off * off) / (2.0 * sigma * sigma)).exp();
        if !self.overspray {
            return core;
        }
        let wide = self.radius;
        (core + 0.05 * (-(off * off) / (2.0 * wide * wide)).exp()).min(1.0)
    }
}

impl Volume {
    /// Spray `amount` of `spray` at `at`, a point on the surface, along
    /// `direction`, from the nozzle toward the surface. `amount` is the
    /// share of a full coat the middle of the spot takes, before the
    /// pigment's own opacity. Returns how many voxels took paint.
    pub fn spray(&mut self, at: Vec3, direction: Vec3, spray: &Spray, amount: f32) -> usize {
        let direction = direction.normalize_or_zero();
        if direction == Vec3::ZERO || amount <= 0.0 || spray.radius <= 0.0 {
            return 0;
        }
        let voxel_size = self.voxel_size();
        let reach = spray.reach();
        let lo = ((at - Vec3::splat(reach)) / voxel_size).floor().as_ivec3();
        let hi = ((at + Vec3::splat(reach)) / voxel_size).ceil().as_ivec3();

        // What each voxel the spray reaches takes, worked out first: the
        // facing reads the field, which the edit below holds.
        let mut takes: FxHashMap<IVec3, f32> = FxHashMap::default();
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = IVec3::new(x, y, z);
                    let position = voxel.as_vec3() * voxel_size;
                    let offset = position - at;
                    let along = offset.dot(direction);
                    if along.abs() > spray.radius {
                        continue;
                    }
                    let off = (offset - direction * along).length();
                    if off > reach {
                        continue;
                    }
                    if !PaintField::paintable(self.sample_voxel(voxel)) {
                        continue;
                    }
                    let normal = self.gradient_world(position).normalize_or_zero();
                    let facing = (-normal.dot(direction)).max(0.0);
                    let take = amount * spray.spot(off) * facing;
                    if take > 1e-4 {
                        takes.insert(voxel, take.min(1.0));
                    }
                }
            }
        }
        if takes.is_empty() {
            return 0;
        }
        let mut laid_on = 0;
        let pigment = spray.pigment;
        self.edit_paint_over(lo, hi, |voxel, _, held, _, protection| {
            let Some(take) = takes.get(&voxel) else {
                return held;
            };
            let free = 1.0 - f32::from(protection) / f32::from(PROTECTED);
            let take = take * free;
            if take <= 0.0 {
                return held;
            }
            let after = laid(held, pigment, take);
            if after != held {
                laid_on += 1;
            }
            after
        });
        laid_on
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::field::BARE;
    use crate::paint::stroke::Mode;
    use proptest::prelude::*;

    const RED: Pigment = Pigment { colour: [220, 20, 20], opacity: 1.0, mode: Mode::Paint };

    fn ball() -> Volume {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        volume
    }

    /// The voxel of the surface on the +x side, at the middle of a spray
    /// from +x.
    fn middle() -> IVec3 {
        IVec3::new(20, 0, 0)
    }

    fn spray(radius: f32) -> Spray {
        Spray { pigment: RED, radius, overspray: false }
    }

    /// Held still, a spot gets darker the longer it is held, and a quick
    /// pass barely tints.
    #[test]
    fn paint_builds_where_the_nozzle_dwells() {
        let mut quick = ball();
        quick.spray(Vec3::new(10.0, 0.0, 0.0), -Vec3::X, &spray(3.0), 0.05);
        let mut held = ball();
        for _ in 0..30 {
            held.spray(Vec3::new(10.0, 0.0, 0.0), -Vec3::X, &spray(3.0), 0.05);
        }
        let (a, b) = (quick.paint().at(middle()), held.paint().at(middle()));
        assert!(a[3] > 0 && a[3] < 40, "a quick pass laid {a:?}");
        assert!(b[3] > 200, "thirty frames held laid only {b:?}");
    }

    /// Nothing lands on the far side of the ball, which faces away and is
    /// behind the surface the spray meets.
    #[test]
    fn the_far_side_takes_nothing() {
        let mut volume = ball();
        for _ in 0..20 {
            volume.spray(Vec3::new(10.0, 0.0, 0.0), -Vec3::X, &spray(4.0), 0.2);
        }
        assert_eq!(volume.paint().at(-middle()), BARE);
        assert_ne!(volume.paint().at(middle()), BARE);
    }

    /// The mask holds the spray off, as it holds a brush.
    #[test]
    fn the_mask_is_a_stencil() {
        let mut volume = ball();
        let (lo, hi) = (IVec3::splat(-30), IVec3::splat(30));
        volume.edit_mask(lo, hi, |_, at, _| if at.y > 0.0 { PROTECTED } else { 0 });
        for _ in 0..20 {
            volume.spray(Vec3::new(10.0, 0.0, 0.0), -Vec3::X, &spray(4.0), 0.2);
        }
        assert_eq!(volume.paint().at(IVec3::new(19, 4, 0)), BARE, "paint under the stencil");
        assert_ne!(volume.paint().at(IVec3::new(19, -4, 0)), BARE);
    }

    /// Overspray reaches further than the spot without it.
    #[test]
    fn overspray_reaches_further() {
        let far = IVec3::new(18, 9, 0);
        let mut clean = ball();
        let mut misty = ball();
        for _ in 0..30 {
            clean.spray(Vec3::new(10.0, 0.0, 0.0), -Vec3::X, &spray(3.0), 0.3);
            misty.spray(
                Vec3::new(10.0, 0.0, 0.0),
                -Vec3::X,
                &Spray { overspray: true, ..spray(3.0) },
                0.3,
            );
        }
        assert_eq!(clean.paint().at(far), BARE);
        assert_ne!(misty.paint().at(far), BARE);
    }

    proptest! {
        /// The spot is strongest at its middle and never stronger further
        /// out, with or without overspray.
        #[test]
        fn the_spot_falls_away_from_its_middle(
            radius in 0.2f32..20.0,
            a in 0.0f32..1.0,
            b in 0.0f32..1.0,
            overspray in proptest::bool::ANY,
        ) {
            let spray = Spray { pigment: RED, radius, overspray };
            let (near, far) = (a.min(b) * radius * 2.0, a.max(b) * radius * 2.0);
            prop_assert!(spray.spot(near) >= spray.spot(far));
            prop_assert!(spray.spot(0.0) <= 1.0 && spray.spot(far) >= 0.0);
        }
    }
}
