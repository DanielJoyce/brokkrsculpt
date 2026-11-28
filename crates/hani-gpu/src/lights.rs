// SPDX-License-Identifier: AGPL-3.0-only

//! Lights that can be set, and the surfaces they light: stage one of issue
//! 203, the lighting model with no panel of its own yet.
//!
//! Until 2026-11-27 the viewport was lit by a matcap (`crate::matcap`): one
//! picture of a lit ball, looked up by the normal in view space, so every
//! light followed the camera and none could be moved. Paint was the clay's
//! brightness times its colour, and a finish one highlight from a fixed
//! direction. This module replaces the three with one shading model, lit by
//! a [`Rig`]:
//!
//! - up to four lights, each a direction, a colour, a strength and whether
//!   it is with the camera or fixed in the world;
//! - a global light, a sky colour and a ground colour, lighting a surface by
//!   which way it faces in the WORLD (a hemisphere), and drawn as the
//!   surrounding a mirror sees: the sky above a sharp horizon, the ground
//!   below it, and toward each light the glow it casts on the room round
//!   the model ([`ROOM`]), all worked out in the shader with no image;
//! - a [`Surface`] at each pixel: a diffuse term (wrapped Lambert, so a
//!   light can reach a little past side-on) and a microfacet highlight
//!   (GGX, with Smith's visibility term and Schlick's Fresnel), by a
//!   roughness and a metalness, and a reflection of the surrounding (Karis's
//!   fit to the split-sum term, the surrounding blurred by the roughness).
//!   A light's highlight is never sharper than [`LIGHT_SIZE`] allows, so on
//!   a mirror each light is a soft bright patch and not a speck.
//!
//! A metallic paint's flakes are a hash of where a point is on the model,
//! 0.2 mm cells each leant its own way, which tilt the normal its
//! highlights are taken from; they fade where a flake is smaller than a
//! pixel. That is in the shader only (`flake_hash`), and [`shade`] is given
//! the leant normal as `glint`.
//!
//! What it costs: four lights' worth of a GGX term and a power for every
//! pixel of the model, whatever the surface, and no loop over more. See
//! `docs/PERFORMANCE.md`.
//!
//! # The default is today's picture
//!
//! [`Rig::default`] and [`Material::Clay`] are the matcap's own numbers, so
//! that nothing on screen changes until a light is moved or another
//! material chosen. The matcap is not three lights, and the parts of it that
//! are not are given to the clay:
//!
//! - its key and its fill ARE lights, with the camera: the key wraps 0.2
//!   past side-on and the fill falls off as the power 1.5 of its cosine and
//!   casts no highlight, as the matcap's do;
//! - its warm rim is not a light (it depends on the angle to the eye and on
//!   no light's direction): it is the clay's rim colour, a sheen at grazing
//!   angles lit by the global light;
//! - its white highlight is the clay's specular colour, at the roughness
//!   whose GGX lobe best matches the matcap's Blinn lobe of power 48;
//! - its cool fill is the clay's: today's clay was in effect a two-tone
//!   (Gooch-style) material, its warm grey under the key and a cool colour,
//!   not its own, under the fill. So a [`Surface`] has two diffuse colours,
//!   `key` under the rig's first light and the global light and `shade`
//!   under every other light, and the default rig's fill is neutral. Paint
//!   keeps its colour under that fill, as it always has: a painter judges a
//!   paint against its swatch. Every other material, and paint, has the two
//!   colours the same.
//!
//! Paint with no finish is lit as the clay is, with its own colour in place
//! of every one of the clay's: a highlight and a rim in its own colour, as
//! the matcap's brightness gave it.
//!
//! The direction to the eye is taken as the view's own axis at every pixel,
//! as the matcap took it. That is what keeps the default the matcap's
//! picture under a perspective camera, and it costs nothing; a highlight
//! still moves over the model as the camera turns, as one does.
//!
//! `sculpt.wgsl` holds the shader's copy of [`shade`], and the offscreen
//! test `the_rust_model_is_the_shaders` holds the two together pixel by
//! pixel. The paint's picture in `hani-app` (`app/paint_preview.rs`) draws
//! a metal by [`shade`].

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::matcap;

/// The most lights a rig has. The shader runs over all four every pixel, a
/// light that is off having no colour; no loop runs over more.
pub const MAX_LIGHTS: usize = 4;

/// The smallest GGX alpha (the square of the roughness) a light's highlight
/// is drawn at: a light is a soft box of some size and not a point, so on a
/// mirror it is a soft bright patch and not a speck. The shader has the
/// other copy.
pub const LIGHT_SIZE: f32 = 0.05;

/// How much brighter the sky is drawn above the horizon, and the ground
/// darker below it, in what a smooth surface reflects: a fraction of their
/// colours, which fades as the surface roughens so that a rough one sees
/// the hemisphere's average. The shader has the other copy.
pub const HORIZON: f32 = 0.75;

/// How bright the room round the model is in what a smooth surface
/// reflects, toward each light, as a share of the light; and how fast it
/// falls away from the light, as a power of the half-cosine. Each light
/// lights the room it is in, and a metal shows that: bright on the side
/// toward the light and dark away from it, which is what lets a metal be
/// read under the default rig, whose global light is only a dim floor. The
/// shader has the other copy.
pub const ROOM: f32 = 0.7;
pub const ROOM_POWER: f32 = 3.0;

/// How much of the light behind a thin edge comes through a material that
/// lets light in (resin): a share of its `through` colour. The shader has
/// the other copy.
pub const THIN_GLOW: f32 = 0.6;

/// One light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Toward the light. In view space for a light with the camera (x to the
    /// right of the screen, y up it, z toward the eye); in the model's own
    /// space for one fixed in the world, turned into view space every pixel
    /// by the view matrix. Need not be of unit length.
    pub direction: Vec3,
    /// Linear RGB.
    pub colour: [f32; 3],
    /// What the colour is multiplied by.
    pub strength: f32,
    /// With the camera, or fixed in the world.
    pub with_camera: bool,
    /// How far past side-on it reaches, as a cosine: 0 is Lambert's law, 0.2
    /// the matcap's key. A large soft source reaches further round.
    pub wrap: f32,
    /// The power its wrapped cosine is raised to: 1 for a light, 1.5 the
    /// matcap's fill, which fell off faster toward its edge.
    pub falloff: f32,
    /// How much of a highlight it casts, 0..=1: 1 for a light, 0 for the
    /// matcap's fill, which cast none.
    pub highlight: f32,
}

impl Light {
    /// A plain light: Lambert's law, a full highlight.
    pub fn new(direction: Vec3, colour: [f32; 3], strength: f32, with_camera: bool) -> Self {
        Self { direction, colour, strength, with_camera, wrap: 0.0, falloff: 1.0, highlight: 1.0 }
    }
}

/// The lights, and the global light.
#[derive(Debug, Clone, PartialEq)]
pub struct Rig {
    /// At most [`MAX_LIGHTS`]; any past that are not drawn. The first is the
    /// key: a material's `key` colour is lit by it, its `shade` colour by
    /// the others.
    pub lights: Vec<Light>,
    /// How strong the global light is.
    pub global: f32,
    /// The sky's colour and the ground's, linear, times `global`: a surface
    /// facing straight up is lit by the sky's, straight down by the
    /// ground's, and one facing sideways by half of each.
    pub sky: [f32; 3],
    pub ground: [f32; 3],
}

impl Default for Rig {
    /// The rig the clay's matcap was made from: its key and its fill, both
    /// with the camera, and its ambient floor as an even global light. Its
    /// rim is the clay's own (see the module's notes).
    fn default() -> Self {
        let key = Light {
            wrap: matcap::KEY_WRAP,
            ..Light::new(matcap::KEY, [1.0; 3], matcap::KEY_STRENGTH, true)
        };
        let fill = Light {
            falloff: matcap::FILL_FALLOFF,
            highlight: 0.0,
            ..Light::new(matcap::FILL, [1.0; 3], fill_strength(), true)
        };
        Self { lights: vec![key, fill], global: matcap::AMBIENT, sky: [1.0; 3], ground: [1.0; 3] }
    }
}

impl Rig {
    /// The global light alone.
    pub fn sky_only(global: f32, sky: [f32; 3], ground: [f32; 3]) -> Self {
        Self { lights: Vec::new(), global, sky, ground }
    }

    /// Where each light is, toward it, in view space, for a view matrix.
    /// What the shader works out every pixel, for tests and for the paint's
    /// picture.
    pub fn directions_in_view(&self, view: Mat4) -> Vec<Vec3> {
        self.lights
            .iter()
            .take(MAX_LIGHTS)
            .map(|light| {
                let way = if light.with_camera {
                    light.direction
                } else {
                    view.transform_vector3(light.direction)
                };
                way.normalize_or(Vec3::Z)
            })
            .collect()
    }
}

/// The strength of the default rig's neutral fill: arithmetic, the share of
/// the clay's own lightness that the matcap's fill colour has, which is how
/// bright paint was under it (paint was lit by the matcap's lightness over
/// the clay's).
fn fill_strength() -> f32 {
    luma(matcap::FILL_COLOUR) / luma(matcap::BASE)
}

fn luma(colour: Vec3) -> f32 {
    colour.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// The roughness of the clay's highlight and of unvarnished paint's: the
/// GGX lobe nearest the matcap's Blinn lobe of power 48, found by fitting
/// the two over every normal a ball shows (a Python copy of both, the peaks
/// matched, the largest difference least at an alpha of 0.24; arithmetic,
/// not measured on a screen).
pub const SHEEN_ROUGHNESS: f32 = 0.49;

/// What the colour of a paint, linear, is multiplied by before it is lit:
/// filament colours and paints are drawn a little under their full value,
/// as they always were (the shader's `PAINT_EXPOSURE` before this).
pub const PAINT_EXPOSURE: f32 = 0.8;

/// What a surface is at a pixel: what the shading model is given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    /// The diffuse colour under the rig's first light and the global light,
    /// linear.
    pub key: Vec3,
    /// The diffuse colour under every other light. The same as `key` for
    /// every surface but the default clay's (see the module's notes).
    pub shade: Vec3,
    /// How far past side-on the light reaches into it, added to each
    /// light's own: light let in, as a resin lets it.
    pub wrap: f32,
    /// The colour of its highlight and of what it reflects, head on.
    pub f0: Vec3,
    /// 0 a mirror, 1 as rough as can be.
    pub roughness: f32,
    /// 1 a metal: no diffuse at all, its colour all in `f0`.
    pub metal: f32,
    /// How much of the surrounding it shows, 0..=1. 0 for the clay and for
    /// unvarnished paint, which the matcap gave none.
    pub reflect: f32,
    /// A sheen at grazing angles, lit by the global light.
    pub rim: Vec3,
    /// The colour of light let through: what its wrapped light past each
    /// light's own reach is, and what glows on a thin edge lit from behind.
    pub through: Vec3,
}

impl Surface {
    /// Unvarnished paint of a colour, linear, before [`PAINT_EXPOSURE`]: lit
    /// as the clay is, with its own colour in place of every one of the
    /// clay's.
    pub fn paint(colour: Vec3) -> Self {
        let albedo = colour * PAINT_EXPOSURE;
        Self {
            key: albedo,
            shade: albedo,
            wrap: 0.0,
            f0: albedo * paint_sheen(),
            roughness: SHEEN_ROUGHNESS,
            metal: 0.0,
            reflect: 0.0,
            rim: albedo * paint_rim(),
            through: Vec3::ZERO,
        }
    }

    /// A metal of a colour, linear, as rough as `roughness`: what a
    /// metallic paint and chrome are.
    pub fn metal(colour: Vec3, roughness: f32) -> Self {
        Self {
            key: Vec3::ZERO,
            shade: Vec3::ZERO,
            wrap: 0.0,
            f0: colour,
            roughness,
            metal: 1.0,
            reflect: 1.0,
            rim: Vec3::ZERO,
            through: Vec3::ZERO,
        }
    }
}

/// The clay's highlight colour, white: the strength whose GGX peak, under
/// the default key, is the matcap's highlight's (arithmetic).
fn clay_sheen() -> f32 {
    let key = matcap::KEY.normalize();
    let half_way = (key + Vec3::Z).normalize();
    let alpha = (SHEEN_ROUGHNESS * SHEEN_ROUGHNESS).max(LIGHT_SIZE);
    matcap::SHINE / (matcap::KEY_STRENGTH * microfacet(half_way, key, alpha))
}

/// Paint's highlight, as a share of its colour: the clay's, over the clay's
/// lightness, as paint's highlight was the matcap's over that lightness.
fn paint_sheen() -> f32 {
    clay_sheen() / luma(matcap::BASE)
}

/// Paint's rim, as a share of its colour, lit by the global light: the
/// matcap's rim's lightness over the clay's, over the global light.
fn paint_rim() -> f32 {
    luma(matcap::RIM_COLOUR) * matcap::RIM_SHARE / (luma(matcap::BASE) * matcap::AMBIENT)
}

/// The surface of the unpainted model. Only [`Material::Clay`] is chosen
/// in this stage; the chooser is stage two of issue 203.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Material {
    /// The matcap's warm grey clay, two-toned: the default.
    #[default]
    Clay,
    /// Grey polystyrene, the plastic of a kit, in a colour the user sets
    /// (sRGB).
    Polystyrene([u8; 3]),
    /// Green stuff, mixed epoxy putty: its own colour, nearly matte, a
    /// little waxy.
    GreenStuff,
    /// Pewter: a dull warm grey metal, fairly rough.
    Pewter,
    /// Resin, in a colour the user sets (sRGB): a plastic that lets light
    /// in.
    Resin([u8; 3]),
}

/// Polystyrene's colour until one is chosen: a mid neutral grey.
pub const POLYSTYRENE_GREY: [u8; 3] = [150, 150, 150];
/// Resin's colours to offer: milky white, the default, grey and a light tan.
pub const RESIN_WHITE: [u8; 3] = [232, 229, 220];
pub const RESIN_GREY: [u8; 3] = [150, 152, 156];
pub const RESIN_TAN: [u8; 3] = [214, 190, 152];
/// Green stuff's colour, fixed: the blue-green of mixed epoxy putty.
pub const GREEN_STUFF: [u8; 3] = [92, 138, 88];
/// Pewter's colour, fixed: a dull warm grey.
pub const PEWTER: [u8; 3] = [196, 192, 182];

impl Material {
    /// Every material, the user's colours at their defaults.
    pub const ALL: [Material; 5] = [
        Material::Clay,
        Material::Polystyrene(POLYSTYRENE_GREY),
        Material::GreenStuff,
        Material::Pewter,
        Material::Resin(RESIN_WHITE),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Material::Clay => "Clay",
            Material::Polystyrene(_) => "Polystyrene",
            Material::GreenStuff => "Green stuff",
            Material::Pewter => "Pewter",
            Material::Resin(_) => "Resin",
        }
    }

    /// The surface it is.
    pub fn surface(self) -> Surface {
        let linear = |rgb: [u8; 3]| Vec3::from(hani_core::paint::linear(rgb));
        match self {
            Material::Clay => Surface {
                key: matcap::BASE,
                // Whatever makes the default's neutral fill give back the
                // matcap's fill colour: arithmetic.
                shade: matcap::FILL_COLOUR / fill_strength(),
                wrap: 0.0,
                f0: Vec3::splat(clay_sheen()),
                roughness: SHEEN_ROUGHNESS,
                metal: 0.0,
                reflect: 0.0,
                rim: matcap::RIM_COLOUR * matcap::RIM_SHARE / matcap::AMBIENT,
                through: Vec3::ZERO,
            },
            // A plastic: a soft broad white highlight, a little of the
            // surrounding, no metal.
            Material::Polystyrene(rgb) => Surface {
                key: linear(rgb),
                shade: linear(rgb),
                wrap: 0.0,
                f0: Vec3::splat(0.04),
                roughness: 0.42,
                metal: 0.0,
                reflect: 1.0,
                rim: Vec3::ZERO,
                through: Vec3::ZERO,
            },
            // Nearly matte, a little waxy: a broad dim highlight, light
            // reaching a little past side-on, and a sheen of its own colour
            // at grazing angles.
            Material::GreenStuff => {
                let colour = linear(GREEN_STUFF);
                Surface {
                    key: colour,
                    shade: colour,
                    wrap: 0.12,
                    f0: Vec3::splat(0.035),
                    roughness: 0.75,
                    metal: 0.0,
                    reflect: 1.0,
                    rim: colour * 0.8,
                    through: colour * 0.8,
                }
            }
            Material::Pewter => Surface::metal(linear(PEWTER), 0.5),
            // Light let in: it wraps half way past side-on, and what comes
            // round is a little warmer and softer than what lands; a thin
            // edge lit from behind glows. No subsurface scattering.
            Material::Resin(rgb) => {
                let colour = linear(rgb);
                Surface {
                    key: colour,
                    shade: colour,
                    wrap: 0.5,
                    f0: Vec3::splat(0.04),
                    roughness: 0.32,
                    metal: 0.0,
                    reflect: 1.0,
                    rim: Vec3::ZERO,
                    through: colour * Vec3::new(1.0, 0.82, 0.66),
                }
            }
        }
    }
}

/// The rig and the model's material as the shader is handed them, in
/// [`crate::Uniforms::lights`]. The layout matches `Lights` in
/// `sculpt.wgsl`; `tests::the_block_is_laid_out_as_the_shader_reads_it`
/// holds the two together.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LightBlock {
    /// Toward each light, and in `w` 1 for a light fixed in the world (the
    /// shader turns it by the view matrix) and 0 for one with the camera.
    pub direction: [[f32; 4]; MAX_LIGHTS],
    /// Each light's colour times its strength, and in `w` its highlight. A
    /// light that is off is all nought.
    pub colour: [[f32; 4]; MAX_LIGHTS],
    /// Each light's wrap, and its falloff.
    pub wrap: [f32; 4],
    pub falloff: [f32; 4],
    /// The sky's colour and the ground's, times the global light; `w` spare.
    pub sky: [f32; 4],
    pub ground: [f32; 4],
    /// The model's material, a [`Surface`]: `key` and in `w` its wrap.
    pub key: [f32; 4],
    /// `shade`, and in `w` how much it reflects.
    pub shade: [f32; 4],
    /// `f0`, and in `w` its roughness.
    pub f0: [f32; 4],
    /// `rim`, and in `w` its metalness.
    pub rim: [f32; 4],
    /// `through`; `w` spare.
    pub through: [f32; 4],
    /// How paint is lit: [`PAINT_EXPOSURE`], its highlight and its rim as
    /// shares of its colour, and their roughness.
    pub paint: [f32; 4],
    /// Non zero in `x`: draw the clay and paint by the matcap, exactly as
    /// before 2026-11-27, and no finish. Kept for the offscreen test that
    /// holds the default to that picture, and for nothing else.
    pub matcap: [u32; 4],
}

impl LightBlock {
    /// The rig, and the model's material.
    pub fn new(rig: &Rig, material: Material) -> Self {
        let mut block = Self::zeroed();
        for (at, light) in rig.lights.iter().take(MAX_LIGHTS).enumerate() {
            let way = light.direction.normalize_or(Vec3::Z);
            block.direction[at] = [way.x, way.y, way.z, if light.with_camera { 0.0 } else { 1.0 }];
            let [r, g, b] = light.colour.map(|c| c * light.strength);
            block.colour[at] = [r, g, b, light.highlight];
            block.wrap[at] = light.wrap;
            block.falloff[at] = light.falloff;
        }
        // A light that is off still has a falloff, so its pow is of a
        // finite power.
        for at in rig.lights.len().min(MAX_LIGHTS)..MAX_LIGHTS {
            block.falloff[at] = 1.0;
        }
        let global = rig.global;
        block.sky = [rig.sky[0] * global, rig.sky[1] * global, rig.sky[2] * global, 0.0];
        block.ground =
            [rig.ground[0] * global, rig.ground[1] * global, rig.ground[2] * global, 0.0];
        let surface = material.surface();
        let four = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        block.key = four(surface.key, surface.wrap);
        block.shade = four(surface.shade, surface.reflect);
        block.f0 = four(surface.f0, surface.roughness);
        block.rim = four(surface.rim, surface.metal);
        block.through = four(surface.through, 0.0);
        block.paint = [PAINT_EXPOSURE, paint_sheen(), paint_rim(), SHEEN_ROUGHNESS];
        block
    }

    /// The default rig and the default clay, drawn by the matcap as before
    /// 2026-11-27. For the offscreen test that holds the two together.
    pub fn matcap() -> Self {
        Self { matcap: [1, 0, 0, 0], ..Self::default() }
    }

    /// The model's material as the block holds it.
    pub fn surface(&self) -> Surface {
        let three = |v: [f32; 4]| Vec3::new(v[0], v[1], v[2]);
        Surface {
            key: three(self.key),
            shade: three(self.shade),
            wrap: self.key[3],
            f0: three(self.f0),
            roughness: self.f0[3],
            metal: self.rim[3],
            reflect: self.shade[3],
            rim: three(self.rim),
            through: three(self.through),
        }
    }
}

impl Default for LightBlock {
    fn default() -> Self {
        Self::new(&Rig::default(), Material::Clay)
    }
}

/// One light's GGX highlight, with Smith's height-correlated visibility
/// term, times the cosine of the light: what multiplies the light's colour
/// and the Fresnel term. The eye is down the view's own axis. `sculpt.wgsl`'s
/// `microfacet` is the other copy.
pub fn microfacet(normal: Vec3, light: Vec3, alpha: f32) -> f32 {
    let half_way = (light + Vec3::Z).normalize_or(Vec3::Z);
    let nl = normal.dot(light).max(0.0);
    let nv = normal.z.max(1.0e-3);
    let nh = normal.dot(half_way).max(0.0);
    let a2 = alpha * alpha;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    let distribution = a2 / (std::f32::consts::PI * d * d);
    let visibility = 0.5
        / (nl * (nv * nv * (1.0 - a2) + a2).sqrt()
            + nv * (nl * nl * (1.0 - a2) + a2).sqrt()
            + 1.0e-5);
    distribution * visibility * nl
}

/// Schlick's Fresnel term, by the angle between the eye and the half way
/// vector.
fn fresnel(f0: Vec3, light: Vec3) -> Vec3 {
    let half_way = (light + Vec3::Z).normalize_or(Vec3::Z);
    let vh = half_way.z.max(0.0);
    f0 + (Vec3::ONE - f0) * (1.0 - vh).powi(5)
}

/// How much of the surrounding a surface reflects, by Karis's fit to the
/// split-sum term ("Physically Based Shading on Mobile", 2014).
fn environment_brdf(f0: Vec3, roughness: f32, nv: f32) -> Vec3 {
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r: [f32; 4] = std::array::from_fn(|i| roughness * c0[i] + c1[i]);
    let a004 = (r[0] * r[0]).min((-9.28 * nv).exp2()) * r[0] + r[1];
    let (a, b) = (-1.04 * a004 + r[2], 1.04 * a004 + r[3]);
    f0 * a + Vec3::splat(b)
}

/// The surrounding a mirror sees, in a direction in the world: the sky above
/// a horizon and the ground below it, the sky lighter toward the top, the
/// horizon blurred and the difference faded as the surface roughens.
pub fn surrounding(block: &LightBlock, way: Vec3, roughness: f32) -> Vec3 {
    let sky = Vec3::new(block.sky[0], block.sky[1], block.sky[2]);
    let ground = Vec3::new(block.ground[0], block.ground[1], block.ground[2]);
    let blur = 0.01 + 1.2 * roughness * roughness;
    let t = smoothstep(-blur, blur, way.y);
    let contrast = HORIZON * (1.0 - roughness);
    let above = sky * (1.0 + contrast * (0.4 + 0.6 * way.y.max(0.0)));
    let below = ground * (1.0 - contrast);
    below + (above - below) * t
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The colour of a surface, linear, lit by a block under a view: the shading
/// model, `shade` in `sculpt.wgsl` line for line.
///
/// `normal` is in view space and `world` the same normal in the world;
/// `glint` is the normal the lights' highlights are taken from, which is
/// `normal` except on a metallic paint's flakes; `thin` is how thin the
/// surface is here, 0..=1, from the engine's edge signal.
pub fn shade(
    block: &LightBlock,
    view: Mat4,
    surface: &Surface,
    normal: Vec3,
    world: Vec3,
    glint: Vec3,
    thin: f32,
) -> Vec3 {
    let three = |v: [f32; 4]| Vec3::new(v[0], v[1], v[2]);
    let hemisphere = three(block.ground)
        + (three(block.sky) - three(block.ground)) * (world.y * 0.5 + 0.5).clamp(0.0, 1.0);
    let alpha = (surface.roughness * surface.roughness).max(LIGHT_SIZE);
    // Reflected about the normal from the eye, and bent toward the normal as
    // the surface roughens.
    let mirrored = normal * (2.0 * normal.z) - Vec3::Z;
    let reflected = (mirrored + (normal - mirrored) * surface.roughness * surface.roughness)
        .normalize_or(Vec3::Z);
    let mut diffuse = surface.key * hemisphere;
    let mut specular = Vec3::ZERO;
    let mut behind = hemisphere;
    let mut room = Vec3::ZERO;
    for at in 0..MAX_LIGHTS {
        let d = block.direction[at];
        let way = Vec3::new(d[0], d[1], d[2]);
        let light =
            if d[3] > 0.5 { view.transform_vector3(way) } else { way }.normalize_or(Vec3::Z);
        let colour = three(block.colour[at]);
        let x = normal.dot(light);
        let reach = |wrap: f32| ((x + wrap) / (1.0 + wrap)).clamp(0.0, 1.0).powf(block.falloff[at]);
        let plain = reach(block.wrap[at]);
        let wrapped = reach(block.wrap[at] + surface.wrap);
        let albedo = if at == 0 { surface.key } else { surface.shade };
        diffuse += colour * (albedo * plain + surface.through * (wrapped - plain));
        behind += colour * (-x).max(0.0);
        specular += colour
            * block.colour[at][3]
            * microfacet(glint, light, alpha)
            * fresnel(surface.f0, light);
        room += colour
            * block.colour[at][3]
            * (reflected.dot(light) * 0.5 + 0.5).max(0.0).powf(ROOM_POWER);
    }
    let nv = normal.z.max(0.0);
    let rim = surface.rim * hemisphere * (1.0 - nv).max(0.0).powf(matcap::RIM_POWER);
    // What it reflects: the sky and the ground, in the world, and the room
    // the lights light.
    let in_world = view.inverse().transform_vector3(reflected).normalize_or(Vec3::Y);
    let seen = surrounding(block, in_world, surface.roughness) + room * ROOM;
    let environment = seen * environment_brdf(surface.f0, surface.roughness, nv) * surface.reflect;
    let glow = surface.through * thin * behind * THIN_GLOW;
    diffuse * (1.0 - surface.metal) + specular + rim + environment + glow
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A light fixed in the world is where the view matrix turns it: from
    /// the upper left of the world, seen by a camera that has gone a quarter
    /// of the way round the model, it is up and away from the eye. One with
    /// the camera is where it was put, whatever the view.
    #[test]
    fn a_fixed_light_turns_with_the_view_and_a_camera_light_does_not() {
        let way = Vec3::new(-1.0, 1.0, 0.0).normalize();
        let rig = Rig {
            lights: vec![
                Light::new(way, [1.0; 3], 1.0, false),
                Light::new(way, [1.0; 3], 1.0, true),
            ],
            ..Rig::default()
        };
        // Looking at the origin from +z: the view is the identity's turn.
        let front = glam::camera::rh::view::look_at_mat4(Vec3::Z * 10.0, Vec3::ZERO, Vec3::Y);
        let seen = rig.directions_in_view(front);
        assert!((seen[0] - way).length() < 1e-5 && (seen[1] - way).length() < 1e-5);
        // From +x, the camera turned a quarter to look along -x: the world's
        // -x is straight away from it, so a light up and to the world's
        // left is up and behind, and a camera light is still up and left.
        let side = glam::camera::rh::view::look_at_mat4(Vec3::X * 10.0, Vec3::ZERO, Vec3::Y);
        let seen = rig.directions_in_view(side);
        let wanted = Vec3::new(0.0, 1.0, -1.0).normalize();
        assert!((seen[0] - wanted).length() < 1e-5, "{:?}", seen[0]);
        assert!((seen[1] - way).length() < 1e-5, "{:?}", seen[1]);
        // And the block says which is which, for the shader.
        let block = LightBlock::new(&rig, Material::Clay);
        assert_eq!((block.direction[0][3], block.direction[1][3]), (1.0, 0.0));
        assert_eq!(block.colour[2], [0.0; 4], "a light that is not there has no colour");
    }

    /// The default rig and clay give back the matcap's picture, worked out
    /// on the CPU over every normal a ball shows: the shading model against
    /// `matcap::clay`'s own formula. The GPU's version of this, against the
    /// matcap texture, is `the_default_rig_draws_todays_picture` in the
    /// offscreen tests.
    #[test]
    fn the_default_is_the_matcap_on_the_cpu() {
        use crate::matcap::*;
        let block = LightBlock::default();
        let clay = block.surface();
        let key = KEY.normalize();
        let fill = FILL.normalize();
        let half_way = (key + Vec3::Z).normalize();
        let encode = |linear: f32| {
            let linear = linear.clamp(0.0, 1.0);
            let e = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            (e * 255.0).round() as i32
        };
        let (mut worst_clay, mut worst_paint) = (0, 0);
        let white = Vec3::splat(0.9);
        let red = Vec3::new(0.6, 0.03, 0.03);
        let n = 100;
        for y in 0..n {
            for x in 0..n {
                let (nx, ny) = (
                    (x as f32 + 0.5) / n as f32 * 2.0 - 1.0,
                    (y as f32 + 0.5) / n as f32 * 2.0 - 1.0,
                );
                let r2 = nx * nx + ny * ny;
                if r2 >= 1.0 {
                    continue;
                }
                let normal = Vec3::new(nx, ny, (1.0 - r2).sqrt());
                // What a perspective camera at three radii shows of a ball.
                if normal.z < 0.34 {
                    continue;
                }
                let diffuse = ((normal.dot(key) + KEY_WRAP) / (1.0 + KEY_WRAP)).clamp(0.0, 1.0);
                let old = BASE * (AMBIENT + KEY_STRENGTH * diffuse)
                    + FILL_COLOUR * normal.dot(fill).max(0.0).powf(FILL_FALLOFF)
                    + RIM_COLOUR * (1.0 - normal.z).powf(RIM_POWER) * RIM_SHARE
                    + Vec3::splat(normal.dot(half_way).max(0.0).powf(SHINE_POWER) * SHINE);
                let new = shade(&block, Mat4::IDENTITY, &clay, normal, normal, normal, 0.0);
                for c in 0..3 {
                    worst_clay = worst_clay.max((encode(old[c]) - encode(new[c])).abs());
                }
                let brightness = luma(old) / luma(BASE);
                for colour in [white, red] {
                    let old = colour * brightness * PAINT_EXPOSURE;
                    let surface = Surface::paint(colour);
                    let new = shade(&block, Mat4::IDENTITY, &surface, normal, normal, normal, 0.0);
                    for c in 0..3 {
                        worst_paint = worst_paint.max((encode(old[c]) - encode(new[c])).abs());
                    }
                }
            }
        }
        assert!(worst_clay <= 4, "the clay is {worst_clay} levels from the matcap");
        assert!(worst_paint <= 5, "paint is {worst_paint} levels from today's");
    }

    /// Every material differs from the clay, and the two whose colour the
    /// user chooses take it.
    #[test]
    fn each_material_is_its_own_and_takes_its_colour() {
        let clay = Material::Clay.surface();
        for material in Material::ALL.into_iter().skip(1) {
            assert_ne!(material.surface(), clay, "{}", material.label());
        }
        assert_ne!(
            Material::Polystyrene([200, 30, 30]).surface().key,
            Material::Polystyrene(POLYSTYRENE_GREY).surface().key
        );
        assert_ne!(
            Material::Resin(RESIN_TAN).surface().key,
            Material::Resin(RESIN_WHITE).surface().key
        );
        assert_eq!(Material::Pewter.surface().metal, 1.0);
        assert_eq!(Material::Polystyrene(POLYSTYRENE_GREY).surface().metal, 0.0);
        assert_eq!(Material::default(), Material::Clay);
    }

    /// The block is laid out as `Lights` in `sculpt.wgsl` reads it: its
    /// fields, in order, with their types, read out of the source, and
    /// every field where the shader's alignment puts it.
    #[test]
    fn the_block_is_laid_out_as_the_shader_reads_it() {
        let shader = include_str!("shaders/sculpt.wgsl");
        let start = shader.find("struct Lights {").expect("sculpt.wgsl declares `struct Lights`");
        let body = &shader[start..];
        let body = &body[..body.find('}').expect("the struct closes")];
        let fields: Vec<String> = body
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//"))
            .map(str::to_string)
            .collect();
        let wanted = [
            "direction: array<vec4<f32>, 4>,",
            "colour: array<vec4<f32>, 4>,",
            "wrap: vec4<f32>,",
            "falloff: vec4<f32>,",
            "sky: vec4<f32>,",
            "ground: vec4<f32>,",
            "key: vec4<f32>,",
            "shade: vec4<f32>,",
            "f0: vec4<f32>,",
            "rim: vec4<f32>,",
            "through: vec4<f32>,",
            "paint: vec4<f32>,",
            "matcap: vec4<u32>,",
        ];
        assert_eq!(fields, wanted, "sculpt.wgsl's Lights and LightBlock disagree");
        use std::mem::offset_of;
        let offsets = [
            offset_of!(LightBlock, direction),
            offset_of!(LightBlock, colour),
            offset_of!(LightBlock, wrap),
            offset_of!(LightBlock, falloff),
            offset_of!(LightBlock, sky),
            offset_of!(LightBlock, ground),
            offset_of!(LightBlock, key),
            offset_of!(LightBlock, shade),
            offset_of!(LightBlock, f0),
            offset_of!(LightBlock, rim),
            offset_of!(LightBlock, through),
            offset_of!(LightBlock, paint),
            offset_of!(LightBlock, matcap),
        ];
        // Each a whole number of 16 byte rows after the last, as WGSL's
        // uniform layout puts arrays of vec4 and vec4s: 64 for the arrays.
        let sizes = [64, 64, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16];
        let mut at = 0;
        for (offset, size) in offsets.iter().zip(sizes) {
            assert_eq!(*offset, at);
            at += size;
        }
        assert_eq!(size_of::<LightBlock>(), at);
    }

    /// The shader's constants are these: read out of its source.
    #[test]
    fn the_shaders_constants_are_these() {
        let shader = include_str!("shaders/sculpt.wgsl");
        let read = |name: &str| -> f32 {
            shader
                .lines()
                .find_map(|line| line.trim().strip_prefix(&format!("const {name}: f32 = ")))
                .and_then(|rest| rest.trim_end_matches(';').trim().parse().ok())
                .unwrap_or_else(|| panic!("sculpt.wgsl declares `const {name}: f32 = <n>;`"))
        };
        assert_eq!(read("LIGHT_SIZE"), LIGHT_SIZE);
        assert_eq!(read("HORIZON"), HORIZON);
        assert_eq!(read("THIN_GLOW"), THIN_GLOW);
        assert_eq!(read("ROOM"), ROOM);
        assert_eq!(read("ROOM_POWER"), ROOM_POWER);
        assert_eq!(read("RIM_POWER"), matcap::RIM_POWER);
    }

    /// The sculpt shader translates to GLSL ES 3.00, which is what the
    /// browser build's WebGL2 runs: naga, as wgpu's GL backend uses it,
    /// writes both stages. This is not running it in a browser.
    #[test]
    fn the_shader_translates_for_webgl2() {
        let module = naga::front::wgsl::parse_str(include_str!("shaders/sculpt.wgsl"))
            .expect("sculpt.wgsl parses");
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("sculpt.wgsl validates with no capabilities beyond the base");
        for (stage, entry) in [
            (naga::ShaderStage::Vertex, "vertex_main"),
            (naga::ShaderStage::Fragment, "fragment_main"),
        ] {
            let options = naga::back::glsl::Options {
                version: naga::back::glsl::Version::Embedded { version: 300, is_webgl: true },
                ..Default::default()
            };
            let pipeline = naga::back::glsl::PipelineOptions {
                shader_stage: stage,
                entry_point: entry.to_string(),
                multiview: None,
            };
            let mut out = String::new();
            let mut writer = naga::back::glsl::Writer::new(
                &mut out,
                &module,
                &info,
                &options,
                &pipeline,
                naga::proc::BoundsCheckPolicies::default(),
            )
            .unwrap_or_else(|error| panic!("{entry} does not translate: {error}"));
            writer.write().unwrap_or_else(|error| panic!("{entry} does not translate: {error}"));
            assert!(out.contains("#version 300 es"), "{entry}");
        }
    }

    /// The shader's `finish_of` is `hani_core::paint::Finish`'s roughness
    /// and metal, code by code, read out of its source.
    #[test]
    fn the_shaders_finishes_are_the_engines() {
        use hani_core::paint::Finish;
        let shader = include_str!("shaders/sculpt.wgsl");
        for finish in Finish::ALL {
            let (rough, metal) = finish.roughness_and_metal();
            let line =
                format!("case {}u: {{ return vec2<f32>({rough:?}, {metal:?}); }}", finish.code());
            assert!(shader.contains(&line), "sculpt.wgsl's finish_of lacks `{line}`");
        }
    }
}
