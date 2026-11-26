// SPDX-License-Identifier: AGPL-3.0-only

//! How the paint brush touches the model (loaded, a wash, a drybrush, a
//! sponge, a fill), the brush's settings for each, and the exports that
//! carry colour.
//!
//! Painting by filament slot, and the recipes it was mixed from, were
//! here until the maintainer removed them on 2026-11-25. A document
//! painted that way opens with its slots laid into colour paint
//! (`brokkr_core::paint::old_slots`); the document's paint table is still
//! read from a file for that and nothing else.

use brokkr_core::paint::touch::{Coat, Drybrush, Sponge, Touch};
use glam::Vec3;
use iced::widget::{button, checkbox, column, row, text};
use iced::{Element, Length};

use super::Brokkr;
use crate::message::{Message, PaintTouch, TouchSetting};
use crate::theme;

/// The colour a PLY gives a surface nobody painted: the viewport's clay, as
/// near as one flat colour can be to a lit material. A warm mid grey, so
/// that an unpainted part of a model reads as "not painted" and not as a
/// paint somebody chose.
pub(super) const UNPAINTED: [u8; 3] = [168, 160, 150];

/// The sizes a baked texture can be, in texels on a side. The middle one is
/// what the application starts on: at a 28 mm figure it is a texel every 30
/// microns or so, which is finer than the voxels the paint is in.
pub(super) const TEXTURE_SIZES: [u32; 3] = [1024, 2048, 4096];

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
            TouchSetting::SpongeReach(value) => self.sponge.reach = value.clamp(0.0, 1.0),
            TouchSetting::SpongeLight(value) => self.sponge.toward_light = value.clamp(0.0, 1.0),
            TouchSetting::SpongeFoam(foam) => self.sponge.foam = foam,
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
            // loaded coat is the answer that does least if it ever is. And
            // a wash is the colour brush's, which reads the kind itself.
            PaintTouch::Fill
            | PaintTouch::Wash
            | PaintTouch::Contrast
            | PaintTouch::Airbrush
            | PaintTouch::Decal => Touch::Loaded,
        };
        let load = if self.load_mm.0 > 0.0 {
            (1.0 - self.travelled / self.load_mm.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        Coat { touch, seed: self.seed, origin: self.origin, load, crease_gain }
    }
}

impl Brokkr {
    /// The machine's filaments as colours, slot 1 first.
    pub(super) fn filament_colours(&self) -> Vec<[u8; 3]> {
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
            table: self.doc.paints(),
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
                // And what is painted in colour prints in the filament
                // nearest it.
                for (meta, mesh, _) in bodies.iter_mut() {
                    mesh.slots = match self.doc.volume(meta.id) {
                        Some(volume) => bake.printed_slots(volume, mesh),
                        None => bake.filament_slots(mesh),
                    };
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
            table: self.doc.paints(),
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
        let encoded = |texture: &brokkr_core::export::texture::Texture| {
            let mut png = Vec::new();
            image::write_buffer_with_format(
                &mut std::io::Cursor::new(&mut png),
                &texture.rgba,
                texture.size,
                texture.size,
                image::ExtendedColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .map_err(std::io::Error::other)
            .map(|()| png)
        };
        let mut pictures = Vec::with_capacity(bodies.len());
        for (_, texture) in &textures {
            pictures.push(encoded(texture)?);
        }
        // Every body's occlusion, roughness and metal, as a second picture.
        // See `brokkr_core::export::glb`.
        let margin = brokkr_core::export::unwrap::UnwrapSpec::default().margin;
        let mut finishes = Vec::with_capacity(bodies.len());
        for ((meta, mesh, _), (unwrap, _)) in bodies.iter().zip(&textures) {
            finishes.push(match self.doc.volume(meta.id) {
                Some(volume) => Some(encoded(&unwrap.finish_baked(mesh, volume, margin))?),
                None => None,
            });
        }
        let parts: Vec<brokkr_core::export::glb::Body<'_>> = bodies
            .iter()
            .zip(&textures)
            .zip(&pictures)
            .zip(&finishes)
            .map(|((((meta, mesh, _), (unwrap, _)), png), finish)| brokkr_core::export::glb::Body {
                name: meta.name.as_str(),
                mesh,
                unwrap,
                png,
                finish_png: finish.as_deref(),
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
                button(text(format!("{size}")).size(theme::LABEL_SIZE))
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
                    button(text(format.label()).size(theme::LABEL_SIZE))
                        .style(theme::tool_button)
                        .on_press(Message::Export(format)),
                )
            });
        column![
            text("With the paint as a texture, of size")
                .size(theme::LABEL_SIZE)
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

    /// The lines a fill stops at and how far it may run, as the panel has
    /// them set, for pouring `slot`.
    pub(super) fn fill_lines(&self, slot: u8) -> brokkr_core::paint::fill::Fill {
        let sharpness = Some(self.touch.fill_sharpness);
        brokkr_core::paint::fill::Fill {
            slot,
            stop_at_creases: sharpness.filter(|_| self.touch.fill_stops_at_creases),
            stop_at_edges: sharpness.filter(|_| self.touch.fill_stops_at_edges),
            crease_gain: super::CAVITY_FEATURE_MM / self.doc.voxel_size() / 4.0,
            limit: FILL_LIMIT,
            into_the_line: brokkr_core::paint::fill::INTO_THE_LINE,
        }
    }

    /// What a fill did, on the status line.
    pub(super) fn say_what_was_filled(&mut self, filled: brokkr_core::paint::fill::Filled) {
        if filled.reached_the_limit {
            self.status = format!(
                "the fill stopped at {FILL_LIMIT} voxels of surface: nothing on the model stopped \
                 it sooner. Coat the body paints all of it"
            );
        } else if filled.voxels > 0 {
            self.status = format!("filled {} voxels", filled.voxels);
        }
    }

    /// How the brush touches: a button each, and the settings of the one
    /// chosen.
    pub(super) fn touch_picker(&self, settings: bool) -> Element<'_, Message> {
        let offered: Vec<PaintTouch> = PaintTouch::ALL.to_vec();
        // Four to a row and no more: five in one row, seen on a screen,
        // cut the end off "Loaded". Five are laid out as three and two.
        // Eight are three, three and two: four in a row clipped "Wash",
        // seen on a screen.
        let rows: Vec<&[PaintTouch]> = match offered.len() {
            0..=4 => vec![&offered[..]],
            _ => offered.chunks(3).collect(),
        };
        let mut picker = column![].spacing(theme::S1);
        for line in rows {
            let kinds = line.iter().fold(row![].spacing(theme::S1), |assembled, kind| {
                assembled.push(
                    button(text(kind.label()).size(theme::LABEL_SIZE))
                        // A share of the row for each letter and two for
                        // the padding: in four equal buttons, seen on a
                        // screen, "Drybrush" ran into its own edge.
                        .width(Length::FillPortion(kind.label().len() as u16 + 2))
                        .style(if *kind == self.touch.kind {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::PaintTouchChosen(*kind)),
                )
            });
            picker = picker.push(kinds);
        }
        if !settings {
            return picker.into();
        }
        let changed = |setting: TouchSetting| Message::PaintTouchChanged(setting);
        match self.touch.kind {
            PaintTouch::Loaded => {}
            PaintTouch::Wash | PaintTouch::Contrast => {}
            PaintTouch::Airbrush => picker = picker.push(self.airbrush_settings()),
            PaintTouch::Decal => picker = picker.push(self.decal_settings()),
            PaintTouch::Drybrush => {
                let dry = self.touch.dry;
                let load = self.touch.load_mm.0;
                picker = picker
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.catches",
                            "Catches",
                            dry.catch,
                            CATCH_RANGE,
                            move |v| changed(TouchSetting::DryCatch(v)),
                        )
                        .step(0.01)
                        .shown(format!("{:.2}", dry.catch)),
                    )
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.streaks",
                            "Streaks",
                            dry.streak,
                            STREAK_RANGE,
                            move |v| changed(TouchSetting::DryStreak(v)),
                        )
                        .step(0.01)
                        .shown(format!("{:.2}", dry.streak)),
                    )
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.bristle",
                            "Bristle",
                            dry.bristle_mm,
                            BRISTLE_RANGE,
                            move |v| changed(TouchSetting::DryBristle(v)),
                        )
                        .step(0.1)
                        .shown(format!("{:.1} mm", dry.bristle_mm)),
                    )
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.runs_out",
                            "Runs out after",
                            load,
                            LOAD_RANGE,
                            move |v| changed(TouchSetting::DryLoad(v)),
                        )
                        .step(5.0)
                        .shown(if load > 0.0 {
                            format!("{load:.0} mm")
                        } else {
                            "never".to_string()
                        }),
                    );
            }
            PaintTouch::Sponge => {
                let sponge = self.touch.sponge;
                picker = picker
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.covers",
                            "Covers",
                            sponge.density,
                            DENSITY_RANGE,
                            move |v| changed(TouchSetting::SpongeDensity(v)),
                        )
                        .step(0.01)
                        .shown(format!("{:.0}%", sponge.density * 100.0)),
                    )
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.speck",
                            "Speck",
                            sponge.grain_mm,
                            GRAIN_RANGE,
                            move |v| changed(TouchSetting::SpongeGrain(v)),
                        )
                        .step(0.1)
                        .shown(format!("{:.1} mm", sponge.grain_mm)),
                    )
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.more_on_edges",
                            "More on edges",
                            sponge.edges,
                            EDGES_RANGE,
                            move |v| changed(TouchSetting::SpongeEdges(v)),
                        )
                        .step(0.01)
                        .shown(format!("{:.2}", sponge.edges)),
                    )
                    // The reworked sponge: stage 4 of `docs/PAINT-RACK.md`.
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.reach",
                            "Into recesses",
                            sponge.reach,
                            0.0..=1.0,
                            move |v| changed(TouchSetting::SpongeReach(v)),
                        )
                        .step(0.01)
                        .shown(format!("{:.0}%", sponge.reach * 100.0)),
                    )
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.toward_light",
                            "Toward the light",
                            sponge.toward_light,
                            0.0..=1.0,
                            move |v| changed(TouchSetting::SpongeLight(v)),
                        )
                        .step(0.01)
                        .shown(format!("{:.2}", sponge.toward_light)),
                    )
                    .push(crate::ui::segmented(
                        brokkr_core::paint::touch::Foam::ALL.map(|foam| (foam, foam.label())),
                        sponge.foam,
                        move |foam| changed(TouchSetting::SpongeFoam(foam)),
                    ));
            }
            PaintTouch::Fill => {
                let touch = &self.touch;
                picker = picker
                    .push(
                        checkbox(touch.fill_stops_at_creases)
                            .label("stops at creases")
                            .on_toggle(move |on| changed(TouchSetting::FillStopsAtCreases(on)))
                            .text_size(theme::LABEL_SIZE),
                    )
                    .push(
                        checkbox(touch.fill_stops_at_edges)
                            .label("stops at edges")
                            .on_toggle(move |on| changed(TouchSetting::FillStopsAtEdges(on)))
                            .text_size(theme::LABEL_SIZE),
                    )
                    .push(
                        crate::ui::number(
                            &self.drafts,
                            "paints.a_line_is_sharper_than",
                            "A line is sharper than",
                            touch.fill_sharpness,
                            SHARPNESS_RANGE,
                            move |v| changed(TouchSetting::FillSharpness(v)),
                        )
                        .step(0.01)
                        .shown(format!("{:.2}", touch.fill_sharpness)),
                    );
                // How alike is alike. See `app/painting.rs`.
                picker = self.fill_likeness(picker);
            }
        }
        picker.into()
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

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
        // As if a frame followed: a move waits for one. See `pointer_in`.
        app.handle_moves();
    }

    /// The paint brush in hand.
    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
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
                    if volume.paint().at(glam::IVec3::new(x, y, z))[3] != 0 {
                        count += 1;
                    }
                }
            }
        }
        count
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
            drop(app.touch_picker(true));
            drop(app.touch_picker(false));
        }
    }

    /// Within a few levels a channel: the middle of a soft-edged stroke.
    fn near(one: [u8; 3], other: [u8; 3]) -> bool {
        one.iter().zip(&other).all(|(a, b)| a.abs_diff(*b) <= 6)
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
        let colour = [20, 140, 200];
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour(colour)));
        // A broad full middle, so that much of the stroke is the colour
        // itself and not its soft edge.
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Wide));
        stroke(&mut app);

        app.export(crate::message::ExportFormat::Ply, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        let colours = ply_colours(&std::fs::read(&path).expect("the file should be there"));
        // Near the paint's colour, not exactly it: a colour stroke has a
        // soft edge, where slot paint had none.
        let painted = colours.iter().filter(|c| near(**c, colour)).count();
        let clay = colours.iter().filter(|c| **c == UNPAINTED).count();
        assert!(painted > 100, "only {painted} vertices of the PLY are the paint's colour");
        assert!(clay > painted, "the unpainted ball is not clay: {clay} of {}", colours.len());

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A painted body exported with a texture: three files that name each
    /// other, and a picture with the paint in it.
    #[test]
    fn a_painted_body_exports_as_an_obj_a_material_file_and_a_picture() {
        let directory = scratch("textured");
        let path = directory.join("figure.obj");
        let mut app = app();
        let colour = [20, 140, 200];
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour(colour)));
        // A broad full middle, so that much of the stroke is the colour
        // itself and not its soft edge.
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Wide));
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
        let count = |wanted: [u8; 3]| {
            picture
                .pixels()
                .filter(|pixel| near([pixel.0[0], pixel.0[1], pixel.0[2]], wanted))
                .count()
        };
        let (painted, clay) = (count(colour), count(UNPAINTED));
        assert!(painted > 200, "only {painted} texels are the paint's colour");
        assert!(clay > painted, "the unpainted ball is not clay: {clay} texels");
        assert!(picture.pixels().all(|pixel| pixel.0[3] == 255), "the picture is not opaque");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A body with a gloss varnish on it exports as a GLB whose material
    /// names a roughness and metal picture, smooth where the varnish is.
    /// Stage 6 of `docs/PAINT-RACK.md`.
    #[test]
    fn a_varnished_body_exports_its_roughness_in_the_glb() {
        let directory = scratch("glb-varnish");
        let path = directory.join("figure.glb");
        let mut app = app();
        update(&mut app, Message::TextureSizeChanged(512));
        update(
            &mut app,
            Message::Coats(crate::message::CoatsMessage::Varnish(
                brokkr_core::paint::Finish::Gloss,
            )),
        );
        update(&mut app, Message::Coats(crate::message::CoatsMessage::VarnishCoat));
        app.export(crate::message::ExportFormat::Glb, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);

        let bytes = std::fs::read(&path).expect("the GLB should be there");
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four")) as usize;
        let json_length = word(12);
        let json: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + json_length]).expect("the JSON chunk is JSON");
        let bin = &bytes[20 + json_length + 8..];
        let index = |value: &serde_json::Value| value.as_u64().expect("an index") as usize;
        let primitive = &json["meshes"][0]["primitives"][0];
        let material = &json["materials"][index(&primitive["material"])];
        let texture = &material["pbrMetallicRoughness"]["metallicRoughnessTexture"]["index"];
        let image = &json["images"][index(&json["textures"][index(texture)]["source"])];
        let view = &json["bufferViews"][index(&image["bufferView"])];
        let (offset, length) = (index(&view["byteOffset"]), index(&view["byteLength"]));
        let picture = image::load_from_memory(&bin[offset..offset + length])
            .expect("the picture inside is a picture")
            .into_rgba8();
        let (rough, _) = brokkr_core::paint::Finish::Gloss.roughness_and_metal();
        let gloss = (rough * 255.0).round() as u8;
        let smooth =
            picture.pixels().filter(|pixel| pixel.0[1] == gloss && pixel.0[2] == 0).count();
        assert!(smooth > 1000, "only {smooth} texels have the gloss's roughness");
    }

    /// A 3MF prints colour paint in the loaded filament nearest its colour:
    /// a stroke in filament 2's colour prints in filament 2 and in no other.
    #[test]
    fn a_3mf_prints_colour_paint_in_the_nearest_filament() {
        let directory = scratch("paint-3mf");
        let path = directory.join("painted.3mf");
        let mut app = app();
        let second = app.palette.slots[1].swatch().into_rgba8();
        let colour = [second[0], second[1], second[2]];
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour(colour)));
        // A broad full middle, so that much of the stroke is the colour
        // itself and not its soft edge.
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Wide));
        stroke(&mut app);
        assert!(painted_voxels(&app) > 0);
        app.export(crate::message::ExportFormat::ThreeMf, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();
        assert!(triangles_in(&written, 2) > 100, "the stroke did not print in filament 2");
        for other in [3, 4] {
            assert_eq!(triangles_in(&written, other), 0, "filament {other} prints something");
        }
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
        let colour = [20, 140, 200];
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour(colour)));
        // A broad full middle, so that much of the stroke is the colour
        // itself and not its soft edge.
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Wide));
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
        let count = |wanted: [u8; 3]| {
            picture
                .pixels()
                .filter(|pixel| near([pixel.0[0], pixel.0[1], pixel.0[2]], wanted))
                .count()
        };
        let (painted, clay) = (count(colour), count(UNPAINTED));
        assert!(painted > 200, "only {painted} texels are the paint's colour");
        assert!(clay > painted, "the unpainted ball is not clay: {clay} texels");

        // Its occlusion, in the red of its roughness and metal picture: a
        // ball is open to the sky everywhere, so the red is nearly full.
        let packed = &material["occlusionTexture"]["index"];
        assert_eq!(
            packed, &material["pbrMetallicRoughness"]["metallicRoughnessTexture"]["index"],
            "the occlusion is not the packed picture"
        );
        let image = &json["images"][index(&json["textures"][index(packed)]["source"])];
        let view = &json["bufferViews"][index(&image["bufferView"])];
        let (offset, length) = (index(&view["byteOffset"]), index(&view["byteLength"]));
        let occlusion = image::load_from_memory(&bin[offset..offset + length])
            .expect("the packed picture is a picture")
            .into_rgba8();
        // Of the texels baked, which are the matte's roughness in green;
        // the rest of the picture is the unwrap's grey background.
        let matte = (brokkr_core::paint::Finish::Matte.roughness_and_metal().0 * 255.0).round();
        let baked: Vec<u8> = occlusion
            .pixels()
            .filter(|pixel| f32::from(pixel.0[1]) == matte)
            .map(|pixel| pixel.0[0])
            .collect();
        let open = baked.iter().filter(|red| **red >= 240).count();
        assert!(baked.len() > 100_000, "only {} texels were baked", baked.len());
        assert!(open * 10 > baked.len() * 9, "a ball reads shut in: {open} of {}", baked.len());

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
}
