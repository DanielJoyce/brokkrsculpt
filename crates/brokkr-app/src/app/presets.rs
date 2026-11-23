// SPDX-License-Identifier: AGPL-3.0-only

//! The brushes: a brush as it is set up, kept under a name.
//!
//! `docs/STUDIO-PLAN.md` section 16. A brush is its settings: what it does
//! to the surface, as amounts of the five things a brush can do
//! ([`Actions`]); the cross-section of its stroke ([`Profile`]); how big
//! and how hard, when the brush says; and how it is led (a lazy stroke,
//! size from pressure) or that it is a stamp. A [`Preset`] is all of that
//! under a name, and the palette on the panel is a list of them.
//!
//! # The brushes that come with the application
//!
//! [`core`]. Draw, Clay, Inflate, Pinch, Smooth and Flatten are upstream's
//! six, each as the settings it is, and the rest cost only their settings:
//! a pinch turned round is Spread, a levelling brush that only cuts is
//! Scrape, a push and a pinch is Crease. They cannot be forgotten. One can
//! be changed, by keeping a brush under its name, and the changed one then
//! stands where it stood; forgetting that puts the first back.
//!
//! # The files
//!
//! A brush a user keeps is one file, `<name>.brush`, in the `brushes`
//! directory of the DATA directory ([`crate::paths::data_file`]): a brush
//! is something its user made and might hand to somebody else, which is
//! what that directory is for, and one file a brush is what makes handing
//! one over a matter of copying a file. A file somebody else made is
//! dropped into the directory and is there the next time the application
//! starts.
//!
//! The shape is the flat `key = value` one `filaments.conf` and
//! `printer.conf` use ([`crate::paths::entries`]). A line is a brush: its
//! name, and after the `=` its settings as `key:value` pairs with commas
//! between them.
//!
//! ```text
//! Fine crease = brush:Draw, push:-0.6, pinch:1, size:1.5, strength:0.3, lazy:12
//! Scrape = brush:Clay, plane:-0.15, side:Cut
//! Rivet row = brush:Draw, size:1, stamp:Rivet, along:1.5
//! ```
//!
//! `brush:` names the classic brush the settings start from, and the
//! amounts after it (`push`, `inflate`, `pinch`, `smooth`, `level`, and for
//! a brush that levels `plane` and `side`) are what differs from it. So a
//! line that says only `brush:Clay` is Clay, and a line with no `size` or
//! `strength` is a brush that leaves those as they are in hand.
//!
//! A file may hold more than one line, and each is a brush: a pack.
//!
//! **A setting this module cannot make sense of is left at its default and
//! the rest of the line is kept**, and a number out of range is brought
//! into it. The file is the user's to edit by hand, and the failure worth
//! avoiding is a typo in one setting throwing away a brush, or a typed
//! `size:900` costing a second a stamp.
//!
//! ## Where they were before
//!
//! All in one file, `brushes.conf`, first in the settings directory and
//! then in the data directory. While there is no `brushes` directory that
//! file is still read, from either place, and what it holds is written out
//! as files of the new kind, so that nobody's brushes went missing when
//! they moved. The old file is left where it is.
//!
//! # What a brush does not hold
//!
//! * **A stamp picture read from a file.** A brush kept with one in hand
//!   says "the stamp, with the picture in hand" and uses whichever picture
//!   that is when it is chosen. The pictures that come with the
//!   application are held by name.
//! * **A surface pattern**, which is a dozen settings of its own with its
//!   own panel.
//! * **The paint brush's colour and touch**, which belong to the paint
//!   rack.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use brokkr_core::BrushKind;
use brokkr_core::brush::alpha::BuiltIn;
use brokkr_core::brush::compound::{Actions, MOST_PLATEAU, Profile, Shape, Side};
use contracts::debug_ensures;

use super::studio::{ALONG_RANGE, HEIGHT_RANGE, LAZY_RANGE, SIZE_RANGE};
use super::{MAX_STRENGTH, MIN_RADIUS_MM, MIN_STRENGTH};

/// The one file the brushes used to be in.
const BEFORE: &str = "brushes.conf";

/// The directory the brushes are in, under the data directory.
const DIRECTORY: &str = "brushes";

/// What a brush's file is called after its name.
const EXTENSION: &str = "brush";

/// The most brushes of a user's own kept. The panel shows them all, two to
/// a row, under the ones that come with the application.
pub(super) const MAX_PRESETS: usize = 24;

/// The longest a brush's name may be, in characters: what fits a button.
pub(super) const LONGEST_NAME: usize = 24;

/// The biggest brush a preset may ask for, in millimetres. The application
/// brings it down further to what the document's voxel size allows when
/// the preset is chosen; this only keeps the number sane.
const BIGGEST_MM: f32 = 100.0;

/// The stamp a preset takes up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum StampOf {
    /// One of the pictures that come with the application.
    BuiltIn(BuiltIn),
    /// Whichever picture is in hand when the preset is chosen.
    InHand,
}

/// A brush as it is set up.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Preset {
    pub name: String,
    /// The classic brush this one goes by: its key, its remembered
    /// strength, and whether the invert key does anything. For a brush
    /// with amounts of its own it is their [`Actions::family`].
    pub kind: BrushKind,
    /// What the brush does, where that is not what `kind` does.
    pub custom: Option<Actions>,
    /// The cross-section of its stroke.
    pub profile: Profile,
    /// How big, or nothing for a brush that leaves the size in hand.
    pub size_mm: Option<f32>,
    /// How hard, or nothing for a brush that takes up the strength its
    /// family was last used at.
    pub strength: Option<f32>,
    /// The stamp, and how tall it is as a share of half its width. A brush
    /// has amounts of its own or the stamp and not both: the stamp wins.
    pub stamp: Option<(StampOf, f32)>,
    /// How far apart the stamp is laid along a drag, in stamp widths, or
    /// nothing for one stamp sized by the drag. Nothing without a stamp.
    pub along: f32,
    pub lazy_px: f32,
    pub size_from_pressure: f32,
}

/// The six amounts a line of the file may give, in the order
/// [`Preset::amounts`] has them.
const AMOUNTS: [&str; 6] = ["push", "inflate", "pinch", "smooth", "level", "plane"];

impl Preset {
    /// The classic Draw brush, under `name`, at whatever size and strength
    /// are in hand.
    pub fn plain(name: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: BrushKind::Draw,
            custom: None,
            profile: Profile::default(),
            size_mm: None,
            strength: None,
            stamp: None,
            along: 0.0,
            lazy_px: 0.0,
            size_from_pressure: 0.0,
        }
    }

    /// The classic brush of `kind`, under `name`.
    fn classic(name: &str, kind: BrushKind) -> Self {
        Self { kind, ..Self::plain(name) }
    }

    /// A brush that does `actions`, under `name`.
    fn doing(name: &str, actions: Actions) -> Self {
        Self { custom: Some(actions), ..Self::plain(name) }.settled()
    }

    /// This preset with the things a preset keeps true of itself made
    /// true. See [`Preset::is_settled`].
    #[debug_ensures(ret.is_settled(), "what is settled is settled")]
    pub fn settled(mut self) -> Self {
        if self.stamp.is_some() {
            self.custom = None;
        } else {
            self.along = 0.0;
        }
        if let Some(actions) = self.custom {
            let mut actions = actions.legal();
            // A plane nothing is levelled to is no setting at all, and two
            // brushes that differ only in one are the same brush.
            if actions.level == 0.0 {
                actions.plane = 0.0;
                actions.side = Side::Both;
            }
            self.kind = actions.family();
            self.custom = (Actions::of(self.kind) != Some(actions)).then_some(actions);
        }
        let plateau = self.profile.plateau;
        self.profile.plateau =
            if plateau.is_finite() { plateau.clamp(0.0, MOST_PLATEAU) } else { 0.0 };
        self
    }

    /// Whether the preset is one [`Preset::settled`] would leave alone:
    ///
    /// * amounts of its own are legal, are not just what its classic brush
    ///   does, and have that brush as their family;
    /// * a stamp has no amounts of its own, and only a stamp is in a row;
    /// * the plateau is within what a profile can have.
    ///
    /// Two presets that are the same brush are then equal, which is what
    /// the palette lights its buttons by.
    pub fn is_settled(&self) -> bool {
        let amounts = self.custom.is_none_or(|actions| {
            actions.is_legal()
                && actions.family() == self.kind
                && Actions::of(self.kind) != Some(actions)
                && (actions.level != 0.0 || (actions.plane == 0.0 && actions.side == Side::Both))
        });
        let stamp = if self.stamp.is_some() { self.custom.is_none() } else { self.along == 0.0 };
        amounts && stamp && (0.0..=MOST_PLATEAU).contains(&self.profile.plateau)
    }

    /// Whether the brush in hand, as the preset `now`, is this brush: the
    /// same in everything but the name, and in the size and the strength
    /// only where this brush says what they are.
    pub fn is(&self, now: &Preset) -> bool {
        self.kind == now.kind
            && self.custom == now.custom
            && self.profile == now.profile
            && self.size_mm.is_none_or(|size| Some(size) == now.size_mm)
            && self.strength.is_none_or(|strength| Some(strength) == now.strength)
            && self.stamp == now.stamp
            && self.along == now.along
            && self.lazy_px == now.lazy_px
            && self.size_from_pressure == now.size_from_pressure
    }

    /// The six amounts, in the order of [`AMOUNTS`].
    fn amounts(actions: &Actions) -> [f32; 6] {
        [actions.push, actions.inflate, actions.pinch, actions.smooth, actions.level, actions.plane]
    }

    /// The settings half of a line of the file.
    fn settings(&self) -> String {
        let mut fields = vec![format!("brush:{}", self.kind.label())];
        if let (Some(actions), Some(classic)) = (self.custom, Actions::of(self.kind)) {
            // What differs from the classic brush the line names.
            let (mine, its) = (Self::amounts(&actions), Self::amounts(&classic));
            for (index, key) in AMOUNTS.into_iter().enumerate() {
                if mine[index] != its[index] {
                    fields.push(format!("{key}:{}", mine[index]));
                }
            }
            if actions.side != classic.side {
                fields.push(format!("side:{}", actions.side.label()));
            }
        }
        fields.push(format!("falloff:{}", self.profile.shape.label()));
        if self.profile.plateau > 0.0 {
            fields.push(format!("plateau:{}", self.profile.plateau));
        }
        if let Some(size) = self.size_mm {
            fields.push(format!("size:{size}"));
        }
        if let Some(strength) = self.strength {
            fields.push(format!("strength:{strength}"));
        }
        if let Some((stamp, height)) = self.stamp {
            let picture = match stamp {
                StampOf::BuiltIn(kind) => kind.label(),
                StampOf::InHand => IN_HAND,
            };
            fields.push(format!("stamp:{picture}"));
            fields.push(format!("height:{height}"));
            if self.along > 0.0 {
                fields.push(format!("along:{}", self.along));
            }
        }
        if self.lazy_px > 0.0 {
            fields.push(format!("lazy:{}", self.lazy_px));
        }
        if self.size_from_pressure > 0.0 {
            fields.push(format!("pressure:{}", self.size_from_pressure));
        }
        fields.join(", ")
    }

    /// Take the settings on a line of the file, each one that makes sense.
    fn take(mut self, settings: &str) -> Self {
        // The stamp's height may come before the stamp, and an amount
        // before the brush it is an amount of, on a line somebody wrote by
        // hand: both are kept until the whole line has been read.
        let mut height = None;
        let mut amounts: [Option<f32>; 6] = [None; 6];
        let mut side = None;
        let mut crease = false;
        for field in settings.split(',') {
            let Some((key, value)) = field.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            let named = |label: &str| label.eq_ignore_ascii_case(value);
            let number = |range: std::ops::RangeInclusive<f32>| {
                value
                    .parse::<f32>()
                    .ok()
                    .filter(|number| number.is_finite())
                    .map(|number| number.clamp(*range.start(), *range.end()))
            };
            match key.as_str() {
                "brush" => {
                    if let Some(kind) = BrushKind::ALL.into_iter().find(|kind| named(kind.label()))
                    {
                        self.kind = kind;
                    }
                }
                "push" | "inflate" | "pinch" | "plane" => {
                    let index = AMOUNTS.iter().position(|amount| *amount == key);
                    if let (Some(index), Some(amount)) = (index, number(-1.0..=1.0)) {
                        amounts[index] = Some(amount);
                    }
                }
                "smooth" | "level" => {
                    let index = AMOUNTS.iter().position(|amount| *amount == key);
                    if let (Some(index), Some(amount)) = (index, number(0.0..=1.0)) {
                        amounts[index] = Some(amount);
                    }
                }
                "side" => side = Side::ALL.into_iter().find(|side| named(side.label())).or(side),
                // How a crease was said before a brush had amounts.
                "studio" => crease |= named("Crease"),
                "size" => self.size_mm = number(MIN_RADIUS_MM..=BIGGEST_MM).or(self.size_mm),
                "strength" => {
                    self.strength = number(MIN_STRENGTH..=MAX_STRENGTH).or(self.strength);
                }
                "falloff" => {
                    if let Some(shape) = Shape::ALL.into_iter().find(|shape| named(shape.label())) {
                        self.profile.shape = shape;
                    }
                }
                "plateau" => {
                    if let Some(plateau) = number(0.0..=MOST_PLATEAU) {
                        self.profile.plateau = plateau;
                    }
                }
                "stamp" => {
                    let stamp = if named(IN_HAND) {
                        Some(StampOf::InHand)
                    } else {
                        BuiltIn::ALL
                            .into_iter()
                            .find(|kind| named(kind.label()))
                            .map(StampOf::BuiltIn)
                    };
                    if let Some(stamp) = stamp {
                        self.stamp = Some((stamp, DEFAULT_HEIGHT));
                    }
                }
                "height" => height = number(HEIGHT_RANGE).or(height),
                "along" => {
                    if let Some(widths) = number(ALONG_RANGE) {
                        self.along = widths;
                    }
                }
                "lazy" => {
                    if let Some(pixels) = number(LAZY_RANGE) {
                        self.lazy_px = pixels;
                    }
                }
                "pressure" => {
                    if let Some(share) = number(SIZE_RANGE) {
                        self.size_from_pressure = share;
                    }
                }
                _ => {}
            }
        }
        if let (Some((stamp, _)), Some(height)) = (self.stamp, height) {
            self.stamp = Some((stamp, height));
        }
        // The amounts, laid over what the brush the line names does. A
        // line that names one of the two that do nothing to the field and
        // then gives amounts is a brush of those amounts.
        if crease || side.is_some() || amounts.iter().any(Option::is_some) {
            let mut actions =
                if crease { Actions::CREASE } else { Actions::of(self.kind).unwrap_or_default() };
            let [push, inflate, pinch, smooth, level, plane] = amounts;
            actions.push = push.unwrap_or(actions.push);
            actions.inflate = inflate.unwrap_or(actions.inflate);
            actions.pinch = pinch.unwrap_or(actions.pinch);
            actions.smooth = smooth.unwrap_or(actions.smooth);
            actions.level = level.unwrap_or(actions.level);
            actions.plane = plane.unwrap_or(actions.plane);
            actions.side = side.unwrap_or(actions.side);
            self.custom = Some(actions);
        }
        self.settled()
    }

    /// The brush's line of a file.
    fn line(&self) -> String {
        format!("{} = {}\n", self.name, self.settings())
    }
}

/// Amounts as a brush holds them: the classic brush they go by, and the
/// amounts themselves where they are not just what that brush does.
pub(super) fn held(actions: Actions) -> (BrushKind, Option<Actions>) {
    let preset = Preset::doing("", actions);
    (preset.kind, preset.custom)
}

/// How a preset's line says "the stamp, with the picture in hand".
const IN_HAND: &str = "picture";

/// How tall a stamp is when a line names one and gives no height: what the
/// studio block starts at.
const DEFAULT_HEIGHT: f32 = 0.35;

/// A name as the file can hold it and a button can show it, or `None` when
/// nothing is left of it.
///
/// A name is the key of a line, so it cannot hold an `=` or a line end, and
/// a line that starts with `#` is a comment. Those characters are dropped
/// and not refused: a name typed with one in is still clearly a name.
pub(super) fn named(name: &str) -> Option<String> {
    let kept: String = name
        .chars()
        .filter(|character| !character.is_control() && !matches!(character, '=' | '#'))
        .collect();
    let kept: String = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let kept: String = kept.chars().take(LONGEST_NAME).collect();
    let kept = kept.trim_end().to_string();
    (!kept.is_empty()).then_some(kept)
}

/// The brushes that come with the application, in the order the palette
/// shows them, three to a row.
///
/// The first seven are upstream's six and the one upstream's Pinch turns
/// into under the invert key. Every one of them is a brush on the field or
/// a stamp, and none says how big it is unless being that size is the
/// point of it.
pub(super) fn core() -> &'static [Preset] {
    static CORE: LazyLock<Vec<Preset>> = LazyLock::new(|| {
        let nothing = Actions::default();
        let stamp = |name: &str, picture: BuiltIn| Preset {
            stamp: Some((StampOf::BuiltIn(picture), DEFAULT_HEIGHT)),
            ..Preset::plain(name)
        };
        vec![
            Preset::classic("Draw", BrushKind::Draw),
            Preset::classic("Clay", BrushKind::Clay),
            Preset::classic("Inflate", BrushKind::Inflate),
            Preset::classic("Pinch", BrushKind::Pinch),
            Preset::doing("Spread", Actions { pinch: -1.0, ..nothing }),
            Preset::classic("Smooth", BrushKind::Smooth),
            Preset::classic("Flatten", BrushKind::Flatten),
            // Cuts down to a plane held a little under the surface and
            // fills nothing: Clay turned over, and shallower. The plane
            // has to be under the surface for there to be anything to
            // cut: nothing stands proud of a plane that touches a ball.
            Preset::doing(
                "Scrape",
                Actions { level: 1.0, plane: SCRAPE_DEPTH, side: Side::Cut, ..nothing },
            ),
            // Fills up to the plane through the surface under the brush
            // and takes nothing off: a hollow filled, a facet built.
            Preset::doing("Fill", Actions { level: 1.0, side: Side::Fill, ..nothing }),
            // Levels and smooths at once: a face brought flat and clean.
            Preset::doing("Polish", Actions { smooth: 0.5, level: 0.5, ..nothing }),
            Preset::doing("Crease", Actions::CREASE),
            // A stroke with the cross-section of a ball: a bead of weld.
            Preset {
                profile: Profile { shape: Shape::Round, plateau: 0.0 },
                ..Preset::plain("Bead")
            },
            // A stroke flat across most of its width: a strap, a step.
            Preset {
                profile: Profile { shape: Shape::Smooth, plateau: 0.6 },
                ..Preset::plain("Band")
            },
            Preset {
                size_mm: Some(1.5),
                strength: Some(0.3),
                lazy_px: 12.0,
                ..Preset::doing("Fine crease", Actions::CREASE)
            },
            Preset { size_mm: Some(2.0), lazy_px: 30.0, ..Preset::plain("Long line") },
            stamp("Rivets", BuiltIn::Rivet),
            Preset { size_mm: Some(1.0), along: 1.5, ..stamp("Rivet row", BuiltIn::Rivet) },
            Preset { size_mm: Some(2.0), along: 0.5, ..stamp("Scales", BuiltIn::Scale) },
        ]
    });
    &CORE
}

/// How far under the surface Scrape holds its plane, in radii of the
/// brush: under half of how far proud Clay holds its own (0.35), so that
/// a scraper takes a shaving where clay lays a strip.
const SCRAPE_DEPTH: f32 = -0.15;

/// Where a brush on the palette came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// It came with the application, and is as it came.
    Core,
    /// The user's, kept under the name of one that came with the
    /// application and standing where that one stood.
    Changed,
    /// The user's own.
    Own,
}

/// The brushes a user has kept, in the order of their names.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Presets {
    pub list: Vec<Preset>,
}

impl Presets {
    /// The palette: the brushes that come with the application, each
    /// replaced by the user's of the same name where there is one, and
    /// then the rest of the user's.
    pub fn palette(&self) -> Vec<(&Preset, Place)> {
        let mine = |name: &str| self.list.iter().find(|held| held.name == name);
        let mut palette: Vec<(&Preset, Place)> = core()
            .iter()
            .map(|came| match mine(&came.name) {
                Some(changed) => (changed, Place::Changed),
                None => (came, Place::Core),
            })
            .collect();
        let came = |name: &str| core().iter().any(|came| came.name == name);
        palette.extend(
            self.list.iter().filter(|held| !came(&held.name)).map(|held| (held, Place::Own)),
        );
        palette
    }

    /// Read brushes out of the text of a file.
    ///
    /// A name used twice is one brush, the later line's.
    pub fn parse(text: &str) -> Self {
        let mut presets = Self::default();
        for (key, value) in crate::paths::entries(text) {
            let Some(name) = named(key) else {
                continue;
            };
            presets.put(Preset::plain(&name).take(value));
        }
        presets
    }

    /// The text of a file that holds these brushes.
    pub fn to_config(&self) -> String {
        let mut text = String::from(
            "# A BrokkrSculpt brush: its name, then its settings.\n\
             # Safe to edit by hand, and to hand to somebody else.\n",
        );
        for preset in &self.list {
            text.push_str(&preset.line());
        }
        text
    }

    /// Keep a brush: in place of the one with its name, or where its name
    /// puts it. `false`, and nothing kept, when there is no room for
    /// another.
    #[debug_ensures(self.is_in_order(), "the brushes stay in the order of their names")]
    pub fn put(&mut self, preset: Preset) -> bool {
        let preset = preset.settled();
        if let Some(held) = self.list.iter_mut().find(|held| held.name == preset.name) {
            *held = preset;
            return true;
        }
        if self.list.len() >= MAX_PRESETS {
            return false;
        }
        let at = self.list.partition_point(|held| before(&held.name, &preset.name));
        self.list.insert(at, preset);
        true
    }

    /// Stop keeping the brush of this name, and hand it back.
    pub fn remove(&mut self, name: &str) -> Option<Preset> {
        let at = self.list.iter().position(|held| held.name == name)?;
        Some(self.list.remove(at))
    }

    /// Whether the brushes are in the order of their names, no name twice.
    fn is_in_order(&self) -> bool {
        self.list.windows(2).all(|pair| before(&pair[0].name, &pair[1].name))
    }

    /// The brushes this machine has kept.
    pub fn load() -> Self {
        let directory = crate::paths::data_file(DIRECTORY);
        let earlier =
            first_there(crate::paths::data_file(BEFORE), crate::paths::config_file(BEFORE));
        Self::load_from(directory.as_deref(), earlier.as_deref(), !cfg!(test))
    }

    /// Split out so the tests can point it at a directory of their own.
    ///
    /// The directory's files when there is a directory. When there is
    /// none, the one file the brushes used to be in, and with `move_in`
    /// what that held is written out as files of the new kind.
    pub fn load_from(directory: Option<&Path>, earlier: Option<&Path>, move_in: bool) -> Self {
        if let Some(directory) = directory.filter(|directory| directory.is_dir()) {
            let mut presets = Self::default();
            for file in files_in(directory) {
                let Ok(text) = std::fs::read_to_string(&file) else {
                    continue;
                };
                for preset in Self::parse(&text).list {
                    presets.put(preset);
                }
            }
            return presets;
        }
        let Some(text) = earlier.and_then(|file| std::fs::read_to_string(file).ok()) else {
            return Self::default();
        };
        let presets = Self::parse(&text);
        if let (true, Some(directory)) = (move_in, directory) {
            for preset in &presets.list {
                if let Err(why) = write_in(directory, preset) {
                    log::warn!("a brush was not moved to its own file: {why}");
                }
            }
        }
        presets
    }

    /// Write a brush to its file on this machine.
    ///
    /// **Inert under `cfg(test)`**, as `Palette::save` is and for its
    /// reason: an application test that keeps a brush goes through the
    /// real `update`, and would otherwise write into the brushes of
    /// whoever ran the tests.
    pub fn write(preset: &Preset) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        let Some(directory) = crate::paths::data_file(DIRECTORY) else {
            return Err("no data directory to write to".to_string());
        };
        write_in(&directory, preset)
    }

    /// Take a brush's file away from this machine. Inert under
    /// `cfg(test)`, as [`Presets::write`] is.
    pub fn erase(name: &str) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        let Some(directory) = crate::paths::data_file(DIRECTORY) else {
            return Err("no data directory to write to".to_string());
        };
        strike(&directory, name)
    }
}

/// Whether one name comes before another on the palette: by its letters
/// whatever their case, and by the name itself between two that differ
/// only in case, so that the order is one order.
fn before(one: &str, other: &str) -> bool {
    (one.to_lowercase(), one) < (other.to_lowercase(), other)
}

/// The brush files in a directory, in the order of their names.
fn files_in(directory: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION))
        })
        .collect();
    files.sort();
    files
}

/// What a brush's file is called before its extension: the letters and
/// digits of its name in lower case, with a hyphen for each run of
/// anything else. A name of nothing but anything else is `brush`.
fn stem_of(name: &str) -> String {
    let mut stem = String::new();
    for character in name.chars() {
        if character.is_alphanumeric() {
            stem.extend(character.to_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let stem = stem.trim_end_matches('-');
    if stem.is_empty() { EXTENSION.to_string() } else { stem.to_string() }
}

/// Take the brush of this name out of every file in the directory that
/// has it: the file itself when the brush is all it holds, and the brush's
/// line when the file is a pack.
fn strike(directory: &Path, name: &str) -> Result<(), String> {
    for file in files_in(directory) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let mut held = Presets::parse(&text);
        if held.remove(name).is_none() {
            continue;
        }
        let done = if held.list.is_empty() {
            std::fs::remove_file(&file)
        } else {
            std::fs::write(&file, held.to_config())
        };
        done.map_err(|why| format!("could not change {}: {why}", file.display()))?;
    }
    Ok(())
}

/// Write a brush to a file of its own in the directory, in place of
/// wherever in the directory it was before.
///
/// Two names may come to one file name ("Wet clay" and "wet-clay"). The
/// second is given a number after it and does not write over the first.
fn write_in(directory: &Path, preset: &Preset) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|why| format!("could not create {}: {why}", directory.display()))?;
    strike(directory, &preset.name)?;
    let stem = stem_of(&preset.name);
    let file = (1..)
        .map(|count| {
            let stem = if count == 1 { stem.clone() } else { format!("{stem}-{count}") };
            directory.join(format!("{stem}.{EXTENSION}"))
        })
        .find(|file| !file.exists())
        .expect("a count that has no end");
    let text = Presets { list: vec![preset.clone()] }.to_config();
    std::fs::write(&file, text).map_err(|why| format!("could not write {}: {why}", file.display()))
}

/// The first of two files that is there: where the one file of brushes
/// was last, and where it was first. The later place when neither is,
/// which reads as no file at all.
fn first_there(here: Option<PathBuf>, before: Option<PathBuf>) -> Option<PathBuf> {
    match (here, before) {
        (Some(here), _) if here.is_file() => Some(here),
        (_, Some(before)) if before.is_file() => Some(before),
        (here, _) => here,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, empty.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("brokkr-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    /// Every setting a preset has, none of them at its default.
    fn full(name: &str) -> Preset {
        Preset {
            name: name.to_string(),
            kind: BrushKind::Clay,
            custom: None,
            profile: Profile { shape: Shape::Sharp, plateau: 0.4 },
            size_mm: Some(4.25),
            strength: Some(0.33),
            stamp: Some((StampOf::BuiltIn(BuiltIn::Scale), 0.6)),
            along: 2.5,
            lazy_px: 17.5,
            size_from_pressure: 0.4,
        }
    }

    /// The same, as a brush on the field with amounts of its own.
    fn made(name: &str, actions: Actions) -> Preset {
        Preset { stamp: None, custom: Some(actions), ..full(name) }.settled()
    }

    /// Brushes written and read back are the brushes that were written:
    /// every setting, for amounts of every kind, for each kind of stamp
    /// and for a plain brush.
    #[test]
    fn brushes_come_back_from_their_file_as_they_went_in() {
        let nothing = Actions::default();
        let mut presets = Presets::default();
        for preset in [
            full("Scales, big"),
            Preset { stamp: Some((StampOf::InHand, 0.11)), ..full("My own picture") },
            Preset { along: 0.0, ..full("One scale") },
            made("Crease: fine", Actions::CREASE),
            made("Spread", Actions { pinch: -1.0, ..nothing }),
            made("Scraper", Actions { level: 1.0, side: Side::Cut, ..nothing }),
            made("Deep clay", Actions { plane: 0.8, ..Actions::CLAY }),
            made(
                "Everything",
                Actions {
                    push: 0.25,
                    inflate: -0.5,
                    pinch: 0.75,
                    smooth: 0.1,
                    level: 0.9,
                    plane: -0.3,
                    side: Side::Fill,
                },
            ),
            made("Nothing at all", nothing),
            Preset::plain("Plain"),
            Preset { kind: BrushKind::Paint, stamp: None, along: 0.0, ..full("Paint") },
            Preset { kind: BrushKind::Move, stamp: None, along: 0.0, ..full("Move") },
            Preset {
                profile: Profile { shape: Shape::Round, plateau: 0.0 },
                ..Preset::plain("Dome")
            },
        ] {
            assert!(preset.is_settled(), "{preset:?}");
            assert!(presets.put(preset));
        }
        let back = Presets::parse(&presets.to_config());
        assert_eq!(back, presets);
        // Controls: the fixture is not all defaults, so "it came back" is
        // not "nothing was read and the defaults matched", and the amounts
        // really are amounts of their own.
        assert!(!Preset::plain("").is(&full("")));
        assert!(presets.list.iter().filter(|preset| preset.custom.is_some()).count() >= 6);
        // And the brushes that come with the application are themselves
        // lines that read back.
        for came in core() {
            assert_eq!(Presets::parse(&came.line()).list, vec![came.clone()], "{}", came.name);
        }
    }

    /// The brushes that come with the application: each is settled, is a
    /// brush on the field or a stamp, has a name of its own that a file
    /// could hold, and they fill their rows of three.
    #[test]
    fn the_brushes_that_come_with_the_application_are_sound() {
        let came = core();
        assert_eq!(came.len() % 3, 0, "{} brushes do not fill rows of three", came.len());
        for (index, preset) in came.iter().enumerate() {
            assert!(preset.is_settled(), "{preset:?}");
            assert!(Actions::of(preset.kind).is_some(), "{} is not a field brush", preset.name);
            assert_eq!(named(&preset.name).as_deref(), Some(preset.name.as_str()));
            assert!(
                came[..index].iter().all(|earlier| earlier.name != preset.name),
                "two brushes are called {}",
                preset.name
            );
            // No two are the same brush under two names.
            for earlier in &came[..index] {
                assert!(
                    !(earlier.is(preset) && preset.is(earlier)),
                    "{} and {} are one brush",
                    earlier.name,
                    preset.name
                );
            }
        }
        // Upstream's six are six of them, each as nothing but itself.
        for kind in BrushKind::ALL {
            let Some(_) = Actions::of(kind) else {
                continue;
            };
            let classic = came.iter().find(|preset| preset.name == kind.label());
            assert_eq!(classic, Some(&Preset::classic(kind.label(), kind)), "{kind:?}");
        }
        // And the ones that cost only their settings do what they say.
        let does = |name: &str| {
            let preset = came.iter().find(|preset| preset.name == name).expect(name);
            preset.custom.or_else(|| Actions::of(preset.kind)).expect("a field brush")
        };
        assert_eq!(does("Crease"), Actions::CREASE);
        assert_eq!(does("Fine crease"), Actions::CREASE);
        assert_eq!(does("Spread").pinch, -1.0);
        assert_eq!((does("Scrape").level, does("Scrape").side), (1.0, Side::Cut));
        assert_eq!((does("Fill").level, does("Fill").side), (1.0, Side::Fill));
    }

    /// Every brush, shape, side and picture there is can be named in the
    /// file, in any letter case, and a line as it was written before a
    /// brush had amounts still reads as the brush it was.
    #[test]
    fn every_brush_shape_side_and_picture_can_be_named() {
        let read = |line: &str| Presets::parse(line).list.remove(0);
        for kind in BrushKind::ALL {
            let line = format!("a = brush:{}", kind.label().to_uppercase());
            assert_eq!(read(&line), Preset::classic("a", kind), "{line}");
        }
        for shape in Shape::ALL {
            let line = format!("a = falloff:{}", shape.label().to_lowercase());
            assert_eq!(read(&line).profile.shape, shape, "{line}");
        }
        for side in Side::ALL {
            let line = format!("a = brush:Clay, side:{}", side.label().to_uppercase());
            let actions = read(&line).custom.unwrap_or(Actions::CLAY);
            assert_eq!(actions, Actions { side, ..Actions::CLAY }, "{line}");
        }
        for kind in BuiltIn::ALL {
            let line = format!("a = stamp:{}", kind.label());
            assert_eq!(read(&line).stamp, Some((StampOf::BuiltIn(kind), DEFAULT_HEIGHT)), "{line}");
        }
        // As the five the application used to start with were written.
        let crease = read("a = brush:Draw, size:1.5, strength:0.3, studio:Crease, lazy:12");
        assert_eq!(crease.custom, Some(Actions::CREASE));
        assert_eq!((crease.size_mm, crease.strength, crease.lazy_px), (Some(1.5), Some(0.3), 12.0));
        let clay = read("a = brush:Clay, size:6, strength:0.25, falloff:Smooth, pressure:0.5");
        assert_eq!((clay.kind, clay.custom), (BrushKind::Clay, None));
        // An operator that has a classic brush was never a studio one.
        assert_eq!(read("a = studio:Draw").custom, None);
    }

    /// The amounts on a line are laid over what the brush it names does,
    /// wherever on the line the brush is named, and the brush a line of
    /// amounts goes by is the one they make.
    #[test]
    fn amounts_are_laid_over_the_brush_the_line_names() {
        let read = |settings: &str| Presets::parse(&format!("a = {settings}")).list.remove(0);
        // Clay with its plane held further off is still a clay.
        let deep = read("brush:Clay, plane:0.8");
        assert_eq!(deep.custom, Some(Actions { plane: 0.8, ..Actions::CLAY }));
        assert_eq!(deep.kind, BrushKind::Clay);
        assert_eq!(read("plane:0.8, brush:Clay"), deep, "the order on the line mattered");
        // A push in and a pinch is a crease, and goes by Draw.
        let crease = read("push:-0.6, pinch:1");
        assert_eq!((crease.kind, crease.custom), (BrushKind::Draw, Some(Actions::CREASE)));
        // Amounts that are just a classic brush's are that brush.
        assert_eq!(read("brush:Smooth, smooth:1"), Preset::classic("a", BrushKind::Smooth));
        assert_eq!(read("brush:Draw, push:0, smooth:1"), Preset::classic("a", BrushKind::Smooth));
        // Amounts on a line that names a brush with none are a brush of
        // those amounts.
        let from_paint = read("brush:Paint, inflate:0.5");
        assert_eq!(from_paint.kind, BrushKind::Inflate);
        assert_eq!(from_paint.custom.map(|actions| actions.inflate), Some(0.5));
        // A plane nothing is levelled to is no setting.
        assert_eq!(read("brush:Draw, plane:0.5, side:Cut"), Preset::plain("a"));
        // The push and the inflation share the voxel a stamp may move.
        let both = read("push:1, inflate:1").custom.expect("amounts of its own");
        assert!(both.is_legal() && both.push > 0.0 && both.inflate > 0.0, "{both:?}");
        // A stamp has no amounts of its own.
        assert_eq!(read("stamp:Dome, push:-0.5").custom, None);
    }

    /// A setting that makes no sense is left at its default and the rest of
    /// the line is kept. A number out of range is brought into it.
    #[test]
    fn a_bad_setting_costs_only_itself() {
        let plain = Preset::plain("a");
        let read = |settings: &str| Presets::parse(&format!("a = {settings}")).list.remove(0);

        let preset = read(
            "brush:Chisel, size:big, strength:, falloff:9, studio:Nothing, stamp:Cog, lazy:x, \
             pressure:NaN, height:inf, colour:red, , no colon here, :, push:hard, side:Left, \
             plateau:wide, lazy:20",
        );
        assert_eq!(preset, Preset { lazy_px: 20.0, ..plain.clone() });

        let preset = read("size:900, strength:7, lazy:-4, pressure:3, stamp:Dome, height:55");
        assert_eq!(preset.size_mm, Some(BIGGEST_MM));
        assert_eq!(preset.strength, Some(MAX_STRENGTH));
        assert_eq!(preset.lazy_px, *LAZY_RANGE.start());
        assert_eq!(preset.size_from_pressure, *SIZE_RANGE.end());
        assert_eq!(preset.stamp, Some((StampOf::BuiltIn(BuiltIn::Dome), *HEIGHT_RANGE.end())));

        let preset = read("size:0, strength:0, height:0.5, plateau:7");
        assert_eq!(preset.size_mm, Some(MIN_RADIUS_MM));
        assert_eq!(preset.strength, Some(MIN_STRENGTH));
        assert_eq!(preset.profile.plateau, MOST_PLATEAU);
        assert_eq!(preset.stamp, None, "a height alone took up the stamp");

        let amounts = read("push:-9, pinch:4, smooth:-2, level:3").custom.expect("amounts");
        assert_eq!(
            (amounts.push, amounts.pinch, amounts.smooth, amounts.level),
            (-1.0, 1.0, 0.0, 1.0)
        );

        // A height before its stamp on the line still counts, and a stamp
        // and amounts on one line is the stamp.
        let preset = read("height:0.8, studio:Crease, stamp:Bar");
        assert_eq!(preset.stamp, Some((StampOf::BuiltIn(BuiltIn::Bar), 0.8)));
        assert_eq!(preset.custom, None);
        // Control: the crease on its own is taken.
        assert_eq!(read("studio:Crease").custom, Some(Actions::CREASE));

        // A row is a row of stamps: its spacing is brought into range,
        // and without a stamp it is nothing.
        assert_eq!(read("stamp:Dome, along:9").along, *ALONG_RANGE.end());
        assert_eq!(read("along:0.1, stamp:Dome").along, *ALONG_RANGE.start());
        assert_eq!(read("along:2").along, 0.0, "a row of no stamp");
        assert_eq!(read("along:2, studio:Crease").along, 0.0);
        assert_eq!(read("stamp:Dome, along:x").along, 0.0);
        assert_eq!(read("stamp:Dome, along:2").along, 2.0);
    }

    /// A name is what the file can hold as a key: no `=`, no `#`, no line
    /// ends, single spaces, and no longer than a button.
    #[test]
    fn a_name_is_made_safe_for_the_file() {
        assert_eq!(named("  Fine   crease \n").as_deref(), Some("Fine crease"));
        assert_eq!(named("a = b").as_deref(), Some("a b"));
        assert_eq!(named("# not a comment").as_deref(), Some("not a comment"));
        assert_eq!(named("line\none\ttwo\u{7}").as_deref(), Some("lineonetwo"));
        assert_eq!(named(""), None);
        assert_eq!(named(" =#\n "), None);
        let long = named(&"ab ".repeat(40)).expect("a name");
        assert!(long.chars().count() <= LONGEST_NAME, "{long:?}");
        assert!(!long.ends_with(' '), "{long:?} ends in a space the file would trim");
        // Characters, not bytes: a name in another script is not cut in
        // the middle of a letter.
        let wide = named(&"\u{00e9}".repeat(60)).expect("a name");
        assert_eq!(wide.chars().count(), LONGEST_NAME);

        // And a name made safe is a name that comes back from the file.
        for name in ["a = b", "# x", "  two  words  ", "Fine crease, small: 1.5"] {
            let name = named(name).expect("a name");
            let presets = Presets { list: vec![Preset::plain(&name)] };
            assert_eq!(Presets::parse(&presets.to_config()), presets, "{name:?}");
        }
    }

    /// A name used again replaces the brush that had it, the brushes are
    /// kept in the order of their names, and there is room for only so
    /// many.
    #[test]
    fn a_name_used_again_replaces_and_the_list_has_an_end() {
        let sized = |name: &str, size: f32| Preset { size_mm: Some(size), ..Preset::plain(name) };
        let mut presets =
            Presets::parse("b = size:2\nA = size:1\nc = size:4\na = size:5\nb = size:3\n");
        let names: Vec<&str> = presets.list.iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(names, ["A", "a", "b", "c"], "not in the order of their names");
        assert_eq!(presets.list[2], sized("b", 3.0), "the later line did not replace the earlier");

        for index in 0..MAX_PRESETS + 5 {
            presets.put(Preset::plain(&format!("p{index:02}")));
        }
        assert_eq!(presets.list.len(), MAX_PRESETS);
        assert!(!presets.put(Preset::plain("one more")), "there was room past the end");
        // Replacing one that is there is not adding one.
        assert!(presets.put(sized("b", 9.0)));
        assert_eq!(presets.list[2], sized("b", 9.0));
        assert_eq!(presets.list.len(), MAX_PRESETS);
        // And taking one out makes room for one.
        assert_eq!(presets.remove("b"), Some(sized("b", 9.0)));
        assert_eq!(presets.remove("b"), None);
        assert!(presets.put(Preset::plain("one more")));

        // A file with more lines than that keeps the first of them.
        let many: String =
            (0..MAX_PRESETS + 9).map(|index| format!("n{index:02} = size:2\n")).collect();
        let read = Presets::parse(&many);
        assert_eq!(read.list.len(), MAX_PRESETS);
        assert_eq!(read.list[MAX_PRESETS - 1].name, format!("n{}", MAX_PRESETS - 1));
    }

    /// The palette is the brushes that come with the application and then
    /// the user's. One of the user's with the name of one that came stands
    /// where that one stood, and taking it away puts that one back.
    #[test]
    fn the_palette_is_what_came_and_then_what_was_kept() {
        let came = core();
        let mut presets = Presets::default();
        let palette = presets.palette();
        assert_eq!(palette.len(), came.len());
        assert!(
            palette
                .iter()
                .zip(came)
                .all(|((shown, place), came)| { *shown == came && *place == Place::Core })
        );

        let mine = Preset { size_mm: Some(7.0), ..Preset::plain("Zigzag") };
        let clay = Preset { strength: Some(0.5), ..Preset::classic("Clay", BrushKind::Clay) };
        assert!(presets.put(mine.clone()) && presets.put(clay.clone()));
        let palette = presets.palette();
        assert_eq!(palette.len(), came.len() + 1, "the changed one was shown twice");
        let at = came.iter().position(|came| came.name == "Clay").expect("Clay comes with it");
        assert_eq!(palette[at], (&clay, Place::Changed));
        assert_eq!(palette[came.len()], (&mine, Place::Own));
        for (index, (shown, place)) in palette.iter().enumerate() {
            if index != at && index < came.len() {
                assert_eq!((*shown, *place), (&came[index], Place::Core));
            }
        }

        presets.remove("Clay");
        assert_eq!(presets.palette()[at], (&came[at], Place::Core), "Clay did not come back");
    }

    /// Whether the brush in hand is a brush on the palette: in everything
    /// but its name, and in its size and strength only when the brush on
    /// the palette says what they are.
    #[test]
    fn a_brush_is_the_brush_in_hand_by_what_it_says() {
        let now = Preset { size_mm: Some(3.0), strength: Some(0.15), ..Preset::plain("") };
        assert!(Preset::plain("Draw").is(&now), "a brush that says nothing of its size");
        assert!(Preset { size_mm: Some(3.0), ..Preset::plain("x") }.is(&now));
        assert!(!Preset { size_mm: Some(2.0), ..Preset::plain("x") }.is(&now));
        assert!(!Preset { strength: Some(0.3), ..Preset::plain("x") }.is(&now));
        assert!(!Preset { lazy_px: 1.0, ..Preset::plain("x") }.is(&now));
        assert!(!Preset::classic("x", BrushKind::Clay).is(&now));
        assert!(!Preset::doing("x", Actions::CREASE).is(&now));
        let round = Profile { shape: Shape::Round, plateau: 0.0 };
        assert!(!Preset { profile: round, ..Preset::plain("x") }.is(&now));
    }

    /// A brush is a file of its own, named after it, and comes back from
    /// the directory; forgetting it takes the file away; and a file
    /// somebody else made, whatever it is called and however many brushes
    /// it holds, is read too.
    #[test]
    fn a_brush_is_a_file_of_its_own() {
        let directory = scratch("brush-files");
        let there = || {
            let mut names: Vec<String> = files_in(&directory)
                .iter()
                .filter_map(|file| Some(file.file_name()?.to_string_lossy().into_owned()))
                .collect();
            names.sort();
            names
        };
        let load = || Presets::load_from(Some(&directory), None, false);
        assert_eq!(load(), Presets::default(), "an empty directory is no brushes");

        let crease = made("Crease: FINE", Actions::CREASE);
        let scales = full("Écailles 2");
        write_in(&directory, &crease).expect("writing");
        write_in(&directory, &scales).expect("writing");
        assert_eq!(there(), ["crease-fine.brush", "écailles-2.brush"]);
        let mut kept = Presets::default();
        assert!(kept.put(crease.clone()) && kept.put(scales.clone()));
        assert_eq!(load(), kept);

        // Kept again, changed: one file still, and the change in it.
        let changed = Preset { lazy_px: 40.0, ..crease.clone() };
        write_in(&directory, &changed).expect("writing");
        assert_eq!(there().len(), 2);
        assert_eq!(load().list[0], changed);

        // Two names that come to one file name are two files.
        let other = Preset::plain("crease fine");
        write_in(&directory, &other).expect("writing");
        assert_eq!(there(), ["crease-fine-2.brush", "crease-fine.brush", "écailles-2.brush"]);
        assert_eq!(load().list.len(), 3);

        // A pack from somebody else, and a file that is not a brush.
        std::fs::write(directory.join("From a friend.BRUSH"), "One = brush:Clay\nTwo = size:2\n")
            .expect("writing");
        std::fs::write(directory.join("notes.txt"), "Three = brush:Clay\n").expect("writing");
        let names: Vec<String> = load().list.into_iter().map(|preset| preset.name).collect();
        // In the order of their names, which is the order of their
        // characters and not a dictionary's: an accented capital comes
        // after the plain letters.
        assert_eq!(names, ["crease fine", "Crease: FINE", "One", "Two", "Écailles 2"]);

        // Forgetting one out of a pack leaves the rest of the pack, and
        // forgetting the last of a file takes the file.
        strike(&directory, "One").expect("striking");
        assert_eq!(load().list.len(), 4);
        assert!(directory.join("From a friend.BRUSH").is_file());
        strike(&directory, "Two").expect("striking");
        assert!(!directory.join("From a friend.BRUSH").exists());
        strike(&directory, "Crease: FINE").expect("striking");
        assert_eq!(there(), ["crease-fine-2.brush", "écailles-2.brush"]);
        strike(&directory, "nobody").expect("striking a brush that is not there");
        assert_eq!(load().list.len(), 2);
        assert!(directory.join("notes.txt").is_file(), "a file that is no brush was touched");
        std::fs::remove_dir_all(&directory).ok();
    }

    /// What a brush's file is called is made of its name and is a name a
    /// file can have.
    #[test]
    fn a_file_is_named_after_its_brush() {
        assert_eq!(stem_of("Fine crease"), "fine-crease");
        assert_eq!(stem_of("  A/B\\C: d?  "), "a-b-c-d");
        assert_eq!(stem_of("..."), "brush");
        assert_eq!(stem_of("Ünïcode 世界"), "ünïcode-世界");
        for name in ["../../etc/passwd", "a\0b", "CON", "x".repeat(80).as_str(), "*?<>|\""] {
            let stem = stem_of(name);
            assert!(!stem.is_empty());
            assert!(
                stem.chars().all(|character| character.is_alphanumeric() || character == '-'),
                "{stem:?} from {name:?}"
            );
        }
    }

    /// While there is no directory of brushes, the one file they used to
    /// be in is read, from the data directory or from the settings
    /// directory it was in before that, and what it holds is moved into
    /// files of the new kind. Once there is a directory the old file is
    /// not looked at again.
    #[test]
    fn brushes_kept_in_the_old_file_are_moved_to_their_own() {
        let root = scratch("brush-places");
        let (here, before) = (root.join("new.conf"), root.join("old.conf"));
        let both = || (Some(here.clone()), Some(before.clone()));
        // Neither is there: the later place, which reads as no file.
        let (new, old) = both();
        assert_eq!(first_there(new, old), Some(here.clone()));
        // Only the first one.
        std::fs::write(&before, "Old = brush:Clay\nLine = studio:Crease, lazy:12\n")
            .expect("writing");
        let (new, old) = both();
        assert_eq!(first_there(new, old), Some(before.clone()));
        // Both: the later one.
        std::fs::write(&here, "New = brush:Draw\n").expect("writing");
        let (new, old) = both();
        assert_eq!(first_there(new, old), Some(here.clone()));
        // No data directory at all on this machine: the first place.
        assert_eq!(first_there(None, Some(before.clone())), Some(before.clone()));
        assert_eq!(first_there(None, None), None);

        let directory = root.join("brushes");
        // Read without moving in, which is what a test of the application
        // does: the brushes are found and nothing is written.
        let found = Presets::load_from(Some(&directory), Some(&before), false);
        let names: Vec<&str> = found.list.iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(names, ["Line", "Old"]);
        assert_eq!(found.list[0].custom, Some(Actions::CREASE));
        assert!(!directory.exists());
        // Moved in: the same brushes, now a file each, and the old file
        // still where it was.
        assert_eq!(Presets::load_from(Some(&directory), Some(&before), true), found);
        assert_eq!(files_in(&directory).len(), 2);
        assert!(before.is_file());
        // And from then on the directory is what is read.
        std::fs::write(&before, "Changed = brush:Draw\n").expect("writing");
        assert_eq!(Presets::load_from(Some(&directory), Some(&before), true), found);
        // No file anywhere is no brushes kept.
        assert_eq!(Presets::load_from(None, None, true), Presets::default());
        assert_eq!(
            Presets::load_from(Some(&root.join("nowhere")), Some(&root.join("nothing")), false),
            Presets::default()
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// The file is a user's to edit, so anything may be in it. Whatever is,
    /// reading it does not panic, and every preset read has a usable name
    /// and every number in its range.
    #[test]
    fn a_damaged_file_reads_without_panicking_and_into_range() {
        let mut good = Presets::default();
        for preset in [
            full("Scales"),
            made("Crease", Actions::CREASE),
            made(
                "Scraper",
                Actions { level: 0.7, plane: -0.2, side: Side::Cut, ..Actions::default() },
            ),
            Preset::plain("Plain"),
        ] {
            good.put(preset);
        }
        let good = good.to_config();
        let pool: Vec<char> =
            "=:,#\n \t.-+eE0123456789NaNinfBrushstampheightpushlevelside\u{0}\u{7f}\u{e9}\u{4e16}"
                .chars()
                .collect();
        // A small generator of its own, so that the damage is the same
        // damage on every run.
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move |below: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % below as u64) as usize
        };
        let (mut kept, mut changed) = (0_usize, 0_usize);
        for _ in 0..4000 {
            let mut text: Vec<char> = good.chars().collect();
            for _ in 0..1 + next(6) {
                let at = next(text.len());
                match next(3) {
                    0 => text[at] = pool[next(pool.len())],
                    1 => text.insert(at, pool[next(pool.len())]),
                    _ => {
                        text.remove(at);
                    }
                }
            }
            let text: String = text.into_iter().collect();
            let read = Presets::parse(&text);
            assert!(read.list.len() <= MAX_PRESETS);
            assert!(read.is_in_order(), "{text:?}");
            for preset in &read.list {
                assert_eq!(named(&preset.name).as_deref(), Some(preset.name.as_str()), "{text:?}");
                assert!(preset.is_settled(), "{text:?}");
                assert!(
                    preset.size_mm.is_none_or(|size| (MIN_RADIUS_MM..=BIGGEST_MM).contains(&size)),
                    "{text:?}"
                );
                assert!(
                    preset
                        .strength
                        .is_none_or(|strength| (MIN_STRENGTH..=MAX_STRENGTH).contains(&strength)),
                    "{text:?}"
                );
                assert!(LAZY_RANGE.contains(&preset.lazy_px), "{text:?}");
                assert!(SIZE_RANGE.contains(&preset.size_from_pressure), "{text:?}");
                if let Some((_, height)) = preset.stamp {
                    assert!(HEIGHT_RANGE.contains(&height), "{text:?}");
                    assert!(preset.along == 0.0 || ALONG_RANGE.contains(&preset.along), "{text:?}");
                }
                // And what was read is itself a file that reads back.
                let alone = Presets { list: vec![preset.clone()] };
                assert_eq!(Presets::parse(&alone.to_config()), alone, "{text:?}");
            }
            kept += read.list.len();
            changed += usize::from(read != Presets::parse(&good));
        }
        // Controls: the damage was survivable often enough to have tested
        // the reader (most presets were still read), and was damage (most
        // files read differently from the good one).
        assert!(kept > 4000 * 3, "only {kept} presets were read from 4000 damaged files");
        assert!(changed > 1000, "only {changed} of 4000 damaged files read differently");
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn actions() -> impl Strategy<Value = Actions> {
            // Mostly in range and on a slider's steps, sometimes well out.
            let amount =
                || prop_oneof![(-20_i32..=20).prop_map(|step| step as f32 * 0.05), -3.0_f32..3.0];
            let side = prop::sample::select(Side::ALL.to_vec());
            (amount(), amount(), amount(), amount(), amount(), amount(), side).prop_map(
                |(push, inflate, pinch, smooth, level, plane, side)| Actions {
                    push,
                    inflate,
                    pinch,
                    smooth,
                    level,
                    plane,
                    side,
                },
            )
        }

        fn stamp() -> impl Strategy<Value = Option<(StampOf, f32)>> {
            let picture = prop_oneof![
                Just(StampOf::InHand),
                prop::sample::select(BuiltIn::ALL.to_vec()).prop_map(StampOf::BuiltIn),
            ];
            prop::option::of((picture, 0.02_f32..=1.0))
        }

        prop_compose! {
            /// A brush as the panel could have it in hand, or a file give
            /// it, before it is settled.
            fn preset()(
                kind in prop::sample::select(BrushKind::ALL.to_vec()),
                custom in prop::option::of(actions()),
                shape in prop::sample::select(Shape::ALL.to_vec()),
                plateau in -0.5_f32..1.5,
                size_mm in prop::option::of(MIN_RADIUS_MM..=BIGGEST_MM),
                strength in prop::option::of(MIN_STRENGTH..=MAX_STRENGTH),
                stamp in stamp(),
                along in prop_oneof![Just(0.0_f32), 0.5_f32..=4.0],
                lazy_px in prop_oneof![Just(0.0_f32), 0.0_f32..=80.0],
                size_from_pressure in 0.0_f32..=1.0,
            ) -> Preset {
                Preset {
                    name: "A brush".to_string(),
                    kind,
                    custom,
                    profile: Profile { shape, plateau },
                    size_mm,
                    strength,
                    stamp,
                    along,
                    lazy_px,
                    size_from_pressure,
                }
            }
        }

        proptest! {
            /// Whatever a brush is, settled it is a brush a preset can be,
            /// settling it again changes nothing, and written to a file it
            /// comes back the same brush.
            #[test]
            fn any_brush_settles_and_comes_back_from_its_file(preset in preset()) {
                let settled = preset.settled();
                prop_assert!(settled.is_settled(), "{:?}", settled);
                prop_assert_eq!(settled.clone().settled(), settled.clone());
                prop_assert!(settled.is(&settled));
                let back = Presets::parse(&settled.line());
                prop_assert_eq!(back.list, vec![settled]);
            }

            /// Amounts as a brush holds them are the same value for the
            /// same brush: held again they do not change, and the classic
            /// brush they go by is their family.
            #[test]
            fn amounts_are_held_one_way(actions in actions()) {
                let (kind, custom) = held(actions);
                let does = custom.or_else(|| Actions::of(kind)).expect("a field brush");
                prop_assert!(does.is_legal());
                prop_assert_eq!(does.family(), kind);
                prop_assert_eq!(held(does), (kind, custom));
            }
        }
    }
}
