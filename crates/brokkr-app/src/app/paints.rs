// SPDX-License-Identifier: AGPL-3.0-only

//! The paint rack: the document's paints, and the editor for the one in hand.
//!
//! A child module of `app` for the reason `panel.rs` is one, and a file of
//! its own rather than more of `panel.rs` or `app.rs` because both of those
//! are upstream's and this is not: `docs/STUDIO-PLAN.md` decision D6, add and
//! do not modify. What `app.rs` holds for this is two fields and four message
//! arms that call in here.
//!
//! # What a paint is, to the application
//!
//! [`brokkr_core::paint`] has the whole argument. In short a voxel carries a
//! slot number, the number names a recipe in the document's table, and the
//! recipe plus the shape of the surface give the colour. So everything here
//! edits the TABLE. No message in this file touches a voxel, which is why
//! none of them has anything to remesh once paints are in use, and why
//! changing a wash recolours every voxel that wears it in the same frame.
//!
//! # What is owed
//!
//! * **Undo.** An edit here dirties the document and is not in the history.
//!   The history is a list of changes to voxels and to the scene tree
//!   (`brokkr_core::undo::Change`), and a table edit is neither. It needs a
//!   variant of its own, in upstream's file, and that is its own piece of
//!   work.
//! * **Removing a paint.** Slots are positions in the table, so taking one
//!   out renumbers every paint after it, and every voxel painted with those
//!   would have to be rewritten to match. Until that pass exists a paint,
//!   once mixed, stays.

use brokkr_core::paint::touch::{Coat, Drybrush, Sponge, Touch};
use brokkr_core::paint::{Finish, MAX_PAINTS, Paint, Wash};
use glam::Vec3;
use iced::widget::{button, checkbox, column, container, pick_list, row, slider, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use super::Brokkr;
use crate::message::{Message, PaintCoat, PaintEdit, PaintTouch, TouchSetting, WashKind};
use crate::theme;

/// The pale coat a contrast paint goes over. A warm off-white: a contrast
/// paint over pure white reads as a highlighter pen.
pub(super) const UNDERCOAT: [u8; 3] = [232, 226, 214];

/// How much of the base coat's colour a ready-made shade keeps, per channel,
/// in sRGB levels. A shade is the base's own hue, darker: black in the
/// recesses of a red cloak reads as soot, and dark red reads as shadow.
const SHADE_OF_BASE: f32 = 0.35;

/// How many swatches go in a row of the rack and of the colour range.
const SWATCHES_A_ROW: usize = 8;

/// The slider ranges, which are also what an edit is clamped to on arrival.
pub(super) const STAIN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.5;
pub(super) const FLOW_RANGE: std::ops::RangeInclusive<f32> = 0.0..=3.0;
pub(super) const EDGE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.5;
pub(super) const AMOUNT_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
pub(super) const START_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.95;

/// The colour a PLY gives a surface nobody painted: the viewport's clay, as
/// near as one flat colour can be to a lit material. A warm mid grey, so
/// that an unpainted part of a model reads as "not painted" and not as a
/// paint somebody chose.
pub(super) const UNPAINTED: [u8; 3] = [168, 160, 150];

/// The sizes a baked texture can be, in texels on a side. The middle one is
/// what the application starts on: at a 28 mm figure it is a texel every 30
/// microns or so, which is finer than the voxels the paint is in.
pub(super) const TEXTURE_SIZES: [u32; 3] = [1024, 2048, 4096];

/// One entry of the "prints in" list: a filament slot, or 0 for the nearest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrintsIn {
    slot: u8,
    label: String,
}

impl std::fmt::Display for PrintsIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// How far a drybrush goes on one load by default, in millimetres. About the
/// height of a 28 mm figure and a bit: one pass down a cloak.
const DEFAULT_LOAD_MM: f32 = 40.0;

/// The ranges of the touch sliders, which are also what a setting is clamped
/// to on arrival.
pub(super) const CATCH_RANGE: std::ops::RangeInclusive<f32> = 0.05..=1.0;
pub(super) const STREAK_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
pub(super) const BRISTLE_RANGE: std::ops::RangeInclusive<f32> = 0.1..=2.0;
pub(super) const LOAD_RANGE: std::ops::RangeInclusive<f32> = 0.0..=200.0;
pub(super) const DENSITY_RANGE: std::ops::RangeInclusive<f32> = 0.02..=1.0;
pub(super) const GRAIN_RANGE: std::ops::RangeInclusive<f32> = 0.1..=4.0;
pub(super) const EDGES_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
pub(super) const SHARPNESS_RANGE: std::ops::RangeInclusive<f32> = 0.05..=1.0;

/// The most surface voxels one click of a fill may flood.
///
/// A fill costs about a microsecond and a half a voxel it paints, measured
/// in `brokkr_core::paint::fill`, and paints about six through the band for
/// each one on the surface. So this is about a second and a half at worst,
/// which is as long as a click should ever take, and is a patch 7 cm across
/// at a quarter millimetre. A model with no lines on it at all is what the
/// *Coat the body* button is for.
pub(super) const FILL_LIMIT: usize = 150_000;

/// How the paint brush touches the model, and the stroke in progress.
///
/// One struct on the application and not seven fields, so that upstream's
/// `Brokkr` grows by a line. The settings are the brush's: they are not
/// saved with a document and no document changes when they do.
#[derive(Debug)]
pub(super) struct TouchState {
    pub kind: PaintTouch,
    pub dry: Drybrush,
    pub sponge: Sponge,
    /// Whether a fill stops at creases, at edges, and how sharp a line has
    /// to be to count, as a share of a full crease.
    pub fill_stops_at_creases: bool,
    pub fill_stops_at_edges: bool,
    pub fill_sharpness: f32,
    /// How far a drybrush goes before it runs out, in millimetres. Zero for
    /// one that never does.
    pub load_mm: LoadMm,
    /// How many strokes have begun, which is where each one's seed comes
    /// from: a second stroke has to leave a different speckle from the
    /// first, and the same one every time the same session is replayed.
    strokes: u32,
    /// Where the stroke in progress began, and its seed.
    origin: Vec3,
    seed: u32,
    /// How far it has gone, and from where the next step is measured.
    travelled: f32,
    last: Vec3,
    /// Scratch for the gate, kept so that a stroke does not allocate.
    pub scratch: brokkr_core::paint::touch::TouchScratch,
}

/// A length in millimetres, named so that a bare number in a test is not
/// mistaken for a share.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LoadMm(pub f32);

impl Default for TouchState {
    fn default() -> Self {
        Self {
            kind: PaintTouch::default(),
            dry: Drybrush::default(),
            sponge: Sponge::default(),
            // Both on: a fill that stops at nothing is the whole body, and
            // there is a button for that.
            fill_stops_at_creases: true,
            fill_stops_at_edges: true,
            fill_sharpness: 0.3,
            load_mm: LoadMm(DEFAULT_LOAD_MM),
            strokes: 0,
            origin: Vec3::ZERO,
            seed: 0,
            travelled: 0.0,
            last: Vec3::ZERO,
            scratch: brokkr_core::paint::touch::TouchScratch::new(),
        }
    }
}

impl TouchState {
    /// Change one setting, clamped to its slider's range.
    pub(super) fn set(&mut self, setting: TouchSetting) {
        match setting {
            TouchSetting::DryCatch(value) => self.dry.catch = clamped(value, CATCH_RANGE),
            TouchSetting::DryStreak(value) => self.dry.streak = clamped(value, STREAK_RANGE),
            TouchSetting::DryBristle(mm) => self.dry.bristle_mm = clamped(mm, BRISTLE_RANGE),
            TouchSetting::DryLoad(mm) => self.load_mm = LoadMm(clamped(mm, LOAD_RANGE)),
            TouchSetting::SpongeDensity(value) => {
                self.sponge.density = clamped(value, DENSITY_RANGE);
            }
            TouchSetting::SpongeGrain(mm) => self.sponge.grain_mm = clamped(mm, GRAIN_RANGE),
            TouchSetting::SpongeEdges(value) => self.sponge.edges = clamped(value, EDGES_RANGE),
            TouchSetting::FillStopsAtCreases(on) => self.fill_stops_at_creases = on,
            TouchSetting::FillStopsAtEdges(on) => self.fill_stops_at_edges = on,
            TouchSetting::FillSharpness(value) => {
                self.fill_sharpness = clamped(value, SHARPNESS_RANGE);
            }
        }
    }

    /// The coat for the stamp at `centre`. `first` is whether it is the
    /// first stamp of a stroke, which is when the seed and the origin are
    /// fixed and the load is full.
    fn coat(&mut self, centre: Vec3, first: bool, crease_gain: f32) -> Coat {
        if first {
            self.strokes = self.strokes.wrapping_add(1);
            // Spread, so that stroke 1 and stroke 2 do not differ in one bit
            // of a hash's input.
            self.seed = self.strokes.wrapping_mul(0x9e37_79b1) ^ 0x5bd1_e995;
            self.origin = centre;
            self.travelled = 0.0;
        } else {
            self.travelled += centre.distance(self.last);
        }
        self.last = centre;

        let touch = match self.kind {
            PaintTouch::Loaded => Touch::Loaded,
            PaintTouch::Drybrush => Touch::Drybrush(self.dry),
            PaintTouch::Sponge => Touch::Sponge(self.sponge),
            // A fill lays no stamps, so this is never asked for one. A
            // loaded coat is the answer that does least if it ever is.
            PaintTouch::Fill => Touch::Loaded,
        };
        let load = if self.load_mm.0 > 0.0 {
            (1.0 - self.travelled / self.load_mm.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        Coat { touch, seed: self.seed, origin: self.origin, load, crease_gain }
    }
}

/// A starting range of colours to pick from, so that mixing a paint begins
/// with a click and the three sliders are for adjusting.
///
/// Plain descriptions and nobody's product names. The order is the order on
/// the panel: neutrals, then round the wheel from red, then skin and metal.
pub(super) const RANGE: [(&str, [u8; 3]); 24] = [
    ("Black", [24, 24, 26]),
    ("Charcoal", [62, 64, 68]),
    ("Grey", [128, 130, 132]),
    ("Bone", [222, 208, 170]),
    ("Off-white", [236, 234, 226]),
    ("Dark brown", [58, 38, 26]),
    ("Leather", [122, 78, 44]),
    ("Sand", [196, 160, 104]),
    ("Dark red", [110, 20, 24]),
    ("Red", [190, 32, 36]),
    ("Orange", [224, 112, 32]),
    ("Yellow", [236, 196, 48]),
    ("Olive", [96, 104, 48]),
    ("Green", [44, 132, 60]),
    ("Dark green", [22, 70, 44]),
    ("Turquoise", [28, 144, 148]),
    ("Blue", [36, 84, 172]),
    ("Dark blue", [22, 36, 92]),
    ("Purple", [100, 48, 136]),
    ("Magenta", [176, 44, 112]),
    ("Pale flesh", [232, 184, 150]),
    ("Dark flesh", [140, 86, 60]),
    ("Steel", [150, 156, 164]),
    ("Gold", [200, 156, 64]),
];

/// The colour a ready-made shade wash takes over a base coat.
pub(super) fn shade_of(base: [u8; 3]) -> [u8; 3] {
    base.map(|level| (f32::from(level) * SHADE_OF_BASE).round() as u8)
}

fn swatch_colour(colour: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(colour[0], colour[1], colour[2])
}

/// A paint as one flat colour, for its swatch: what it looks like on a flat.
/// Not the base coat, because a contrast paint's base is the undercoat and a
/// rack of contrast paints would be a rack of identical off-whites.
fn on_a_flat(paint: &Paint) -> iced::Color {
    swatch_colour(brokkr_core::paint::srgb(paint.shade(0.0)))
}

impl Brokkr {
    /// The highest slot the paint brush may be given.
    ///
    /// With paints in use that is the last paint, and without it is the last
    /// filament, as it was. Never zero, which is "unpainted" and is what the
    /// brush's erase direction writes.
    pub(super) fn last_paint_slot(&self) -> u8 {
        let count =
            if self.paints.is_empty() { self.palette.slots.len() } else { self.paints.len() };
        count.clamp(1, usize::from(u8::MAX)) as u8
    }

    /// The machine's filaments as colours, slot 1 first.
    fn filament_colours(&self) -> Vec<[u8; 3]> {
        self.palette
            .slots
            .iter()
            .map(|slot| {
                let colour = slot.swatch().into_rgba8();
                [colour[0], colour[1], colour[2]]
            })
            .collect()
    }

    /// What the paints have to become for a file of this format.
    ///
    /// For a PLY, one colour a vertex of each body, in the bodies' order.
    /// For a 3MF, nothing is returned and each body's slots are rewritten in
    /// place from paint numbers to the filaments the paints print in. A
    /// slot with no paint is a filament slot already and maps to itself, so
    /// a document with no paints comes through exactly as it was. STL and
    /// OBJ carry neither.
    ///
    /// A wash is baked at the viewport's gain, so it pools in the file where
    /// it pooled on screen.
    pub(super) fn bake_for(
        &self,
        format: crate::message::ExportFormat,
        bodies: &mut [brokkr_core::ExportedBody],
    ) -> Vec<Vec<[u8; 3]>> {
        use crate::message::ExportFormat;
        let filaments = self.filament_colours();
        let bake = brokkr_core::paint::bake::Bake {
            table: &self.paints,
            filaments: &filaments,
            unpainted: UNPAINTED,
            crease_gain: super::CAVITY_FEATURE_MM / self.doc.voxel_size() / 4.0,
        };
        match format {
            ExportFormat::Ply => bodies
                .iter()
                .map(|(meta, mesh, _)| match self.doc.volume(meta.id) {
                    Some(volume) => bake.vertex_colours(volume, mesh),
                    // A body the document no longer holds cannot have been
                    // exported in the first place. Clay, and not a panic.
                    None => vec![UNPAINTED; mesh.positions.len()],
                })
                .collect(),
            ExportFormat::ThreeMf => {
                // With no paints every slot is a filament slot already and
                // maps to itself, so this needs no "if there are paints".
                for (_, mesh, _) in bodies.iter_mut() {
                    mesh.slots = bake.filament_slots(mesh);
                }
                Vec::new()
            }
            // A texture is baked from the model and not from the mesh's
            // slots, by `textures_for`.
            ExportFormat::Stl | ExportFormat::Obj | ExportFormat::Textured | ExportFormat::Glb => {
                Vec::new()
            }
        }
    }

    /// Each body laid out flat, and its paint as a picture on that.
    ///
    /// Each body is unwrapped and baked here, from the mesh that is being
    /// written, whatever detail it was exported at: the texture is read off
    /// the model and not off the mesh, so a decimated export keeps its
    /// washes. See `brokkr_core::export::texture`.
    fn textures_for(
        &self,
        bodies: &[brokkr_core::ExportedBody],
    ) -> Vec<(brokkr_core::export::unwrap::Unwrap, brokkr_core::export::texture::Texture)> {
        use brokkr_core::export::texture::Texture;
        use brokkr_core::export::unwrap::UnwrapSpec;

        let filaments = self.filament_colours();
        let bake = brokkr_core::paint::bake::Bake {
            table: &self.paints,
            filaments: &filaments,
            unpainted: UNPAINTED,
            crease_gain: super::CAVITY_FEATURE_MM / self.doc.voxel_size() / 4.0,
        };
        let spec = UnwrapSpec { size: self.texture_size, ..UnwrapSpec::default() };
        bodies
            .iter()
            .map(|(meta, mesh, _)| {
                let unwrap = mesh.unwrapped(spec);
                let texture = match self.doc.volume(meta.id) {
                    Some(volume) => unwrap.baked(mesh, volume, &bake, spec.margin),
                    // As for a PLY in `bake_for`: clay, and not a panic.
                    None => {
                        let [red, green, blue] = UNPAINTED;
                        let texels = spec.size as usize * spec.size as usize;
                        Texture { size: spec.size, rgba: [red, green, blue, 255].repeat(texels) }
                    }
                };
                (unwrap, texture)
            })
            .collect()
    }

    /// Write the bodies as an OBJ with a material file and a picture each.
    ///
    /// The OBJ goes to `obj`, which the caller has open at `path`. The
    /// material file and the pictures go beside it under the same name:
    /// `figure.obj`, `figure.mtl`, `figure_1.png`, `figure_2.png`.
    pub(super) fn write_textured(
        &self,
        bodies: &[brokkr_core::ExportedBody],
        path: &std::path::Path,
        obj: &mut impl std::io::Write,
    ) -> std::io::Result<()> {
        use brokkr_core::export::textured::{Textured, write_mtl, write_obj};

        let stem =
            path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let textures = self.textures_for(bodies);
        let mut pictures = Vec::with_capacity(bodies.len());
        for (index, (_, texture)) in textures.iter().enumerate() {
            let picture = format!("{stem}_{}.png", index + 1);
            image::save_buffer(
                path.with_file_name(&picture),
                &texture.rgba,
                texture.size,
                texture.size,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(std::io::Error::other)?;
            pictures.push(picture);
        }
        let textured: Vec<Textured<'_>> = bodies
            .iter()
            .zip(&textures)
            .zip(&pictures)
            .map(|(((meta, mesh, _), (unwrap, _)), picture)| Textured {
                name: meta.name.as_str(),
                mesh,
                unwrap,
                picture,
            })
            .collect();
        let materials = format!("{stem}.mtl");
        write_obj(&textured, &materials, obj)?;
        let mut file =
            std::io::BufWriter::new(std::fs::File::create(path.with_file_name(&materials))?);
        write_mtl(&textured, &mut file)
    }

    /// Write the bodies as one GLB, each with its picture inside the file.
    ///
    /// The same unwrap and the same bake as [`Self::write_textured`]. The
    /// pictures are PNGs here too, made in memory.
    pub(super) fn write_glb(
        &self,
        bodies: &[brokkr_core::ExportedBody],
        out: &mut impl std::io::Write,
    ) -> std::io::Result<()> {
        let textures = self.textures_for(bodies);
        let mut pictures = Vec::with_capacity(bodies.len());
        for (_, texture) in &textures {
            let mut png = Vec::new();
            image::write_buffer_with_format(
                &mut std::io::Cursor::new(&mut png),
                &texture.rgba,
                texture.size,
                texture.size,
                image::ExtendedColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .map_err(std::io::Error::other)?;
            pictures.push(png);
        }
        let parts: Vec<brokkr_core::export::glb::Body<'_>> = bodies
            .iter()
            .zip(&textures)
            .zip(&pictures)
            .map(|(((meta, mesh, _), (unwrap, _)), png)| brokkr_core::export::glb::Body {
                name: meta.name.as_str(),
                mesh,
                unwrap,
                png,
            })
            .collect();
        brokkr_core::export::glb::write(&parts, out)
    }

    /// The exports that carry the paint as a picture: how big the picture
    /// is, the three sizes with the one in use lit, and under it a button
    /// for each format that has one.
    pub(super) fn textured_exports(&self) -> Element<'_, Message> {
        let sizes = TEXTURE_SIZES.into_iter().fold(row![].spacing(theme::S1), |assembled, size| {
            assembled.push(
                button(text(format!("{size}")).size(theme::CAPTION_SIZE))
                    .width(Length::Fill)
                    .style(if size == self.texture_size {
                        theme::tool_button_active
                    } else {
                        theme::tool_button
                    })
                    .on_press(Message::TextureSizeChanged(size)),
            )
        });
        let formats = crate::message::ExportFormat::ALL
            .into_iter()
            .filter(|format| format.has_texture())
            .fold(row![].spacing(theme::S2), |assembled, format| {
                assembled.push(
                    button(text(format.label()).size(theme::TEXT_SIZE_SMALL))
                        .style(theme::tool_button)
                        .on_press(Message::Export(format)),
                )
            });
        column![
            text("With the paint as a texture, of size")
                .size(theme::CAPTION_SIZE)
                .color(theme::TEXT_DIM),
            sizes,
            formats
        ]
        .spacing(theme::S1)
        .into()
    }

    /// The coat the paint stroke lays with the stamp at `centre`.
    ///
    /// The crease gain is the viewport's, so that what a drybrush catches is
    /// what "show creases" lights.
    pub(super) fn coat_for(&mut self, centre: Vec3, first: bool) -> Coat {
        let gain = super::CAVITY_FEATURE_MM / self.doc.voxel_size() / 4.0;
        self.touch.coat(centre, first, gain)
    }

    /// Pour `slot` onto the model at `point`, a world position on its
    /// surface, to run until the model's own lines stop it.
    ///
    /// Called for every pointer event of a drag with the Fill touch chosen,
    /// so dragging across several panels fills each as the pointer reaches
    /// it. A point already holding the paint costs one lookup and does
    /// nothing. The drag is one undo entry, on the stroke's own recorder.
    pub(super) fn pour(&mut self, point: Vec3, slot: u8) {
        let sharpness = Some(self.touch.fill_sharpness);
        let fill = brokkr_core::paint::fill::Fill {
            slot,
            stop_at_creases: sharpness.filter(|_| self.touch.fill_stops_at_creases),
            stop_at_edges: sharpness.filter(|_| self.touch.fill_stops_at_edges),
            crease_gain: super::CAVITY_FEATURE_MM / self.doc.voxel_size() / 4.0,
            limit: FILL_LIMIT,
            into_the_line: brokkr_core::paint::fill::INTO_THE_LINE,
        };
        let filled = self.doc.active_volume_mut().fill_paint(point, fill);
        if filled.reached_the_limit {
            self.status = format!(
                "the fill stopped at {FILL_LIMIT} voxels of surface: nothing on the model stopped \
                 it sooner. Coat the body paints all of it"
            );
        } else if filled.voxels > 0 {
            self.status = format!("filled {} voxels", filled.voxels);
        }
    }

    /// Coat the whole of the active body in the paint in hand, wherever the
    /// mask leaves it at least half free. One undo entry.
    ///
    /// Not a fill with nothing to stop it: that floods outward from a point
    /// and this is one pass over the body, which is the difference between
    /// seconds and milliseconds on a large model.
    pub(super) fn coat_the_body(&mut self) {
        let slot = self.paint_slot;
        let Some((low, high)) = self.doc.active_volume().world_bounds() else {
            return;
        };
        self.arm_recorder();
        let volume = self.doc.active_volume_mut();
        let (lo, hi) = volume.voxel_bounds(low, high);
        // The half line every paint stamp draws for the mask.
        let protected = brokkr_core::PROTECTED / 2 + 1;
        volume.edit_colour(
            lo,
            hi,
            |_, _, held, protection| {
                if protection >= protected { held } else { slot }
            },
        );
        self.finish_stroke();
        self.remesh_dirty();
        self.status = format!("coated the body in paint {slot}");
    }

    /// How the brush touches: a button each, and the settings of the one
    /// chosen.
    fn touch_picker(&self, settings: bool) -> Element<'_, Message> {
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let kinds =
            PaintTouch::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, kind| {
                assembled.push(
                    button(text(kind.label()).size(theme::CAPTION_SIZE))
                        .width(Length::Fill)
                        .style(if kind == self.touch.kind {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::PaintTouchChosen(kind)),
                )
            });
        let mut picker = column![kinds].spacing(theme::S1);
        if !settings {
            return picker.into();
        }
        let changed = |setting: TouchSetting| Message::PaintTouchChanged(setting);
        match self.touch.kind {
            PaintTouch::Loaded => {}
            PaintTouch::Drybrush => {
                let dry = self.touch.dry;
                let load = self.touch.load_mm.0;
                picker = picker
                    .push(reading(format!("Catches  {:.2}", dry.catch)))
                    .push(
                        slider(CATCH_RANGE, dry.catch, move |v| changed(TouchSetting::DryCatch(v)))
                            .step(0.01_f32),
                    )
                    .push(reading(format!("Streaks  {:.2}", dry.streak)))
                    .push(
                        slider(STREAK_RANGE, dry.streak, move |v| {
                            changed(TouchSetting::DryStreak(v))
                        })
                        .step(0.01_f32),
                    )
                    .push(reading(format!("Bristle  {:.1} mm", dry.bristle_mm)))
                    .push(
                        slider(BRISTLE_RANGE, dry.bristle_mm, move |v| {
                            changed(TouchSetting::DryBristle(v))
                        })
                        .step(0.1_f32),
                    )
                    .push(reading(if load > 0.0 {
                        format!("Runs out after  {load:.0} mm")
                    } else {
                        "Never runs out".to_string()
                    }))
                    .push(
                        slider(LOAD_RANGE, load, move |v| changed(TouchSetting::DryLoad(v)))
                            .step(5.0_f32),
                    );
            }
            PaintTouch::Sponge => {
                let sponge = self.touch.sponge;
                picker = picker
                    .push(reading(format!("Covers  {:.0}%", sponge.density * 100.0)))
                    .push(
                        slider(DENSITY_RANGE, sponge.density, move |v| {
                            changed(TouchSetting::SpongeDensity(v))
                        })
                        .step(0.01_f32),
                    )
                    .push(reading(format!("Speck  {:.1} mm", sponge.grain_mm)))
                    .push(
                        slider(GRAIN_RANGE, sponge.grain_mm, move |v| {
                            changed(TouchSetting::SpongeGrain(v))
                        })
                        .step(0.1_f32),
                    )
                    .push(reading(format!("More on edges  {:.2}", sponge.edges)))
                    .push(
                        slider(EDGES_RANGE, sponge.edges, move |v| {
                            changed(TouchSetting::SpongeEdges(v))
                        })
                        .step(0.01_f32),
                    );
            }
            PaintTouch::Fill => {
                let touch = &self.touch;
                picker = picker
                    .push(
                        checkbox(touch.fill_stops_at_creases)
                            .label("stops at creases")
                            .on_toggle(move |on| changed(TouchSetting::FillStopsAtCreases(on)))
                            .text_size(theme::CAPTION_SIZE),
                    )
                    .push(
                        checkbox(touch.fill_stops_at_edges)
                            .label("stops at edges")
                            .on_toggle(move |on| changed(TouchSetting::FillStopsAtEdges(on)))
                            .text_size(theme::CAPTION_SIZE),
                    )
                    .push(reading(format!("A line is sharper than  {:.2}", touch.fill_sharpness)))
                    .push(
                        slider(SHARPNESS_RANGE, touch.fill_sharpness, move |v| {
                            changed(TouchSetting::FillSharpness(v))
                        })
                        .step(0.01_f32),
                    )
                    .push(
                        text("click a panel to fill it, or a groove to line it")
                            .size(theme::CAPTION_SIZE)
                            .color(theme::TEXT_MUTE),
                    )
                    .push(
                        button(text("Coat the body").size(theme::TEXT_SIZE_SMALL))
                            .style(theme::tool_button)
                            .on_press(Message::PaintAllApplied),
                    );
            }
        }
        picker.into()
    }

    /// The paints belong to the document, so a new document has none: a
    /// fresh sphere and an imported mesh both start on filament slots.
    pub(super) fn forget_paints(&mut self) {
        self.paints = brokkr_core::paint::PaintTable::default();
        self.paint_slot = self.paint_slot.clamp(1, self.last_paint_slot());
    }

    /// Start using paints: one plain paint per filament slot.
    ///
    /// In the filament's own name and colour, so that a body already painted
    /// by slot looks exactly as it did the moment before, and what was
    /// "filament 2" is now a paint that can be given a wash.
    ///
    /// The one message here that remeshes. A wash reads the crease signal,
    /// the signal travels in the mesh, and a document without paints whose
    /// creases are switched off has meshes without it.
    pub(super) fn start_paints(&mut self) {
        if !self.paints.is_empty() {
            return;
        }
        for (index, slot) in self.palette.slots.iter().take(MAX_PAINTS).enumerate() {
            let colour = slot.swatch().into_rgba8();
            // Each names the filament it was, so a model painted by slot
            // prints exactly as it did before its slots were paints.
            self.paints.add(Paint {
                filament: index as u8 + 1,
                ..Paint::flat(slot.name.clone(), [colour[0], colour[1], colour[2]])
            });
        }
        if self.paints.is_empty() {
            // A machine with no filament slots at all still gets something to
            // paint with.
            self.paints.add(Paint::flat("Primer", RANGE[2].1));
        }
        self.paint_slot = self.paint_slot.clamp(1, self.last_paint_slot());
        self.unsaved = true;
        self.status = format!("mixed {} paints, one for each filament slot", self.paints.len());
        self.doc.mark_everything_dirty();
        self.remesh_dirty();
    }

    /// Add one more paint and take it up.
    pub(super) fn add_paint(&mut self) {
        self.start_paints();
        // Walk the range, so that a run of new paints is a run of different
        // colours and not the same one eight times.
        let (name, colour) = RANGE[self.paints.len() % RANGE.len()];
        match self.paints.add(Paint::flat(name, colour)) {
            Some(slot) => {
                self.paint_slot = slot;
                self.paint_coat = PaintCoat::Base;
                self.unsaved = true;
                self.status = format!("paint {slot} added");
            }
            None => {
                self.status = format!("the rack is full: a document holds {MAX_PAINTS} paints");
            }
        }
    }

    /// Change the paint in hand.
    ///
    /// Every number is clamped to its slider's range on arrival, for the
    /// reason the brush radius is: a message can come from anywhere.
    pub(super) fn edit_paint(&mut self, edit: PaintEdit) {
        let coat = self.paint_coat;
        let Some(paint) = self.paints.paint_mut(self.paint_slot) else {
            return;
        };
        let before = paint.clone();
        match edit {
            PaintEdit::Name(name) => paint.name = name,
            PaintEdit::Finish(finish) => paint.finish = finish,
            PaintEdit::Filament(filament) => paint.filament = filament,
            PaintEdit::Colour(colour) => *coat_colour(paint, coat) = colour,
            PaintEdit::Channel(channel, level) => {
                if let Some(slot) = coat_colour(paint, coat).get_mut(channel) {
                    *slot = level;
                }
            }
            PaintEdit::WashKind(kind) => match kind {
                WashKind::None => {
                    paint.wash = Wash { colour: paint.wash.colour, ..Wash::NONE };
                }
                WashKind::Shade => paint.wash = Wash::shade(shade_of(paint.base)),
                // A contrast paint is its pigment over a pale undercoat, so
                // the colour the paint had becomes the wash and the base
                // becomes the undercoat. Asked for twice, the base already
                // IS the undercoat, and then the pigment is left alone:
                // otherwise the second click would wash off-white over
                // off-white and the paint would be gone.
                WashKind::Contrast => {
                    let pigment =
                        if paint.base == UNDERCOAT { paint.wash.colour } else { paint.base };
                    paint.wash = Wash::contrast(pigment);
                    paint.base = UNDERCOAT;
                }
            },
            PaintEdit::WashStain(amount) => paint.wash.stain = clamped(amount, STAIN_RANGE),
            PaintEdit::WashFlow(amount) => paint.wash.flow = clamped(amount, FLOW_RANGE),
            PaintEdit::WashEdge(amount) => paint.wash.edge = clamped(amount, EDGE_RANGE),
            PaintEdit::HighlightAmount(amount) => {
                paint.highlight.amount = clamped(amount, AMOUNT_RANGE);
            }
            PaintEdit::HighlightStart(start) => {
                paint.highlight.start = clamped(start, START_RANGE);
            }
        }
        if *paint != before {
            self.unsaved = true;
        }
    }

    /// What the paint brush is offered: the filament slots while the document
    /// has no paints, and the rack once it has.
    ///
    /// `editing` is whether there is room for the editor under the rack. The
    /// panel has it and the pop-up menu does not.
    pub(super) fn paint_picker(&self, editing: bool) -> Element<'_, Message> {
        if self.paints.is_empty() {
            return column![
                self.touch_picker(editing),
                self.filament_picker(),
                button(text("Mix paints").size(theme::TEXT_SIZE_SMALL))
                    .style(theme::tool_button)
                    .on_press(Message::PaintsStarted),
                text("washes and highlights, on top of the slots")
                    .size(theme::CAPTION_SIZE)
                    .color(theme::TEXT_MUTE),
            ]
            .spacing(theme::S2)
            .into();
        }

        let mut rack = column![].spacing(theme::S1);
        for (line, paints) in self.paints.paints().chunks(SWATCHES_A_ROW).enumerate() {
            let mut swatches = row![].spacing(theme::S1);
            for (place, paint) in paints.iter().enumerate() {
                let slot = (line * SWATCHES_A_ROW + place + 1) as u8;
                let (style, _) = theme::tool_toggle(slot == self.paint_slot);
                swatches = swatches.push(
                    button(
                        container(text(""))
                            .width(14)
                            .height(14)
                            .style(theme::swatch(on_a_flat(paint))),
                    )
                    .padding(theme::S1)
                    .style(style)
                    .on_press(Message::PaintSlotChanged(slot)),
                );
            }
            rack = rack.push(swatches);
        }

        let name =
            self.paints.paint(self.paint_slot).map(|paint| paint.name.as_str()).unwrap_or("none");
        let mut picker = column![
            self.touch_picker(editing),
            text(format!("Paint  {}. {name}", self.paint_slot))
                .size(theme::TEXT_SIZE_SMALL)
                .color(theme::TEXT_DIM),
            rack,
        ]
        .spacing(theme::S2);
        if editing {
            picker = picker.push(
                checkbox(self.flat_paint)
                    .label("flat colour, no light")
                    .on_toggle(|_| Message::FlatPaintToggled)
                    .text_size(theme::CAPTION_SIZE),
            );
            let mut add =
                button(text("Add a paint").size(theme::TEXT_SIZE_SMALL)).style(theme::tool_button);
            if self.paints.len() < MAX_PAINTS {
                add = add.on_press(Message::PaintAdded);
            }
            picker = picker.push(add);
            if let Some(paint) = self.paints.paint(self.paint_slot) {
                picker = picker.push(self.paint_editor(paint));
            }
        }
        picker.into()
    }

    /// What a paint can be told to print in: the nearest filament, which is
    /// first and is what an unnamed paint gets, then each slot the machine
    /// has.
    pub(super) fn prints_in_choices(&self) -> Vec<PrintsIn> {
        let mut choices = vec![PrintsIn { slot: 0, label: "the nearest filament".to_string() }];
        for (index, slot) in self.palette.slots.iter().take(usize::from(u8::MAX)).enumerate() {
            choices.push(PrintsIn {
                slot: index as u8 + 1,
                label: format!("{}. {}", index + 1, slot.name),
            });
        }
        choices
    }

    /// The paint in hand: its name, and one coat at a time.
    fn paint_editor<'a>(&'a self, paint: &'a Paint) -> Element<'a, Message> {
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let coat = self.paint_coat;

        let coats =
            PaintCoat::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, each| {
                assembled.push(
                    button(text(each.label()).size(theme::CAPTION_SIZE))
                        .width(Length::Fill)
                        .style(if each == coat {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::PaintCoatChosen(each)),
                )
            });

        let colour = match coat {
            PaintCoat::Base => paint.base,
            PaintCoat::Wash => paint.wash.colour,
            PaintCoat::Highlight => paint.highlight.colour,
        };
        let mut range = column![].spacing(theme::S1);
        for line in RANGE.chunks(SWATCHES_A_ROW) {
            let mut swatches = row![].spacing(theme::S1);
            for (_, each) in line {
                let (style, _) = theme::tool_toggle(*each == colour);
                swatches = swatches.push(
                    button(
                        container(text(""))
                            .width(14)
                            .height(14)
                            .style(theme::swatch(swatch_colour(*each))),
                    )
                    .padding(2)
                    .style(style)
                    .on_press(Message::PaintEdited(PaintEdit::Colour(*each))),
                );
            }
            range = range.push(swatches);
        }
        let channels = ["R", "G", "B"].into_iter().enumerate().fold(
            column![].spacing(theme::S1),
            |assembled, (channel, label)| {
                assembled.push(
                    row![
                        text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE).width(10),
                        slider(0..=255_u8, colour[channel], move |level| {
                            Message::PaintEdited(PaintEdit::Channel(channel, level))
                        }),
                        text(format!("{:3}", colour[channel]))
                            .size(theme::CAPTION_SIZE)
                            .color(theme::TEXT_DIM)
                            .width(22),
                    ]
                    .spacing(theme::S2)
                    .align_y(Alignment::Center),
                )
            },
        );

        let amounts: Element<'_, Message> = match coat {
            PaintCoat::Base => {
                let choices = self.prints_in_choices();
                // A filament the machine does not have prints in the
                // nearest, and the list says so by showing that.
                let chosen = choices
                    .iter()
                    .find(|choice| choice.slot == paint.filament)
                    .or(choices.first())
                    .cloned();
                column![
                    reading("Finish".to_string()),
                    pick_list(Finish::ALL, Some(paint.finish), |finish| {
                        Message::PaintEdited(PaintEdit::Finish(finish))
                    })
                    .text_size(theme::CAPTION_SIZE)
                    .width(Length::Fill),
                    reading("Prints in".to_string()),
                    pick_list(choices, chosen, |choice: PrintsIn| {
                        Message::PaintEdited(PaintEdit::Filament(choice.slot))
                    })
                    .text_size(theme::CAPTION_SIZE)
                    .width(Length::Fill),
                ]
                .spacing(theme::S1)
                .into()
            }
            PaintCoat::Wash => {
                let kinds =
                    WashKind::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, kind| {
                        assembled.push(
                            button(text(kind.label()).size(theme::CAPTION_SIZE))
                                .width(Length::Fill)
                                .style(theme::tool_button)
                                .on_press(Message::PaintEdited(PaintEdit::WashKind(kind))),
                        )
                    });
                column![
                    kinds,
                    reading(format!("On the flats  {:.2}", paint.wash.stain)),
                    slider(STAIN_RANGE, paint.wash.stain, |amount| {
                        Message::PaintEdited(PaintEdit::WashStain(amount))
                    })
                    .step(0.01_f32),
                    reading(format!("Into the recesses  {:.2}", paint.wash.flow)),
                    slider(FLOW_RANGE, paint.wash.flow, |amount| {
                        Message::PaintEdited(PaintEdit::WashFlow(amount))
                    })
                    .step(0.05_f32),
                    reading(format!("Off the edges  {:.2}", paint.wash.edge)),
                    slider(EDGE_RANGE, paint.wash.edge, |amount| {
                        Message::PaintEdited(PaintEdit::WashEdge(amount))
                    })
                    .step(0.01_f32),
                ]
                .spacing(theme::S1)
                .into()
            }
            PaintCoat::Highlight => column![
                reading(format!("How much  {:.2}", paint.highlight.amount)),
                slider(AMOUNT_RANGE, paint.highlight.amount, |amount| {
                    Message::PaintEdited(PaintEdit::HighlightAmount(amount))
                })
                .step(0.01_f32),
                // Low is a drybrush over every raised form. High is an edge
                // highlight on the sharpest lines only.
                reading(format!("Only edges sharper than  {:.2}", paint.highlight.start)),
                slider(START_RANGE, paint.highlight.start, |start| {
                    Message::PaintEdited(PaintEdit::HighlightStart(start))
                })
                .step(0.01_f32),
            ]
            .spacing(theme::S1)
            .into(),
        };

        column![
            text_input("name", &paint.name)
                .on_input(|name| Message::PaintEdited(PaintEdit::Name(name)))
                .padding(Padding { top: 1.0, bottom: 1.0, left: theme::S1, right: theme::S1 })
                .size(theme::TEXT_SIZE_SMALL)
                .width(Length::Fill),
            coats,
            range,
            channels,
            amounts,
        ]
        .spacing(theme::S2)
        .into()
    }
}

/// The colour a coat's swatches and sliders address.
fn coat_colour(paint: &mut Paint, coat: PaintCoat) -> &mut [u8; 3] {
    match coat {
        PaintCoat::Base => &mut paint.base,
        PaintCoat::Wash => &mut paint.wash.colour,
        PaintCoat::Highlight => &mut paint.highlight.colour,
    }
}

/// Into a slider's range, with a number that is not a number sent to the
/// bottom of it rather than through: `f32::clamp` passes a NaN, and a NaN in
/// a paint is a NaN in a uniform.
fn clamped(value: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if value.is_nan() { *range.start() } else { value.clamp(*range.start(), *range.end()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brokkr_core::BrushKind;
    use brokkr_core::paint::PaintTable;

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
    }

    fn edit(app: &mut Brokkr, edit: PaintEdit) {
        update(app, Message::PaintEdited(edit));
    }

    /// Mixing paints gives one per filament slot in the filament's colour, so
    /// a body painted by slot does not change colour under the user. It
    /// dirties the document, reaches the shader, and puts the crease signal
    /// into the meshes, which a wash is about to need.
    #[test]
    fn mixing_paints_copies_the_filament_slots_and_remeshes_with_the_signal() {
        let mut app = app();
        app.unsaved = false;
        assert!(app.paints.is_empty(), "a new document already had paints");
        assert_eq!(
            app.shared.paint_view().paints,
            brokkr_gpu::PaintBlock::default(),
            "the shader was told about paints a new document does not have"
        );
        let before = app.shared.pending_cavity_for_tests().len();

        update(&mut app, Message::PaintsStarted);

        assert_eq!(app.paints.len(), app.palette.slots.len(), "not one paint per filament slot");
        for (paint, slot) in app.paints.paints().iter().zip(&app.palette.slots) {
            let colour = slot.swatch().into_rgba8();
            assert_eq!(paint.base, [colour[0], colour[1], colour[2]], "{}", slot.name);
            assert_eq!(paint.name, slot.name);
            assert!(paint.wash.is_none() && paint.highlight.is_none(), "a new paint is plain");
        }
        assert!(app.unsaved, "mixing paints did not dirty the document");

        let view = app.shared.paint_view();
        assert_eq!(
            view.paints,
            brokkr_gpu::PaintBlock::from(&app.paints),
            "the table did not reach the shared frame"
        );
        assert_ne!(view.paints, brokkr_gpu::PaintBlock::default());

        let published = app.shared.pending_cavity_for_tests();
        let fresh = &published[before..];
        assert!(fresh.iter().any(|(vertices, _)| *vertices > 0), "mixing paints remeshed nothing");
        assert!(
            fresh.iter().all(|(vertices, cavity)| vertices == cavity),
            "the meshes do not carry the crease signal a wash reads"
        );

        // Asked for again, nothing happens: the table is not doubled.
        let count = app.paints.len();
        update(&mut app, Message::PaintsStarted);
        assert_eq!(app.paints.len(), count, "mixing twice doubled the rack");
    }

    /// With paints in use the crease toggle is only a view toggle: the meshes
    /// carry the signal either way, so it must neither strip it nor remesh.
    #[test]
    fn the_crease_toggle_leaves_the_signal_in_the_meshes_once_paints_need_it() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);
        let before = app.shared.pending_cavity_for_tests().len();

        update(&mut app, Message::ShowCavityToggled);
        update(&mut app, Message::ShowCavityToggled);
        assert_eq!(
            app.shared.pending_cavity_for_tests().len(),
            before,
            "the toggle remeshed a document whose meshes already had the signal"
        );

        // And anything that does remesh keeps the signal with creases hidden.
        app.doc.mark_everything_dirty();
        app.remesh_dirty();
        let published = app.shared.pending_cavity_for_tests();
        let fresh = &published[before..];
        assert!(fresh.iter().any(|(vertices, _)| *vertices > 0), "the control remeshed nothing");
        assert!(
            fresh.iter().all(|(vertices, cavity)| vertices == cavity),
            "a remesh with creases hidden dropped the signal a wash reads"
        );
    }

    /// The brush's slot is clamped to the rack once there is one, and a new
    /// paint is taken up as it is added.
    #[test]
    fn the_brush_is_offered_the_paints_and_takes_up_a_new_one() {
        let mut app = app();
        let filaments = app.palette.slots.len() as u8;
        update(&mut app, Message::PaintSlotChanged(200));
        assert_eq!(app.paint_slot, filaments, "without paints the limit is the filaments");

        update(&mut app, Message::PaintAdded);
        assert_eq!(app.paints.len(), usize::from(filaments) + 1, "adding did not mix first");
        assert_eq!(app.paint_slot, filaments + 1, "the new paint was not taken up");

        update(&mut app, Message::PaintSlotChanged(200));
        assert_eq!(app.paint_slot, filaments + 1, "a slot past the rack was accepted");
        update(&mut app, Message::PaintSlotChanged(0));
        assert_eq!(app.paint_slot, 1, "slot 0 is unpainted and cannot be chosen");

        // The rack fills and then says so, without wrapping the slot byte.
        for _ in 0..MAX_PAINTS + 3 {
            update(&mut app, Message::PaintAdded);
        }
        assert_eq!(app.paints.len(), MAX_PAINTS, "the rack went past what the shader holds");
        assert!(app.status.contains("full"), "a full rack was not reported: {}", app.status);
        assert!(usize::from(app.paint_slot) <= MAX_PAINTS);
    }

    /// Every edit lands on the paint in hand and the coat shown, reaches the
    /// shader, dirties the document, and touches no voxel and no history.
    #[test]
    fn an_edit_changes_the_paint_in_hand_and_nothing_else() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);
        update(&mut app, Message::PaintSlotChanged(2));
        let others: Vec<_> = app
            .paints
            .paints()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, p)| p.clone())
            .collect();
        let undo_before = app.history.stats();
        let meshes_before = app.shared.pending_cavity_for_tests().len();
        app.unsaved = false;

        edit(&mut app, PaintEdit::Colour([10, 20, 30]));
        assert_eq!(app.paints.paint(2).unwrap().base, [10, 20, 30]);
        assert!(app.unsaved, "an edit did not dirty the document");

        update(&mut app, Message::PaintCoatChosen(PaintCoat::Wash));
        edit(&mut app, PaintEdit::Colour([40, 50, 60]));
        edit(&mut app, PaintEdit::Channel(1, 99));
        edit(&mut app, PaintEdit::Channel(7, 1));
        assert_eq!(app.paints.paint(2).unwrap().wash.colour, [40, 99, 60]);
        assert_eq!(app.paints.paint(2).unwrap().base, [10, 20, 30], "the wash edit hit the base");

        update(&mut app, Message::PaintCoatChosen(PaintCoat::Highlight));
        edit(&mut app, PaintEdit::Colour([200, 210, 220]));
        assert_eq!(app.paints.paint(2).unwrap().highlight.colour, [200, 210, 220]);

        edit(&mut app, PaintEdit::WashStain(0.4));
        edit(&mut app, PaintEdit::WashFlow(2.0));
        edit(&mut app, PaintEdit::WashEdge(0.3));
        edit(&mut app, PaintEdit::HighlightAmount(0.6));
        edit(&mut app, PaintEdit::HighlightStart(0.5));
        edit(&mut app, PaintEdit::Name("Cloak".to_string()));
        edit(&mut app, PaintEdit::Finish(Finish::Gloss));
        let paint = app.paints.paint(2).unwrap().clone();
        assert_eq!(
            (paint.wash.stain, paint.wash.flow, paint.wash.edge),
            (0.4, 2.0, 0.3),
            "a wash amount went to the wrong place"
        );
        assert_eq!((paint.highlight.amount, paint.highlight.start), (0.6, 0.5));
        assert_eq!((paint.name.as_str(), paint.finish), ("Cloak", Finish::Gloss));

        let after: Vec<_> = app
            .paints
            .paints()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, p)| p.clone())
            .collect();
        assert_eq!(after, others, "an edit reached a paint that was not in hand");
        assert_eq!(
            app.shared.paint_view().paints,
            brokkr_gpu::PaintBlock::from(&app.paints),
            "the edited table did not reach the shared frame"
        );
        assert_eq!(app.history.stats(), undo_before, "a table edit reached the voxel history");
        assert_eq!(
            app.shared.pending_cavity_for_tests().len(),
            meshes_before,
            "a table edit remeshed, and it changes no voxel"
        );

        // An edit that changes nothing does not dirty a clean document.
        app.unsaved = false;
        edit(&mut app, PaintEdit::WashFlow(2.0));
        assert!(!app.unsaved, "an edit that changed nothing dirtied the document");
    }

    /// Numbers from anywhere are held to the sliders' ranges, and a NaN goes
    /// to the bottom of one instead of into a uniform.
    #[test]
    fn amounts_are_clamped_and_a_nan_never_reaches_a_paint() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);

        edit(&mut app, PaintEdit::WashStain(99.0));
        edit(&mut app, PaintEdit::WashFlow(-4.0));
        edit(&mut app, PaintEdit::WashEdge(f32::INFINITY));
        edit(&mut app, PaintEdit::HighlightAmount(7.0));
        edit(&mut app, PaintEdit::HighlightStart(1.0));
        let paint = app.paints.paint(app.paint_slot).unwrap().clone();
        assert_eq!(paint.wash.stain, *STAIN_RANGE.end());
        assert_eq!(paint.wash.flow, *FLOW_RANGE.start());
        assert_eq!(paint.wash.edge, *EDGE_RANGE.end());
        assert_eq!(paint.highlight.amount, *AMOUNT_RANGE.end());
        assert_eq!(paint.highlight.start, *START_RANGE.end(), "a start of 1 divides by nothing");

        for nan in [
            PaintEdit::WashStain(f32::NAN),
            PaintEdit::WashFlow(f32::NAN),
            PaintEdit::WashEdge(f32::NAN),
            PaintEdit::HighlightAmount(f32::NAN),
            PaintEdit::HighlightStart(f32::NAN),
        ] {
            edit(&mut app, nan);
        }
        let paint = app.paints.paint(app.paint_slot).unwrap();
        for amount in [
            paint.wash.stain,
            paint.wash.flow,
            paint.wash.edge,
            paint.highlight.amount,
            paint.highlight.start,
        ] {
            assert_eq!(amount, 0.0, "a NaN was not sent to the bottom of its range");
        }
    }

    /// The three ready-made washes. A shade is the base's hue, darker, and
    /// leaves the base alone. A contrast moves the colour into the wash over
    /// the undercoat, and asked for twice it does not wash the paint away.
    #[test]
    fn the_ready_made_washes_are_what_their_names_say() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);
        let red = [190, 32, 36];
        edit(&mut app, PaintEdit::Colour(red));
        let slot = app.paint_slot;

        edit(&mut app, PaintEdit::WashKind(WashKind::Shade));
        let paint = app.paints.paint(slot).unwrap().clone();
        assert_eq!(paint.base, red, "a shade repainted the base coat");
        assert_eq!(paint.wash, Wash::shade(shade_of(red)));
        assert!(paint.wash.colour.iter().zip(red).all(|(wash, base)| *wash < base.max(1)));
        let flat = paint.shade(0.0);
        let crease = paint.shade(1.0);
        assert!(crease[0] < flat[0] * 0.5, "a shade did not darken the recess");

        edit(&mut app, PaintEdit::WashKind(WashKind::Contrast));
        let paint = app.paints.paint(slot).unwrap().clone();
        assert_eq!(paint.base, UNDERCOAT);
        assert_eq!(paint.wash, Wash::contrast(red), "the colour did not move into the wash");

        edit(&mut app, PaintEdit::WashKind(WashKind::Contrast));
        let again = app.paints.paint(slot).unwrap().clone();
        assert_eq!(again, paint, "asking for contrast twice washed the pigment away");

        edit(&mut app, PaintEdit::WashKind(WashKind::None));
        let paint = app.paints.paint(slot).unwrap().clone();
        assert!(paint.wash.is_none(), "None left a wash on");
        assert_eq!(paint.wash.colour, red, "None threw the pigment away with the film");
        assert_eq!(paint.shade(1.0), paint.shade(0.0), "a paint with no wash still shades");
    }

    /// Paints are the document's: they are saved with it, come back when it
    /// is opened, and do not follow the user into a new document.
    #[test]
    fn paints_are_saved_with_the_document_and_do_not_outlive_it() {
        let directory = std::env::temp_dir().join(format!("brokkr-paints-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        let path = directory.join("painted.brokkr");

        let mut app = app();
        update(&mut app, Message::PaintAdded);
        edit(&mut app, PaintEdit::Name("Cloak".to_string()));
        edit(&mut app, PaintEdit::WashKind(WashKind::Contrast));
        update(&mut app, Message::PaintCoatChosen(PaintCoat::Highlight));
        edit(&mut app, PaintEdit::HighlightAmount(0.5));
        let saved = app.paints.clone();
        let slot = app.paint_slot;
        app.save_project(&path);
        assert!(!app.unsaved, "the save failed: {}", app.status);

        let mut reopened = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        reopened.paint_slot = 200;
        reopened.open_project(&path);
        assert_eq!(reopened.paints, saved, "the paints did not come back: {}", reopened.status);
        assert!(usize::from(reopened.paint_slot) <= saved.len(), "the slot outran the rack");
        assert_eq!(
            reopened.shared.paint_view().paints,
            brokkr_gpu::PaintBlock::from(&saved),
            "an opened document's paints did not reach the shader"
        );
        assert_eq!(saved.paint(slot).unwrap().name, "Cloak");

        // A fresh sphere is a new document and starts with none.
        update(&mut reopened, Message::ResetSphere);
        assert_eq!(reopened.paints, PaintTable::default(), "paints outlived their document");
        assert!(usize::from(reopened.paint_slot) <= reopened.palette.slots.len());

        // And so does a document opened over a painted one, when the file has
        // none: the control that the open path assigns and does not merge.
        let plain = directory.join("plain.brokkr");
        let mut bare = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        bare.save_project(&plain);
        app.open_project(&plain);
        assert!(app.paints.is_empty(), "opening an unpainted file kept the old paints");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The panel builds with and without paints and with each coat showing.
    /// `view` borrows the whole application, so a widget tree that indexes
    /// past a rack or a range panics here and not in front of a user.
    #[test]
    fn the_rack_and_the_editor_build_for_every_coat() {
        let mut app = app();
        drop(app.paint_picker(true));
        drop(app.paint_picker(false));
        for _ in 0..MAX_PAINTS {
            update(&mut app, Message::PaintAdded);
        }
        for coat in PaintCoat::ALL {
            update(&mut app, Message::PaintCoatChosen(coat));
            drop(app.paint_picker(true));
            drop(app.paint_picker(false));
        }
        // A slot the table does not have draws no editor and does not panic.
        app.paint_slot = 200;
        drop(app.paint_picker(true));
    }

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    /// A short drag across the middle of the model, as the paint brush
    /// tests upstream make it.
    fn stroke(app: &mut Brokkr) {
        use crate::message::{PointerButton, PointerEvent};
        let centre = iced::Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre,
                size: SIZE,
            }),
        );
        for step in 1..=4 {
            let position = iced::Vector::new(centre.x + 12.0 * step as f32, centre.y);
            update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
        }
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// How many voxels of the active body are painted at all.
    fn painted_voxels(app: &Brokkr) -> usize {
        let volume = app.doc.active_volume();
        let Some((low, high)) = volume.world_bounds() else {
            return 0;
        };
        let (lo, hi) = volume.voxel_bounds(low, high);
        let mut count = 0;
        // The stroke is on the side of the ball facing the camera, so one
        // sweep of the whole box is the simple way to find all of it.
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    if volume.colour().at(glam::IVec3::new(x, y, z)) != 0 {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// A sponge stroke through the whole application: it paints a speckle
    /// and not a stripe, it is one undo, and a second stroke over the same
    /// place adds to the first.
    #[test]
    fn a_sponge_stroke_leaves_a_speckle_and_is_one_undo() {
        let mut loaded = app();
        stroke(&mut loaded);
        let stripe = painted_voxels(&loaded);
        assert!(stripe > 500, "the loaded stroke painted only {stripe} voxels");

        let mut app = app();
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Sponge));
        update(&mut app, Message::PaintTouchChanged(TouchSetting::SpongeDensity(0.3)));
        update(&mut app, Message::PaintTouchChanged(TouchSetting::SpongeEdges(0.0)));
        stroke(&mut app);
        let speckle = painted_voxels(&app);
        assert!(
            speckle * 10 > stripe && speckle * 2 < stripe,
            "a sponge at 30% painted {speckle} voxels where a loaded brush painted {stripe}"
        );

        stroke(&mut app);
        let twice = painted_voxels(&app);
        assert!(
            twice > speckle + speckle / 5,
            "a second sponge stroke added only {} voxels to {speckle}",
            twice - speckle
        );

        update(&mut app, Message::Undo);
        assert_eq!(painted_voxels(&app), speckle, "one undo did not take off exactly one stroke");
        update(&mut app, Message::Undo);
        assert_eq!(painted_voxels(&app), 0, "the first stroke was not one undo");
    }

    /// A drybrush leaves a smooth ball clean, because nothing on it stands
    /// up. The plainest statement of what the tool is: it is not a way of
    /// painting less, it is a way of painting edges.
    #[test]
    fn a_drybrush_leaves_a_smooth_ball_clean() {
        let mut app = app();
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Drybrush));
        update(&mut app, Message::PaintTouchChanged(TouchSetting::DryCatch(0.9)));
        stroke(&mut app);
        assert_eq!(painted_voxels(&app), 0, "a drybrush painted a surface with no edges on it");
    }

    /// A stroke keeps one seed and one origin from press to release, the
    /// next stroke gets another, and the load runs down with the distance.
    #[test]
    fn a_stroke_keeps_its_seed_and_runs_its_load_down() {
        let mut touch = TouchState { kind: PaintTouch::Drybrush, ..TouchState::default() };
        touch.set(TouchSetting::DryLoad(20.0));
        let first = touch.coat(Vec3::new(1.0, 2.0, 3.0), true, 0.5);
        let next = touch.coat(Vec3::new(6.0, 2.0, 3.0), false, 0.5);
        let last = touch.coat(Vec3::new(6.0, 14.0, 3.0), false, 0.5);
        assert_eq!(first.load, 1.0, "a stroke did not begin with a full brush");
        assert_eq!(first.origin, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(first.crease_gain, 0.5);
        assert_eq!((next.seed, next.origin), (first.seed, first.origin));
        assert_eq!((last.seed, last.origin), (first.seed, first.origin));
        assert!((next.load - 0.75).abs() < 1e-6, "5 mm of 20 left a load of {}", next.load);
        assert!((last.load - 0.15).abs() < 1e-6, "17 mm of 20 left a load of {}", last.load);
        let spent = touch.coat(Vec3::new(6.0, 40.0, 3.0), false, 0.5);
        assert_eq!(spent.load, 0.0, "the load went below nothing or stayed above it");

        let second = touch.coat(Vec3::new(9.0, 9.0, 9.0), true, 0.5);
        assert_ne!(second.seed, first.seed, "two strokes shared a seed");
        assert_eq!(second.load, 1.0, "the brush was not reloaded for the next stroke");
        assert_eq!(second.origin, Vec3::new(9.0, 9.0, 9.0));

        // A load of zero is a brush that never runs out.
        touch.set(TouchSetting::DryLoad(0.0));
        touch.coat(Vec3::ZERO, true, 0.5);
        assert_eq!(touch.coat(Vec3::splat(500.0), false, 0.5).load, 1.0);

        // And the kind chosen is the kind laid.
        assert!(matches!(first.touch, Touch::Drybrush(_)));
        touch.kind = PaintTouch::Sponge;
        assert!(matches!(touch.coat(Vec3::ZERO, true, 0.5).touch, Touch::Sponge(_)));
        touch.kind = PaintTouch::Loaded;
        assert!(matches!(touch.coat(Vec3::ZERO, true, 0.5).touch, Touch::Loaded));
    }

    /// How many voxels of the active body could be painted at all.
    fn paintable_voxels(app: &Brokkr) -> usize {
        let volume = app.doc.active_volume();
        let (low, high) = volume.world_bounds().expect("the body has bricks");
        let (lo, hi) = volume.voxel_bounds(low, high);
        let mut count = 0;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let distance = volume.sample_voxel(glam::IVec3::new(x, y, z));
                    count += usize::from(brokkr_core::colour::ColourField::paintable(distance));
                }
            }
        }
        count
    }

    /// The Fill touch through the application. The starting ball has no
    /// lines on it and is bigger than one click may flood, so a fill on it
    /// runs to its limit, says so, and is one undo for the whole drag.
    #[test]
    fn a_fill_pours_until_its_limit_on_a_ball_and_is_one_undo() {
        let mut app = app();
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Fill));
        stroke(&mut app);
        let poured = painted_voxels(&app);
        assert!(poured >= FILL_LIMIT, "a fill of {FILL_LIMIT} surface voxels painted {poured}");
        assert!(poured < paintable_voxels(&app), "the limit did not stop the fill");
        assert!(app.status.contains("stopped"), "the limit was not reported: {}", app.status);
        assert!(app.unsaved, "a fill did not dirty the document");
        // A fill is not a stroke, and lays no stamps of the brush along the
        // drag: the pour is all that happens.
        assert_eq!(app.perf.stamps, 0, "a fill also stamped the brush along the drag");

        update(&mut app, Message::Undo);
        assert_eq!(painted_voxels(&app), 0, "the whole drag was not one undo");

        // The settings reach the fill: with both stops off a fill reads no
        // line at all, and on a ball that changes nothing, so the proof is
        // in the fields.
        update(&mut app, Message::PaintTouchChanged(TouchSetting::FillStopsAtCreases(false)));
        update(&mut app, Message::PaintTouchChanged(TouchSetting::FillStopsAtEdges(false)));
        update(&mut app, Message::PaintTouchChanged(TouchSetting::FillSharpness(0.61)));
        assert!(!app.touch.fill_stops_at_creases && !app.touch.fill_stops_at_edges);
        assert_eq!(app.touch.fill_sharpness, 0.61);
        update(&mut app, Message::PaintTouchChanged(TouchSetting::FillStopsAtEdges(true)));
        assert!(app.touch.fill_stops_at_edges && !app.touch.fill_stops_at_creases);
        update(&mut app, Message::PaintTouchChanged(TouchSetting::FillSharpness(0.0)));
        assert_eq!(app.touch.fill_sharpness, *SHARPNESS_RANGE.start(), "a line of no sharpness");
        update(&mut app, Message::PaintTouchChanged(TouchSetting::FillSharpness(f32::NAN)));
        assert_eq!(app.touch.fill_sharpness, *SHARPNESS_RANGE.start());
        let fresh = TouchState::default();
        assert!(fresh.fill_stops_at_creases && fresh.fill_stops_at_edges, "a fill starts open");
    }

    /// Coat the body paints every voxel that can be painted, in the paint
    /// in hand, in one undo; and a masked body takes none of it.
    #[test]
    fn coating_the_body_paints_all_of_it_once_and_respects_the_mask() {
        let mut app = app();
        update(&mut app, Message::PaintSlotChanged(2));
        let all = paintable_voxels(&app);
        update(&mut app, Message::PaintAllApplied);
        assert_eq!(painted_voxels(&app), all, "the coat missed some of the body");
        assert!(app.status.contains("paint 2"), "{}", app.status);
        assert!(app.unsaved);
        update(&mut app, Message::Undo);
        assert_eq!(painted_voxels(&app), 0, "the coat was not one undo");

        update(&mut app, Message::MaskAllApplied);
        update(&mut app, Message::PaintAllApplied);
        assert_eq!(painted_voxels(&app), 0, "a fully masked body was coated");
    }

    /// The flat view is a view: it reaches the shader and touches nothing
    /// else.
    #[test]
    fn the_flat_view_reaches_the_shader_and_changes_no_document() {
        let mut app = app();
        app.unsaved = false;
        assert!(!app.shared.paint_view().flat, "paint starts lit");
        update(&mut app, Message::FlatPaintToggled);
        assert!(app.shared.paint_view().flat, "the toggle did not reach the shared frame");
        assert!(app.shared.paint_view().shown);
        assert!(!app.unsaved, "a view toggle dirtied the document");
        update(&mut app, Message::FlatPaintToggled);
        assert!(!app.shared.paint_view().flat);
    }

    /// What a drybrush calls a full edge is what the viewport calls one, so
    /// the brush catches what "show creases" lights.
    #[test]
    fn the_brush_reads_edges_at_the_viewports_gain() {
        let mut app = app();
        let coat = app.coat_for(Vec3::ZERO, true);
        assert_eq!(
            coat.crease_gain,
            app.shared.cavity_view().gain,
            "the drybrush and the crease view disagree about what a full edge is"
        );
        assert!(coat.crease_gain > 0.0);
    }

    /// Each setting goes to its own field, clamped to its slider.
    #[test]
    fn touch_settings_land_where_they_say_and_are_clamped() {
        let mut app = app();
        let set = |app: &mut Brokkr, setting| update(app, Message::PaintTouchChanged(setting));
        set(&mut app, TouchSetting::DryCatch(0.31));
        set(&mut app, TouchSetting::DryStreak(0.32));
        set(&mut app, TouchSetting::DryBristle(0.33));
        set(&mut app, TouchSetting::DryLoad(34.0));
        set(&mut app, TouchSetting::SpongeDensity(0.35));
        set(&mut app, TouchSetting::SpongeGrain(0.36));
        set(&mut app, TouchSetting::SpongeEdges(0.37));
        let touch = &app.touch;
        assert_eq!((touch.dry.catch, touch.dry.streak, touch.dry.bristle_mm), (0.31, 0.32, 0.33));
        assert_eq!(touch.load_mm.0, 34.0);
        assert_eq!(
            (touch.sponge.density, touch.sponge.grain_mm, touch.sponge.edges),
            (0.35, 0.36, 0.37)
        );

        set(&mut app, TouchSetting::DryCatch(9.0));
        set(&mut app, TouchSetting::DryBristle(0.0));
        set(&mut app, TouchSetting::DryLoad(-5.0));
        set(&mut app, TouchSetting::SpongeDensity(f32::NAN));
        set(&mut app, TouchSetting::SpongeGrain(0.0));
        let touch = &app.touch;
        assert_eq!(touch.dry.catch, *CATCH_RANGE.end());
        assert_eq!(touch.dry.bristle_mm, *BRISTLE_RANGE.start(), "a bristle of no width");
        assert_eq!(touch.load_mm.0, 0.0);
        assert_eq!(touch.sponge.density, *DENSITY_RANGE.start(), "a NaN reached the sponge");
        assert_eq!(touch.sponge.grain_mm, *GRAIN_RANGE.start(), "a speck of no size");

        // The panel builds for each kind, with and without its sliders.
        for kind in PaintTouch::ALL {
            update(&mut app, Message::PaintTouchChosen(kind));
            assert_eq!(app.touch.kind, kind);
            drop(app.paint_picker(true));
            drop(app.paint_picker(false));
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("brokkr-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    /// The colours of a PLY written by the application, read off its header
    /// and its vertex records: 27 bytes each, the last three the colour.
    fn ply_colours(bytes: &[u8]) -> Vec<[u8; 3]> {
        let end = b"end_header\n";
        let body =
            bytes.windows(end.len()).position(|w| w == end).expect("no end_header") + end.len();
        let header = std::str::from_utf8(&bytes[..body]).expect("the header is text");
        let vertices: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("element vertex "))
            .expect("no vertex element")
            .parse()
            .expect("a count");
        (0..vertices)
            .map(|index| {
                let at = body + index * 27 + 24;
                [bytes[at], bytes[at + 1], bytes[at + 2]]
            })
            .collect()
    }

    /// A painted body exported as PLY: the stroke is in the file in the
    /// paint's own colour, the rest is clay, and a paint with a wash bakes
    /// something other than its flat colour somewhere.
    #[test]
    fn a_painted_body_exports_as_a_ply_in_its_paints_colours() {
        let directory = scratch("ply");
        let path = directory.join("painted.ply");
        let mut app = app();
        update(&mut app, Message::PaintAdded);
        let colour = [20, 140, 200];
        edit(&mut app, PaintEdit::Colour(colour));
        stroke(&mut app);

        app.export(crate::message::ExportFormat::Ply, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        let colours = ply_colours(&std::fs::read(&path).expect("the file should be there"));
        let painted = colours.iter().filter(|c| **c == colour).count();
        let clay = colours.iter().filter(|c| **c == UNPAINTED).count();
        assert!(painted > 200, "only {painted} vertices of the PLY are the paint's colour");
        assert!(clay > painted, "the unpainted ball is not clay: {clay} of {}", colours.len());
        assert_eq!(painted + clay, colours.len(), "a colour nobody painted is in the file");

        // With no paints mixed, a body painted by filament slot is still
        // exported in colour: the filament's.
        let mut plain = self::app();
        stroke(&mut plain);
        let slot = plain.palette.slots[usize::from(plain.paint_slot) - 1].swatch().into_rgba8();
        let filament = [slot[0], slot[1], slot[2]];
        assert_ne!(filament, UNPAINTED, "the fixture's filament is the clay colour");
        plain.export(crate::message::ExportFormat::Ply, &path);
        let colours = ply_colours(&std::fs::read(&path).expect("the file should be there"));
        assert!(colours.iter().filter(|c| **c == filament).count() > 200);

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A painted body exported with a texture: three files that name each
    /// other, and a picture with the paint in it.
    #[test]
    fn a_painted_body_exports_as_an_obj_a_material_file_and_a_picture() {
        let directory = scratch("textured");
        let path = directory.join("figure.obj");
        let mut app = app();
        update(&mut app, Message::PaintAdded);
        let colour = [20, 140, 200];
        edit(&mut app, PaintEdit::Colour(colour));
        stroke(&mut app);

        // The size is one of those on offer, or it is not taken.
        update(&mut app, Message::TextureSizeChanged(1024));
        assert_eq!(app.texture_size, 1024);
        update(&mut app, Message::TextureSizeChanged(3000));
        assert_eq!(app.texture_size, 1024, "a size not on offer was taken");
        drop(app.textured_exports());

        app.export(crate::message::ExportFormat::Textured, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);

        let obj = std::fs::read_to_string(&path).expect("the OBJ should be there");
        assert!(obj.contains("mtllib figure.mtl"), "the OBJ does not name its material file");
        let places = obj.lines().filter(|line| line.starts_with("vt ")).count();
        let faces = obj.lines().filter(|line| line.starts_with("f ")).count();
        assert!(places > 1000 && faces > 1000, "{places} places and {faces} faces");
        assert!(obj.lines().filter(|line| line.starts_with("f ")).all(|line| line.contains('/')));

        let materials =
            std::fs::read_to_string(directory.join("figure.mtl")).expect("the material file");
        assert!(materials.contains("map_Kd figure_1.png"), "{materials}");

        let picture =
            image::open(directory.join("figure_1.png")).expect("the picture").into_rgba8();
        assert_eq!((picture.width(), picture.height()), (1024, 1024));
        let count =
            |wanted: [u8; 3]| picture.pixels().filter(|pixel| pixel.0[..3] == wanted).count();
        let (painted, clay) = (count(colour), count(UNPAINTED));
        assert!(painted > 500, "only {painted} texels are the paint's colour");
        assert!(clay > painted, "the unpainted ball is not clay: {clay} texels");
        assert!(picture.pixels().all(|pixel| pixel.0[3] == 255), "the picture is not opaque");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A painted body exports as one GLB: one file, that a JSON parser
    /// which is not this application's own reads, with the mesh in metres
    /// and the picture inside it.
    #[test]
    fn a_painted_body_exports_as_a_glb_with_its_picture_inside() {
        let directory = scratch("glb");
        let path = directory.join("figure.glb");
        let mut app = app();
        update(&mut app, Message::PaintAdded);
        let colour = [20, 140, 200];
        edit(&mut app, PaintEdit::Colour(colour));
        stroke(&mut app);
        update(&mut app, Message::TextureSizeChanged(1024));

        app.export(crate::message::ExportFormat::Glb, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        let files: Vec<_> = std::fs::read_dir(&directory)
            .expect("the directory")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert_eq!(files.len(), 1, "a GLB is one file, and there are {files:?}");

        let bytes = std::fs::read(&path).expect("the GLB should be there");
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four")) as usize;
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(word(8), bytes.len(), "the header counts the whole file");
        let json_length = word(12);
        let json: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + json_length]).expect("the JSON chunk is JSON");
        let bin = &bytes[20 + json_length + 8..];
        assert_eq!(json["buffers"][0]["byteLength"].as_u64(), Some(bin.len() as u64));
        let index = |value: &serde_json::Value| value.as_u64().expect("an index") as usize;

        // The picture, found the way a viewer finds it: the mesh's
        // material, its texture, that texture's image, that image's bytes.
        let primitive = &json["meshes"][0]["primitives"][0];
        let material = &json["materials"][index(&primitive["material"])];
        let texture = &material["pbrMetallicRoughness"]["baseColorTexture"]["index"];
        let image = &json["images"][index(&json["textures"][index(texture)]["source"])];
        assert_eq!(image["mimeType"], "image/png");
        let view = &json["bufferViews"][index(&image["bufferView"])];
        let (offset, length) = (index(&view["byteOffset"]), index(&view["byteLength"]));
        let picture = image::load_from_memory(&bin[offset..offset + length])
            .expect("the picture inside is a picture")
            .into_rgba8();
        assert_eq!((picture.width(), picture.height()), (1024, 1024));
        let count =
            |wanted: [u8; 3]| picture.pixels().filter(|pixel| pixel.0[..3] == wanted).count();
        let (painted, clay) = (count(colour), count(UNPAINTED));
        assert!(painted > 500, "only {painted} texels are the paint's colour");
        assert!(clay > painted, "the unpainted ball is not clay: {clay} texels");

        // Metres. The body is as wide in the file, times a thousand, as
        // it is in the document, in millimetres.
        let positions = &json["accessors"][index(&primitive["attributes"]["POSITION"])];
        let across = |axis: usize| {
            positions["max"][axis].as_f64().expect("a number")
                - positions["min"][axis].as_f64().expect("a number")
        };
        let mut visible = Vec::new();
        app.doc.saved_visibility(&mut visible);
        let bodies = app.doc.export_bodies(&visible);
        let mesh = &bodies[0].1;
        let low = mesh.positions.iter().copied().fold(glam::Vec3::splat(f32::MAX), glam::Vec3::min);
        let high =
            mesh.positions.iter().copied().fold(glam::Vec3::splat(f32::MIN), glam::Vec3::max);
        for axis in 0..3 {
            let (file, document) = (across(axis) * 1000.0, f64::from((high - low)[axis]));
            assert!(document > 10.0, "the fixture is {document} mm across: too small to tell");
            assert!(
                (file - document).abs() < 0.5,
                "axis {axis}: {file} mm in the file and {document} mm in the document"
            );
        }

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The 3MF's `paint_color` code for each filament slot, from the table in
    /// `brokkr_core::export::threemf`, for the slots these tests use.
    fn code(filament: u8) -> &'static str {
        ["4", "8", "0C", "1C", "2C", "3C"][usize::from(filament) - 1]
    }

    fn triangles_in(written: &str, filament: u8) -> usize {
        written.matches(&format!("paint_color=\"{}\"", code(filament))).count()
    }

    /// A 3MF says which filament prints each triangle, and a paint number is
    /// not a filament number. The stroke is painted with a paint past the
    /// machine's last filament and told to print in filament 2: the file
    /// names filament 2 for it and no other.
    #[test]
    fn a_3mf_names_the_filament_a_paint_prints_in_and_not_the_paints_number() {
        let directory = scratch("paint-3mf");
        let path = directory.join("painted.3mf");
        let mut app = app();
        update(&mut app, Message::PaintAdded);
        let slot = app.paint_slot;
        assert!(usize::from(slot) > app.palette.slots.len(), "the paint is not past the filaments");
        edit(&mut app, PaintEdit::Filament(2));
        assert_eq!(app.paints.paint(slot).unwrap().filament, 2);
        stroke(&mut app);

        let read = |app: &mut Brokkr| {
            app.export(crate::message::ExportFormat::ThreeMf, &path);
            assert!(app.status.starts_with("exported"), "{}", app.status);
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned()
        };
        let written = read(&mut app);
        let on_two = triangles_in(&written, 2);
        assert!(on_two > 100, "only {on_two} triangles print in filament 2");
        for other in [1, 3, 4] {
            assert_eq!(triangles_in(&written, other), 0, "filament {other} prints something");
        }

        // Told to print in another, the same triangles move to it.
        edit(&mut app, PaintEdit::Filament(3));
        let written = read(&mut app);
        assert_eq!(triangles_in(&written, 3), on_two, "the triangles did not follow the paint");
        assert_eq!(triangles_in(&written, 2), 0);

        // Told nothing, it takes the nearest: the paint is made the colour
        // of filament 1 exactly, so that is the one.
        edit(&mut app, PaintEdit::Filament(0));
        let first = app.palette.slots[0].swatch().into_rgba8();
        update(&mut app, Message::PaintCoatChosen(PaintCoat::Base));
        edit(&mut app, PaintEdit::Colour([first[0], first[1], first[2]]));
        let written = read(&mut app);
        assert_eq!(triangles_in(&written, 1), on_two, "an unnamed paint did not take the nearest");

        // And the export changed nothing in the document: the voxels still
        // hold the paint's number.
        assert!(painted_voxels(&app) > 0);
        let volume = app.doc.active_volume();
        let (low, high) = volume.world_bounds().unwrap();
        let (lo, hi) = volume.voxel_bounds(low, high);
        let mut seen = std::collections::BTreeSet::new();
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    seen.insert(volume.colour().at(glam::IVec3::new(x, y, z)));
                }
            }
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), [0, slot], "exporting rewrote the slots");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A document with no paints exports the 3MF it always did: its slots
    /// are filament slots and nothing maps them.
    #[test]
    fn a_3mf_of_a_document_without_paints_is_untouched_by_all_this() {
        let directory = scratch("plain-3mf");
        let path = directory.join("plain.3mf");
        let mut app = app();
        update(&mut app, Message::PaintSlotChanged(3));
        stroke(&mut app);
        app.export(crate::message::ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();
        assert!(triangles_in(&written, 3) > 100, "slot 3 did not reach the file as filament 3");
        for other in [1, 2, 4] {
            assert_eq!(triangles_in(&written, other), 0);
        }
        std::fs::remove_dir_all(&directory).ok();
    }

    /// Mixed paints name the filament they were, so mixing changes nothing
    /// about how a model already painted by slot prints. And the list a
    /// paint chooses from is the machine's filaments, after "the nearest".
    #[test]
    fn mixed_paints_print_in_the_filaments_they_came_from() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);
        for (index, paint) in app.paints.paints().iter().enumerate() {
            assert_eq!(usize::from(paint.filament), index + 1, "{}", paint.name);
        }
        let choices = app.prints_in_choices();
        assert_eq!(choices.len(), app.palette.slots.len() + 1);
        assert_eq!(choices[0].slot, 0);
        assert!(choices[0].label.contains("nearest"));
        for (index, choice) in choices.iter().enumerate().skip(1) {
            assert_eq!(usize::from(choice.slot), index);
            assert!(choice.label.contains(&app.palette.slots[index - 1].name));
        }
        // A new paint names none.
        update(&mut app, Message::PaintAdded);
        assert_eq!(app.paints.paint(app.paint_slot).unwrap().filament, 0);
    }

    /// The range is the starting colours a painter is offered, so two entries
    /// the same would be one wasted, and the undercoat must not be one of
    /// them or picking it would silently mean "keep the old pigment".
    #[test]
    fn the_range_has_no_repeats_and_does_not_hold_the_undercoat() {
        for (index, (name, colour)) in RANGE.iter().enumerate() {
            assert_ne!(*colour, UNDERCOAT, "{name} is the undercoat");
            for (other, other_colour) in &RANGE[index + 1..] {
                assert_ne!(colour, other_colour, "{name} and {other} are the same colour");
                assert_ne!(name, other, "two paints are called {name}");
            }
        }
    }
}
