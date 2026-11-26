// SPDX-License-Identifier: AGPL-3.0-only

//! The controls every panel is built from.
//!
//! One function a kind of control, so that a section heading, a number, a
//! choice of a few and a button look and behave the same wherever they are.
//! Before this the panels had five ways of laying out a slider, two kinds of
//! heading that looked alike and only one of which folded, and buttons in
//! three text sizes. `docs/WORKSPACES.md` is the design this follows.
//!
//! # A number
//!
//! [`number`] is a label, a field that takes typing and a slider under them.
//! What is typed is held as a draft, by a key the caller names, until it is
//! submitted with enter; a submit that does not read as a number drops the
//! draft and leaves the value where it was. Both the slider and a submit
//! send [`Message::FieldSet`], which clears the draft before the message it
//! carries is acted on, so a draft never outlives the value it was typed
//! against. `Hani::update` unwraps that message first, before any of the
//! guards that look at what a message is, so the carried message meets them
//! exactly as it would have without the wrapper.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use iced::widget::{Space, button, column, container, row, slider, text, text_input, tooltip};
use iced::{Alignment, Element, Length};

use crate::icon::{self, IconName};
use crate::message::Message;
use crate::theme;

/// What is being typed into number fields, by key.
pub type Drafts = HashMap<&'static str, String>;

/// The width of a number's typed field.
const FIELD_WIDTH: f32 = 68.0;

/// The width of an inline number's slider.
const INLINE_SLIDER: f32 = 76.0;

/// A label inside a section, above a group of controls.
pub fn sub_label<'a>(label: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
    text(label).size(theme::LABEL_SIZE).color(theme::TEXT_MUTE)
}

/// A section's heading: a caret and a title, which folds the section.
///
/// Every heading in a panel is one of these, and every one folds. `extra`
/// is drawn beside it and not inside it, so pressing it does not fold the
/// section (the Bodies heading's SOLO badge is one).
pub fn heading<'a>(
    title: &'a str,
    open: bool,
    on_toggle: Message,
    extra: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let heading = button(
        row![
            icon::icon(
                if open { IconName::CaretDown } else { IconName::CaretRight },
                theme::ICON_INLINE,
                theme::TEXT_MUTE,
            ),
            text(title).size(theme::HEADING_SIZE).color(theme::TEXT_MUTE),
        ]
        .align_y(Alignment::Center)
        .spacing(theme::S2),
    )
    .width(Length::Fill)
    .padding(0)
    .style(theme::section_heading)
    .on_press(on_toggle);
    match extra {
        Some(extra) => row![heading, extra].align_y(Alignment::Center).spacing(theme::S2).into(),
        None => heading.into(),
    }
}

/// A heading and, when it is open, what is under it.
pub fn section<'a>(
    title: &'a str,
    open: bool,
    on_toggle: Message,
    extra: Option<Element<'a, Message>>,
    body: impl FnOnce() -> Element<'a, Message>,
) -> Element<'a, Message> {
    let heading = heading(title, open, on_toggle, extra);
    if open { column![heading, body()].spacing(theme::S2).into() } else { heading }
}

/// A smaller fold inside a section, for a block most people leave shut.
pub fn disclosure<'a>(title: &'a str, open: bool, on_toggle: Message) -> Element<'a, Message> {
    button(
        row![
            icon::icon(
                if open { IconName::CaretDown } else { IconName::CaretRight },
                theme::ICON_INLINE,
                theme::TEXT_DIM,
            ),
            text(title).size(theme::LABEL_SIZE).color(theme::TEXT_DIM),
        ]
        .align_y(Alignment::Center)
        .spacing(theme::S2),
    )
    .padding(0)
    .style(theme::section_heading)
    .on_press(on_toggle)
    .into()
}

/// A button of the ordinary weight.
pub fn button_secondary<'a>(
    label: impl text::IntoFragment<'a>,
    on_press: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    button(text(label).size(theme::LABEL_SIZE))
        .padding(theme::PAD_BUTTON)
        .style(theme::tool_button)
        .on_press_maybe(on_press)
}

/// A button that is one of a set, lit when it is the chosen one.
pub fn toggle<'a>(
    label: impl text::IntoFragment<'a>,
    selected: bool,
    on_press: Message,
) -> iced::widget::Button<'a, Message> {
    let (style, _) = theme::tool_toggle(selected);
    button(text(label).size(theme::LABEL_SIZE).align_x(Alignment::Center).width(Length::Fill))
        .padding(theme::PAD_CHOICE)
        .style(style)
        .on_press(on_press)
}

/// A choice of a few, side by side, one of them lit.
///
/// For two to five choices. More than that wants a grid of tiles or a
/// drop-down list.
pub fn segmented<'a, T: Copy + PartialEq + 'a>(
    choices: impl IntoIterator<Item = (T, &'a str)>,
    selected: T,
    on_choose: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message> {
    choices
        .into_iter()
        .fold(row![].spacing(theme::S1), |assembled, (value, name)| {
            // A share of the row for each letter and two for the padding:
            // in equal shares, seen on a screen, "Smooth" ran past the edges
            // of its button where "Wide" had room to spare.
            let share = name.len() as u16 + 2;
            assembled.push(
                toggle(name, value == selected, on_choose(value)).width(Length::FillPortion(share)),
            )
        })
        .into()
}

/// A name shown when the pointer rests on `content`.
///
/// Every button that shows only an icon has one: a test in this module
/// fails on one that does not.
pub fn tip<'a>(
    content: impl Into<Element<'a, Message>>,
    name: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(name).size(theme::LABEL_SIZE).color(theme::TEXT))
            .padding(theme::PAD_TIP)
            .style(theme::menu_card),
        tooltip::Position::Bottom,
    )
    .gap(theme::S1)
    .snap_within_viewport(true)
    .into()
}

/// A number: a label, a field that takes typing, and a slider.
///
/// Built with [`number`] and given to a column like any element. `key` names
/// the number for its draft, and must be unique among the numbers on screen
/// at once.
pub struct Number<'a> {
    drafts: &'a Drafts,
    key: &'static str,
    label: String,
    value: f32,
    range: RangeInclusive<f32>,
    step: f32,
    shown: Option<String>,
    trailing: Option<Element<'a, Message>>,
    on_release: Option<Message>,
    tip: Option<&'static str>,
    inline: bool,
    on_change: Box<dyn Fn(f32) -> Message + 'a>,
}

/// Start a [`Number`].
pub fn number<'a>(
    drafts: &'a Drafts,
    key: &'static str,
    label: impl Into<String>,
    value: f32,
    range: RangeInclusive<f32>,
    on_change: impl Fn(f32) -> Message + 'a,
) -> Number<'a> {
    Number {
        drafts,
        key,
        label: label.into(),
        value,
        range,
        step: 0.01,
        shown: None,
        trailing: None,
        on_release: None,
        tip: None,
        inline: false,
        on_change: Box::new(on_change),
    }
}

impl<'a> Number<'a> {
    /// The slider's step. Typing is not held to it.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// How the value is written in its field, with its unit. Two places of
    /// decimals and no unit if this is not given.
    pub fn shown(mut self, shown: impl Into<String>) -> Self {
        self.shown = Some(shown.into());
        self
    }

    /// What is sent when the slider is let go, for a number whose drag is
    /// one undo entry or one save.
    pub fn on_release(mut self, message: Message) -> Self {
        self.on_release = Some(message);
        self
    }

    /// What the number's label says on hover: a key that sets it from the
    /// viewport, or what it works on.
    pub fn tip(mut self, tip: &'static str) -> Self {
        self.tip = Some(tip);
        self
    }

    /// Label, a short slider and the field on one line, for the context
    /// bar over the viewport, where there is width and no height.
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }

    /// Something beside the field, such as an Apply button.
    pub fn trailing(mut self, trailing: impl Into<Element<'a, Message>>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }
}

impl Number<'_> {
    /// What pressing enter in the field sends: what is typed, held to the
    /// range, through the same message the slider sends; or, when what is
    /// typed is not a number, the draft let go and the value left alone.
    pub fn submitted(&self) -> Message {
        let carried = self
            .drafts
            .get(self.key)
            .and_then(|typed| parse_number(typed))
            .map(|typed| (self.on_change)(typed.clamp(*self.range.start(), *self.range.end())))
            .unwrap_or(Message::FieldDropped(self.key));
        Message::FieldSet(self.key, Box::new(carried))
    }
}

impl<'a> From<Number<'a>> for Element<'a, Message> {
    fn from(number: Number<'a>) -> Self {
        let submit = number.submitted();
        let Number {
            drafts,
            key,
            label,
            value,
            range,
            step,
            shown,
            trailing,
            on_release,
            tip: hover,
            inline,
            on_change,
        } = number;
        let shown = shown.unwrap_or_else(|| format!("{value:.2}"));
        let draft = drafts.get(key);
        let field = text_input("", draft.map_or(shown.as_str(), String::as_str))
            .on_input(move |typed| Message::FieldEdited(key, typed))
            .on_submit(submit)
            .size(theme::LABEL_SIZE)
            .padding(theme::PAD_FIELD)
            .align_x(Alignment::End)
            .width(Length::Fixed(FIELD_WIDTH));
        let label: Element<'a, Message> = {
            let label = text(label).size(theme::LABEL_SIZE).color(theme::TEXT_DIM);
            match hover {
                Some(hover) => tip(label, hover),
                None => label.into(),
            }
        };
        let mut slide =
            slider(range, value, move |moved| Message::FieldSet(key, Box::new(on_change(moved))))
                .step(step);
        if let Some(released) = on_release {
            slide = slide.on_release(released);
        }
        if inline {
            let mut line = row![label, slide.width(Length::Fixed(INLINE_SLIDER)), field]
                .spacing(theme::S2)
                .align_y(Alignment::Center);
            if let Some(trailing) = trailing {
                line = line.push(trailing);
            }
            return line.into();
        }
        let mut top = row![label, Space::new().width(Length::Fill), field]
            .align_y(Alignment::Center)
            .spacing(theme::S2);
        if let Some(trailing) = trailing {
            top = top.push(trailing);
        }
        column![top, slide].spacing(theme::S1).into()
    }
}

/// The number in what was typed into a field: the first run of characters
/// that can make one, so that a unit typed after it or a sign before it does
/// no harm. `None` when there is none.
pub fn parse_number(typed: &str) -> Option<f32> {
    let start = typed.find(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')?;
    let run: String = typed[start..]
        .chars()
        .enumerate()
        .take_while(|&(at, c)| {
            c.is_ascii_digit() || c == '.' || (at == 0 && (c == '-' || c == '+'))
        })
        .map(|(_, c)| c)
        .collect();
    run.parse::<f32>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_number_is_read_past_its_unit_and_its_label() {
        assert_eq!(parse_number("3.00 mm"), Some(3.0));
        assert_eq!(parse_number("x 1.50"), Some(1.5));
        assert_eq!(parse_number("-0.5"), Some(-0.5));
        assert_eq!(parse_number("+0.50 mm"), Some(0.5));
        assert_eq!(parse_number(".25"), Some(0.25));
        assert_eq!(parse_number("100%"), Some(100.0));
        assert_eq!(parse_number("off"), None);
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("-"), None);
    }

    /// Every button that shows only an icon has its name on hover, which
    /// it can have only through [`tip`]: a source file with a
    /// `button(icon::icon(` not just inside a `crate::ui::tip(` fails here.
    #[test]
    fn every_button_that_is_only_an_icon_is_named_on_hover() {
        fn rust_files(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("the source directory reads") {
                let path = entry.expect("an entry reads").path();
                if path.is_dir() {
                    rust_files(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    found.push(path);
                }
            }
        }
        let mut files = Vec::new();
        rust_files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let mut unnamed = Vec::new();
        for path in files.iter().filter(|path| !path.ends_with("ui.rs")) {
            let source = std::fs::read_to_string(path).expect("a source file reads");
            let squeezed: String = source.split_whitespace().collect();
            for (at, _) in squeezed.match_indices("button(icon::icon(") {
                if !squeezed[..at].ends_with("crate::ui::tip(") {
                    let end = (at + 60).min(squeezed.len());
                    unnamed.push(format!("{}: {}", path.display(), &squeezed[at..end]));
                }
            }
        }
        assert!(unnamed.is_empty(), "icon-only buttons with no tooltip:\n{}", unnamed.join("\n"));
    }

    /// Pressing enter sends what the slider would send for the number typed,
    /// held to the range; a field nobody typed in, or typed nonsense into,
    /// lets the draft go and sends nothing else.
    #[test]
    fn a_typed_number_is_what_the_slider_would_send() {
        let carried = |drafts: &Drafts| {
            let number =
                number(drafts, "k", "Radius", 1.0, 0.5..=20.0, Message::BrushRadiusChanged);
            match number.submitted() {
                Message::FieldSet("k", carried) => format!("{carried:?}"),
                other => panic!("not a FieldSet under its own key: {other:?}"),
            }
        };
        let mut drafts = Drafts::new();
        assert_eq!(carried(&drafts), format!("{:?}", Message::FieldDropped("k")));
        drafts.insert("k", "4.5 mm".into());
        assert_eq!(carried(&drafts), format!("{:?}", Message::BrushRadiusChanged(4.5)));
        drafts.insert("k", "400".into());
        assert_eq!(carried(&drafts), format!("{:?}", Message::BrushRadiusChanged(20.0)));
        drafts.insert("k", "big".into());
        assert_eq!(carried(&drafts), format!("{:?}", Message::FieldDropped("k")));
    }

    proptest! {
        /// Whatever a field shows, with two places and a unit, reads back as
        /// the value it shows: so pressing enter on a field nobody changed
        /// sets what was already there.
        #[test]
        fn what_a_field_shows_reads_back(value in -1000.0_f32..1000.0, unit in "( mm| %|°|)") {
            let shown = format!("{value:.2}{unit}");
            let read = parse_number(&shown).expect("a shown number reads");
            let rounded: f32 = format!("{value:.2}").parse().unwrap();
            prop_assert_eq!(read, rounded);
        }
    }
}
