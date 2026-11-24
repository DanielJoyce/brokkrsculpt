// SPDX-License-Identifier: AGPL-3.0-only

//! The brushes, where a user can reach them.
//!
//! `docs/STUDIO-PLAN.md` section 16: a brush is its settings.
//! `brokkr_core::brush::compound` is the one brush all of them are, and
//! this is the panel it is set up from:
//!
//! * **The palette.** The brushes that come with the application and the
//!   ones the user kept, a button each. Upstream's six are six of them.
//!   See [`super::presets`].
//! * **What a brush does**, as an amount of each of the five things a
//!   brush can do: push, inflate, pinch, smooth, level. A crease is a push
//!   in and a pinch, and is a brush on the palette with nothing written
//!   for it.
//! * **The profile**: the cross-section of the stroke, as one of five
//!   shapes and how much of its middle is flat.
//! * **A lazy stroke.** The brush trails the pointer on a short lead, so a
//!   hand's tremor is not in the line. The single most useful thing for a
//!   panel line, a seam, or lettering.
//! * **Size from pressure.** A pen pressed harder is a bigger brush, for a
//!   stroke that swells and tapers.
//! * **The stamp.** One image pressed into the surface with one drag: press
//!   where it goes, drag to size it and to turn it. A rivet, a scale, a
//!   rune, or any picture, read as heights.
//!
//! A child module of `app` and a file of its own for the reason `paints.rs`
//! is one: `docs/STUDIO-PLAN.md` decision D6, add and do not modify. What
//! `app.rs` holds for this is one field, one message arm, and a few changed
//! lines in the stroke and where a brush or a falloff curve is chosen.
//!
//! # A brush goes by a classic brush
//!
//! Upstream's brushes are an enum with a digit key each, a remembered
//! strength each and a button each on the tool strip, and a great deal of
//! `app.rs` asks that enum its questions: which strength is in hand,
//! whether the invert key does anything. So the brush in hand is still one
//! of upstream's ([`brokkr_core::Brush`], with its size, its strength, its
//! falloff and its pattern), and what is kept here is only what it does
//! that the classic brush of its kind does not: amounts of its own, a
//! profile no falloff curve has. A brush with amounts of its own goes by
//! the classic brush they are nearest to, their [`Actions::family`].
//!
//! Choosing a classic brush, by its key or its button, puts all of that
//! away: it is that brush on the palette. Holding shift still smooths.
//!
//! # A classic brush still runs upstream's kernel
//!
//! The generic brush does what a classic brush does to the bit
//! (`one_action_is_the_classic_brush_of_that_name`), and costs up to a
//! fifth more doing it. So a brush in hand that IS a classic brush, with
//! one of upstream's curves and no plateau, is stamped by upstream's own
//! code, called and not copied, as it was before there was a palette.
//! Anything else is the generic brush.

use std::sync::Arc;

use brokkr_core::brush::alpha::{Alpha, BuiltIn};
use brokkr_core::brush::compound::{Actions, Compound, Layer, MOST_PLATEAU, Profile, Shape, Side};
use brokkr_core::brush::stamp::{Placement, StampError, StampGesture};
use brokkr_core::{Brush, BrushDirection, FalloffCurve, Stamp};
use glam::{Vec2, Vec3};
use iced::widget::{button, column, container, row, slider, text, text_input};
use iced::{Element, Length, Task};

use super::Brokkr;
use super::presets::{Place, Preset, Presets, StampOf};
use crate::message::{LoadedStamp, Message, StudioMessage};
use crate::theme;

/// How tall a stamp is, as a share of half its width. At 1 a dome is a
/// hemisphere.
pub(super) const HEIGHT_RANGE: std::ops::RangeInclusive<f32> = 0.02..=1.0;

/// How far behind the pointer a lazy stroke may trail, in pixels of the
/// viewport. Eighty is a long lead: at that the brush is being towed.
pub(super) const LAZY_RANGE: std::ops::RangeInclusive<f32> = 0.0..=80.0;

/// How far apart the stamps of a row may be, centre to centre, in stamp
/// widths. Half a width is stamps overlapping by half, which is a rope or
/// a row of scales; past four the row is not read as a row.
pub(super) const ALONG_RANGE: std::ops::RangeInclusive<f32> = 0.5..=4.0;

/// How far apart a row's stamps are when the row is first asked for: with
/// a gap of half a stamp between them, which reads as a row of separate
/// things.
const DEFAULT_ALONG: f32 = 1.5;

/// How tall a brush's picture is drawn on its button, in pixels.
const SWATCH_HEIGHT: f32 = 24.0;

/// How wide the palette at the pointer is, and roughly how tall its
/// heading and each of its rows come out: only to keep it on the screen,
/// so being a little out costs nothing.
const POPUP_WIDTH: f32 = 450.0;

/// How many brushes there are to a row of the palette at the pointer.
/// Five, where the panel has room for three: at three, the brushes that
/// come with the application were taller than a small window.
const POPUP_ACROSS: usize = 5;
const POPUP_HEAD: f32 = 44.0;
const POPUP_ROW: f32 = 25.0 + SWATCH_HEIGHT;
const POPUP_GROUP: f32 = 18.0;

/// The most steps taken along the pointer's ray to find the surface as it
/// was before a stroke. See [`Brokkr::layer_aim`].
const AIM_STEPS: usize = 48;

/// How many times the pointer's line is halved to find where a step ends.
/// Twelve is a four thousandth of the line: under a hundredth of a
/// millimetre for any drag a viewport has room for.
const HALVINGS: usize = 12;

/// The most stamps one pointer event lays along a stroke. A pointer that
/// jumps the width of the model in one event is a row that long, and no
/// longer than this.
const MOST_A_MOVE: usize = 64;

/// How much of the brush's size pressure decides, 0 for none of it.
pub(super) const SIZE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;

/// The least a brush shrinks to under the lightest touch, as a share of its
/// size. Not nothing: a stroke that begins with no brush at all begins with
/// a gap, and the first stamps of a pen stroke are always its lightest.
const SMALLEST_SHARE: f32 = 0.1;

/// How much of the middle of a stroke may be flat. See [`MOST_PLATEAU`].
pub(super) const PLATEAU_RANGE: std::ops::RangeInclusive<f32> = 0.0..=MOST_PLATEAU;

/// What the application remembers for its brushes, past what upstream's
/// own brush holds.
#[derive(Debug)]
pub(super) struct StudioState {
    /// What the brush in hand does, where that is not what the classic
    /// brush it goes by does. Always as [`super::presets::held`] gives it,
    /// so that the same brush is the same value.
    pub custom: Option<Actions>,
    /// The stroke has the cross-section of a ball, in place of the classic
    /// brush's falloff curve.
    pub round: bool,
    /// How much of the middle of the stroke is flat.
    pub plateau: f32,
    /// Whether the panel shows what the brush is made of.
    pub open: bool,
    /// The picture in hand is laid across the tip of the brush.
    pub tip_on: bool,
    /// Where the palette is up at the pointer, in pixels of the viewport,
    /// or nothing when it is put away.
    pub popup: Option<Vec2>,
    /// Every stamp works on what the last one left, as a brush always
    /// has. Off, a stroke lays one layer.
    pub build_up: bool,
    /// What the stroke in progress has given each voxel, for a stroke
    /// that lays one layer.
    layer: Layer,
    /// How far apart the brush is set down along a stroke, in its own
    /// widths, or nothing for a line.
    pub dabs: f32,
    /// How far the brush trails the pointer, in pixels. Zero is off.
    pub lazy_px: f32,
    /// How much of the brush's size pressure decides.
    pub size_from_pressure: f32,
    /// Where the lazy brush is, in pixels of the viewport.
    anchor: Vec2,
    /// Whether a drag on the model presses the stamp in.
    pub stamp_on: bool,
    /// The stamp image, and what to call it.
    pub alpha: Arc<Alpha>,
    pub alpha_name: String,
    /// The built-in the image is, or `None` for one read from a file.
    pub built_in: Option<BuiltIn>,
    /// How tall the stamp is, as a share of half its width.
    pub stamp_height: f32,
    /// The stamp being placed, and where the press that began it landed:
    /// the point on the surface and the surface's normal there.
    gesture: StampGesture,
    press: Option<(Vec3, Vec3)>,
    /// How far apart stamps are laid along a drag, in stamp widths, or
    /// nothing for one stamp sized by the drag.
    pub stamp_along: f32,
    /// Where the next stamp of a row is owed: a point on the surface, and
    /// the pixel it was under.
    trail: Option<(Vec3, Vec2)>,
    /// The brushes the user kept, and the name being typed for the next.
    /// See [`super::presets`].
    pub presets: Presets,
    pub preset_name: String,
}

impl Default for StudioState {
    fn default() -> Self {
        Self {
            custom: None,
            round: false,
            plateau: 0.0,
            open: false,
            tip_on: false,
            popup: None,
            build_up: true,
            layer: Layer::new(),
            dabs: 0.0,
            lazy_px: 0.0,
            size_from_pressure: 0.0,
            anchor: Vec2::ZERO,
            stamp_on: false,
            alpha: Arc::new(Alpha::built_in(BuiltIn::Rivet)),
            alpha_name: BuiltIn::Rivet.label().to_string(),
            built_in: Some(BuiltIn::Rivet),
            stamp_height: 0.35,
            gesture: StampGesture::new(),
            press: None,
            stamp_along: 0.0,
            trail: None,
            presets: Presets::default(),
            preset_name: String::new(),
        }
    }
}

impl StudioState {
    /// Where the brush is when the pointer is at `pixel`.
    ///
    /// The brush is on a lead of `lazy_px`. While the pointer is within the
    /// lead's length of it, the brush stays where it is; past that it is
    /// pulled straight toward the pointer and stops a lead's length short.
    /// So a wobble smaller than the lead moves nothing, and a deliberate
    /// stroke is followed along a line smoother than the hand's.
    pub(super) fn follow(&mut self, pixel: Vec2, start: bool) -> Vec2 {
        if start {
            // A new stroke, and for one that lays a layer, a new layer:
            // one line of it, unless the brush is set down in dabs.
            self.layer.begin(self.dabs <= 0.0);
        }
        if start || self.lazy_px <= 0.0 {
            self.anchor = pixel;
            return pixel;
        }
        let lead = pixel - self.anchor;
        let length = lead.length();
        if length > self.lazy_px {
            self.anchor += lead * ((length - self.lazy_px) / length);
        }
        self.anchor
    }

    /// The brush with its radius set by how hard the pen is pressed.
    ///
    /// A mouse reports full pressure and gets the brush as it is.
    pub(super) fn sized(&self, brush: Brush, pressure: f32) -> Brush {
        let lightness = 1.0 - pressure.clamp(0.0, 1.0);
        let share = (1.0 - self.size_from_pressure * lightness).max(SMALLEST_SHARE);
        Brush { radius: brush.radius * share, ..brush }
    }

    /// Whether a drag on the model is a stamp and not a stroke.
    pub(super) fn stamping(&self) -> bool {
        self.stamp_on
    }

    /// Back to the classic brush: no amounts of its own, no profile a
    /// falloff curve does not have, no stamp.
    pub(super) fn put_away(&mut self) {
        self.custom = None;
        self.round = false;
        self.plateau = 0.0;
        self.tip_on = false;
        self.build_up = true;
        self.dabs = 0.0;
        self.stamp_on = false;
    }

    /// One of the classic brush's falloff curves was chosen, which is not
    /// the ball's cross-section.
    pub(super) fn falloff_chosen(&mut self) {
        self.round = false;
    }

    /// The cross-section of the stroke: the ball's or the classic brush's
    /// falloff curve, with the flat middle.
    pub(super) fn profile(&self, falloff: FalloffCurve) -> Profile {
        let shape = if self.round { Shape::Round } else { Shape::of(falloff) };
        Profile { shape, plateau: self.plateau }
    }

    /// Apply a change, and say what the status line should, if anything.
    fn set(&mut self, message: StudioMessage) -> Option<String> {
        match message {
            StudioMessage::RoundChosen => self.round = true,
            StudioMessage::PlateauChanged(share) => self.plateau = clamped(share, PLATEAU_RANGE),
            StudioMessage::SettingsShown(shown) => self.open = shown,
            StudioMessage::BuildUpToggled(on) => self.build_up = on,
            StudioMessage::DabsChanged(widths) => {
                // Not a number is not more than nothing, and is a line.
                self.dabs = if widths > 0.0 { clamped(widths, ALONG_RANGE) } else { 0.0 };
            }
            StudioMessage::TipToggled(on) => {
                self.tip_on = on;
                if on {
                    // The stamp is the picture pressed in whole, and is
                    // not a brush that has a tip.
                    self.stamp_on = false;
                }
            }
            StudioMessage::LazyChanged(pixels) => self.lazy_px = clamped(pixels, LAZY_RANGE),
            StudioMessage::SizeFromPressureChanged(share) => {
                self.size_from_pressure = clamped(share, SIZE_RANGE);
            }
            StudioMessage::StampToggled(on) => {
                self.stamp_on = on;
                if on {
                    self.custom = None;
                    self.tip_on = false;
                    return Some(
                        "stamp: press where it goes, drag to size and turn it".to_string(),
                    );
                }
            }
            StudioMessage::StampChosen(kind) => {
                self.alpha = Arc::new(Alpha::built_in(kind));
                self.alpha_name = kind.label().to_string();
                self.built_in = Some(kind);
            }
            StudioMessage::StampHeightChanged(share) => {
                self.stamp_height = clamped(share, HEIGHT_RANGE);
            }
            StudioMessage::StampAlongChanged(widths) => {
                // Not a number is not more than nothing, and is off.
                self.stamp_along = if widths > 0.0 { clamped(widths, ALONG_RANGE) } else { 0.0 };
            }
            // The dialog is opened by `Brokkr::studio_changed`, which is the
            // one that can return a task.
            StudioMessage::StampImageRequested | StudioMessage::StampImageLoaded(None) => {}
            StudioMessage::StampImageLoaded(Some(Ok(loaded))) => {
                self.alpha = loaded.alpha;
                self.alpha_name = loaded.name;
                self.built_in = None;
                return Some(format!("stamp image: {}", self.alpha_name));
            }
            StudioMessage::StampImageLoaded(Some(Err(why))) => {
                return Some(format!("that picture cannot be a stamp: {why}"));
            }
            // Kept as typed. It is made a name the file can hold when the
            // preset is saved, so that a space can be typed at all.
            StudioMessage::PresetNamed(name) => {
                self.preset_name = name.chars().take(4 * super::presets::LONGEST_NAME).collect();
            }
            // These change the classic brush too, or want the pointer or
            // a file, none of which is this struct's:
            // `Brokkr::studio_changed` has them.
            StudioMessage::PaletteToggled
            | StudioMessage::BrushImportRequested
            | StudioMessage::BrushImported(_)
            | StudioMessage::BrushesReloaded
            | StudioMessage::ActionsChosen(_)
            | StudioMessage::PresetChosen(_)
            | StudioMessage::PresetSaved
            | StudioMessage::PresetRemoved(_) => {}
        }
        None
    }
}

/// Read a picture as a stamp image: its brightness is its height.
///
/// Sixteen bits of grey where the file has them, because eight is 256 steps
/// and a stamp a centimetre tall would have terraces 40 microns high, which
/// a resin printer prints.
pub(super) fn read_stamp_image(path: &std::path::Path) -> Result<LoadedStamp, String> {
    let picture = image::open(path).map_err(|error| error.to_string())?.into_luma16();
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    let heights: Vec<f32> =
        picture.as_raw().iter().map(|level| f32::from(*level) / f32::from(u16::MAX)).collect();
    let alpha = Alpha::from_heights(width, height, &heights).map_err(|error| error.to_string())?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "picture".to_string());
    Ok(LoadedStamp { name, alpha: Arc::new(alpha) })
}

/// Ask for a picture and read it. `None` when the dialog is dismissed.
async fn pick_stamp_image() -> Option<Result<LoadedStamp, String>> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Stamp image")
        .add_filter("Picture", &["png", "jpg", "jpeg"])
        .pick_file()
        .await?;
    Some(read_stamp_image(handle.path()))
}

/// Ask for brush files and read them: each one's name and text, or why it
/// could not be read. `None` when the dialog is dismissed.
async fn pick_brush_files() -> Option<Vec<Result<(String, String), String>>> {
    let handles = rfd::AsyncFileDialog::new()
        .set_title("Import brushes")
        .add_filter("Brush", &["brush", "conf"])
        .pick_files()
        .await?;
    Some(handles.iter().map(|handle| read_brush_file(handle.path())).collect())
}

/// A brush file's name and its text.
pub(super) fn read_brush_file(path: &std::path::Path) -> Result<(String, String), String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a file".to_string());
    // A brush is a line of text. A file of megabytes is not a brush, and
    // reading one because it was picked by mistake helps nobody.
    let size = std::fs::metadata(path).map_err(|why| format!("{name}: {why}"))?.len();
    if size > MOST_BRUSH_FILE {
        return Err(format!("{name} is not a brush file: it is {size} bytes"));
    }
    match std::fs::read_to_string(path) {
        Ok(text) => Ok((name, text)),
        Err(why) => Err(format!("{name}: {why}")),
    }
}

/// The biggest file read as a brush, in bytes. A pack of every brush there
/// is room for is a few kilobytes.
const MOST_BRUSH_FILE: u64 = 256 * 1024;

/// Into a slider's range, with a NaN sent to the bottom of it.
fn clamped(value: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if value.is_nan() { *range.start() } else { value.clamp(*range.start(), *range.end()) }
}

impl Brokkr {
    pub(super) fn studio_changed(&mut self, message: StudioMessage) -> Task<Message> {
        if message == StudioMessage::StampImageRequested {
            return Task::perform(pick_stamp_image(), |loaded| {
                Message::Studio(StudioMessage::StampImageLoaded(loaded))
            });
        }
        // A brush's amounts changed, or the stamp taken up: whatever was
        // in hand to add is put away. See `app/adding.rs`. (A brush taken
        // up from the palette does the same, once it is known to be
        // there: `Brokkr::preset_changed`.)
        if matches!(message, StudioMessage::ActionsChosen(_) | StudioMessage::StampToggled(true)) {
            self.inserts.put_away();
        }
        match message {
            StudioMessage::ActionsChosen(actions) => {
                self.actions_changed(actions);
                return Task::none();
            }
            // Up at the pointer, or in the middle of the view when the
            // pointer has yet to be over it; down if it is up.
            StudioMessage::PaletteToggled => {
                // A pointer that is off the viewport is on the button that
                // asked, over in the panel.
                let size = self.viewport_size;
                let over_the_view = self
                    .cursor
                    .filter(|at| (0.0..=size.x).contains(&at.x) && (0.0..=size.y).contains(&at.y));
                self.studio.popup = match self.studio.popup {
                    Some(_) => None,
                    None => Some(over_the_view.unwrap_or(size / 2.0)),
                };
                return Task::none();
            }
            StudioMessage::BrushImportRequested => {
                return Task::perform(pick_brush_files(), |read| {
                    Message::Studio(StudioMessage::BrushImported(read))
                });
            }
            StudioMessage::BrushImported(read) => {
                self.brushes_imported(read);
                return Task::none();
            }
            StudioMessage::BrushesReloaded => {
                self.studio.presets = Presets::load();
                // And the maps beside them, whose files may have been
                // changed by hand too: `app/maps.rs`.
                super::maps::forget_kept();
                self.inserts.maps = super::maps::kept(super::maps::directory().as_deref());
                self.status = format!(
                    "brushes read again: {} of your own{}",
                    self.studio.presets.list.len(),
                    Presets::place().map(|place| format!(", from {place}")).unwrap_or_default()
                );
                return Task::none();
            }
            _ => {}
        }
        if matches!(
            message,
            StudioMessage::PresetChosen(_)
                | StudioMessage::PresetSaved
                | StudioMessage::PresetRemoved(_)
        ) {
            self.preset_changed(message);
            return Task::none();
        }
        if let Some(status) = self.studio.set(message) {
            self.status = status;
        }
        Task::none()
    }

    /// Take in the brushes of the files that were picked: each is kept as
    /// if it had been kept here, in a file of its own, in place of one of
    /// the same name.
    fn brushes_imported(&mut self, read: Option<Vec<Result<(String, String), String>>>) {
        let Some(read) = read else {
            return;
        };
        let (mut taken, mut refused): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        for file in read {
            let (name, text) = match file {
                Ok(read) => read,
                Err(why) => {
                    refused.push(why);
                    continue;
                }
            };
            let brushes = Presets::parse(&text).list;
            if brushes.is_empty() {
                refused.push(format!("{name} holds no brush"));
            }
            for brush in brushes {
                if !self.studio.presets.put(brush.clone()) {
                    refused.push(format!("no room for {}", brush.name));
                    continue;
                }
                if let Err(why) = Presets::write(&brush) {
                    refused.push(why);
                }
                taken.push(brush.name);
            }
        }
        self.status = match (taken.len(), refused.first()) {
            (0, Some(why)) => format!("no brush imported: {why}"),
            (0, None) => "no brush imported".to_string(),
            (count, None) => format!("imported {count}: {}", taken.join(", ")),
            (count, Some(why)) => format!("imported {count}: {}; but {why}", taken.join(", ")),
        };
    }

    /// Put the palette at the pointer away, and say whether it was up: a
    /// press or Escape that does so has done what it was for.
    pub(super) fn close_palette(&mut self) -> bool {
        self.studio.popup.take().is_some()
    }

    /// The brush's own strength, which with the mask tool up is not the
    /// one in hand: that one is the mask's.
    fn own_strength(&self) -> f32 {
        let slot = Self::strength_slot(self.brush.kind);
        if self.live_strength_slot() == slot { self.brush.strength } else { self.strengths[slot] }
    }

    /// Change what the brush in hand does.
    ///
    /// The amounts decide which classic brush it goes by, and that may be
    /// another than it went by a moment ago: a Draw with its push taken
    /// out and some smoothing put in is a brush of Smooth's family. It is
    /// still the brush that was in hand, so it keeps the strength it had
    /// and does not take up the one its new family was last used at.
    fn actions_changed(&mut self, actions: Actions) {
        let (kind, custom) = super::presets::held(actions);
        if kind != self.brush.kind {
            let strength = self.own_strength();
            self.swap_strength(|app| {
                app.brush.kind = kind;
                app.strengths[Self::strength_slot(kind)] = strength;
            });
        }
        self.studio.custom = custom;
        self.studio.stamp_on = false;
    }

    /// The brush as it is set up now, as a preset under `name`.
    fn preset_now(&self, name: String) -> Preset {
        let picture = self.studio.built_in.map_or(StampOf::InHand, StampOf::BuiltIn);
        let stamp = self.studio.stamp_on.then_some((picture, self.studio.stamp_height));
        Preset {
            name,
            kind: self.brush.kind,
            custom: self.studio.custom,
            profile: self.studio.profile(self.brush.falloff),
            size_mm: Some(self.brush.radius),
            strength: Some(self.own_strength()),
            stamp,
            along: if self.studio.stamp_on { self.studio.stamp_along } else { 0.0 },
            tip: self.studio.tip_on.then_some(picture),
            build_up: self.studio.build_up,
            dabs: self.studio.dabs,
            add: self.inserts.setup(),
            lazy_px: self.studio.lazy_px,
            size_from_pressure: self.studio.size_from_pressure,
        }
        .settled()
    }

    /// Which brush on the palette the brush in hand is, if it is one.
    ///
    /// More than one may answer: Draw says nothing of its size, and a
    /// brush kept as Draw at two millimetres is Draw too. The one that
    /// says the most is the one that is lit, and the earlier of two that
    /// say as much.
    fn lit(palette: &[(&Preset, Place)], now: &Preset) -> Option<usize> {
        let says = |preset: &Preset| {
            usize::from(preset.size_mm.is_some()) + usize::from(preset.strength.is_some())
        };
        palette
            .iter()
            .enumerate()
            .filter(|(_, (preset, _))| preset.is(now))
            .max_by_key(|(index, (preset, _))| (says(preset), std::cmp::Reverse(*index)))
            .map(|(index, _)| index)
    }

    /// Take up a brush from the palette, keep one, or forget one.
    fn preset_changed(&mut self, message: StudioMessage) {
        match message {
            StudioMessage::PresetChosen(index) => {
                let Some(preset) =
                    self.studio.presets.palette().get(index).map(|(preset, _)| (*preset).clone())
                else {
                    return;
                };
                // Whatever was in hand to add is put away. Here and not
                // before the brush was found: asked for a brush that is
                // not there, the one in hand stays in hand, which a test
                // of exactly that caught.
                self.inserts.put_away();
                // The brush and its strength as `BrushKindChanged` and
                // `BrushStrengthChanged` set them: the strength goes to the
                // brush's own slot, so that with the mask tool up it waits
                // there and does not become the mask's. A brush that says
                // nothing of its strength takes up what its family was
                // last used at, as the classic brush's own button does.
                self.swap_strength(|app| {
                    app.brush.kind = preset.kind;
                    if let Some(strength) = preset.strength {
                        app.strengths[Self::strength_slot(preset.kind)] = strength;
                    }
                });
                if let Some(curve) = preset.profile.shape.curve() {
                    self.brush.falloff = curve;
                }
                if let Some(size) = preset.size_mm {
                    self.brush.radius = size.clamp(super::MIN_RADIUS_MM, self.max_radius());
                }
                // The rest through the studio block's own setters, which
                // is what keeps "amounts of its own or the stamp, not
                // both" in one place.
                self.studio.put_away();
                self.studio.custom = preset.custom;
                self.studio.round = preset.profile.shape == Shape::Round;
                self.studio.set(StudioMessage::PlateauChanged(preset.profile.plateau));
                // The picture, for the stamp or for the tip: one of the
                // application's by name, or whichever is in hand.
                let picture = preset.stamp.map(|(picture, _)| picture).or(preset.tip);
                if let Some(StampOf::BuiltIn(kind)) = picture {
                    self.studio.set(StudioMessage::StampChosen(kind));
                }
                if let Some((_, height)) = preset.stamp {
                    self.studio.set(StudioMessage::StampHeightChanged(height));
                    self.studio.set(StudioMessage::StampToggled(true));
                }
                self.studio.set(StudioMessage::TipToggled(preset.tip.is_some()));
                self.studio.set(StudioMessage::BuildUpToggled(preset.build_up));
                self.studio.set(StudioMessage::DabsChanged(preset.dabs));
                self.studio.set(StudioMessage::StampAlongChanged(preset.along));
                self.studio.set(StudioMessage::LazyChanged(preset.lazy_px));
                self.studio.set(StudioMessage::SizeFromPressureChanged(preset.size_from_pressure));
                // A brush that adds is taken up last, over the plain brush
                // the rest of it is. See `app/adding.rs`.
                if let Some(setup) = preset.add {
                    self.take_up(setup);
                }
                self.status = format!("brush: {}", preset.name);
                // Choosing a brush is what the palette at the pointer was
                // up for.
                self.studio.popup = None;
            }
            StudioMessage::PresetSaved => {
                let Some(name) = super::presets::named(&self.studio.preset_name) else {
                    self.status = "give the brush a name first".to_string();
                    return;
                };
                let preset = self.preset_now(name.clone());
                if preset.add.is_some_and(|setup| setup.what == super::adding::Kind::Body) {
                    self.status = "a copy of a body cannot be kept as a brush: the body is this \
                                   document's"
                        .to_string();
                    return;
                }
                if !self.studio.presets.put(preset.clone()) {
                    self.status = "no room for another brush: forget one first".to_string();
                    return;
                }
                self.studio.preset_name.clear();
                self.status = match Presets::write(&preset) {
                    Ok(()) => format!("brush kept: {name}"),
                    Err(why) => format!("brush kept until the application closes: {why}"),
                };
            }
            StudioMessage::PresetRemoved(index) => {
                let palette = self.studio.presets.palette();
                let Some((name, place)) =
                    palette.get(index).map(|(preset, place)| (preset.name.clone(), *place))
                else {
                    return;
                };
                if place == Place::Core {
                    self.status = format!("{name} comes with the application, and stays");
                    return;
                }
                self.studio.presets.remove(&name);
                let done = Presets::erase(&name);
                self.status = match (place, done) {
                    (Place::Changed, Ok(())) => format!("{name} is back as it came"),
                    (_, Ok(())) => format!("brush forgotten: {name}"),
                    (_, Err(why)) => {
                        format!("brush forgotten until the application closes: {why}")
                    }
                };
            }
            _ => {}
        }
    }

    /// One pointer event of a stamp: the press fixes where it goes, and
    /// every move after that presses it again at the size and the turn the
    /// drag now says, from the surface as it was at the press.
    ///
    /// The size is how far the pointer has gone from the press, measured in
    /// the plane of the surface there and not on the screen, so a stamp on
    /// a face seen at a slant is not squashed. The top of the image points
    /// along the drag. The inverted direction engraves.
    pub(super) fn stamp_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        if self.studio.stamp_along > 0.0 {
            self.stamp_along(pixel, direction, start);
            return;
        }
        if start {
            self.studio.press = None;
            let Some(point) = self.surface_under(pixel) else {
                return;
            };
            let Some(normal) = self.doc.active_volume().gradient_world(point).try_normalize()
            else {
                return;
            };
            self.arm_recorder();
            self.studio.gesture.begin();
            self.studio.press = Some((point, normal));
            return;
        }
        let Some((centre, normal)) = self.studio.press else {
            return;
        };
        let (origin, ray) = self.ray_through(pixel);
        let facing = ray.dot(normal);
        if facing.abs() < 1.0e-3 {
            // The surface is edge on to the pointer: there is no telling
            // how far along it the drag has gone.
            return;
        }
        let drag = origin + ray * ((centre - origin).dot(normal) / facing) - centre;
        let radius = drag.length();
        let placement = Placement {
            centre,
            normal,
            up: drag,
            // A radius of nothing, which is a pointer back on the press,
            // is refused by the gesture, and the refusal puts back whatever
            // a longer drag had pressed.
            radius,
            height: radius * self.studio.stamp_height * direction.outward_sign(),
        };
        let studio = &mut self.studio;
        let pressed = studio.gesture.apply(
            self.doc.active_volume_mut(),
            &studio.alpha,
            &placement,
            self.symmetry,
            super::MIRROR_CENTRE,
        );
        if pressed == Err(StampError::TooBig) {
            self.status = "that stamp is too big to press: drag back in".to_string();
        }
        self.remesh_dirty();
    }

    /// One pointer event of a row of stamps: the image pressed in again and
    /// again along the drag, each at the brush's size, with its top
    /// pointing the way the row is going.
    ///
    /// A stamp is laid where the last one's step ended, once the pointer
    /// has gone a step on from there, because until then there is no
    /// telling which way it should face. So the first is at the press, the
    /// row follows the pointer a step behind, and a press with no drag
    /// lays nothing.
    ///
    /// **Where a step ends is looked for on the screen and not in space.**
    /// The straight line from a stamp toward the pointer cuts under a
    /// curved surface, by 3 mm for a 12 mm step on the starting ball, and
    /// the field cannot bring a point back from there: three voxels under
    /// the surface it is flat and has no gradient to follow. (That was
    /// built first, and a test of a long step found it.) What is always on
    /// the surface is whatever is under a pixel. So the next stamp's place
    /// is the point under the line the pointer drew, from the last stamp's
    /// pixel to where the pointer is now, that is one step from the last
    /// stamp, found by halving.
    ///
    /// Each stamp is a whole gesture of the core's, begun and ended here,
    /// so a row is as many stamps as it looks like and none is pressed
    /// twice. The undo entry is the drag's: one for the row.
    fn stamp_along(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        // A row is a stroke, and can be led like one.
        let pixel = self.studio.follow(pixel, start);
        if start {
            self.studio.trail = None;
            let Some(point) = self.surface_under(pixel) else {
                return;
            };
            self.arm_recorder();
            self.studio.trail = Some((point, pixel));
            return;
        }
        let (Some((mut anchor, mut under)), Some(pointer)) =
            (self.studio.trail, self.surface_under(pixel))
        else {
            return;
        };
        let radius = self.brush.radius;
        let step = radius * 2.0 * self.studio.stamp_along;
        let height = radius * self.studio.stamp_height * direction.outward_sign();
        let mut laid = false;
        for _ in 0..MOST_A_MOVE {
            if (pointer - anchor).length() < step {
                break;
            }
            // The furthest point along the pointer's line that is still
            // within a step of this stamp.
            let (mut near, mut far) = (0.0_f32, 1.0_f32);
            for _ in 0..HALVINGS {
                let middle = (near + far) / 2.0;
                match self.surface_under(under.lerp(pixel, middle)) {
                    Some(at) if (at - anchor).length() < step => near = middle,
                    _ => far = middle,
                }
            }
            let next_under = under.lerp(pixel, near);
            let next = self.surface_under(next_under).filter(|_| near > 0.0);
            let Some(next) = next else {
                // Nothing within a step along that line: a gap in the
                // model, or a drop behind an edge. The row takes up again
                // from under the pointer. NOT UNDER TEST: it wants a model
                // with a gap in it, and the fixtures here are a ball.
                (anchor, under) = (pointer, pixel);
                break;
            };
            let normal = self.doc.active_volume().gradient_world(anchor).try_normalize();
            if let Some(normal) = normal {
                let placement =
                    Placement { centre: anchor, normal, up: next - anchor, radius, height };
                let studio = &mut self.studio;
                studio.gesture.begin();
                let pressed = studio.gesture.apply(
                    self.doc.active_volume_mut(),
                    &studio.alpha,
                    &placement,
                    self.symmetry,
                    super::MIRROR_CENTRE,
                );
                studio.gesture.end();
                if pressed == Err(StampError::TooBig) {
                    self.status =
                        "that stamp is too big to press: make the brush smaller".to_string();
                    break;
                }
                laid |= pressed.is_ok();
            }
            (anchor, under) = (next, next_under);
        }
        self.studio.trail = Some((anchor, under));
        if laid {
            self.remesh_dirty();
        }
    }

    /// Whether the stroke the brush in hand makes lays one layer: a brush
    /// on the field with build-up off, and not Smooth under shift.
    fn lays_a_layer(&self) -> bool {
        let brush = self.effective_brush();
        !self.studio.build_up
            && !self.studio.stamping()
            && brush.kind == self.brush.kind
            && (self.studio.custom.is_some() || Actions::of(brush.kind).is_some())
    }

    /// Where a stamp under `pixel` is aimed, given where the pointer's ray
    /// meets the surface as it is now.
    ///
    /// There, for a brush that builds up. A stroke that lays one layer
    /// works from the surface as it was before the stroke
    /// (`brokkr_core::brush::compound::layer`), and is aimed at that
    /// surface: on along the same ray to where it meets it, or back along
    /// it for a stroke that cuts. The field as it was says how far off its
    /// surface a point is, up to three voxels, and a step of that much
    /// along the ray never passes through the surface.
    ///
    /// A ray that has not met it within two of the brush's radii was not
    /// going to: the pointer is over the layer's own edge, past where the
    /// model used to be. That stamp is left where the ray landed, and the
    /// core puts it back along its normal.
    pub(super) fn layer_aim(&self, pixel: Vec2, landed: Vec3) -> Vec3 {
        if !self.lays_a_layer() {
            return landed;
        }
        self.before_under(pixel, landed, self.brush.radius * 2.0).unwrap_or(landed)
    }

    /// Where the pointer's ray meets the surface as it was before the open
    /// stroke, given where it lands on the surface as it is now: on along
    /// the ray, or back along it, by no more than `furthest`. Nothing when
    /// it does not meet it within that.
    pub(super) fn before_under(&self, pixel: Vec2, landed: Vec3, furthest: f32) -> Option<Vec3> {
        let volume = self.doc.active_volume();
        let voxel_size = volume.voxel_size();
        let (_, ray) = self.ray_through(pixel);
        let mut at = landed;
        // Steps of up to three voxels: enough of them for the furthest it
        // may go and for the last short ones of a ray that grazes.
        for _ in 0..AIM_STEPS {
            let off = volume.sample_world_before(at);
            if off.abs() < 0.01 {
                return Some(at);
            }
            at += ray * (off * voxel_size);
            if at.distance(landed) >= furthest {
                break;
            }
        }
        None
    }

    /// Which way the surface faces at a stamp's centre: as it is now, or
    /// for a stroke that lays one layer, as it was before the stroke.
    pub(super) fn stamp_facing(&self, centre: Vec3) -> Vec3 {
        let volume = self.doc.active_volume();
        if self.lays_a_layer() {
            volume.gradient_world_before(centre)
        } else {
            volume.gradient_world(centre)
        }
    }

    /// How far apart the stamps of a stroke are laid: a quarter of the
    /// brush's radius, which reads as a line and is upstream's, or for a
    /// brush set down in dabs, that many of its widths. Shift is Smooth,
    /// which is a line whatever is in hand.
    pub(super) fn stroke_spacing(&self) -> f32 {
        let brush = self.effective_brush();
        if brush.kind == self.brush.kind && self.studio.dabs > 0.0 && !self.studio.stamping() {
            brush.radius * 2.0 * self.studio.dabs
        } else {
            brush.spacing(self.doc.voxel_size())
        }
    }

    /// What the brush in hand is called, for the panel's heading: its
    /// name on the palette when it is one of those, and otherwise the
    /// classic brush it goes by, marked as changed when it is not that
    /// brush either.
    pub(super) fn live_brush_name(&self) -> String {
        let now = self.preset_now(String::new());
        let palette = self.studio.presets.palette();
        if let Some(index) = Self::lit(&palette, &now) {
            return palette[index].0.name.to_uppercase();
        }
        // Something to add that is none of the palette's: a copy of a
        // body, or a shape with settings of its own.
        if let Some(adding) = self.adding_name() {
            return adding;
        }
        if self.studio.stamping() {
            return "STAMP".to_string();
        }
        // A brush on the field that is none of the palette's is not the
        // classic brush it goes by either, which is on the palette: it
        // has amounts of its own, or a profile, a tip or a lead that one
        // does not.
        let family = self.brush.kind.label().to_uppercase();
        if Actions::of(self.brush.kind).is_some() { format!("{family}, CHANGED") } else { family }
    }

    /// Lay one stamp of the field brush, and its mirrors.
    ///
    /// `brush` is the brush the stroke resolved, which is Smooth while
    /// shift is held and the brush in hand otherwise. The brush in hand
    /// does what its own amounts say, and Smooth under shift what Smooth
    /// does; both have the profile in hand.
    ///
    /// A classic brush with a classic curve and no picture across its tip
    /// is upstream's stamp, called and not copied. See the module
    /// documentation.
    pub(super) fn stamp_field(&mut self, brush: Brush, stamp: &Stamp) {
        let brush = self.studio.sized(brush, stamp.pressure);
        // Shift is Smooth, and not the brush in hand with its amounts and
        // the picture across its tip.
        let in_hand = brush.kind == self.brush.kind;
        let custom = self.studio.custom.filter(|_| in_hand);
        let tip = (in_hand && self.studio.tip_on).then(|| Arc::clone(&self.studio.alpha));
        let profile = self.studio.profile(brush.falloff);
        let one_layer = in_hand && !self.studio.build_up;
        let classic = custom.is_none()
            && tip.is_none()
            && !one_layer
            && profile == Profile::of(brush.falloff);
        let actions = custom.or_else(|| Actions::of(brush.kind)).filter(|_| !classic);
        let volume = self.doc.active_volume_mut();
        match actions {
            Some(actions) => {
                let compound = Compound {
                    actions,
                    radius: brush.radius,
                    strength: brush.strength,
                    profile,
                    pattern: brush.pattern,
                };
                if one_layer {
                    compound.lay_symmetric(
                        volume,
                        stamp,
                        tip.as_deref(),
                        self.symmetry,
                        super::MIRROR_CENTRE,
                        &mut self.studio.layer,
                        &mut self.brush_scratch,
                    );
                } else {
                    compound.apply_symmetric(
                        volume,
                        stamp,
                        tip.as_deref(),
                        self.symmetry,
                        super::MIRROR_CENTRE,
                        &mut self.brush_scratch,
                    );
                }
            }
            None => brush.apply_symmetric(
                volume,
                stamp,
                self.symmetry,
                super::MIRROR_CENTRE,
                &mut self.brush_scratch,
            ),
        }
    }

    /// The cross-section of the stroke: upstream's four falloff curves and
    /// the ball's, the live one lit, and how much of the middle is flat.
    ///
    /// In place of upstream's own row of four while a brush on the field
    /// is in hand. The four are still upstream's setting, changed by
    /// upstream's message.
    pub(super) fn profile_row(&self) -> Element<'_, Message> {
        let live = self.studio.profile(self.brush.falloff).shape;
        let shapes = Shape::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, shape| {
            let chosen = match shape.curve() {
                Some(curve) => Message::FalloffChanged(curve),
                None => Message::Studio(StudioMessage::RoundChosen),
            };
            assembled.push(
                button(text(shape.label()).size(theme::CAPTION_SIZE).width(Length::Fill).center())
                    .width(Length::Fill)
                    .padding(iced::Padding { top: 5.0, bottom: 5.0, left: 2.0, right: 2.0 })
                    .style(if shape == live {
                        theme::tool_button_active
                    } else {
                        theme::tool_button
                    })
                    .on_press(chosen),
            )
        });
        column![
            text("Profile").size(theme::TEXT_SIZE_SMALL).color(theme::TEXT_DIM),
            shapes,
            text(if self.studio.plateau > 0.0 {
                format!("Flat middle  {:.2} of the radius", self.studio.plateau)
            } else {
                "Flat middle  none".to_string()
            })
            .size(theme::CAPTION_SIZE)
            .color(theme::TEXT_DIM),
            slider(PLATEAU_RANGE, self.studio.plateau, |share| {
                Message::Studio(StudioMessage::PlateauChanged(share))
            })
            .step(0.05_f32),
        ]
        .spacing(theme::S2)
        .into()
    }

    /// What the brush in hand is made of, as a slider for each amount.
    fn amounts_panel(&self, actions: Actions) -> Element<'_, Message> {
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let amount = |label: String,
                      range: std::ops::RangeInclusive<f32>,
                      value: f32,
                      with: fn(Actions, f32) -> Actions| {
            column![
                reading(label),
                slider(range, value, move |amount| {
                    Message::Studio(StudioMessage::ActionsChosen(with(actions, amount)))
                })
                .step(0.05_f32),
            ]
            .spacing(theme::S1)
        };
        // A stamp moves the surface a voxel at most, so the push and the
        // inflation share one: the one being moved takes what it is given
        // and the other gives way. `Actions::legal` would bring both down
        // in proportion, and a slider that stops short of the pointer
        // reads as a slider that is stuck.
        let either = |amount: f32, more: &str, less: &str| {
            if amount > 0.0 {
                format!("{amount:.2} {more}")
            } else if amount < 0.0 {
                format!("{:.2} {less}", -amount)
            } else {
                "none".to_string()
            }
        };
        let share =
            |amount: f32| if amount > 0.0 { format!("{amount:.2}") } else { "none".to_string() };
        let mut amounts = column![
            amount(
                format!("Push  {}", either(actions.push, "out", "in")),
                -1.0..=1.0,
                actions.push,
                |actions, push| Actions {
                    push,
                    inflate: actions.inflate.clamp(push.abs() - 1.0, 1.0 - push.abs()),
                    ..actions
                },
            ),
            amount(
                format!("Inflate  {}", either(actions.inflate, "swelling", "shrinking")),
                -1.0..=1.0,
                actions.inflate,
                |actions, inflate| Actions {
                    inflate,
                    push: actions.push.clamp(inflate.abs() - 1.0, 1.0 - inflate.abs()),
                    ..actions
                },
            ),
            amount(
                format!("Gather  {}", either(actions.gather, "together", "apart")),
                -1.0..=1.0,
                actions.gather,
                |actions, gather| Actions { gather, ..actions },
            ),
            // Upstream's pinch, by what it does to what is under the
            // brush and not by its name: see `Actions::pinch`.
            amount(
                format!("Pinch  {}", either(actions.pinch, "steeper sided", "worn down")),
                -1.0..=1.0,
                actions.pinch,
                |actions, pinch| Actions { pinch, ..actions },
            ),
            amount(
                format!("Smooth  {}", share(actions.smooth)),
                0.0..=1.0,
                actions.smooth,
                |actions, smooth| Actions { smooth, ..actions },
            ),
            amount(
                format!("Level  {}", share(actions.level)),
                0.0..=1.0,
                actions.level,
                |actions, level| Actions { level, ..actions },
            ),
        ]
        .spacing(theme::S1);
        // The plane is a setting of a brush that levels, and of no other.
        if actions.level > 0.0 {
            let sides = Side::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, side| {
                assembled.push(
                    button(text(side.label()).size(theme::CAPTION_SIZE))
                        .width(Length::Fill)
                        .style(if side == actions.side {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::Studio(StudioMessage::ActionsChosen(Actions {
                            side,
                            ..actions
                        }))),
                )
            });
            amounts = amounts
                .push(amount(
                    format!(
                        "Level to a plane  {}",
                        either(actions.plane, "radii proud", "radii under")
                    )
                    .replace("none", "at the surface"),
                    -1.0..=1.0,
                    actions.plane,
                    |actions, plane| Actions { plane, ..actions },
                ))
                .push(sides)
                .push(
                    text(match actions.side {
                        Side::Both => "fills up to the plane and cuts down to it",
                        Side::Fill => "fills up to the plane and cuts nothing",
                        Side::Cut => "cuts down to the plane and fills nothing",
                    })
                    .size(theme::CAPTION_SIZE)
                    .color(theme::TEXT_MUTE),
                );
        }
        if actions.is_nothing() {
            amounts = amounts.push(
                text("a brush of nothing does nothing")
                    .size(theme::CAPTION_SIZE)
                    .color(theme::TEXT_MUTE),
            );
        }
        amounts.into()
    }

    /// The palette: the brushes there are, and keeping the one in hand.
    /// Over the brush's own block, because choosing a brush is what comes
    /// first.
    pub(super) fn palette_panel(&self) -> Element<'_, Message> {
        let changed = |message: StudioMessage| Message::Studio(message);
        let now = self.preset_now(String::new());
        let palette = self.studio.presets.palette();
        let lit = Self::lit(&palette, &now);
        let forgettable = lit.filter(|index| palette[*index].1 != Place::Core);
        let keep = row![
            text_input("name this brush", &self.studio.preset_name)
                .on_input(move |name| changed(StudioMessage::PresetNamed(name)))
                .on_submit(changed(StudioMessage::PresetSaved))
                .padding(iced::Padding { top: 1.0, bottom: 1.0, left: theme::S1, right: theme::S1 })
                .size(theme::TEXT_SIZE_SMALL)
                .width(Length::Fill),
            button(text("Keep").size(theme::CAPTION_SIZE))
                .style(theme::tool_button)
                .on_press(changed(StudioMessage::PresetSaved)),
            // Pressable only with a brush of the user's lit: that is the
            // one it means. With one that stands in for a brush that came
            // with the application, forgetting it puts that one back.
            button(
                text(match forgettable.map(|index| palette[index].1) {
                    Some(Place::Changed) => "Restore",
                    _ => "Forget",
                })
                .size(theme::CAPTION_SIZE)
            )
            .style(theme::tool_button)
            .on_press_maybe(forgettable.map(|index| changed(StudioMessage::PresetRemoved(index)))),
        ]
        .spacing(theme::S1);
        // A brush is a file, and files come from elsewhere: take some in,
        // or read the directory again after putting some there by hand.
        let files = row![
            button(text("Import\u{2026}").size(theme::CAPTION_SIZE).width(Length::Fill).center())
                .width(Length::Fill)
                .style(theme::tool_button)
                .on_press(changed(StudioMessage::BrushImportRequested)),
            button(text("Read again").size(theme::CAPTION_SIZE).width(Length::Fill).center())
                .width(Length::Fill)
                .style(theme::tool_button)
                .on_press(changed(StudioMessage::BrushesReloaded)),
        ]
        .spacing(theme::S1);
        // The heading, and the palette at the pointer for a hand that is
        // on a pen and not on the `b` key.
        let heading = row![
            text("BRUSHES").size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE).width(Length::Fill),
            button(text("At the pointer  b").size(theme::CAPTION_SIZE))
                .padding(iced::Padding { top: 1.0, bottom: 1.0, left: theme::S2, right: theme::S2 })
                .style(theme::tool_button)
                .on_press(changed(StudioMessage::PaletteToggled)),
        ]
        .align_y(iced::Alignment::Center);
        let mut panel = column![heading, self.brush_grid(3, 2), keep, files,].spacing(theme::S1);
        if let Some(place) = Presets::place() {
            panel = panel.push(
                text(format!("yours are kept in {place}"))
                    .size(theme::CAPTION_SIZE)
                    .color(theme::TEXT_MUTE),
            );
        }
        panel.into()
    }

    /// The brushes there are, a button each, the one in hand lit: the ones
    /// that come with the application `across` to a row under a heading
    /// for each group, and the user's own `own_across` to a row under them.
    ///
    /// A button to forget beside each was tried first, and seen on a
    /// screen it left the names 74 pixels to wrap in.
    fn brush_grid(&self, across: usize, own_across: usize) -> Element<'_, Message> {
        let now = self.preset_now(String::new());
        let palette = self.studio.presets.palette();
        let lit = Self::lit(&palette, &now);
        let came = super::presets::core().len();
        let brush = |index: usize| {
            let (preset, place) = palette[index];
            // One kept under the name of one that came with the
            // application is marked, so that it is not taken for it.
            let name = match place {
                Place::Changed => format!("{} *", preset.name),
                Place::Core | Place::Own => preset.name.clone(),
            };
            // A picture of a stroke of it, made by the brush itself, over
            // its name. See `app/swatch.rs`.
            let picture = iced::widget::image(super::swatch::handle(preset))
                .width(Length::Fill)
                .height(Length::Fixed(SWATCH_HEIGHT))
                .content_fit(iced::ContentFit::Contain);
            let face = column![
                picture,
                text(name).size(theme::CAPTION_SIZE).width(Length::Fill).center(),
            ]
            .spacing(2.0);
            button(face)
                .width(Length::Fill)
                .padding(iced::Padding { top: 3.0, bottom: 3.0, left: 2.0, right: 2.0 })
                .style(if lit == Some(index) {
                    theme::tool_button_active
                } else {
                    theme::tool_button
                })
                .on_press(Message::Studio(StudioMessage::PresetChosen(index)))
        };
        let rows = |from: usize, to: usize, across: usize| {
            let indices: Vec<usize> = (from..to).collect();
            indices.chunks(across).fold(column![].spacing(theme::S1), |rows, held| {
                let mut assembled = row![].spacing(theme::S1);
                for index in held {
                    assembled = assembled.push(brush(*index));
                }
                // A short last row keeps its buttons the width of the rest.
                for _ in held.len()..across {
                    assembled = assembled.push(iced::widget::Space::new().width(Length::Fill));
                }
                rows.push(assembled)
            })
        };
        let heading =
            |words: &'static str| text(words).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        // The ones that come with the application under a heading for
        // each kind of work, and then the user's own.
        let mut grid = column![].spacing(theme::S1);
        for (group, places) in super::presets::groups() {
            grid = grid.push(heading(group)).push(rows(places.start, places.end, across));
        }
        if palette.len() > came {
            grid = grid.push(heading("Yours")).push(rows(came, palette.len(), own_across));
        }
        grid.into()
    }

    /// How many rows the palette at the pointer has: each group of the
    /// brushes that come with the application in rows of its own, and
    /// then the user's.
    fn popup_rows(&self) -> usize {
        let came = super::presets::core().len();
        let own = self.studio.presets.palette().len().saturating_sub(came);
        let grouped: usize =
            super::presets::groups().map(|(_, places)| places.len().div_ceil(POPUP_ACROSS)).sum();
        grouped + own.div_ceil(POPUP_ACROSS)
    }

    /// Roughly how tall the palette at the pointer comes out: its heading,
    /// its rows, and a heading for each group of them.
    pub(super) fn popup_height(&self) -> f32 {
        let came = super::presets::core().len();
        let headings =
            super::presets::GROUPS.len() + usize::from(self.studio.presets.palette().len() > came);
        POPUP_HEAD + self.popup_rows() as f32 * POPUP_ROW + headings as f32 * POPUP_GROUP
    }

    /// Where the palette at the pointer goes, given where the pointer was:
    /// its top left, so that it is under the pointer and on the screen.
    pub(super) fn popup_corner(&self, at: Vec2) -> Vec2 {
        let height = self.popup_height();
        Vec2::new(
            (at.x - POPUP_WIDTH / 2.0).min(self.viewport_size.x - POPUP_WIDTH).max(0.0),
            (at.y - height / 2.0).min(self.viewport_size.y - height).max(0.0),
        )
    }

    /// The palette at the pointer: every brush, a short move from where
    /// the hand already is. ZBrush's `b`. Choosing one puts it away, and
    /// so does a press anywhere else, Escape, or `b` again.
    pub(super) fn palette_popup(&self) -> Option<Element<'_, Message>> {
        let corner = self.popup_corner(self.studio.popup?);
        let card = column![
            text("BRUSHES").size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE),
            self.brush_grid(POPUP_ACROSS, POPUP_ACROSS),
        ]
        .spacing(theme::S2);
        Some(
            container(
                container(card)
                    .padding(theme::S3)
                    .width(Length::Fixed(POPUP_WIDTH))
                    .style(theme::menu_card),
            )
            .padding(iced::Padding { left: corner.x, top: corner.y, ..iced::Padding::ZERO })
            .into(),
        )
    }

    /// Which picture is in hand, for the stamp to press or to lay across
    /// a brush's tip: the six that come with the application, three to a
    /// row, and a button that asks for any picture.
    fn picture_picker(&self) -> Element<'_, Message> {
        let changed = |message: StudioMessage| Message::Studio(message);
        let mut pictures = column![].spacing(theme::S1);
        for kinds in BuiltIn::ALL.chunks(3) {
            pictures =
                pictures.push(kinds.iter().fold(row![].spacing(theme::S1), |assembled, kind| {
                    let live = self.studio.built_in == Some(*kind);
                    assembled.push(
                        button(text(kind.label()).size(theme::CAPTION_SIZE))
                            .width(Length::Fill)
                            .style(if live {
                                theme::tool_button_active
                            } else {
                                theme::tool_button
                            })
                            .on_press(changed(StudioMessage::StampChosen(*kind))),
                    )
                }));
        }
        let own = self.studio.built_in.is_none();
        pictures
            .push(
                button(
                    text(if own {
                        format!("Picture: {}", self.studio.alpha_name)
                    } else {
                        "A picture…".to_string()
                    })
                    .size(theme::CAPTION_SIZE),
                )
                .width(Length::Fill)
                .style(if own { theme::tool_button_active } else { theme::tool_button })
                .on_press(changed(StudioMessage::StampImageRequested)),
            )
            .into()
    }

    /// What the brush in hand is made of and how it is led: under the
    /// classic brush's size and strength.
    pub(super) fn studio_panel(&self) -> Element<'_, Message> {
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let changed = |message: StudioMessage| Message::Studio(message);
        let stamping = self.studio.stamp_on;
        let mut stamp = column![
            button(text("Stamp").size(theme::CAPTION_SIZE))
                .width(Length::Fill)
                .style(if stamping { theme::tool_button_active } else { theme::tool_button })
                .on_press(changed(StudioMessage::StampToggled(!stamping))),
        ]
        .spacing(theme::S1);
        if stamping {
            stamp = stamp
                .push(self.picture_picker())
                .push(reading(format!("Height  {:.2} of its size", self.studio.stamp_height)))
                .push(
                    slider(HEIGHT_RANGE, self.studio.stamp_height, move |share| {
                        changed(StudioMessage::StampHeightChanged(share))
                    })
                    .step(0.01_f32),
                )
                .push({
                    // One stamp sized by the drag, or a row of them at the
                    // brush's size: pressed again, the row is put away.
                    let row = self.studio.stamp_along > 0.0;
                    button(text("Along the stroke").size(theme::CAPTION_SIZE))
                        .width(Length::Fill)
                        .style(if row { theme::tool_button_active } else { theme::tool_button })
                        .on_press(changed(StudioMessage::StampAlongChanged(if row {
                            0.0
                        } else {
                            DEFAULT_ALONG
                        })))
                });
            if self.studio.stamp_along > 0.0 {
                stamp = stamp
                    .push(reading(format!(
                        "One every  {:.1} of its width",
                        self.studio.stamp_along
                    )))
                    .push(
                        slider(ALONG_RANGE, self.studio.stamp_along, move |widths| {
                            changed(StudioMessage::StampAlongChanged(widths))
                        })
                        .step(0.1_f32),
                    );
            }
            stamp = stamp.push(
                text(if self.studio.stamp_along > 0.0 {
                    "drag to lay a row, each the brush's size"
                } else {
                    "press where it goes, drag to size and turn it"
                })
                .size(theme::CAPTION_SIZE)
                .color(theme::TEXT_MUTE),
            );
        }
        // What the brush is made of, for a brush on the field: put away
        // until it is asked for, because most strokes are made with a
        // brush as it came.
        let made_of = self.studio.custom.or_else(|| Actions::of(self.brush.kind));
        let mut settings = column![].spacing(theme::S1);
        if let Some(actions) = made_of.filter(|_| !stamping) {
            // A picture across the tip: the one in hand, which is the one
            // the stamp presses.
            let tipped = self.studio.tip_on;
            settings = settings.push(
                button(text("Picture across the tip").size(theme::CAPTION_SIZE))
                    .width(Length::Fill)
                    .style(if tipped { theme::tool_button_active } else { theme::tool_button })
                    .on_press(changed(StudioMessage::TipToggled(!tipped))),
            );
            if tipped {
                settings = settings.push(self.picture_picker()).push(
                    text("where the picture is bright the brush works, its top along the stroke")
                        .size(theme::CAPTION_SIZE)
                        .color(theme::TEXT_MUTE),
                );
            }
            // Whether a stroke builds on itself, and whether it is a line
            // or the brush set down every so often.
            let builds = self.studio.build_up;
            let way = |label: &'static str, on: bool| {
                button(text(label).size(theme::CAPTION_SIZE).width(Length::Fill).center())
                    .width(Length::Fill)
                    .style(if builds == on {
                        theme::tool_button_active
                    } else {
                        theme::tool_button
                    })
                    .on_press(changed(StudioMessage::BuildUpToggled(on)))
            };
            settings = settings
                .push(row![way("Builds up", true), way("One layer", false)].spacing(theme::S1))
                .push(reading(if self.studio.dabs > 0.0 {
                    format!("Set down every  {:.1} of its width", self.studio.dabs)
                } else {
                    "Set down  as a line".to_string()
                }))
                .push(
                    slider(0.0..=*ALONG_RANGE.end(), self.studio.dabs, move |widths| {
                        changed(StudioMessage::DabsChanged(widths))
                    })
                    .step(0.5_f32),
                );
            let open = self.studio.open;
            settings = settings.push(
                button(
                    text(if open { "What it does  \u{25be}" } else { "What it does  \u{25b8}" })
                        .size(theme::CAPTION_SIZE),
                )
                .width(Length::Fill)
                .style(if open { theme::tool_button_active } else { theme::tool_button })
                .on_press(changed(StudioMessage::SettingsShown(!open))),
            );
            if open {
                settings = settings.push(self.amounts_panel(actions));
            }
        }
        column![
            settings,
            text("STROKE").size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE),
            stamp,
            reading(if self.studio.lazy_px > 0.0 {
                format!("Lazy  {:.0} px behind the pointer", self.studio.lazy_px)
            } else {
                "Lazy  off".to_string()
            }),
            slider(LAZY_RANGE, self.studio.lazy_px, move |pixels| {
                changed(StudioMessage::LazyChanged(pixels))
            })
            .step(1.0_f32),
            reading(format!("Size from pressure  {:.2}", self.studio.size_from_pressure)),
            slider(SIZE_RANGE, self.studio.size_from_pressure, move |share| {
                changed(StudioMessage::SizeFromPressureChanged(share))
            })
            .step(0.01_f32),
        ]
        .spacing(theme::S1)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};
    use brokkr_core::BrushKind;
    use glam::Vec3;

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    fn centre() -> iced::Vector {
        iced::Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
    }

    fn press(app: &mut Brokkr) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre(),
                size: SIZE,
            }),
        );
    }

    fn drag_by(app: &mut Brokkr, pixels: f32) {
        let position = iced::Vector::new(centre().x + pixels, centre().y);
        update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
    }

    fn release(app: &mut Brokkr) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short drag across the middle of the model.
    fn stroke(app: &mut Brokkr) {
        press(app);
        for step in 1..=4 {
            drag_by(app, 12.0 * step as f32);
        }
        release(app);
    }

    /// How far the surface under the middle of the viewport has moved from
    /// where a fresh ball has it, in millimetres, outward positive.
    fn shift_under_the_stroke(app: &Brokkr) -> f32 {
        let fresh = self::app();
        // The point of the fresh ball's surface nearest the camera, which is
        // where the stroke pressed.
        let toward_camera = (fresh.camera.eye() - Vec3::ZERO).normalize();
        let on_the_surface = toward_camera * super::super::MODEL_RADIUS_MM;
        let volume = app.doc.active_volume();
        -volume.sample_world(on_the_surface) * volume.voxel_size()
    }

    /// The lead: nothing moves inside it, and past it the brush is exactly a
    /// lead's length behind the pointer, on the line between them.
    #[test]
    fn a_lazy_brush_trails_the_pointer_by_its_lead_and_no_more() {
        let mut studio = StudioState { lazy_px: 20.0, ..StudioState::default() };
        let start = Vec2::new(100.0, 100.0);
        assert_eq!(studio.follow(start, true), start, "a stroke begins under the pointer");
        assert_eq!(studio.follow(Vec2::new(115.0, 100.0), false), start, "inside the lead");
        assert_eq!(studio.follow(Vec2::new(100.0, 119.0), false), start, "inside the lead");
        let pulled = studio.follow(Vec2::new(150.0, 100.0), false);
        assert_eq!(pulled, Vec2::new(130.0, 100.0), "the brush is not a lead behind the pointer");
        // Back toward it by less than the lead: the brush does not move.
        assert_eq!(studio.follow(Vec2::new(140.0, 100.0), false), pulled);
        // Off at an angle: toward the pointer, a lead short.
        let pointer = Vec2::new(130.0, 160.0);
        let turned = studio.follow(pointer, false);
        assert!((turned.distance(pointer) - 20.0).abs() < 1.0e-3, "{turned}");
        assert!((turned.x - 130.0).abs() < 1.0e-3, "the brush left the line to the pointer");
        // A new stroke forgets the old one's place.
        assert_eq!(studio.follow(Vec2::new(5.0, 5.0), true), Vec2::new(5.0, 5.0));

        // With no lead the brush IS the pointer.
        let mut off = StudioState::default();
        off.follow(start, true);
        assert_eq!(off.follow(Vec2::new(300.0, 7.0), false), Vec2::new(300.0, 7.0));
    }

    #[test]
    fn pressure_sizes_the_brush_by_as_much_as_it_is_given() {
        let brush = Brush { radius: 4.0, ..Brush::default() };
        let none = StudioState::default();
        assert_eq!(none.sized(brush, 0.2).radius, 4.0, "with no share, pressure sized the brush");
        let all = StudioState { size_from_pressure: 1.0, ..StudioState::default() };
        assert_eq!(all.sized(brush, 1.0).radius, 4.0, "full pressure is the whole brush");
        assert_eq!(all.sized(brush, 0.5).radius, 2.0);
        assert_eq!(all.sized(brush, 0.0).radius, 4.0 * SMALLEST_SHARE, "no touch, no brush");
        assert_eq!(all.sized(brush, 7.0).radius, 4.0, "pressure past full grew the brush");
        let half = StudioState { size_from_pressure: 0.5, ..StudioState::default() };
        assert_eq!(half.sized(brush, 0.0).radius, 2.0);
        // And nothing else about the brush is touched.
        assert_eq!(all.sized(brush, 0.5), Brush { radius: 2.0, ..brush });
    }

    /// With a crease's amounts in hand, a stroke cuts in where the classic
    /// Draw raises; a classic brush chosen puts them away; shift still
    /// smooths.
    #[test]
    fn amounts_of_its_own_replace_what_the_stroke_does_until_a_classic_brush_is_chosen() {
        let mut drawn = app();
        stroke(&mut drawn);
        let raised = shift_under_the_stroke(&drawn);
        assert!(raised > 0.05, "the classic Draw control raised only {raised} mm");

        let mut creased = app();
        update(&mut creased, Message::Studio(StudioMessage::ActionsChosen(Actions::CREASE)));
        assert_eq!(creased.live_brush_name(), "CREASE");
        assert_eq!(creased.brush.kind, BrushKind::Draw, "a crease goes by Draw");
        stroke(&mut creased);
        let cut = shift_under_the_stroke(&creased);
        assert!(cut < -0.02, "a crease stroke moved the surface by {cut} mm, which is not in");
        update(&mut creased, Message::Undo);
        // Against a ball nobody has touched and not against nothing: the
        // probe is a point in space, and the lattice's ball is not exactly
        // on it.
        assert_eq!(
            shift_under_the_stroke(&creased),
            shift_under_the_stroke(&app()),
            "a crease stroke was not one undo"
        );

        // Shift smooths, whatever is chosen: the same stroke with shift held
        // leaves exactly what it leaves with the classic brush in hand.
        let smoothed = |studio: bool| {
            let mut app = app();
            stroke(&mut app);
            if studio {
                update(&mut app, Message::Studio(StudioMessage::ActionsChosen(Actions::CREASE)));
            }
            app.shift = true;
            stroke(&mut app);
            shift_under_the_stroke(&app)
        };
        assert_eq!(smoothed(true), smoothed(false), "shift did not smooth over a crease");

        // A classic brush puts the amounts away.
        update(&mut creased, Message::BrushKindChanged(BrushKind::Inflate));
        assert_eq!(creased.studio.custom, None, "choosing a classic brush kept the crease");
        assert_eq!(creased.live_brush_name(), "INFLATE");

        // And amounts that are just a classic brush's are that brush.
        update(&mut creased, Message::Studio(StudioMessage::ActionsChosen(Actions::CLAY)));
        assert_eq!((creased.brush.kind, creased.studio.custom), (BrushKind::Clay, None));
        assert_eq!(creased.live_brush_name(), "CLAY");
    }

    /// The lead reaches the stroke: with a lead longer than the drag, the
    /// brush never leaves where it was pressed and lays no stamp after the
    /// first.
    #[test]
    fn a_lazy_stroke_lays_nothing_until_the_pointer_outruns_its_lead() {
        let mut app = app();
        update(&mut app, Message::Studio(StudioMessage::LazyChanged(60.0)));
        press(&mut app);
        drag_by(&mut app, 20.0);
        assert_eq!(app.perf.stamps, 0, "the brush moved inside its lead");
        drag_by(&mut app, 45.0);
        assert_eq!(app.perf.stamps, 0, "the brush moved inside its lead");
        drag_by(&mut app, 140.0);
        assert!(app.perf.stamps > 0, "the pointer outran the lead and the brush did not follow");
        release(&mut app);

        // The control: with no lead the same first drag lays stamps.
        let mut direct = self::app();
        press(&mut direct);
        drag_by(&mut direct, 45.0);
        assert!(direct.perf.stamps > 0, "a 45 pixel drag lays no stamp even without a lead");
    }

    /// How many voxels of the active body are painted at all.
    fn painted_voxels(app: &Brokkr) -> usize {
        let volume = app.doc.active_volume();
        let (low, high) = volume.world_bounds().expect("the body has bricks");
        let (lo, hi) = volume.voxel_bounds(low, high);
        let mut count = 0;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    count += usize::from(volume.colour().at(glam::IVec3::new(x, y, z)) != 0);
                }
            }
        }
        count
    }

    /// A pen pressed lightly is a smaller brush when size follows pressure,
    /// for the paint brush as for the others, and is the brush as it was
    /// when size does not. Measured with paint, which reads pressure for
    /// nothing else: a slot has no half.
    #[test]
    fn a_light_touch_paints_a_narrower_stroke_when_size_follows_pressure() {
        let painted = |share: f32, pressure: f32| {
            let mut app = app();
            update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
            // By slot, where a voxel is painted or is not: in colour a
            // light touch is a thinner coat as well as a narrower one.
            update(
                &mut app,
                Message::Painting(crate::message::PaintingMessage::Target(
                    crate::message::PaintTarget::Slots,
                )),
            );
            update(&mut app, Message::Studio(StudioMessage::SizeFromPressureChanged(share)));
            app.tablet.simulate(crate::tablet::PenState {
                in_proximity: true,
                pressure,
                eraser: false,
                tilt: Vec2::ZERO,
            });
            stroke(&mut app);
            painted_voxels(&app)
        };
        let (full, light) = (painted(1.0, 1.0), painted(1.0, 0.4));
        assert!(full > 500, "the full pressure stroke painted only {full} voxels");
        assert!(
            light * 2 < full,
            "at 0.4 of the pressure the stroke painted {light} voxels against {full}"
        );
        assert_eq!(painted(0.0, 0.4), full, "with no share, pressure still sized the paint brush");
    }

    /// How many voxels of the active body differ from an untouched ball's.
    fn moved_voxels(app: &Brokkr) -> usize {
        let fresh = self::app();
        let (volume, untouched) = (app.doc.active_volume(), fresh.doc.active_volume());
        let (low, high) = untouched.world_bounds().expect("the body has bricks");
        let (lo, hi) = untouched.voxel_bounds(low - Vec3::splat(4.0), high + Vec3::splat(4.0));
        let mut count = 0;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = glam::IVec3::new(x, y, z);
                    count += usize::from(
                        volume.sample_voxel(voxel).to_bits()
                            != untouched.sample_voxel(voxel).to_bits(),
                    );
                }
            }
        }
        count
    }

    /// And the sculpt brushes: at the same light pressure, a stroke whose
    /// size follows pressure touches fewer voxels than one whose size does
    /// not. The pressure is the same in both, so the strength is, and the
    /// only thing left to differ is how wide the brush was.
    #[test]
    fn a_light_touch_sculpts_a_narrower_stroke_when_size_follows_pressure() {
        let moved = |share: f32| {
            let mut app = app();
            update(&mut app, Message::Studio(StudioMessage::SizeFromPressureChanged(share)));
            app.tablet.simulate(crate::tablet::PenState {
                in_proximity: true,
                pressure: 0.4,
                eraser: false,
                tilt: Vec2::ZERO,
            });
            stroke(&mut app);
            moved_voxels(&app)
        };
        let (wide, narrow) = (moved(0.0), moved(1.0));
        assert!(wide > 500, "the control stroke moved only {wide} voxels");
        assert!(narrow > 0, "a light touch with size from pressure moved nothing at all");
        assert!(
            narrow * 2 < wide,
            "with size following pressure the stroke moved {narrow} voxels against {wide}"
        );
    }

    /// Whether two applications' active bodies hold the same field, to the
    /// bit.
    fn same_field(one: &Brokkr, other: &Brokkr) -> bool {
        let (a, b) = (one.doc.active_volume(), other.doc.active_volume());
        let (low, high) = a.world_bounds().expect("the body has bricks");
        let (lo, hi) = a.voxel_bounds(low - Vec3::splat(8.0), high + Vec3::splat(8.0));
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = glam::IVec3::new(x, y, z);
                    if a.sample_voxel(voxel).to_bits() != b.sample_voxel(voxel).to_bits() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// How far the surface under the middle of the viewport is from where a
    /// fresh ball has it, in millimetres, outward positive, found by walking
    /// along the line to the camera until the field changes sign.
    ///
    /// `shift_under_the_stroke` reads the field at one point, and the field
    /// stops at three voxels: it cannot tell a stamp one millimetre tall
    /// from one three millimetres tall.
    fn height_under_the_press(app: &Brokkr) -> f32 {
        let toward_camera = (self::app().camera.eye() - Vec3::ZERO).normalize();
        let volume = app.doc.active_volume();
        let inside = |along: f32| {
            volume.sample_world(toward_camera * (super::super::MODEL_RADIUS_MM + along)) < 0.0
        };
        // From well inside, outward, to the first point that is not solid.
        let mut along = -8.0;
        while along < 20.0 && inside(along) {
            along += 0.02;
        }
        along
    }

    /// A stamp dragged out to each of `pixels` in turn and let go.
    fn stamped(pixels: &[f32], configure: impl Fn(&mut Brokkr)) -> Brokkr {
        let mut app = app();
        update(&mut app, Message::Studio(StudioMessage::StampToggled(true)));
        configure(&mut app);
        press(&mut app);
        for at in pixels {
            drag_by(&mut app, *at);
        }
        release(&mut app);
        app
    }

    /// The stamp through the whole application: a drag presses the image
    /// in, a longer drag presses a bigger one, a drag out and back is the
    /// stamp it ended as, inverted it engraves, and it is one undo.
    #[test]
    fn a_stamp_drag_presses_the_image_in_at_the_size_of_the_drag() {
        let small = stamped(&[30.0], |_| {});
        // The stamp as it starts is one of the brushes on the palette.
        assert_eq!(small.live_brush_name(), "RIVETS");
        let (small_moved, small_rise) = (moved_voxels(&small), height_under_the_press(&small));
        assert!(small_moved > 200, "a stamp moved only {small_moved} voxels");
        assert!(small_rise > 0.2, "the middle of a rivet rose by {small_rise} mm");
        assert!(small.unsaved, "a stamp did not dirty the document");

        // Twice the drag is twice the stamp: four times the area, and twice
        // as tall, since the height is a share of the size.
        let large = stamped(&[60.0], |_| {});
        assert!(
            moved_voxels(&large) > small_moved * 3,
            "twice the drag moved {} voxels against {small_moved}",
            moved_voxels(&large)
        );
        let large_rise = height_under_the_press(&large);
        assert!(
            (large_rise / small_rise - 2.0).abs() < 0.25,
            "twice the drag rose {large_rise} mm against {small_rise}"
        );

        // Out and back: only where the drag ended counts. To the bit for a
        // drag that stays small, which is the core's own guarantee
        // (`a_drag_replaces_the_stamp_and_never_presses_it_in_twice`).
        let wandered = stamped(&[10.0, 55.0, 30.0], |_| {});
        assert!(same_field(&wandered, &small), "a stamp remembers the sizes it passed through");
        // And to within the pointer's own reading for one that goes far out
        // first. Not to the bit, and the stamp is not why: a stamp 7 mm
        // across changes how far the model reaches, the view follows that,
        // and the same pixel is then a ray two parts in ten thousand
        // different. Measured: a radius of 2.4912 mm where it was 2.4917.
        let far = stamped(&[90.0, 30.0], |_| {});
        let (far_moved, far_rise) = (moved_voxels(&far), height_under_the_press(&far));
        assert!(
            far_moved.abs_diff(small_moved) * 50 < small_moved,
            "after a long drag back in, {far_moved} voxels moved against {small_moved}"
        );
        assert!((far_rise - small_rise).abs() < 0.03, "{far_rise} mm against {small_rise}");

        // Taller when told to be, and engraved when inverted.
        let tall = stamped(&[30.0], |app| {
            update(app, Message::Studio(StudioMessage::StampHeightChanged(0.7)))
        });
        let tall_rise = height_under_the_press(&tall);
        assert!(
            (tall_rise / small_rise - 2.0).abs() < 0.25,
            "at twice the height setting the stamp rose {tall_rise} mm against {small_rise}"
        );
        let engraved = stamped(&[30.0], |app| app.control = true);
        let sunk = height_under_the_press(&engraved);
        assert!(
            (sunk + small_rise).abs() < small_rise * 0.15,
            "an inverted stamp put the middle at {sunk} mm where upright it rose {small_rise}"
        );

        // One undo takes the whole gesture off.
        let mut undone = small;
        update(&mut undone, Message::Undo);
        assert_eq!(moved_voxels(&undone), 0, "a stamp was not one undo");

        // A press with no drag presses nothing and leaves nothing to undo.
        let pressed_only = stamped(&[], |_| {});
        assert_eq!(moved_voxels(&pressed_only), 0);
        assert!(!pressed_only.unsaved, "a press that stamped nothing dirtied the document");
    }

    /// Each built-in is its own shape on the model, and a picture is a
    /// stamp too.
    #[test]
    fn the_stamp_image_chosen_is_the_one_pressed() {
        let with = |kind: BuiltIn| {
            stamped(&[40.0], |app| update(app, Message::Studio(StudioMessage::StampChosen(kind))))
        };
        let (rivet, star) = (with(BuiltIn::Rivet), with(BuiltIn::Star));
        assert!(!same_field(&rivet, &star), "a star was pressed as a rivet");
        assert_eq!(star.studio.built_in, Some(BuiltIn::Star));
        assert_eq!(star.studio.alpha_name, "Star");

        // A picture, written here and read back by the same function the
        // dialog uses: white in the top right quarter, black elsewhere.
        let directory = std::env::temp_dir().join(format!("brokkr-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        let path = directory.join("corner mark.png");
        // And mid grey in the bottom left, at a level only sixteen bits
        // can hold: 32768 is half way, and read as eight bits it is white.
        let picture = image::ImageBuffer::from_fn(32, 32, |x, y| {
            image::Luma([if x >= 16 && y < 16 {
                u16::MAX
            } else if x < 16 && y >= 16 {
                32768
            } else {
                0
            }])
        });
        picture.save(&path).expect("writing the fixture");
        let loaded = read_stamp_image(&path).expect("a PNG is a stamp");
        assert_eq!(loaded.name, "corner mark");
        assert!(loaded.alpha.sample(0.5, 0.5) > 0.9, "the white quarter is not top right");
        assert_eq!(loaded.alpha.sample(-0.5, 0.5), 0.0);
        assert_eq!(loaded.alpha.sample(0.5, -0.5), 0.0);
        let grey = loaded.alpha.sample(-0.5, -0.5);
        assert!((grey - 0.5).abs() < 0.001, "half grey was read as a height of {grey}");

        let mut app = app();
        update(
            &mut app,
            Message::Studio(StudioMessage::StampImageLoaded(Some(Ok(loaded.clone())))),
        );
        assert_eq!(app.studio.built_in, None);
        assert_eq!(app.studio.alpha_name, "corner mark");
        assert!(Arc::ptr_eq(&app.studio.alpha, &loaded.alpha), "the picture was not taken up");
        assert!(app.status.contains("corner mark"), "{}", app.status);
        drop(app.studio_panel());

        // A file that is not a picture, and one that is not there, are each
        // refused with a reason and change nothing.
        let not_a_picture = directory.join("notes.png");
        std::fs::write(&not_a_picture, b"this is not a picture").unwrap();
        for path in [not_a_picture, directory.join("missing.png")] {
            let refused = read_stamp_image(&path);
            let why = refused.expect_err("something that is not a picture was read as one");
            assert!(!why.is_empty());
            update(&mut app, Message::Studio(StudioMessage::StampImageLoaded(Some(Err(why)))));
            assert!(app.status.contains("cannot be a stamp"), "{}", app.status);
            assert_eq!(app.studio.alpha_name, "corner mark", "a refusal replaced the stamp");
        }
        // A dismissed dialog is nothing at all.
        let status = app.status.clone();
        update(&mut app, Message::Studio(StudioMessage::StampImageLoaded(None)));
        assert_eq!(app.status, status);
        std::fs::remove_dir_all(&directory).ok();
    }

    /// The stamp, amounts of a brush's own and a classic brush are three
    /// things to have in hand, and taking one up puts the others away.
    #[test]
    fn the_stamp_amounts_and_a_classic_brush_each_put_the_others_away() {
        let mut app = app();
        let set = |app: &mut Brokkr, message| update(app, Message::Studio(message));
        set(&mut app, StudioMessage::ActionsChosen(Actions::CREASE));
        set(&mut app, StudioMessage::StampToggled(true));
        assert!(app.studio.stamping() && app.studio.custom.is_none(), "the stamp kept the crease");
        assert!(app.status.contains("stamp"), "{}", app.status);
        set(&mut app, StudioMessage::ActionsChosen(Actions::CREASE));
        assert!(!app.studio.stamping() && app.studio.custom == Some(Actions::CREASE));
        set(&mut app, StudioMessage::StampToggled(true));
        update(&mut app, Message::BrushKindChanged(BrushKind::Smooth));
        assert!(!app.studio.stamping(), "choosing a classic brush left the stamp up");
        assert_eq!(app.live_brush_name(), "SMOOTH");
        // Toggled off by its own button, a stroke is a stroke again.
        set(&mut app, StudioMessage::StampToggled(true));
        set(&mut app, StudioMessage::StampToggled(false));
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        stroke(&mut app);
        assert!(app.perf.stamps > 0, "with the stamp put away a drag laid no brush stamps");

        set(&mut app, StudioMessage::StampHeightChanged(5.0));
        assert_eq!(app.studio.stamp_height, *HEIGHT_RANGE.end());
        set(&mut app, StudioMessage::StampHeightChanged(f32::NAN));
        assert_eq!(app.studio.stamp_height, *HEIGHT_RANGE.start());
        set(&mut app, StudioMessage::StampToggled(true));
        drop(app.studio_panel());
    }

    #[test]
    fn studio_settings_are_clamped_and_the_panel_builds() {
        let mut app = app();
        let set = |app: &mut Brokkr, message| update(app, Message::Studio(message));
        set(&mut app, StudioMessage::LazyChanged(33.0));
        set(&mut app, StudioMessage::SizeFromPressureChanged(0.4));
        assert_eq!((app.studio.lazy_px, app.studio.size_from_pressure), (33.0, 0.4));
        set(&mut app, StudioMessage::LazyChanged(900.0));
        set(&mut app, StudioMessage::SizeFromPressureChanged(-3.0));
        assert_eq!(app.studio.lazy_px, *LAZY_RANGE.end());
        assert_eq!(app.studio.size_from_pressure, 0.0);
        set(&mut app, StudioMessage::LazyChanged(f32::NAN));
        set(&mut app, StudioMessage::SizeFromPressureChanged(f32::NAN));
        assert_eq!((app.studio.lazy_px, app.studio.size_from_pressure), (0.0, 0.0));
        set(&mut app, StudioMessage::PlateauChanged(7.0));
        assert_eq!(app.studio.plateau, *PLATEAU_RANGE.end());
        set(&mut app, StudioMessage::PlateauChanged(f32::NAN));
        assert_eq!(app.studio.plateau, 0.0);
        // Every panel there is, with the brush's settings put away and
        // shown, for a brush that levels (which has a plane to set), one
        // that does nothing, the stamp, and the two tools that are not
        // brushes on the field.
        let panels = |app: &Brokkr| {
            drop(app.studio_panel());
            drop(app.palette_panel());
            drop(app.profile_row());
        };
        panels(&app);
        set(&mut app, StudioMessage::SettingsShown(true));
        assert!(app.studio.open);
        for actions in [Actions::CREASE, Actions::CLAY, Actions::default()] {
            set(&mut app, StudioMessage::ActionsChosen(actions));
            panels(&app);
        }
        set(&mut app, StudioMessage::RoundChosen);
        set(&mut app, StudioMessage::StampToggled(true));
        panels(&app);
        for kind in [BrushKind::Move, BrushKind::Paint] {
            update(&mut app, Message::BrushKindChanged(kind));
            panels(&app);
        }
        set(&mut app, StudioMessage::SettingsShown(false));
        assert!(!app.studio.open);
    }

    fn set(app: &mut Brokkr, message: StudioMessage) {
        update(app, Message::Studio(message));
    }

    /// A unit step across the viewport: the way a test's drag runs.
    #[derive(Clone, Copy)]
    struct Way(f32, f32);

    const ACROSS: Way = Way(1.0, 0.0);
    const UP_THE_VIEW: Way = Way(0.0, -1.0);

    /// A row of stamps: the stamp taken up, set to lay along the stroke,
    /// and dragged `pixels` from the middle of the view the way `way` runs.
    fn row(pixels: f32, way: Way, configure: impl Fn(&mut Brokkr)) -> Brokkr {
        let mut app = app();
        set(&mut app, StudioMessage::StampToggled(true));
        set(&mut app, StudioMessage::StampAlongChanged(1.5));
        update(&mut app, Message::BrushRadiusChanged(1.0));
        configure(&mut app);
        press(&mut app);
        // In a few moves, as a hand makes them.
        for part in 1..=5 {
            let far = pixels * part as f32 / 5.0;
            let position = iced::Vector::new(centre().x + way.0 * far, centre().y + way.1 * far);
            update(&mut app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
        }
        release(&mut app);
        app
    }

    /// Where on a ball nothing has been pressed into the pointer is,
    /// `pixels` from the middle of the view along `way`.
    fn on_the_ball(pixels: f32, way: Way) -> Vec3 {
        // A press and a release with the stamp up lay nothing, and tell
        // the application how big its viewport is.
        let looking = stamped(&[], |_| {});
        let pixel = Vec2::new(centre().x + way.0 * pixels, centre().y + way.1 * pixels);
        looking.surface_under(pixel).expect("the ball is under the pointer")
    }

    /// How far the surface has risen above a fresh ball's, every tenth of
    /// a millimetre from the press along the line a drag ran: a row of
    /// stamps in profile.
    fn profile(app: &Brokkr, way: Way, length_mm: f32) -> Vec<f32> {
        let (from, toward) =
            (on_the_ball(0.0, way).normalize(), on_the_ball(40.0, way).normalize());
        let side = (toward - from * from.dot(toward)).normalize();
        let radius = super::super::MODEL_RADIUS_MM;
        let volume = app.doc.active_volume();
        (0..(length_mm * 10.0) as usize)
            .map(|step| {
                let angle = step as f32 * 0.1 / radius;
                let out = from * angle.cos() + side * angle.sin();
                let inside = |along: f32| volume.sample_world(out * (radius + along)) < 0.0;
                let mut along = -3.0;
                while along < 6.0 && inside(along) {
                    along += 0.02;
                }
                along
            })
            .collect()
    }

    /// The runs of a profile that pass `over`: where each begins and ends,
    /// in millimetres along it.
    fn runs(profile: &[f32], over: impl Fn(f32) -> bool) -> Vec<(f32, f32)> {
        let mut found: Vec<(f32, f32)> = Vec::new();
        let mut open = None;
        for (index, rise) in profile.iter().enumerate() {
            let at = index as f32 * 0.1;
            match (over(*rise), open) {
                (true, None) => open = Some(at),
                (false, Some(began)) => {
                    found.push((began, at));
                    open = None;
                }
                _ => {}
            }
        }
        if let Some(began) = open {
            found.push((began, profile.len() as f32 * 0.1));
        }
        found
    }

    /// The stamp laid along a stroke: a row of them, the first at the
    /// press, each the brush's size and a step from the last, as many as
    /// the drag is steps long. Inverted they are engraved, a bigger brush
    /// is bigger stamps further apart, and the row is one undo.
    #[test]
    fn a_stamp_along_the_stroke_lays_a_row_at_the_brushs_size() {
        let drag = 150.0;
        let length = (on_the_ball(drag, ACROSS) - on_the_ball(0.0, ACROSS)).length();
        // A brush 1 mm in radius, every 1.5 widths: a step of 3 mm.
        let step = 3.0;
        let expected = (length / step).floor() as usize;
        let spare = (length / step).fract();
        assert!(
            expected >= 3 && (0.15..0.85).contains(&spare),
            "the fixture's drag is {length} mm: too short, or too near a whole number of steps"
        );

        let laid = row(drag, ACROSS, |_| {});
        // And so is this row: a rivet a millimetre across, every width and
        // a half.
        assert_eq!(laid.live_brush_name(), "RIVET ROW");
        let risen = profile(&laid, ACROSS, length + 4.0);
        let tallest = risen.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            (0.25..0.45).contains(&tallest),
            "a rivet 1 mm in radius at a height of 0.35 rose {tallest} mm"
        );
        let found = runs(&risen, |rise| rise > tallest / 2.0);
        assert_eq!(found.len(), expected, "{found:?} along a drag of {length} mm");
        // The first is at the press: the profile begins in the middle of it.
        assert!(found[0].0 == 0.0 && found[0].1 < 1.0, "{found:?}");
        for (index, (began, ended)) in found.iter().enumerate().skip(1) {
            let middle = (began + ended) / 2.0;
            assert!(
                (middle - step * index as f32).abs() < 0.3,
                "stamp {index} is {middle} mm along: {found:?}"
            );
            // The brush's size, and not the drag's: a rivet's flat top and
            // half its bevel, of a stamp 2 mm across.
            assert!((1.0..2.0).contains(&(ended - began)), "stamp {index}: {found:?}");
        }
        assert!(laid.unsaved, "a row did not dirty the document");

        // A brush twice the size: stamps twice as far apart and twice as
        // tall, so half as many of them.
        let big = row(drag, ACROSS, |app| update(app, Message::BrushRadiusChanged(2.0)));
        let big_risen = profile(&big, ACROSS, length + 6.0);
        let big_tallest = big_risen.iter().copied().fold(0.0_f32, f32::max);
        assert!((big_tallest / tallest - 2.0).abs() < 0.3, "{big_tallest} mm against {tallest}");
        let big_found = runs(&big_risen, |rise| rise > big_tallest / 2.0);
        assert_eq!(big_found.len(), (length / (2.0 * step)).floor() as usize, "{big_found:?}");

        // Taller when told to be.
        let tall = row(drag, ACROSS, |app| set(app, StudioMessage::StampHeightChanged(0.7)));
        let tall_tallest =
            profile(&tall, ACROSS, length + 4.0).iter().copied().fold(0.0_f32, f32::max);
        assert!(
            (tall_tallest / tallest - 2.0).abs() < 0.3,
            "at twice the height setting the row rose {tall_tallest} mm against {tallest}"
        );

        // Closer together when told to be.
        let close = row(drag, ACROSS, |app| set(app, StudioMessage::StampAlongChanged(1.0)));
        let close_found = runs(&profile(&close, ACROSS, length + 4.0), |rise| rise > tallest / 2.0);
        assert_eq!(close_found.len(), (length / 2.0).floor() as usize, "{close_found:?}");

        // Inverted, the row is engraved.
        let engraved = row(drag, ACROSS, |app| app.control = true);
        let sunk = profile(&engraved, ACROSS, length + 4.0);
        let deepest = sunk.iter().copied().fold(0.0_f32, f32::min);
        assert!((deepest + tallest).abs() < 0.1, "engraved to {deepest} mm against {tallest}");
        assert_eq!(runs(&sunk, |rise| rise < deepest / 2.0).len(), expected);

        // A pointer that covers the whole drag in one move lays the whole
        // row, and not one stamp of it.
        let mut jumped = app();
        set(&mut jumped, StudioMessage::StampToggled(true));
        set(&mut jumped, StudioMessage::StampAlongChanged(1.5));
        update(&mut jumped, Message::BrushRadiusChanged(1.0));
        press(&mut jumped);
        drag_by(&mut jumped, drag);
        // Each stamp of a row is a gesture begun and ended: with the
        // button still down there is none open, for an Escape to take the
        // last stamp back out of the middle of.
        assert!(moved_voxels(&jumped) > 0 && !jumped.studio.gesture.is_active());
        release(&mut jumped);
        let jumped_found =
            runs(&profile(&jumped, ACROSS, length + 4.0), |rise| rise > tallest / 2.0);
        assert_eq!(jumped_found.len(), expected, "{jumped_found:?}");

        // A row can be led, as a stroke can: on a lead of 60 pixels it is
        // the row of a drag 60 pixels shorter.
        let lead = 60.0;
        let led = row(drag, ACROSS, |app| set(app, StudioMessage::LazyChanged(lead)));
        let led_length = (on_the_ball(drag - lead, ACROSS) - on_the_ball(0.0, ACROSS)).length();
        let led_found = runs(&profile(&led, ACROSS, length + 4.0), |rise| rise > tallest / 2.0);
        assert!(
            (0.15..0.85).contains(&(led_length / step).fract()),
            "the led fixture is {led_length} mm: too near a whole number of steps"
        );
        assert_eq!(led_found.len(), (led_length / step).floor() as usize, "{led_found:?}");
        assert!(led_found.len() < expected, "the lead made no difference: no test of it");

        // A second press off the model does not carry the row on from
        // where the first left it.
        let mut again = row(drag, ACROSS, |_| {});
        let before = moved_voxels(&again);
        let corner = iced::Vector::new(3.0, 3.0);
        update(
            &mut again,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: corner,
                size: SIZE,
            }),
        );
        drag_by(&mut again, 0.0);
        release(&mut again);
        assert_eq!(moved_voxels(&again), before, "a press off the model went on with the row");

        // One undo takes the whole row off.
        let mut undone = laid;
        update(&mut undone, Message::Undo);
        assert_eq!(moved_voxels(&undone), 0, "a row was not one undo");

        // A press with no drag, or a drag shorter than a step, lays nothing
        // and leaves nothing to undo.
        for short in [0.0, 20.0] {
            let none = row(short, ACROSS, |_| {});
            assert_eq!(moved_voxels(&none), 0, "a drag of {short} pixels laid a stamp");
            assert!(!none.unsaved, "a row of nothing dirtied the document");
        }
    }

    /// A long step on a curved surface still lands on it. The straight
    /// line from one stamp toward the pointer cuts under a ball, and a
    /// stamp pressed from under the surface is not the stamp it should be:
    /// the second of a row a long way from the first is as tall as the
    /// first.
    #[test]
    fn a_long_step_of_a_row_lands_back_on_the_surface() {
        let drag = 240.0;
        let length = (on_the_ball(drag, ACROSS) - on_the_ball(0.0, ACROSS)).length();
        let laid = row(drag, ACROSS, |app| {
            update(app, Message::BrushRadiusChanged(2.0));
            set(app, StudioMessage::StampAlongChanged(3.0));
        });
        let risen = profile(&laid, ACROSS, length + 6.0);
        let first = risen[..20].iter().copied().fold(0.0_f32, f32::max);
        let found = runs(&risen, |rise| rise > first / 2.0);
        assert!(found.len() >= 2, "a drag of {length} mm laid {found:?}");
        let (began, ended) = found[1];
        let second = risen[(began * 10.0) as usize..(ended * 10.0) as usize]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        assert!((second / first - 1.0).abs() < 0.1, "{second} mm against {first}");
        // A step of 12 mm in a straight line is 12.08 mm round a ball of
        // 30, which is how the profile measures.
        let middle = (began + ended) / 2.0;
        assert!((middle - 12.08).abs() < 0.3, "the second stamp is {middle} mm along");
        let (first_run, second_run) = (found[0], found[1]);
        // The first is cut in half by where the profile begins.
        assert!(
            ((second_run.1 - second_run.0) / (2.0 * (first_run.1 - first_run.0)) - 1.0).abs()
                < 0.15,
            "{found:?}"
        );
    }

    /// The stamps of a row face the way the row is going. A bar, which is
    /// longer one way than the other, is the same length along a row
    /// dragged up the view as along one dragged across it.
    #[test]
    fn the_stamps_of_a_row_face_along_it() {
        let bars = |way: Way| {
            let laid = row(150.0, way, |app| set(app, StudioMessage::StampChosen(BuiltIn::Bar)));
            let risen = profile(&laid, way, 14.0);
            let tallest = risen.iter().copied().fold(0.0_f32, f32::max);
            let found = runs(&risen, |rise| rise > tallest / 2.0);
            assert!(found.len() >= 3, "{found:?}");
            // The second: the first is cut in half by where the profile begins.
            found[1].1 - found[1].0
        };
        let (across, up) = (bars(ACROSS), bars(UP_THE_VIEW));
        assert!(
            (across - up).abs() < 0.25,
            "{across} mm along one row and {up} mm along the other"
        );

        // Control: the bar is longer one way than the other, so a row that
        // ignored its direction would have shown here. A bar pressed once,
        // by a drag across, measured along the drag and across it.
        let one = stamped(&[30.0], |app| set(app, StudioMessage::StampChosen(BuiltIn::Bar)));
        let length_of = |way: Way| {
            let risen = profile(&one, way, 4.0);
            let tallest = risen.iter().copied().fold(0.0_f32, f32::max);
            runs(&risen, |rise| rise > tallest / 2.0).first().map_or(0.0, |run| run.1 - run.0)
        };
        let (along, athwart) = (length_of(ACROSS), length_of(UP_THE_VIEW));
        assert!(
            (along - athwart).abs() > 0.5,
            "a bar is {along} mm one way and {athwart} mm the other: no test of which way it faces"
        );
    }

    /// The row's spacing is kept in range, and nothing or less is no row.
    #[test]
    fn a_rows_spacing_is_clamped_and_nothing_is_one_stamp() {
        let mut app = app();
        for (asked, taken) in
            [(1.5, 1.5), (99.0, 4.0), (0.1, 0.5), (0.0, 0.0), (-2.0, 0.0), (f32::NAN, 0.0)]
        {
            set(&mut app, StudioMessage::StampAlongChanged(asked));
            assert_eq!(app.studio.stamp_along, taken, "asked for {asked}");
        }
        set(&mut app, StudioMessage::StampToggled(true));
        for along in [0.0, 1.5] {
            set(&mut app, StudioMessage::StampAlongChanged(along));
            drop(app.studio_panel());
        }

        // Put away again, a drag is the one stamp it always was.
        let single = stamped(&[30.0], |_| {});
        let back = stamped(&[30.0], |app| {
            set(app, StudioMessage::StampAlongChanged(1.5));
            set(app, StudioMessage::StampAlongChanged(0.0));
        });
        assert!(same_field(&single, &back));
    }

    /// Where on the palette the brush of this name is.
    fn preset_called(app: &Brokkr, name: &str) -> usize {
        app.studio
            .presets
            .palette()
            .iter()
            .position(|(preset, _)| preset.name == name)
            .unwrap_or_else(|| panic!("there is no brush called {name}"))
    }

    fn choose(app: &mut Brokkr, name: &str) {
        let index = preset_called(app, name);
        set(app, StudioMessage::PresetChosen(index));
    }

    /// The brush of the user's own that has this name.
    fn kept<'a>(app: &'a Brokkr, name: &str) -> &'a Preset {
        let held = app.studio.presets.list.iter().find(|preset| preset.name == name);
        held.unwrap_or_else(|| panic!("no brush was kept as {name}"))
    }

    /// Which brush on the palette is lit.
    fn lit(app: &Brokkr) -> Option<String> {
        let now = app.preset_now(String::new());
        let palette = app.studio.presets.palette();
        Brokkr::lit(&palette, &now).map(|index| palette[index].0.name.clone())
    }

    /// Choosing a brush sets up the whole brush at once, the classic
    /// brush's half and the studio block's, and leaves nothing behind of
    /// the brush before it.
    #[test]
    fn a_brush_from_the_palette_sets_up_the_whole_brush_at_once() {
        let mut app = app();
        // Everything a brush sets, set to something no brush that comes
        // with the application has: one that left a setting alone would
        // otherwise leave it at the value it was going to set anyway.
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Wide));
        set(&mut app, StudioMessage::PlateauChanged(0.3));
        set(&mut app, StudioMessage::StampChosen(BuiltIn::Star));
        set(&mut app, StudioMessage::StampHeightChanged(0.9));
        set(&mut app, StudioMessage::StampAlongChanged(3.0));
        choose(&mut app, "Fine crease");
        assert_eq!(app.brush.kind, BrushKind::Draw);
        assert_eq!(app.brush.falloff, brokkr_core::FalloffCurve::Smooth);
        assert_eq!((app.brush.radius, app.brush.strength), (1.5, 0.3));
        assert_eq!(app.studio.custom, Some(Actions::CREASE));
        assert_eq!(app.studio.plateau, 0.0);
        assert!(!app.studio.stamp_on);
        assert_eq!(app.studio.lazy_px, 12.0);
        assert_eq!(app.status, "brush: Fine crease");

        choose(&mut app, "Rivets");
        assert!(app.studio.stamp_on);
        assert_eq!(app.studio.built_in, Some(BuiltIn::Rivet));
        assert_eq!(app.studio.alpha_name, "Rivet");
        assert_eq!(app.studio.stamp_height, 0.35);
        assert_eq!(app.studio.stamp_along, 0.0, "one rivet came up as a row of them");
        assert_eq!(app.studio.custom, None, "the crease stayed up under the stamp");
        assert_eq!(app.studio.lazy_px, 0.0, "the crease's lazy stroke stayed on");
        assert_eq!(app.brush.radius, 1.5, "a brush that says nothing of its size changed it");

        choose(&mut app, "Rivet row");
        assert!(app.studio.stamp_on);
        assert_eq!((app.brush.radius, app.studio.stamp_along), (1.0, 1.5));

        choose(&mut app, "Band");
        assert!(!app.studio.stamp_on, "the stamp stayed up under a brush on the field");
        assert_eq!((app.brush.kind, app.studio.custom), (BrushKind::Draw, None));
        assert_eq!((app.studio.round, app.studio.plateau), (false, 0.6));
        choose(&mut app, "Bead");
        assert_eq!((app.studio.round, app.studio.plateau), (true, 0.0));
        // A brush that says nothing of its strength takes up what its
        // family was last used at, which for Draw is what Fine crease set.
        assert_eq!(app.brush.strength, 0.3);
        choose(&mut app, "Smooth");
        assert_eq!(app.brush.strength, BrushKind::Smooth.default_strength());
        assert!(!app.studio.round, "the ball's cross-section stayed on under Smooth");

        // Every brush, once taken, is the brush in hand, and is the one
        // that is lit: this is every setting at once.
        let palette: Vec<Preset> =
            app.studio.presets.palette().iter().map(|(preset, _)| (*preset).clone()).collect();
        for (index, preset) in palette.iter().enumerate() {
            set(&mut app, StudioMessage::PresetChosen(index));
            assert!(preset.is(&app.preset_now(String::new())), "{}", preset.name);
            assert_eq!(lit(&app).as_deref(), Some(preset.name.as_str()));
            assert_eq!(app.live_brush_name(), preset.name.to_uppercase());
            drop(app.palette_panel());
            drop(app.studio_panel());
            drop(app.profile_row());
        }

        // A brush that is not there is nothing, and not a panic.
        let before = app.preset_now(String::new());
        set(&mut app, StudioMessage::PresetChosen(palette.len()));
        assert_eq!(app.preset_now(String::new()), before);
    }

    /// Upstream's six on the palette are upstream's six: chosen there or
    /// by upstream's own button, the brush in hand is the same brush, the
    /// same button is lit, and a stroke leaves the same field to the bit.
    #[test]
    fn a_classic_brush_on_the_palette_is_the_classic_brush() {
        for kind in BrushKind::ALL {
            if Actions::of(kind).is_none() {
                continue;
            }
            let mut by_button = app();
            update(&mut by_button, Message::BrushKindChanged(kind));
            let mut by_palette = app();
            choose(&mut by_palette, kind.label());
            assert_eq!(by_palette.brush, by_button.brush, "{kind:?}");
            assert_eq!(lit(&by_button).as_deref(), Some(kind.label()), "{kind:?}");
            assert_eq!(by_button.live_brush_name(), kind.label().to_uppercase());
            // Smooth and Flatten on a ball nobody has touched have little
            // to do: give all of them a ridge to work on.
            for app in [&mut by_button, &mut by_palette] {
                let chosen = app.brush.kind;
                update(app, Message::BrushKindChanged(BrushKind::Draw));
                stroke(app);
                update(app, Message::BrushKindChanged(chosen));
            }
            choose(&mut by_palette, kind.label());
            let mut ridge = app();
            stroke(&mut ridge);
            stroke(&mut by_button);
            stroke(&mut by_palette);
            assert!(same_field(&by_button, &by_palette), "{kind:?} is not the same stroke");
            // Control: the stroke did something, so "the same" is not
            // "neither did anything".
            assert!(!same_field(&by_button, &ridge), "{kind:?} did nothing to compare");
        }
    }

    /// Every brush that comes with the application does something to the
    /// model when it is dragged across it, and what it did is one undo.
    #[test]
    fn every_brush_on_the_palette_does_something_and_is_one_undo() {
        let names: Vec<String> =
            super::super::presets::core().iter().map(|preset| preset.name.clone()).collect();
        for name in names {
            // A ridge across the middle, for the brushes that take
            // material off or level it to have something to work on.
            let mut app = app();
            stroke(&mut app);
            let mut ridge = self::app();
            stroke(&mut ridge);
            choose(&mut app, &name);
            // A long enough drag for the lazy brushes to outrun their
            // lead and for a row of stamps to lay one.
            press(&mut app);
            for step in 1..=12 {
                drag_by(&mut app, 10.0 * step as f32);
            }
            release(&mut app);
            assert!(!same_field(&app, &ridge), "{name} did nothing to the model");
            update(&mut app, Message::Undo);
            assert!(same_field(&app, &ridge), "{name} was not one undo");
        }
    }

    /// What a brush is made of is what its stroke does: a push in cuts,
    /// a scraper takes off what stands proud and nothing else, and a brush
    /// goes by the classic brush its amounts are nearest, keeping the
    /// strength it had.
    #[test]
    fn what_a_brush_is_made_of_is_what_its_stroke_does() {
        let nothing = Actions::default();
        let stroked = |actions: Actions| {
            let mut app = app();
            set(&mut app, StudioMessage::ActionsChosen(actions));
            stroke(&mut app);
            shift_under_the_stroke(&app)
        };
        let (out, half, cut) = (
            stroked(Actions::DRAW),
            stroked(Actions { push: 0.5, ..nothing }),
            stroked(Actions { push: -1.0, ..nothing }),
        );
        assert!(out > 0.05 && cut < -0.05, "a push out moved {out} mm and a push in {cut} mm");
        assert!(half > 0.02 && half < out * 0.75, "half a push moved {half} mm against {out} mm");
        // Against a ball nobody has touched and not against nothing: the
        // probe is a point in space, and the lattice's ball is not exactly
        // on it.
        let untouched = shift_under_the_stroke(&app());
        assert_eq!(stroked(nothing), untouched, "a brush of nothing moved the surface");

        // A brush that only cuts, with its plane through the surface under
        // the pointer, has nothing to take off a ball nobody has touched:
        // nothing stands proud of a plane that touches a ball. To a micron
        // and not to the bit, because the press lands a hair off the
        // surface and the plane through it shaves that hair.
        let cut_only = Actions { level: 1.0, side: Side::Cut, ..nothing };
        let shaved = stroked(cut_only) - untouched;
        assert!(shaved.abs() < 0.001, "it moved a ball with nothing proud by {shaved} mm");
        // Which is why Scrape holds its plane a little under the surface,
        // as Clay holds its a little proud. (With the plane at the surface
        // it did nothing to a ridge either, run along it or beside it:
        // the top of a ridge is the plane, and beside one a ball falls
        // away from the plane faster than a low ridge rises to it. This
        // test found both.) Run along a ridge it takes the ridge off.
        let scrape = super::super::presets::core()
            .iter()
            .find(|preset| preset.name == "Scrape")
            .and_then(|preset| preset.custom)
            .expect("Scrape comes with the application, with amounts of its own");
        assert!(scrape.plane < 0.0 && scrape.side == Side::Cut, "{scrape:?}");
        let worked = |actions: Actions| {
            let mut app = app();
            stroke(&mut app);
            let ridge = shift_under_the_stroke(&app);
            set(&mut app, StudioMessage::ActionsChosen(actions));
            update(&mut app, Message::BrushStrengthChanged(0.8));
            for _ in 0..3 {
                stroke(&mut app);
            }
            (ridge, shift_under_the_stroke(&app))
        };
        let (ridge, scraped) = worked(scrape);
        assert!(scraped < ridge * 0.25, "a scraper took a ridge of {ridge} mm to {scraped} mm");
        // The same plane filled up to and not cut down to leaves it be.
        let (ridge, filled) = worked(Actions { side: Side::Fill, ..scrape });
        assert!(filled >= ridge - 0.001, "a brush that only fills took {ridge} mm to {filled} mm");

        // The family, and the strength carried across it.
        let mut app = self::app();
        update(&mut app, Message::BrushStrengthChanged(0.33));
        set(&mut app, StudioMessage::ActionsChosen(Actions { smooth: 0.5, ..nothing }));
        assert_eq!(app.brush.kind, BrushKind::Smooth);
        assert_eq!(app.brush.strength, 0.33, "the brush took up Smooth's own strength");
        assert_eq!(app.live_brush_name(), "SMOOTH, CHANGED");
        // A brush of Smooth's family has no opposite, as Smooth has none.
        app.control = true;
        assert_eq!(app.stroke_direction(), BrushDirection::Add);
        set(&mut app, StudioMessage::ActionsChosen(Actions { push: 0.5, smooth: 0.5, ..nothing }));
        assert_eq!(app.brush.kind, BrushKind::Draw);
        assert_eq!(app.stroke_direction(), BrushDirection::Subtract);
        assert_eq!(app.brush.strength, 0.33);
        // Amounts out of range are brought into it on the way in.
        set(
            &mut app,
            StudioMessage::ActionsChosen(Actions { push: 9.0, pinch: f32::NAN, ..nothing }),
        );
        assert_eq!((app.brush.kind, app.studio.custom), (BrushKind::Draw, None));
    }

    /// How far the surface has moved from the fresh ball's at a point
    /// `above` millimetres up the viewport from the middle of the stroke.
    fn shift_beside_the_stroke(app: &Brokkr, above: f32) -> f32 {
        let fresh = self::app();
        let toward_camera = (fresh.camera.eye() - Vec3::ZERO).normalize();
        let up = fresh.camera.up().normalize();
        let on_the_surface = (toward_camera * super::super::MODEL_RADIUS_MM + up * above)
            .normalize()
            * super::super::MODEL_RADIUS_MM;
        let volume = app.doc.active_volume();
        -volume.sample_world(on_the_surface) * volume.voxel_size()
    }

    /// The profile is the cross-section of the stroke: with a flat middle
    /// the stroke is as tall most of the way to its rim as it is at its
    /// middle, where the classic brush has fallen well away; the ball's
    /// cross-section is a setting beside upstream's four curves and is put
    /// away by choosing one of them; and a classic brush chosen has a
    /// classic profile.
    #[test]
    fn the_profile_is_the_cross_section_of_the_stroke() {
        let section = |configure: &dyn Fn(&mut Brokkr)| {
            let mut app = app();
            update(&mut app, Message::BrushRadiusChanged(4.0));
            configure(&mut app);
            stroke(&mut app);
            (shift_under_the_stroke(&app), shift_beside_the_stroke(&app, 2.0))
        };
        // The probe beside the stroke is half the brush's radius from the
        // press, and the stroke runs on from the press, so even a stroke
        // flat right across is a little lower there: the later stamps are
        // further from it than from the middle. Measured: the classic
        // brush is 0.46 as tall there as at its middle, and with seven
        // tenths of it flat, 0.83.
        let (middle, beside) = section(&|_| {});
        assert!(middle > 0.05, "the classic Draw control raised only {middle} mm");
        assert!(
            beside < middle * 0.6,
            "the classic brush had not fallen away at the probe: {beside} mm against {middle} mm"
        );
        let (flat_middle, flat_beside) =
            section(&|app| set(app, StudioMessage::PlateauChanged(0.7)));
        assert!(
            flat_beside > flat_middle * 0.75,
            "half way out a stroke with a flat middle rose {flat_beside} mm against {flat_middle}"
        );
        let (_, round_beside) = section(&|app| set(app, StudioMessage::RoundChosen));
        assert!(round_beside > beside, "a ball's cross-section is fuller than the smooth curve's");

        let mut app = app();
        set(&mut app, StudioMessage::RoundChosen);
        assert_eq!(app.studio.profile(app.brush.falloff).shape, Shape::Round);
        assert_eq!(lit(&app).as_deref(), Some("Bead"));
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Sharp));
        assert_eq!(app.studio.profile(app.brush.falloff).shape, Shape::Sharp);
        set(&mut app, StudioMessage::RoundChosen);
        set(&mut app, StudioMessage::PlateauChanged(0.5));
        update(&mut app, Message::BrushKindChanged(BrushKind::Clay));
        assert_eq!(
            app.studio.profile(app.brush.falloff),
            Profile::of(brokkr_core::FalloffCurve::Sharp),
            "a classic brush chosen kept a profile no classic brush has"
        );
    }

    /// The brush in hand is kept under the name typed, comes back as it
    /// was, is replaced by the same name used again, and can be forgotten.
    #[test]
    fn the_brush_in_hand_is_kept_under_a_name_and_comes_back() {
        let mut app = app();
        assert_eq!(app.studio.presets, Presets::default(), "the fixture starts with brushes kept");
        let came = app.studio.presets.palette().len();
        update(&mut app, Message::BrushRadiusChanged(4.5));
        update(&mut app, Message::BrushStrengthChanged(0.4));
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Sharp));
        let puff = Actions { inflate: 0.75, smooth: 0.25, ..Actions::default() };
        set(&mut app, StudioMessage::ActionsChosen(puff));
        set(&mut app, StudioMessage::PlateauChanged(0.2));
        set(&mut app, StudioMessage::LazyChanged(25.0));
        set(&mut app, StudioMessage::SizeFromPressureChanged(0.7));
        assert_eq!(lit(&app), None, "a brush nobody kept is lit on the palette");

        // With no name there is nothing to keep it under.
        set(&mut app, StudioMessage::PresetSaved);
        assert_eq!(app.status, "give the brush a name first");
        set(&mut app, StudioMessage::PresetNamed(" = # ".to_string()));
        set(&mut app, StudioMessage::PresetSaved);
        assert_eq!(app.studio.presets.list.len(), 0, "a brush was kept with no name");

        // What is typed is kept as typed, up to a length: a held key does
        // not grow the box for ever.
        set(&mut app, StudioMessage::PresetNamed("x".repeat(1000)));
        assert_eq!(app.studio.preset_name.len(), 4 * super::super::presets::LONGEST_NAME);
        set(&mut app, StudioMessage::PresetNamed("  Puff = up \n".to_string()));
        drop(app.palette_panel());
        set(&mut app, StudioMessage::PresetSaved);
        assert_eq!(app.studio.presets.list.len(), 1);
        assert_eq!(app.status, "brush kept: Puff up");
        assert_eq!(app.studio.preset_name, "", "the name was left in the box");
        // It is on the palette after the ones that came, and is lit.
        assert_eq!(preset_called(&app, "Puff up"), came);
        assert_eq!(lit(&app).as_deref(), Some("Puff up"));
        assert_eq!(app.live_brush_name(), "PUFF UP");

        // Away to another brush altogether, and back.
        choose(&mut app, "Rivets");
        assert!(app.studio.stamp_on && app.studio.custom.is_none());
        choose(&mut app, "Puff up");
        assert_eq!((app.brush.kind, app.studio.custom), (BrushKind::Inflate, Some(puff)));
        assert_eq!((app.brush.radius, app.brush.strength), (4.5, 0.4));
        assert_eq!(app.brush.falloff, brokkr_core::FalloffCurve::Sharp);
        assert_eq!(app.studio.plateau, 0.2);
        assert_eq!((app.studio.lazy_px, app.studio.size_from_pressure), (25.0, 0.7));
        assert!(!app.studio.stamp_on);

        // The same name again is the same brush, changed.
        update(&mut app, Message::BrushRadiusChanged(2.0));
        set(&mut app, StudioMessage::PresetNamed("Puff up".to_string()));
        set(&mut app, StudioMessage::PresetSaved);
        assert_eq!(app.studio.presets.list.len(), 1);
        assert_eq!(kept(&app, "Puff up").size_mm, Some(2.0));

        // Forgotten, and only it.
        set(&mut app, StudioMessage::PresetRemoved(came));
        assert_eq!(app.status, "brush forgotten: Puff up");
        assert_eq!(app.studio.presets, Presets::default());
        set(&mut app, StudioMessage::PresetRemoved(came));
        assert_eq!(app.studio.presets, Presets::default());
        assert_eq!(app.studio.presets.palette().len(), came, "a brush that came was removed");

        // There is room for only so many, and the one too many is refused
        // out loud.
        for index in 0..super::super::presets::MAX_PRESETS + 1 {
            set(&mut app, StudioMessage::PresetNamed(format!("p{index}")));
            set(&mut app, StudioMessage::PresetSaved);
        }
        assert_eq!(app.studio.presets.list.len(), super::super::presets::MAX_PRESETS);
        assert_eq!(app.status, "no room for another brush: forget one first");
        drop(app.palette_panel());
    }

    /// A brush that came with the application cannot be forgotten. One
    /// kept under its name stands where it stood, is marked as the user's,
    /// and forgetting that puts the first back.
    #[test]
    fn a_brush_that_came_can_be_changed_and_put_back_and_not_forgotten() {
        let mut app = app();
        let came = app.studio.presets.palette().len();
        let clay = preset_called(&app, "Clay");
        set(&mut app, StudioMessage::PresetRemoved(clay));
        assert_eq!(app.status, "Clay comes with the application, and stays");
        assert_eq!(app.studio.presets.palette().len(), came);

        // Clay as this user likes it: bigger, harder, and led.
        choose(&mut app, "Clay");
        update(&mut app, Message::BrushRadiusChanged(5.0));
        update(&mut app, Message::BrushStrengthChanged(0.5));
        set(&mut app, StudioMessage::LazyChanged(10.0));
        set(&mut app, StudioMessage::PresetNamed("Clay".to_string()));
        set(&mut app, StudioMessage::PresetSaved);
        let palette = app.studio.presets.palette();
        assert_eq!(palette.len(), came, "the changed brush is on the palette twice");
        assert_eq!(palette[clay].1, Place::Changed);
        assert_eq!(palette[clay].0, kept(&app, "Clay"));
        drop(palette);

        choose(&mut app, "Draw");
        choose(&mut app, "Clay");
        assert_eq!((app.brush.radius, app.brush.strength, app.studio.lazy_px), (5.0, 0.5, 10.0));
        // Upstream's own button is still upstream's Clay, at the strength
        // Clay was last used at and with nothing of the studio's.
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        update(&mut app, Message::BrushKindChanged(BrushKind::Clay));
        assert_eq!(app.brush.kind, BrushKind::Clay);
        drop(app.palette_panel());

        choose(&mut app, "Clay");
        set(&mut app, StudioMessage::PresetRemoved(clay));
        assert_eq!(app.status, "Clay is back as it came");
        assert_eq!(app.studio.presets, Presets::default());
        let palette = app.studio.presets.palette();
        assert_eq!((palette.len(), palette[clay].1), (came, Place::Core));
    }

    /// With the mask tool up the strength in hand is the mask's. A brush
    /// taken then does not change it, and a brush kept then does not keep
    /// it: both mean the brush's own.
    #[test]
    fn a_brush_and_the_mask_tool_keep_their_strengths_apart() {
        let mut app = app();
        update(&mut app, Message::ToolChanged(super::super::Tool::Mask));
        let masks = app.brush.strength;
        assert_ne!(masks, 0.3, "the fixture cannot tell the two strengths apart");
        choose(&mut app, "Fine crease");
        assert_eq!(app.brush.strength, masks, "the brush's strength became the mask's");

        set(&mut app, StudioMessage::PresetNamed("Kept under the mask".to_string()));
        set(&mut app, StudioMessage::PresetSaved);
        let held = kept(&app, "Kept under the mask");
        assert_eq!((held.kind, held.strength), (BrushKind::Draw, Some(0.3)));

        // Changing what the brush does under the mask leaves the mask's
        // strength the mask's too, and carries the brush's own with it.
        set(
            &mut app,
            StudioMessage::ActionsChosen(Actions { smooth: 1.0, level: 0.5, ..Actions::default() }),
        );
        assert_eq!(app.brush.kind, BrushKind::Flatten);
        assert_eq!(app.brush.strength, masks);

        // The mask put down, the brush is the one in hand, strength and all.
        update(&mut app, Message::ToolChanged(super::super::Tool::Mask));
        assert_eq!(app.brush.kind, BrushKind::Flatten);
        assert_eq!(app.brush.strength, 0.3);

        // A strength typed into the menu's field goes to the brush in
        // hand, and to its slot only when the brush is next changed. A
        // brush kept before then keeps the one in hand.
        app.edit_menu_field(super::super::SizingTarget::Strength, "0.5".to_string());
        assert_eq!(app.brush.strength, 0.5);
        set(&mut app, StudioMessage::PresetNamed("Typed".to_string()));
        set(&mut app, StudioMessage::PresetSaved);
        assert_eq!(kept(&app, "Typed").strength, Some(0.5));
    }

    /// A brush bigger than this document's brush may be is brought down
    /// to what it may be, and a stamp with a picture of the user's own
    /// takes up the stamp with whichever picture is in hand.
    #[test]
    fn a_brush_is_fitted_to_the_document_and_the_picture_in_hand() {
        let mut app = app();
        assert!(app.studio.presets.put(Preset { size_mm: Some(100.0), ..Preset::plain("Huge") }));
        assert!(app.max_radius() < 100.0, "the fixture's brush may be as big as the preset");
        choose(&mut app, "Huge");
        assert_eq!(app.brush.radius, app.max_radius());

        let mine = LoadedStamp {
            name: "mine.png".to_string(),
            alpha: Arc::new(Alpha::built_in(BuiltIn::Star)),
        };
        set(&mut app, StudioMessage::StampImageLoaded(Some(Ok(mine))));
        set(&mut app, StudioMessage::StampToggled(true));
        set(&mut app, StudioMessage::StampHeightChanged(0.8));
        set(&mut app, StudioMessage::PresetNamed("Mine".to_string()));
        set(&mut app, StudioMessage::PresetSaved);
        assert_eq!(kept(&app, "Mine").stamp, Some((StampOf::InHand, 0.8)));

        choose(&mut app, "Huge");
        assert!(!app.studio.stamp_on);

        // A row is a row of stamps. With the stamp put away, the spacing
        // it was last given is not part of the brush in hand.
        set(&mut app, StudioMessage::StampAlongChanged(2.0));
        set(&mut app, StudioMessage::PresetNamed("No stamp".to_string()));
        set(&mut app, StudioMessage::PresetSaved);
        assert_eq!(kept(&app, "No stamp").along, 0.0);
        set(&mut app, StudioMessage::StampAlongChanged(0.0));

        choose(&mut app, "Mine");
        assert!(app.studio.stamp_on);
        assert_eq!(app.studio.stamp_height, 0.8);
        assert_eq!(app.studio.alpha_name, "mine.png", "the picture in hand was put down");
        assert_eq!(app.studio.built_in, None);
    }

    /// A drag from the middle of the view out to `pixels` and back, `times`
    /// over, in one stroke: a hand scrubbing.
    fn scrub(app: &mut Brokkr, pixels: f32, times: usize) {
        press(app);
        for _ in 0..times {
            for step in 1..=8 {
                drag_by(app, pixels * step as f32 / 8.0);
            }
            for step in (0..8).rev() {
                drag_by(app, pixels * step as f32 / 8.0);
            }
        }
        release(app);
    }

    /// With build-up off a stroke lays one layer: as tall as the strength
    /// and the size say, no taller for being scrubbed, and one undo. The
    /// same scrubbing with build-up on is taller for every pass.
    #[test]
    fn a_brush_with_build_up_off_lays_one_layer_however_it_is_scrubbed() {
        let scrubbed = |name: &str, strength: f32, times: usize| {
            let mut app = app();
            choose(&mut app, name);
            update(&mut app, Message::BrushRadiusChanged(3.0));
            update(&mut app, Message::BrushStrengthChanged(strength));
            scrub(&mut app, 60.0, times);
            app
        };
        let (once, thrice) = (scrubbed("Layer", 0.4, 1), scrubbed("Layer", 0.4, 3));
        assert!(!once.studio.build_up && once.lays_a_layer());
        let (tall, again) = (height_under_the_press(&once), height_under_the_press(&thrice));
        // Strength times the radius times the half of it a layer may be.
        let expected = 0.4 * 3.0 * brokkr_core::brush::compound::LAYER_DEPTH;
        assert!((tall - expected).abs() < 0.1, "a layer is {tall} mm tall and not {expected}");
        assert!((again - tall).abs() <= 0.02, "scrubbed three times it is {again} mm, not {tall}");
        // Control: the same brush building up is taller each time.
        let (built, more) = (scrubbed("Draw", 0.4, 1), scrubbed("Draw", 0.4, 3));
        let (first, third) = (height_under_the_press(&built), height_under_the_press(&more));
        assert!(third > first + 0.3, "build-up scrubbed: {first} mm, then {third} mm");

        // Clay that does not build on itself is a strip of one thickness.
        let (strip, strips) = (scrubbed("Clay strips", 0.3, 1), scrubbed("Clay strips", 0.3, 3));
        let (thick, still) = (height_under_the_press(&strip), height_under_the_press(&strips));
        assert!(thick > 0.5, "a strip of clay is only {thick} mm thick");
        assert!(
            (still - thick).abs() <= 0.04,
            "scrubbed three times it is {still} mm, not {thick}"
        );
        let (clay, clays) = (scrubbed("Clay", 0.3, 1), scrubbed("Clay", 0.3, 3));
        assert!(
            height_under_the_press(&clays) > height_under_the_press(&clay) + 0.3,
            "clay did not build on itself"
        );

        // One undo, and shift still smooths with a classic Smooth.
        let mut app = scrubbed("Layer", 0.4, 2);
        update(&mut app, Message::Undo);
        assert!(same_field(&app, &self::app()), "a layer was not one undo");
        app.shift = true;
        assert!(!app.lays_a_layer(), "Smooth under shift was taken for a layer");
        stroke(&mut app);
        app.shift = false;

        // Upstream's own button is a brush that builds up.
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        assert!(app.studio.build_up && !app.lays_a_layer());
    }

    /// A stroke that lays one layer is aimed at the surface as it was
    /// before the stroke: along the pointer's ray, under what the stroke
    /// has built, and facing the way that surface faced.
    #[test]
    fn a_layer_is_aimed_at_the_surface_as_it_was() {
        let mut app = app();
        let middle = Vec2::new(centre().x, centre().y);
        // Where the ray lands on a ball nobody has touched. (Asked of an
        // application that has had a pointer event, and so knows how big
        // its viewport is: asked of this one, yet to have one, the answer
        // was 20 mm away.)
        let landed = on_the_ball(0.0, ACROSS);
        // A brush that builds up is aimed where the ray lands.
        assert_eq!(app.layer_aim(middle, landed), landed);

        choose(&mut app, "Layer");
        update(&mut app, Message::BrushStrengthChanged(0.6));
        press(&mut app);
        drag_by(&mut app, 30.0);
        // Mid stroke: the ray lands on top of the layer, most of a
        // millimetre nearer the camera than the ball was.
        let on_top = app.surface_under(middle).expect("the layer is under the pointer");
        let risen = (on_top - landed).length();
        assert!(risen > 0.5, "the fixture's layer is only {risen} mm tall");
        let aimed = app.layer_aim(middle, on_top);
        assert!((aimed - landed).length() < 0.03, "aimed {} mm off", (aimed - landed).length());
        let volume = app.doc.active_volume();
        assert!(volume.sample_world_before(aimed).abs() < 0.02);
        let faces = app.stamp_facing(aimed);
        assert!(faces.dot(landed.normalize()) > 0.999, "it faces {faces}");
        release(&mut app);
        // The stroke over, the surface as it was is the surface as it is.
        assert_eq!(app.layer_aim(middle, on_top), on_top);
    }

    /// A brush set down in dabs lays separate marks a width or so apart,
    /// and with a picture across its tip each mark is the picture.
    #[test]
    fn a_brush_set_down_in_dabs_lays_separate_marks() {
        let dragged = |configure: &dyn Fn(&mut Brokkr)| {
            let mut app = app();
            choose(&mut app, "Layer");
            update(&mut app, Message::BrushRadiusChanged(1.0));
            update(&mut app, Message::BrushStrengthChanged(0.6));
            configure(&mut app);
            press(&mut app);
            for part in 1..=6 {
                drag_by(&mut app, 20.0 * part as f32);
            }
            release(&mut app);
            app
        };
        let length = (on_the_ball(120.0, ACROSS) - on_the_ball(0.0, ACROSS)).length();
        let marks =
            |app: &Brokkr| runs(&profile(app, ACROSS, length + 2.0), |rise| rise > 0.12).len();

        // As a line: one mark, the length of the drag.
        let line = dragged(&|_| {});
        assert_eq!(line.stroke_spacing(), 0.25);
        assert_eq!(marks(&line), 1, "a line is in pieces");

        // Every width and a half: a mark each 3 mm, with gaps between.
        let dabs = dragged(&|app| set(app, StudioMessage::DabsChanged(1.5)));
        assert_eq!(dabs.stroke_spacing(), 3.0);
        let expected = (length / 3.0).floor() as usize + 1;
        assert!(expected >= 3, "the fixture's drag is only {length} mm");
        assert_eq!(marks(&dabs), expected, "over a drag of {length} mm");
        // Shift is Smooth, which is a line whatever is in hand.
        let mut smoothing = dragged(&|app| set(app, StudioMessage::DabsChanged(1.5)));
        smoothing.shift = true;
        assert_eq!(smoothing.stroke_spacing(), 0.25);

        // With a picture across the tip, a dab is the picture: a bar
        // across the stroke is narrower along it than the brush is.
        let wide = |app: &Brokkr| {
            let found = runs(&profile(app, ACROSS, length + 2.0), |rise| rise > 0.12);
            found.iter().map(|(from, to)| to - from).fold(0.0_f32, f32::max)
        };
        let bars = dragged(&|app| {
            set(app, StudioMessage::DabsChanged(1.5));
            set(app, StudioMessage::StampChosen(BuiltIn::Bar));
            set(app, StudioMessage::TipToggled(true));
        });
        assert_eq!(marks(&bars), expected);
        assert!(
            wide(&bars) < wide(&dabs) * 0.7,
            "a bar is {} mm along the stroke and a plain dab {}",
            wide(&bars),
            wide(&dabs)
        );
        // And Stitches on the palette is that brush.
        let mut app = app();
        choose(&mut app, "Stitches");
        assert!(app.studio.tip_on && !app.studio.build_up && !app.studio.stamp_on);
        assert_eq!((app.studio.built_in, app.studio.dabs), (Some(BuiltIn::Bar), 1.0));

        // Dabs are clamped as a row's spacing is, and nothing is a line.
        set(&mut app, StudioMessage::DabsChanged(99.0));
        assert_eq!(app.studio.dabs, *ALONG_RANGE.end());
        set(&mut app, StudioMessage::DabsChanged(0.1));
        assert_eq!(app.studio.dabs, *ALONG_RANGE.start());
        for nothing in [0.0, -1.0, f32::NAN] {
            set(&mut app, StudioMessage::DabsChanged(nothing));
            assert_eq!(app.studio.dabs, 0.0);
        }
    }

    /// A picture across the tip shapes what a stroke does: less of the
    /// surface is moved than with a plain tip, which picture it is matters,
    /// taking it off is the plain brush again, and Smooth under shift does
    /// not have it.
    #[test]
    fn a_picture_across_the_tip_shapes_the_stroke() {
        let stroked = |picture: Option<BuiltIn>| {
            let mut app = app();
            update(&mut app, Message::BrushStrengthChanged(0.6));
            if let Some(picture) = picture {
                set(&mut app, StudioMessage::StampChosen(picture));
                set(&mut app, StudioMessage::TipToggled(true));
            }
            stroke(&mut app);
            app
        };
        let (plain, bar, star) =
            (stroked(None), stroked(Some(BuiltIn::Bar)), stroked(Some(BuiltIn::Star)));
        let (all, some) = (moved_voxels(&plain), moved_voxels(&bar));
        assert!(some > 100 && some < all * 9 / 10, "a bar moved {some} voxels, a plain tip {all}");
        assert!(!same_field(&bar, &star), "which picture is across the tip made no difference");

        let mut app = app();
        update(&mut app, Message::BrushStrengthChanged(0.6));
        set(&mut app, StudioMessage::StampChosen(BuiltIn::Bar));
        set(&mut app, StudioMessage::TipToggled(true));
        assert_eq!(app.live_brush_name(), "DRAW, CHANGED");
        set(&mut app, StudioMessage::TipToggled(false));
        stroke(&mut app);
        assert!(same_field(&app, &plain), "with the picture taken off it is not the plain brush");

        // The stamp and a tip are two uses of the one picture in hand,
        // and taking up either puts the other away.
        set(&mut app, StudioMessage::TipToggled(true));
        set(&mut app, StudioMessage::StampToggled(true));
        assert!(app.studio.stamp_on && !app.studio.tip_on);
        set(&mut app, StudioMessage::TipToggled(true));
        assert!(!app.studio.stamp_on && app.studio.tip_on);
        // And a classic brush chosen has a plain tip.
        update(&mut app, Message::BrushKindChanged(BrushKind::Clay));
        assert!(!app.studio.tip_on);

        // Shift smooths with a plain Smooth, tip or no tip.
        let smoothed = |tipped: bool| {
            let mut app = stroked(None);
            set(&mut app, StudioMessage::TipToggled(tipped));
            app.shift = true;
            stroke(&mut app);
            app
        };
        assert!(same_field(&smoothed(true), &smoothed(false)), "the tip was on Smooth under shift");
        drop(app.studio_panel());
    }

    /// `b` brings the palette of brushes up at the pointer. Choosing a
    /// brush there takes it up and puts the palette away; so does `b`
    /// again, Escape, and a press anywhere else, which is then not a
    /// stroke.
    #[test]
    fn the_palette_comes_to_the_pointer_and_goes_when_it_has_been_used() {
        let toggle = Message::Studio(StudioMessage::PaletteToggled);
        for key in ["b", "B"] {
            let means = crate::viewport::shortcut(key, false, key == "B", false);
            assert!(
                matches!(means, Some(Message::Studio(StudioMessage::PaletteToggled))),
                "{key} means {means:?}"
            );
        }

        let mut app = app();
        assert!(app.palette_popup().is_none());
        // The pointer over the view: the palette is up where it is.
        let at = iced::Vector::new(300.0, 200.0);
        update(&mut app, Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }));
        update(&mut app, toggle.clone());
        assert_eq!(app.studio.popup, Some(Vec2::new(300.0, 200.0)));
        assert!(app.palette_popup().is_some());
        drop(app.view());

        // A brush chosen is the brush in hand, and the palette is away.
        choose(&mut app, "Crease");
        assert_eq!(app.studio.custom, Some(Actions::CREASE));
        assert_eq!(app.studio.popup, None);

        // Up and down again by its own key.
        update(&mut app, toggle.clone());
        update(&mut app, toggle.clone());
        assert_eq!(app.studio.popup, None);

        // Escape.
        update(&mut app, toggle.clone());
        update(&mut app, Message::MenuClosed);
        assert_eq!(app.studio.popup, None);

        // A press anywhere else puts it away and is not a stroke.
        update(&mut app, toggle.clone());
        press(&mut app);
        release(&mut app);
        assert_eq!(app.studio.popup, None);
        assert!(same_field(&app, &self::app()), "the press that put the palette away sculpted");
        // The next press is a stroke again.
        stroke(&mut app);
        assert!(!same_field(&app, &self::app()));

        // With the pointer off the view, on the panel's own button for it,
        // the palette comes up in the middle of the view.
        let off = iced::Vector::new(SIZE.x + 120.0, 40.0);
        update(&mut app, Message::Pointer(PointerEvent::Moved { position: off, size: SIZE }));
        update(&mut app, toggle);
        assert_eq!(app.studio.popup, Some(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0)));

        // Wherever it is asked for, all of it is on the screen.
        for at in
            [Vec2::ZERO, Vec2::new(SIZE.x, SIZE.y), Vec2::new(SIZE.x, 0.0), Vec2::new(400.0, 300.0)]
        {
            let corner = app.popup_corner(at);
            let height = app.popup_height();
            assert!(corner.x >= 0.0 && corner.x + POPUP_WIDTH <= SIZE.x, "{at}: {corner}");
            assert!(corner.y >= 0.0 && corner.y + height <= SIZE.y, "{at}: {corner}");
        }
    }

    /// Brush files taken in are brushes kept: every brush of every file,
    /// a pack's among them, in place of one of the same name, with a file
    /// that is no brush said and not fatal.
    #[test]
    fn brush_files_are_taken_in() {
        let mut app = app();
        let came = app.studio.presets.palette().len();
        let read = vec![
            Ok(("one.brush".to_string(), "Gouge = push:-1, size:2\n".to_string())),
            Ok(("pack.brush".to_string(), "Knurl = tip:Star, dabs:1\nStep = plateau:0.5\n".into())),
            Ok(("notes.brush".to_string(), "nothing to see\n".to_string())),
            Err("gone.brush: no such file".to_string()),
        ];
        set(&mut app, StudioMessage::BrushImported(Some(read)));
        assert_eq!(app.studio.presets.list.len(), 3);
        assert_eq!(app.studio.presets.palette().len(), came + 3);
        assert!(app.status.starts_with("imported 3: Gouge, Knurl, Step; but "), "{}", app.status);
        choose(&mut app, "Gouge");
        assert_eq!(app.studio.custom.map(|actions| actions.push), Some(-1.0));
        assert_eq!(app.brush.radius, 2.0);
        drop(app.palette_panel());

        // The same name again is the same brush, changed, and not a second.
        let again = vec![Ok(("one.brush".to_string(), "Gouge = push:-0.5\n".to_string()))];
        set(&mut app, StudioMessage::BrushImported(Some(again)));
        assert_eq!(app.studio.presets.list.len(), 3);
        assert_eq!(app.status, "imported 1: Gouge");
        choose(&mut app, "Gouge");
        assert_eq!(app.studio.custom.map(|actions| actions.push), Some(-0.5));

        // Nothing picked, and nothing in what was picked.
        let status = app.status.clone();
        set(&mut app, StudioMessage::BrushImported(None));
        assert_eq!(app.status, status);
        set(&mut app, StudioMessage::BrushImported(Some(vec![Err("x: unreadable".to_string())])));
        assert_eq!(app.status, "no brush imported: x: unreadable");
        set(&mut app, StudioMessage::BrushImported(Some(Vec::new())));
        assert_eq!(app.status, "no brush imported");

        // A file is read by its name and its text, and one too big to be
        // a brush is refused before it is read.
        let directory = std::env::temp_dir().join(format!("brokkr-import-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        let good = directory.join("mine.brush");
        std::fs::write(&good, "Mine = brush:Clay\n").expect("writing");
        assert_eq!(read_brush_file(&good), Ok(("mine.brush".into(), "Mine = brush:Clay\n".into())));
        let big = directory.join("big.brush");
        std::fs::write(&big, vec![b'x'; MOST_BRUSH_FILE as usize + 1]).expect("writing");
        assert!(read_brush_file(&big).is_err_and(|why| why.contains("not a brush file")));
        assert!(read_brush_file(&directory.join("nowhere.brush")).is_err());
        std::fs::remove_dir_all(&directory).ok();
    }
}
