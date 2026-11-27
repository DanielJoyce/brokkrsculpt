// SPDX-License-Identifier: AGPL-3.0-only

//! COATS, in the Paint mode's settings: a base coat, a zenithal and a
//! spongechop in one press each, on layers of their own. Stage 4 of
//! `docs/PAINT-RACK.md`, section 7; the coats themselves are
//! [`hani_core::paint::coats`].
//!
//! Each colour of a coat is a swatch: pressing it takes the colour in
//! hand, so a coat's colours come from the rack. Each Apply is one undo
//! entry holding the stack as it was. The light the zenithal and the
//! spongechop come from is also the sponge's, which prefers what faces
//! it.

use hani_core::paint::coats::{ABOVE, Spongechop, Zenithal, light_from};
use iced::widget::{button, checkbox, column, container, row, text};
use iced::{Alignment, Element, Length};

use super::Hani;
use crate::message::{CoatColour, CoatsMessage, Message};
use crate::theme;

/// How wide a zenithal's cone may be, in degrees.
const CONE_RANGE: std::ops::RangeInclusive<f32> = 5.0..=90.0;

/// The coats' settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CoatsState {
    pub open: bool,
    /// Where the light is: degrees round from the front, and up from level.
    pub azimuth: f32,
    pub height: f32,
    pub zenithal: Zenithal,
    /// The zenithal's middle primer, kept while it is off.
    pub middle: [u8; 3],
    pub chop: Spongechop,
    /// The spongechop's contrast, kept while it is off.
    pub contrast: [u8; 3],
    /// The finish a varnish coat lays, and a stroke on a Varnish layer.
    pub varnish: hani_core::paint::Finish,
}

impl Default for CoatsState {
    fn default() -> Self {
        Self {
            open: true,
            azimuth: ABOVE.0,
            height: ABOVE.1,
            zenithal: Zenithal::default(),
            middle: [128, 128, 124],
            chop: Spongechop::default(),
            contrast: [150, 24, 20],
            varnish: hani_core::paint::Finish::Matte,
        }
    }
}

impl CoatsState {
    fn light(&self) -> glam::Vec3 {
        light_from(self.azimuth, self.height)
    }
}

impl Hani {
    /// What is on the brush, for a stroke in `direction`: on a Varnish
    /// layer, the finish chosen, as its code, and not the colour in hand.
    /// See `hani_core::paint::layers`.
    pub(super) fn pigment(
        &self,
        direction: hani_core::BrushDirection,
    ) -> hani_core::paint::stroke::Pigment {
        let pigment = self.painting.pigment(direction);
        let layers = self.doc.active_volume().layers();
        let varnish = layers
            .metas()
            .get(layers.active())
            .is_some_and(|meta| meta.blend == hani_core::paint::layers::Blend::Varnish);
        if !varnish {
            return pigment;
        }
        let code = self.painting.coats.varnish.code();
        hani_core::paint::stroke::Pigment { colour: [code, 0, 0], opacity: 1.0, ..pigment }
    }

    pub(super) fn coats_changed(&mut self, message: CoatsMessage) {
        let coats = &mut self.painting.coats;
        match message {
            CoatsMessage::Opened => coats.open = !coats.open,
            CoatsMessage::Above => (coats.azimuth, coats.height) = ABOVE,
            CoatsMessage::FromView => {
                let toward = (self.camera.eye() - self.camera.target).normalize_or(glam::Vec3::Y);
                coats.height = toward.y.clamp(-1.0, 1.0).asin().to_degrees();
                coats.azimuth = toward.x.atan2(toward.z).to_degrees();
            }
            CoatsMessage::Azimuth(degrees) => coats.azimuth = degrees.clamp(-180.0, 180.0),
            CoatsMessage::Height(degrees) => coats.height = degrees.clamp(-90.0, 90.0),
            CoatsMessage::Take(which) => {
                let colour = self.painting.colour;
                match which {
                    CoatColour::Dark => coats.zenithal.dark = colour,
                    CoatColour::Middle => {
                        coats.middle = colour;
                        coats.zenithal.middle = Some(colour);
                    }
                    CoatColour::Bright => coats.zenithal.bright = colour,
                    CoatColour::ChopDark => coats.chop.dark = colour,
                    CoatColour::ChopMiddle => coats.chop.middle = colour,
                    CoatColour::ChopBright => coats.chop.bright = colour,
                    CoatColour::Contrast => {
                        coats.contrast = colour;
                        coats.chop.contrast = Some(colour);
                    }
                }
            }
            CoatsMessage::Middle(on) => {
                coats.zenithal.middle = on.then_some(coats.middle);
            }
            CoatsMessage::Cone(degrees) => {
                coats.zenithal.cone = degrees.clamp(*CONE_RANGE.start(), *CONE_RANGE.end());
            }
            CoatsMessage::Shadows(on) => coats.zenithal.shadows = on,
            CoatsMessage::Contrast(on) => coats.chop.contrast = on.then_some(coats.contrast),
            CoatsMessage::BaseCoat => {
                let colour = self.painting.colour;
                self.apply_coat("base coat", move |volume| volume.base_coat(colour));
            }
            CoatsMessage::Zenithal => {
                let zenithal = Zenithal { light: coats.light(), ..coats.zenithal };
                self.apply_coat("zenithal", move |volume| volume.zenithal(&zenithal));
            }
            CoatsMessage::Varnish(finish) => coats.varnish = finish,
            CoatsMessage::VarnishCoat => {
                let finish = coats.varnish;
                self.apply_coat("varnish", move |volume| volume.varnish_coat(finish));
            }
            CoatsMessage::Spongechop => {
                // A new speckle each time it is pressed, so two spongechops
                // are not the same.
                coats.chop.seed = coats.chop.seed.wrapping_add(1);
                let chop =
                    Spongechop { light: coats.light(), sponge: self.touch.sponge, ..coats.chop };
                self.apply_coat("spongechop", move |volume| volume.spongechop(&chop));
            }
        }
        // The sponge prefers what faces the same light.
        self.touch.sponge.light = self.painting.coats.light();
    }

    /// Lay a coat on the active body, as one undo entry.
    fn apply_coat(&mut self, name: &str, coat: impl FnOnce(&mut hani_core::Volume) -> bool) {
        let body = self.doc.active();
        let layers = self.doc.active_volume().layers();
        let before = super::layers::Before::Added(layers.metas(), layers.active_id());
        if !coat(self.doc.active_volume_mut()) {
            self.status = format!(
                "no room for the {name}: a body holds {} layers at most",
                hani_core::paint::layers::MAX_LAYERS
            );
            return;
        }
        // The layers the coat added, which is all undoing it takes out.
        self.push_layers_entry(body, before, None);
        self.remesh_dirty();
        self.status = format!("laid a {name}");
    }

    /// A coat's colour: a swatch that takes the colour in hand.
    fn coat_swatch(&self, colour: [u8; 3], which: CoatColour) -> Element<'_, Message> {
        crate::ui::tip(
            button(
                container(text(""))
                    .width(18)
                    .height(18)
                    .style(theme::swatch(iced::Color::from_rgb8(colour[0], colour[1], colour[2]))),
            )
            .padding(2)
            .style(theme::tool_button)
            .on_press(Message::Coats(CoatsMessage::Take(which))),
            "Take the colour in hand",
        )
    }

    /// The COATS block.
    pub(super) fn coats_block(&self) -> Element<'_, Message> {
        let coats = &self.painting.coats;
        // The heading's hover line is in `app/help.rs`.
        let heading = crate::ui::tip(
            crate::ui::heading("COATS", coats.open, Message::Coats(CoatsMessage::Opened), None),
            super::help::COATS_TIP,
        );
        if coats.open {
            column![heading, self.coats_panel()].spacing(theme::S2).into()
        } else {
            heading
        }
    }

    fn coats_panel(&self) -> Element<'_, Message> {
        let coats = &self.painting.coats;
        let send = |message: CoatsMessage| Message::Coats(message);
        let label =
            |words: &'static str| text(words).size(theme::LABEL_SIZE).color(theme::TEXT_DIM);
        // What each coat lays, on hover: `app/tips.rs`.
        let apply = |message: CoatsMessage| {
            let what = match message {
                CoatsMessage::BaseCoat => "Lay the base coat over the whole body, on a new layer",
                CoatsMessage::Zenithal => "Prime from the light: light where it faces the light",
                CoatsMessage::Spongechop => {
                    "Sponge a dark, a middle and a light by height and light"
                }
                CoatsMessage::VarnishCoat => "Varnish the whole body with this finish",
                _ => "Lay this coat on a new layer",
            };
            crate::ui::tip(
                button(text("Apply").size(theme::LABEL_SIZE))
                    .padding(theme::PAD_CHOICE)
                    .style(theme::tool_button)
                    .on_press(send(message)),
                what,
            )
        };
        let word = |words: &'static str, what: &'static str, message: CoatsMessage| {
            crate::ui::tip(
                button(
                    text(words)
                        .size(theme::LABEL_SIZE)
                        .align_x(Alignment::Center)
                        .width(Length::Fill),
                )
                .width(Length::Fill)
                .padding(theme::PAD_CHOICE)
                .style(theme::tool_button)
                .on_press(send(message)),
                what,
            )
        };

        let light = column![
            row![
                word("From above", "Put the light straight overhead", CoatsMessage::Above),
                word(
                    "From the view",
                    "Put the light where you are looking from",
                    CoatsMessage::FromView
                )
            ]
            .spacing(theme::S1),
            crate::ui::number(
                &self.drafts,
                "coats.azimuth",
                "Light round",
                coats.azimuth,
                -180.0..=180.0,
                |v| { Message::Coats(CoatsMessage::Azimuth(v)) }
            )
            .step(1.0)
            .shown(format!("{:.0}°", coats.azimuth)),
            crate::ui::number(
                &self.drafts,
                "coats.height",
                "Light up",
                coats.height,
                -90.0..=90.0,
                |v| { Message::Coats(CoatsMessage::Height(v)) }
            )
            .step(1.0)
            .shown(format!("{:.0}°", coats.height)),
        ]
        .spacing(theme::S2);

        let base = row![label("Base coat").width(Length::Fill), apply(CoatsMessage::BaseCoat)]
            .spacing(theme::S2)
            .align_y(Alignment::Center);

        let zenithal = &coats.zenithal;
        let mut stops = row![self.coat_swatch(zenithal.dark, CoatColour::Dark)].spacing(theme::S1);
        if let Some(middle) = zenithal.middle {
            stops = stops.push(self.coat_swatch(middle, CoatColour::Middle));
        }
        let zenithal_block = column![
            row![label("Zenithal").width(Length::Fill), apply(CoatsMessage::Zenithal)]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
            row![
                stops.push(self.coat_swatch(zenithal.bright, CoatColour::Bright)),
                checkbox(zenithal.middle.is_some())
                    .label("Middle")
                    .on_toggle(|on| Message::Coats(CoatsMessage::Middle(on)))
                    .text_size(theme::LABEL_SIZE),
                checkbox(zenithal.shadows)
                    .label("Shadows")
                    .on_toggle(|on| Message::Coats(CoatsMessage::Shadows(on)))
                    .text_size(theme::LABEL_SIZE),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
            crate::ui::number(
                &self.drafts,
                "coats.cone",
                "Spray cone",
                zenithal.cone,
                CONE_RANGE,
                |v| { Message::Coats(CoatsMessage::Cone(v)) }
            )
            .step(1.0)
            .shown(format!("{:.0}°", zenithal.cone)),
        ]
        .spacing(theme::S2);

        let chop = &coats.chop;
        let mut colours = row![
            self.coat_swatch(chop.dark, CoatColour::ChopDark),
            self.coat_swatch(chop.middle, CoatColour::ChopMiddle),
            self.coat_swatch(chop.bright, CoatColour::ChopBright),
        ]
        .spacing(theme::S1)
        .align_y(Alignment::Center);
        if let Some(contrast) = chop.contrast {
            colours = colours.push(self.coat_swatch(contrast, CoatColour::Contrast));
        }
        let chop_block = column![
            row![label("Spongechop").width(Length::Fill), apply(CoatsMessage::Spongechop)]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
            row![
                colours,
                checkbox(chop.contrast.is_some())
                    .label("Contrast")
                    .on_toggle(|on| Message::Coats(CoatsMessage::Contrast(on)))
                    .text_size(theme::LABEL_SIZE),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
        ]
        .spacing(theme::S2);

        let varnish = column![
            row![label("Varnish").width(Length::Fill), apply(CoatsMessage::VarnishCoat)]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
            crate::ui::segmented_tips(
                [
                    hani_core::paint::Finish::Matte,
                    hani_core::paint::Finish::Satin,
                    hani_core::paint::Finish::Gloss,
                ]
                .map(|finish| (
                    finish,
                    finish.label(),
                    super::tips::finish(finish)
                )),
                coats.varnish,
                |finish| Message::Coats(CoatsMessage::Varnish(finish)),
            ),
        ]
        .spacing(theme::S2);

        column![light, base, zenithal_block, chop_block, varnish].spacing(theme::S3).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn coats(app: &mut Hani, message: CoatsMessage) {
        update(app, Message::Coats(message));
    }

    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Paint));
        update(&mut app, Message::WorkspaceChosen(crate::message::Workspace::Paint));
        app
    }

    fn names(app: &Hani) -> Vec<String> {
        app.doc.active_volume().layers().metas().into_iter().map(|meta| meta.name).collect()
    }

    /// Each coat is its layers, and one undo.
    #[test]
    fn each_coat_is_its_layers_and_one_undo() {
        let mut app = app();
        coats(&mut app, CoatsMessage::BaseCoat);
        assert_eq!(names(&app), ["Base coat", "Paint"]);
        coats(&mut app, CoatsMessage::Zenithal);
        assert_eq!(names(&app)[1], "Zenithal");
        coats(&mut app, CoatsMessage::Contrast(true));
        coats(&mut app, CoatsMessage::Spongechop);
        assert_eq!(names(&app).len(), 7, "{:?}", names(&app));
        update(&mut app, Message::Undo);
        assert_eq!(names(&app).len(), 3, "the spongechop was not one undo");
        update(&mut app, Message::Undo);
        update(&mut app, Message::Undo);
        assert_eq!(names(&app), ["Paint"]);
    }

    /// A zenithal from above is seen light on top and dark underneath:
    /// what is drawn is the stack put together.
    #[test]
    fn a_zenithal_is_seen_light_above_and_dark_below() {
        let mut app = app();
        coats(&mut app, CoatsMessage::Zenithal);
        let volume = app.doc.active_volume();
        let (low, high) = volume.world_bounds().expect("a body");
        let middle = (low + high) * 0.5;
        let voxel_size = volume.voxel_size();
        let on_surface = |from: glam::Vec3, way: glam::Vec3| {
            let mut at = from;
            while volume.sample_world(at) < 0.0 {
                at += way * voxel_size * 0.5;
            }
            (at / voxel_size).round().as_ivec3()
        };
        let seen = |voxel: glam::IVec3| {
            let mut best = [0u8; 4];
            for dy in -2..=2 {
                let rgba = volume.display_paint().at(voxel + glam::IVec3::new(0, dy, 0));
                if rgba[3] > best[3] {
                    best = rgba;
                }
            }
            best
        };
        let top = seen(on_surface(middle, glam::Vec3::Y));
        let bottom = seen(on_surface(middle, -glam::Vec3::Y));
        assert!(top[0] > 200 && bottom[0] < 60, "top {top:?} and bottom {bottom:?}");
    }

    /// A varnish coat is a Varnish layer and one undo, and a stroke on a
    /// Varnish layer lays the finish chosen and not the colour in hand.
    #[test]
    fn a_varnish_is_a_layer_and_strokes_on_it_lay_the_finish() {
        use hani_core::paint::Finish;
        let mut app = app();
        coats(&mut app, CoatsMessage::Varnish(Finish::Gloss));
        coats(&mut app, CoatsMessage::VarnishCoat);
        assert_eq!(names(&app), ["Paint", "Varnish: Gloss"]);
        let pigment = app.pigment(hani_core::BrushDirection::Add);
        assert_eq!(pigment.colour, [Finish::Gloss.code(), 0, 0]);
        update(&mut app, Message::Layers(crate::message::LayersMessage::Chosen(0)));
        assert_eq!(app.pigment(hani_core::BrushDirection::Add).colour, app.painting.colour);
        update(&mut app, Message::Undo);
        assert_eq!(names(&app), ["Paint"]);
    }

    /// A coat's swatch takes the colour in hand; the light from the view
    /// is the way back to the camera, and the sponge takes the light.
    #[test]
    fn a_swatch_takes_the_colour_and_the_light_follows_the_view() {
        let mut app = app();
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour([1, 2, 3])));
        coats(&mut app, CoatsMessage::Take(CoatColour::Bright));
        assert_eq!(app.painting.coats.zenithal.bright, [1, 2, 3]);
        coats(&mut app, CoatsMessage::Take(CoatColour::Middle));
        assert_eq!(app.painting.coats.zenithal.middle, Some([1, 2, 3]));
        coats(&mut app, CoatsMessage::Middle(false));
        assert_eq!(app.painting.coats.zenithal.middle, None);
        coats(&mut app, CoatsMessage::FromView);
        let toward = (app.camera.eye() - app.camera.target).normalize();
        assert!((app.painting.coats.light() - toward).length() < 1e-3);
        assert!((app.touch.sponge.light - toward).length() < 1e-3);
        drop(app.coats_block());
    }
}
