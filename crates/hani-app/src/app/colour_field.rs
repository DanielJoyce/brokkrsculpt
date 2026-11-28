// SPDX-License-Identifier: AGPL-3.0-only

//! A colour's control: its swatch and a hex field on one line, and its red,
//! green and blue as three numbers under them.
//!
//! The paint designer's (`app/designer.rs`), taken out of it on 2026-11-28
//! when the Lighting block (`app/lighting.rs`) needed one for each light,
//! the sky, the ground and the two materials that take a colour, so that
//! there is one colour control and not two that drift apart.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};

use crate::message::Message;
use crate::theme;

/// A colour as the hex field writes it: six hex digits, no `#`.
pub(super) fn hex_of(rgb: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub(super) fn swatch(rgb: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(rgb[0], rgb[1], rgb[2])
}

/// The swatch and the hex field. `typed` is what the field shows, which is
/// the colour's own hex until something else is typed. With `pressed`, the
/// swatch is a button, said on hover by `tip`.
pub(super) fn hex_row<'a>(
    rgb: [u8; 3],
    typed: String,
    on_hex: impl Fn(String) -> Message + 'a,
    pressed: Option<(Message, &'static str)>,
) -> Element<'a, Message> {
    let chip = container(text("")).width(36).height(22).style(theme::swatch(swatch(rgb)));
    let chip: Element<'a, Message> = match pressed {
        Some((message, what)) => crate::ui::tip(
            button(chip).padding(0).style(theme::tool_button).on_press(message),
            what,
        ),
        None => chip.into(),
    };
    row![
        chip,
        text_input("RRGGBB", &typed)
            .on_input(on_hex)
            .size(theme::LABEL_SIZE)
            .padding(theme::PAD_FIELD)
            .width(Length::Fixed(90.0)),
    ]
    .spacing(theme::S2)
    .align_y(Alignment::Center)
    .into()
}

/// Red, green and blue, each a number from 0 to 255 under its own draft
/// key.
pub(super) fn channels<'a>(
    drafts: &'a crate::ui::Drafts,
    keys: [&'static str; 3],
    rgb: [u8; 3],
    on_channel: impl Fn(usize, u8) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    keys.into_iter()
        .zip(["Red", "Green", "Blue"])
        .enumerate()
        .fold(column![].spacing(theme::S2), |assembled, (channel, (key, name))| {
            assembled.push(
                crate::ui::number(drafts, key, name, f32::from(rgb[channel]), 0.0..=255.0, {
                    move |level| on_channel(channel, level.round().clamp(0.0, 255.0) as u8)
                })
                .step(1.0)
                .shown(rgb[channel].to_string()),
            )
        })
        .into()
}
