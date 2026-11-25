// SPDX-License-Identifier: AGPL-3.0-only

//! The context bar over the viewport, and the controls it shares with the
//! right-click menu.
//!
//! `docs/WORKSPACES.md` is the design (stage U3). What is changed every
//! few strokes for the tool in hand (its size, its strength or opacity,
//! its falloff, the mirror planes) is on one line over the viewport,
//! always in the same place. The right-click menu is the same controls in
//! a card at the pointer, built here by the same functions, so the two
//! cannot come to offer different things: before this, the panel offered
//! five falloff shapes and a flat middle while the menu offered four, and
//! the menu's typed fields were its own.

use brokkr_core::{BrushKind, FalloffCurve, MirrorAxis};
use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Length};

use super::{Brokkr, Tool};
use crate::message::{AddMessage, Message, Rail, StudioMessage};
use crate::theme;

impl Brokkr {
    /// Whether the tool in hand lays colour rather than shaping the field.
    fn painting_now(&self) -> bool {
        self.brush.kind == BrushKind::Paint && self.tool != Tool::Mask
    }

    /// Whether the tool in hand is sized and weighted at all: the cut and
    /// the transform gizmo are neither.
    fn sized_now(&self) -> bool {
        matches!(self.rail_lit(), Rail::Brush | Rail::Move | Rail::Paint | Rail::Mask)
    }

    /// The tool's size, as a number. `inline` for the bar.
    pub(super) fn radius_number(&self, inline: bool) -> Element<'_, Message> {
        let number = crate::ui::number(
            &self.drafts,
            "brush.radius",
            "Radius",
            self.brush.radius,
            super::MIN_RADIUS_MM..=self.max_radius(),
            Message::BrushRadiusChanged,
        )
        .step(0.05)
        .shown(format!("{:.2} mm", self.brush.radius))
        .tip("Or hold S and drag in the view");
        if inline { number.inline().into() } else { number.into() }
    }

    /// The tool's weight: strength, or the paint's opacity. `None` while
    /// painting by filament slot, where a stroke has no half.
    pub(super) fn weight_number(&self, inline: bool) -> Option<Element<'_, Message>> {
        let number = if self.painting_now() {
            if !self.painting_colour() {
                return None;
            }
            crate::ui::number(
                &self.drafts,
                "paint.opacity",
                "Opacity",
                self.painting.opacity,
                super::painting::OPACITY_RANGE,
                |opacity| Message::Painting(crate::message::PaintingMessage::Opacity(opacity)),
            )
            .shown(format!("{:.2}", self.painting.opacity))
        } else {
            crate::ui::number(
                &self.drafts,
                "brush.strength",
                "Strength",
                self.brush.strength,
                super::MIN_STRENGTH..=self.max_strength(),
                Message::BrushStrengthChanged,
            )
            .step(0.01)
            .shown(format!("{:.2}", self.brush.strength))
            .tip("Or hold U and drag in the view")
        };
        Some(if inline { number.inline().into() } else { number.into() })
    }

    /// How the tool's edge falls away: the five shapes of a profile for a
    /// brush on the field, upstream's four curves for the mask and for a
    /// colour, and nothing for a filament slot, which has no edge.
    pub(super) fn falloff_choice(&self) -> Option<Element<'_, Message>> {
        if self.painting_now() && !self.painting_colour() {
            return None;
        }
        if self.painting_now() || self.tool == Tool::Mask {
            return Some(crate::ui::segmented(
                FalloffCurve::ALL.map(|curve| (curve, curve.label())),
                self.brush.falloff,
                Message::FalloffChanged,
            ));
        }
        use brokkr_core::brush::compound::Shape;
        let live = self.studio.profile(self.brush.falloff).shape;
        Some(crate::ui::segmented(Shape::ALL.map(|shape| (shape, shape.label())), live, |shape| {
            match shape.curve() {
                Some(curve) => Message::FalloffChanged(curve),
                None => Message::Studio(StudioMessage::RoundChosen),
            }
        }))
    }

    /// The bar over the viewport.
    pub(super) fn context_bar(&self) -> Element<'_, Message> {
        let lit = self.rail_lit();
        let name = text(match lit {
            Rail::Measure => "MEASURE".to_string(),
            Rail::Place => self.live_brush_name(),
            _ => self.live_tool_label(),
        })
        .size(theme::LABEL_SIZE)
        .color(theme::TEXT);
        let mut line = row![container(name).width(Length::Fixed(NAME_WIDTH))]
            .spacing(theme::S3)
            .align_y(Alignment::Center);
        if self.sized_now() {
            line = line.push(self.radius_number(true));
            if let Some(weight) = self.weight_number(true) {
                line = line.push(weight);
            }
            if let Some(falloff) = self.falloff_choice() {
                line = line.push(container(falloff).width(Length::Fixed(FALLOFF_WIDTH)));
            }
        } else if lit == Rail::Place {
            for number in self.placing_numbers() {
                line = line.push(number);
            }
        }
        line = line.push(self.mirror_toggles());
        container(line.wrap().vertical_spacing(theme::S1))
            .padding(iced::Padding { top: 3.0, bottom: 3.0, left: theme::S3, right: theme::S3 })
            .width(Length::Fill)
            .style(theme::panel)
            .into()
    }

    /// How what is being added meets the surface, on the bar: the soft
    /// join and how far it is held off. The same numbers, under the same
    /// keys, as Kitbash's settings. A map has neither.
    fn placing_numbers(&self) -> Vec<Element<'_, Message>> {
        let state = &self.inserts;
        if matches!(state.what, None | Some(super::Insert::Map(_))) {
            return Vec::new();
        }
        let voxel_size = self.doc.voxel_size();
        vec![
            crate::ui::number(
                &self.drafts,
                "add.blend",
                "Soft join",
                state.blend,
                0.0..=brokkr_core::solid::MOST_BLEND,
                |voxels| Message::Add(AddMessage::BlendChanged(voxels)),
            )
            .step(0.25)
            .shown(if state.separate || state.blend <= 0.0 {
                "none".to_string()
            } else {
                format!("{:.2} mm", state.blend * voxel_size)
            })
            .inline()
            .into(),
            crate::ui::number(
                &self.drafts,
                "add.lift",
                "Held off",
                state.lift,
                super::adding::LIFT_RANGE,
                |lift| Message::Add(AddMessage::LiftChanged(lift)),
            )
            .step(0.05)
            .shown(format!("{:+.2}", state.lift))
            .inline()
            .into(),
        ]
    }

    /// The three mirror planes, as letters that light. They were on the
    /// tool strip.
    fn mirror_toggles(&self) -> Element<'_, Message> {
        MirrorAxis::ALL
            .into_iter()
            .fold(
                row![text("Mirror").size(theme::LABEL_SIZE).color(theme::TEXT_DIM)]
                    .spacing(theme::S1)
                    .align_y(Alignment::Center),
                |assembled, axis| {
                    let (style, _) = theme::tool_toggle(self.symmetry.axis(axis));
                    assembled.push(crate::ui::tip(
                        button(text(axis.label()).size(theme::LABEL_SIZE))
                            .padding(theme::PAD_BUTTON)
                            .style(style)
                            .on_press(Message::SymmetryAxisToggled(axis)),
                        match axis {
                            MirrorAxis::X => "Mirror across X  X",
                            MirrorAxis::Y => "Mirror across Y  Y",
                            MirrorAxis::Z => "Mirror across Z  Z",
                        },
                    ))
                },
            )
            .into()
    }
}

/// How wide the tool's name is on the bar, so that the controls after it do
/// not move as it changes from DRAW to CLAY STRIPS.
const NAME_WIDTH: f32 = 96.0;

/// How wide the falloff choice is on the bar: five shapes at the label
/// size, which is what a profile has.
const FALLOFF_WIDTH: f32 = 236.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        app.welcome = false;
        app
    }

    /// The bar and the right-click menu offer the same falloff, for every
    /// kind of tool: they are built by one function, and this says which
    /// choice that function makes for each.
    #[test]
    fn the_falloff_offered_follows_the_tool() {
        let mut app = app();
        // A brush on the field: a profile's five shapes.
        assert!(app.falloff_choice().is_some());
        assert!(!app.painting_now());
        // The mask: upstream's four curves.
        drop(app.update(Message::ToolChanged(Tool::Mask)));
        assert!(app.falloff_choice().is_some());
        drop(app.update(Message::ToolChanged(Tool::Mask)));
        // The cut and the gizmo have no size, no weight and no falloff on
        // the bar; the bar still has the mirror planes.
        drop(app.update(Message::ToolChanged(Tool::Cut)));
        assert!(!app.sized_now());
        drop(app.update(Message::ToolChanged(Tool::Cut)));
        assert!(app.sized_now());
        // Measuring has no size either, and a part to add has its own two
        // numbers in place of a brush's.
        drop(app.update(Message::RailChosen(Rail::Measure)));
        assert!(!app.sized_now());
        drop(app.update(Message::RailChosen(Rail::Place)));
        assert!(!app.sized_now());
        assert_eq!(app.placing_numbers().len(), 2);
        drop(app.update(Message::RailChosen(Rail::Brush)));
        // Paint, by colour: an opacity and a falloff.
        drop(app.update(Message::BrushKindChanged(BrushKind::Paint)));
        assert!(app.painting_now());
        if app.painting_colour() {
            assert!(app.weight_number(false).is_some());
            assert!(app.falloff_choice().is_some());
        } else {
            assert!(app.weight_number(false).is_none());
            assert!(app.falloff_choice().is_none());
        }
        let _ = app.context_bar();
    }
}
