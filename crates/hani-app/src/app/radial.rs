// SPDX-License-Identifier: AGPL-3.0-only

//! Radial symmetry: a number of copies of every stroke turned evenly round
//! an axis, with the mirror planes in SYMMETRY in the panel.
//!
//! Asked for by the maintainer on 2026-11-28: "add radial symmetry with a
//! configurable number". The copies are made in the core
//! (`hani_core::Symmetry::flips`): every turn of every combination of the
//! mirror planes that are on, so X and six round Y gives twelve. The axis
//! goes through the symmetry's frame, which is the body's own (see
//! `app/symmetry_frame.rs`). Like the mirror planes it is a setting of the
//! session, kept in a file with the bodies' frames.

use hani_core::{MirrorAxis, Symmetry};
use iced::Element;
use iced::widget::{button, column, row, text};

use super::Hani;
use crate::message::Message;
use crate::theme;

/// What the radial controls ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RadialMessage {
    /// How many copies, the stroke's own among them; one is off.
    Copies(f32),
    /// The axis they turn about.
    Axis(MirrorAxis),
    /// The active body's planes back through the middle of the room. See
    /// `app/symmetry_frame.rs`.
    ResetFrame,
}

impl Hani {
    pub(super) fn radial_changed(&mut self, message: RadialMessage) {
        let (copies, axis) = (self.symmetry.radial(), self.symmetry.radial_axis());
        self.symmetry = match message {
            RadialMessage::Copies(count) => {
                let count = count.round().clamp(1.0, f32::from(Symmetry::MAX_RADIAL)) as u8;
                self.symmetry.with_radial(count, axis)
            }
            RadialMessage::Axis(next) => self.symmetry.with_radial(copies, next),
            RadialMessage::ResetFrame => return self.reset_frame(),
        };
        self.status = format!("symmetry: {}", self.symmetry.label());
    }

    /// SYMMETRY in the panel: the planes, as the context bar has them, the
    /// number of radial copies and the axis they turn about. The context
    /// bar had no room for the radial copies at a 1280 pixel window (seen
    /// on a screen: they were cut off after Z).
    pub(super) fn symmetry_panel(&self) -> Element<'_, Message> {
        let copies = self.symmetry.radial();
        let axis = self.symmetry.radial_axis();
        let planes = MirrorAxis::ALL.into_iter().fold(
            row![text("Mirror").size(theme::LABEL_SIZE).color(theme::TEXT_DIM)]
                .spacing(theme::S1)
                .align_y(iced::Alignment::Center),
            |assembled, plane| {
                let (style, _) = theme::tool_toggle(self.symmetry.axis(plane));
                assembled.push(crate::ui::tip(
                    button(text(plane.label()).size(theme::LABEL_SIZE))
                        .padding(theme::PAD_BUTTON)
                        .style(style)
                        .on_press(Message::SymmetryAxisToggled(plane)),
                    "Mirror every stroke across this plane",
                ))
            },
        );
        column![
            planes,
            crate::ui::number(
                &self.drafts,
                "symmetry.radial",
                "Radial copies",
                f32::from(copies),
                1.0..=f32::from(Symmetry::MAX_RADIAL),
                |count| Message::Radial(RadialMessage::Copies(count)),
            )
            .step(1.0)
            .shown(if copies == 1 { "off".to_string() } else { copies.to_string() })
            .tip("Copies of every stroke turned evenly round the axis; 1 is off"),
            row![
                text("Round").size(theme::LABEL_SIZE).color(theme::TEXT_DIM),
                crate::ui::segmented_tips(
                    MirrorAxis::ALL.map(|each| (
                        each,
                        each.label(),
                        "The axis the radial copies turn about"
                    )),
                    axis,
                    |each| Message::Radial(RadialMessage::Axis(each)),
                ),
            ]
            .spacing(theme::S2)
            .align_y(iced::Alignment::Center),
            // The body's own frame, moved with it by the gizmo. See
            // `app/symmetry_frame.rs`.
            crate::ui::tip(
                crate::ui::button_secondary(
                    "Recentre",
                    (!self.frame_is_the_rooms())
                        .then_some(Message::Radial(RadialMessage::ResetFrame)),
                ),
                "Put this body's planes and axis back through the middle of the room, unturned",
            ),
        ]
        .spacing(theme::S2)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// The count sets the copies, held to the most there can be, and the
    /// axis turns through X, Y and Z; a stroke with six copies round Y lays
    /// five twins.
    #[test]
    fn the_radial_copies_and_their_axis_are_set() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        assert_eq!(app.symmetry.radial(), 1);
        update(&mut app, Message::Radial(RadialMessage::Copies(6.0)));
        assert_eq!(app.symmetry.radial(), 6);
        assert_eq!(app.symmetry.radial_axis(), MirrorAxis::Y);
        update(&mut app, Message::Radial(RadialMessage::Copies(99.0)));
        assert_eq!(app.symmetry.radial(), Symmetry::MAX_RADIAL);
        update(&mut app, Message::Radial(RadialMessage::Copies(6.0)));
        update(&mut app, Message::Radial(RadialMessage::Axis(MirrorAxis::Z)));
        assert_eq!(app.symmetry.radial_axis(), MirrorAxis::Z);
        assert_eq!(app.symmetry.radial(), 6, "the axis took the copies with it");
        update(&mut app, Message::Radial(RadialMessage::Axis(MirrorAxis::Y)));
        let mut flips = [hani_core::Flip::IDENTITY; Symmetry::MAX_MIRRORS];
        assert_eq!(app.symmetry.flips(glam::Vec3::ZERO, &mut flips), 5);
        update(&mut app, Message::Radial(RadialMessage::Copies(1.0)));
        assert!(app.symmetry.is_off());
    }
}
