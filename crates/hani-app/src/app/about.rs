// SPDX-License-Identifier: AGPL-3.0-only

//! About: what this build is, where it runs and what it was built with.
//!
//! Asked for by the maintainer on 2026-11-27: "add an about screen, move
//! build and platform and feature flags compiled with to that, all
//! non-help info goes there, and in help and about, use the logo. You can
//! use the simple icon logo in help, use the full logo with HaniSculpt in
//! the about screen". The version, the build and the licence were lines
//! at the foot of the Help menu, under its actions; they are here now,
//! with the platform, the profile and the features it was compiled with,
//! under the lockup. Opened from the Help menu, closed by its button or
//! Escape.

use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Length};

use super::Hani;
use crate::message::Message;
use crate::theme;

/// What the Graphics line says while the adapter is being asked for.
const ASKING: &str = "asking…";

/// Where this build runs: the operating system and the processor, or a
/// browser.
pub(super) fn platform() -> String {
    if cfg!(target_arch = "wasm32") {
        "a web browser (WebAssembly)".to_string()
    } else {
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
    }
}

/// How it was compiled: the profile, and each of the application's own
/// features with whether it is in.
pub(super) fn compiled_with() -> Vec<(&'static str, bool)> {
    vec![("debug assertions", cfg!(debug_assertions)), ("uinput", cfg!(feature = "uinput"))]
}

impl Hani {
    /// Escape: close About, and say whether it was open.
    pub(super) fn about_escaped(&mut self) -> bool {
        std::mem::replace(&mut self.about_open, false)
    }

    /// About opened or closed. The first time it opens, the graphics
    /// adapter is asked for on the task pool (`app/adapter.rs`).
    pub(super) fn about_changed(&mut self, open: bool) -> iced::Task<Message> {
        self.about_open = open;
        if open && self.graphics.is_none() {
            // Asked once: while the answer is on its way the line says so,
            // and a second opening does not ask again.
            self.graphics = Some(ASKING.to_string());
            return iced::Task::perform(super::adapter::asked(), Message::GraphicsNamed);
        }
        iced::Task::none()
    }

    /// The card, over everything below it.
    pub(super) fn about_card(&self) -> Element<'_, Message> {
        let line = |label: &'static str, value: String| {
            row![
                text(label)
                    .size(theme::LABEL_SIZE)
                    .color(theme::TEXT_DIM)
                    .width(Length::Fixed(140.0)),
                text(value).size(theme::LABEL_SIZE).color(theme::TEXT),
            ]
            .spacing(theme::S3)
        };
        let features = compiled_with()
            .into_iter()
            .map(|(name, on)| format!("{name} {}", if on { "on" } else { "off" }))
            .collect::<Vec<_>>()
            .join(", ");
        let mut facts = column![
            line("Version", env!("CARGO_PKG_VERSION").to_string()),
            line("Build", super::build_label()),
            line("Platform", platform()),
            crate::ui::tip(
                line("Graphics", self.graphics.clone().unwrap_or_else(|| ASKING.to_string())),
                "The adapter a request like the window's is given, and its backend",
            ),
            line("Compiled with", features),
            line("Licence", "AGPL-3.0-only".to_string()),
        ]
        .spacing(theme::S2);
        if !self.update_freshness.is_empty() {
            facts = facts.push(line("Updates", self.update_freshness.clone()));
        }
        let card = container(
            column![
                row![
                    space().width(Length::Fill),
                    crate::ui::button_secondary("Close", Some(Message::About(false))),
                ],
                container(crate::logo::lockup(120.0)).center_x(Length::Fill),
                facts,
                crate::ui::tip(
                    button(
                        text(crate::links::REPOSITORY.trim_start_matches("https://"))
                            .size(theme::LABEL_SIZE)
                            .color(theme::ACCENT_HOT),
                    )
                    .padding(0)
                    .style(theme::section_heading)
                    .on_press(Message::LinkOpened(crate::links::REPOSITORY.to_string())),
                    "The source, the releases and the issues",
                ),
            ]
            .spacing(theme::S4)
            .align_x(Alignment::Start),
        )
        .padding(theme::S5)
        .max_width(560.0)
        .style(theme::menu_card);
        super::panel::modal_layer(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// Opened from the Help menu's message and closed by its own; Escape
    /// closes it too; the card is drawn while it is open.
    #[test]
    fn about_opens_closes_and_escapes() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.welcome = false;
        update(&mut app, Message::About(true));
        assert!(app.about_open);
        drop(app.about_card());
        update(&mut app, Message::About(false));
        assert!(!app.about_open);
        update(&mut app, Message::About(true));
        assert!(app.about_escaped(), "Escape did not find it open");
        assert!(!app.about_open);
        assert!(!app.about_escaped(), "Escape found it open twice");
    }

    /// The first opening asks for the adapter and says so on the card until
    /// it is named; a second opening does not ask again.
    #[test]
    fn about_asks_for_the_adapter_once() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        assert_eq!(app.graphics, None);
        update(&mut app, Message::About(true));
        assert_eq!(app.graphics.as_deref(), Some(ASKING));
        update(&mut app, Message::GraphicsNamed("Radeon RX 7600 (Vulkan)".to_string()));
        drop(app.about_card());
        update(&mut app, Message::About(false));
        update(&mut app, Message::About(true));
        assert_eq!(app.graphics.as_deref(), Some("Radeon RX 7600 (Vulkan)"));
    }

    /// The platform names this machine's system, and the features name the
    /// application's own.
    #[test]
    fn the_platform_and_the_features_are_this_builds() {
        assert!(platform().contains(std::env::consts::OS));
        let features: Vec<&str> = compiled_with().into_iter().map(|(name, _)| name).collect();
        assert!(features.contains(&"uinput"));
        assert_eq!(
            compiled_with().into_iter().find(|(name, _)| *name == "debug assertions"),
            Some(("debug assertions", cfg!(debug_assertions)))
        );
    }
}
