// SPDX-License-Identifier: AGPL-3.0-only

//! The paint brush, painting: a colour under the brush, an opacity, and a
//! way of laying it.
//!
//! `docs/STUDIO-PLAN.md` section 15 is the design, and
//! [`brokkr_core::paint::stroke`] is the brush. This is what the panel
//! offers of it and what a drag of the pointer does with it.
//!
//! # Beside the rack, for now
//!
//! Painting by filament slot is still here, behind the second of two
//! buttons at the top of the paint brush's block. It goes when the exports
//! read the paint and an old document's slots are laid into it (stages P4
//! and P5 of the plan). Until then a document painted the old way can still
//! be worked on the old way, and nothing of the rack's has been touched to
//! make room: this module is the new brush and a switch.
//!
//! # What a drag does
//!
//! A press starts a stroke, and the stroke lays one coat however many times
//! it crosses itself. Ctrl or alt, which unpaints a slot, takes paint off
//! here too, by the brush's own opacity and with its own soft edge. With
//! the eyedropper armed, a press takes the colour under the pointer and
//! nothing is painted until the next press.

use brokkr_core::paint::field::Rgba;
use brokkr_core::paint::stroke::{Catch, Lay, Mode, PaintStroke, Pigment, laid};
use brokkr_core::paint::touch::Coat;
use brokkr_core::{Brush, BrushDirection, Stamp};
use glam::Vec3;
use iced::widget::{button, checkbox, column, container, row, text};
use iced::{Alignment, Element, Length};

use super::Brokkr;
use super::paints::RANGE;
use crate::message::{Message, PaintMode, PaintTarget, PaintTouch, PaintingMessage};
use crate::theme;

/// How little and how much of itself a full touch of the brush may lay.
pub(super) const OPACITY_RANGE: std::ops::RangeInclusive<f32> = 0.05..=1.0;

/// How unlike the paint it began on a fill runs over, until it is told
/// otherwise: an eighth of the way from black to white. Enough to take a
/// shaded panel as one panel, and not enough to run from one colour onto
/// another.
const USUAL_LIKENESS: u8 = 32;

/// How many swatches of the range go on a line of the panel.
const SWATCHES_A_ROW: usize = 8;

/// The colour the brush starts with: a red a miniature painter would know,
/// and not a grey that could be taken for the clay.
const FIRST_COLOUR: [u8; 3] = [178, 34, 34];

/// The painting brush's own state, in one field of `Brokkr`. The brush's,
/// and not the document's: nothing here is saved with a file.
#[derive(Debug)]
pub(super) struct PaintingState {
    pub target: PaintTarget,
    /// sRGB.
    pub colour: [u8; 3],
    pub opacity: f32,
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
}

impl Default for PaintingState {
    fn default() -> Self {
        Self {
            target: PaintTarget::default(),
            colour: FIRST_COLOUR,
            opacity: 1.0,
            mode: PaintMode::default(),
            picking: false,
            held_off: false,
            fill_like: USUAL_LIKENESS,
            stroke: PaintStroke::new(),
            layers: Default::default(),
        }
    }
}

impl PaintingState {
    /// What is on the brush, for a stroke in `direction`: the inverted
    /// direction takes paint off, whatever the mode.
    fn pigment(&self, direction: BrushDirection) -> Pigment {
        let mode = match (direction, self.mode) {
            (BrushDirection::Subtract, _) | (_, PaintMode::Erase) => Mode::Erase,
            (_, PaintMode::Paint) => Mode::Paint,
            (_, PaintMode::Glaze) => Mode::Glaze,
        };
        Pigment { colour: self.colour, opacity: self.opacity, mode }
    }
}

fn swatch_colour(colour: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(colour[0], colour[1], colour[2])
}

impl Brokkr {
    /// Whether the paint brush is laying colour and not filament slots.
    pub(super) fn painting_colour(&self) -> bool {
        self.painting.target == PaintTarget::Colour
    }

    pub(super) fn painting_changed(&mut self, message: PaintingMessage) {
        match message {
            PaintingMessage::Target(target) => {
                self.painting.target = target;
                self.painting.picking = false;
                // A slot cannot take a wash.
                if target == PaintTarget::Slots && self.touch.kind == PaintTouch::Wash {
                    self.touch.kind = PaintTouch::Loaded;
                }
            }
            PaintingMessage::Colour(colour) => self.painting.colour = colour,
            PaintingMessage::Channel(channel, level) => {
                if let Some(slot) = self.painting.colour.get_mut(channel) {
                    *slot = level;
                }
            }
            PaintingMessage::Opacity(opacity) => {
                self.painting.opacity = opacity.clamp(*OPACITY_RANGE.start(), *OPACITY_RANGE.end());
            }
            PaintingMessage::Mode(mode) => self.painting.mode = mode,
            PaintingMessage::FillLike(levels) => self.painting.fill_like = levels,
            PaintingMessage::PickArmed => self.painting.picking = !self.painting.picking,
            PaintingMessage::Coated => self.coat_the_body_in_colour(),
        }
    }

    /// What the eyedropper does with a press at `point`, a world position
    /// on the active body's surface, and whether the drag is spoken for:
    /// `true` when nothing is to be painted by this pointer event.
    ///
    /// Asked before the stroke is armed, so that taking a colour is not an
    /// entry in the undo history.
    pub(super) fn eyedropper_takes(&mut self, point: Vec3, start: bool) -> bool {
        if !self.painting_colour() {
            return false;
        }
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
            PaintTouch::Loaded | PaintTouch::Fill => Catch::All,
            PaintTouch::Wash => Catch::Recesses,
            PaintTouch::Drybrush => Catch::Raised(self.touch.dry),
            PaintTouch::Sponge => Catch::Specks(self.touch.sponge),
        };
        let lay = Lay {
            pigment: self.painting.pigment(direction),
            catch,
            seed: coat.seed,
            origin: coat.origin,
            load: coat.load,
            crease_gain: coat.crease_gain,
        };
        self.painting.stroke.stamp_symmetric(
            self.doc.active_volume_mut(),
            brush,
            stamp,
            &lay,
            self.symmetry,
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
        let pigment = self.painting.pigment(direction);
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
        let pigment = self.painting.pigment(BrushDirection::Add);
        self.arm_recorder();
        let volume = self.doc.active_volume_mut();
        let (lo, hi) = volume.voxel_bounds(low, high);
        volume.edit_paint(lo, hi, |_, _, held: Rgba, protection| {
            let free = 1.0 - f32::from(protection) / f32::from(brokkr_core::PROTECTED);
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
        let targets =
            PaintTarget::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, target| {
                assembled.push(
                    button(text(target.label()).size(theme::LABEL_SIZE))
                        .width(Length::Fill)
                        .style(if target == self.painting.target {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::Painting(PaintingMessage::Target(target))),
                )
            });
        let chosen: Element<'_, Message> = match self.painting.target {
            PaintTarget::Colour => self.colour_picker(editing),
            PaintTarget::Slots => self.paint_picker(editing),
        };
        column![targets, chosen].spacing(theme::S2).into()
    }

    /// The colour brush: what it does, how it touches, the colour, and how
    /// much of it.
    fn colour_picker(&self, editing: bool) -> Element<'_, Message> {
        let reading = |label: String| text(label).size(theme::LABEL_SIZE).color(theme::TEXT_DIM);
        let painting = &self.painting;
        let colour = painting.colour;

        let modes =
            PaintMode::ALL.into_iter().fold(row![].spacing(theme::S1), |assembled, mode| {
                assembled.push(
                    button(text(mode.label()).size(theme::LABEL_SIZE))
                        .width(Length::Fill)
                        .style(if mode == painting.mode {
                            theme::tool_button_active
                        } else {
                            theme::tool_button
                        })
                        .on_press(Message::Painting(PaintingMessage::Mode(mode))),
                )
            });

        // The button is as wide as the row will let it be and no wider: at
        // its own width, beside three three-digit numbers, its label broke
        // onto a second line. Seen on a screen.
        let in_hand = row![
            container(text("")).width(28).height(18).style(theme::swatch(swatch_colour(colour))),
            reading(format!("{:3} {:3} {:3}", colour[0], colour[1], colour[2])),
            button(text("Pick from model").size(theme::LABEL_SIZE))
                .width(Length::Fill)
                .style(if painting.picking {
                    theme::tool_button_active
                } else {
                    theme::tool_button
                })
                .on_press(Message::Painting(PaintingMessage::PickArmed)),
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

        // The machine's own filaments first. A colour painted at full
        // strength in one of these prints in exactly that filament: the
        // nearest filament to a filament's colour is itself.
        let mut spools = row![].spacing(theme::S1);
        for slot in self.palette.slots.iter().take(SWATCHES_A_ROW) {
            let swatch = slot.swatch().into_rgba8();
            let each = [swatch[0], swatch[1], swatch[2]];
            let (style, _) = theme::tool_toggle(each == colour);
            spools = spools.push(
                button(
                    container(text("")).width(14).height(14).style(theme::swatch(slot.swatch())),
                )
                .padding(2)
                .style(style)
                .on_press(Message::Painting(PaintingMessage::Colour(each))),
            );
        }
        let spools =
            column![reading("The machine's filaments".to_string()), spools].spacing(theme::S1);

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
                    .on_press(Message::Painting(PaintingMessage::Colour(*each))),
                );
            }
            range = range.push(swatches);
        }
        let channels = [("paint.red", "Red"), ("paint.green", "Green"), ("paint.blue", "Blue")]
            .into_iter()
            .enumerate()
            .fold(column![].spacing(theme::S2), |assembled, (channel, (key, label))| {
                assembled.push(
                    crate::ui::number(
                        &self.drafts,
                        key,
                        label,
                        f32::from(colour[channel]),
                        0.0..=255.0,
                        move |level| {
                            Message::Painting(PaintingMessage::Channel(
                                channel,
                                level.round() as u8,
                            ))
                        },
                    )
                    .step(1.0)
                    .shown(colour[channel].to_string()),
                )
            });
        picker = picker
            .push(spools)
            .push(range)
            .push(channels)
            .push(
                checkbox(self.flat_paint)
                    .label("flat colour, no light")
                    .on_toggle(|_| Message::FlatPaintToggled)
                    .text_size(theme::LABEL_SIZE),
            )
            .push(
                button(text("Coat the body").size(theme::LABEL_SIZE))
                    .style(theme::tool_button)
                    .on_press(Message::Painting(PaintingMessage::Coated)),
            );
        picker.into()
    }
}

/// The colour brush done the way a user does it: messages and pointer
/// events in, and the body's paint looked at afterwards.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};
    use brokkr_core::BrushKind;
    use brokkr_core::paint::field::{BARE, PaintField};
    use glam::IVec3;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };
    const CENTRE: Vector = Vector { x: 400.0, y: 300.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn set(app: &mut Brokkr, message: PaintingMessage) {
        update(app, Message::Painting(message));
    }

    /// The application with the paint brush in hand, as it starts: in
    /// colour.
    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
    }

    fn press(app: &mut Brokkr, at: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
    }

    fn drag_to(app: &mut Brokkr, at: Vector) {
        update(app, Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }));
    }

    fn release(app: &mut Brokkr) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short drag across the middle of the model, there and back.
    fn stroke(app: &mut Brokkr) {
        press(app, CENTRE);
        for step in [20.0, 40.0, 20.0, 0.0] {
            drag_to(app, Vector::new(CENTRE.x + step, CENTRE.y));
        }
        release(app);
    }

    /// Every painted voxel of the active body.
    fn painted(app: &Brokkr) -> Vec<Rgba> {
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
        volume.edit_mask(lo, hi, |_, at, _| if at.x < 0.0 { brokkr_core::PROTECTED } else { 0 });
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
        let washed = laid_with(PaintTouch::Wash);
        let most = f32::from(most_covered(&washed)) / 255.0;
        let flat = brokkr_core::paint::stroke::WASH_FLAT;
        assert!(
            (most - flat).abs() < 0.1,
            "a wash on a ball left {most} at most, and {flat} is a flat"
        );
    }

    /// A click with the Fill touch pours the colour over the paint it lands
    /// on and paint like it, and stops at paint that is not; told that
    /// anything is alike, it takes the body. Each click is one undo.
    #[test]
    fn a_fill_pours_the_colour_over_paint_like_where_it_landed() {
        let click = |app: &mut Brokkr| {
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
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Wide));
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

    /// The other way of painting is one button away and is what it was,
    /// and drops the one touch it has not got.
    #[test]
    fn filaments_are_painted_as_before_behind_their_button() {
        let mut app = app();
        assert!(app.painting_colour());
        set(&mut app, PaintingMessage::Target(PaintTarget::Slots));
        stroke(&mut app);
        assert!(painted(&app).is_empty(), "a slot stroke laid colour");
        assert!(app.doc.active_volume().stats().colour_bricks > 0, "a slot stroke painted no slot");

        update(&mut app, Message::PaintTouchChosen(PaintTouch::Fill));
        set(&mut app, PaintingMessage::Target(PaintTarget::Colour));
        assert_eq!(app.touch.kind, PaintTouch::Fill, "both ways of painting have a fill");
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Wash));
        set(&mut app, PaintingMessage::Target(PaintTarget::Slots));
        assert_eq!(app.touch.kind, PaintTouch::Loaded, "a slot cannot take a wash");
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Sponge));
        set(&mut app, PaintingMessage::Target(PaintTarget::Colour));
        assert_eq!(app.touch.kind, PaintTouch::Sponge, "a touch both have was dropped");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("brokkr-colour-{name}-{}", std::process::id()));
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
        update(&mut app, Message::FalloffChanged(brokkr_core::FalloffCurve::Wide));
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

        for target in PaintTarget::ALL {
            set(&mut app, PaintingMessage::Target(target));
            for mode in PaintMode::ALL {
                set(&mut app, PaintingMessage::Mode(mode));
                for touch in PaintTouch::ALL {
                    update(&mut app, Message::PaintTouchChosen(touch));
                    drop(app.painting_picker(true));
                    drop(app.painting_picker(false));
                }
            }
        }
        // The whole window, with the colour brush up.
        set(&mut app, PaintingMessage::Target(PaintTarget::Colour));
        drop(app.view());
    }
}
