// SPDX-License-Identifier: AGPL-3.0-only

//! How big the interface is drawn: by the screen, or as chosen.
//!
//! Asked for by the maintainer on 2026-11-27: "some buttons are really
//! small (like the little buttons by hierarchy) so I think we need some
//! kind of UI scaling based on screen size and density. We should use
//! rem/logical pixels to set size and provide preview of button sizes".
//!
//! # Logical pixels already, and one factor over them
//!
//! Every size in this application (`theme.rs`'s text sizes, spacing,
//! paddings and icon sizes, and every literal at a call site) is in iced's
//! logical pixels, which the operating system's own density factor turns
//! into physical ones. That is the "rem" asked for: a size relative to a
//! base the system sets. What was missing is a factor of the
//! application's own over it, for the screen whose system says 1.0 and is
//! a 27 inch 4K panel. iced multiplies its scale factor by the system's,
//! so this is that factor ([`Hani::interface_scale`], given to
//! `iced::application` in `main.rs`).
//!
//! # Auto
//!
//! The interface was laid out for a window of about 1280 by 900 logical
//! pixels on a 1080-line screen. *Auto* asks the monitor the window is on
//! how many lines it has at the system's own density, and draws at that
//! over 1080, in eighths, never smaller than 1 and never larger than 2: a
//! 4K screen at a system factor of 1 is drawn at 2, a 1440-line one at
//! 1.25, and one whose system already doubles is left as it is.
//!
//! # Choosing, and seeing before choosing
//!
//! A size chosen by hand is applied only with *Apply*. Until then the
//! *Preview* beside the slider draws a few of the application's controls
//! at the size chosen, so dragging the slider does not redraw the whole
//! window under the pointer that is dragging it. The choice is a setting,
//! `interface.conf`.

use iced::widget::{button, checkbox, column, container, row, slider, text};
use iced::{Alignment, Element, Length, Task};

use super::Hani;
use crate::icon;
use crate::message::{Message, ScaleMessage};
use crate::theme;

/// The setting's file.
const FILE: &str = "interface.conf";

/// The smallest and largest factor that can be chosen.
pub const RANGE: std::ops::RangeInclusive<f32> = 0.75..=2.0;

/// The lines of the screen the interface was laid out on.
const DESIGNED_LINES: f32 = 1080.0;

/// The interface's size: chosen, or by the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UiScale {
    pub auto: bool,
    /// The factor chosen by hand, used when `auto` is off.
    pub chosen: f32,
    /// What the slider shows before Apply.
    pub pending: f32,
    /// The lines of the monitor the window is on, at the system's density,
    /// once it has been asked.
    pub lines: Option<f32>,
    /// Whether the monitor has been asked.
    pub asked: bool,
}

impl Default for UiScale {
    fn default() -> Self {
        Self { auto: true, chosen: 1.0, pending: 1.0, lines: None, asked: false }
    }
}

/// The factor for a monitor of `lines` lines: in eighths, 1 to 2.
pub fn auto_for(lines: f32) -> f32 {
    if !lines.is_finite() || lines <= 0.0 {
        return 1.0;
    }
    ((lines / DESIGNED_LINES * 8.0).round() / 8.0).clamp(1.0, 2.0)
}

impl UiScale {
    /// The factor the interface is drawn at.
    pub fn factor(&self) -> f32 {
        if self.auto { self.lines.map_or(1.0, auto_for) } else { self.chosen }
    }

    /// The setting as kept, or the default: Auto.
    pub fn kept() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        let value = crate::paths::config_file(FILE)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| {
                crate::paths::entries(&text)
                    .find(|(key, _)| *key == "scale")
                    .map(|(_, value)| value.to_string())
            });
        Self::from_setting(value.as_deref())
    }

    fn from_setting(value: Option<&str>) -> Self {
        match value.and_then(|value| value.parse::<f32>().ok()).filter(|value| value.is_finite()) {
            Some(factor) => {
                let factor = factor.clamp(*RANGE.start(), *RANGE.end());
                Self { auto: false, chosen: factor, pending: factor, ..Self::default() }
            }
            None => Self::default(),
        }
    }

    fn setting(&self) -> String {
        if self.auto { "auto".to_string() } else { format!("{}", self.chosen) }
    }

    fn keep(&self) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        let path = crate::paths::config_file(FILE).ok_or("no settings directory")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
        }
        std::fs::write(path, format!("scale = {}\n", self.setting())).map_err(|why| why.to_string())
    }
}

impl Hani {
    /// The factor iced draws the interface at, over the system's own.
    pub fn interface_scale(&self) -> f32 {
        self.ui_scale.factor()
    }

    /// Ask the monitor its size, once, when there is a window to ask about.
    pub(super) fn ask_the_monitor(&mut self) -> Task<Message> {
        if self.ui_scale.asked {
            return Task::none();
        }
        self.ui_scale.asked = true;
        iced::window::latest()
            .and_then(iced::window::monitor_size)
            .map(|size| Message::Scale(ScaleMessage::MonitorMeasured(size.map(|size| size.height))))
    }

    pub(super) fn scale_changed(&mut self, message: ScaleMessage) {
        let scale = &mut self.ui_scale;
        match message {
            // iced gives the monitor in logical pixels at the factor the
            // window is drawn at, which includes this one: undone here, so
            // Auto does not chase its own tail.
            ScaleMessage::MonitorMeasured(height) => {
                let drawn_at = scale.factor();
                scale.lines = height.map(|height| height * drawn_at);
                if scale.auto {
                    scale.pending = scale.factor();
                }
            }
            ScaleMessage::AutoToggled(on) => {
                scale.auto = on;
                if !on {
                    scale.chosen = scale.factor();
                    scale.pending = scale.chosen;
                }
                self.keep_scale();
            }
            ScaleMessage::Previewed(factor) => {
                scale.pending = (factor * 20.0).round() / 20.0;
            }
            ScaleMessage::Applied => {
                scale.auto = false;
                scale.chosen = scale.pending.clamp(*RANGE.start(), *RANGE.end());
                self.keep_scale();
            }
            ScaleMessage::Reset => {
                *scale = UiScale { lines: scale.lines, asked: scale.asked, ..UiScale::default() };
                scale.pending = scale.factor();
                self.keep_scale();
            }
        }
    }

    fn keep_scale(&mut self) {
        let factor = self.ui_scale.factor();
        self.status = match self.ui_scale.keep() {
            Ok(()) if self.ui_scale.auto => {
                format!("interface size: auto, {:.0}%", factor * 100.0)
            }
            Ok(()) => format!("interface size: {:.0}%", factor * 100.0),
            Err(why) => format!("interface size until the application closes: {why}"),
        };
    }

    /// The Preferences block: Auto, the slider, Apply, and the preview.
    pub(super) fn scale_block(&self) -> Element<'_, Message> {
        let scale = &self.ui_scale;
        let send = |message: ScaleMessage| Message::Scale(message);
        let auto_label = match scale.lines {
            Some(lines) => format!("Auto, by the screen ({:.0}%)", auto_for(lines) * 100.0),
            None => "Auto, by the screen".to_string(),
        };
        let pending = scale.pending;
        let applied = scale.factor();
        let differs = (pending - applied).abs() > 0.001;
        column![
            checkbox(scale.auto)
                .label(auto_label)
                .on_toggle(move |on| send(ScaleMessage::AutoToggled(on)))
                .text_size(theme::LABEL_SIZE),
            row![
                text("Size").size(theme::LABEL_SIZE).color(theme::TEXT_DIM),
                slider(RANGE, pending, move |factor| send(ScaleMessage::Previewed(factor)))
                    .step(0.05_f32)
                    .width(Length::Fill),
                text(format!("{:.0}%", pending * 100.0))
                    .size(theme::LABEL_SIZE)
                    .width(Length::Fixed(40.0)),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
            preview(pending / applied.max(0.01)),
            row![
                crate::ui::button_secondary(
                    "Apply",
                    differs.then_some(send(ScaleMessage::Applied))
                ),
                crate::ui::button_secondary("Reset to auto", Some(send(ScaleMessage::Reset))),
            ]
            .spacing(theme::S2),
        ]
        .spacing(theme::S2)
        .into()
    }
}

/// A few of the application's controls drawn `relative` times the size
/// the interface is drawn at now: what they will look like at the size on
/// the slider.
fn preview<'a>(relative: f32) -> Element<'a, Message> {
    let at = |size: f32| size * relative;
    let pad = |padding: iced::Padding| iced::Padding {
        top: padding.top * relative,
        right: padding.right * relative,
        bottom: padding.bottom * relative,
        left: padding.left * relative,
    };
    let icon_button = |name: icon::IconName, lit: bool| {
        button(icon::icon(name, at(theme::ICON_BUTTON), theme::TEXT_DIM))
            .padding(pad(theme::PAD_BUTTON))
            .width(Length::Fixed(at(theme::HIT)))
            .height(Length::Fixed(at(theme::HIT)))
            .style(if lit { theme::tool_button_active } else { theme::tool_button })
    };
    let sample = column![
        text("Preview").size(at(theme::HEADING_SIZE)).color(theme::TEXT_MUTE),
        row![
            icon_button(icon::IconName::Explode, false),
            icon_button(icon::IconName::Solo, true),
            icon_button(icon::IconName::SeeThrough, false),
            button(text("Save as new…").size(at(theme::LABEL_SIZE)))
                .padding(pad(theme::PAD_BUTTON))
                .style(theme::tool_button),
        ]
        .spacing(at(theme::S2))
        .align_y(Alignment::Center),
        row![
            text("Radius").size(at(theme::LABEL_SIZE)).color(theme::TEXT_DIM),
            container(text("3.00 mm").size(at(theme::LABEL_SIZE)))
                .padding(pad(theme::PAD_FIELD))
                .style(theme::panel),
        ]
        .spacing(at(theme::S2))
        .align_y(Alignment::Center),
    ]
    .spacing(at(theme::S2));
    container(sample).padding(theme::S3).width(Length::Fill).style(theme::panel).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    #[test]
    fn auto_draws_a_big_screen_bigger_in_eighths_and_never_smaller() {
        for (lines, factor) in [
            (720.0, 1.0),
            (1080.0, 1.0),
            (1440.0, 1.375),
            (1600.0, 1.5),
            (2160.0, 2.0),
            (2880.0, 2.0),
            (f32::NAN, 1.0),
            (0.0, 1.0),
        ] {
            assert_eq!(auto_for(lines), factor, "{lines} lines");
        }
    }

    #[test]
    fn auto_is_not_fooled_by_its_own_factor() {
        let mut app = app();
        update(&mut app, Message::Scale(ScaleMessage::MonitorMeasured(Some(2160.0))));
        assert_eq!(app.interface_scale(), 2.0);
        // Asked again with the window now drawn at 2, the same monitor
        // reads 1080 logical lines: still a 2160-line screen.
        update(&mut app, Message::Scale(ScaleMessage::MonitorMeasured(Some(1080.0))));
        assert_eq!(app.interface_scale(), 2.0);
    }

    #[test]
    fn a_size_is_only_applied_with_apply_and_reset_goes_back_to_auto() {
        let mut app = app();
        update(&mut app, Message::Scale(ScaleMessage::MonitorMeasured(Some(1080.0))));
        update(&mut app, Message::Scale(ScaleMessage::AutoToggled(false)));
        update(&mut app, Message::Scale(ScaleMessage::Previewed(1.52)));
        assert_eq!(app.interface_scale(), 1.0, "the slider resized the window before Apply");
        assert_eq!(app.ui_scale.pending, 1.5, "the slider is not held to twentieths");
        drop(app.scale_block());
        update(&mut app, Message::Scale(ScaleMessage::Applied));
        assert_eq!(app.interface_scale(), 1.5);
        assert!(app.status.contains("150%"), "{}", app.status);
        update(&mut app, Message::Scale(ScaleMessage::Reset));
        assert!(app.ui_scale.auto);
        assert_eq!(app.interface_scale(), 1.0);
    }

    #[test]
    fn the_setting_reads_back_and_nonsense_is_auto() {
        let chosen = UiScale { auto: false, chosen: 1.25, ..UiScale::default() };
        assert_eq!(UiScale::from_setting(Some(&chosen.setting())).factor(), 1.25);
        assert!(UiScale::from_setting(Some("auto")).auto);
        assert!(UiScale::from_setting(Some("big")).auto);
        assert!(UiScale::from_setting(None).auto);
        assert_eq!(UiScale::from_setting(Some("9")).chosen, *RANGE.end());
        assert!(UiScale::from_setting(Some("NaN")).auto);
    }
}
