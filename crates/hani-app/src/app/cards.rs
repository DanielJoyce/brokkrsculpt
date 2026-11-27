// SPDX-License-Identifier: AGPL-3.0-only

//! The window's frame, beyond the panels: the status bar at its foot, and
//! the two cards of the menus, Preferences and Keyboard shortcuts.
//!
//! `docs/WORKSPACES.md` is the design (stage U2). What a panel says is a
//! label, a placeholder and a status line after an action; the status line
//! is here, once, where it was in the title bar, in the export block and
//! beside the Measure button at the same time. What a key does was grey
//! text in the panel; it is on the shortcuts card, and on the tooltips of
//! the controls it sets. The pen and the SpaceMouse are set once and not
//! every stroke, so they left the panel for Preferences.

use iced::widget::{button, checkbox, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length};

use super::Hani;
use crate::message::Message;
use crate::theme;

/// Every key and held modifier, as the shortcuts card lists them: the key,
/// and what it does. In the order a user meets them.
pub(super) const SHORTCUTS: [(&str, &str); 26] = [
    ("Left drag", "Sculpt, paint or mask with the tool in hand"),
    ("Ctrl or Alt, held", "Carve instead of add; erase paint; unmask"),
    ("Shift, held", "Smooth while held"),
    ("Middle or right drag", "Orbit"),
    ("Shift, middle or right drag", "Pan"),
    ("Wheel", "Zoom about the surface under the pointer"),
    ("Right click", "The tool's settings at the pointer"),
    ("1 to 8", "Pick a brush"),
    ("P", "The mode's palette, at the pointer"),
    ("S, held and dragged", "Brush size"),
    ("U, held and dragged", "Brush strength"),
    ("[ and ]", "Brush size, a step at a time"),
    ("X, Y, Z", "Mirror across a plane"),
    ("M", "Mask, in Sculpt and Paint"),
    ("H, held", "Show the mask"),
    ("Ctrl, cut drag or loop", "Mask by the cut"),
    ("C", "Cut in Sculpt; in Print, part the body with keys"),
    ("W", "Move, turn and scale a body, in Transform"),
    ("F", "Frame the body"),
    ("Ctrl Z, Ctrl Shift Z", "Undo, redo"),
    ("Ctrl G, Ctrl Shift G", "Group bodies, ungroup them, in Transform"),
    ("Ctrl 1 to 4", "Sculpt, Transform, Paint, Print"),
    ("Tab", "Back to the mode before"),
    ("Ctrl comma", "Hide the body; with Alt, show every body"),
    ("?", "These keys"),
    ("Ctrl ?", "Snip a part of the window to report"),
];

impl Hani {
    /// The foot of the window: what the last action did, and three readings.
    pub(super) fn status_bar(&self) -> Element<'_, Message> {
        // A prompt while a gesture waits for its clicks, and otherwise what
        // the last action did.
        let prompt =
            self.measure_prompt().or_else(|| self.insert_prompt()).or_else(|| self.stamp_prompt());
        let failed = prompt.is_none()
            && (self.status.contains("not exported") || self.status.contains("could not"));
        let reading = |words: String| text(words).size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE);
        let bodies = self.doc.body_count();
        container(
            row![
                text(prompt.map_or_else(|| self.status.clone(), str::to_string))
                    .size(theme::CAPTION_SIZE)
                    .color(if failed { theme::ERROR } else { theme::TEXT_DIM })
                    .width(Length::Fill),
                reading(format!("voxel {:.2} mm", self.doc.voxel_size())),
                reading(format!("{} triangles", self.shared.stats().triangles)),
                reading(format!("{bodies} {}", if bodies == 1 { "body" } else { "bodies" })),
            ]
            .spacing(theme::S4)
            .align_y(Alignment::Center),
        )
        .padding(iced::Padding { top: 3.0, bottom: 3.0, left: theme::S4, right: theme::S4 })
        .width(Length::Fill)
        .style(theme::panel)
        .into()
    }

    /// Edit, Preferences.
    pub(super) fn preferences_card(&self) -> Element<'_, Message> {
        let heading =
            |words: &'static str| text(words).size(theme::HEADING_SIZE).color(theme::TEXT_MUTE);
        let application = column![
            heading("THIS APPLICATION"),
            checkbox(self.update_settings.when != crate::update::When::Never)
                .label("Check for updates")
                .on_toggle(Message::UpdateCheckSet)
                .text_size(theme::LABEL_SIZE),
            checkbox(self.welcome_on_startup)
                .label("Welcome screen on startup")
                .on_toggle(Message::WelcomeOnStartupSet)
                .text_size(theme::LABEL_SIZE),
            // See `app/cores.rs`.
            self.cores_row(),
            // See `app/ui_scale.rs`.
            heading("INTERFACE SIZE"),
            self.scale_block(),
            // See `app/pooling.rs`.
            heading("PAINT"),
            self.pooling_numbers(),
            // See `app/kept_settings.rs`.
            self.kept_settings_block(),
        ]
        .spacing(theme::S2);
        let card = container(
            column![
                row![
                    text("Preferences").size(theme::TITLE_SIZE).color(theme::TEXT),
                    space().width(Length::Fill),
                    crate::ui::tip(
                        crate::ui::button_secondary("Close", Some(Message::PreferencesClosed)),
                        "Back to the sculpt; every setting is kept as it is changed"
                    ),
                ]
                .align_y(Alignment::Center),
                scrollable(
                    row![
                        column![heading("PEN"), self.pen_panel(), application]
                            .spacing(theme::S3)
                            .width(Length::Fill),
                        column![heading("SPACEMOUSE"), self.spacemouse_panel()]
                            .spacing(theme::S3)
                            .width(Length::Fill),
                    ]
                    .spacing(theme::S5)
                    .padding(iced::Padding { right: theme::S4, ..iced::Padding::ZERO }),
                )
                .height(Length::Shrink),
            ]
            .spacing(theme::S4),
        )
        .padding(theme::S5)
        .max_width(620.0)
        .max_height(640.0)
        .style(theme::menu_card);
        super::panel::modal_layer(card)
    }

    /// Help, Keyboard shortcuts.
    pub(super) fn shortcuts_card(&self) -> Element<'_, Message> {
        let line = |(key, what): (&'static str, &'static str)| {
            row![
                container(text(key).size(theme::LABEL_SIZE).color(theme::ACCENT))
                    .width(Length::Fixed(170.0)),
                text(what).size(theme::LABEL_SIZE).color(theme::TEXT_DIM).width(Length::Fill),
            ]
            .spacing(theme::S3)
        };
        let half = SHORTCUTS.len().div_ceil(2);
        let card = container(
            column![
                row![
                    text("Keyboard shortcuts").size(theme::TITLE_SIZE).color(theme::TEXT),
                    space().width(Length::Fill),
                    crate::ui::tip(
                        button(text("Close").size(theme::LABEL_SIZE))
                            .padding(theme::PAD_BUTTON)
                            .style(theme::tool_button)
                            .on_press(Message::ShortcutsClosed),
                        "Back to the sculpt"
                    ),
                ]
                .align_y(Alignment::Center),
                row![
                    column(SHORTCUTS[..half].iter().copied().map(line).map(Element::from))
                        .spacing(theme::S2)
                        .width(Length::Fill),
                    column(SHORTCUTS[half..].iter().copied().map(line).map(Element::from))
                        .spacing(theme::S2)
                        .width(Length::Fill),
                ]
                .spacing(theme::S5),
            ]
            .spacing(theme::S4),
        )
        .padding(theme::S5)
        .max_width(900.0)
        .style(theme::menu_card);
        super::panel::modal_layer(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PanelSection, TopMenu};

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn escape(app: &mut Hani) {
        update(
            app,
            Message::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                modifiers: iced::keyboard::Modifiers::default(),
            },
        );
    }

    /// Each card of the menus is a modal: it takes its menu away, holds the
    /// keys while it is up, and goes on Escape or on its own Close.
    #[test]
    fn the_cards_of_the_menus_open_hold_the_keys_and_close() {
        let mut app = app();
        app.welcome = false;
        for (open, close) in [
            (Message::PreferencesOpened, Message::PreferencesClosed),
            (Message::ShortcutsOpened, Message::ShortcutsClosed),
        ] {
            update(&mut app, Message::TopMenuToggled(TopMenu::Edit));
            update(&mut app, open.clone());
            assert!(app.top_menu.is_none(), "the menu stayed open under {open:?}");
            assert!(app.modal_open(), "{open:?} is not modal");
            // A key behind the card does nothing: here, X would mirror.
            let mirrored = app.symmetry_settings();
            update(
                &mut app,
                Message::KeyPressed {
                    key: iced::keyboard::Key::Character("x".into()),
                    modifiers: iced::keyboard::Modifiers::default(),
                },
            );
            assert_eq!(app.symmetry_settings(), mirrored, "a key went past {open:?}");
            escape(&mut app);
            assert!(!app.modal_open(), "Escape left {open:?} up");
            update(&mut app, open.clone());
            update(&mut app, close);
            assert!(!app.modal_open(), "Close left {open:?} up");
        }
    }

    /// "?" brings the shortcuts card up, and Ctrl and comma still hides a
    /// body, which is why Preferences has no key of its own.
    #[test]
    fn question_mark_is_the_shortcuts_card() {
        assert!(matches!(
            crate::viewport::shortcut("?", false, true, false),
            Some(Message::ShortcutsOpened)
        ));
        assert!(matches!(
            crate::viewport::shortcut(",", true, false, false),
            Some(Message::ActiveBodyVisibilityToggled)
        ));
    }

    /// Every key the shortcuts card names is a key the application has: the
    /// letters on the card each decode to something.
    #[test]
    fn every_letter_on_the_card_is_a_key() {
        for (key, _) in SHORTCUTS {
            // A line that names Ctrl is a chord, and its letters are decoded
            // with Ctrl held: "Ctrl G" is no bare G.
            let command = key.contains("Ctrl");
            for word in key.split([',', ' ']) {
                if word.len() == 1 && word.chars().all(|c| c.is_ascii_alphabetic()) {
                    assert!(
                        crate::viewport::shortcut(&word.to_lowercase(), command, false, false)
                            .is_some(),
                        "the card names {word}, which is no key"
                    );
                }
            }
        }
    }

    /// The pen and the SpaceMouse are on the Preferences card and in no
    /// block of the panel.
    #[test]
    fn the_devices_are_not_in_the_panel() {
        for section in PanelSection::ALL {
            assert!(
                !matches!(section.title(), "PEN" | "SPACEMOUSE"),
                "{section:?} is still in the panel"
            );
        }
    }
}
