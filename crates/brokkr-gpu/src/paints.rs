// SPDX-License-Identifier: AGPL-3.0-only

//! A document's paints, as the shader is handed them.
//!
//! [`brokkr_core::paint`] says what a paint is and how it shades. This is the
//! same table laid out for a uniform buffer: four vectors a paint, every
//! colour already in linear light, indexed by the slot byte a voxel carries.
//! `sculpt.wgsl` reads it and repeats [`brokkr_core::paint::Paint::shade`]'s
//! arithmetic per pixel; `the_shader_shades_a_paint_the_way_the_engine_does`
//! holds the two together.

use brokkr_core::paint::{MAX_PAINTS, PaintTable, linear};
use bytemuck::{Pod, Zeroable};

/// Slots the block has room for, counting the unpainted zero.
pub const PAINT_SLOTS: usize = MAX_PAINTS + 1;

/// Every paint of a document, by slot. Slot 0 is unpainted and is never read.
///
/// Four parallel arrays of `vec4` and not one array of a struct, so that the
/// layout is the same in WGSL and in Rust with nothing to align: every entry
/// is 16 bytes at a multiple of 16.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct PaintBlock {
    /// Base coat, linear RGB. **`w` is 1 where the slot names a paint and 0
    /// where it does not**, which is how the shader tells a document's paint
    /// from a filament slot: an empty table is all zeros, and a document with
    /// no paints draws exactly as it did before paints existed.
    pub base: [[f32; 4]; PAINT_SLOTS],
    /// The wash's pigment, linear RGB, and in `w` its stain.
    pub wash: [[f32; 4]; PAINT_SLOTS],
    /// `[wash flow, wash edge, highlight amount, highlight start]`.
    pub film: [[f32; 4]; PAINT_SLOTS],
    /// The highlight's colour, linear RGB. `w` is spare.
    pub light: [[f32; 4]; PAINT_SLOTS],
}

const _: () = assert!(
    size_of::<PaintBlock>() == 4 * 16 * PAINT_SLOTS,
    "PaintBlock must stay four vec4 arrays, in step with sculpt.wgsl"
);

impl Default for PaintBlock {
    /// No paints. Written out because arrays this long have no `Default`.
    fn default() -> Self {
        Self::zeroed()
    }
}

impl From<&PaintTable> for PaintBlock {
    fn from(table: &PaintTable) -> Self {
        let mut block = Self::zeroed();
        for (index, paint) in table.paints().iter().enumerate() {
            let slot = index + 1;
            let [r, g, b] = linear(paint.base);
            block.base[slot] = [r, g, b, 1.0];
            let [r, g, b] = linear(paint.wash.colour);
            block.wash[slot] = [r, g, b, paint.wash.stain];
            // The shader's smoothstep needs its edges in order, and a start
            // of exactly one would hand it none. The engine's own treats that
            // case as a step; this keeps the two on the same side of it.
            let start = paint.highlight.start.clamp(0.0, 0.999);
            block.film[slot] =
                [paint.wash.flow, paint.wash.edge, paint.highlight.amount.clamp(0.0, 1.0), start];
            let [r, g, b] = linear(paint.highlight.colour);
            block.light[slot] = [r, g, b, 0.0];
        }
        block
    }
}

impl PaintBlock {
    /// What the shader computes for `slot` at a crease of `crease`, written
    /// out in Rust from the shader's own lines.
    ///
    /// Not used to draw anything. It exists so that a test can put the
    /// shader's arithmetic beside the engine's without a GPU: the lines it is
    /// transcribed from are asserted to be IN the shader source, so editing
    /// either the shader or this fails that test.
    #[cfg(test)]
    pub(crate) fn shade_as_the_shader_does(&self, slot: usize, crease: f32) -> [f32; 3] {
        use brokkr_core::paint::WASH_FLOOR;

        let s = crease.clamp(-1.0, 1.0);
        let recess = s.max(0.0);
        let edge = (-s).max(0.0);
        let (base, wash, film, light) =
            (self.base[slot], self.wash[slot], self.film[slot], self.light[slot]);
        let thickness = (wash[3] + film[0] * recess - film[1] * edge).max(0.0);
        let smoothstep = |low: f32, high: f32, x: f32| {
            let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let caught = (smoothstep(film[3], 1.0, edge) * film[2]).clamp(0.0, 1.0);
        let mut out = [0.0; 3];
        for channel in 0..3 {
            let washed = base[channel] * wash[channel].max(WASH_FLOOR).powf(thickness);
            out[channel] = washed + (light[channel] - washed) * caught;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brokkr_core::paint::{Finish, Highlight, Paint, WASH_FLOOR, Wash};

    fn table() -> PaintTable {
        let mut table = PaintTable::default();
        table.add(Paint::flat("bone", [222, 208, 170]));
        table.add(Paint {
            finish: Finish::Satin,
            wash: Wash::shade([70, 40, 25]),
            highlight: Highlight { colour: [240, 225, 200], amount: 0.7, start: 0.25 },
            ..Paint::flat("leather", [90, 55, 30])
        });
        table.add(Paint {
            wash: Wash::contrast([30, 60, 150]),
            ..Paint::flat("primer", [235, 235, 235])
        });
        // A wash of pure black, and a highlight that starts at the very edge:
        // the two places the arithmetic has a corner.
        table.add(Paint {
            wash: Wash { colour: [0, 0, 0], stain: 0.0, flow: 2.0, edge: 0.5 },
            highlight: Highlight { colour: [255, 255, 255], amount: 1.0, start: 0.0 },
            ..Paint::flat("soot", [120, 120, 120])
        });
        table
    }

    #[test]
    fn an_empty_table_is_all_zeros_and_a_paint_marks_its_slot() {
        let empty = PaintBlock::from(&PaintTable::default());
        assert_eq!(empty, PaintBlock::default());
        assert!(bytemuck::bytes_of(&empty).iter().all(|byte| *byte == 0));

        let block = PaintBlock::from(&table());
        assert_eq!(block.base[0][3], 0.0, "slot 0 is unpainted and was marked as a paint");
        for slot in 1..=4 {
            assert_eq!(block.base[slot][3], 1.0, "slot {slot} is not marked as a paint");
        }
        assert_eq!(block.base[5][3], 0.0, "a slot past the table was marked as a paint");
    }

    #[test]
    fn the_shader_shades_a_paint_the_way_the_engine_does() {
        let shader = include_str!("shaders/sculpt.wgsl");
        // The shader's own lines. `shade_as_the_shader_does` is these,
        // transcribed; if one of them is edited this fails, and if the
        // transcription is wrong the comparison below does.
        for line in [
            "let recess = max(s, 0.0);",
            "let edge = max(-s, 0.0);",
            "let thickness = max(wash.w + film.x * recess - film.y * edge, 0.0);",
            "albedo = albedo * pow(max(wash.rgb, vec3<f32>(WASH_FLOOR)), vec3<f32>(thickness));",
            "let caught = clamp(smoothstep(film.w, 1.0, edge) * film.z, 0.0, 1.0);",
            "albedo = mix(albedo, light.rgb, caught);",
        ] {
            assert!(shader.contains(line), "sculpt.wgsl no longer has the line: {line}");
        }
        assert!(
            shader.contains(&format!("const WASH_FLOOR: f32 = {WASH_FLOOR};")),
            "sculpt.wgsl's WASH_FLOOR is not the engine's {WASH_FLOOR}"
        );
        assert!(
            shader.contains(&format!("paint_base: array<vec4<f32>, {PAINT_SLOTS}>,")),
            "sculpt.wgsl's paint arrays are not {PAINT_SLOTS} long"
        );
        assert!(
            shader.contains(&format!("min(in.slot, {}u)", PAINT_SLOTS - 1)),
            "sculpt.wgsl clamps the slot to something other than {}",
            PAINT_SLOTS - 1
        );

        let table = table();
        let block = PaintBlock::from(&table);
        let mut compared = 0;
        for (index, paint) in table.paints().iter().enumerate() {
            for step in -24..=24 {
                let crease = step as f32 / 20.0;
                let engine = paint.shade(crease);
                let shader = block.shade_as_the_shader_does(index + 1, crease);
                for channel in 0..3 {
                    assert!(
                        (engine[channel] - shader[channel]).abs() < 1.0e-5,
                        "{} at a crease of {crease}: the engine gives {engine:?} and the shader \
                         {shader:?}",
                        paint.name
                    );
                }
                compared += 1;
            }
        }
        assert_eq!(compared, 4 * 49);
    }
}
