// SPDX-License-Identifier: AGPL-3.0-only

//! The brush engine, where a user can reach it.
//!
//! `brokkr_core::brush::engine` rebuilt the brush as a composition and held
//! it to upstream's brushes bit for bit. That bought nothing a user could
//! see. This is the first of what it was built to carry:
//!
//! * **Operators upstream has no brush for.** Crease is the first: a groove
//!   cut and pinched in one stamp.
//! * **A lazy stroke.** The brush trails the pointer on a short lead, so a
//!   hand's tremor is not in the line. The single most useful thing for a
//!   panel line, a seam, or lettering.
//! * **Size from pressure.** A pen pressed harder is a bigger brush, for a
//!   stroke that swells and tapers.
//!
//! A child module of `app` and a file of its own for the reason `paints.rs`
//! is one: `docs/STUDIO-PLAN.md` decision D6, add and do not modify. What
//! `app.rs` holds for this is one field, one message arm, and four changed
//! lines in the stroke.
//!
//! # A studio operator borrows a classic brush's settings
//!
//! Upstream's brushes are an enum with a digit key each, a remembered
//! strength each and a button each on the tool strip. A new operator added
//! to that enum would be a change to upstream's hottest file and to every
//! `match` on it. So a studio operator is chosen BESIDE the classic brush
//! and not instead of it: the radius, the strength, the falloff and the
//! pattern are still the classic brush's, and the operator replaces only
//! what one weighted voxel does. Choosing any classic brush puts the
//! operator away, and holding shift still smooths.

use brokkr_core::brush::engine::{BrushDef, FieldOp, Tip};
use brokkr_core::{Brush, Stamp};
use glam::Vec2;
use iced::widget::{button, column, row, slider, text};
use iced::{Element, Length};

use super::Brokkr;
use crate::message::{Message, StudioMessage};
use crate::theme;

/// How far behind the pointer a lazy stroke may trail, in pixels of the
/// viewport. Eighty is a long lead: at that the brush is being towed.
pub(super) const LAZY_RANGE: std::ops::RangeInclusive<f32> = 0.0..=80.0;

/// How much of the brush's size pressure decides, 0 for none of it.
pub(super) const SIZE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;

/// The least a brush shrinks to under the lightest touch, as a share of its
/// size. Not nothing: a stroke that begins with no brush at all begins with
/// a gap, and the first stamps of a pen stroke are always its lightest.
const SMALLEST_SHARE: f32 = 0.1;

/// What the application remembers for the studio brushes.
#[derive(Debug, Default)]
pub(super) struct StudioState {
    /// The operator in use, or `None` for the classic brush's own.
    pub op: Option<FieldOp>,
    /// How far the brush trails the pointer, in pixels. Zero is off.
    pub lazy_px: f32,
    /// How much of the brush's size pressure decides.
    pub size_from_pressure: f32,
    /// Where the lazy brush is, in pixels of the viewport.
    anchor: Vec2,
}

impl StudioState {
    /// Where the brush is when the pointer is at `pixel`.
    ///
    /// The brush is on a lead of `lazy_px`. While the pointer is within the
    /// lead's length of it, the brush stays where it is; past that it is
    /// pulled straight toward the pointer and stops a lead's length short.
    /// So a wobble smaller than the lead moves nothing, and a deliberate
    /// stroke is followed along a line smoother than the hand's.
    pub(super) fn follow(&mut self, pixel: Vec2, start: bool) -> Vec2 {
        if start || self.lazy_px <= 0.0 {
            self.anchor = pixel;
            return pixel;
        }
        let lead = pixel - self.anchor;
        let length = lead.length();
        if length > self.lazy_px {
            self.anchor += lead * ((length - self.lazy_px) / length);
        }
        self.anchor
    }

    /// The brush with its radius set by how hard the pen is pressed.
    ///
    /// A mouse reports full pressure and gets the brush as it is.
    pub(super) fn sized(&self, brush: Brush, pressure: f32) -> Brush {
        let lightness = 1.0 - pressure.clamp(0.0, 1.0);
        let share = (1.0 - self.size_from_pressure * lightness).max(SMALLEST_SHARE);
        Brush { radius: brush.radius * share, ..brush }
    }

    fn set(&mut self, message: StudioMessage) {
        match message {
            StudioMessage::OperatorChosen(op) => {
                // Only the operators that have no classic brush. One that
                // has is chosen as that brush, with its own key and its own
                // remembered strength.
                self.op = op.filter(|op| FieldOp::STUDIO.contains(op));
            }
            StudioMessage::LazyChanged(pixels) => self.lazy_px = clamped(pixels, LAZY_RANGE),
            StudioMessage::SizeFromPressureChanged(share) => {
                self.size_from_pressure = clamped(share, SIZE_RANGE);
            }
        }
    }
}

/// Into a slider's range, with a NaN sent to the bottom of it.
fn clamped(value: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if value.is_nan() { *range.start() } else { value.clamp(*range.start(), *range.end()) }
}

impl Brokkr {
    pub(super) fn studio_changed(&mut self, message: StudioMessage) {
        self.studio.set(message);
    }

    /// The operator a field stamp runs with: the studio one, unless shift is
    /// held, which smooths whatever is chosen.
    fn live_operator(&self) -> Option<FieldOp> {
        self.studio.op.filter(|_| !self.shift)
    }

    /// What the brush in hand is called, for the panel's heading.
    pub(super) fn live_brush_name(&self) -> String {
        match self.live_operator() {
            Some(op) => op.label().to_uppercase(),
            None => self.brush.kind.label().to_uppercase(),
        }
    }

    /// Lay one stamp of the field brush, and its mirrors.
    ///
    /// `brush` is the brush the stroke resolved, which is Smooth while shift
    /// is held. With a studio operator chosen the stamp goes through the
    /// engine with that operator and the brush's own size, strength and tip;
    /// without one it is upstream's stamp, called and not copied.
    pub(super) fn stamp_field(&mut self, brush: Brush, stamp: &Stamp) {
        let brush = self.studio.sized(brush, stamp.pressure);
        let operator = self.live_operator();
        let volume = self.doc.active_volume_mut();
        match operator {
            Some(op) => BrushDef {
                op,
                radius: brush.radius,
                strength: brush.strength,
                tip: Tip { falloff: brush.falloff, pattern: brush.pattern },
            }
            .apply_symmetric(
                volume,
                stamp,
                self.symmetry,
                super::MIRROR_CENTRE,
                &mut self.brush_scratch,
            ),
            None => brush.apply_symmetric(
                volume,
                stamp,
                self.symmetry,
                super::MIRROR_CENTRE,
                &mut self.brush_scratch,
            ),
        }
    }

    /// The studio controls, under the classic brush's own.
    pub(super) fn studio_panel(&self) -> Element<'_, Message> {
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let changed = |message: StudioMessage| Message::Studio(message);
        let operators =
            FieldOp::STUDIO.into_iter().fold(row![].spacing(theme::S1), |assembled, op| {
                let live = self.studio.op == Some(op);
                assembled.push(
                    button(text(op.label()).size(theme::CAPTION_SIZE))
                        .width(Length::Fill)
                        .style(if live { theme::tool_button_active } else { theme::tool_button })
                        // Pressed again it is put away, back to the classic
                        // brush whose settings it was borrowing.
                        .on_press(changed(StudioMessage::OperatorChosen((!live).then_some(op)))),
                )
            });
        column![
            text("STUDIO").size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE),
            operators,
            reading(if self.studio.lazy_px > 0.0 {
                format!("Lazy  {:.0} px behind the pointer", self.studio.lazy_px)
            } else {
                "Lazy  off".to_string()
            }),
            slider(LAZY_RANGE, self.studio.lazy_px, move |pixels| {
                changed(StudioMessage::LazyChanged(pixels))
            })
            .step(1.0_f32),
            reading(format!("Size from pressure  {:.2}", self.studio.size_from_pressure)),
            slider(SIZE_RANGE, self.studio.size_from_pressure, move |share| {
                changed(StudioMessage::SizeFromPressureChanged(share))
            })
            .step(0.01_f32),
        ]
        .spacing(theme::S1)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};
    use brokkr_core::BrushKind;
    use glam::Vec3;

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    fn centre() -> iced::Vector {
        iced::Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
    }

    fn press(app: &mut Brokkr) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre(),
                size: SIZE,
            }),
        );
    }

    fn drag_by(app: &mut Brokkr, pixels: f32) {
        let position = iced::Vector::new(centre().x + pixels, centre().y);
        update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
    }

    fn release(app: &mut Brokkr) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short drag across the middle of the model.
    fn stroke(app: &mut Brokkr) {
        press(app);
        for step in 1..=4 {
            drag_by(app, 12.0 * step as f32);
        }
        release(app);
    }

    /// How far the surface under the middle of the viewport has moved from
    /// where a fresh ball has it, in millimetres, outward positive.
    fn shift_under_the_stroke(app: &Brokkr) -> f32 {
        let fresh = self::app();
        // The point of the fresh ball's surface nearest the camera, which is
        // where the stroke pressed.
        let toward_camera = (fresh.camera.eye() - Vec3::ZERO).normalize();
        let on_the_surface = toward_camera * super::super::MODEL_RADIUS_MM;
        let volume = app.doc.active_volume();
        -volume.sample_world(on_the_surface) * volume.voxel_size()
    }

    /// The lead: nothing moves inside it, and past it the brush is exactly a
    /// lead's length behind the pointer, on the line between them.
    #[test]
    fn a_lazy_brush_trails_the_pointer_by_its_lead_and_no_more() {
        let mut studio = StudioState { lazy_px: 20.0, ..StudioState::default() };
        let start = Vec2::new(100.0, 100.0);
        assert_eq!(studio.follow(start, true), start, "a stroke begins under the pointer");
        assert_eq!(studio.follow(Vec2::new(115.0, 100.0), false), start, "inside the lead");
        assert_eq!(studio.follow(Vec2::new(100.0, 119.0), false), start, "inside the lead");
        let pulled = studio.follow(Vec2::new(150.0, 100.0), false);
        assert_eq!(pulled, Vec2::new(130.0, 100.0), "the brush is not a lead behind the pointer");
        // Back toward it by less than the lead: the brush does not move.
        assert_eq!(studio.follow(Vec2::new(140.0, 100.0), false), pulled);
        // Off at an angle: toward the pointer, a lead short.
        let pointer = Vec2::new(130.0, 160.0);
        let turned = studio.follow(pointer, false);
        assert!((turned.distance(pointer) - 20.0).abs() < 1.0e-3, "{turned}");
        assert!((turned.x - 130.0).abs() < 1.0e-3, "the brush left the line to the pointer");
        // A new stroke forgets the old one's place.
        assert_eq!(studio.follow(Vec2::new(5.0, 5.0), true), Vec2::new(5.0, 5.0));

        // With no lead the brush IS the pointer.
        let mut off = StudioState::default();
        off.follow(start, true);
        assert_eq!(off.follow(Vec2::new(300.0, 7.0), false), Vec2::new(300.0, 7.0));
    }

    #[test]
    fn pressure_sizes_the_brush_by_as_much_as_it_is_given() {
        let brush = Brush { radius: 4.0, ..Brush::default() };
        let none = StudioState::default();
        assert_eq!(none.sized(brush, 0.2).radius, 4.0, "with no share, pressure sized the brush");
        let all = StudioState { size_from_pressure: 1.0, ..StudioState::default() };
        assert_eq!(all.sized(brush, 1.0).radius, 4.0, "full pressure is the whole brush");
        assert_eq!(all.sized(brush, 0.5).radius, 2.0);
        assert_eq!(all.sized(brush, 0.0).radius, 4.0 * SMALLEST_SHARE, "no touch, no brush");
        assert_eq!(all.sized(brush, 7.0).radius, 4.0, "pressure past full grew the brush");
        let half = StudioState { size_from_pressure: 0.5, ..StudioState::default() };
        assert_eq!(half.sized(brush, 0.0).radius, 2.0);
        // And nothing else about the brush is touched.
        assert_eq!(all.sized(brush, 0.5), Brush { radius: 2.0, ..brush });
    }

    /// Crease chosen, a stroke cuts in where the classic Draw under it
    /// raises; a classic brush chosen puts it away; shift still smooths.
    #[test]
    fn a_studio_operator_replaces_what_the_stroke_does_until_a_classic_brush_is_chosen() {
        let mut drawn = app();
        stroke(&mut drawn);
        let raised = shift_under_the_stroke(&drawn);
        assert!(raised > 0.05, "the classic Draw control raised only {raised} mm");

        let mut creased = app();
        update(&mut creased, Message::Studio(StudioMessage::OperatorChosen(Some(FieldOp::Crease))));
        assert_eq!(creased.live_brush_name(), "CREASE");
        stroke(&mut creased);
        let cut = shift_under_the_stroke(&creased);
        assert!(cut < -0.02, "a crease stroke moved the surface by {cut} mm, which is not in");
        update(&mut creased, Message::Undo);
        // Against a ball nobody has touched and not against nothing: the
        // probe is a point in space, and the lattice's ball is not exactly
        // on it.
        assert_eq!(
            shift_under_the_stroke(&creased),
            shift_under_the_stroke(&app()),
            "a crease stroke was not one undo"
        );

        // Shift smooths, whatever is chosen: the same stroke with shift held
        // leaves exactly what it leaves with no studio operator at all.
        let smoothed = |studio: bool| {
            let mut app = app();
            stroke(&mut app);
            if studio {
                update(
                    &mut app,
                    Message::Studio(StudioMessage::OperatorChosen(Some(FieldOp::Crease))),
                );
            }
            app.shift = true;
            assert_eq!(app.live_brush_name(), app.brush.kind.label().to_uppercase());
            stroke(&mut app);
            shift_under_the_stroke(&app)
        };
        assert_eq!(smoothed(true), smoothed(false), "shift did not smooth over a studio operator");

        // A classic brush puts the operator away.
        update(&mut creased, Message::BrushKindChanged(BrushKind::Inflate));
        assert_eq!(creased.studio.op, None, "choosing a classic brush kept the studio operator");
        assert_eq!(creased.live_brush_name(), "INFLATE");

        // And an operator that HAS a classic brush is not a studio operator.
        update(&mut creased, Message::Studio(StudioMessage::OperatorChosen(Some(FieldOp::Draw))));
        assert_eq!(creased.studio.op, None, "a classic operator was taken as a studio one");
    }

    /// The lead reaches the stroke: with a lead longer than the drag, the
    /// brush never leaves where it was pressed and lays no stamp after the
    /// first.
    #[test]
    fn a_lazy_stroke_lays_nothing_until_the_pointer_outruns_its_lead() {
        let mut app = app();
        update(&mut app, Message::Studio(StudioMessage::LazyChanged(60.0)));
        press(&mut app);
        drag_by(&mut app, 20.0);
        assert_eq!(app.perf.stamps, 0, "the brush moved inside its lead");
        drag_by(&mut app, 45.0);
        assert_eq!(app.perf.stamps, 0, "the brush moved inside its lead");
        drag_by(&mut app, 140.0);
        assert!(app.perf.stamps > 0, "the pointer outran the lead and the brush did not follow");
        release(&mut app);

        // The control: with no lead the same first drag lays stamps.
        let mut direct = self::app();
        press(&mut direct);
        drag_by(&mut direct, 45.0);
        assert!(direct.perf.stamps > 0, "a 45 pixel drag lays no stamp even without a lead");
    }

    /// How many voxels of the active body are painted at all.
    fn painted_voxels(app: &Brokkr) -> usize {
        let volume = app.doc.active_volume();
        let (low, high) = volume.world_bounds().expect("the body has bricks");
        let (lo, hi) = volume.voxel_bounds(low, high);
        let mut count = 0;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    count += usize::from(volume.colour().at(glam::IVec3::new(x, y, z)) != 0);
                }
            }
        }
        count
    }

    /// A pen pressed lightly is a smaller brush when size follows pressure,
    /// for the paint brush as for the others, and is the brush as it was
    /// when size does not. Measured with paint, which reads pressure for
    /// nothing else: a slot has no half.
    #[test]
    fn a_light_touch_paints_a_narrower_stroke_when_size_follows_pressure() {
        let painted = |share: f32, pressure: f32| {
            let mut app = app();
            update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
            update(&mut app, Message::Studio(StudioMessage::SizeFromPressureChanged(share)));
            app.tablet.simulate(crate::tablet::PenState {
                in_proximity: true,
                pressure,
                eraser: false,
                tilt: Vec2::ZERO,
            });
            stroke(&mut app);
            painted_voxels(&app)
        };
        let (full, light) = (painted(1.0, 1.0), painted(1.0, 0.4));
        assert!(full > 500, "the full pressure stroke painted only {full} voxels");
        assert!(
            light * 2 < full,
            "at 0.4 of the pressure the stroke painted {light} voxels against {full}"
        );
        assert_eq!(painted(0.0, 0.4), full, "with no share, pressure still sized the paint brush");
    }

    /// How many voxels of the active body differ from an untouched ball's.
    fn moved_voxels(app: &Brokkr) -> usize {
        let fresh = self::app();
        let (volume, untouched) = (app.doc.active_volume(), fresh.doc.active_volume());
        let (low, high) = untouched.world_bounds().expect("the body has bricks");
        let (lo, hi) = untouched.voxel_bounds(low - Vec3::splat(4.0), high + Vec3::splat(4.0));
        let mut count = 0;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = glam::IVec3::new(x, y, z);
                    count += usize::from(
                        volume.sample_voxel(voxel).to_bits()
                            != untouched.sample_voxel(voxel).to_bits(),
                    );
                }
            }
        }
        count
    }

    /// And the sculpt brushes: at the same light pressure, a stroke whose
    /// size follows pressure touches fewer voxels than one whose size does
    /// not. The pressure is the same in both, so the strength is, and the
    /// only thing left to differ is how wide the brush was.
    #[test]
    fn a_light_touch_sculpts_a_narrower_stroke_when_size_follows_pressure() {
        let moved = |share: f32| {
            let mut app = app();
            update(&mut app, Message::Studio(StudioMessage::SizeFromPressureChanged(share)));
            app.tablet.simulate(crate::tablet::PenState {
                in_proximity: true,
                pressure: 0.4,
                eraser: false,
                tilt: Vec2::ZERO,
            });
            stroke(&mut app);
            moved_voxels(&app)
        };
        let (wide, narrow) = (moved(0.0), moved(1.0));
        assert!(wide > 500, "the control stroke moved only {wide} voxels");
        assert!(narrow > 0, "a light touch with size from pressure moved nothing at all");
        assert!(
            narrow * 2 < wide,
            "with size following pressure the stroke moved {narrow} voxels against {wide}"
        );
    }

    #[test]
    fn studio_settings_are_clamped_and_the_panel_builds() {
        let mut app = app();
        let set = |app: &mut Brokkr, message| update(app, Message::Studio(message));
        set(&mut app, StudioMessage::LazyChanged(33.0));
        set(&mut app, StudioMessage::SizeFromPressureChanged(0.4));
        assert_eq!((app.studio.lazy_px, app.studio.size_from_pressure), (33.0, 0.4));
        set(&mut app, StudioMessage::LazyChanged(900.0));
        set(&mut app, StudioMessage::SizeFromPressureChanged(-3.0));
        assert_eq!(app.studio.lazy_px, *LAZY_RANGE.end());
        assert_eq!(app.studio.size_from_pressure, 0.0);
        set(&mut app, StudioMessage::LazyChanged(f32::NAN));
        set(&mut app, StudioMessage::SizeFromPressureChanged(f32::NAN));
        assert_eq!((app.studio.lazy_px, app.studio.size_from_pressure), (0.0, 0.0));
        drop(app.studio_panel());
        set(&mut app, StudioMessage::OperatorChosen(Some(FieldOp::Crease)));
        drop(app.studio_panel());
    }
}
