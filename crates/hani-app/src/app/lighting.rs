// SPDX-License-Identifier: AGPL-3.0-only

//! The Lighting block: the lights, the global light and the unpainted
//! surface's material, set while looking at the model. Issue 203, stage
//! two; stage one is the lighting model itself (`hani_gpu::lights`).
//!
//! Asked for by the maintainer on 2026-11-27: "Maybe a way to set lights
//! which move with the camera vs a global illumination vs lights fixed
//! globally", and of the materials: "grey polystyrene, green stuff ( epoxy
//! clay ), pewter and resin", then "let them choose the color of plastic
//! and resin, but pewter and greenstuff stay fixed."
//!
//! # Where it is
//!
//! LIGHTING is the first section of the dock's second tab in every
//! workspace, and View > Lighting opens it there (and shuts it). Lights are
//! set by looking at the model, so the block is beside the model and not a
//! card over it; and the view is the same in Sculpt and in Paint, so the
//! block is the same in both. It starts shut, a heading of one line.
//!
//! # What it holds
//!
//! - A preset, each one press: [`Preset`]. Choosing one replaces the rig;
//!   changing anything of the rig after makes it the user's own, and no
//!   preset is lit.
//! - Up to four lights ([`Lamp`]), one shown at a time: on or off, with the
//!   camera or fixed in the world, its direction as two angles (round the
//!   model and up from level, `hani_gpu::rigs`), its strength, its colour,
//!   and under Shape how far past side-on it reaches, how fast it fades
//!   there and how much of a highlight it casts. Those three are what
//!   Studio's lights have that a plain light does not (the matcap's key
//!   wraps and its fill casts no highlight), and since they are kept they
//!   have their controls.
//! - The global light: its strength, the sky's colour and the ground's.
//! - The material of every body where it is unpainted, and the colours of
//!   the two that take one, each kept apart from the other.
//! - Reset, which puts back Studio and the clay.
//!
//! A light that is off is kept in its place, its colour nought in the
//! block, so that the first light stays the first (a material's `key`
//! colour is lit by it).
//!
//! # Kept
//!
//! All of it, in `lighting.conf` under the configuration directory, a
//! `key = value` file (`paths::entries`). Written half a second after the
//! last change, from the frame tick, so a slider's drag or a field typed
//! into is one write and not sixty a second. Read at start, in
//! `Hani::new` and not in what the tests build through. What cannot be read
//! (a number that is not one or out of its range, more than four lights, a
//! name nobody knows) is the default for that value, and a missing file is
//! the defaults. A preset is kept by its name and read back as that preset,
//! so a preset whose numbers a later build tunes follows the build. In a
//! browser there is no file, and nothing is kept (`docs/WEB.md`).

use std::ops::RangeInclusive;

use hani_gpu::lights::{
    Light, LightBlock, MAX_LIGHTS, Material, POLYSTYRENE_GREY, RESIN_GREY, RESIN_TAN, RESIN_WHITE,
    Rig,
};
use hani_gpu::rigs;
use iced::widget::{checkbox, column, container, pick_list, row, space, text};
use iced::{Alignment, Element, Length};
use web_time::Instant;

use super::Hani;
use super::colour_field;
use crate::message::{DockTab, Message, PanelSection};
use crate::theme;

/// The settings file.
const FILE: &str = "lighting.conf";

/// How long after the last change the file is written.
const STILL_FOR: f32 = 0.5;

/// How strong the global light can be set.
pub(super) const GLOBAL: RangeInclusive<f32> = 0.0..=2.0;

/// A ready-made rig, each one press. See `hani_gpu::rigs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Studio,
    Lightbox,
    Overhead,
    Sky,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Studio, Preset::Lightbox, Preset::Overhead, Preset::Sky];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Studio => "Studio",
            Preset::Lightbox => "Lightbox",
            Preset::Overhead => "Overhead",
            Preset::Sky => "Sky",
        }
    }

    /// Its name in the file.
    fn slug(self) -> &'static str {
        match self {
            Preset::Studio => "studio",
            Preset::Lightbox => "lightbox",
            Preset::Overhead => "overhead",
            Preset::Sky => "sky",
        }
    }

    fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.slug() == slug)
    }

    pub fn rig(self) -> Rig {
        match self {
            Preset::Studio => rigs::studio(),
            Preset::Lightbox => rigs::lightbox(),
            Preset::Overhead => rigs::overhead(),
            Preset::Sky => rigs::sky(),
        }
    }

    /// What it does to the picture, on hover.
    fn tip(self) -> &'static str {
        match self {
            Preset::Studio => "The usual lights, turning with the view",
            Preset::Lightbox => "Key, fill and rim fixed round the model: turning moves the shine",
            Preset::Overhead => "One strong light from above, and a sky",
            Preset::Sky => "Only the sky and the ground, no light of its own",
        }
    }
}

/// The material of every body where it is unpainted. The two that take a
/// colour take it from [`Setting::polystyrene`] and [`Setting::resin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Unpainted {
    #[default]
    Clay,
    Polystyrene,
    GreenStuff,
    Pewter,
    Resin,
}

impl Unpainted {
    pub const ALL: [Unpainted; 5] = [
        Unpainted::Clay,
        Unpainted::Polystyrene,
        Unpainted::GreenStuff,
        Unpainted::Pewter,
        Unpainted::Resin,
    ];

    fn slug(self) -> &'static str {
        match self {
            Unpainted::Clay => "clay",
            Unpainted::Polystyrene => "polystyrene",
            Unpainted::GreenStuff => "green-stuff",
            Unpainted::Pewter => "pewter",
            Unpainted::Resin => "resin",
        }
    }

    fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|each| each.slug() == slug)
    }
}

impl std::fmt::Display for Unpainted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let material = match self {
            Unpainted::Clay => Material::Clay,
            Unpainted::Polystyrene => Material::Polystyrene(POLYSTYRENE_GREY),
            Unpainted::GreenStuff => Material::GreenStuff,
            Unpainted::Pewter => Material::Pewter,
            Unpainted::Resin => Material::Resin(RESIN_WHITE),
        };
        f.write_str(material.label())
    }
}

/// A colour the block sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    /// The light at this place in the rig.
    Light(usize),
    Sky,
    Ground,
    Polystyrene,
    Resin,
}

/// One of a light's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LampNumber {
    /// Degrees round the model: 0 in front, 90 to the right.
    Round,
    /// Degrees up from level.
    Up,
    Strength,
    /// How far past side-on it reaches, as a cosine.
    Wrap,
    /// The power its light fades by toward side-on.
    Falloff,
    /// How much of a highlight it casts, 0..=1.
    Highlight,
}

impl LampNumber {
    pub const ALL: [LampNumber; 6] = [
        LampNumber::Round,
        LampNumber::Up,
        LampNumber::Strength,
        LampNumber::Wrap,
        LampNumber::Falloff,
        LampNumber::Highlight,
    ];

    pub fn range(self) -> RangeInclusive<f32> {
        match self {
            LampNumber::Round => 0.0..=360.0,
            LampNumber::Up => -90.0..=90.0,
            LampNumber::Strength => 0.0..=3.0,
            LampNumber::Wrap => 0.0..=1.0,
            LampNumber::Falloff => 0.5..=4.0,
            LampNumber::Highlight => 0.0..=1.0,
        }
    }

    /// Its draft's key, and its name in the file.
    fn key(self) -> &'static str {
        match self {
            LampNumber::Round => "lighting.round",
            LampNumber::Up => "lighting.up",
            LampNumber::Strength => "lighting.strength",
            LampNumber::Wrap => "lighting.wrap",
            LampNumber::Falloff => "lighting.falloff",
            LampNumber::Highlight => "lighting.highlight",
        }
    }

    fn label(self) -> &'static str {
        match self {
            LampNumber::Round => "Round",
            LampNumber::Up => "Up",
            LampNumber::Strength => "Strength",
            LampNumber::Wrap => "Wrap",
            LampNumber::Falloff => "Falloff",
            LampNumber::Highlight => "Highlight",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            LampNumber::Round => "0 in front, 90 right, 180 behind, 270 left",
            LampNumber::Up => "0 level, 90 straight above, -90 straight below",
            LampNumber::Strength => "1 is a full light",
            LampNumber::Wrap => "How far round past side-on it lights the form",
            LampNumber::Falloff => "How fast it fades toward side-on",
            LampNumber::Highlight => "How much of a shine it puts on the surface",
        }
    }
}

/// One light as the block sets it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lamp {
    pub on: bool,
    pub with_camera: bool,
    pub round: f32,
    pub up: f32,
    pub strength: f32,
    /// Linear RGB, 0..=1, as the rig holds it.
    pub colour: [f32; 3],
    pub wrap: f32,
    pub falloff: f32,
    pub highlight: f32,
}

impl Lamp {
    /// A rig's light.
    pub fn of(light: &Light) -> Self {
        let (round, up) = rigs::angles(light.direction);
        Self {
            on: true,
            with_camera: light.with_camera,
            round,
            up,
            strength: light.strength,
            colour: light.colour,
            wrap: light.wrap,
            falloff: light.falloff,
            highlight: light.highlight,
        }
    }

    /// The light it is in the rig. One that is off has no strength.
    pub fn light(&self) -> Light {
        Light {
            wrap: self.wrap,
            falloff: self.falloff,
            highlight: self.highlight,
            ..Light::new(
                rigs::toward(self.round, self.up),
                self.colour,
                if self.on { self.strength } else { 0.0 },
                self.with_camera,
            )
        }
    }

    /// What Add puts in: a plain white light fixed in the world, high on
    /// the front right, at half strength. Also what a light's value that
    /// cannot be read from the file is.
    pub fn added() -> Self {
        Self::of(&Light::new(rigs::toward(45.0, 30.0), [1.0; 3], 0.5, false))
    }

    pub fn number(&self, which: LampNumber) -> f32 {
        match which {
            LampNumber::Round => self.round,
            LampNumber::Up => self.up,
            LampNumber::Strength => self.strength,
            LampNumber::Wrap => self.wrap,
            LampNumber::Falloff => self.falloff,
            LampNumber::Highlight => self.highlight,
        }
    }

    fn set(&mut self, which: LampNumber, value: f32) {
        let value = value.clamp(*which.range().start(), *which.range().end());
        match which {
            LampNumber::Round => self.round = value,
            LampNumber::Up => self.up = value,
            LampNumber::Strength => self.strength = value,
            LampNumber::Wrap => self.wrap = value,
            LampNumber::Falloff => self.falloff = value,
            LampNumber::Highlight => self.highlight = value,
        }
    }
}

/// Everything the block sets, which is everything kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// The preset the rig is, or none once it has been changed.
    pub preset: Option<Preset>,
    /// At most [`MAX_LIGHTS`].
    pub lamps: Vec<Lamp>,
    pub global: f32,
    /// Linear RGB, 0..=1.
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    pub material: Unpainted,
    /// sRGB, as `hani_gpu::lights::Material` takes them.
    pub polystyrene: [u8; 3],
    pub resin: [u8; 3],
}

impl Default for Setting {
    /// Studio and the clay: the viewport as it has always been.
    fn default() -> Self {
        let mut setting = Self {
            preset: None,
            lamps: Vec::new(),
            global: 0.0,
            sky: [0.0; 3],
            ground: [0.0; 3],
            material: Unpainted::Clay,
            polystyrene: POLYSTYRENE_GREY,
            resin: RESIN_WHITE,
        };
        setting.take(Preset::Studio);
        setting
    }
}

impl Setting {
    /// Put a preset's rig in place of the one there is.
    fn take(&mut self, preset: Preset) {
        let rig = preset.rig();
        self.preset = Some(preset);
        self.lamps = rig.lights.iter().take(MAX_LIGHTS).map(Lamp::of).collect();
        self.global = rig.global;
        self.sky = rig.sky;
        self.ground = rig.ground;
    }

    /// The rig the renderer is given.
    pub fn rig(&self) -> Rig {
        Rig {
            lights: self.lamps.iter().map(Lamp::light).collect(),
            global: self.global,
            sky: self.sky,
            ground: self.ground,
        }
    }

    /// The material the renderer is given.
    pub fn material(&self) -> Material {
        match self.material {
            Unpainted::Clay => Material::Clay,
            Unpainted::Polystyrene => Material::Polystyrene(self.polystyrene),
            Unpainted::GreenStuff => Material::GreenStuff,
            Unpainted::Pewter => Material::Pewter,
            Unpainted::Resin => Material::Resin(self.resin),
        }
    }

    pub fn block(&self) -> LightBlock {
        LightBlock::new(&self.rig(), self.material())
    }

    /// A colour, in sRGB as the colour control shows it. `None` for a light
    /// there is not.
    pub fn tint(&self, tint: Tint) -> Option<[u8; 3]> {
        let srgb = hani_core::paint::srgb;
        Some(match tint {
            Tint::Light(at) => srgb(self.lamps.get(at)?.colour),
            Tint::Sky => srgb(self.sky),
            Tint::Ground => srgb(self.ground),
            Tint::Polystyrene => self.polystyrene,
            Tint::Resin => self.resin,
        })
    }

    fn set_tint(&mut self, tint: Tint, rgb: [u8; 3]) {
        let linear = hani_core::paint::linear(rgb);
        match tint {
            Tint::Light(at) => {
                if let Some(lamp) = self.lamps.get_mut(at) {
                    lamp.colour = linear;
                }
            }
            Tint::Sky => self.sky = linear,
            Tint::Ground => self.ground = linear,
            Tint::Polystyrene => self.polystyrene = rgb,
            Tint::Resin => self.resin = rgb,
        }
    }

    /// The file.
    pub fn to_text(&self) -> String {
        let three = |c: [f32; 3]| format!("{} {} {}", c[0], c[1], c[2]);
        let mut text = String::new();
        let mut line = |key: &str, value: String| {
            text.push_str(key);
            text.push_str(" = ");
            text.push_str(&value);
            text.push('\n');
        };
        line("preset", self.preset.map_or("own", Preset::slug).to_string());
        line("material", self.material.slug().to_string());
        line("polystyrene", colour_field::hex_of(self.polystyrene));
        line("resin", colour_field::hex_of(self.resin));
        line("global", self.global.to_string());
        line("sky", three(self.sky));
        line("ground", three(self.ground));
        line("lights", self.lamps.len().to_string());
        for (at, lamp) in self.lamps.iter().enumerate() {
            let key = |name: &str| format!("light{}.{name}", at + 1);
            line(&key("on"), lamp.on.to_string());
            line(&key("camera"), lamp.with_camera.to_string());
            for which in LampNumber::ALL {
                line(&key(file_name(which)), lamp.number(which).to_string());
            }
            line(&key("colour"), three(lamp.colour));
        }
        text
    }

    /// What a file says, the default for whatever in it cannot be read.
    pub fn from_text(text: &str) -> Self {
        let entries: std::collections::HashMap<&str, &str> = crate::paths::entries(text).collect();
        let mut setting = Self::default();
        let usual = setting.clone();
        let number = |key: &str, range: RangeInclusive<f32>| {
            entries
                .get(key)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && range.contains(value))
        };
        let colour = |key: &str| {
            let parts: Vec<f32> = entries
                .get(key)?
                .split_whitespace()
                .map(|part| part.parse::<f32>().ok().filter(|c| (0.0..=1.0).contains(c)))
                .collect::<Option<_>>()?;
            <[f32; 3]>::try_from(parts).ok()
        };
        let flag = |key: &str| match entries.get(key).copied() {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        };
        setting.material =
            entries.get("material").and_then(|slug| Unpainted::from_slug(slug)).unwrap_or_default();
        let hex = |key: &str| entries.get(key).and_then(|value| crate::paint_index::hex(value));
        setting.polystyrene = hex("polystyrene").unwrap_or(usual.polystyrene);
        setting.resin = hex("resin").unwrap_or(usual.resin);

        if let Some(preset) = entries.get("preset").and_then(|slug| Preset::from_slug(slug)) {
            setting.take(preset);
            return setting;
        }
        let Some(count) = entries
            .get("lights")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|count| *count <= MAX_LIGHTS)
        else {
            // No rig that can be read: Studio's.
            return setting;
        };
        setting.preset = None;
        setting.global = number("global", GLOBAL).unwrap_or(usual.global);
        setting.sky = colour("sky").unwrap_or(usual.sky);
        setting.ground = colour("ground").unwrap_or(usual.ground);
        setting.lamps = (1..=count)
            .map(|at| {
                let key = |name: &str| format!("light{at}.{name}");
                let mut lamp = Lamp::added();
                lamp.on = flag(&key("on")).unwrap_or(lamp.on);
                lamp.with_camera = flag(&key("camera")).unwrap_or(lamp.with_camera);
                for which in LampNumber::ALL {
                    if let Some(value) = number(&key(file_name(which)), which.range()) {
                        lamp.set(which, value);
                    }
                }
                lamp.colour = colour(&key("colour")).unwrap_or(lamp.colour);
                lamp
            })
            .collect();
        setting
    }

    /// The file at `path`, or the defaults.
    pub fn read_from(path: Option<&std::path::Path>) -> Self {
        path.and_then(|path| std::fs::read_to_string(path).ok())
            .map_or_else(Self::default, |text| Self::from_text(&text))
    }

    /// Write the file at `path`. Nothing, and no fault, where there is no
    /// settings directory, as in a browser.
    pub fn keep_in(&self, path: Option<&std::path::Path>) -> Result<(), String> {
        let Some(path) = path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
        }
        std::fs::write(path, self.to_text()).map_err(|why| why.to_string())
    }
}

/// A light's number's name in the file.
fn file_name(which: LampNumber) -> &'static str {
    which.key().trim_start_matches("lighting.")
}

/// A change in the block.
#[derive(Debug, Clone, PartialEq)]
pub enum LightingMessage {
    /// View > Lighting: the block shown in the dock, or shut.
    Shown(bool),
    Preset(Preset),
    /// Show this light's controls.
    Chosen(usize),
    Added,
    Removed(usize),
    On(usize, bool),
    WithCamera(usize, bool),
    Lamp(usize, LampNumber, f32),
    /// The global light's strength.
    Global(f32),
    Colour(Tint, [u8; 3]),
    /// One channel of a colour, red 0, green 1, blue 2.
    Channel(Tint, usize, u8),
    /// A colour's hex field, as typed.
    Hex(Tint, String),
    /// Show this colour's red, green and blue, or none.
    Opened(Option<Tint>),
    /// A light's Shape, shown or folded.
    Shaped(bool),
    Material(Unpainted),
    Reset,
}

/// The block's state: the setting, and how the block is showing it.
#[derive(Debug, Clone, Default)]
pub(super) struct LightingState {
    pub setting: Setting,
    /// The light whose controls are shown.
    pub chosen: usize,
    /// The colour whose channels are shown.
    pub open: Option<Tint>,
    /// A hex field being typed into, and what is in it.
    pub typed: Option<(Tint, String)>,
    /// A light's Shape is unfolded.
    pub shaped: bool,
    /// When the setting last changed and has not been kept since.
    changed: Option<Instant>,
}

impl LightingState {
    /// The kept setting, or the defaults. Never read by a test, which must
    /// not depend on the user's files.
    pub(super) fn kept() -> Self {
        let setting = if cfg!(test) {
            Setting::default()
        } else {
            Setting::read_from(crate::paths::config_file(FILE).as_deref())
        };
        Self { setting, ..Self::default() }
    }
}

impl Hani {
    /// Hand the setting to the renderer.
    pub(super) fn publish_lighting(&self) {
        self.shared.set_lights(self.lighting.setting.block());
    }

    /// Whether the block is on screen: its tab up and its section open.
    pub(super) fn lighting_shown(&self) -> bool {
        self.desk.memory[self.desk.now as usize].tab == DockTab::Inspector
            && self.expanded[PanelSection::Lighting as usize]
    }

    pub(super) fn lighting_changed(&mut self, message: LightingMessage) {
        let before = self.lighting.setting.clone();
        let state = &mut self.lighting;
        let setting = &mut state.setting;
        match message {
            LightingMessage::Shown(shown) => {
                if shown {
                    self.desk.memory[self.desk.now as usize].tab = DockTab::Inspector;
                }
                self.expanded[PanelSection::Lighting as usize] = shown;
                return;
            }
            LightingMessage::Preset(preset) => {
                setting.take(preset);
                state.chosen = 0;
                self.status = format!("{} lights", preset.label());
            }
            LightingMessage::Chosen(at) => {
                if at < setting.lamps.len() {
                    state.chosen = at;
                    state.typed = None;
                    for which in LampNumber::ALL {
                        self.drafts.remove(which.key());
                    }
                    if matches!(state.open, Some(Tint::Light(_))) {
                        state.open = Some(Tint::Light(at));
                    }
                }
            }
            LightingMessage::Added => {
                if setting.lamps.len() >= MAX_LIGHTS {
                    self.status = format!("{MAX_LIGHTS} lights at most");
                    return;
                }
                setting.lamps.push(Lamp::added());
                state.chosen = setting.lamps.len() - 1;
            }
            LightingMessage::Removed(at) => {
                if at < setting.lamps.len() {
                    setting.lamps.remove(at);
                    state.chosen = state.chosen.min(setting.lamps.len().saturating_sub(1));
                    state.open = None;
                    state.typed = None;
                }
            }
            LightingMessage::On(at, on) => {
                if let Some(lamp) = setting.lamps.get_mut(at) {
                    lamp.on = on;
                }
            }
            LightingMessage::WithCamera(at, with) => {
                if let Some(lamp) = setting.lamps.get_mut(at) {
                    lamp.with_camera = with;
                }
            }
            LightingMessage::Lamp(at, which, value) => {
                if let Some(lamp) = setting.lamps.get_mut(at)
                    && value.is_finite()
                {
                    lamp.set(which, value);
                }
            }
            LightingMessage::Global(value) => {
                if value.is_finite() {
                    setting.global = value.clamp(*GLOBAL.start(), *GLOBAL.end());
                }
            }
            LightingMessage::Colour(tint, rgb) => {
                setting.set_tint(tint, rgb);
                state.typed = None;
            }
            LightingMessage::Channel(tint, channel, level) => {
                if let Some(mut rgb) = setting.tint(tint)
                    && let Some(slot) = rgb.get_mut(channel)
                {
                    *slot = level;
                    setting.set_tint(tint, rgb);
                }
                state.typed = None;
            }
            LightingMessage::Hex(tint, typed) => {
                if let Some(rgb) = crate::paint_index::hex(&typed) {
                    setting.set_tint(tint, rgb);
                }
                state.typed = Some((tint, typed));
            }
            LightingMessage::Opened(open) => {
                state.open = open;
                for key in CHANNEL_KEYS {
                    self.drafts.remove(key);
                }
            }
            LightingMessage::Shaped(shaped) => state.shaped = shaped,
            LightingMessage::Material(material) => setting.material = material,
            LightingMessage::Reset => {
                *setting = Setting::default();
                state.chosen = 0;
                state.open = None;
                state.typed = None;
                self.status = "lights and material reset".to_string();
            }
        }
        let setting = &mut self.lighting.setting;
        if *setting == before {
            return;
        }
        // Anything of the rig changed after a preset makes it the user's
        // own. The material is not the rig.
        let rig_changed = setting.lamps != before.lamps
            || setting.global != before.global
            || setting.sky != before.sky
            || setting.ground != before.ground;
        if rig_changed && setting.preset == before.preset {
            setting.preset = None;
        }
        // The chosen light's fields show its own numbers.
        for which in LampNumber::ALL {
            self.drafts.remove(which.key());
        }
        self.lighting.changed = Some(Instant::now());
        self.publish_lighting();
    }

    /// From the frame tick: keep the setting once it has been still for
    /// [`STILL_FOR`].
    pub(super) fn keep_lighting_when_still(&mut self) {
        let Some(changed) = self.lighting.changed else {
            return;
        };
        if changed.elapsed().as_secs_f32() < STILL_FOR {
            return;
        }
        self.lighting.changed = None;
        if cfg!(test) {
            return;
        }
        if let Err(why) = self.lighting.setting.keep_in(crate::paths::config_file(FILE).as_deref())
        {
            self.status = format!("the lighting was not kept: {why}");
        }
    }

    /// LIGHTING, the dock's section.
    pub(super) fn lighting_section(&self) -> Element<'_, Message> {
        self.section(PanelSection::Lighting, || self.lighting_block())
    }

    /// The block's controls.
    pub(super) fn lighting_block(&self) -> Element<'_, Message> {
        let send = Message::Lighting;
        let state = &self.lighting;
        let setting = &state.setting;

        let presets = crate::ui::segmented_tips(
            Preset::ALL.into_iter().map(|preset| (Some(preset), preset.label(), preset.tip())),
            setting.preset,
            move |preset| match preset {
                Some(preset) => send(LightingMessage::Preset(preset)),
                None => send(LightingMessage::Reset),
            },
        );

        let mut tabs = row![crate::ui::sub_label("Lights").width(Length::Fill)]
            .spacing(theme::S1)
            .align_y(Alignment::Center);
        for (at, name) in ["1", "2", "3", "4"].into_iter().enumerate().take(setting.lamps.len()) {
            tabs = tabs.push(
                container(crate::ui::tip(
                    crate::ui::toggle(name, at == state.chosen, send(LightingMessage::Chosen(at))),
                    "This light's settings",
                ))
                .width(Length::Fixed(26.0)),
            );
        }
        if setting.lamps.len() < MAX_LIGHTS {
            tabs = tabs.push(crate::ui::tip(
                crate::ui::button_secondary("Add", Some(send(LightingMessage::Added))),
                "Another light, fixed in place",
            ));
        }

        let mut block = column![presets, tabs].spacing(theme::S3);
        if let Some(lamp) = setting.lamps.get(state.chosen) {
            block = block.push(self.lamp_controls(state.chosen, lamp));
        }

        block = block.push(crate::ui::sub_label("Global light")).push(
            crate::ui::number(
                &self.drafts,
                "lighting.global",
                "Strength",
                setting.global,
                GLOBAL,
                |value| Message::Lighting(LightingMessage::Global(value)),
            )
            .shown(format!("{:.2}", setting.global))
            .tip("The sky above and the ground below, all round"),
        );
        block = block
            .push(self.colour_line("Sky", Tint::Sky))
            .push(self.colour_line("Ground", Tint::Ground));

        block = block.push(crate::ui::sub_label("Unpainted")).push(
            row![
                text("Material").size(theme::LABEL_SIZE).color(theme::TEXT_DIM),
                space().width(Length::Fill),
                pick_list(Unpainted::ALL, Some(setting.material), |material| {
                    Message::Lighting(LightingMessage::Material(material))
                })
                .text_size(theme::LABEL_SIZE),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
        );
        match setting.material {
            Unpainted::Polystyrene => {
                block = block.push(self.colour_line("Colour", Tint::Polystyrene));
            }
            Unpainted::Resin => {
                block = block.push(self.colour_line("Colour", Tint::Resin));
                let mut ready = row![space().width(Length::Fixed(LABEL_WIDTH))].spacing(theme::S1);
                for (rgb, name) in
                    [(RESIN_WHITE, "Milky white"), (RESIN_GREY, "Grey"), (RESIN_TAN, "Light tan")]
                {
                    ready = ready.push(crate::ui::tip(
                        iced::widget::button(
                            container(text(""))
                                .width(18)
                                .height(14)
                                .style(theme::swatch(colour_field::swatch(rgb))),
                        )
                        .padding(2)
                        .style(theme::tool_button)
                        .on_press(send(LightingMessage::Colour(Tint::Resin, rgb))),
                        name,
                    ));
                }
                block = block.push(ready);
            }
            Unpainted::Clay | Unpainted::GreenStuff | Unpainted::Pewter => {}
        }
        block = block.push(crate::ui::tip(
            crate::ui::button_secondary("Reset", Some(send(LightingMessage::Reset))),
            "Back to Studio's lights and the clay",
        ));
        block.into()
    }

    /// One light's controls.
    fn lamp_controls(&self, at: usize, lamp: &Lamp) -> Element<'_, Message> {
        let send = Message::Lighting;
        let switches = row![
            checkbox(lamp.on)
                .label("On")
                .on_toggle(move |on| send(LightingMessage::On(at, on)))
                .text_size(theme::LABEL_SIZE),
            crate::ui::tip(
                checkbox(lamp.with_camera)
                    .label("With the camera")
                    .on_toggle(move |with| send(LightingMessage::WithCamera(at, with)))
                    .text_size(theme::LABEL_SIZE),
                "Off: fixed in place, so turning the view moves its shine",
            ),
            space().width(Length::Fill),
            crate::ui::tip(
                crate::ui::button_secondary("Remove", Some(send(LightingMessage::Removed(at)))),
                "This light only; the others stay",
            ),
        ]
        .spacing(theme::S3)
        .align_y(Alignment::Center);
        let number = |which: LampNumber| {
            let value = lamp.number(which);
            let shown = match which {
                LampNumber::Round | LampNumber::Up => format!("{value:.0}°"),
                _ => format!("{value:.2}"),
            };
            crate::ui::number(&self.drafts, which.key(), which.label(), value, which.range(), {
                move |value| Message::Lighting(LightingMessage::Lamp(at, which, value))
            })
            .step(if matches!(which, LampNumber::Round | LampNumber::Up) { 1.0 } else { 0.01 })
            .shown(shown)
            .tip(which.tip())
        };
        let mut controls = column![
            switches,
            number(LampNumber::Round),
            number(LampNumber::Up),
            number(LampNumber::Strength),
            self.colour_line("Colour", Tint::Light(at)),
            crate::ui::disclosure(
                "Shape",
                self.lighting.shaped,
                send(LightingMessage::Shaped(!self.lighting.shaped))
            ),
        ]
        .spacing(theme::S2);
        if self.lighting.shaped {
            controls = controls
                .push(number(LampNumber::Wrap))
                .push(number(LampNumber::Falloff))
                .push(number(LampNumber::Highlight));
        }
        container(controls).padding(iced::Padding { left: theme::S3, ..iced::Padding::ZERO }).into()
    }

    /// A colour's line: its label, the swatch and the hex field, and its
    /// channels under them while it is the one open.
    fn colour_line(&self, label: &'static str, tint: Tint) -> Element<'_, Message> {
        let state = &self.lighting;
        let rgb = state.setting.tint(tint).unwrap_or([0; 3]);
        let typed = match &state.typed {
            Some((typing, typed)) if *typing == tint => typed.clone(),
            _ => colour_field::hex_of(rgb),
        };
        let open = state.open == Some(tint);
        let line = row![
            text(label).size(theme::LABEL_SIZE).color(theme::TEXT_DIM).width(LABEL_WIDTH),
            colour_field::hex_row(
                rgb,
                typed,
                move |typed| Message::Lighting(LightingMessage::Hex(tint, typed)),
                Some((
                    Message::Lighting(LightingMessage::Opened((!open).then_some(tint))),
                    "Red, green and blue",
                )),
            ),
        ]
        .spacing(theme::S2)
        .align_y(Alignment::Center);
        if !open {
            return line.into();
        }
        column![
            line,
            colour_field::channels(&self.drafts, CHANNEL_KEYS, rgb, move |channel, level| {
                Message::Lighting(LightingMessage::Channel(tint, channel, level))
            }),
        ]
        .spacing(theme::S2)
        .into()
    }
}

/// The open colour's channels' draft keys. One colour is open at a time.
const CHANNEL_KEYS: [&str; 3] = ["lighting.red", "lighting.green", "lighting.blue"];

/// The width of a colour's label, so the swatches line up.
const LABEL_WIDTH: f32 = 56.0;

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn light(app: &mut Hani, message: LightingMessage) {
        update(app, Message::Lighting(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// What the renderer was handed.
    fn handed(app: &Hani) -> LightBlock {
        app.shared.lights()
    }

    /// Two rigs the same, a light's direction to within rounding.
    fn same_rig(a: &Rig, b: &Rig) -> bool {
        a.lights.len() == b.lights.len()
            && a.global == b.global
            && a.sky == b.sky
            && a.ground == b.ground
            && a.lights.iter().zip(&b.lights).all(|(a, b)| {
                (a.direction.normalize() - b.direction.normalize()).length() < 1.0e-5
                    && (a.colour, a.strength, a.with_camera, a.wrap, a.falloff, a.highlight)
                        == (b.colour, b.strength, b.with_camera, b.wrap, b.falloff, b.highlight)
            })
    }

    /// Each preset gives the rig it says, and the renderer is handed it;
    /// the start is Studio, which is the default rig, and the clay.
    #[test]
    fn each_preset_gives_the_rig_it_says() {
        let mut app = app();
        assert_eq!(app.lighting.setting.preset, Some(Preset::Studio));
        assert!(same_rig(&app.lighting.setting.rig(), &Rig::default()));
        assert_eq!(handed(&app), LightBlock::default());
        for preset in [Preset::Lightbox, Preset::Overhead, Preset::Sky, Preset::Studio] {
            light(&mut app, LightingMessage::Preset(preset));
            let setting = &app.lighting.setting;
            assert_eq!(setting.preset, Some(preset));
            assert!(same_rig(&setting.rig(), &preset.rig()), "{preset:?}");
            let wanted = LightBlock::new(&preset.rig(), Material::Clay);
            let got = handed(&app);
            for at in 0..MAX_LIGHTS {
                for c in 0..4 {
                    assert!((got.direction[at][c] - wanted.direction[at][c]).abs() < 1.0e-5);
                }
            }
            assert_eq!(
                (got.colour, got.sky, got.ground),
                (wanted.colour, wanted.sky, wanted.ground)
            );
        }
        assert!(app.lighting.setting.rig().lights.iter().all(|light| light.with_camera));
        light(&mut app, LightingMessage::Preset(Preset::Lightbox));
        assert!(app.lighting.setting.lamps.iter().all(|lamp| !lamp.with_camera));
        light(&mut app, LightingMessage::Preset(Preset::Sky));
        assert!(app.lighting.setting.lamps.is_empty());

        // Changing a number after makes it the user's own; the material
        // does not.
        light(&mut app, LightingMessage::Preset(Preset::Overhead));
        light(&mut app, LightingMessage::Material(Unpainted::Pewter));
        assert_eq!(app.lighting.setting.preset, Some(Preset::Overhead));
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Strength, 0.7));
        assert_eq!(app.lighting.setting.preset, None);
    }

    /// A light's two angles give the direction they should, with the
    /// camera and fixed alike: straight up, level and to the left, and 0
    /// and 360 degrees round are one light.
    #[test]
    fn a_lights_angles_give_its_direction() {
        let mut app = app();
        let toward = |app: &Hani| {
            let d = handed(app).direction[0];
            Vec3::new(d[0], d[1], d[2])
        };
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Up, 90.0));
        assert!((toward(&app) - Vec3::Y).length() < 1.0e-5, "{:?}", toward(&app));
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Up, 0.0));
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Round, 270.0));
        assert!((toward(&app) - Vec3::NEG_X).length() < 1.0e-5, "{:?}", toward(&app));
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Round, 0.0));
        let front = toward(&app);
        assert!((front - Vec3::Z).length() < 1.0e-5);
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Round, 360.0));
        assert!((toward(&app) - front).length() < 1.0e-5);
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Round, 90.0));
        assert!((toward(&app) - Vec3::X).length() < 1.0e-5, "90 is not to the right");
    }

    /// "With the camera" reaches the rig's flag and the block's.
    #[test]
    fn with_the_camera_reaches_the_rig() {
        let mut app = app();
        assert!(app.lighting.setting.rig().lights[0].with_camera);
        light(&mut app, LightingMessage::WithCamera(0, false));
        assert!(!app.lighting.setting.rig().lights[0].with_camera);
        assert_eq!(handed(&app).direction[0][3], 1.0, "the shader was not told it is fixed");
        light(&mut app, LightingMessage::WithCamera(0, true));
        assert_eq!(handed(&app).direction[0][3], 0.0);
        // Off is no light, kept in its place.
        light(&mut app, LightingMessage::On(0, false));
        assert_eq!(handed(&app).colour[0][..3], [0.0; 3]);
        assert_eq!(app.lighting.setting.lamps.len(), 2);
    }

    /// Lights are added to four and no more, and removed.
    #[test]
    fn a_fifth_light_is_refused() {
        let mut app = app();
        assert_eq!(app.lighting.setting.lamps.len(), 2);
        light(&mut app, LightingMessage::Added);
        light(&mut app, LightingMessage::Added);
        assert_eq!(app.lighting.setting.lamps.len(), 4);
        assert_eq!(app.lighting.chosen, 3);
        light(&mut app, LightingMessage::Added);
        assert_eq!(app.lighting.setting.lamps.len(), 4);
        assert_eq!(app.status, "4 lights at most");
        light(&mut app, LightingMessage::Removed(3));
        light(&mut app, LightingMessage::Removed(0));
        assert_eq!(app.lighting.setting.lamps.len(), 2);
        assert!(app.lighting.chosen < 2);
        light(&mut app, LightingMessage::Removed(9));
        assert_eq!(app.lighting.setting.lamps.len(), 2);
    }

    /// Strengths and colours are held to their ranges, and a value that is
    /// not a number changes nothing; nor does a hex that is not one.
    #[test]
    fn values_are_held_to_their_ranges() {
        let mut app = app();
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Strength, 99.0));
        assert_eq!(app.lighting.setting.lamps[0].strength, 3.0);
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Up, -400.0));
        assert_eq!(app.lighting.setting.lamps[0].up, -90.0);
        light(&mut app, LightingMessage::Global(-1.0));
        assert_eq!(app.lighting.setting.global, 0.0);
        let before = app.lighting.setting.clone();
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Strength, f32::NAN));
        light(&mut app, LightingMessage::Lamp(1, LampNumber::Round, f32::INFINITY));
        light(&mut app, LightingMessage::Global(f32::NAN));
        light(&mut app, LightingMessage::Hex(Tint::Sky, "blue".to_string()));
        light(&mut app, LightingMessage::Channel(Tint::Sky, 7, 10));
        light(&mut app, LightingMessage::Lamp(7, LampNumber::Strength, 1.0));
        assert_eq!(app.lighting.setting, before);
        // A typed field that is not a number is dropped by the field
        // (`ui::Number::submitted`), and the light is as it was.
        app.drafts.insert(LampNumber::Strength.key(), "bright".to_string());
        let submitted = crate::ui::number(
            &app.drafts,
            LampNumber::Strength.key(),
            "Strength",
            1.0,
            LampNumber::Strength.range(),
            |value| Message::Lighting(LightingMessage::Lamp(0, LampNumber::Strength, value)),
        )
        .submitted();
        update(&mut app, submitted);
        assert_eq!(app.lighting.setting, before);
        // A colour channel, and a hex.
        light(&mut app, LightingMessage::Channel(Tint::Light(0), 0, 255));
        light(&mut app, LightingMessage::Channel(Tint::Light(0), 1, 0));
        light(&mut app, LightingMessage::Channel(Tint::Light(0), 2, 0));
        assert_eq!(app.lighting.setting.lamps[0].colour, [1.0, 0.0, 0.0]);
        light(&mut app, LightingMessage::Hex(Tint::Ground, "#808080".to_string()));
        assert_eq!(app.lighting.setting.tint(Tint::Ground), Some([128, 128, 128]));
    }

    /// Each material reaches the renderer, and Polystyrene's colour and
    /// Resin's are each their own.
    #[test]
    fn each_material_reaches_the_renderer() {
        let mut app = app();
        for (unpainted, material) in [
            (Unpainted::Polystyrene, Material::Polystyrene(POLYSTYRENE_GREY)),
            (Unpainted::GreenStuff, Material::GreenStuff),
            (Unpainted::Pewter, Material::Pewter),
            (Unpainted::Resin, Material::Resin(RESIN_WHITE)),
            (Unpainted::Clay, Material::Clay),
        ] {
            light(&mut app, LightingMessage::Material(unpainted));
            assert_eq!(handed(&app).surface(), material.surface(), "{unpainted}");
        }
        light(&mut app, LightingMessage::Material(Unpainted::Polystyrene));
        light(&mut app, LightingMessage::Colour(Tint::Polystyrene, [200, 30, 30]));
        assert_eq!(handed(&app).surface(), Material::Polystyrene([200, 30, 30]).surface());
        light(&mut app, LightingMessage::Material(Unpainted::Resin));
        assert_eq!(app.lighting.setting.resin, RESIN_WHITE, "resin took polystyrene's colour");
        assert_eq!(handed(&app).surface(), Material::Resin(RESIN_WHITE).surface());
        light(&mut app, LightingMessage::Colour(Tint::Resin, RESIN_TAN));
        assert_eq!(app.lighting.setting.polystyrene, [200, 30, 30]);
        assert_eq!(handed(&app).surface(), Material::Resin(RESIN_TAN).surface());
    }

    /// What is kept is read back the same, through a scratch directory and
    /// never the user's: a preset by its name, and a rig of the user's own
    /// value by value.
    #[test]
    fn what_is_kept_is_read_back_the_same() {
        let scratch = std::env::temp_dir().join(format!("hani-lighting-{}", std::process::id()));
        let path = scratch.join(FILE);
        let mut app = app();
        light(&mut app, LightingMessage::Preset(Preset::Lightbox));
        light(&mut app, LightingMessage::Material(Unpainted::Resin));
        light(&mut app, LightingMessage::Colour(Tint::Resin, RESIN_GREY));
        light(&mut app, LightingMessage::Colour(Tint::Polystyrene, [10, 20, 30]));
        app.lighting.setting.keep_in(Some(&path)).expect("kept");
        let back = Setting::read_from(Some(&path));
        assert_eq!(back.preset, Some(Preset::Lightbox));
        assert_eq!(back, app.lighting.setting);

        light(&mut app, LightingMessage::Lamp(1, LampNumber::Round, 123.25));
        light(&mut app, LightingMessage::On(2, false));
        light(&mut app, LightingMessage::WithCamera(0, true));
        light(&mut app, LightingMessage::Lamp(0, LampNumber::Falloff, 2.5));
        light(&mut app, LightingMessage::Hex(Tint::Sky, "102030".to_string()));
        light(&mut app, LightingMessage::Global(0.42));
        assert_eq!(app.lighting.setting.preset, None);
        app.lighting.setting.keep_in(Some(&path)).expect("kept");
        assert_eq!(Setting::read_from(Some(&path)), app.lighting.setting);
        light(&mut app, LightingMessage::Preset(Preset::Sky));
        app.lighting.setting.keep_in(Some(&path)).expect("kept");
        assert_eq!(Setting::read_from(Some(&path)), app.lighting.setting);
        std::fs::remove_dir_all(&scratch).ok();

        assert_eq!(Setting::read_from(Some(&scratch.join("not-there"))), Setting::default());
        assert_eq!(Setting::read_from(None), Setting::default());
        assert_eq!(Setting::default().keep_in(None), Ok(()));
    }

    /// A kept file of nonsense gives the defaults for what cannot be read,
    /// and what can be read is read.
    #[test]
    fn a_file_of_nonsense_gives_the_defaults() {
        assert_eq!(Setting::from_text(""), Setting::default());
        assert_eq!(Setting::from_text("\u{0}\u{1}garbage\n= =\n=="), Setting::default());
        let five = "preset = own\nlights = 5\nlight1.strength = 1\nmaterial = pewter\n";
        let read = Setting::from_text(five);
        assert_eq!(read.lamps, Setting::default().lamps, "five lights were read");
        assert_eq!(read.material, Unpainted::Pewter);
        let bad = "preset = banana\nmaterial = wood\npolystyrene = #GGGGGG\nresin = 12\n\
                   lights = 2\nglobal = lots\nsky = 1 2 3\nground = 0.5 0.5\n\
                   light1.round = 999\nlight1.up = NaN\nlight1.strength = 0.25\n\
                   light1.on = maybe\nlight1.colour = 0.1 0.2 x\nlight2.falloff = inf\n";
        let read = Setting::from_text(bad);
        let usual = Setting::default();
        let added = Lamp::added();
        assert_eq!(read.preset, None);
        assert_eq!(read.material, Unpainted::Clay);
        assert_eq!((read.polystyrene, read.resin), (usual.polystyrene, usual.resin));
        assert_eq!((read.global, read.sky, read.ground), (usual.global, usual.sky, usual.ground));
        assert_eq!(read.lamps.len(), 2);
        assert_eq!(read.lamps[0], Lamp { strength: 0.25, ..added });
        assert_eq!(read.lamps[1], added);
    }

    /// Reset puts back Studio and the clay, the colours too.
    #[test]
    fn reset_restores_the_defaults() {
        let mut app = app();
        light(&mut app, LightingMessage::Preset(Preset::Overhead));
        light(&mut app, LightingMessage::Added);
        light(&mut app, LightingMessage::Material(Unpainted::Polystyrene));
        light(&mut app, LightingMessage::Colour(Tint::Polystyrene, [1, 2, 3]));
        light(&mut app, LightingMessage::Reset);
        assert_eq!(app.lighting.setting, Setting::default());
        // The default block, the lights' directions to within the rounding
        // of their two angles.
        let (got, wanted) = (handed(&app), LightBlock::default());
        for at in 0..MAX_LIGHTS {
            for c in 0..4 {
                assert!((got.direction[at][c] - wanted.direction[at][c]).abs() < 1.0e-5);
            }
        }
        assert_eq!(LightBlock { direction: wanted.direction, ..got }, wanted);
    }

    /// View > Lighting shows the block in the dock and shuts it; every
    /// state of the block draws, and the window with it.
    #[test]
    fn every_state_of_the_block_draws() {
        let mut app = app();
        assert!(!app.lighting_shown());
        light(&mut app, LightingMessage::Shown(true));
        assert!(app.lighting_shown());
        drop(app.view());
        for preset in Preset::ALL {
            light(&mut app, LightingMessage::Preset(preset));
            for material in Unpainted::ALL {
                light(&mut app, LightingMessage::Material(material));
                drop(app.lighting_block());
            }
        }
        light(&mut app, LightingMessage::Preset(Preset::Lightbox));
        light(&mut app, LightingMessage::Added);
        for at in 0..MAX_LIGHTS {
            light(&mut app, LightingMessage::Chosen(at));
            for shaped in [false, true] {
                light(&mut app, LightingMessage::Shaped(shaped));
                drop(app.lighting_block());
            }
        }
        for tint in [Tint::Light(1), Tint::Sky, Tint::Ground, Tint::Polystyrene, Tint::Resin] {
            light(&mut app, LightingMessage::Opened(Some(tint)));
            light(&mut app, LightingMessage::Hex(tint, "12".to_string()));
            drop(app.lighting_block());
        }
        for workspace in crate::message::Workspace::ALL {
            update(&mut app, Message::WorkspaceChosen(workspace));
            light(&mut app, LightingMessage::Shown(true));
            drop(app.view());
        }
        light(&mut app, LightingMessage::Shown(false));
        assert!(!app.lighting_shown());
    }
}
