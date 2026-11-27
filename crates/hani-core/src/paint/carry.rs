// SPDX-License-Identifier: AGPL-3.0-only

//! Paint carried onto the surface a sculpting stroke makes.
//!
//! Asked for by the maintainer on 2026-11-27: "if I am sculpting over a
//! painted voxel, try as much as possible to have the paint carry to the
//! new voxels instead of getting the unpainted surface color".
//!
//! Paint is held a voxel at a time through the narrow band (`field.rs`), so
//! a stroke that moves the surface less than the band's depth keeps it: the
//! voxels the surface moves onto were painted with it. Draw a stroke out
//! further than that, or carve deeper, and the surface reaches voxels the
//! band never held, which are bare, and the clay shows.
//!
//! So after a sculpting stroke's stamps, each voxel that has come into the
//! band since the stroke began and holds no paint takes the paint of the
//! nearest painted voxel along the surface's normal, inward or outward, in
//! every layer. Only a voxel new to the band, so the edge of a painted
//! patch the stroke passes beside does not creep sideways: a voxel that was
//! in the band before the stroke keeps what it held, bare or not. Along the
//! normal only, so the paint comes from the column of the band the new
//! voxel stands on, which is where the material it is made of came from.

use glam::{IVec3, Vec3};
use rustc_hash::FxHashMap;

use super::field::{BARE, PaintField, Rgba};
use crate::brick::{Brick, BrickCoord, NARROW_BAND};
use crate::volume::Volume;

/// How far along the normal a new voxel looks for paint, in voxels: the
/// band's depth either way and one over, so a voxel at the band's far side
/// finds the one the stroke started from.
const REACH: i32 = NARROW_BAND as i32 * 2 + 1;

impl Volume {
    /// Carry paint onto the voxels in `lo..=hi` that have come into the
    /// band since the open stroke began and hold none. See the module
    /// documentation. The number of voxels given paint, over every layer.
    pub fn carry_paint_onto_new_surface(&mut self, lo: IVec3, hi: IVec3) -> usize {
        if self.layer_fields().iter().all(|field| field.is_empty()) {
            return 0;
        }
        // Which voxels are new to the band, and which way is out at each,
        // once for every layer: the field is the same under all of them. A
        // brick with no paint in it or beside it in any layer has none to
        // be given, the search reaching no further than a brick, and is
        // passed over: on a body painted in one place, a stroke elsewhere
        // costs this a look at each brick's neighbours and nothing more.
        let fields = self.layer_fields();
        let painted_near = |coord: BrickCoord| {
            (-1..=1).any(|z| {
                (-1..=1).any(|y| {
                    (-1..=1).any(|x| {
                        let near = BrickCoord(coord.0 + IVec3::new(x, y, z));
                        fields.iter().any(|field| field.has_brick(near))
                    })
                })
            })
        };
        let mut new: Vec<(IVec3, Vec3)> = Vec::new();
        let (first, last) = (BrickCoord::containing(lo).0, BrickCoord::containing(hi).0);
        for bz in first.z..=last.z {
            for by in first.y..=last.y {
                for bx in first.x..=last.x {
                    let coord = BrickCoord::new(bx, by, bz);
                    // A brick saturated throughout now has no voxel in the
                    // band: most of a stamp's box, and passed over before
                    // its paint is asked after. Measured on 2026-11-27 with
                    // `what_carrying_paint_costs`: see `docs/PERFORMANCE.md`.
                    let Some(brick @ Brick::Dense(_)) = self.brick(coord) else {
                        continue;
                    };
                    if !painted_near(coord) {
                        continue;
                    }
                    let origin = coord.origin();
                    let from = lo.max(origin);
                    let to = hi.min(coord.max_voxel());
                    for z in from.z..=to.z {
                        for y in from.y..=to.y {
                            for x in from.x..=to.x {
                                let voxel = IVec3::new(x, y, z);
                                let local = voxel - origin;
                                let now =
                                    brick.get(local.x as usize, local.y as usize, local.z as usize);
                                if !PaintField::paintable(now)
                                    || PaintField::paintable(self.sample_voxel_before(voxel))
                                {
                                    continue;
                                }
                                let out = self.gradient_of_voxel(voxel);
                                if out != Vec3::ZERO {
                                    new.push((voxel, out));
                                }
                            }
                        }
                    }
                }
            }
        }
        if new.is_empty() {
            return 0;
        }
        let mut carried = 0;
        self.edit_each_layer(|volume, _| {
            let found = carried_in(volume.paint(), &new);
            if found.is_empty() {
                return;
            }
            carried += found.len();
            volume.edit_paint(lo, hi, |voxel, _, held, _| {
                if held[3] != 0 {
                    return held;
                }
                found.get(&voxel).copied().unwrap_or(held)
            });
        });
        carried
    }

    /// Which way is out at `voxel`, by central differences of the field,
    /// unit length; nought where the field is flat.
    fn gradient_of_voxel(&self, voxel: IVec3) -> Vec3 {
        let along = |axis: IVec3| self.sample_voxel(voxel + axis) - self.sample_voxel(voxel - axis);
        Vec3::new(along(IVec3::X), along(IVec3::Y), along(IVec3::Z)).normalize_or_zero()
    }
}

/// For each new voxel that holds no paint in `paint`, the paint of the
/// nearest painted voxel along its normal, the nearer side first at each
/// step, inward before outward at the same distance.
fn carried_in(paint: &PaintField, new: &[(IVec3, Vec3)]) -> FxHashMap<IVec3, Rgba> {
    let mut found = FxHashMap::default();
    for &(voxel, out) in new {
        if paint.at(voxel)[3] != 0 {
            continue;
        }
        'search: for step in 1..=REACH {
            for side in [-1.0_f32, 1.0] {
                let at = (voxel.as_vec3() + out * (side * step as f32)).round().as_ivec3();
                let held = paint.at(at);
                if held != BARE && held[3] != 0 {
                    found.insert(voxel, held);
                    break 'search;
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::{INSIDE, OUTSIDE};

    const VOXEL: f32 = 0.5;

    /// A slab whose top is the plane z = `top`, in millimetres.
    fn slab(top: f32) -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::new(12.0, 12.0, 12.0);
        volume.edit_box(-reach, reach, move |position, _| {
            ((position.z - top) / VOXEL).clamp(INSIDE, OUTSIDE)
        });
        volume
    }

    /// Raise the slab's top to `top` inside a square of half side `half`,
    /// in the open stroke.
    fn raise(volume: &mut Volume, top: f32, half: f32) {
        let reach = Vec3::new(12.0, 12.0, 12.0);
        volume.edit_box(-reach, reach, move |position, held| {
            if position.x.abs() <= half && position.y.abs() <= half {
                ((position.z - top) / VOXEL).clamp(INSIDE, OUTSIDE)
            } else {
                held
            }
        });
    }

    const RED: Rgba = [200, 30, 20, 255];

    /// The voxel on the surface over `(x, y)`, in a slab whose top is `top`.
    fn on_top(x: f32, y: f32, top: f32) -> IVec3 {
        (Vec3::new(x, y, top) / VOXEL).round().as_ivec3()
    }

    /// Paint on a slab, the top then raised by more than the band in the
    /// middle: the raised top is painted as the slab was, and the slab
    /// beside it, never in the stroke's new band, is left as it was.
    #[test]
    fn a_raised_surface_takes_the_paint_it_was_raised_from() {
        let mut volume = slab(0.0);
        let box_lo = IVec3::splat(-24);
        let box_hi = IVec3::splat(24);
        // Red where |x| < 6, through the band; bare elsewhere.
        volume.edit_paint(
            box_lo,
            box_hi,
            |_, position, _, _| {
                if position.x.abs() < 6.0 { RED } else { BARE }
            },
        );
        volume.begin_stroke();
        // Raised 4 mm, eight voxels, past the band's three.
        raise(&mut volume, 4.0, 4.0);
        assert_eq!(volume.paint().at(on_top(0.0, 0.0, 4.0)), BARE, "the fixture kept the paint");
        let carried = volume.carry_paint_onto_new_surface(box_lo, box_hi);
        assert!(carried > 0);
        // The raised top, its middle and near its edge: red.
        for (x, y) in [(0.0, 0.0), (3.5, -3.5), (-3.0, 2.0)] {
            assert_eq!(volume.paint().at(on_top(x, y, 4.0)), RED, "at ({x}, {y})");
        }
        // Beside the raised square, where the slab is as it was: as it was.
        assert_eq!(volume.paint().at(on_top(5.0, 8.0, 0.0)), RED);
        assert_eq!(volume.paint().at(on_top(9.0, 0.0, 0.0)), BARE);
        let _ = volume.end_stroke();
    }

    /// The stroke's new band takes paint only where there is paint under
    /// it: a raised square half over a painted stripe is painted over the
    /// stripe and bare beside it, and nothing new is painted past the
    /// stripe's edge.
    #[test]
    fn paint_is_carried_along_the_normal_and_not_sideways() {
        let mut volume = slab(0.0);
        let box_lo = IVec3::splat(-24);
        let box_hi = IVec3::splat(24);
        volume.edit_paint(
            box_lo,
            box_hi,
            |_, position, _, _| {
                if position.x < 0.0 { RED } else { BARE }
            },
        );
        volume.begin_stroke();
        raise(&mut volume, 4.0, 4.0);
        volume.carry_paint_onto_new_surface(box_lo, box_hi);
        assert_eq!(volume.paint().at(on_top(-2.0, 0.0, 4.0)), RED);
        for x in [0.5_f32, 1.0, 2.0, 3.5] {
            assert_eq!(volume.paint().at(on_top(x, 0.0, 4.0)), BARE, "at x = {x}");
        }
        let _ = volume.end_stroke();
    }

    /// A body with no paint at all is not looked at.
    #[test]
    fn nothing_is_carried_where_nothing_is_painted() {
        let mut volume = slab(0.0);
        volume.begin_stroke();
        raise(&mut volume, 4.0, 4.0);
        assert_eq!(volume.carry_paint_onto_new_surface(IVec3::splat(-24), IVec3::splat(24)), 0);
        let _ = volume.end_stroke();
    }
}
