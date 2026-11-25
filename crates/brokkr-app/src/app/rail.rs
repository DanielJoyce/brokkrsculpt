// SPDX-License-Identifier: AGPL-3.0-only

//! The tool rail: the kinds of tool, down the left of the window.
//!
//! `docs/WORKSPACES.md` is the design (stage U3). It was a strip of the
//! eight brush kinds, then the mirror planes, then three modes. Brushes are
//! picked from the palette now, where every one of them is, and the keys 1
//! to 8 pick its first eight; the mirror planes are on the context bar over
//! the viewport. What is left is a kind of tool a button: the brush in
//! hand, upstream's Move and the paint brush (which are not brushes of the
//! palette), the mask, the transform gizmo and the cut.
//!
//! Each button keeps the word under its icon, and for the three modes the
//! word is a state, as it was on the strip: "armed" where the cut is about
//! to go off, "blur" where a mask drag will blur, "free" where a transform
//! will not snap. An icon says which tool; the word says whether the next
//! drag will cost the surface anything.

use brokkr_core::BrushKind;
use iced::widget::{button, column, container, scrollable, text};
use iced::{Alignment, Element, Length};

use super::{Brokkr, Tool};
use crate::icon::{self, IconName};
use crate::message::{Message, Rail, StudioMessage};
use crate::theme;

/// How wide the rail is: room for "Transform" at the caption size and the
/// scrollbar. Measured on the screen.
pub(super) const RAIL_WIDTH: f32 = 64.0;

impl Brokkr {
    /// Which rail button is lit.
    pub(super) fn rail_lit(&self) -> Rail {
        // Measuring takes the left click from every tool, so it is lit over
        // them; and something to add in hand is Kitbash's Place.
        if self.measure.armed {
            return Rail::Measure;
        }
        if self.tool == Tool::Sculpt && self.inserting() {
            return Rail::Place;
        }
        match self.tool {
            Tool::Mask => Rail::Mask,
            Tool::Transform => Rail::Transform,
            Tool::Cut => Rail::Cut,
            Tool::Sculpt => match self.brush.kind {
                BrushKind::Move => Rail::Move,
                BrushKind::Paint => Rail::Paint,
                _ => Rail::Brush,
            },
        }
    }

    /// What a press on a rail button does.
    pub(super) fn rail_chosen(&mut self, chosen: Rail) -> iced::Task<Message> {
        let to_sculpt = |app: &mut Self| -> iced::Task<Message> {
            if app.tool == Tool::Sculpt {
                iced::Task::none()
            } else {
                app.update(Message::ToolChanged(Tool::Sculpt))
            }
        };
        // Measuring is put down by any other button, as Escape puts it down.
        let mut measuring = iced::Task::none();
        if chosen != Rail::Measure && self.measure.armed {
            measuring = self.update(Message::Measure(crate::message::MeasureMessage::Armed(false)));
        }
        let task = match chosen {
            Rail::Measure => {
                let task = to_sculpt(self);
                return iced::Task::batch([
                    task,
                    self.update(Message::Measure(crate::message::MeasureMessage::Armed(
                        !self.measure.armed,
                    ))),
                ]);
            }
            Rail::Place => {
                let task = to_sculpt(self);
                let insert = self.kitbash_insert();
                iced::Task::batch([
                    task,
                    self.update(Message::Add(crate::message::AddMessage::Chosen(Some(insert)))),
                ])
            }
            Rail::Brush => {
                let task = to_sculpt(self);
                // Back from Move or Paint to the palette's brush that was
                // in hand when the rail left it, or the first.
                if matches!(self.brush.kind, BrushKind::Move | BrushKind::Paint) {
                    let index = self.rail_last_brush.unwrap_or(0);
                    return iced::Task::batch([
                        task,
                        self.update(Message::Studio(StudioMessage::PresetChosen(index))),
                    ]);
                }
                task
            }
            Rail::Move | Rail::Paint => {
                // The palette's brush in hand is remembered, for the Brush
                // button to come back to.
                if !matches!(self.brush.kind, BrushKind::Move | BrushKind::Paint) {
                    let now = self.preset_now(String::new());
                    let palette = self.studio.presets.palette();
                    self.rail_last_brush = Self::lit(&palette, &now).or(self.rail_last_brush);
                }
                let task = to_sculpt(self);
                let kind = if chosen == Rail::Move { BrushKind::Move } else { BrushKind::Paint };
                iced::Task::batch([task, self.update(Message::BrushKindChanged(kind))])
            }
            Rail::Mask => self.update(Message::ToolChanged(Tool::Mask)),
            Rail::Transform => self.update(Message::ToolChanged(Tool::Transform)),
            Rail::Cut => self.update(Message::ToolChanged(Tool::Cut)),
        };
        iced::Task::batch([measuring, task])
    }

    /// The rail itself.
    pub(super) fn rail(&self) -> Element<'_, Message> {
        let lit = self.rail_lit();
        let smoothing = self.shift;
        let button_of = |which: Rail| {
            let on = lit == which;
            let (name, word, tip): (IconName, &'static str, &'static str) = match which {
                // While shift is held every stroke smooths, so the button
                // says so, as the strip's Smooth lit up for it.
                // The icon is the kind of the brush in hand, so the button
                // says which brush the palette has lit.
                Rail::Brush => (
                    IconName::for_brush(match self.brush.kind {
                        BrushKind::Move | BrushKind::Paint => BrushKind::Draw,
                        kind => kind,
                    }),
                    if smoothing && on { "smooth" } else { "Brush" },
                    "The palette's brush in hand",
                ),
                Rail::Place => (IconName::Plus, "Place", "A drag on the model sets the part down"),
                Rail::Measure => (
                    IconName::Info,
                    if on { "on" } else { "Measure" },
                    "Measure: click two points on the model",
                ),
                Rail::Move => (IconName::BrushMove, "Move", "Move: a drag carries the surface"),
                Rail::Paint => (IconName::BrushPaint, "Paint", "Paint"),
                Rail::Mask => (
                    IconName::Mask,
                    match (on, smoothing) {
                        (true, true) => "blur",
                        (true, false) => "on",
                        (false, _) => "Mask",
                    },
                    "Mask, to protect what you have  M",
                ),
                Rail::Transform => (
                    IconName::Gizmo,
                    match (on, smoothing) {
                        (true, true) => "free",
                        (true, false) => "snap",
                        (false, _) => "Transform",
                    },
                    "Move, turn and scale the body  W",
                ),
                Rail::Cut => (
                    IconName::CutPlane,
                    if on { "armed" } else { "Cut" },
                    "Cut; with Alt, part it with keys  C",
                ),
            };
            let (style, ink) = theme::tool_toggle(on);
            crate::ui::tip(
                button(
                    column![
                        icon::icon(name, theme::ICON_TOOL, ink),
                        text(word).size(theme::CAPTION_SIZE),
                    ]
                    .width(Length::Fill)
                    .spacing(0)
                    .align_x(Alignment::Center),
                )
                .width(Length::Fill)
                .padding(theme::PAD_ICON_BUTTON)
                .style(style)
                .on_press(Message::RailChosen(which)),
                tip,
            )
        };
        let buttons = self
            .desk
            .now
            .rail()
            .iter()
            .copied()
            .fold(column![].spacing(theme::S2), |assembled, which| {
                assembled.push(button_of(which))
            });
        container(scrollable(buttons).height(Length::Fill))
            .padding(theme::S2)
            .width(Length::Fixed(RAIL_WIDTH))
            .height(Length::Fill)
            .style(theme::tool_strip)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    fn press(app: &mut Brokkr, which: Rail) {
        drop(app.update(Message::RailChosen(which)));
    }

    /// Every rail button lights itself when pressed, from any other one,
    /// and the Brush button brings back the palette's brush that was in
    /// hand before Move or Paint.
    #[test]
    fn each_rail_button_lights_itself_and_brush_comes_back() {
        let mut app = app();
        app.welcome = false;
        // A brush of the palette other than the first, so coming back to
        // the first would not pass for coming back to it.
        drop(app.update(Message::Studio(StudioMessage::PresetChosen(3))));
        let held = {
            let now = app.preset_now(String::new());
            let palette = app.studio.presets.palette();
            Brokkr::lit(&palette, &now)
        };
        assert_eq!(held, Some(3));
        // The brush under the modes: what the rail lights when a mode is
        // put down again.
        let mut under = Rail::Brush;
        let note = |app: &Brokkr, under: &mut Rail| {
            if matches!(app.rail_lit(), Rail::Brush | Rail::Move | Rail::Paint) {
                *under = app.rail_lit();
            }
        };
        for from in [Rail::Move, Rail::Paint, Rail::Mask, Rail::Cut, Rail::Brush] {
            for to in [Rail::Move, Rail::Paint, Rail::Mask, Rail::Cut, Rail::Brush] {
                press(&mut app, from);
                assert_eq!(app.rail_lit(), from, "{from:?} did not light itself");
                note(&app, &mut under);
                press(&mut app, to);
                // Pressing a mode that is up puts it down, as its key does,
                // and the brush that was in hand under it is lit again.
                if from == to && matches!(to, Rail::Mask | Rail::Cut) {
                    assert_eq!(app.rail_lit(), under, "{to:?} pressed twice stayed up");
                } else {
                    assert_eq!(app.rail_lit(), to, "{from:?} then {to:?} lit the wrong one");
                }
                note(&app, &mut under);
            }
        }
        press(&mut app, Rail::Paint);
        press(&mut app, Rail::Brush);
        let back = {
            let now = app.preset_now(String::new());
            let palette = app.studio.presets.palette();
            Brokkr::lit(&palette, &now)
        };
        assert_eq!(back, Some(3), "Brush did not bring back the brush in hand");
    }
}
