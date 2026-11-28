// SPDX-License-Identifier: AGPL-3.0-only

//! The paint designer: a card for making a paint or changing one, as a
//! brush is made in its block.
//!
//! Asked for by the maintainer on 2026-11-25: "Color sliders, mixer, paint
//! effects, we should have some kind of paint designer popup for designing
//! paints just like we have for brushes. Everything related to color
//! should move to that." So the colour's sliders, a hex field, the
//! machine's filament colours and a mixer are here and not in the panel;
//! the panel keeps what is used while painting (the rack, the way of
//! putting paint on, the opacity, the eyedropper).
//!
//! A paint is a name, a colour and a kind. The kind decides how it goes on
//! by default (its touch) and the layer it goes on (its blend, and metal),
//! which is the paint's effect; the picture shows it on the slapchopped
//! ball (`app/paint_preview.rs`). Save puts it on the rack in hand, or in
//! place of the paint it was made from, and takes it up.
//!
//! A metallic has a Chrome box: ticked, it is laid on a chrome layer and
//! drawn nearly a mirror, as the rare true chrome paints are, and not as a
//! miniature paint's flake metal (issue 203). It is kept with the paint on
//! the rack.

use iced::widget::{
    button, checkbox, column, container, image, pick_list, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Length};

use super::Hani;
use super::rack::RackPaint;
use crate::message::{DesignerMessage, Message};
use crate::paint_index::{Kind, hex};
use crate::theme;

/// How big the designer's picture of the paint is, in pixels.
const PICTURE: usize = 160;

/// A paint being made or changed.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Designer {
    /// The rack paint it was opened on, or none for a new one.
    pub editing: Option<usize>,
    pub name: String,
    pub kind: Kind,
    /// A metallic is chrome. Kept whatever the kind, so that a kind tried
    /// and changed back does not lose it; saved only on a metallic.
    pub chrome: bool,
    /// The colour before anything is mixed into it.
    pub base: [u8; 3],
    /// What is mixed in, and how much of it, 0..=1.
    pub mix_with: Option<[u8; 3]>,
    pub mix: f32,
    /// The hex field as typed.
    pub hex: String,
}

impl Designer {
    fn new(editing: Option<usize>, name: String, kind: Kind, base: [u8; 3]) -> Self {
        Self {
            editing,
            name,
            kind,
            chrome: false,
            base,
            mix_with: None,
            mix: 0.5,
            hex: hex_of(base),
        }
    }

    /// Whether it is drawn and laid as chrome.
    fn is_chrome(&self) -> bool {
        self.kind == Kind::Metallic && self.chrome
    }

    /// The colour as it is: the base with what is mixed in, mixed as
    /// paints are, in linear light.
    pub(super) fn colour(&self) -> [u8; 3] {
        let Some(with) = self.mix_with else {
            return self.base;
        };
        let (a, b) = (hani_core::paint::linear(self.base), hani_core::paint::linear(with));
        hani_core::paint::srgb(std::array::from_fn(|c| a[c] + (b[c] - a[c]) * self.mix))
    }
}

fn hex_of(rgb: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

fn swatch(rgb: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(rgb[0], rgb[1], rgb[2])
}

impl Hani {
    pub(super) fn designer_changed(&mut self, message: DesignerMessage) {
        match message {
            DesignerMessage::New => {
                let colour = self.painting.colour;
                self.painting.designer =
                    Some(Designer::new(None, String::new(), Kind::Paint, colour));
            }
            DesignerMessage::Edit(at) => {
                let Some(paint) = self.painting.rack.rack().paints.get(at).cloned() else {
                    return;
                };
                let mut designer = Designer::new(Some(at), paint.name, paint.kind, paint.rgb);
                designer.chrome = paint.chrome;
                self.painting.designer = Some(designer);
            }
            DesignerMessage::Close => self.painting.designer = None,
            DesignerMessage::Save => self.save_the_design(),
            other => {
                let Some(designer) = self.painting.designer.as_mut() else {
                    return;
                };
                match other {
                    DesignerMessage::Name(name) => designer.name = name,
                    DesignerMessage::Kind(kind) => designer.kind = kind,
                    DesignerMessage::Chrome(chrome) => designer.chrome = chrome,
                    DesignerMessage::Colour(rgb) => {
                        designer.base = rgb;
                        designer.mix_with = None;
                        designer.hex = hex_of(rgb);
                    }
                    DesignerMessage::Channel(channel, level) => {
                        if let Some(slot) = designer.base.get_mut(channel) {
                            *slot = level;
                        }
                        designer.hex = hex_of(designer.base);
                    }
                    DesignerMessage::Hex(typed) => {
                        if let Some(rgb) = hex(&typed) {
                            designer.base = rgb;
                        }
                        designer.hex = typed;
                    }
                    DesignerMessage::MixWith(with) => designer.mix_with = with,
                    DesignerMessage::Mix(amount) => designer.mix = amount.clamp(0.0, 1.0),
                    DesignerMessage::New
                    | DesignerMessage::Edit(_)
                    | DesignerMessage::Close
                    | DesignerMessage::Save => {}
                }
            }
        }
    }

    /// Put the paint designed on the rack, or in place of the one it was
    /// made from, and take it up.
    fn save_the_design(&mut self) {
        let Some(designer) = self.painting.designer.clone() else {
            return;
        };
        let name = designer.name.trim().to_string();
        if name.is_empty() {
            self.status = "give the paint a name first".to_string();
            return;
        }
        let paint = RackPaint {
            name: name.clone(),
            brand: String::new(),
            rgb: designer.colour(),
            kind: designer.kind,
            chrome: designer.is_chrome(),
        };
        let (at, kept) = self.painting.rack.keep_paint(designer.editing, paint);
        self.painting.designer = None;
        self.rack_changed(crate::message::RackMessage::Picked(at));
        self.status = match kept {
            Ok(()) => format!("{name} is on the rack"),
            Err(why) => format!("the racks were not saved: {why}"),
        };
    }

    /// The card.
    pub(super) fn designer_card(&self) -> Option<Element<'_, Message>> {
        let designer = self.painting.designer.as_ref()?;
        let send = Message::Designer;
        let label =
            |words: &'static str| text(words).size(theme::LABEL_SIZE).color(theme::TEXT_DIM);
        let colour = designer.colour();

        let picture = image(super::paint_preview::handle(
            colour,
            designer.kind,
            designer.is_chrome(),
            PICTURE,
            self.painting.pooling,
        ))
        .width(Length::Fixed(PICTURE as f32))
        .height(Length::Fixed(PICTURE as f32));
        let kinds: Vec<Kind> =
            Kind::ALL.into_iter().filter(|kind| *kind != Kind::Reference).collect();
        let effect = match designer.kind.touch() {
            Some(touch) => touch.label(),
            None => "Loaded",
        };
        let left = column![
            picture,
            row![label("Goes on"), text(effect).size(theme::LABEL_SIZE).color(theme::TEXT)]
                .spacing(theme::S2),
        ]
        .spacing(theme::S2)
        .align_x(Alignment::Center);

        // Chrome, for a metallic only: the rare paint that is nearly a
        // mirror.
        let chrome: Element<'_, Message> = if designer.kind == Kind::Metallic {
            crate::ui::tip(
                checkbox(designer.chrome)
                    .label("Chrome")
                    .on_toggle(|on| Message::Designer(DesignerMessage::Chrome(on)))
                    .text_size(theme::LABEL_SIZE),
                "Nearly a mirror, not a metallic paint's fine flakes",
            )
        } else {
            space().height(0).into()
        };

        let channels =
            [("designer.red", "Red"), ("designer.green", "Green"), ("designer.blue", "Blue")]
                .into_iter()
                .enumerate()
                .fold(column![].spacing(theme::S2), |assembled, (channel, (key, name))| {
                    assembled.push(
                        crate::ui::number(
                            &self.drafts,
                            key,
                            name,
                            f32::from(designer.base[channel]),
                            0.0..=255.0,
                            move |level| {
                                Message::Designer(DesignerMessage::Channel(
                                    channel,
                                    level.round() as u8,
                                ))
                            },
                        )
                        .step(1.0)
                        .shown(designer.base[channel].to_string()),
                    )
                });

        // Quick colours: the machine's filaments.
        let mut quick = row![].spacing(theme::S1);
        for slot in &self.palette.slots {
            let rgba = slot.swatch().into_rgba8();
            let rgb = [rgba[0], rgba[1], rgba[2]];
            quick = quick.push(crate::ui::tip(
                button(container(text("")).width(14).height(14).style(theme::swatch(swatch(rgb))))
                    .padding(2)
                    .style(theme::tool_button)
                    .on_press(send(DesignerMessage::Colour(rgb))),
                "Take this filament's colour",
            ));
        }

        // The mixer: a rack paint to mix in, and how much.
        let mut with = row![crate::ui::tip(
            button(text("None").size(theme::LABEL_SIZE))
                .padding(theme::PAD_CHOICE)
                .style(if designer.mix_with.is_none() {
                    theme::tool_button_active
                } else {
                    theme::tool_button
                })
                .on_press(send(DesignerMessage::MixWith(None))),
            "Mix in nothing",
        )]
        .spacing(theme::S1)
        .align_y(Alignment::Center);
        for paint in self.painting.rack.rack().paints.iter().take(10) {
            let (style, _) = theme::tool_toggle(designer.mix_with == Some(paint.rgb));
            with = with.push(crate::ui::tip(
                button(
                    container(text(""))
                        .width(14)
                        .height(14)
                        .style(theme::swatch(swatch(paint.rgb))),
                )
                .padding(2)
                .style(style)
                .on_press(send(DesignerMessage::MixWith(Some(paint.rgb)))),
                paint.name.clone(),
            ));
        }
        let mut mixer = column![label("Mix with"), with].spacing(theme::S1);
        if designer.mix_with.is_some() {
            mixer = mixer.push(
                crate::ui::number(
                    &self.drafts,
                    "designer.mix",
                    "How much",
                    designer.mix,
                    0.0..=1.0,
                    |v| Message::Designer(DesignerMessage::Mix(v)),
                )
                .shown(format!("{:.0}%", designer.mix * 100.0)),
            );
        }

        let right = column![
            text_input("Paint name", &designer.name)
                .on_input(|name| Message::Designer(DesignerMessage::Name(name)))
                .on_submit(send(DesignerMessage::Save))
                .size(theme::LABEL_SIZE)
                .padding(theme::PAD_FIELD),
            row![
                label("Kind"),
                pick_list(kinds, Some(designer.kind), |kind| Message::Designer(
                    DesignerMessage::Kind(kind)
                ))
                .text_size(theme::LABEL_SIZE)
                .width(Length::Fill),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
            chrome,
            row![
                container(text("")).width(36).height(22).style(theme::swatch(swatch(colour))),
                text_input("RRGGBB", &designer.hex)
                    .on_input(|typed| Message::Designer(DesignerMessage::Hex(typed)))
                    .size(theme::LABEL_SIZE)
                    .padding(theme::PAD_FIELD)
                    .width(Length::Fixed(90.0)),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
            channels,
            column![label("Filaments"), quick].spacing(theme::S1),
            mixer,
        ]
        .spacing(theme::S3)
        .width(Length::Fill);

        let title = if designer.editing.is_some() { "Change paint" } else { "New paint" };
        let card = container(
            column![
                row![
                    text(title).size(theme::TITLE_SIZE).color(theme::TEXT),
                    space().width(Length::Fill),
                    crate::ui::tip(
                        crate::ui::button_secondary("Cancel", Some(send(DesignerMessage::Close))),
                        "Close without keeping the changes"
                    ),
                    crate::ui::tip(
                        button(text("Save").size(theme::LABEL_SIZE))
                            .padding(theme::PAD_CHOICE)
                            .style(theme::tool_button_active)
                            .on_press(send(DesignerMessage::Save)),
                        "Keep the changes"
                    ),
                ]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
                scrollable(
                    row![left, right]
                        .spacing(theme::S5)
                        .padding(iced::Padding { right: theme::S4, ..iced::Padding::ZERO })
                )
                .height(Length::Shrink),
            ]
            .spacing(theme::S4),
        )
        .padding(theme::S5)
        .max_width(560.0)
        .max_height(640.0)
        .style(theme::menu_card);
        Some(super::panel::modal_layer(card))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn design(app: &mut Hani, message: DesignerMessage) {
        update(app, Message::Designer(message));
    }

    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Paint));
        update(&mut app, Message::WorkspaceChosen(crate::message::Workspace::Paint));
        app
    }

    /// A paint designed from the colour in hand goes on the rack by its
    /// name, kind and colour, and is taken up; a name is needed.
    #[test]
    fn a_paint_is_designed_and_put_on_the_rack() {
        let mut app = app();
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour([10, 200, 30])));
        design(&mut app, DesignerMessage::New);
        assert_eq!(app.painting.designer.as_ref().expect("open").base, [10, 200, 30]);
        design(&mut app, DesignerMessage::Save);
        assert!(app.painting.designer.is_some(), "a paint with no name was kept");
        design(&mut app, DesignerMessage::Name("Goblin".to_string()));
        design(&mut app, DesignerMessage::Kind(Kind::Contrast));
        design(&mut app, DesignerMessage::Hex("#2050A0".to_string()));
        design(&mut app, DesignerMessage::Channel(0, 40));
        drop(app.designer_card());
        design(&mut app, DesignerMessage::Save);
        assert!(app.painting.designer.is_none());
        let paint = app.painting.rack.rack().paints.last().expect("a paint").clone();
        assert_eq!(
            (paint.name.as_str(), paint.kind, paint.rgb),
            ("Goblin", Kind::Contrast, [40, 0x50, 0xA0])
        );
        assert_eq!(app.painting.colour, [40, 0x50, 0xA0], "the paint was not taken up");
        assert_eq!(app.touch.kind, crate::message::PaintTouch::Contrast);
    }

    /// A metallic designed as chrome is kept as chrome on the rack, read
    /// back from the rack's file as chrome, laid on a chrome layer, and
    /// opened again with its box ticked; the box means nothing on a paint
    /// of another kind.
    #[test]
    fn a_chrome_metallic_is_kept_and_laid_as_chrome() {
        let mut app = app();
        design(&mut app, DesignerMessage::New);
        design(&mut app, DesignerMessage::Name("Mirror".to_string()));
        design(&mut app, DesignerMessage::Kind(Kind::Metallic));
        design(&mut app, DesignerMessage::Chrome(true));
        drop(app.designer_card());
        design(&mut app, DesignerMessage::Save);
        let at = app.painting.rack.rack().paints.len() - 1;
        let paint = app.painting.rack.rack().paints[at].clone();
        assert!(paint.chrome && paint.kind == Kind::Metallic, "{paint:?}");
        let back = super::super::rack::Rack::from_text(&app.painting.rack.rack().to_text())
            .expect("the rack reads back");
        assert!(back.paints[at].chrome, "chrome was lost in the rack's file");
        let volume = app.doc.active_volume();
        let meta = &volume.layers().metas()[volume.layers().active()];
        assert!(meta.metal && meta.chrome, "a chrome paint is not on a chrome layer: {meta:?}");

        design(&mut app, DesignerMessage::Edit(at));
        assert!(app.painting.designer.as_ref().expect("open").chrome);
        design(&mut app, DesignerMessage::Kind(Kind::Paint));
        design(&mut app, DesignerMessage::Save);
        assert!(!app.painting.rack.rack().paints[at].chrome, "a plain paint kept chrome");
    }

    /// A rack paint changed in the designer is changed in its place.
    #[test]
    fn a_rack_paint_is_changed_in_its_place() {
        let mut app = app();
        let count = app.painting.rack.rack().paints.len();
        design(&mut app, DesignerMessage::Edit(2));
        design(&mut app, DesignerMessage::Name("Mine".to_string()));
        design(&mut app, DesignerMessage::Save);
        assert_eq!(app.painting.rack.rack().paints.len(), count);
        assert_eq!(app.painting.rack.rack().paints[2].name, "Mine");
        design(&mut app, DesignerMessage::Edit(2));
        design(&mut app, DesignerMessage::Close);
        assert!(app.painting.designer.is_none());
    }

    proptest! {
        /// Mixing nothing in is the base; all of something in is that
        /// thing; and a mix lies between the two, channel by channel.
        #[test]
        fn a_mix_lies_between_its_two_colours(
            a in any::<[u8; 3]>(),
            b in any::<[u8; 3]>(),
            amount in 0.0f32..1.0,
        ) {
            let mut designer = Designer::new(None, String::new(), Kind::Paint, a);
            designer.mix_with = Some(b);
            designer.mix = 0.0;
            prop_assert_eq!(designer.colour(), a);
            designer.mix = 1.0;
            let all = designer.colour();
            for c in 0..3 { prop_assert!(all[c].abs_diff(b[c]) <= 1); }
            designer.mix = amount;
            let mixed = designer.colour();
            for c in 0..3 {
                prop_assert!(u16::from(mixed[c]) + 1 >= u16::from(a[c].min(b[c]))
                        && u16::from(mixed[c]) <= u16::from(a[c].max(b[c])) + 1);
            }
        }
    }
}
