// SPDX-License-Identifier: AGPL-3.0-only

//! The one-click coats: a base coat, a zenithal and a spongechop, each laid
//! on layers of its own. Stage 4 of `docs/PAINT-RACK.md`, section 7.
//!
//! Each adds its layers to the body's stack and fills them, voxel by voxel
//! of the narrow band, from what the field says of the surface there: which
//! way it faces, how much of a crease or an edge it is, and how high it
//! stands along the light. The mask holds each back as it holds a brush.
//! None records strokes: the application keeps the stack as it was before
//! as one undo entry ([`crate::undo::Change::Layers`]).
//!
//! The light is a direction toward it, as the sun of a zenithal is: up is
//! +y, the front of the model +z.

use glam::{IVec3, Vec3};

use super::field::PaintField;
#[cfg(test)]
use super::field::Rgba;
use super::layers::{Blend, LayerId, crease_from, voxel_of};
use super::stroke::{Mode, Pigment, laid};
use super::touch::{Frame, Sponge};
use crate::brick::{BRICK_VOXELS, Brick, BrickCoord};
use crate::mask::PROTECTED;
use crate::volume::Volume;

/// The way toward a light `azimuth` degrees round from the front, toward
/// +x, and `height` degrees up from level.
pub fn light_from(azimuth: f32, height: f32) -> Vec3 {
    let (az, el) = (azimuth.to_radians(), height.to_radians());
    Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos()).normalize_or(Vec3::Y)
}

/// The zenithal's light as it starts: from above, a little to the front.
pub const ABOVE: (f32, f32) = (0.0, 75.0);

/// A zenithal: primers by how much each point faces a light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zenithal {
    /// Toward the light.
    pub light: Vec3,
    pub dark: [u8; 3],
    /// Between the dark and the light, if there is a third primer.
    pub middle: Option<[u8; 3]>,
    pub bright: [u8; 3],
    /// How wide the spray's cone is, in degrees: narrow is a hard top
    /// light, wide a soft one.
    pub cone: f32,
    /// Whether the model shadows itself: a ray toward the light from each
    /// point, which costs time on a big model.
    pub shadows: bool,
}

impl Default for Zenithal {
    fn default() -> Self {
        Self {
            light: light_from(ABOVE.0, ABOVE.1),
            dark: [12, 12, 14],
            middle: None,
            bright: [245, 245, 240],
            cone: 60.0,
            shadows: false,
        }
    }
}

impl Zenithal {
    /// How lit a point is, 0..=1, from how squarely it faces the light,
    /// through the cone: a narrow cone leaves only what faces the light
    /// nearly head on lit.
    pub fn value(&self, facing: f32) -> f32 {
        let half = ((facing.clamp(-1.0, 1.0) + 1.0) * 0.5).clamp(0.0, 1.0);
        let sharpness = 90.0 / self.cone.clamp(5.0, 90.0);
        half.powf(sharpness)
    }

    /// The primer at a value.
    pub fn colour(&self, value: f32) -> [u8; 3] {
        let value = value.clamp(0.0, 1.0);
        match self.middle {
            None => mix(self.dark, self.bright, value),
            Some(middle) if value < 0.5 => mix(self.dark, middle, value * 2.0),
            Some(middle) => mix(middle, self.bright, value * 2.0 - 1.0),
        }
    }
}

/// A spongechop: a dark coat, a middle and a light sponged by how high,
/// how lit and how far out of a recess each point is, and a contrast over
/// them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spongechop {
    pub light: Vec3,
    pub dark: [u8; 3],
    pub middle: [u8; 3],
    pub bright: [u8; 3],
    /// A contrast paint laid over the three, if any.
    pub contrast: Option<[u8; 3]>,
    /// The sponge the middle and the light are dabbed with.
    pub sponge: Sponge,
    pub seed: u32,
}

impl Default for Spongechop {
    fn default() -> Self {
        Self {
            light: light_from(ABOVE.0, ABOVE.1),
            dark: [12, 12, 14],
            middle: [128, 128, 124],
            bright: [245, 245, 240],
            contrast: None,
            sponge: Sponge { density: 0.6, grain_mm: 0.5, ..Sponge::default() },
            seed: 1,
        }
    }
}

impl Spongechop {
    /// How much of the light a point earns, 0..=1: a mix of how much it
    /// faces the light, how high it stands along it, and how far it is out
    /// of a recess.
    pub fn weight(facing: f32, height: f32, crease: f32) -> f32 {
        let facing = (facing.clamp(-1.0, 1.0) + 1.0) * 0.5;
        let out_of_recess = (0.5 - 0.5 * crease.clamp(-1.0, 1.0) * 4.0).clamp(0.0, 1.0);
        (0.45 * facing + 0.3 * height.clamp(0.0, 1.0) + 0.25 * out_of_recess).clamp(0.0, 1.0)
    }

    /// The share of the surface the middle covers at a weight, and the
    /// light's: the light on less of it, and only the best of it.
    pub fn shares(weight: f32) -> (f32, f32) {
        (smooth(0.3, 0.6, weight), smooth(0.6, 0.85, weight))
    }
}

fn smooth(from: f32, to: f32, value: f32) -> f32 {
    let t = ((value - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(one: [u8; 3], other: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|channel| {
        (f32::from(one[channel]) * (1.0 - t) + f32::from(other[channel]) * t).round() as u8
    })
}

/// What the field says of the surface at a voxel of the band.
#[derive(Debug, Clone, Copy)]
struct Surface {
    position: Vec3,
    normal: Vec3,
    crease: f32,
}

impl Volume {
    fn surface_at(&self, voxel: IVec3) -> Surface {
        let sample = |offset: IVec3| self.sample_voxel(voxel + offset);
        let centre = sample(IVec3::ZERO);
        let around = [
            sample(IVec3::X),
            sample(-IVec3::X),
            sample(IVec3::Y),
            sample(-IVec3::Y),
            sample(IVec3::Z),
            sample(-IVec3::Z),
        ];
        let normal = Vec3::new(around[0] - around[1], around[2] - around[3], around[4] - around[5])
            .normalize_or_zero();
        Surface {
            position: voxel.as_vec3() * self.voxel_size(),
            normal,
            crease: crease_from(centre, around),
        }
    }

    /// Fill the active layer: `paint` says, for each voxel of the band,
    /// what colour and how much of it. The mask holds it back.
    fn fill_active(&mut self, paint: impl Fn(&Volume, IVec3, Surface) -> Option<([u8; 3], f32)>) {
        let coords: Vec<BrickCoord> = self.brick_coords().collect();
        let mut wanted: Vec<Option<([u8; 3], f32)>> = vec![None; BRICK_VOXELS];
        for coord in coords {
            if let Some(Brick::Uniform(value)) = self.brick(coord)
                && !PaintField::paintable(*value)
            {
                continue;
            }
            let origin = coord.origin();
            let mut any = false;
            for (index, slot) in wanted.iter_mut().enumerate() {
                let (x, y, z) = voxel_of(index);
                let voxel = origin + IVec3::new(x, y, z);
                *slot = if PaintField::paintable(self.sample_voxel(voxel)) {
                    paint(self, voxel, self.surface_at(voxel))
                } else {
                    None
                };
                any |= slot.is_some();
            }
            if !any {
                continue;
            }
            let wanted = &wanted;
            self.edit_paint_over(origin, coord.max_voxel(), |voxel, _, held, _, protection| {
                let local = voxel - origin;
                let index = (local.x + local.y * 32 + local.z * 1024) as usize;
                let Some((colour, amount)) = wanted[index] else {
                    return held;
                };
                let free = 1.0 - f32::from(protection) / f32::from(PROTECTED);
                let pigment =
                    Pigment { colour, opacity: 1.0, mode: Mode::Paint, ..Pigment::default() };
                laid(held, pigment, (amount * free).clamp(0.0, 1.0))
            });
        }
    }

    /// How far along `light` the body reaches, low and high, from its box.
    fn reach_along(&self, light: Vec3) -> (f32, f32) {
        let Some((low, high)) = self.world_bounds() else {
            return (0.0, 1.0);
        };
        let mut range = (f32::MAX, f32::MIN);
        for corner in 0..8 {
            let at = Vec3::new(
                if corner & 1 == 0 { low.x } else { high.x },
                if corner & 2 == 0 { low.y } else { high.y },
                if corner & 4 == 0 { low.z } else { high.z },
            );
            let along = at.dot(light);
            range = (range.0.min(along), range.1.max(along));
        }
        range
    }

    /// Whether something of the body stands between `from` and the light.
    fn shadowed(&self, from: Vec3, light: Vec3, length: f32) -> bool {
        let voxel_size = self.voxel_size();
        let mut travelled = voxel_size * 2.5;
        while travelled < length {
            let distance = self.sample_world(from + light * travelled);
            if distance < 0.0 {
                return true;
            }
            travelled += (distance * voxel_size).max(voxel_size);
        }
        false
    }

    /// A new layer, active, by this name and blend; `None` when the stack
    /// is full.
    fn coat_layer(&mut self, name: &str, blend: Blend) -> Option<LayerId> {
        self.add_layer(name, blend)
    }

    /// The whole body in one colour, on a new layer "Base coat" at the
    /// bottom of the stack. `false` when the stack is full.
    pub fn base_coat(&mut self, colour: [u8; 3]) -> bool {
        if self.coat_layer("Base coat", Blend::Normal).is_none() {
            return false;
        }
        let at = self.layers().active();
        self.reorder_layer(at, 0);
        self.fill_active(|_, _, _| Some((colour, 1.0)));
        true
    }

    /// A varnish over all of the body, on a new layer of the Varnish blend
    /// over the active one, "Varnish: Matte" and so on. `false` when the
    /// stack is full. Stage 6 of `docs/PAINT-RACK.md`.
    pub fn varnish_coat(&mut self, finish: crate::paint::Finish) -> bool {
        let name = format!("Varnish: {}", finish.label());
        if self.coat_layer(&name, Blend::Varnish).is_none() {
            return false;
        }
        let code = finish.code();
        self.fill_active(|_, _, _| Some(([code, 0, 0], 1.0)));
        true
    }

    /// A zenithal, on a new layer "Zenithal" over the active one. `false`
    /// when the stack is full.
    pub fn zenithal(&mut self, zenithal: &Zenithal) -> bool {
        if self.coat_layer("Zenithal", Blend::Normal).is_none() {
            return false;
        }
        let light = zenithal.light.normalize_or(Vec3::Y);
        let (low, high) = self.reach_along(light);
        let length = (high - low).max(self.voxel_size()) * 1.5;
        self.fill_active(|volume, _, surface| {
            let mut value = zenithal.value(surface.normal.dot(light));
            if zenithal.shadows && value > 0.0 && volume.shadowed(surface.position, light, length) {
                value *= 0.25;
            }
            Some((zenithal.colour(value), 1.0))
        });
        true
    }

    /// A spongechop: a layer each for the dark, the middle and the light,
    /// and one with the contrast blend for the contrast if there is one,
    /// over the active layer. `false` when the stack has no room for them.
    pub fn spongechop(&mut self, chop: &Spongechop) -> bool {
        let wanted = 3 + usize::from(chop.contrast.is_some());
        if self.layers().len() + wanted > super::layers::MAX_LAYERS {
            return false;
        }
        let light = chop.light.normalize_or(Vec3::Y);
        let (low, high) = self.reach_along(light);
        let span = (high - low).max(1e-3);
        let weight = |surface: Surface| {
            let height = (surface.position.dot(light) - low) / span;
            Spongechop::weight(surface.normal.dot(light), height, surface.crease)
        };
        // A speck's world position is on the surface, as a sponge stamp's
        // is, so the middle and the light share the sponge's pattern.
        let speck = |surface: Surface, seed: u32| {
            Frame::speck(surface.position, chop.sponge, chop.seed.wrapping_add(seed))
        };
        let dabbed = |share: f32, value: f32| ((share * 1.2 - value) / 0.2).clamp(0.0, 1.0);

        self.coat_layer("Spongechop: dark", Blend::Normal);
        self.fill_active(|_, _, _| Some((chop.dark, 1.0)));
        self.coat_layer("Spongechop: middle", Blend::Normal);
        self.fill_active(|_, _, surface| {
            let (middle, _) = Spongechop::shares(weight(surface));
            let amount = dabbed(middle, speck(surface, 0));
            (amount > 0.0).then_some((chop.middle, amount))
        });
        self.coat_layer("Spongechop: light", Blend::Normal);
        self.fill_active(|_, _, surface| {
            let (_, bright) = Spongechop::shares(weight(surface));
            let amount = dabbed(bright, speck(surface, 1));
            (amount > 0.0).then_some((chop.bright, amount))
        });
        if let Some(contrast) = chop.contrast {
            self.coat_layer("Spongechop: contrast", Blend::Contrast);
            self.fill_active(|_, _, _| Some((contrast, 1.0)));
        }
        true
    }
}

/// The paint of a layer at a voxel: what the tests read.
#[cfg(test)]
fn layer_at(volume: &Volume, layer: usize, voxel: IVec3) -> Rgba {
    volume.layer_fields()[layer].at(voxel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::field::BARE;
    use crate::paint::layers::LayerMeta;
    use proptest::prelude::*;

    fn ball() -> Volume {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        volume
    }

    fn luma(rgb: Rgba) -> f32 {
        0.2126 * f32::from(rgb[0]) + 0.7152 * f32::from(rgb[1]) + 0.0722 * f32::from(rgb[2])
    }

    const TOP: IVec3 = IVec3::new(0, 20, 0);
    const BOTTOM: IVec3 = IVec3::new(0, -20, 0);

    /// A varnish coat is a Varnish layer whose finish is every voxel's,
    /// and lays no colour: what is seen is what was under it.
    #[test]
    fn a_varnish_coat_lays_a_finish_and_no_colour() {
        let mut volume = ball();
        volume.edit_paint(IVec3::splat(-30), IVec3::splat(30), |_, _, _, _| [200, 10, 10, 255]);
        let mut dirty = Vec::new();
        volume.take_dirty(&mut dirty);
        let seen = volume.display_paint().at(TOP);
        assert!(volume.varnish_coat(crate::paint::Finish::Gloss));
        volume.take_dirty(&mut dirty);
        assert_eq!(volume.layers().metas()[1].blend, Blend::Varnish);
        assert_eq!(volume.finish_at(TOP), crate::paint::Finish::Gloss.code());
        assert_eq!(volume.display_paint().at(TOP), seen, "a varnish changed the colour");
        // Hidden, it is no finish; flattened, it stays a varnish.
        let mut meta = volume.layers().metas()[1].clone();
        meta.visible = false;
        volume.set_layer_meta(1, meta.clone());
        assert_eq!(volume.finish_at(TOP), 0);
        volume.set_layer_meta(1, LayerMeta { visible: true, ..meta });
        volume.flatten_layers();
        volume.take_dirty(&mut dirty);
        let blends: Vec<_> = volume.layers().metas().into_iter().map(|meta| meta.blend).collect();
        assert_eq!(blends, [Blend::Normal, Blend::Varnish], "a flatten lost the varnish");
        assert_eq!(volume.finish_at(TOP), crate::paint::Finish::Gloss.code());
        assert_eq!(volume.display_paint().at(TOP), seen);
    }

    /// A base coat goes at the bottom of the stack and covers the body.
    #[test]
    fn a_base_coat_covers_the_body_from_the_bottom() {
        let mut volume = ball();
        volume.add_layer("Over", Blend::Normal);
        assert!(volume.base_coat([200, 10, 10]));
        let names: Vec<_> = volume.layers().metas().into_iter().map(|meta| meta.name).collect();
        assert_eq!(names, ["Base coat", "Paint", "Over"]);
        assert_eq!(volume.layers().active(), 0);
        for voxel in [TOP, BOTTOM, IVec3::new(20, 0, 0)] {
            assert_eq!(layer_at(&volume, 0, voxel), [200, 10, 10, 255], "{voxel}");
        }
    }

    /// A zenithal from above is light on top and dark underneath, and a
    /// mask keeps it off.
    #[test]
    fn a_zenithal_is_light_on_top_and_dark_below() {
        let mut volume = ball();
        let (lo, hi) = (IVec3::splat(-30), IVec3::splat(30));
        volume.edit_mask(lo, hi, |_, at, _| if at.x < -15.0 / 2.0 { PROTECTED } else { 0 });
        let zenithal = Zenithal { light: Vec3::Y, ..Zenithal::default() };
        assert!(volume.zenithal(&zenithal));
        let layer = volume.layers().active();
        let (top, bottom) = (layer_at(&volume, layer, TOP), layer_at(&volume, layer, BOTTOM));
        assert!(luma(top) > 200.0 && luma(bottom) < 30.0, "{top:?} over {bottom:?}");
        assert_eq!(layer_at(&volume, layer, IVec3::new(-20, 0, 0)), BARE, "under the mask");
    }

    /// With shadows, a ball over a ball darkens the top of the lower one.
    #[test]
    fn a_zenithal_with_shadows_darkens_what_is_under_something() {
        let top_of_lower = IVec3::new(0, 12, 0);
        let lit = |shadows| {
            let mut volume = Volume::new(0.5);
            for centre in [Vec3::ZERO, Vec3::new(0.0, 16.0, 0.0)] {
                let mut one = Volume::new(0.5);
                one.seed_sphere(centre, 6.0);
                volume.begin_stroke();
                volume.union_from(&one);
                let _ = volume.end_stroke();
            }
            volume.zenithal(&Zenithal { light: Vec3::Y, shadows, ..Zenithal::default() });
            let layer = volume.layers().active();
            luma(layer_at(&volume, layer, top_of_lower))
        };
        assert!(lit(true) < lit(false) * 0.5, "{} against {}", lit(true), lit(false));
    }

    /// A spongechop makes its layers, dark everywhere, and the light more
    /// on top than underneath; with a contrast, a contrast layer over them.
    #[test]
    fn a_spongechop_is_dark_below_and_light_on_top() {
        let mut volume = ball();
        let chop =
            Spongechop { light: Vec3::Y, contrast: Some([150, 20, 20]), ..Spongechop::default() };
        assert!(volume.spongechop(&chop));
        let metas = volume.layers().metas();
        let names: Vec<_> = metas.iter().map(|meta| meta.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Paint",
                "Spongechop: dark",
                "Spongechop: middle",
                "Spongechop: light",
                "Spongechop: contrast"
            ]
        );
        assert_eq!(metas[4].blend, Blend::Contrast);
        // The light covers more of the top cap than of the bottom one.
        let covered = |layer: usize, upper: bool| {
            let mut count = 0;
            for x in -12..=12 {
                for z in -12..=12 {
                    for y in 14..=21 {
                        let voxel = IVec3::new(x, if upper { y } else { -y }, z);
                        if volume.sample_voxel(voxel).abs() < 0.5
                            && layer_at(&volume, layer, voxel)[3] > 0
                        {
                            count += 1;
                        }
                    }
                }
            }
            count
        };
        assert!(
            covered(3, true) > covered(3, false) * 3,
            "light on top {} and below {}",
            covered(3, true),
            covered(3, false)
        );
        assert_eq!(layer_at(&volume, 1, BOTTOM)[3], 255, "the dark does not cover");
    }

    proptest! {
        /// A zenithal's value never falls as a point faces the light more,
        /// and a wider cone never leaves a point darker.
        #[test]
        fn more_facing_is_never_darker(
            a in -1.0f32..1.0,
            b in -1.0f32..1.0,
            narrow in 5.0f32..90.0,
            wider in 0.0f32..85.0,
        ) {
            let z = Zenithal { cone: narrow, ..Zenithal::default() };
            let wide = Zenithal { cone: (narrow + wider).min(90.0), ..Zenithal::default() };
            let (less, more) = (a.min(b), a.max(b));
            prop_assert!(z.value(more) >= z.value(less));
            prop_assert!(wide.value(a) >= z.value(a) - 1e-6);
        }

        /// A spongechop never puts more of the light in a recess than on
        /// the peak beside it, facing and height alike.
        #[test]
        fn a_recess_never_earns_more_light_than_a_peak(
            facing in -1.0f32..1.0,
            height in 0.0f32..1.0,
            recess in 0.0f32..1.0,
            peak in 0.0f32..1.0,
        ) {
            let in_recess = Spongechop::weight(facing, height, recess);
            let on_peak = Spongechop::weight(facing, height, -peak);
            prop_assert!(on_peak >= in_recess);
            let (m1, l1) = Spongechop::shares(in_recess);
            let (m2, l2) = Spongechop::shares(on_peak);
            prop_assert!(m2 >= m1 && l2 >= l1);
            prop_assert!(l1 <= m1 + 1e-6, "the light covers more than the middle");
        }
    }
}
