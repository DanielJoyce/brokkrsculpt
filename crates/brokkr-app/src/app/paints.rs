// SPDX-License-Identifier: AGPL-3.0-only

//! The paint rack: the document's paints, and the editor for the one in hand.
//!
//! A child module of `app` for the reason `panel.rs` is one, and a file of
//! its own rather than more of `panel.rs` or `app.rs` because both of those
//! are upstream's and this is not: `docs/STUDIO-PLAN.md` decision D6, add and
//! do not modify. What `app.rs` holds for this is two fields and four message
//! arms that call in here.
//!
//! # What a paint is, to the application
//!
//! [`brokkr_core::paint`] has the whole argument. In short a voxel carries a
//! slot number, the number names a recipe in the document's table, and the
//! recipe plus the shape of the surface give the colour. So everything here
//! edits the TABLE. No message in this file touches a voxel, which is why
//! none of them has anything to remesh once paints are in use, and why
//! changing a wash recolours every voxel that wears it in the same frame.
//!
//! # What is owed
//!
//! * **Undo.** An edit here dirties the document and is not in the history.
//!   The history is a list of changes to voxels and to the scene tree
//!   (`brokkr_core::undo::Change`), and a table edit is neither. It needs a
//!   variant of its own, in upstream's file, and that is its own piece of
//!   work.
//! * **Removing a paint.** Slots are positions in the table, so taking one
//!   out renumbers every paint after it, and every voxel painted with those
//!   would have to be rewritten to match. Until that pass exists a paint,
//!   once mixed, stays.

use brokkr_core::paint::{Finish, MAX_PAINTS, Paint, Wash};
use iced::widget::{button, column, container, pick_list, row, slider, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use super::Brokkr;
use crate::message::{Message, PaintCoat, PaintEdit, WashKind};
use crate::theme;

/// The pale coat a contrast paint goes over. A warm off-white: a contrast
/// paint over pure white reads as a highlighter pen.
pub(super) const UNDERCOAT: [u8; 3] = [232, 226, 214];

/// How much of the base coat's colour a ready-made shade keeps, per channel,
/// in sRGB levels. A shade is the base's own hue, darker: black in the
/// recesses of a red cloak reads as soot, and dark red reads as shadow.
const SHADE_OF_BASE: f32 = 0.35;

/// How many swatches go in a row of the rack and of the colour range.
const SWATCHES_A_ROW: usize = 8;

/// The slider ranges, which are also what an edit is clamped to on arrival.
pub(super) const STAIN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.5;
pub(super) const FLOW_RANGE: std::ops::RangeInclusive<f32> = 0.0..=3.0;
pub(super) const EDGE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.5;
pub(super) const AMOUNT_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
pub(super) const START_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.95;

/// A starting range of colours to pick from, so that mixing a paint begins
/// with a click and the three sliders are for adjusting.
///
/// Plain descriptions and nobody's product names. The order is the order on
/// the panel: neutrals, then round the wheel from red, then skin and metal.
pub(super) const RANGE: [(&str, [u8; 3]); 24] = [
    ("Black", [24, 24, 26]),
    ("Charcoal", [62, 64, 68]),
    ("Grey", [128, 130, 132]),
    ("Bone", [222, 208, 170]),
    ("Off-white", [236, 234, 226]),
    ("Dark brown", [58, 38, 26]),
    ("Leather", [122, 78, 44]),
    ("Sand", [196, 160, 104]),
    ("Dark red", [110, 20, 24]),
    ("Red", [190, 32, 36]),
    ("Orange", [224, 112, 32]),
    ("Yellow", [236, 196, 48]),
    ("Olive", [96, 104, 48]),
    ("Green", [44, 132, 60]),
    ("Dark green", [22, 70, 44]),
    ("Turquoise", [28, 144, 148]),
    ("Blue", [36, 84, 172]),
    ("Dark blue", [22, 36, 92]),
    ("Purple", [100, 48, 136]),
    ("Magenta", [176, 44, 112]),
    ("Pale flesh", [232, 184, 150]),
    ("Dark flesh", [140, 86, 60]),
    ("Steel", [150, 156, 164]),
    ("Gold", [200, 156, 64]),
];

/// The colour a ready-made shade wash takes over a base coat.
pub(super) fn shade_of(base: [u8; 3]) -> [u8; 3] {
    base.map(|level| (f32::from(level) * SHADE_OF_BASE).round() as u8)
}

fn swatch_colour(colour: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(colour[0], colour[1], colour[2])
}

/// A paint as one flat colour, for its swatch: what it looks like on a flat.
/// Not the base coat, because a contrast paint's base is the undercoat and a
/// rack of contrast paints would be a rack of identical off-whites.
fn on_a_flat(paint: &Paint) -> iced::Color {
    swatch_colour(brokkr_core::paint::srgb(paint.shade(0.0)))
}

impl Brokkr {
    /// The highest slot the paint brush may be given.
    ///
    /// With paints in use that is the last paint, and without it is the last
    /// filament, as it was. Never zero, which is "unpainted" and is what the
    /// brush's erase direction writes.
    pub(super) fn last_paint_slot(&self) -> u8 {
        let count =
            if self.paints.is_empty() { self.palette.slots.len() } else { self.paints.len() };
        count.clamp(1, usize::from(u8::MAX)) as u8
    }

    /// The paints belong to the document, so a new document has none: a
    /// fresh sphere and an imported mesh both start on filament slots.
    pub(super) fn forget_paints(&mut self) {
        self.paints = brokkr_core::paint::PaintTable::default();
        self.paint_slot = self.paint_slot.clamp(1, self.last_paint_slot());
    }

    /// Start using paints: one plain paint per filament slot.
    ///
    /// In the filament's own name and colour, so that a body already painted
    /// by slot looks exactly as it did the moment before, and what was
    /// "filament 2" is now a paint that can be given a wash.
    ///
    /// The one message here that remeshes. A wash reads the crease signal,
    /// the signal travels in the mesh, and a document without paints whose
    /// creases are switched off has meshes without it.
    pub(super) fn start_paints(&mut self) {
        if !self.paints.is_empty() {
            return;
        }
        for slot in self.palette.slots.iter().take(MAX_PAINTS) {
            let colour = slot.swatch().into_rgba8();
            self.paints.add(Paint::flat(slot.name.clone(), [colour[0], colour[1], colour[2]]));
        }
        if self.paints.is_empty() {
            // A machine with no filament slots at all still gets something to
            // paint with.
            self.paints.add(Paint::flat("Primer", RANGE[2].1));
        }
        self.paint_slot = self.paint_slot.clamp(1, self.last_paint_slot());
        self.unsaved = true;
        self.status = format!("mixed {} paints, one for each filament slot", self.paints.len());
        self.doc.mark_everything_dirty();
        self.remesh_dirty();
    }

    /// Add one more paint and take it up.
    pub(super) fn add_paint(&mut self) {
        self.start_paints();
        // Walk the range, so that a run of new paints is a run of different
        // colours and not the same one eight times.
        let (name, colour) = RANGE[self.paints.len() % RANGE.len()];
        match self.paints.add(Paint::flat(name, colour)) {
            Some(slot) => {
                self.paint_slot = slot;
                self.paint_coat = PaintCoat::Base;
                self.unsaved = true;
                self.status = format!("paint {slot} added");
            }
            None => {
                self.status = format!("the rack is full: a document holds {MAX_PAINTS} paints");
            }
        }
    }

    /// Change the paint in hand.
    ///
    /// Every number is clamped to its slider's range on arrival, for the
    /// reason the brush radius is: a message can come from anywhere.
    pub(super) fn edit_paint(&mut self, edit: PaintEdit) {
        let coat = self.paint_coat;
        let Some(paint) = self.paints.paint_mut(self.paint_slot) else {
            return;
        };
        let before = paint.clone();
        match edit {
            PaintEdit::Name(name) => paint.name = name,
            PaintEdit::Finish(finish) => paint.finish = finish,
            PaintEdit::Colour(colour) => *coat_colour(paint, coat) = colour,
            PaintEdit::Channel(channel, level) => {
                if let Some(slot) = coat_colour(paint, coat).get_mut(channel) {
                    *slot = level;
                }
            }
            PaintEdit::WashKind(kind) => match kind {
                WashKind::None => {
                    paint.wash = Wash { colour: paint.wash.colour, ..Wash::NONE };
                }
                WashKind::Shade => paint.wash = Wash::shade(shade_of(paint.base)),
                // A contrast paint is its pigment over a pale undercoat, so
                // the colour the paint had becomes the wash and the base
                // becomes the undercoat. Asked for twice, the base already
                // IS the undercoat, and then the pigment is left alone:
                // otherwise the second click would wash off-white over
                // off-white and the paint would be gone.
                WashKind::Contrast => {
                    let pigment =
                        if paint.base == UNDERCOAT { paint.wash.colour } else { paint.base };
                    paint.wash = Wash::contrast(pigment);
                    paint.base = UNDERCOAT;
                }
            },
            PaintEdit::WashStain(amount) => paint.wash.stain = clamped(amount, STAIN_RANGE),
            PaintEdit::WashFlow(amount) => paint.wash.flow = clamped(amount, FLOW_RANGE),
            PaintEdit::WashEdge(amount) => paint.wash.edge = clamped(amount, EDGE_RANGE),
            PaintEdit::HighlightAmount(amount) => {
                paint.highlight.amount = clamped(amount, AMOUNT_RANGE);
            }
            PaintEdit::HighlightStart(start) => {
                paint.highlight.start = clamped(start, START_RANGE);
            }
        }
        if *paint != before {
            self.unsaved = true;
        }
    }

    /// What the paint brush is offered: the filament slots while the document
    /// has no paints, and the rack once it has.
    ///
    /// `editing` is whether there is room for the editor under the rack. The
    /// panel has it and the pop-up menu does not.
    pub(super) fn paint_picker(&self, editing: bool) -> Element<'_, Message> {
        if self.paints.is_empty() {
            return column![
                self.filament_picker(),
                button(text("Mix paints").size(theme::TEXT_SIZE_SMALL))
                    .style(theme::tool_button)
                    .on_press(Message::PaintsStarted),
                text("washes and highlights, on top of the slots")
                    .size(theme::CAPTION_SIZE)
                    .color(theme::TEXT_MUTE),
            ]
            .spacing(theme::S2)
            .into();
        }

        let mut rack = column![].spacing(theme::S1);
        for (line, paints) in self.paints.paints().chunks(SWATCHES_A_ROW).enumerate() {
            let mut swatches = row![].spacing(theme::S1);
            for (place, paint) in paints.iter().enumerate() {
                let slot = (line * SWATCHES_A_ROW + place + 1) as u8;
                let (style, _) = theme::tool_toggle(slot == self.paint_slot);
                swatches = swatches.push(
                    button(
                        container(text(""))
                            .width(14)
                            .height(14)
                            .style(theme::swatch(on_a_flat(paint))),
                    )
                    .padding(theme::S1)
                    .style(style)
                    .on_press(Message::PaintSlotChanged(slot)),
                );
            }
            rack = rack.push(swatches);
        }

        let name =
            self.paints.paint(self.paint_slot).map(|paint| paint.name.as_str()).unwrap_or("none");
        let mut picker = column![
            text(format!("Paint  {}. {name}", self.paint_slot))
                .size(theme::TEXT_SIZE_SMALL)
                .color(theme::TEXT_DIM),
            rack,
        ]
        .spacing(theme::S2);
        if editing {
            let mut add =
                button(text("Add a paint").size(theme::TEXT_SIZE_SMALL)).style(theme::tool_button);
            if self.paints.len() < MAX_PAINTS {
                add = add.on_press(Message::PaintAdded);
            }
            picker = picker.push(add);
            if let Some(paint) = self.paints.paint(self.paint_slot) {
                picker = picker.push(self.paint_editor(paint));
            }
        }
        picker.into()
    }

    /// The paint in hand: its name, and one coat at a time.
    fn paint_editor<'a>(&'a self, paint: &'a Paint) -> Element<'a, Message> {
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let coat = self.paint_coat;

        let coats =
            PaintCoat::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, each| {
                assembled.push(
                    button(text(each.label()).size(theme::CAPTION_SIZE))
                        .width(Length::Fill)
                        .style(if each == coat {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::PaintCoatChosen(each)),
                )
            });

        let colour = match coat {
            PaintCoat::Base => paint.base,
            PaintCoat::Wash => paint.wash.colour,
            PaintCoat::Highlight => paint.highlight.colour,
        };
        let mut range = column![].spacing(theme::S1);
        for line in RANGE.chunks(SWATCHES_A_ROW) {
            let mut swatches = row![].spacing(theme::S1);
            for (_, each) in line {
                let (style, _) = theme::tool_toggle(*each == colour);
                swatches = swatches.push(
                    button(
                        container(text(""))
                            .width(14)
                            .height(14)
                            .style(theme::swatch(swatch_colour(*each))),
                    )
                    .padding(2)
                    .style(style)
                    .on_press(Message::PaintEdited(PaintEdit::Colour(*each))),
                );
            }
            range = range.push(swatches);
        }
        let channels = ["R", "G", "B"].into_iter().enumerate().fold(
            column![].spacing(theme::S1),
            |assembled, (channel, label)| {
                assembled.push(
                    row![
                        text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE).width(10),
                        slider(0..=255_u8, colour[channel], move |level| {
                            Message::PaintEdited(PaintEdit::Channel(channel, level))
                        }),
                        text(format!("{:3}", colour[channel]))
                            .size(theme::CAPTION_SIZE)
                            .color(theme::TEXT_DIM)
                            .width(22),
                    ]
                    .spacing(theme::S2)
                    .align_y(Alignment::Center),
                )
            },
        );

        let amounts: Element<'_, Message> = match coat {
            PaintCoat::Base => column![
                reading("Finish".to_string()),
                pick_list(Finish::ALL, Some(paint.finish), |finish| {
                    Message::PaintEdited(PaintEdit::Finish(finish))
                })
                .text_size(theme::CAPTION_SIZE)
                .width(Length::Fill),
            ]
            .spacing(theme::S1)
            .into(),
            PaintCoat::Wash => {
                let kinds =
                    WashKind::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, kind| {
                        assembled.push(
                            button(text(kind.label()).size(theme::CAPTION_SIZE))
                                .width(Length::Fill)
                                .style(theme::tool_button)
                                .on_press(Message::PaintEdited(PaintEdit::WashKind(kind))),
                        )
                    });
                column![
                    kinds,
                    reading(format!("On the flats  {:.2}", paint.wash.stain)),
                    slider(STAIN_RANGE, paint.wash.stain, |amount| {
                        Message::PaintEdited(PaintEdit::WashStain(amount))
                    })
                    .step(0.01_f32),
                    reading(format!("Into the recesses  {:.2}", paint.wash.flow)),
                    slider(FLOW_RANGE, paint.wash.flow, |amount| {
                        Message::PaintEdited(PaintEdit::WashFlow(amount))
                    })
                    .step(0.05_f32),
                    reading(format!("Off the edges  {:.2}", paint.wash.edge)),
                    slider(EDGE_RANGE, paint.wash.edge, |amount| {
                        Message::PaintEdited(PaintEdit::WashEdge(amount))
                    })
                    .step(0.01_f32),
                ]
                .spacing(theme::S1)
                .into()
            }
            PaintCoat::Highlight => column![
                reading(format!("How much  {:.2}", paint.highlight.amount)),
                slider(AMOUNT_RANGE, paint.highlight.amount, |amount| {
                    Message::PaintEdited(PaintEdit::HighlightAmount(amount))
                })
                .step(0.01_f32),
                // Low is a drybrush over every raised form. High is an edge
                // highlight on the sharpest lines only.
                reading(format!("Only edges sharper than  {:.2}", paint.highlight.start)),
                slider(START_RANGE, paint.highlight.start, |start| {
                    Message::PaintEdited(PaintEdit::HighlightStart(start))
                })
                .step(0.01_f32),
            ]
            .spacing(theme::S1)
            .into(),
        };

        column![
            text_input("name", &paint.name)
                .on_input(|name| Message::PaintEdited(PaintEdit::Name(name)))
                .padding(Padding { top: 1.0, bottom: 1.0, left: theme::S1, right: theme::S1 })
                .size(theme::TEXT_SIZE_SMALL)
                .width(Length::Fill),
            coats,
            range,
            channels,
            amounts,
        ]
        .spacing(theme::S2)
        .into()
    }
}

/// The colour a coat's swatches and sliders address.
fn coat_colour(paint: &mut Paint, coat: PaintCoat) -> &mut [u8; 3] {
    match coat {
        PaintCoat::Base => &mut paint.base,
        PaintCoat::Wash => &mut paint.wash.colour,
        PaintCoat::Highlight => &mut paint.highlight.colour,
    }
}

/// Into a slider's range, with a number that is not a number sent to the
/// bottom of it rather than through: `f32::clamp` passes a NaN, and a NaN in
/// a paint is a NaN in a uniform.
fn clamped(value: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if value.is_nan() { *range.start() } else { value.clamp(*range.start(), *range.end()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brokkr_core::BrushKind;
    use brokkr_core::paint::PaintTable;

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
    }

    fn edit(app: &mut Brokkr, edit: PaintEdit) {
        update(app, Message::PaintEdited(edit));
    }

    /// Mixing paints gives one per filament slot in the filament's colour, so
    /// a body painted by slot does not change colour under the user. It
    /// dirties the document, reaches the shader, and puts the crease signal
    /// into the meshes, which a wash is about to need.
    #[test]
    fn mixing_paints_copies_the_filament_slots_and_remeshes_with_the_signal() {
        let mut app = app();
        app.unsaved = false;
        assert!(app.paints.is_empty(), "a new document already had paints");
        assert_eq!(
            app.shared.paint_view().paints,
            brokkr_gpu::PaintBlock::default(),
            "the shader was told about paints a new document does not have"
        );
        let before = app.shared.pending_cavity_for_tests().len();

        update(&mut app, Message::PaintsStarted);

        assert_eq!(app.paints.len(), app.palette.slots.len(), "not one paint per filament slot");
        for (paint, slot) in app.paints.paints().iter().zip(&app.palette.slots) {
            let colour = slot.swatch().into_rgba8();
            assert_eq!(paint.base, [colour[0], colour[1], colour[2]], "{}", slot.name);
            assert_eq!(paint.name, slot.name);
            assert!(paint.wash.is_none() && paint.highlight.is_none(), "a new paint is plain");
        }
        assert!(app.unsaved, "mixing paints did not dirty the document");

        let view = app.shared.paint_view();
        assert_eq!(
            view.paints,
            brokkr_gpu::PaintBlock::from(&app.paints),
            "the table did not reach the shared frame"
        );
        assert_ne!(view.paints, brokkr_gpu::PaintBlock::default());

        let published = app.shared.pending_cavity_for_tests();
        let fresh = &published[before..];
        assert!(fresh.iter().any(|(vertices, _)| *vertices > 0), "mixing paints remeshed nothing");
        assert!(
            fresh.iter().all(|(vertices, cavity)| vertices == cavity),
            "the meshes do not carry the crease signal a wash reads"
        );

        // Asked for again, nothing happens: the table is not doubled.
        let count = app.paints.len();
        update(&mut app, Message::PaintsStarted);
        assert_eq!(app.paints.len(), count, "mixing twice doubled the rack");
    }

    /// With paints in use the crease toggle is only a view toggle: the meshes
    /// carry the signal either way, so it must neither strip it nor remesh.
    #[test]
    fn the_crease_toggle_leaves_the_signal_in_the_meshes_once_paints_need_it() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);
        let before = app.shared.pending_cavity_for_tests().len();

        update(&mut app, Message::ShowCavityToggled);
        update(&mut app, Message::ShowCavityToggled);
        assert_eq!(
            app.shared.pending_cavity_for_tests().len(),
            before,
            "the toggle remeshed a document whose meshes already had the signal"
        );

        // And anything that does remesh keeps the signal with creases hidden.
        app.doc.mark_everything_dirty();
        app.remesh_dirty();
        let published = app.shared.pending_cavity_for_tests();
        let fresh = &published[before..];
        assert!(fresh.iter().any(|(vertices, _)| *vertices > 0), "the control remeshed nothing");
        assert!(
            fresh.iter().all(|(vertices, cavity)| vertices == cavity),
            "a remesh with creases hidden dropped the signal a wash reads"
        );
    }

    /// The brush's slot is clamped to the rack once there is one, and a new
    /// paint is taken up as it is added.
    #[test]
    fn the_brush_is_offered_the_paints_and_takes_up_a_new_one() {
        let mut app = app();
        let filaments = app.palette.slots.len() as u8;
        update(&mut app, Message::PaintSlotChanged(200));
        assert_eq!(app.paint_slot, filaments, "without paints the limit is the filaments");

        update(&mut app, Message::PaintAdded);
        assert_eq!(app.paints.len(), usize::from(filaments) + 1, "adding did not mix first");
        assert_eq!(app.paint_slot, filaments + 1, "the new paint was not taken up");

        update(&mut app, Message::PaintSlotChanged(200));
        assert_eq!(app.paint_slot, filaments + 1, "a slot past the rack was accepted");
        update(&mut app, Message::PaintSlotChanged(0));
        assert_eq!(app.paint_slot, 1, "slot 0 is unpainted and cannot be chosen");

        // The rack fills and then says so, without wrapping the slot byte.
        for _ in 0..MAX_PAINTS + 3 {
            update(&mut app, Message::PaintAdded);
        }
        assert_eq!(app.paints.len(), MAX_PAINTS, "the rack went past what the shader holds");
        assert!(app.status.contains("full"), "a full rack was not reported: {}", app.status);
        assert!(usize::from(app.paint_slot) <= MAX_PAINTS);
    }

    /// Every edit lands on the paint in hand and the coat shown, reaches the
    /// shader, dirties the document, and touches no voxel and no history.
    #[test]
    fn an_edit_changes_the_paint_in_hand_and_nothing_else() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);
        update(&mut app, Message::PaintSlotChanged(2));
        let others: Vec<_> = app
            .paints
            .paints()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, p)| p.clone())
            .collect();
        let undo_before = app.history.stats();
        let meshes_before = app.shared.pending_cavity_for_tests().len();
        app.unsaved = false;

        edit(&mut app, PaintEdit::Colour([10, 20, 30]));
        assert_eq!(app.paints.paint(2).unwrap().base, [10, 20, 30]);
        assert!(app.unsaved, "an edit did not dirty the document");

        update(&mut app, Message::PaintCoatChosen(PaintCoat::Wash));
        edit(&mut app, PaintEdit::Colour([40, 50, 60]));
        edit(&mut app, PaintEdit::Channel(1, 99));
        edit(&mut app, PaintEdit::Channel(7, 1));
        assert_eq!(app.paints.paint(2).unwrap().wash.colour, [40, 99, 60]);
        assert_eq!(app.paints.paint(2).unwrap().base, [10, 20, 30], "the wash edit hit the base");

        update(&mut app, Message::PaintCoatChosen(PaintCoat::Highlight));
        edit(&mut app, PaintEdit::Colour([200, 210, 220]));
        assert_eq!(app.paints.paint(2).unwrap().highlight.colour, [200, 210, 220]);

        edit(&mut app, PaintEdit::WashStain(0.4));
        edit(&mut app, PaintEdit::WashFlow(2.0));
        edit(&mut app, PaintEdit::WashEdge(0.3));
        edit(&mut app, PaintEdit::HighlightAmount(0.6));
        edit(&mut app, PaintEdit::HighlightStart(0.5));
        edit(&mut app, PaintEdit::Name("Cloak".to_string()));
        edit(&mut app, PaintEdit::Finish(Finish::Gloss));
        let paint = app.paints.paint(2).unwrap().clone();
        assert_eq!(
            (paint.wash.stain, paint.wash.flow, paint.wash.edge),
            (0.4, 2.0, 0.3),
            "a wash amount went to the wrong place"
        );
        assert_eq!((paint.highlight.amount, paint.highlight.start), (0.6, 0.5));
        assert_eq!((paint.name.as_str(), paint.finish), ("Cloak", Finish::Gloss));

        let after: Vec<_> = app
            .paints
            .paints()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, p)| p.clone())
            .collect();
        assert_eq!(after, others, "an edit reached a paint that was not in hand");
        assert_eq!(
            app.shared.paint_view().paints,
            brokkr_gpu::PaintBlock::from(&app.paints),
            "the edited table did not reach the shared frame"
        );
        assert_eq!(app.history.stats(), undo_before, "a table edit reached the voxel history");
        assert_eq!(
            app.shared.pending_cavity_for_tests().len(),
            meshes_before,
            "a table edit remeshed, and it changes no voxel"
        );

        // An edit that changes nothing does not dirty a clean document.
        app.unsaved = false;
        edit(&mut app, PaintEdit::WashFlow(2.0));
        assert!(!app.unsaved, "an edit that changed nothing dirtied the document");
    }

    /// Numbers from anywhere are held to the sliders' ranges, and a NaN goes
    /// to the bottom of one instead of into a uniform.
    #[test]
    fn amounts_are_clamped_and_a_nan_never_reaches_a_paint() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);

        edit(&mut app, PaintEdit::WashStain(99.0));
        edit(&mut app, PaintEdit::WashFlow(-4.0));
        edit(&mut app, PaintEdit::WashEdge(f32::INFINITY));
        edit(&mut app, PaintEdit::HighlightAmount(7.0));
        edit(&mut app, PaintEdit::HighlightStart(1.0));
        let paint = app.paints.paint(app.paint_slot).unwrap().clone();
        assert_eq!(paint.wash.stain, *STAIN_RANGE.end());
        assert_eq!(paint.wash.flow, *FLOW_RANGE.start());
        assert_eq!(paint.wash.edge, *EDGE_RANGE.end());
        assert_eq!(paint.highlight.amount, *AMOUNT_RANGE.end());
        assert_eq!(paint.highlight.start, *START_RANGE.end(), "a start of 1 divides by nothing");

        for nan in [
            PaintEdit::WashStain(f32::NAN),
            PaintEdit::WashFlow(f32::NAN),
            PaintEdit::WashEdge(f32::NAN),
            PaintEdit::HighlightAmount(f32::NAN),
            PaintEdit::HighlightStart(f32::NAN),
        ] {
            edit(&mut app, nan);
        }
        let paint = app.paints.paint(app.paint_slot).unwrap();
        for amount in [
            paint.wash.stain,
            paint.wash.flow,
            paint.wash.edge,
            paint.highlight.amount,
            paint.highlight.start,
        ] {
            assert_eq!(amount, 0.0, "a NaN was not sent to the bottom of its range");
        }
    }

    /// The three ready-made washes. A shade is the base's hue, darker, and
    /// leaves the base alone. A contrast moves the colour into the wash over
    /// the undercoat, and asked for twice it does not wash the paint away.
    #[test]
    fn the_ready_made_washes_are_what_their_names_say() {
        let mut app = app();
        update(&mut app, Message::PaintsStarted);
        let red = [190, 32, 36];
        edit(&mut app, PaintEdit::Colour(red));
        let slot = app.paint_slot;

        edit(&mut app, PaintEdit::WashKind(WashKind::Shade));
        let paint = app.paints.paint(slot).unwrap().clone();
        assert_eq!(paint.base, red, "a shade repainted the base coat");
        assert_eq!(paint.wash, Wash::shade(shade_of(red)));
        assert!(paint.wash.colour.iter().zip(red).all(|(wash, base)| *wash < base.max(1)));
        let flat = paint.shade(0.0);
        let crease = paint.shade(1.0);
        assert!(crease[0] < flat[0] * 0.5, "a shade did not darken the recess");

        edit(&mut app, PaintEdit::WashKind(WashKind::Contrast));
        let paint = app.paints.paint(slot).unwrap().clone();
        assert_eq!(paint.base, UNDERCOAT);
        assert_eq!(paint.wash, Wash::contrast(red), "the colour did not move into the wash");

        edit(&mut app, PaintEdit::WashKind(WashKind::Contrast));
        let again = app.paints.paint(slot).unwrap().clone();
        assert_eq!(again, paint, "asking for contrast twice washed the pigment away");

        edit(&mut app, PaintEdit::WashKind(WashKind::None));
        let paint = app.paints.paint(slot).unwrap().clone();
        assert!(paint.wash.is_none(), "None left a wash on");
        assert_eq!(paint.wash.colour, red, "None threw the pigment away with the film");
        assert_eq!(paint.shade(1.0), paint.shade(0.0), "a paint with no wash still shades");
    }

    /// Paints are the document's: they are saved with it, come back when it
    /// is opened, and do not follow the user into a new document.
    #[test]
    fn paints_are_saved_with_the_document_and_do_not_outlive_it() {
        let directory = std::env::temp_dir().join(format!("brokkr-paints-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        let path = directory.join("painted.brokkr");

        let mut app = app();
        update(&mut app, Message::PaintAdded);
        edit(&mut app, PaintEdit::Name("Cloak".to_string()));
        edit(&mut app, PaintEdit::WashKind(WashKind::Contrast));
        update(&mut app, Message::PaintCoatChosen(PaintCoat::Highlight));
        edit(&mut app, PaintEdit::HighlightAmount(0.5));
        let saved = app.paints.clone();
        let slot = app.paint_slot;
        app.save_project(&path);
        assert!(!app.unsaved, "the save failed: {}", app.status);

        let mut reopened = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        reopened.paint_slot = 200;
        reopened.open_project(&path);
        assert_eq!(reopened.paints, saved, "the paints did not come back: {}", reopened.status);
        assert!(usize::from(reopened.paint_slot) <= saved.len(), "the slot outran the rack");
        assert_eq!(
            reopened.shared.paint_view().paints,
            brokkr_gpu::PaintBlock::from(&saved),
            "an opened document's paints did not reach the shader"
        );
        assert_eq!(saved.paint(slot).unwrap().name, "Cloak");

        // A fresh sphere is a new document and starts with none.
        update(&mut reopened, Message::ResetSphere);
        assert_eq!(reopened.paints, PaintTable::default(), "paints outlived their document");
        assert!(usize::from(reopened.paint_slot) <= reopened.palette.slots.len());

        // And so does a document opened over a painted one, when the file has
        // none: the control that the open path assigns and does not merge.
        let plain = directory.join("plain.brokkr");
        let mut bare = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        bare.save_project(&plain);
        app.open_project(&plain);
        assert!(app.paints.is_empty(), "opening an unpainted file kept the old paints");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The panel builds with and without paints and with each coat showing.
    /// `view` borrows the whole application, so a widget tree that indexes
    /// past a rack or a range panics here and not in front of a user.
    #[test]
    fn the_rack_and_the_editor_build_for_every_coat() {
        let mut app = app();
        drop(app.paint_picker(true));
        drop(app.paint_picker(false));
        for _ in 0..MAX_PAINTS {
            update(&mut app, Message::PaintAdded);
        }
        for coat in PaintCoat::ALL {
            update(&mut app, Message::PaintCoatChosen(coat));
            drop(app.paint_picker(true));
            drop(app.paint_picker(false));
        }
        // A slot the table does not have draws no editor and does not panic.
        app.paint_slot = 200;
        drop(app.paint_picker(true));
    }

    /// The range is the starting colours a painter is offered, so two entries
    /// the same would be one wasted, and the undercoat must not be one of
    /// them or picking it would silently mean "keep the old pigment".
    #[test]
    fn the_range_has_no_repeats_and_does_not_hold_the_undercoat() {
        for (index, (name, colour)) in RANGE.iter().enumerate() {
            assert_ne!(*colour, UNDERCOAT, "{name} is the undercoat");
            for (other, other_colour) in &RANGE[index + 1..] {
                assert_ne!(colour, other_colour, "{name} and {other} are the same colour");
                assert_ne!(name, other, "two paints are called {name}");
            }
        }
    }
}
