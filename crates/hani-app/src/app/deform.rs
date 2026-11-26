// SPDX-License-Identifier: AGPL-3.0-only

//! The DEFORM block in Transform: twist, bend or taper the active body along
//! an axis. `hani_core::deform` has the geometry; here are the choices and
//! the button, a child module of `app` for the reason `form.rs` is.
//!
//! Each kind keeps its own amount, so going from a twist to a taper and back
//! finds the twist as it was left. An apply is one undo entry holding the
//! body as it was, the same as a bulk.

use web_time::Instant;

use hani_core::deform::{Axis, DeformKind, Deformation, MAX_BEND_DEGREES, MIN_TAPER};
use iced::Element;
use iced::widget::{column, row};

use super::Hani;
use crate::message::{DeformMessage, Message};
use crate::theme;

pub(super) const TWIST_RANGE: std::ops::RangeInclusive<f32> = -720.0..=720.0;
pub(super) const BEND_RANGE: std::ops::RangeInclusive<f32> = -MAX_BEND_DEGREES..=MAX_BEND_DEGREES;
pub(super) const TAPER_RANGE: std::ops::RangeInclusive<f32> = MIN_TAPER..=4.0;

/// What the application remembers for the deformers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DeformState {
    pub kind: DeformKind,
    pub axis: Axis,
    /// One amount a kind, in `DeformKind::ALL`'s order.
    pub amounts: [f32; 3],
}

impl Default for DeformState {
    fn default() -> Self {
        Self { kind: DeformKind::Twist, axis: Axis::Y, amounts: [90.0, 45.0, 0.5] }
    }
}

impl DeformState {
    fn slot(kind: DeformKind) -> usize {
        DeformKind::ALL.iter().position(|each| *each == kind).unwrap_or(0)
    }

    pub fn amount(&self) -> f32 {
        self.amounts[Self::slot(self.kind)]
    }

    fn range(kind: DeformKind) -> std::ops::RangeInclusive<f32> {
        match kind {
            DeformKind::Twist => TWIST_RANGE,
            DeformKind::Bend => BEND_RANGE,
            DeformKind::Taper => TAPER_RANGE,
        }
    }

    pub fn deformation(&self) -> Deformation {
        Deformation { kind: self.kind, axis: self.axis, amount: self.amount() }
    }
}

impl Hani {
    pub(super) fn deform_changed(&mut self, message: DeformMessage) {
        match message {
            DeformMessage::Kind(kind) => self.deform.kind = kind,
            DeformMessage::Axis(axis) => self.deform.axis = axis,
            DeformMessage::Amount(amount) => {
                let range = DeformState::range(self.deform.kind);
                let amount = if amount.is_nan() {
                    self.deform.kind.identity()
                } else {
                    amount.clamp(*range.start(), *range.end())
                };
                self.deform.amounts[DeformState::slot(self.deform.kind)] = amount;
            }
            DeformMessage::Applied => self.deform_active(),
        }
    }

    /// Deform the active body as one undo entry, or say why not.
    fn deform_active(&mut self) {
        let started = Instant::now();
        let how = self.deform.deformation();
        let body = self.doc.active();
        let name = self.active_body_name();
        if how.amount == how.kind.identity() {
            self.status = format!("{name} not deformed: the amount does nothing");
            return;
        }
        let Some(mut deformed) = self.doc.active_volume().deformed(how) else {
            self.status = format!("{name} not deformed: it has no surface");
            return;
        };
        // Stretched and squeezed, the gradient is off one; every brush after
        // reads it.
        deformed.redistance();
        if self.swap_in(body, deformed) {
            let what = match how.kind {
                DeformKind::Twist => format!("twisted {name} {:.0}°", how.amount),
                DeformKind::Bend => format!("bent {name} {:.0}°", how.amount),
                DeformKind::Taper => format!("tapered {name} to {:.2}", how.amount),
            };
            self.status = format!(
                "{what} along {} ({:.0} ms)",
                how.axis,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// The DEFORM block.
    pub(super) fn deform_panel(&self) -> Element<'_, Message> {
        let state = self.deform;
        let kind = state.kind;
        let amount = state.amount();
        let shown = match kind {
            DeformKind::Twist | DeformKind::Bend => format!("{amount:.0}°"),
            DeformKind::Taper => format!("x {amount:.2}"),
        };
        let step = match kind {
            DeformKind::Twist | DeformKind::Bend => 5.0,
            DeformKind::Taper => 0.05,
        };
        let key = match kind {
            DeformKind::Twist => "deform.twist",
            DeformKind::Bend => "deform.bend",
            DeformKind::Taper => "deform.taper",
        };
        column![
            crate::ui::segmented(DeformKind::ALL.map(|each| (each, each.label())), kind, |each| {
                Message::Deform(DeformMessage::Kind(each))
            },),
            row![
                crate::ui::sub_label("Axis"),
                crate::ui::segmented(
                    Axis::ALL.map(|each| (each, each.label())),
                    state.axis,
                    |each| { Message::Deform(DeformMessage::Axis(each)) }
                ),
            ]
            .spacing(theme::S2)
            .align_y(iced::Alignment::Center),
            crate::ui::number(
                &self.drafts,
                key,
                "Amount",
                amount,
                DeformState::range(kind),
                |value| Message::Deform(DeformMessage::Amount(value)),
            )
            .step(step)
            .shown(shown)
            .trailing(crate::ui::tip(
                crate::ui::button_secondary("Apply", Some(Message::Deform(DeformMessage::Applied))),
                "Deform the active body",
            )),
        ]
        .spacing(theme::S2)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    #[test]
    fn a_taper_narrows_the_top_and_undo_puts_it_back() {
        let mut app = app();
        let (low, high) = app.doc.active_volume().surface_bounds().expect("a surface");
        let near_top = Vec3::new(high.x * 0.55, low.y + (high.y - low.y) * 0.8, 0.0);
        assert!(app.doc.active_volume().sample_world(near_top) < 0.0, "starts inside");
        update(&mut app, Message::Deform(DeformMessage::Kind(DeformKind::Taper)));
        update(&mut app, Message::Deform(DeformMessage::Amount(0.3)));
        update(&mut app, Message::Deform(DeformMessage::Applied));
        assert!(app.status.starts_with("tapered"), "{}", app.status);
        assert!(app.doc.active_volume().sample_world(near_top) > 0.0, "{}", app.status);
        update(&mut app, Message::Undo);
        assert!(app.doc.active_volume().sample_world(near_top) < 0.0, "undo puts it back");
    }

    #[test]
    fn each_kind_keeps_its_own_amount() {
        let mut app = app();
        update(&mut app, Message::Deform(DeformMessage::Amount(30.0)));
        update(&mut app, Message::Deform(DeformMessage::Kind(DeformKind::Taper)));
        assert_eq!(app.deform.amount(), 0.5);
        update(&mut app, Message::Deform(DeformMessage::Amount(99.0)));
        assert_eq!(app.deform.amount(), 4.0, "clamped to the taper's range");
        update(&mut app, Message::Deform(DeformMessage::Kind(DeformKind::Twist)));
        assert_eq!(app.deform.amount(), 30.0);
        drop(app.deform_panel());
    }

    #[test]
    fn nothing_asked_is_refused_and_changes_nothing() {
        let mut app = app();
        update(&mut app, Message::Deform(DeformMessage::Amount(0.0)));
        let before = app.history.stats();
        update(&mut app, Message::Deform(DeformMessage::Applied));
        assert!(app.status.contains("does nothing"), "{}", app.status);
        assert_eq!(app.history.stats(), before);
    }
}
