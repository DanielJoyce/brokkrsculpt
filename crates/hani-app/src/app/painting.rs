// SPDX-License-Identifier: AGPL-3.0-only

//! The paint brush, painting: a colour under the brush, an opacity, and a
//! way of laying it.
//!
//! `docs/STUDIO-PLAN.md` section 15 is the design, and
//! [`hani_core::paint::stroke`] is the brush. This is what the panel
//! offers of it and what a drag of the pointer does with it.
//!
//! # What a drag does
//!
//! A press starts a stroke, and the stroke lays one coat however many times
//! it crosses itself. Ctrl or alt, which unpaints a slot, takes paint off
//! here too, by the brush's own opacity and with its own soft edge. With
//! the eyedropper armed, a press takes the colour under the pointer and
//! nothing is painted until the next press.

use glam::Vec3;
use hani_core::paint::field::Rgba;
use hani_core::paint::stroke::{Catch, Lay, Mode, PaintStroke, Pigment, laid};
use hani_core::paint::touch::Coat;
use hani_core::{Brush, BrushDirection, Stamp};
use iced::widget::{button, checkbox, column, container, row, text};
use iced::{Alignment, Element, Length};

use super::Hani;
use crate::message::{Message, PaintMode, PaintTouch, PaintingMessage};
use crate::theme;

/// How little and how much of itself a full touch of the brush may lay.
pub(super) const OPACITY_RANGE: std::ops::RangeInclusive<f32> = 0.05..=1.0;

/// How unlike the paint it began on a fill runs over, until it is told
/// otherwise: an eighth of the way from black to white. Enough to take a
/// shaded panel as one panel, and not enough to run from one colour onto
/// another.
const USUAL_LIKENESS: u8 = 32;

/// The colour the brush starts with: a red a miniature painter would know,
/// and not a grey that could be taken for the clay.
const FIRST_COLOUR: [u8; 3] = [178, 34, 34];

/// The painting brush's own state, in one field of `Hani`. The brush's,
/// and not the document's: nothing here is saved with a file.
#[derive(Debug)]
pub(super) struct PaintingState {
    /// sRGB.
    pub colour: [u8; 3],
    pub opacity: f32,
    /// The way of painting last chosen. What the panel shows lit, and what
    /// the brush is, is [`Hani::mode_in_hand`], which follows the layer.
    pub mode: PaintMode,
    /// Whether the eyedropper is armed: the next press on the model takes
    /// the colour there.
    pub picking: bool,
    /// Whether the drag in progress began with the eyedropper, and so
    /// paints nothing.
    held_off: bool,
    /// How unlike the paint it began on a fill will still run over, in
    /// levels of 255.
    pub fill_like: u8,
    stroke: PaintStroke,
    /// The layers block's gesture under way. See `app/layers.rs`.
    pub layers: super::layers::LayersHeld,
    /// The racks of paints. See `app/rack.rs`.
    pub rack: super::rack::RackState,
    /// The airbrush. See `app/airbrush.rs`.
    pub airbrush: super::airbrush::AirbrushState,
    /// The one-click coats. See `app/coats.rs`.
    pub coats: super::coats::CoatsState,
    /// The Decal touch. See `app/decal.rs`.
    pub decal: super::decal::DecalState,
    /// The paint designer, when it is open. See `app/designer.rs`.
    pub designer: Option<super::designer::Designer>,
    /// How strongly washes and contrasts pool. See `app/pooling.rs`.
    pub pooling: f32,
    /// How far round a recess they pool: `app/pooling.rs`.
    pub pooling_reach: f32,
}

impl Default for PaintingState {
    fn default() -> Self {
        Self {
            colour: FIRST_COLOUR,
            opacity: 1.0,
            mode: PaintMode::default(),
            picking: false,
            held_off: false,
            fill_like: USUAL_LIKENESS,
            stroke: PaintStroke::new(),
            layers: Default::default(),
            rack: Default::default(),
            airbrush: Default::default(),
            coats: Default::default(),
            decal: Default::default(),
            designer: None,
            pooling: super::pooling::kept(),
            pooling_reach: super::pooling::kept_reach(),
        }
    }
}

impl PaintingState {
    /// What is on the brush, for a stroke in `direction`: the inverted
    /// direction takes paint off, whatever the mode.
    pub(super) fn pigment(&self, direction: BrushDirection) -> Pigment {
        let mode = match (direction, self.mode) {
            (BrushDirection::Subtract, _) | (_, PaintMode::Erase) => Mode::Erase,
            // A glaze is laid as paint is, on a layer whose blend is Glaze,
            // which tints whatever is under it: `Hani::suit_the_layer_to_the_brush`.
            (_, PaintMode::Paint | PaintMode::Glaze) => Mode::Paint,
        };
        Pigment { colour: self.colour, opacity: self.opacity, mode }
    }
}

fn swatch_colour(colour: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(colour[0], colour[1], colour[2])
}

impl Hani {
    /// A document painted by filament slot, from before slot painting was
    /// taken out, laid into colour paint as the colours its slots showed.
    /// See `hani_core::paint::old_slots`.
    pub(super) fn lay_old_slots(&mut self) {
        let filaments = self.filament_colours();
        if self.doc.lay_slots_into_paint(&filaments) {
            self.status = format!("{} (its filament paint is now colour paint)", self.status);
        }
    }

    pub(super) fn painting_changed(&mut self, message: PaintingMessage) {
        match message {
            PaintingMessage::Colour(colour) => self.painting.colour = colour,
            PaintingMessage::Channel(channel, level) => {
                if let Some(slot) = self.painting.colour.get_mut(channel) {
                    *slot = level;
                }
            }
            PaintingMessage::Opacity(opacity) => {
                self.painting.opacity = opacity.clamp(*OPACITY_RANGE.start(), *OPACITY_RANGE.end());
            }
            PaintingMessage::Mode(mode) => {
                // Paint or Glaze chosen with a wash's or a contrast's touch
                // in hand is a brush again, as a plain paint from the rack
                // is (`app/rack.rs`): the touch would otherwise want its
                // own layer, and the button chosen would not light.
                if mode != PaintMode::Erase
                    && matches!(self.touch.kind, PaintTouch::Wash | PaintTouch::Contrast)
                {
                    self.touch.kind = PaintTouch::Loaded;
                }
                // The layer follows the brush: see `app/layers.rs`.
                self.painting.mode = mode;
                self.suit_the_layer_to_the_brush(None, None);
            }
            PaintingMessage::FillLike(levels) => self.painting.fill_like = levels,
            PaintingMessage::PickArmed => self.painting.picking = !self.painting.picking,
            PaintingMessage::Coated => self.coat_the_body_in_colour(),
            // See `app/pooling.rs`.
            PaintingMessage::Pooling(pooling) => self.pool(pooling),
            PaintingMessage::PoolingKept => self.pooling_kept(),
            PaintingMessage::PoolingReach(reach) => self.pool_reach(reach),
        }
    }

    /// What the eyedropper does with a press at `point`, a world position
    /// on the active body's surface, and whether the drag is spoken for:
    /// `true` when nothing is to be painted by this pointer event.
    ///
    /// Asked before the stroke is armed, so that taking a colour is not an
    /// entry in the undo history.
    pub(super) fn eyedropper_takes(&mut self, point: Vec3, start: bool) -> bool {
        if start {
            self.painting.held_off = false;
        }
        if self.painting.picking {
            self.painting.picking = false;
            self.painting.held_off = true;
            self.pick_colour(point);
        }
        self.painting.held_off
    }

    /// The way of painting the brush in hand is, and the panel shows lit:
    /// Erase while erasing, and otherwise Glaze on a layer whose blend is
    /// Glaze and Paint on any other, so that it follows a layer chosen by
    /// hand in the layers block.
    pub(super) fn mode_in_hand(&self) -> PaintMode {
        if self.painting.mode == PaintMode::Erase {
            return PaintMode::Erase;
        }
        let layers = self.doc.active_volume().layers();
        match layers.metas().get(layers.active()).map(|meta| meta.blend) {
            Some(hani_core::paint::layers::Blend::Glaze) => PaintMode::Glaze,
            _ => PaintMode::Paint,
        }
    }

    /// The blend the brush in hand wants its layer to have, or nothing
    /// while erasing, which takes paint off whatever layer is active. A
    /// wash's and a contrast's touch want their own; otherwise a glaze
    /// tints and wants a Glaze layer, and a paint covers and wants a
    /// Normal one.
    pub(super) fn blend_in_hand(&self) -> Option<hani_core::paint::layers::Blend> {
        use hani_core::paint::layers::Blend;
        match (self.painting.mode, self.touch.kind) {
            (PaintMode::Erase, _) => None,
            (_, PaintTouch::Wash) => Some(Blend::Wash),
            (_, PaintTouch::Contrast) => Some(Blend::Contrast),
            (PaintMode::Glaze, _) => Some(Blend::Glaze),
            (PaintMode::Paint, _) => Some(Blend::Normal),
        }
    }

    /// A touch chosen in the panel, and the layer suited to it: a wash or
    /// a contrast to a layer of its own blend, and any other touch back to
    /// the layer the way of painting in hand wants. Asked for by the
    /// maintainer on 2026-11-27; before, an ordinary touch after a contrast
    /// went on painting on the contrast's layer, as a film.
    pub(super) fn touch_chosen(&mut self, touch: PaintTouch) {
        self.touch.kind = touch;
        self.painting.mode = self.mode_in_hand();
        self.suit_the_layer_to_the_brush(None, None);
    }

    /// Take the colour of the paint at `point`.
    fn pick_colour(&mut self, point: Vec3) {
        let volume = self.doc.active_volume();
        let voxel = (point / volume.voxel_size()).round().as_ivec3();
        // What is seen there, every layer put together.
        let paint = volume.display_paint().at(voxel);
        if paint[3] == 0 {
            self.status = "nothing is painted there to take a colour from".to_string();
            return;
        }
        self.painting.colour = [paint[0], paint[1], paint[2]];
        self.status =
            format!("took the colour {} {} {} from the model", paint[0], paint[1], paint[2]);
    }

    /// Lay one stamp of the stroke in colour. `coat` is the stroke's own
    /// count of itself, which the slots' brush keeps: its seed, where it
    /// began and how much is left on a dry brush. `first` is whether this
    /// is the first stamp of the stroke.
    pub(super) fn lay_colour(
        &mut self,
        brush: &Brush,
        stamp: &Stamp,
        coat: Coat,
        direction: BrushDirection,
        first: bool,
    ) {
        if first {
            self.painting.stroke.begin();
        }
        let catch = match self.touch.kind {
            // The airbrush lays no stamps: see `app/airbrush.rs`.
            PaintTouch::Loaded | PaintTouch::Fill | PaintTouch::Airbrush | PaintTouch::Decal => {
                Catch::All
            }
            // A wash and a contrast go on where the brush goes: their layer's
            // blend pools them, by the surface's occlusion, when the stack is
            // put together. Pooled here too, by the local crease, they pooled
            // twice and in the wrong place. See `hani_core::paint::occlusion`.
            PaintTouch::Wash | PaintTouch::Contrast => Catch::All,
            PaintTouch::Drybrush => Catch::Raised(self.touch.dry),
            PaintTouch::Sponge => Catch::Specks(self.touch.sponge),
        };
        let lay = Lay {
            pigment: self.pigment(direction),
            catch,
            seed: coat.seed,
            origin: coat.origin,
            load: coat.load,
            crease_gain: coat.crease_gain,
        };
        let symmetry = self.framed_symmetry();
        self.painting.stroke.stamp_symmetric(
            self.doc.active_volume_mut(),
            brush,
            stamp,
            &lay,
            symmetry,
            super::MIRROR_CENTRE,
        );
    }

    /// Pour the colour in hand onto the model at `point`, a world position
    /// on its surface, to run until the model's own lines stop it or the
    /// paint under it is no longer like the paint where it began.
    ///
    /// Called for every pointer event of a drag with the Fill touch
    /// chosen, as a slot's fill is, so that dragging across several panels
    /// fills each as the pointer reaches it. The drag is one undo entry.
    pub(super) fn pour_colour(&mut self, point: Vec3, direction: BrushDirection) {
        let lines = self.fill_lines(0);
        let pigment = self.pigment(direction);
        let like = self.painting.fill_like;
        let filled = self.doc.active_volume_mut().fill_colour(point, pigment, like, lines);
        self.say_what_was_filled(filled);
    }

    /// The one setting a fill of colour has that a fill of a slot has not,
    /// added to the fill's block of the panel.
    pub(super) fn fill_likeness<'a>(
        &'a self,
        block: iced::widget::Column<'a, Message>,
    ) -> iced::widget::Column<'a, Message> {
        let like = self.painting.fill_like;
        block.push(
            crate::ui::number(
                &self.drafts,
                "paint.fill_like",
                "Runs over paint within, levels",
                f32::from(like),
                0.0..=255.0,
                |levels| Message::Painting(PaintingMessage::FillLike(levels.round() as u8)),
            )
            .step(1.0)
            .shown(like.to_string()),
        )
    }

    /// Lay the colour in hand over the whole of the active body, by the
    /// brush's opacity and in its mode, wherever the mask leaves it free
    /// and by as much as it does. One undo entry.
    fn coat_the_body_in_colour(&mut self) {
        let Some((low, high)) = self.doc.active_volume().world_bounds() else {
            return;
        };
        if self.layer_refuses_paint() {
            return;
        }
        let pigment = self.pigment(BrushDirection::Add);
        self.arm_recorder();
        let volume = self.doc.active_volume_mut();
        let (lo, hi) = volume.voxel_bounds(low, high);
        volume.edit_paint(lo, hi, |_, _, held: Rgba, protection| {
            let free = 1.0 - f32::from(protection) / f32::from(hani_core::PROTECTED);
            laid(held, pigment, free)
        });
        self.finish_stroke();
        self.remesh_dirty();
        self.status = "coated the body".to_string();
    }

    /// What the paint brush is offered: the two ways of painting, and the
    /// one chosen.
    ///
    /// `editing` is whether there is room for everything. The panel has it
    /// and the pop-up menu does not.
    pub(super) fn painting_picker(&self, editing: bool) -> Element<'_, Message> {
        let chosen = self.colour_picker(editing);
        // The rack in hand first, with room: the library is the rack. See
        // `app/rack.rs`.
        if editing {
            return column![self.rack_panel(), chosen].spacing(theme::S3).into();
        }
        chosen
    }

    /// The colour brush: what it does, how it touches, the colour, and how
    /// much of it.
    fn colour_picker(&self, editing: bool) -> Element<'_, Message> {
        let reading = |label: String| text(label).size(theme::LABEL_SIZE).color(theme::TEXT_DIM);
        let painting = &self.painting;
        let colour = painting.colour;

        let modes =
            PaintMode::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, mode| {
                assembled.push(crate::ui::tip(
                    button(text(mode.label()).size(theme::LABEL_SIZE))
                        .width(Length::Fill)
                        .style(if mode == self.mode_in_hand() {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::Painting(PaintingMessage::Mode(mode))),
                    super::tips::paint_mode(mode),
                ))
            });

        // The button is as wide as the row will let it be and no wider: at
        // its own width, beside three three-digit numbers, its label broke
        // onto a second line. Seen on a screen.
        let in_hand = row![
            crate::ui::tip(
                button(
                    container(text(""))
                        .width(28)
                        .height(18)
                        .style(theme::swatch(swatch_colour(colour))),
                )
                .padding(2)
                .style(theme::tool_button)
                .on_press(Message::Designer(crate::message::DesignerMessage::New)),
                "New paint from this colour",
            ),
            reading(format!("{:3} {:3} {:3}", colour[0], colour[1], colour[2])),
            crate::ui::tip(
                button(text("Pick from model").size(theme::LABEL_SIZE))
                    .width(Length::Fill)
                    .style(if painting.picking {
                        theme::tool_button_active
                    } else {
                        theme::tool_button
                    })
                    .on_press(Message::Painting(PaintingMessage::PickArmed)),
                "Click the model to take the colour there",
            ),
        ]
        .spacing(theme::S2)
        .align_y(Alignment::Center);

        let mut picker = column![
            modes,
            self.touch_picker(editing),
            in_hand,
            crate::ui::number(
                &self.drafts,
                "paint.opacity",
                "Opacity",
                painting.opacity,
                OPACITY_RANGE,
                |opacity| Message::Painting(PaintingMessage::Opacity(opacity)),
            )
            .shown(format!("{:.2}", painting.opacity)),
        ]
        .spacing(theme::S2);
        if !editing {
            return picker.into();
        }

        // The colour's sliders, the filaments and the mixer are in the
        // paint designer (`app/designer.rs`), opened from the rack.
        picker = picker
            .push(
                checkbox(self.flat_paint)
                    .label("flat colour, no light")
                    .on_toggle(|_| Message::FlatPaintToggled)
                    .text_size(theme::LABEL_SIZE),
            )
            .push(crate::ui::tip(
                button(text("Coat the body").size(theme::LABEL_SIZE))
                    .style(theme::tool_button)
                    .on_press(Message::Painting(PaintingMessage::Coated)),
                "Paints the whole body in this colour, in this mode and touch",
            ));
        picker.into()
    }
}

/// The colour brush done the way a user does it: messages and pointer
/// events in, and the body's paint looked at afterwards.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};
    use glam::IVec3;
    use hani_core::BrushKind;
    use hani_core::paint::field::{BARE, PaintField};
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };
    const CENTRE: Vector = Vector { x: 400.0, y: 300.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn set(app: &mut Hani, message: PaintingMessage) {
        update(app, Message::Painting(message));
    }

    /// The application with the paint brush in hand, as it starts: in
    /// colour.
    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
    }

    fn press(app: &mut Hani, at: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
    }

    fn drag_to(app: &mut Hani, at: Vector) {
        update(app, Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }));
    }

    fn release(app: &mut Hani) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short drag across the middle of the model, there and back.
    fn stroke(app: &mut Hani) {
        press(app, CENTRE);
        for step in [20.0, 40.0, 20.0, 0.0] {
            drag_to(app, Vector::new(CENTRE.x + step, CENTRE.y));
        }
        release(app);
    }

    /// Every painted voxel of the active body.
    fn painted(app: &Hani) -> Vec<Rgba> {
        let volume = app.doc.active_volume();
        let mut paint = Vec::new();
        if volume.paint().is_empty() {
            return paint;
        }
        for coord in volume.brick_coords() {
            let origin = coord.origin();
            for index in 0..32 * 32 * 32 {
                let cell = origin + IVec3::new(index % 32, (index / 32) % 32, index / 1024);
                let rgba = volume.paint().at(cell);
                if rgba != BARE {
                    paint.push(rgba);
                }
            }
        }
        paint
    }

    fn most_covered(paint: &[Rgba]) -> u8 {
        paint.iter().map(|rgba| rgba[3]).max().unwrap_or(0)
    }

    /// A drag lays the colour in hand, with a soft edge, on the paint and
    /// on nothing else, and it is one step of the undo history.
    #[test]
    fn a_drag_lays_the_colour_in_hand_and_is_one_undo() {
        let mut app = app();
        set(&mut app, PaintingMessage::Colour([30, 90, 200]));
        let field_before = app.doc.active_volume().stats().dense_bricks;
        assert!(painted(&app).is_empty());

        stroke(&mut app);
        let paint = painted(&app);
        assert!(paint.len() > 500, "the drag painted {} voxels", paint.len());
        assert!(paint.iter().all(|rgba| rgba[..3] == [30, 90, 200]), "another colour was laid");
        let soft = paint.iter().filter(|rgba| rgba[3] < 200).count();
        assert!(most_covered(&paint) == 255 && soft > 50, "no soft edge: {soft} voxels of it");
        assert!(app.unsaved, "a stroke of paint did not mark the document unsaved");
        let stats = app.doc.active_volume().stats();
        assert_eq!(stats.dense_bricks, field_before, "paint changed the field");
        assert_eq!(stats.colour_bricks, 0, "colour was laid as a filament slot");
        assert!(app.perf.dirty_bricks > 0, "nothing was made again to be drawn");

        update(&mut app, Message::Undo);
        assert!(painted(&app).is_empty(), "undo left paint on the body");
        update(&mut app, Message::Redo);
        assert_eq!(painted(&app), paint, "redo did not put the same paint back");
        update(&mut app, Message::Undo);
        assert!(!app.history.can_undo(), "one drag was more than one step");
    }

    /// A stroke that goes over itself lays one coat, and a second stroke
    /// lays a second.
    #[test]
    fn a_stroke_is_one_coat_and_a_second_stroke_a_second() {
        let mut app = app();
        set(&mut app, PaintingMessage::Opacity(0.5));
        stroke(&mut app);
        let once = most_covered(&painted(&app));
        assert_eq!(once, 128, "half a coat, there and back in one stroke, is {once}");
        stroke(&mut app);
        let twice = most_covered(&painted(&app));
        assert_eq!(twice, 192, "a second half coat over the first is {twice}");
    }

    /// Ctrl takes paint off, as it unpaints a slot, and so does the Erase
    /// mode with nothing held. Neither leaves another colour behind.
    #[test]
    fn ctrl_and_the_erase_mode_take_paint_off() {
        for with_ctrl in [true, false] {
            let mut app = app();
            set(&mut app, PaintingMessage::Coated);
            let coated = painted(&app);
            assert!(coated.len() > 10_000 && coated.iter().all(|rgba| rgba[3] == 255));

            if with_ctrl {
                app.control = true;
            } else {
                set(&mut app, PaintingMessage::Mode(PaintMode::Erase));
            }
            stroke(&mut app);
            let after = painted(&app);
            // Thinned under the brush's soft edge, and all but gone in the
            // middle of it.
            let thinned = after.iter().filter(|rgba| rgba[3] < 255).count();
            let least = after.iter().map(|rgba| rgba[3]).min().unwrap_or(0);
            assert!(after.len() <= coated.len() && thinned > 500, "{thinned} voxels thinned");
            assert!(least < 30, "the middle of the eraser left {least} of 255");
            assert!(after.iter().all(|rgba| rgba[..3] == coated[0][..3]), "erasing laid a colour");
        }
    }

    /// The eyedropper takes the colour under the pointer, paints nothing
    /// with the press that took it, is not a step of the history, and is
    /// put down again once it has been used.
    #[test]
    fn the_eyedropper_takes_a_colour_and_paints_nothing() {
        let mut app = app();
        set(&mut app, PaintingMessage::Colour([200, 160, 20]));
        set(&mut app, PaintingMessage::Coated);
        set(&mut app, PaintingMessage::Colour([10, 10, 10]));
        let before = painted(&app);
        let history = app.history.stats();

        set(&mut app, PaintingMessage::PickArmed);
        assert!(app.painting.picking);
        stroke(&mut app);
        assert_eq!(app.painting.colour, [200, 160, 20], "the colour was not taken");
        assert!(!app.painting.picking, "the eyedropper is still armed");
        assert_eq!(painted(&app), before, "the press that took a colour painted");
        assert_eq!(app.history.stats(), history, "taking a colour is in the undo history");

        // The next press is a stroke again. An eraser's, so that it shows:
        // the colour taken, over a full coat of itself, would change
        // nothing and be no step at all.
        set(&mut app, PaintingMessage::Mode(PaintMode::Erase));
        stroke(&mut app);
        assert_ne!(app.history.stats(), history, "the press after was held off too");
        assert_ne!(painted(&app), before);

        // On bare clay there is nothing to take, and the brush keeps what
        // it has. Armed twice is put down.
        let mut app = self::app();
        set(&mut app, PaintingMessage::PickArmed);
        stroke(&mut app);
        assert_eq!(app.painting.colour, PaintingState::default().colour);
        assert!(app.status.contains("nothing is painted there"), "{}", app.status);
        set(&mut app, PaintingMessage::PickArmed);
        set(&mut app, PaintingMessage::PickArmed);
        assert!(!app.painting.picking);
    }

    /// Coat the body lays the colour by the brush's opacity, as much as the
    /// mask leaves free, in one step.
    #[test]
    fn coat_the_body_goes_by_the_opacity_and_the_mask() {
        let mut app = app();
        set(&mut app, PaintingMessage::Opacity(0.5));
        // Protect the half of the body where x is negative.
        let volume = app.doc.active_volume_mut();
        let (low, high) = volume.world_bounds().expect("a body");
        let (lo, hi) = volume.voxel_bounds(low, high);
        volume.edit_mask(lo, hi, |_, at, _| if at.x < 0.0 { hani_core::PROTECTED } else { 0 });
        set(&mut app, PaintingMessage::Coated);

        let volume = app.doc.active_volume();
        let (mut free, mut held) = (0, 0);
        for coord in volume.brick_coords() {
            let origin = coord.origin();
            for index in 0..32 * 32 * 32 {
                let cell = origin + IVec3::new(index % 32, (index / 32) % 32, index / 1024);
                if !PaintField::paintable(volume.sample_voxel(cell)) {
                    continue;
                }
                let rgba = volume.paint().at(cell);
                if cell.x < 0 {
                    assert_eq!(rgba, BARE, "paint under the mask at {cell}");
                    held += 1;
                } else {
                    assert_eq!(rgba[3], 128, "half a coat at {cell} is {rgba:?}");
                    free += 1;
                }
            }
        }
        assert!(free > 10_000 && held > 10_000, "{free} free and {held} protected");
        update(&mut app, Message::Undo);
        assert!(painted(&app).is_empty(), "the coat was more than one step");
    }

    /// How the brush touches reaches the stroke: a sponge leaves gaps a
    /// loaded brush does not, and a wash on a ball, which has no creases,
    /// leaves what a wash leaves on a flat.
    #[test]
    fn the_touch_chosen_is_the_touch_laid() {
        let laid_with = |touch: PaintTouch| {
            let mut app = app();
            update(&mut app, Message::PaintTouchChosen(touch));
            stroke(&mut app);
            painted(&app)
        };
        let loaded = laid_with(PaintTouch::Loaded);
        let sponged = laid_with(PaintTouch::Sponge);
        assert!(
            sponged.len() * 4 < loaded.len() * 3 && sponged.len() * 10 > loaded.len(),
            "a sponge painted {} voxels and a loaded brush {}",
            sponged.len(),
            loaded.len()
        );
        // A wash and a contrast are laid as the brush goes, on a layer of
        // their own blend, which does the pooling.
        for (touch, blend) in [
            (PaintTouch::Wash, hani_core::paint::layers::Blend::Wash),
            (PaintTouch::Contrast, hani_core::paint::layers::Blend::Contrast),
        ] {
            let mut app = app();
            update(&mut app, Message::PaintTouchChosen(touch));
            let layers = app.doc.active_volume().layers();
            assert_eq!(layers.metas()[layers.active()].blend, blend, "{touch:?}");
            stroke(&mut app);
            assert!(!painted(&app).is_empty(), "{touch:?} laid nothing");
        }
    }

    /// A click with the Fill touch pours the colour over the paint it lands
    /// on and paint like it, and stops at paint that is not; told that
    /// anything is alike, it takes the body. Each click is one undo.
    #[test]
    fn a_fill_pours_the_colour_over_paint_like_where_it_landed() {
        let click = |app: &mut Hani| {
            press(app, CENTRE);
            release(app);
        };
        // On bare clay, on a ball with no lines to stop it: as much of it
        // as one click may flood, and the status line says that is why it
        // stopped.
        let mut app = app();
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Fill));
        set(&mut app, PaintingMessage::Colour([30, 90, 200]));
        click(&mut app);
        let all = painted(&app);
        assert!(all.len() > 100_000, "a fill of a bare ball painted {} voxels", all.len());
        assert!(all.iter().all(|rgba| *rgba == [30, 90, 200, 255]));
        assert!(app.status.contains("the fill stopped at"), "{}", app.status);
        update(&mut app, Message::Undo);
        assert!(painted(&app).is_empty(), "a fill was more than one step");

        // On a stroke of blue across bare clay: the stroke, and not the
        // clay round it.
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Loaded));
        update(&mut app, Message::FalloffChanged(hani_core::FalloffCurve::Wide));
        stroke(&mut app);
        let blue = painted(&app).len();
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Fill));
        set(&mut app, PaintingMessage::Colour([200, 30, 20]));
        click(&mut app);
        let after = painted(&app);
        let red = after.iter().filter(|rgba| rgba[..3] == [200, 30, 20]).count();
        assert_eq!(after.len(), blue, "a fill begun on paint ran onto the clay");
        assert!(red > 300 && red < blue, "{red} of the stroke's {blue} voxels turned red");

        // Told that anything is alike: everything.
        set(&mut app, PaintingMessage::FillLike(255));
        set(&mut app, PaintingMessage::Colour([20, 160, 40]));
        click(&mut app);
        let green = painted(&app).iter().filter(|rgba| **rgba == [20, 160, 40, 255]).count();
        assert!(green > 100_000, "a fill over anything painted {green} voxels");
    }

    /// A document painted by filament slot, from before slot painting was
    /// taken out, comes in with its slots as colour paint in the machine's
    /// filament colours, and its slots let go.
    #[test]
    fn old_slot_paint_comes_in_as_colour() {
        let mut app = app();
        let volume = app.doc.active_volume_mut();
        let (low, high) = volume.world_bounds().expect("a body");
        let (lo, hi) = volume.voxel_bounds(low, high);
        volume.edit_colour(lo, hi, |_, _, _, _| 2);
        app.lay_old_slots();
        let volume = app.doc.active_volume();
        assert!(volume.colour().is_empty(), "the slots were kept");
        let second = app.palette.slots[1].swatch().into_rgba8();
        let laid = painted(&app);
        assert!(!laid.is_empty(), "nothing was laid");
        assert!(laid.iter().all(|rgba| rgba[..3] == second[..3] && rgba[3] == 255));
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("hani-colour-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    /// The colours of a PLY written by the application, read off its header
    /// and its vertex records: 27 bytes each, the last three the colour.
    fn ply_colours(bytes: &[u8]) -> Vec<[u8; 3]> {
        let end = b"end_header\n";
        let body =
            bytes.windows(end.len()).position(|w| w == end).expect("no end_header") + end.len();
        let header = std::str::from_utf8(&bytes[..body]).expect("the header is text");
        let vertices: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("element vertex "))
            .expect("no vertex element")
            .parse()
            .expect("a count");
        (0..vertices)
            .map(|index| {
                let at = body + index * 27 + 24;
                [bytes[at], bytes[at + 1], bytes[at + 2]]
            })
            .collect()
    }

    /// What was painted is in what is exported: a stroke in the colour of
    /// the machine's second filament, written as a PLY, as an OBJ with a
    /// picture, and as a 3MF.
    #[test]
    fn painted_colour_is_in_the_files_that_carry_colour() {
        use crate::message::ExportFormat;
        let directory = scratch("exports");
        let mut app = app();
        let swatch = app.palette.slots[1].swatch().into_rgba8();
        let colour = [swatch[0], swatch[1], swatch[2]];
        let clay = crate::app::paints::UNPAINTED;
        assert_ne!(colour, clay, "the fixture's filament is the clay's colour");
        set(&mut app, PaintingMessage::Colour(colour));
        // Wide, so that the middle of the stroke is all of the colour.
        update(&mut app, Message::FalloffChanged(hani_core::FalloffCurve::Wide));
        stroke(&mut app);

        // A colour for every vertex: the paint, the clay, and the stroke's
        // soft edge between them, which is neither.
        let path = directory.join("painted.ply");
        app.export(ExportFormat::Ply, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        let colours = ply_colours(&std::fs::read(&path).expect("the file should be there"));
        // The colour to within a few levels: all of it is only where every
        // voxel round a vertex took all the brush had.
        let near = |one: &[u8], other: [u8; 3]| {
            (0..3).all(|c| (i32::from(one[c]) - i32::from(other[c])).abs() <= 4)
        };
        let painted = colours.iter().filter(|c| near(&c[..], colour)).count();
        let bare = colours.iter().filter(|c| **c == clay).count();
        assert!(painted > 50, "only {painted} vertices of the PLY are the colour painted");
        assert!(bare > painted, "the unpainted ball is not clay: {bare} of {}", colours.len());
        assert!(painted + bare < colours.len(), "the stroke has no soft edge in the file");

        // A picture: texels of the colour, and more of clay.
        let path = directory.join("painted.obj");
        app.export(ExportFormat::Textured, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        let picture = image::open(directory.join("painted_1.png"))
            .expect("the picture beside the OBJ")
            .into_rgba8();
        let count =
            |wanted: [u8; 3]| picture.pixels().filter(|pixel| near(&pixel.0[..3], wanted)).count();
        assert!(count(colour) > 300, "only {} texels are the colour painted", count(colour));
        assert!(count(clay) > count(colour));

        // A filament for every triangle: the second, where the stroke is,
        // and no other anywhere.
        let path = directory.join("painted.3mf");
        app.export(ExportFormat::ThreeMf, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();
        // The 3MF's `paint_color` code for each of the first four slots.
        let triangles = |filament: usize| {
            let code = ["4", "8", "0C", "1C"][filament - 1];
            written.matches(&format!("paint_color=\"{code}\"")).count()
        };
        assert!(triangles(2) > 100, "only {} triangles print in the filament", triangles(2));
        for other in [1, 3, 4] {
            assert_eq!(triangles(other), 0, "filament {other} prints something");
        }

        // And a body nobody painted is exported as it always was.
        let mut plain = self::app();
        let path = directory.join("plain.3mf");
        plain.export(ExportFormat::ThreeMf, &path);
        let written =
            String::from_utf8_lossy(&std::fs::read(&path).expect("the file should be there"))
                .into_owned();
        assert!(!written.contains("paint_color"), "an unpainted body names a filament");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// What arrives from the panel is kept to what the brush can be, and
    /// the panel can be drawn for every state of it.
    #[test]
    fn the_panels_messages_are_kept_in_range_and_every_state_draws() {
        let mut app = app();
        set(&mut app, PaintingMessage::Opacity(7.0));
        assert_eq!(app.painting.opacity, 1.0);
        set(&mut app, PaintingMessage::Opacity(-1.0));
        assert_eq!(app.painting.opacity, *OPACITY_RANGE.start());
        set(&mut app, PaintingMessage::Colour([1, 2, 3]));
        set(&mut app, PaintingMessage::Channel(1, 200));
        set(&mut app, PaintingMessage::Channel(9, 77));
        assert_eq!(app.painting.colour, [1, 200, 3]);

        for mode in PaintMode::ALL {
            set(&mut app, PaintingMessage::Mode(mode));
            for touch in PaintTouch::ALL {
                update(&mut app, Message::PaintTouchChosen(touch));
                drop(app.painting_picker(true));
                drop(app.painting_picker(false));
            }
        }
        // The whole window, with the colour brush up.
        drop(app.view());
    }

    /// A wash laid in one stroke out and back beside itself, the way a
    /// painter goes back over a stroke, by pointer moves that come several
    /// to a frame: no voxel of what is seen is lighter after the return
    /// than before it. The maintainer saw a paler seam between the two
    /// passes, 2026-11-25. See `hani_core::paint::stroke`.
    #[test]
    fn going_back_beside_a_wash_never_lightens_it() {
        use crate::message::RackMessage;
        let mut app = app();
        update(&mut app, Message::WorkspaceChosen(crate::message::Workspace::Paint));
        set(&mut app, PaintingMessage::Colour([220, 220, 220]));
        set(&mut app, PaintingMessage::Coated);
        let wash = app
            .painting
            .rack
            .rack()
            .paints
            .iter()
            .position(|paint| paint.kind == crate::paint_index::Kind::Wash)
            .expect("the starter rack has a wash");
        update(&mut app, Message::Rack(RackMessage::Picked(wash)));
        let seen = |app: &Hani| {
            let volume = app.doc.active_volume();
            let mut seen = std::collections::HashMap::new();
            for coord in volume.brick_coords() {
                for index in 0..32 * 32 * 32 {
                    let cell =
                        coord.origin() + IVec3::new(index % 32, (index / 32) % 32, index / 1024);
                    let rgba = volume.display_paint().at(cell);
                    if rgba[3] != 0 {
                        seen.insert(cell, rgba);
                    }
                }
            }
            seen
        };
        press(&mut app, Vector::new(300.0, 300.0));
        for step in 1..=20 {
            drag_to(&mut app, Vector::new(300.0 + step as f32 * 10.0, 300.0));
            if step % 4 == 0 {
                update(&mut app, Message::Frame);
            }
        }
        update(&mut app, Message::Frame);
        let out = seen(&app);
        for step in 1..=40 {
            drag_to(&mut app, Vector::new(500.0 - step as f32 * 5.0, 315.0));
            if step % 3 == 0 {
                update(&mut app, Message::Frame);
            }
        }
        update(&mut app, Message::Frame);
        let back = seen(&app);
        release(&mut app);
        let lightness = |rgba: Rgba| u32::from(rgba[0]) + u32::from(rgba[1]) + u32::from(rgba[2]);
        let lighter = out
            .iter()
            .filter(|(cell, was)| {
                back.get(*cell).is_some_and(|now| lightness(*now) > lightness(**was))
            })
            .count();
        assert_eq!(lighter, 0, "{lighter} voxels lighter after going back");
        assert!(
            out.iter().any(|(cell, was)| back.get(cell) != Some(was)),
            "going back laid nothing"
        );
    }

    /// What a layer holds at every voxel of the body, bottom layer first:
    /// `held(app, 0)` is the bottom layer's paint, to the byte.
    fn held(app: &Hani, index: usize) -> Vec<(IVec3, Rgba)> {
        let volume = app.doc.active_volume();
        let field = volume.layer_fields()[index];
        every(app, field)
    }

    /// What is seen at `cells`: the stack put together.
    fn seen_at(app: &Hani, cells: &[IVec3]) -> Vec<Rgba> {
        let shown = app.doc.active_volume().display_paint();
        cells.iter().map(|cell| shown.at(*cell)).collect()
    }

    fn every(app: &Hani, field: &PaintField) -> Vec<(IVec3, Rgba)> {
        let volume = app.doc.active_volume();
        let mut out = Vec::new();
        for coord in volume.brick_coords() {
            for index in 0..32 * 32 * 32 {
                let cell = coord.origin() + IVec3::new(index % 32, (index / 32) % 32, index / 1024);
                let rgba = field.at(cell);
                if rgba != BARE {
                    out.push((cell, rgba));
                }
            }
        }
        out
    }

    /// The active layer's blend, and how many layers there are.
    fn active(app: &Hani) -> (hani_core::paint::layers::Blend, usize) {
        let layers = app.doc.active_volume().layers();
        (layers.metas()[layers.active()].blend, layers.len())
    }

    /// A stroke, and a frame for the stack to be put together.
    fn stroke_seen(app: &mut Hani) {
        stroke(app);
        update(app, Message::Frame);
    }

    /// The cells the active layer covers fully.
    fn covered(app: &Hani) -> Vec<IVec3> {
        let index = app.doc.active_volume().layers().active();
        held(app, index).into_iter().filter(|(_, rgba)| rgba[3] == 255).map(|(c, _)| c).collect()
    }

    /// Glaze, chosen over a body that is painted, makes a Glaze layer over
    /// the paint, and a stroke there tints what is seen of the paint under
    /// it without touching that paint, to the byte. Paint, chosen again,
    /// makes a Normal layer over the glaze, and a stroke there covers.
    /// Each layer change is one step of the undo history. Before
    /// 2026-11-27 a glaze tinted only its own layer's paint and, over a
    /// voxel that layer had none on, laid a coat that covered the paint
    /// under it.
    #[test]
    fn a_glaze_tints_the_layers_under_it_and_paint_covers_it() {
        use hani_core::paint::layers::Blend;
        let mut app = app();
        // Over an empty layer, the layer itself takes the blend and gives
        // it back. Glaze chosen with a contrast's touch in hand is a brush
        // again.
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Contrast));
        assert_eq!(active(&app), (Blend::Contrast, 1));
        set(&mut app, PaintingMessage::Mode(PaintMode::Glaze));
        assert_eq!(app.touch.kind, PaintTouch::Loaded);
        assert_eq!(active(&app), (Blend::Glaze, 1));
        assert_eq!(app.mode_in_hand(), PaintMode::Glaze);
        set(&mut app, PaintingMessage::Mode(PaintMode::Paint));
        assert_eq!(active(&app), (Blend::Normal, 1));

        set(&mut app, PaintingMessage::Colour([220, 220, 220]));
        set(&mut app, PaintingMessage::Coated);
        let below = held(&app, 0);
        let steps = app.history.stats().undo_entries;

        set(&mut app, PaintingMessage::Mode(PaintMode::Glaze));
        assert_eq!(active(&app), (Blend::Glaze, 2), "no glaze layer over the paint");
        assert_eq!(app.mode_in_hand(), PaintMode::Glaze);
        assert_eq!(app.history.stats().undo_entries, steps + 1, "a new layer is one step");
        set(&mut app, PaintingMessage::Colour([30, 60, 160]));
        stroke_seen(&mut app);
        assert_eq!(held(&app, 0), below, "the glaze changed the paint under it");
        let glazed = covered(&app);
        assert!(glazed.len() > 100, "the glaze covered {} voxels fully", glazed.len());
        for [r, g, b, a] in seen_at(&app, &glazed) {
            assert_eq!(a, 255, "the paint under the glaze is not seen");
            // Bluer than the grey, and the grey still showing: not the
            // glaze's own colour laid over it.
            assert!(b > r + 20 && r > 30 + 40, "a blue glaze over grey is {:?}", [r, g, b]);
        }

        set(&mut app, PaintingMessage::Mode(PaintMode::Paint));
        assert_eq!(active(&app), (Blend::Normal, 3), "no normal layer over the glaze");
        set(&mut app, PaintingMessage::Colour([200, 30, 20]));
        stroke_seen(&mut app);
        let painted = covered(&app);
        assert!(painted.len() > 100);
        let shown = seen_at(&app, &painted);
        assert!(shown.iter().all(|rgba| *rgba == [200, 30, 20, 255]), "paint did not cover");

        // Undo takes back the stroke, the layer, the glaze's stroke and
        // its layer, one step each.
        for layers in [3, 2, 2, 1] {
            update(&mut app, Message::Undo);
            assert_eq!(active(&app).1, layers);
        }
        assert_eq!(active(&app), (Blend::Normal, 1));
    }

    /// After a wash or a contrast, an ordinary touch goes on a Normal layer
    /// and covers. Before 2026-11-27 it went on painting on the wash's or
    /// the contrast's layer, as a film.
    #[test]
    fn an_ordinary_touch_after_a_wash_or_a_contrast_covers() {
        use hani_core::paint::layers::Blend;
        for (touch, blend) in
            [(PaintTouch::Wash, Blend::Wash), (PaintTouch::Contrast, Blend::Contrast)]
        {
            let mut app = app();
            set(&mut app, PaintingMessage::Colour([220, 220, 220]));
            set(&mut app, PaintingMessage::Coated);
            update(&mut app, Message::PaintTouchChosen(touch));
            assert_eq!(active(&app), (blend, 2), "{touch:?}");
            set(&mut app, PaintingMessage::Colour([30, 60, 160]));
            stroke_seen(&mut app);
            update(&mut app, Message::PaintTouchChosen(PaintTouch::Loaded));
            assert_eq!(active(&app), (Blend::Normal, 3), "after {touch:?}");
            assert_eq!(app.mode_in_hand(), PaintMode::Paint);
            set(&mut app, PaintingMessage::Colour([200, 30, 20]));
            stroke_seen(&mut app);
            let painted = covered(&app);
            assert!(painted.len() > 100);
            let shown = seen_at(&app, &painted);
            assert!(
                shown.iter().all(|rgba| *rgba == [200, 30, 20, 255]),
                "paint after {touch:?} did not cover"
            );
        }
    }

    /// Erase changes no layer, whatever touch is chosen while it is in
    /// hand, and takes paint off the layer that is active. The way of
    /// painting shown follows a layer chosen by hand.
    #[test]
    fn erase_changes_no_layer_and_the_mode_shown_follows_the_layer() {
        use crate::message::LayersMessage;
        use hani_core::paint::layers::Blend;
        let mut app = app();
        set(&mut app, PaintingMessage::Coated);
        set(&mut app, PaintingMessage::Mode(PaintMode::Glaze));
        stroke_seen(&mut app);
        assert_eq!(active(&app), (Blend::Glaze, 2));

        update(&mut app, Message::Layers(LayersMessage::Chosen(0)));
        assert_eq!(
            app.mode_in_hand(),
            PaintMode::Paint,
            "a Normal layer chosen by hand is not shown as Paint"
        );
        update(&mut app, Message::Layers(LayersMessage::Chosen(1)));
        assert_eq!(
            app.mode_in_hand(),
            PaintMode::Glaze,
            "a Glaze layer chosen by hand is not shown as Glaze"
        );
        update(&mut app, Message::Layers(LayersMessage::Chosen(0)));

        let steps = app.history.stats().undo_entries;
        let under = held(&app, 0);
        set(&mut app, PaintingMessage::Mode(PaintMode::Erase));
        for touch in [PaintTouch::Wash, PaintTouch::Contrast, PaintTouch::Loaded] {
            update(&mut app, Message::PaintTouchChosen(touch));
            assert_eq!(active(&app), (Blend::Normal, 2), "Erase with {touch:?} changed the layer");
        }
        assert_eq!(app.mode_in_hand(), PaintMode::Erase);
        assert_eq!(app.history.stats().undo_entries, steps, "Erase put a layer change on");
        let glaze = held(&app, 1);
        stroke_seen(&mut app);
        assert_eq!(held(&app, 1), glaze, "the eraser took paint off a layer not active");
        let thinned = held(&app, 0).len() < under.len()
            || held(&app, 0).iter().zip(&under).any(|((_, now), (_, was))| now[3] < was[3]);
        assert!(thinned, "the eraser took nothing off the active layer");
    }
}
