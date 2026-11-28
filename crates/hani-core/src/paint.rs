// SPDX-License-Identifier: AGPL-3.0-only

//! Paints: what a painted voxel's number means, as a miniature painter means
//! it.
//!
//! A voxel already carries one byte of paint ([`crate::colour`]): a slot
//! number, which until now named a filament. This module is what lets that
//! number name a PAINT instead, and a paint here is not a colour. It is a
//! recipe: a base coat, a wash over it, and a highlight on its edges. The
//! voxel says which recipe; the recipe and the shape of the surface under it
//! say what colour.
//!
//! # Why not a colour per voxel, which is what the plan said
//!
//! `docs/STUDIO-PLAN.md` first drew painting as a second field of four bytes
//! a voxel, red, green, blue and a finish, written by brushes that deposit
//! colour. Three things were against it once the engine had been read.
//!
//! * **The slot field is woven through everything.** Rotating, shifting,
//!   resampling, warping, merging, splitting, the Move brush, undo, the
//!   project file: about 350 mentions across 17 files, each one a place the
//!   paint is carried and each with a test that fails without it. A second
//!   field means doing all of that again, in upstream's hottest files, on a
//!   fork that has to keep merging them.
//! * **It doubles the memory of a painted model**, +100% of the field against
//!   the +25% a slot costs, which `colour.rs` measured and gave as the reason
//!   a slot is an index in the first place.
//! * **A wash painted into colour is dead the moment it is painted.** It is
//!   right for the surface as it was. Sculpt a fold deeper and the shadow in
//!   it stays where the old fold was.
//!
//! A recipe has none of those. The voxel's byte is the one it already has,
//! so every transform, the undo and the file carry it without knowing. And a
//! wash is not stored anywhere: it is worked out from the surface each time
//! the surface is drawn, so it runs into a crease that was sculpted after the
//! paint went on, and changing a wash's colour changes it everywhere it was
//! used, at once, without touching a voxel.
//!
//! That is also how a contrast paint is sold: one pot that is a base coat, a
//! shade and a highlight, by how it behaves on the model.
//!
//! # What it costs
//!
//! **No blends.** A voxel is one paint or another, so the boundary between
//! two is an edge, at the half-triangle precision [`crate::mesh`] cuts it to,
//! and never a gradient. Speckle, stipple and a drybrush's broken edge are
//! all things an index can hold. A soft airbrushed fade from one colour to
//! another is not.
//!
//! **[`MAX_PAINTS`] of them**, because the shader is handed the whole table.
//!
//! # The arithmetic, and where its twin is
//!
//! [`Paint::shade`] is the colour of a paint at a point, given how concave the
//! surface is there. `sculpt.wgsl` computes the same thing per pixel, and
//! `the_shader_shades_a_paint_the_way_the_engine_does` in `hani-gpu` holds
//! the two together. An export bakes with this one.
//!
//! The wash is a film of pigment, by Beer and Lambert: a film of thickness
//! `T` passes `colour^T` of the light that reaches the surface under it. It
//! is thin on an edge, so the base shows through; it is `stain` thick on a
//! flat, which is the tint; and it pools in a crease, where `colour^3` is
//! darker AND more saturated, which is what a wash does and what multiplying
//! by grey does not.

pub mod bake;
pub mod carry;
pub mod coats;
pub mod decal;
pub mod field;
pub mod file;
pub mod fill;
pub mod layer_file;
#[cfg(test)]
mod layer_tests;
pub mod layers;
pub mod occlusion;
pub mod old_slots;
#[cfg(test)]
mod properties;
pub mod remove;
pub mod spray;
pub mod stroke;
pub mod touch;

/// The most paints a document can hold, not counting "unpainted".
///
/// A slot is a byte, so 255 is the ceiling the storage allows. This is the
/// ceiling the shader allows: it is handed every paint in a uniform, and 63
/// of them with their four vectors each is four kilobytes.
pub const MAX_PAINTS: usize = 63;

/// How glossy a paint is. A varnish layer's finish is drawn and exported
/// (`paint/layers.rs`, stage 6 of `docs/PAINT-RACK.md`); a slot paint's is
/// saved and not drawn yet.
///
/// Each is a surface in the viewport's one shading model
/// (`hani_gpu::lights`): a roughness and a metalness, by
/// [`Finish::roughness_and_metal`]. Metallic is a miniature paint's metal,
/// fine flakes in a medium, so fairly rough and grainy; Chrome is the rare
/// true chrome, nearly a mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Finish {
    #[default]
    Matte,
    Satin,
    Gloss,
    Metallic,
    /// Added on 2026-11-27, after the others, so that every byte written
    /// before it still means what it meant: its byte is 4 and its code 5.
    Chrome,
}

impl Finish {
    pub const ALL: [Finish; 5] =
        [Finish::Matte, Finish::Satin, Finish::Gloss, Finish::Metallic, Finish::Chrome];

    pub fn label(self) -> &'static str {
        match self {
            Finish::Matte => "Matte",
            Finish::Satin => "Satin",
            Finish::Gloss => "Gloss",
            Finish::Metallic => "Metallic",
            Finish::Chrome => "Chrome",
        }
    }

    /// Whether it is a metal: its colour is what it reflects, and nothing
    /// of it is diffuse.
    pub fn is_metal(self) -> bool {
        self.roughness_and_metal().1 >= 0.5
    }

    fn byte(self) -> u8 {
        self as u8
    }

    /// Its code in a varnish layer and on a vertex: one more than its byte,
    /// so that nought is no finish at all.
    pub fn code(self) -> u8 {
        self.byte() + 1
    }

    pub fn from_code(code: u8) -> Option<Self> {
        code.checked_sub(1).and_then(Self::from_byte)
    }

    /// How rough and how metal it looks, each 0..=1: what the viewport
    /// shades by and what an export's roughness and metal picture holds.
    /// The shader's `finish_of` is the other copy, and
    /// `hani_gpu::lights::tests` reads it to hold the two together.
    ///
    /// Metallic's roughness was 0.35 until 2026-11-27, when the viewport
    /// drew a metal by a formula of its own; under the microfacet model
    /// 0.35 read as polished, and 0.5 as a miniature paint's flake metal,
    /// judged on the contact sheet of `hani-gpu`'s offscreen tests.
    pub fn roughness_and_metal(self) -> (f32, f32) {
        match self {
            Finish::Matte => (0.9, 0.0),
            Finish::Satin => (0.5, 0.0),
            Finish::Gloss => (0.15, 0.0),
            Finish::Metallic => (0.5, 1.0),
            Finish::Chrome => (0.05, 1.0),
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|finish| finish.byte() == byte)
    }
}

impl std::fmt::Display for Finish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A film of thin pigment over the base coat.
///
/// All three amounts are thicknesses of film, in units where 1 is "the wash's
/// own colour over white".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wash {
    /// The pigment, sRGB.
    pub colour: [u8; 3],
    /// How thick the film lies on a flat: how much the whole area is tinted.
    /// A shade wash has little of this. A contrast paint has a lot.
    pub stain: f32,
    /// How much more gathers in a full crease.
    pub flow: f32,
    /// How much runs off a full edge. Where this takes the film to nothing,
    /// the base shows through untouched, which is the highlight a contrast
    /// paint leaves on its own.
    pub edge: f32,
}

impl Wash {
    /// No film at all. Multiplying by `colour^0` changes nothing.
    pub const NONE: Wash = Wash { colour: [0, 0, 0], stain: 0.0, flow: 0.0, edge: 0.0 };

    /// A dark shade that sits in the recesses and leaves the flats nearly
    /// clean.
    ///
    /// The stain is small because a film is an exponent: a dark brown passes
    /// about a twentieth of the light, and 0.15 of a film of it already takes
    /// 40% off a flat, which is a coat of paint and not a shade. At 0.06 it
    /// takes a fifth.
    pub fn shade(colour: [u8; 3]) -> Self {
        Self { colour, stain: 0.06, flow: 1.6, edge: 0.06 }
    }

    /// One coat that tints the flats, shades the recesses and thins on the
    /// edges: how a contrast or speed paint behaves over a pale undercoat.
    pub fn contrast(colour: [u8; 3]) -> Self {
        Self { colour, stain: 0.9, flow: 1.4, edge: 0.7 }
    }

    pub fn is_none(&self) -> bool {
        self.stain <= 0.0 && self.flow <= 0.0
    }
}

/// A lighter colour caught on raised edges: a drybrush, or an edge highlight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    /// sRGB.
    pub colour: [u8; 3],
    /// How opaque it is at a full edge, 0..=1.
    pub amount: f32,
    /// How sharp an edge has to be before any lands, as a fraction of a full
    /// edge, 0..1. Low drybrushes the whole of every raised form; high picks
    /// out only the sharpest lines.
    pub start: f32,
}

impl Highlight {
    pub const NONE: Highlight = Highlight { colour: [255, 255, 255], amount: 0.0, start: 0.2 };

    pub fn is_none(&self) -> bool {
        self.amount <= 0.0
    }
}

/// One paint: a base coat, a wash and a highlight.
#[derive(Debug, Clone, PartialEq)]
pub struct Paint {
    pub name: String,
    /// The base coat, sRGB.
    pub base: [u8; 3],
    pub finish: Finish,
    pub wash: Wash,
    pub highlight: Highlight,
    /// The filament this paint prints in on a machine that prints in
    /// filaments, 1-based, or 0 for whichever loaded filament is nearest its
    /// colour. See [`bake`]. Nothing on screen reads it.
    pub filament: u8,
    /// How much of a zenithal undercoat shows through the base coat, 0..=1:
    /// lighter where the surface faces up and darker where it faces down,
    /// as a model primed dark from below and light from above is. Nothing
    /// changes on a surface that faces sideways, so a paint's own colour
    /// is the colour of its upright faces. See [`Paint::shade_facing`].
    pub zenith: f32,
}

/// How far a full zenithal undercoat lifts a surface facing straight up,
/// and how far it darkens one facing straight down, as fractions of the
/// base coat. Uneven for the reason a crease is darkened further than an
/// edge is lifted: a shadow can go most of the way to black and still read
/// as that paint, and a light lifted as far would clip. The shader has the
/// same two numbers.
pub const ZENITH_LIFT: f32 = 0.35;
pub const ZENITH_SHADE: f32 = 0.55;

/// The longest a paint's name is kept, in bytes.
pub const MAX_PAINT_NAME_BYTES: usize = 48;

impl Paint {
    /// A plain base coat of one colour.
    pub fn flat(name: impl Into<String>, base: [u8; 3]) -> Self {
        Self {
            name: name.into(),
            base,
            finish: Finish::Matte,
            wash: Wash::NONE,
            highlight: Highlight::NONE,
            filament: 0,
            zenith: 0.0,
        }
    }

    /// The colour of this paint, in linear light, where the surface is as
    /// concave as `crease` says.
    ///
    /// `crease` is a fraction of a full crease: 1 at the feature size the
    /// viewer calls a crease, 0 on a flat, -1 on a full edge. It is the
    /// decoded cavity ([`crate::signals`]) times the view's gain, clamped.
    ///
    /// Not lit. The viewport multiplies this by its lighting and an export
    /// writes it as it is.
    ///
    /// On a surface that faces sideways, which is where a zenithal
    /// undercoat changes nothing: what a rack or a legend shows.
    pub fn shade(&self, crease: f32) -> [f32; 3] {
        self.shade_facing(crease, 0.0)
    }

    /// [`Paint::shade`] on a surface whose normal has `up` of itself
    /// pointing up: 1 for a surface facing straight up, -1 for one facing
    /// straight down.
    ///
    /// The order is the painter's. The undercoat is under everything, so
    /// the base coat is lifted or darkened by it first; the wash is a film
    /// over that; and the highlight is laid on last and is not shaded by
    /// either.
    pub fn shade_facing(&self, crease: f32, up: f32) -> [f32; 3] {
        let crease = crease.clamp(-1.0, 1.0);
        let recess = crease.max(0.0);
        let edge = (-crease).max(0.0);
        let mut colour = linear(self.base);
        let zenith = self.zenith.clamp(0.0, 1.0);
        if zenith > 0.0 {
            let up = up.clamp(-1.0, 1.0);
            let undercoat =
                1.0 + zenith * (ZENITH_LIFT * up.max(0.0) - ZENITH_SHADE * (-up).max(0.0));
            for channel in &mut colour {
                *channel *= undercoat;
            }
        }

        if !self.wash.is_none() {
            let film = (self.wash.stain + self.wash.flow * recess - self.wash.edge * edge).max(0.0);
            let pigment = linear(self.wash.colour);
            for channel in 0..3 {
                // A pigment that passes no light at all in a channel would be
                // `0^0` on an edge. Held just off zero, where the answer is
                // the limit: no film, no change.
                colour[channel] *= pigment[channel].max(WASH_FLOOR).powf(film);
            }
        }

        if !self.highlight.is_none() {
            let caught = smoothstep(self.highlight.start, 1.0, edge) * self.highlight.amount;
            let light = linear(self.highlight.colour);
            for channel in 0..3 {
                colour[channel] += (light[channel] - colour[channel]) * caught.clamp(0.0, 1.0);
            }
        }
        colour
    }
}

/// The least light a wash's pigment is taken to pass, per channel. See
/// [`Paint::shade`]. The shader has the same number.
pub const WASH_FLOOR: f32 = 0.003;

/// sRGB bytes to linear light.
pub fn linear(srgb: [u8; 3]) -> [f32; 3] {
    srgb.map(|byte| {
        let c = f32::from(byte) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    })
}

/// Linear light back to sRGB bytes, for an export.
pub fn srgb(linear: [f32; 3]) -> [u8; 3] {
    linear.map(|c| {
        let c = c.clamp(0.0, 1.0);
        let encoded = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (encoded * 255.0).round() as u8
    })
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    if to <= from {
        return if x >= to { 1.0 } else { 0.0 };
    }
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A document's paints. Slot `n` of the voxel field is paint `n - 1` here,
/// and slot 0 is unpainted.
///
/// **Empty means the document is not using paints at all**, and its slots
/// are filament slots, as they were before this existed. That is every
/// document made before now and every one whose owner only prints.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintTable {
    paints: Vec<Paint>,
}

/// Why a paint table could not be read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintError {
    /// The bytes stop before the table does.
    Truncated,
    /// More paints than a table may hold.
    TooMany(usize),
    /// A name is longer than is kept, or is not text.
    BadName,
    /// A finish byte names no finish.
    BadFinish(u8),
    /// An amount is not a finite number.
    NotFinite,
    /// There are bytes left over after the last paint.
    Trailing,
    /// The table is a version this build does not know.
    Version(u8),
}

impl std::fmt::Display for PaintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaintError::Truncated => f.write_str("the paint table is cut short"),
            PaintError::TooMany(count) => {
                write!(f, "the paint table holds {count} paints, more than {MAX_PAINTS}")
            }
            PaintError::BadName => f.write_str("a paint's name is not readable"),
            PaintError::BadFinish(byte) => write!(f, "a paint has an unknown finish ({byte})"),
            PaintError::NotFinite => f.write_str("a paint holds a number that is not a number"),
            PaintError::Trailing => f.write_str("there is something after the last paint"),
            PaintError::Version(version) => {
                write!(f, "the paint table is version {version}, which this build cannot read")
            }
        }
    }
}

/// The version byte a table is written with.
///
/// Version 1 had no filament byte. It was only ever written by builds of
/// this branch on the day it was made, and is still read: a paint from one
/// names no filament and takes the nearest. Version 2 had no zenithal
/// undercoat, and a paint from one has none.
const TABLE_VERSION: u8 = 3;
/// The oldest version [`PaintTable::decode`] reads.
const OLDEST_TABLE_VERSION: u8 = 1;

impl PaintTable {
    pub fn is_empty(&self) -> bool {
        self.paints.is_empty()
    }

    pub fn len(&self) -> usize {
        self.paints.len()
    }

    pub fn paints(&self) -> &[Paint] {
        &self.paints
    }

    /// The paint a slot names, or `None` for slot 0 and for a slot past the
    /// end of the table.
    pub fn paint(&self, slot: u8) -> Option<&Paint> {
        self.paints.get(usize::from(slot).checked_sub(1)?)
    }

    pub fn paint_mut(&mut self, slot: u8) -> Option<&mut Paint> {
        self.paints.get_mut(usize::from(slot).checked_sub(1)?)
    }

    /// Add a paint and return the slot that now names it, or `None` when the
    /// table is full.
    pub fn add(&mut self, mut paint: Paint) -> Option<u8> {
        if self.paints.len() >= MAX_PAINTS {
            return None;
        }
        paint.name = fitted(&paint.name);
        self.paints.push(paint);
        Some(self.paints.len() as u8)
    }

    /// The colour slot `slot` draws in where the surface is as concave as
    /// `crease` says, or `None` where there is no paint to draw.
    pub fn shade(&self, slot: u8, crease: f32) -> Option<[f32; 3]> {
        self.paint(slot).map(|paint| paint.shade(crease))
    }

    /// The table as bytes, for the project file.
    ///
    /// Little endian, fixed order, no padding: writing one table twice gives
    /// the same bytes, which the project file's own tests require of
    /// everything in it.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![TABLE_VERSION, self.paints.len() as u8];
        for paint in &self.paints {
            let name = fitted(&paint.name);
            out.push(name.len() as u8);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&paint.base);
            out.push(paint.finish.byte());
            out.push(paint.filament);
            out.extend_from_slice(&paint.wash.colour);
            for amount in [paint.wash.stain, paint.wash.flow, paint.wash.edge] {
                out.extend_from_slice(&amount.to_le_bytes());
            }
            out.extend_from_slice(&paint.highlight.colour);
            for amount in [paint.highlight.amount, paint.highlight.start, paint.zenith] {
                out.extend_from_slice(&amount.to_le_bytes());
            }
        }
        out
    }

    /// Read a table back. Every count and length is checked before it is
    /// used, and anything left over is an error and not ignored.
    pub fn decode(bytes: &[u8]) -> Result<Self, PaintError> {
        let mut rest = bytes;
        let mut take = |count: usize| -> Result<&[u8], PaintError> {
            if rest.len() < count {
                return Err(PaintError::Truncated);
            }
            let (head, tail) = rest.split_at(count);
            rest = tail;
            Ok(head)
        };
        let version = take(1)?[0];
        if !(OLDEST_TABLE_VERSION..=TABLE_VERSION).contains(&version) {
            return Err(PaintError::Version(version));
        }
        let count = usize::from(take(1)?[0]);
        if count > MAX_PAINTS {
            return Err(PaintError::TooMany(count));
        }

        let mut paints = Vec::with_capacity(count);
        for _ in 0..count {
            let name_len = usize::from(take(1)?[0]);
            if name_len > MAX_PAINT_NAME_BYTES {
                return Err(PaintError::BadName);
            }
            let name = std::str::from_utf8(take(name_len)?).map_err(|_| PaintError::BadName)?;
            let name = name.to_string();
            let colour = |bytes: &[u8]| [bytes[0], bytes[1], bytes[2]];
            let base = colour(take(3)?);
            let finish_byte = take(1)?[0];
            let finish =
                Finish::from_byte(finish_byte).ok_or(PaintError::BadFinish(finish_byte))?;
            // Any byte is a filament: one the machine does not have is
            // treated as none when the paint is printed, since the machine
            // is not the document's to know.
            let filament = if version >= 2 { take(1)?[0] } else { 0 };
            let wash_colour = colour(take(3)?);
            let mut amount = || -> Result<f32, PaintError> {
                let bytes = take(4)?;
                let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if value.is_finite() { Ok(value) } else { Err(PaintError::NotFinite) }
            };
            let (stain, flow, edge) = (amount()?, amount()?, amount()?);
            let highlight_colour = {
                let bytes = take(3)?;
                [bytes[0], bytes[1], bytes[2]]
            };
            let mut amount = || -> Result<f32, PaintError> {
                let bytes = take(4)?;
                let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if value.is_finite() { Ok(value) } else { Err(PaintError::NotFinite) }
            };
            let (amount_at_edge, start) = (amount()?, amount()?);
            let zenith = if version >= 3 { amount()? } else { 0.0 };
            paints.push(Paint {
                name,
                base,
                finish,
                wash: Wash { colour: wash_colour, stain, flow, edge },
                highlight: Highlight { colour: highlight_colour, amount: amount_at_edge, start },
                filament,
                zenith,
            });
        }
        if !rest.is_empty() {
            return Err(PaintError::Trailing);
        }
        Ok(Self { paints })
    }
}

/// A name cut to what is kept, on a character boundary.
fn fitted(name: &str) -> String {
    let mut end = name.len().min(MAX_PAINT_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Noise;

    fn luma(colour: [f32; 3]) -> f32 {
        0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2]
    }

    /// How far a colour is from grey, as a fraction of its brightest channel.
    fn saturation(colour: [f32; 3]) -> f32 {
        let max = colour[0].max(colour[1]).max(colour[2]);
        let min = colour[0].min(colour[1]).min(colour[2]);
        if max <= 0.0 { 0.0 } else { (max - min) / max }
    }

    fn bone() -> Paint {
        Paint::flat("bone", [222, 208, 170])
    }

    #[test]
    fn a_flat_paint_is_its_base_colour_whatever_the_surface_does() {
        let paint = bone();
        let base = linear(paint.base);
        for crease in [-1.0, -0.4, 0.0, 0.3, 1.0, 7.0, -7.0] {
            assert_eq!(paint.shade(crease), base, "at a crease of {crease}");
        }
    }

    #[test]
    fn a_wash_is_darker_and_more_saturated_in_a_crease_and_leaves_an_edge_alone() {
        let paint = Paint { wash: Wash::shade([70, 40, 25]), ..bone() };
        let base = linear(paint.base);
        let flat = paint.shade(0.0);
        let crease = paint.shade(1.0);
        let edge = paint.shade(-1.0);

        assert!(
            luma(crease) < luma(flat) * 0.5,
            "a full crease is {} against {}",
            luma(crease),
            luma(flat)
        );
        assert!(luma(flat) < luma(base), "a shade wash leaves the flats untinted");
        assert!(
            luma(flat) > luma(base) * 0.75,
            "a shade wash darkened the flats to {} of {}",
            luma(flat),
            luma(base)
        );
        // The one thing a multiply by grey gets wrong: a pooled wash is not
        // just darker, it is more strongly the wash's colour.
        assert!(
            saturation(crease) > saturation(flat) + 0.1,
            "a crease is {} saturated and a flat {}",
            saturation(crease),
            saturation(flat)
        );
        // The film has run off the edge entirely, so the base shows.
        assert_eq!(edge, base, "an edge under a shade wash is not the base coat");

        // And it deepens steadily: no step, and never lighter further in.
        let mut last = luma(paint.shade(-1.0));
        for step in -19..=20 {
            let here = luma(paint.shade(step as f32 / 20.0));
            assert!(here <= last + 1.0e-6, "the wash got lighter at {step}");
            last = here;
        }
    }

    #[test]
    fn a_contrast_paint_tints_the_flats_and_thins_on_the_edges() {
        // Over a pale undercoat, one coat: the flats take the colour, the
        // recesses go deep, the edges come out lighter than the flats.
        let paint =
            Paint { wash: Wash::contrast([30, 60, 150]), ..Paint::flat("primer", [235, 235, 235]) };
        let (edge, flat, crease) = (paint.shade(-1.0), paint.shade(0.0), paint.shade(1.0));
        assert!(saturation(flat) > 0.5, "the flats are barely tinted: {}", saturation(flat));
        assert!(luma(edge) > luma(flat) * 1.5, "the edges are no lighter than the flats");
        assert!(luma(crease) < luma(flat) * 0.6, "the recesses are no deeper than the flats");
        assert!(flat[2] > flat[0] * 2.0, "a blue contrast paint did not come out blue");
    }

    #[test]
    fn a_highlight_lands_on_edges_past_its_start_and_nowhere_else() {
        let paint = Paint {
            highlight: Highlight { colour: [255, 255, 255], amount: 0.8, start: 0.3 },
            ..Paint::flat("leather", [90, 55, 30])
        };
        let base = linear(paint.base);
        assert_eq!(paint.shade(0.0), base, "a flat was highlighted");
        assert_eq!(paint.shade(0.6), base, "a crease was highlighted");
        assert_eq!(paint.shade(-0.3), base, "an edge at the start was highlighted");
        let half = paint.shade(-0.65);
        let full = paint.shade(-1.0);
        assert!(luma(half) > luma(base) && luma(half) < luma(full));
        // At a full edge it is `amount` of the way to the highlight.
        let expected = base[0] + (1.0 - base[0]) * 0.8;
        assert!((full[0] - expected).abs() < 1.0e-5, "{} against {expected}", full[0]);
    }

    #[test]
    fn a_wash_of_pure_black_on_an_edge_is_no_wash() {
        // `0^0`. The film is nothing, so the answer has to be the base.
        let paint =
            Paint { wash: Wash { colour: [0, 0, 0], stain: 0.0, flow: 2.0, edge: 0.0 }, ..bone() };
        let base = linear(paint.base);
        let flat = paint.shade(0.0);
        for channel in 0..3 {
            assert!(flat[channel].is_finite());
            assert!((flat[channel] - base[channel]).abs() < 1.0e-6, "{flat:?} against {base:?}");
        }
        assert!(luma(paint.shade(1.0)) < 1.0e-3, "pooled black is not black");
    }

    #[test]
    fn srgb_and_linear_are_each_others_inverse() {
        for byte in 0..=255u8 {
            assert_eq!(srgb(linear([byte, byte, byte])), [byte, byte, byte]);
        }
        assert_eq!(linear([0, 0, 0]), [0.0, 0.0, 0.0]);
        assert!((linear([255, 255, 255])[0] - 1.0).abs() < 1.0e-6);
        // Mid grey in sRGB is about a fifth of the light.
        assert!((linear([128, 128, 128])[0] - 0.2158).abs() < 1.0e-3);
    }

    #[test]
    fn slot_zero_is_unpainted_and_a_slot_past_the_table_is_nothing() {
        let mut table = PaintTable::default();
        assert!(table.is_empty());
        assert_eq!(table.shade(1, 0.0), None, "an empty table shaded a slot");
        let slot = table.add(bone()).expect("room for one");
        assert_eq!(slot, 1);
        assert_eq!(table.paint(0), None, "slot 0 is unpainted and named a paint");
        assert_eq!(table.paint(1).map(|paint| paint.name.as_str()), Some("bone"));
        assert_eq!(table.paint(2), None);
        assert_eq!(table.shade(1, 0.0), Some(linear([222, 208, 170])));
    }

    #[test]
    fn a_full_table_takes_no_more() {
        let mut table = PaintTable::default();
        for index in 0..MAX_PAINTS {
            assert_eq!(table.add(bone()), Some(index as u8 + 1));
        }
        assert_eq!(table.add(bone()), None);
        assert_eq!(table.len(), MAX_PAINTS);
    }

    fn a_table() -> PaintTable {
        let mut table = PaintTable::default();
        table.add(bone());
        table.add(Paint {
            name: "ünïcödé leather".to_string(),
            finish: Finish::Satin,
            wash: Wash::shade([70, 40, 25]),
            highlight: Highlight { colour: [240, 220, 200], amount: 0.6, start: 0.25 },
            zenith: 0.4,
            ..Paint::flat("x", [90, 55, 30])
        });
        table.add(Paint {
            finish: Finish::Metallic,
            wash: Wash::contrast([30, 60, 150]),
            filament: 3,
            zenith: 1.0,
            ..Paint::flat("", [200, 200, 205])
        });
        table
    }

    /// The table as version 2 wrote it: a filament byte, and no undercoat.
    fn as_version_two(table: &PaintTable) -> Vec<u8> {
        let mut out = vec![2, table.len() as u8];
        for paint in table.paints() {
            out.push(paint.name.len() as u8);
            out.extend_from_slice(paint.name.as_bytes());
            out.extend_from_slice(&paint.base);
            out.push(paint.finish.byte());
            out.push(paint.filament);
            out.extend_from_slice(&paint.wash.colour);
            for amount in [paint.wash.stain, paint.wash.flow, paint.wash.edge] {
                out.extend_from_slice(&amount.to_le_bytes());
            }
            out.extend_from_slice(&paint.highlight.colour);
            for amount in [paint.highlight.amount, paint.highlight.start] {
                out.extend_from_slice(&amount.to_le_bytes());
            }
        }
        out
    }

    /// A table from before the zenithal undercoat reads, every paint as it
    /// was and with no undercoat. The current version is not those bytes
    /// relabelled: it is four bytes a paint longer, and carries the
    /// undercoat back.
    #[test]
    fn a_table_from_before_the_undercoat_still_reads_with_none() {
        let table = a_table();
        assert!(table.paints().iter().any(|paint| paint.zenith != 0.0), "the fixture has none");
        let old = PaintTable::decode(&as_version_two(&table)).expect("version 2 was refused");
        assert_eq!(old.len(), table.len());
        for (old, new) in old.paints().iter().zip(table.paints()) {
            assert_eq!(*old, Paint { zenith: 0.0, ..new.clone() });
        }
        let mut relabelled = as_version_two(&table);
        relabelled[0] = TABLE_VERSION;
        assert!(PaintTable::decode(&relabelled).is_err(), "version 2 bytes read as version 3");
        assert_eq!(table.encode().len(), as_version_two(&table).len() + 4 * table.len());
        assert_eq!(PaintTable::decode(&table.encode()).expect("the current version"), table);
    }

    /// A zenithal undercoat lifts what faces up, darkens what faces down by
    /// more, and leaves what faces sideways alone, in proportion to how
    /// much of it the paint is given. A wash goes over it and a highlight
    /// over both.
    #[test]
    fn an_undercoat_lifts_what_faces_up_and_darkens_what_faces_down() {
        let plain = bone();
        let primed = Paint { zenith: 1.0, ..bone() };
        let half = Paint { zenith: 0.5, ..bone() };
        let flat = luma(plain.shade(0.0));
        // No undercoat: the way a surface faces is nothing to the paint.
        for up in [-1.0, -0.3, 0.0, 0.6, 1.0] {
            assert_eq!(plain.shade_facing(0.0, up), plain.shade(0.0));
        }
        // Sideways, and what a rack shows, is the paint's own colour.
        assert_eq!(primed.shade_facing(0.0, 0.0), plain.shade(0.0));
        assert_eq!(primed.shade(0.0), plain.shade(0.0));

        let ratio = |paint: &Paint, up: f32| luma(paint.shade_facing(0.0, up)) / flat;
        assert!((ratio(&primed, 1.0) - (1.0 + ZENITH_LIFT)).abs() < 1e-5);
        assert!((ratio(&primed, -1.0) - (1.0 - ZENITH_SHADE)).abs() < 1e-5);
        assert!((ratio(&primed, 0.5) - (1.0 + ZENITH_LIFT / 2.0)).abs() < 1e-5);
        assert!((ratio(&primed, -0.5) - (1.0 - ZENITH_SHADE / 2.0)).abs() < 1e-5);
        assert!((ratio(&half, 1.0) - (1.0 + ZENITH_LIFT / 2.0)).abs() < 1e-5);
        assert!((ratio(&half, -1.0) - (1.0 - ZENITH_SHADE / 2.0)).abs() < 1e-5);
        // Past straight up is straight up, and more than all of an
        // undercoat is all of it.
        assert_eq!(primed.shade_facing(0.0, 7.0), primed.shade_facing(0.0, 1.0));
        assert_eq!(primed.shade_facing(0.0, -7.0), primed.shade_facing(0.0, -1.0));
        let over = Paint { zenith: 9.0, ..bone() };
        assert_eq!(over.shade_facing(0.0, -1.0), primed.shade_facing(0.0, -1.0));
        let under = Paint { zenith: -3.0, ..bone() };
        assert_eq!(under.shade_facing(0.0, -1.0), plain.shade(0.0));

        // A wash is a film over the undercoat: the same share of the light
        // comes through it whichever way the surface under it faces.
        let washed = Paint { wash: Wash::shade([70, 40, 25]), ..primed.clone() };
        let through =
            |up: f32| luma(washed.shade_facing(0.8, up)) / luma(primed.shade_facing(0.8, up));
        assert!(
            (through(1.0) - through(-1.0)).abs() < 0.02,
            "{} and {}",
            through(1.0),
            through(-1.0)
        );
        assert!(through(0.0) < 0.9, "the wash took nothing off");
        // A highlight laid on full is the highlight's colour, undercoat or
        // none, up or down.
        let lit = Paint {
            highlight: Highlight { colour: [250, 240, 230], amount: 1.0, start: 0.0 },
            ..primed.clone()
        };
        assert_eq!(lit.shade_facing(-1.0, -1.0), lit.shade_facing(-1.0, 1.0));
        assert_eq!(lit.shade_facing(-1.0, -1.0), linear([250, 240, 230]));
    }

    /// A table as the first version of the format wrote it: the same bytes
    /// without the filament byte after each finish.
    fn as_version_one(table: &PaintTable) -> Vec<u8> {
        let mut out = vec![1, table.len() as u8];
        for paint in table.paints() {
            out.push(paint.name.len() as u8);
            out.extend_from_slice(paint.name.as_bytes());
            out.extend_from_slice(&paint.base);
            out.push(paint.finish.byte());
            out.extend_from_slice(&paint.wash.colour);
            for amount in [paint.wash.stain, paint.wash.flow, paint.wash.edge] {
                out.extend_from_slice(&amount.to_le_bytes());
            }
            out.extend_from_slice(&paint.highlight.colour);
            for amount in [paint.highlight.amount, paint.highlight.start] {
                out.extend_from_slice(&amount.to_le_bytes());
            }
        }
        out
    }

    /// A table written before paints named a filament still reads, every
    /// paint naming none. And a version from the future is refused by name
    /// and not read as something it is not.
    #[test]
    fn a_table_from_before_filaments_still_reads_and_a_newer_one_is_refused() {
        let table = a_table();
        assert!(table.paints().iter().any(|paint| paint.filament != 0), "the fixture names none");
        let old = PaintTable::decode(&as_version_one(&table)).expect("version 1 was refused");
        assert_eq!(old.len(), table.len());
        for (old, new) in old.paints().iter().zip(table.paints()) {
            assert_eq!(old.filament, 0, "a version 1 paint named a filament");
            assert_eq!(*old, Paint { filament: 0, zenith: 0.0, ..new.clone() });
        }
        // The control for the version switch: the same bytes relabelled as
        // the current version are one byte a paint short and do not read.
        let mut relabelled = as_version_one(&table);
        relabelled[0] = TABLE_VERSION;
        assert!(PaintTable::decode(&relabelled).is_err(), "version 1 bytes read as version 2");

        let mut newer = table.encode();
        newer[0] = TABLE_VERSION + 1;
        assert_eq!(PaintTable::decode(&newer), Err(PaintError::Version(TABLE_VERSION + 1)));
        newer[0] = 0;
        assert_eq!(PaintTable::decode(&newer), Err(PaintError::Version(0)));
    }

    #[test]
    fn a_table_survives_being_written_and_read() {
        let table = a_table();
        let bytes = table.encode();
        assert_eq!(PaintTable::decode(&bytes), Ok(table.clone()));
        assert_eq!(table.encode(), bytes, "writing the same table twice gave different bytes");
        assert_eq!(PaintTable::decode(&PaintTable::default().encode()), Ok(PaintTable::default()));
    }

    #[test]
    fn a_long_name_is_cut_on_a_character_and_still_reads_back() {
        let mut table = PaintTable::default();
        // Two bytes a character, so a cut at an odd byte would split one.
        table.add(Paint::flat("é".repeat(40), [1, 2, 3]));
        let kept = &table.paints()[0].name;
        assert!(kept.len() <= MAX_PAINT_NAME_BYTES);
        assert!(kept.chars().all(|c| c == 'é'), "the name was cut through a character");
        assert_eq!(PaintTable::decode(&table.encode()), Ok(table));
    }

    #[test]
    fn a_damaged_table_gets_an_answer_and_never_a_panic() {
        // Every single byte flipped to every value that changes it, every
        // truncation, and noise appended: each must decode to a table or to an
        // error.
        let bytes = a_table().encode();
        let mut refused = 0;
        let mut accepted = 0;
        let mut tally = |result: Result<PaintTable, PaintError>| match result {
            Ok(table) => {
                accepted += 1;
                // Whatever came back has to be something the rest can use.
                assert!(table.len() <= MAX_PAINTS);
                for paint in table.paints() {
                    assert!(paint.name.len() <= MAX_PAINT_NAME_BYTES);
                    let colour = paint.shade(0.5);
                    assert!(colour.iter().all(|c| c.is_finite()));
                }
            }
            Err(_) => refused += 1,
        };
        for cut in 0..bytes.len() {
            tally(PaintTable::decode(&bytes[..cut]));
        }
        let mut noise = Noise::seeded(0x9A17);
        for _ in 0..20_000 {
            let mut mutant = bytes.clone();
            for _ in 0..=noise.below(3) {
                let at = noise.below(mutant.len());
                mutant[at] = noise.byte();
            }
            if noise.below(8) == 0 {
                mutant.push(noise.byte());
            }
            tally(PaintTable::decode(&mutant));
        }
        // The control. A reader that refused everything at its first byte
        // would pass the loop above without one mutant reaching the paints:
        // most mutations land in a colour or an amount and must be ACCEPTED.
        assert!(accepted > 5_000, "only {accepted} mutants were read as tables");
        assert!(refused > 1_000, "only {refused} mutants were refused");
    }

    /// A small sculpted document and the bytes it writes, with and without
    /// paints.
    fn written(paints: PaintTable) -> (crate::body::Document, Vec<u8>) {
        use crate::project::ProjectState;

        let mut volume = crate::volume::Volume::new(0.5);
        volume.seed_sphere(glam::Vec3::splat(10.0), 6.0);
        let doc = crate::body::Document::from_volume(volume);
        let state = ProjectState { paints, ..ProjectState::default() };
        let mut bytes = Vec::new();
        crate::project::write(&mut bytes, &doc, &state).expect("the fixture writes");
        (doc, bytes)
    }

    #[test]
    fn a_documents_paints_are_saved_with_it_and_cost_one_without_them_nothing() {
        let (_, plain) = written(PaintTable::default());
        let (doc, painted) = written(a_table());

        // The whole of what paints add is a tail. Everything before it is the
        // file as it has always been written, byte for byte, which is what a
        // build from before paints reads and then stops.
        assert!(painted.len() > plain.len());
        assert_eq!(&painted[..plain.len()], &plain[..], "paints changed the file before its tail");
        let tail = &painted[plain.len()..];
        assert_eq!(tail.len(), 8 + 4 + a_table().encode().len());

        let (read_doc, state) =
            crate::project::read(&mut painted.as_slice()).expect("a painted file reads");
        assert_eq!(state.paints, a_table());
        crate::testing::assert_same_field(
            read_doc.active_volume(),
            doc.active_volume(),
            "the geometry of a painted file",
        );
        // And a file without them reads with none.
        let (_, state) = crate::project::read(&mut plain.as_slice()).expect("a plain file reads");
        assert!(state.paints.is_empty());
    }

    #[test]
    fn a_damaged_paint_tail_costs_the_paints_and_never_the_sculpt() {
        let (doc, plain) = written(PaintTable::default());
        let (_, painted) = written(a_table());
        let opens_whole = |bytes: &[u8], what: &str| -> PaintTable {
            let (read_doc, state) = crate::project::read(&mut &bytes[..])
                .unwrap_or_else(|error| panic!("{what}: the file would not open: {error}"));
            crate::testing::assert_same_field(read_doc.active_volume(), doc.active_volume(), what);
            state.paints
        };

        // Cut anywhere in the tail.
        let mut lost = 0;
        for cut in plain.len()..painted.len() {
            let paints = opens_whole(&painted[..cut], "a tail cut short");
            assert!(paints.is_empty(), "a tail cut at {cut} still gave a table");
            lost += 1;
        }
        assert!(lost > 100, "the tail is only {lost} bytes, so little was tried");

        // A tag that is not the paint tag, a length past the cap, and noise
        // where the table goes.
        let mut wrong_tag = painted.clone();
        wrong_tag[plain.len()] ^= 0xFF;
        assert!(opens_whole(&wrong_tag, "a wrong tag").is_empty());
        let mut huge = painted.clone();
        huge[plain.len() + 8..plain.len() + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(opens_whole(&huge, "an absurd length").is_empty());
        let mut noise = Noise::seeded(0xB0A7);
        let mut kept = 0;
        for _ in 0..2_000 {
            let mut mutant = painted.clone();
            let at = plain.len() + noise.below(mutant.len() - plain.len());
            mutant[at] = noise.byte();
            // Whatever it reads as, it reads, and the sculpt is whole.
            kept += usize::from(!opens_whole(&mutant, "a damaged tail").is_empty());
        }
        // The control: most single byte changes land in a colour and leave a
        // table that still decodes. If none did, the reader would be refusing
        // every tail and the loop above would prove nothing about reading one.
        assert!(kept > 500, "only {kept} of 2000 damaged tails still read as a table");
    }

    #[test]
    fn each_way_a_table_can_be_wrong_is_named() {
        let good = a_table().encode();
        assert_eq!(PaintTable::decode(&[]), Err(PaintError::Truncated));
        assert_eq!(PaintTable::decode(&[9, 0]), Err(PaintError::Version(9)));
        assert_eq!(PaintTable::decode(&[TABLE_VERSION, 200]), Err(PaintError::TooMany(200)));
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(PaintTable::decode(&trailing), Err(PaintError::Trailing));
        assert_eq!(PaintTable::decode(&good[..good.len() - 1]), Err(PaintError::Truncated));
        // The first paint's name length, set past what is kept.
        let mut long = good.clone();
        long[2] = MAX_PAINT_NAME_BYTES as u8 + 1;
        assert_eq!(PaintTable::decode(&long), Err(PaintError::BadName));
        // Its finish byte: name length, the name, three of base, then it.
        let finish_at = 3 + usize::from(good[2]) + 3;
        let mut finish = good.clone();
        finish[finish_at] = 77;
        assert_eq!(PaintTable::decode(&finish), Err(PaintError::BadFinish(77)));
        // And a NaN where an amount goes: the filament byte, three of wash
        // colour, then the stain.
        let mut nan = good;
        nan[finish_at + 5..finish_at + 9].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(PaintTable::decode(&nan), Err(PaintError::NotFinite));
    }
}
