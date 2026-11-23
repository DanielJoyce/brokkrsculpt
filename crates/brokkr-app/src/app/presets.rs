// SPDX-License-Identifier: AGPL-3.0-only

//! Brush presets: a brush as it is set up, kept under a name.
//!
//! A brush here is a dozen settings in two places, the classic brush's
//! (which brush, how big, how hard, which falloff) and the studio block's
//! (an operator or the stamp, a lazy stroke, size from pressure). Getting a
//! fine crease or a row of rivets back means setting all of them again. A
//! preset is all of them at once, under a name the user gave it.
//!
//! # The file
//!
//! `brushes.conf` in the config directory, the flat `key = value` shape
//! `filaments.conf` and `printer.conf` already use
//! ([`crate::paths::entries`]). A line is a preset: its name, and after the
//! `=` its settings as `key:value` pairs with commas between them.
//!
//! ```text
//! Fine crease = brush:Draw, size:1.5, strength:0.3, studio:Crease, lazy:12
//! Rivets = brush:Draw, size:2, stamp:Rivet, height:0.4
//! Rivet row = brush:Draw, size:1, stamp:Rivet, along:1.5
//! ```
//!
//! **A setting this module cannot make sense of is left at its default and
//! the rest of the line is kept**, and a number out of range is brought
//! into it. The file is the user's to edit by hand, and the failure worth
//! avoiding is a typo in one setting throwing away a brush, or a typed
//! `size:900` costing a second a stamp.
//!
//! # What a preset does not hold
//!
//! * **A stamp picture read from a file.** A preset saved with one in hand
//!   says "the stamp, with the picture in hand" and uses whichever picture
//!   that is when it is chosen. The pictures that come with the
//!   application are held by name.
//! * **A surface pattern**, which is a dozen settings of its own with its
//!   own panel.
//! * **The paint brush's touch** (drybrush, sponge, fill), which belongs to
//!   the paint rack.

use brokkr_core::brush::alpha::BuiltIn;
use brokkr_core::brush::engine::FieldOp;
use brokkr_core::{BrushKind, FalloffCurve};

use super::studio::{ALONG_RANGE, HEIGHT_RANGE, LAZY_RANGE, SIZE_RANGE};
use super::{MAX_STRENGTH, MIN_RADIUS_MM, MIN_STRENGTH};

/// The file the presets live in.
const FILE: &str = "brushes.conf";

/// The most presets kept. The panel shows them all, two to a row.
pub(super) const MAX_PRESETS: usize = 24;

/// The longest a preset's name may be, in characters: what fits a button.
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
    pub kind: BrushKind,
    pub radius_mm: f32,
    pub strength: f32,
    pub falloff: FalloffCurve,
    /// A studio operator in place of the classic brush's own.
    pub op: Option<FieldOp>,
    /// The stamp, and how tall it is as a share of half its width. A preset
    /// has an operator or the stamp and not both: the stamp wins.
    pub stamp: Option<(StampOf, f32)>,
    /// How far apart the stamp is laid along a drag, in stamp widths, or
    /// nothing for one stamp sized by the drag. Nothing without a stamp.
    pub along: f32,
    pub lazy_px: f32,
    pub size_from_pressure: f32,
}

impl Preset {
    /// The classic Draw brush as it starts, under `name`.
    pub fn plain(name: &str) -> Self {
        let brush = brokkr_core::Brush::default();
        Self {
            name: name.to_string(),
            kind: brush.kind,
            radius_mm: brush.radius,
            strength: brush.strength,
            falloff: brush.falloff,
            op: None,
            stamp: None,
            along: 0.0,
            lazy_px: 0.0,
            size_from_pressure: 0.0,
        }
    }

    /// The settings half of a line of the file.
    fn settings(&self) -> String {
        let mut fields = vec![
            format!("brush:{}", self.kind.label()),
            format!("size:{}", self.radius_mm),
            format!("strength:{}", self.strength),
            format!("falloff:{}", self.falloff.label()),
        ];
        if let Some(op) = self.op {
            fields.push(format!("studio:{}", op.label()));
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
    fn take(&mut self, settings: &str) {
        // The stamp's height may come before the stamp on a line somebody
        // wrote by hand.
        let mut height = None;
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
                "size" => {
                    if let Some(size) = number(MIN_RADIUS_MM..=BIGGEST_MM) {
                        self.radius_mm = size;
                    }
                }
                "strength" => {
                    if let Some(strength) = number(MIN_STRENGTH..=MAX_STRENGTH) {
                        self.strength = strength;
                    }
                }
                "falloff" => {
                    let curves = [
                        FalloffCurve::Smooth,
                        FalloffCurve::Linear,
                        FalloffCurve::Sharp,
                        FalloffCurve::Wide,
                    ];
                    if let Some(curve) = curves.into_iter().find(|curve| named(curve.label())) {
                        self.falloff = curve;
                    }
                }
                "studio" => {
                    if let Some(op) = FieldOp::STUDIO.into_iter().find(|op| named(op.label())) {
                        self.op = Some(op);
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
        // An operator or the stamp, as the studio block itself has it,
        // and a row only of stamps.
        if self.stamp.is_some() {
            self.op = None;
        } else {
            self.along = 0.0;
        }
    }
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

/// The presets this machine has.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Presets {
    pub list: Vec<Preset>,
}

impl Default for Presets {
    /// What a machine with no file has: five brushes that show what the
    /// studio block is for, and which are a user's to change or remove.
    fn default() -> Self {
        let plain = Preset::plain("");
        let named = |name: &str| Preset { name: name.to_string(), ..plain.clone() };
        Self {
            list: vec![
                Preset {
                    radius_mm: 1.5,
                    strength: 0.3,
                    op: Some(FieldOp::Crease),
                    lazy_px: 12.0,
                    ..named("Fine crease")
                },
                Preset { radius_mm: 2.0, lazy_px: 30.0, ..named("Long line") },
                Preset {
                    stamp: Some((StampOf::BuiltIn(BuiltIn::Rivet), DEFAULT_HEIGHT)),
                    ..named("Rivets")
                },
                Preset {
                    radius_mm: 1.0,
                    stamp: Some((StampOf::BuiltIn(BuiltIn::Rivet), DEFAULT_HEIGHT)),
                    along: 1.5,
                    ..named("Rivet row")
                },
                Preset {
                    kind: BrushKind::Clay,
                    radius_mm: 6.0,
                    strength: 0.25,
                    size_from_pressure: 0.5,
                    ..named("Clay build")
                },
            ],
        }
    }
}

impl Presets {
    /// The presets on this machine, or the ones it starts with when there
    /// is no file.
    pub fn load() -> Self {
        Self::load_from(crate::paths::config_file(FILE).as_deref())
    }

    /// Split out so the tests can point it at a file of their own.
    pub fn load_from(path: Option<&std::path::Path>) -> Self {
        let Some(text) = path.and_then(|path| std::fs::read_to_string(path).ok()) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    /// Read presets out of the config text.
    ///
    /// A file that is there and names none is no presets, and not the
    /// starting five: somebody removed them. A name used twice is one
    /// preset, the later line's, where the earlier one stood.
    pub fn parse(text: &str) -> Self {
        let mut presets = Self { list: Vec::new() };
        for (key, value) in crate::paths::entries(text) {
            let Some(name) = named(key) else {
                continue;
            };
            let mut preset = Preset::plain(&name);
            preset.take(value);
            presets.put(preset);
        }
        presets
    }

    /// The config text for these presets, ready to write back.
    pub fn to_config(&self) -> String {
        let mut text = String::from(
            "# BrokkrSculpt brush presets: a name, then its settings.\n\
             # Written when a preset is saved or removed, and safe to edit by hand.\n",
        );
        for preset in &self.list {
            text.push_str(&format!("{} = {}\n", preset.name, preset.settings()));
        }
        text
    }

    /// Keep a preset: in place of the one with its name, or at the end.
    /// `false`, and nothing kept, when there is no room for another.
    pub fn put(&mut self, preset: Preset) -> bool {
        if let Some(held) = self.list.iter_mut().find(|held| held.name == preset.name) {
            *held = preset;
            return true;
        }
        if self.list.len() >= MAX_PRESETS {
            return false;
        }
        self.list.push(preset);
        true
    }

    /// Write the presets to this machine's config file.
    ///
    /// **Inert under `cfg(test)`**, as `Palette::save` is and for its
    /// reason: an application test that saves a preset goes through the
    /// real `update`, and would otherwise replace the presets of whoever
    /// ran the tests.
    pub fn save(&self) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        let Some(path) = crate::paths::config_file(FILE) else {
            return Err("no config directory to write to".to_string());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|why| format!("could not create {}: {why}", parent.display()))?;
        }
        std::fs::write(&path, self.to_config())
            .map_err(|why| format!("could not write {}: {why}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every setting a preset has, none of them at its default.
    fn full(name: &str) -> Preset {
        Preset {
            name: name.to_string(),
            kind: BrushKind::Clay,
            radius_mm: 4.25,
            strength: 0.33,
            falloff: FalloffCurve::Sharp,
            op: None,
            stamp: Some((StampOf::BuiltIn(BuiltIn::Scale), 0.6)),
            along: 2.5,
            lazy_px: 17.5,
            size_from_pressure: 0.4,
        }
    }

    /// Presets written and read back are the presets that were written:
    /// every setting, for an operator, for each kind of stamp and for a
    /// plain brush, in the order they were in.
    #[test]
    fn presets_come_back_from_their_file_as_they_went_in() {
        let presets = Presets {
            list: vec![
                full("Scales, big"),
                Preset { stamp: Some((StampOf::InHand, 0.11)), ..full("My own picture") },
                Preset {
                    stamp: None,
                    along: 0.0,
                    op: Some(FieldOp::Crease),
                    ..full("Crease: fine")
                },
                Preset { along: 0.0, ..full("One scale") },
                Preset::plain("Plain"),
                Preset { kind: BrushKind::Paint, falloff: FalloffCurve::Wide, ..full("Paint") },
                Preset { kind: BrushKind::Move, falloff: FalloffCurve::Linear, ..full("Move") },
            ],
        };
        let back = Presets::parse(&presets.to_config());
        assert_eq!(back, presets);
        // Control: the fixture is not all defaults, so "it came back" is
        // not "nothing was read and the defaults matched".
        assert_ne!(Preset { name: String::new(), ..full("") }, Preset::plain(""));
        // And the starting presets are themselves a file that reads back.
        assert_eq!(Presets::parse(&Presets::default().to_config()), Presets::default());
        assert_eq!(Presets::default().list.len(), 5);
    }

    /// Every brush, falloff, operator and picture there is can be named in
    /// the file, in any letter case.
    #[test]
    fn every_brush_falloff_operator_and_picture_can_be_named() {
        for kind in BrushKind::ALL {
            let line = format!("a = brush:{}", kind.label().to_uppercase());
            assert_eq!(Presets::parse(&line).list[0].kind, kind, "{line}");
        }
        for curve in
            [FalloffCurve::Smooth, FalloffCurve::Linear, FalloffCurve::Sharp, FalloffCurve::Wide]
        {
            let line = format!("a = falloff:{}", curve.label().to_lowercase());
            assert_eq!(Presets::parse(&line).list[0].falloff, curve, "{line}");
        }
        for op in FieldOp::STUDIO {
            let line = format!("a = studio:{}", op.label());
            assert_eq!(Presets::parse(&line).list[0].op, Some(op), "{line}");
        }
        for kind in BuiltIn::ALL {
            let line = format!("a = stamp:{}", kind.label());
            let stamp = Presets::parse(&line).list[0].stamp;
            assert_eq!(stamp, Some((StampOf::BuiltIn(kind), DEFAULT_HEIGHT)), "{line}");
        }
        // An operator that has a classic brush is chosen as that brush and
        // is not a studio operator.
        assert_eq!(Presets::parse("a = studio:Draw").list[0].op, None);
    }

    /// A setting that makes no sense is left at its default and the rest of
    /// the line is kept. A number out of range is brought into it.
    #[test]
    fn a_bad_setting_costs_only_itself() {
        let plain = Preset::plain("a");
        let read = |settings: &str| Presets::parse(&format!("a = {settings}")).list.remove(0);

        let preset = read(
            "brush:Chisel, size:big, strength:, falloff:9, studio:Nothing, stamp:Cog, lazy:x, \
             pressure:NaN, height:inf, colour:red, , no colon here, :, lazy:20",
        );
        assert_eq!(preset, Preset { lazy_px: 20.0, ..plain.clone() });

        let preset = read("size:900, strength:7, lazy:-4, pressure:3, stamp:Dome, height:55");
        assert_eq!(preset.radius_mm, BIGGEST_MM);
        assert_eq!(preset.strength, MAX_STRENGTH);
        assert_eq!(preset.lazy_px, *LAZY_RANGE.start());
        assert_eq!(preset.size_from_pressure, *SIZE_RANGE.end());
        assert_eq!(preset.stamp, Some((StampOf::BuiltIn(BuiltIn::Dome), *HEIGHT_RANGE.end())));

        let preset = read("size:0, strength:0, height:0.5");
        assert_eq!(preset.radius_mm, MIN_RADIUS_MM);
        assert_eq!(preset.strength, MIN_STRENGTH);
        assert_eq!(preset.stamp, None, "a height alone took up the stamp");

        // A height before its stamp on the line still counts, and a stamp
        // and an operator on one line is the stamp.
        let preset = read("height:0.8, studio:Crease, stamp:Bar");
        assert_eq!(preset.stamp, Some((StampOf::BuiltIn(BuiltIn::Bar), 0.8)));
        assert_eq!(preset.op, None);
        // Control: the operator on its own is taken.
        assert_eq!(read("studio:Crease").op, Some(FieldOp::Crease));

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

    /// A name used again replaces the preset where it stood, and there is
    /// room for only so many.
    #[test]
    fn a_name_used_again_replaces_and_the_list_has_an_end() {
        let mut presets = Presets::parse("a = size:1\nb = size:2\na = size:3\n");
        assert_eq!(presets.list.len(), 2);
        assert_eq!((presets.list[0].name.as_str(), presets.list[0].radius_mm), ("a", 3.0));
        assert_eq!((presets.list[1].name.as_str(), presets.list[1].radius_mm), ("b", 2.0));

        for index in 0..MAX_PRESETS + 5 {
            presets.put(Preset::plain(&format!("p{index}")));
        }
        assert_eq!(presets.list.len(), MAX_PRESETS);
        assert!(!presets.put(Preset::plain("one more")), "there was room past the end");
        // Replacing one that is there is not adding one.
        assert!(presets.put(Preset { radius_mm: 9.0, ..Preset::plain("b") }));
        assert_eq!(presets.list[1].radius_mm, 9.0);
        assert_eq!(presets.list.len(), MAX_PRESETS);

        // A file with more lines than that keeps the first of them.
        let many: String =
            (0..MAX_PRESETS + 9).map(|index| format!("n{index} = size:2\n")).collect();
        let read = Presets::parse(&many);
        assert_eq!(read.list.len(), MAX_PRESETS);
        assert_eq!(read.list[MAX_PRESETS - 1].name, format!("n{}", MAX_PRESETS - 1));
    }

    /// No file is the starting presets. A file with nothing in it is none.
    #[test]
    fn no_file_is_the_starting_presets_and_an_empty_file_is_none() {
        let directory = std::env::temp_dir().join(format!("brokkr-presets-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        let path = directory.join("brushes.conf");
        assert_eq!(Presets::load_from(Some(&path)), Presets::default());
        assert_eq!(Presets::load_from(None), Presets::default());

        std::fs::write(&path, "# nothing kept\n").expect("writing the file");
        assert_eq!(Presets::load_from(Some(&path)).list, Vec::new());

        let presets = Presets { list: vec![full("One"), Preset::plain("Two")] };
        std::fs::write(&path, presets.to_config()).expect("writing the file");
        assert_eq!(Presets::load_from(Some(&path)), presets);
        std::fs::remove_dir_all(&directory).ok();
    }

    /// The file is a user's to edit, so anything may be in it. Whatever is,
    /// reading it does not panic, and every preset read has a usable name
    /// and every number in its range.
    #[test]
    fn a_damaged_file_reads_without_panicking_and_into_range() {
        let good = Presets {
            list: vec![
                full("Scales"),
                Preset { stamp: None, along: 0.0, op: Some(FieldOp::Crease), ..full("Crease") },
                Preset::plain("Plain"),
            ],
        }
        .to_config();
        let pool: Vec<char> =
            "=:,#\n \t.-+eE0123456789NaNinfBrushstampheight\u{0}\u{7f}\u{e9}\u{4e16}"
                .chars()
                .collect();
        // A small generator of its own: the tests have no random crate.
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
            for preset in &read.list {
                assert_eq!(named(&preset.name).as_deref(), Some(preset.name.as_str()), "{text:?}");
                assert!((MIN_RADIUS_MM..=BIGGEST_MM).contains(&preset.radius_mm), "{text:?}");
                assert!((MIN_STRENGTH..=MAX_STRENGTH).contains(&preset.strength), "{text:?}");
                assert!(LAZY_RANGE.contains(&preset.lazy_px), "{text:?}");
                assert!(SIZE_RANGE.contains(&preset.size_from_pressure), "{text:?}");
                if let Some((_, height)) = preset.stamp {
                    assert!(HEIGHT_RANGE.contains(&height), "{text:?}");
                    assert_eq!(preset.op, None, "{text:?}");
                    assert!(preset.along == 0.0 || ALONG_RANGE.contains(&preset.along), "{text:?}");
                } else {
                    assert_eq!(preset.along, 0.0, "{text:?}");
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
        assert!(kept > 4000 * 2, "only {kept} presets were read from 4000 damaged files");
        assert!(changed > 1000, "only {changed} of 4000 damaged files read differently");
    }
}
