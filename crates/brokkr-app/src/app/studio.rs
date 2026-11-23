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
//! * **The stamp.** One image pressed into the surface with one drag: press
//!   where it goes, drag to size it and to turn it. A rivet, a scale, a
//!   rune, or any picture, read as heights.
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

use std::sync::Arc;

use brokkr_core::brush::alpha::{Alpha, BuiltIn};
use brokkr_core::brush::engine::{BrushDef, FieldOp, Tip};
use brokkr_core::brush::stamp::{Placement, StampError, StampGesture};
use brokkr_core::{Brush, BrushDirection, Stamp};
use glam::{Vec2, Vec3};
use iced::widget::{button, column, row, slider, text};
use iced::{Element, Length, Task};

use super::Brokkr;
use crate::message::{LoadedStamp, Message, StudioMessage};
use crate::theme;

/// How tall a stamp is, as a share of half its width. At 1 a dome is a
/// hemisphere.
pub(super) const HEIGHT_RANGE: std::ops::RangeInclusive<f32> = 0.02..=1.0;

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
#[derive(Debug)]
pub(super) struct StudioState {
    /// The operator in use, or `None` for the classic brush's own.
    pub op: Option<FieldOp>,
    /// How far the brush trails the pointer, in pixels. Zero is off.
    pub lazy_px: f32,
    /// How much of the brush's size pressure decides.
    pub size_from_pressure: f32,
    /// Where the lazy brush is, in pixels of the viewport.
    anchor: Vec2,
    /// Whether a drag on the model presses the stamp in.
    pub stamp_on: bool,
    /// The stamp image, and what to call it.
    pub alpha: Arc<Alpha>,
    pub alpha_name: String,
    /// The built-in the image is, or `None` for one read from a file.
    pub built_in: Option<BuiltIn>,
    /// How tall the stamp is, as a share of half its width.
    pub stamp_height: f32,
    /// The stamp being placed, and where the press that began it landed:
    /// the point on the surface and the surface's normal there.
    gesture: StampGesture,
    press: Option<(Vec3, Vec3)>,
}

impl Default for StudioState {
    fn default() -> Self {
        Self {
            op: None,
            lazy_px: 0.0,
            size_from_pressure: 0.0,
            anchor: Vec2::ZERO,
            stamp_on: false,
            alpha: Arc::new(Alpha::built_in(BuiltIn::Rivet)),
            alpha_name: BuiltIn::Rivet.label().to_string(),
            built_in: Some(BuiltIn::Rivet),
            stamp_height: 0.35,
            gesture: StampGesture::new(),
            press: None,
        }
    }
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

    /// Whether a drag on the model is a stamp and not a stroke.
    pub(super) fn stamping(&self) -> bool {
        self.stamp_on
    }

    /// Back to the classic brush: no studio operator, no stamp.
    pub(super) fn put_away(&mut self) {
        self.op = None;
        self.stamp_on = false;
    }

    /// Apply a change, and say what the status line should, if anything.
    fn set(&mut self, message: StudioMessage) -> Option<String> {
        match message {
            StudioMessage::OperatorChosen(op) => {
                // Only the operators that have no classic brush. One that
                // has is chosen as that brush, with its own key and its own
                // remembered strength.
                self.op = op.filter(|op| FieldOp::STUDIO.contains(op));
                if self.op.is_some() {
                    self.stamp_on = false;
                }
            }
            StudioMessage::LazyChanged(pixels) => self.lazy_px = clamped(pixels, LAZY_RANGE),
            StudioMessage::SizeFromPressureChanged(share) => {
                self.size_from_pressure = clamped(share, SIZE_RANGE);
            }
            StudioMessage::StampToggled(on) => {
                self.stamp_on = on;
                if on {
                    self.op = None;
                    return Some(
                        "stamp: press where it goes, drag to size and turn it".to_string(),
                    );
                }
            }
            StudioMessage::StampChosen(kind) => {
                self.alpha = Arc::new(Alpha::built_in(kind));
                self.alpha_name = kind.label().to_string();
                self.built_in = Some(kind);
            }
            StudioMessage::StampHeightChanged(share) => {
                self.stamp_height = clamped(share, HEIGHT_RANGE);
            }
            // The dialog is opened by `Brokkr::studio_changed`, which is the
            // one that can return a task.
            StudioMessage::StampImageRequested | StudioMessage::StampImageLoaded(None) => {}
            StudioMessage::StampImageLoaded(Some(Ok(loaded))) => {
                self.alpha = loaded.alpha;
                self.alpha_name = loaded.name;
                self.built_in = None;
                return Some(format!("stamp image: {}", self.alpha_name));
            }
            StudioMessage::StampImageLoaded(Some(Err(why))) => {
                return Some(format!("that picture cannot be a stamp: {why}"));
            }
        }
        None
    }
}

/// Read a picture as a stamp image: its brightness is its height.
///
/// Sixteen bits of grey where the file has them, because eight is 256 steps
/// and a stamp a centimetre tall would have terraces 40 microns high, which
/// a resin printer prints.
pub(super) fn read_stamp_image(path: &std::path::Path) -> Result<LoadedStamp, String> {
    let picture = image::open(path).map_err(|error| error.to_string())?.into_luma16();
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    let heights: Vec<f32> =
        picture.as_raw().iter().map(|level| f32::from(*level) / f32::from(u16::MAX)).collect();
    let alpha = Alpha::from_heights(width, height, &heights).map_err(|error| error.to_string())?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "picture".to_string());
    Ok(LoadedStamp { name, alpha: Arc::new(alpha) })
}

/// Ask for a picture and read it. `None` when the dialog is dismissed.
async fn pick_stamp_image() -> Option<Result<LoadedStamp, String>> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Stamp image")
        .add_filter("Picture", &["png", "jpg", "jpeg"])
        .pick_file()
        .await?;
    Some(read_stamp_image(handle.path()))
}

/// Into a slider's range, with a NaN sent to the bottom of it.
fn clamped(value: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if value.is_nan() { *range.start() } else { value.clamp(*range.start(), *range.end()) }
}

impl Brokkr {
    pub(super) fn studio_changed(&mut self, message: StudioMessage) -> Task<Message> {
        if message == StudioMessage::StampImageRequested {
            return Task::perform(pick_stamp_image(), |loaded| {
                Message::Studio(StudioMessage::StampImageLoaded(loaded))
            });
        }
        if let Some(status) = self.studio.set(message) {
            self.status = status;
        }
        Task::none()
    }

    /// One pointer event of a stamp: the press fixes where it goes, and
    /// every move after that presses it again at the size and the turn the
    /// drag now says, from the surface as it was at the press.
    ///
    /// The size is how far the pointer has gone from the press, measured in
    /// the plane of the surface there and not on the screen, so a stamp on
    /// a face seen at a slant is not squashed. The top of the image points
    /// along the drag. The inverted direction engraves.
    pub(super) fn stamp_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        if start {
            self.studio.press = None;
            let Some(point) = self.surface_under(pixel) else {
                return;
            };
            let Some(normal) = self.doc.active_volume().gradient_world(point).try_normalize()
            else {
                return;
            };
            self.arm_recorder();
            self.studio.gesture.begin();
            self.studio.press = Some((point, normal));
            return;
        }
        let Some((centre, normal)) = self.studio.press else {
            return;
        };
        let (origin, ray) = self.ray_through(pixel);
        let facing = ray.dot(normal);
        if facing.abs() < 1.0e-3 {
            // The surface is edge on to the pointer: there is no telling
            // how far along it the drag has gone.
            return;
        }
        let drag = origin + ray * ((centre - origin).dot(normal) / facing) - centre;
        let radius = drag.length();
        let placement = Placement {
            centre,
            normal,
            up: drag,
            // A radius of nothing, which is a pointer back on the press,
            // is refused by the gesture, and the refusal puts back whatever
            // a longer drag had pressed.
            radius,
            height: radius * self.studio.stamp_height * direction.outward_sign(),
        };
        let studio = &mut self.studio;
        let pressed = studio.gesture.apply(
            self.doc.active_volume_mut(),
            &studio.alpha,
            &placement,
            self.symmetry,
            super::MIRROR_CENTRE,
        );
        if pressed == Err(StampError::TooBig) {
            self.status = "that stamp is too big to press: drag back in".to_string();
        }
        self.remesh_dirty();
    }

    /// The operator a field stamp runs with: the studio one, unless shift is
    /// held, which smooths whatever is chosen.
    fn live_operator(&self) -> Option<FieldOp> {
        self.studio.op.filter(|_| !self.shift)
    }

    /// What the brush in hand is called, for the panel's heading.
    pub(super) fn live_brush_name(&self) -> String {
        if self.studio.stamping() {
            return "STAMP".to_string();
        }
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
        let stamping = self.studio.stamp_on;
        let operators = operators.push(
            button(text("Stamp").size(theme::CAPTION_SIZE))
                .width(Length::Fill)
                .style(if stamping { theme::tool_button_active } else { theme::tool_button })
                .on_press(changed(StudioMessage::StampToggled(!stamping))),
        );
        let mut stamp = column![].spacing(theme::S1);
        if stamping {
            for kinds in BuiltIn::ALL.chunks(3) {
                stamp =
                    stamp.push(kinds.iter().fold(row![].spacing(theme::S1), |assembled, kind| {
                        let live = self.studio.built_in == Some(*kind);
                        assembled.push(
                            button(text(kind.label()).size(theme::CAPTION_SIZE))
                                .width(Length::Fill)
                                .style(if live {
                                    theme::tool_button_active
                                } else {
                                    theme::tool_button
                                })
                                .on_press(changed(StudioMessage::StampChosen(*kind))),
                        )
                    }));
            }
            stamp = stamp
                .push(
                    button(
                        text(if self.studio.built_in.is_none() {
                            format!("Picture: {}", self.studio.alpha_name)
                        } else {
                            "A picture…".to_string()
                        })
                        .size(theme::CAPTION_SIZE),
                    )
                    .width(Length::Fill)
                    .style(if self.studio.built_in.is_none() {
                        theme::tool_button_active
                    } else {
                        theme::tool_button
                    })
                    .on_press(changed(StudioMessage::StampImageRequested)),
                )
                .push(reading(format!("Height  {:.2} of its size", self.studio.stamp_height)))
                .push(
                    slider(HEIGHT_RANGE, self.studio.stamp_height, move |share| {
                        changed(StudioMessage::StampHeightChanged(share))
                    })
                    .step(0.01_f32),
                )
                .push(
                    text("press where it goes, drag to size and turn it")
                        .size(theme::CAPTION_SIZE)
                        .color(theme::TEXT_MUTE),
                );
        }
        column![
            text("STUDIO").size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE),
            operators,
            stamp,
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

    /// Whether two applications' active bodies hold the same field, to the
    /// bit.
    fn same_field(one: &Brokkr, other: &Brokkr) -> bool {
        let (a, b) = (one.doc.active_volume(), other.doc.active_volume());
        let (low, high) = a.world_bounds().expect("the body has bricks");
        let (lo, hi) = a.voxel_bounds(low - Vec3::splat(8.0), high + Vec3::splat(8.0));
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = glam::IVec3::new(x, y, z);
                    if a.sample_voxel(voxel).to_bits() != b.sample_voxel(voxel).to_bits() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// How far the surface under the middle of the viewport is from where a
    /// fresh ball has it, in millimetres, outward positive, found by walking
    /// along the line to the camera until the field changes sign.
    ///
    /// `shift_under_the_stroke` reads the field at one point, and the field
    /// stops at three voxels: it cannot tell a stamp one millimetre tall
    /// from one three millimetres tall.
    fn height_under_the_press(app: &Brokkr) -> f32 {
        let toward_camera = (self::app().camera.eye() - Vec3::ZERO).normalize();
        let volume = app.doc.active_volume();
        let inside = |along: f32| {
            volume.sample_world(toward_camera * (super::super::MODEL_RADIUS_MM + along)) < 0.0
        };
        // From well inside, outward, to the first point that is not solid.
        let mut along = -8.0;
        while along < 20.0 && inside(along) {
            along += 0.02;
        }
        along
    }

    /// A stamp dragged out to each of `pixels` in turn and let go.
    fn stamped(pixels: &[f32], configure: impl Fn(&mut Brokkr)) -> Brokkr {
        let mut app = app();
        update(&mut app, Message::Studio(StudioMessage::StampToggled(true)));
        configure(&mut app);
        press(&mut app);
        for at in pixels {
            drag_by(&mut app, *at);
        }
        release(&mut app);
        app
    }

    /// The stamp through the whole application: a drag presses the image
    /// in, a longer drag presses a bigger one, a drag out and back is the
    /// stamp it ended as, inverted it engraves, and it is one undo.
    #[test]
    fn a_stamp_drag_presses_the_image_in_at_the_size_of_the_drag() {
        let small = stamped(&[30.0], |_| {});
        assert_eq!(small.live_brush_name(), "STAMP");
        let (small_moved, small_rise) = (moved_voxels(&small), height_under_the_press(&small));
        assert!(small_moved > 200, "a stamp moved only {small_moved} voxels");
        assert!(small_rise > 0.2, "the middle of a rivet rose by {small_rise} mm");
        assert!(small.unsaved, "a stamp did not dirty the document");

        // Twice the drag is twice the stamp: four times the area, and twice
        // as tall, since the height is a share of the size.
        let large = stamped(&[60.0], |_| {});
        assert!(
            moved_voxels(&large) > small_moved * 3,
            "twice the drag moved {} voxels against {small_moved}",
            moved_voxels(&large)
        );
        let large_rise = height_under_the_press(&large);
        assert!(
            (large_rise / small_rise - 2.0).abs() < 0.25,
            "twice the drag rose {large_rise} mm against {small_rise}"
        );

        // Out and back: only where the drag ended counts. To the bit for a
        // drag that stays small, which is the core's own guarantee
        // (`a_drag_replaces_the_stamp_and_never_presses_it_in_twice`).
        let wandered = stamped(&[10.0, 55.0, 30.0], |_| {});
        assert!(same_field(&wandered, &small), "a stamp remembers the sizes it passed through");
        // And to within the pointer's own reading for one that goes far out
        // first. Not to the bit, and the stamp is not why: a stamp 7 mm
        // across changes how far the model reaches, the view follows that,
        // and the same pixel is then a ray two parts in ten thousand
        // different. Measured: a radius of 2.4912 mm where it was 2.4917.
        let far = stamped(&[90.0, 30.0], |_| {});
        let (far_moved, far_rise) = (moved_voxels(&far), height_under_the_press(&far));
        assert!(
            far_moved.abs_diff(small_moved) * 50 < small_moved,
            "after a long drag back in, {far_moved} voxels moved against {small_moved}"
        );
        assert!((far_rise - small_rise).abs() < 0.03, "{far_rise} mm against {small_rise}");

        // Taller when told to be, and engraved when inverted.
        let tall = stamped(&[30.0], |app| {
            update(app, Message::Studio(StudioMessage::StampHeightChanged(0.7)))
        });
        let tall_rise = height_under_the_press(&tall);
        assert!(
            (tall_rise / small_rise - 2.0).abs() < 0.25,
            "at twice the height setting the stamp rose {tall_rise} mm against {small_rise}"
        );
        let engraved = stamped(&[30.0], |app| app.control = true);
        let sunk = height_under_the_press(&engraved);
        assert!(
            (sunk + small_rise).abs() < small_rise * 0.15,
            "an inverted stamp put the middle at {sunk} mm where upright it rose {small_rise}"
        );

        // One undo takes the whole gesture off.
        let mut undone = small;
        update(&mut undone, Message::Undo);
        assert_eq!(moved_voxels(&undone), 0, "a stamp was not one undo");

        // A press with no drag presses nothing and leaves nothing to undo.
        let pressed_only = stamped(&[], |_| {});
        assert_eq!(moved_voxels(&pressed_only), 0);
        assert!(!pressed_only.unsaved, "a press that stamped nothing dirtied the document");
    }

    /// Each built-in is its own shape on the model, and a picture is a
    /// stamp too.
    #[test]
    fn the_stamp_image_chosen_is_the_one_pressed() {
        let with = |kind: BuiltIn| {
            stamped(&[40.0], |app| update(app, Message::Studio(StudioMessage::StampChosen(kind))))
        };
        let (rivet, star) = (with(BuiltIn::Rivet), with(BuiltIn::Star));
        assert!(!same_field(&rivet, &star), "a star was pressed as a rivet");
        assert_eq!(star.studio.built_in, Some(BuiltIn::Star));
        assert_eq!(star.studio.alpha_name, "Star");

        // A picture, written here and read back by the same function the
        // dialog uses: white in the top right quarter, black elsewhere.
        let directory = std::env::temp_dir().join(format!("brokkr-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        let path = directory.join("corner mark.png");
        // And mid grey in the bottom left, at a level only sixteen bits
        // can hold: 32768 is half way, and read as eight bits it is white.
        let picture = image::ImageBuffer::from_fn(32, 32, |x, y| {
            image::Luma([if x >= 16 && y < 16 {
                u16::MAX
            } else if x < 16 && y >= 16 {
                32768
            } else {
                0
            }])
        });
        picture.save(&path).expect("writing the fixture");
        let loaded = read_stamp_image(&path).expect("a PNG is a stamp");
        assert_eq!(loaded.name, "corner mark");
        assert!(loaded.alpha.sample(0.5, 0.5) > 0.9, "the white quarter is not top right");
        assert_eq!(loaded.alpha.sample(-0.5, 0.5), 0.0);
        assert_eq!(loaded.alpha.sample(0.5, -0.5), 0.0);
        let grey = loaded.alpha.sample(-0.5, -0.5);
        assert!((grey - 0.5).abs() < 0.001, "half grey was read as a height of {grey}");

        let mut app = app();
        update(
            &mut app,
            Message::Studio(StudioMessage::StampImageLoaded(Some(Ok(loaded.clone())))),
        );
        assert_eq!(app.studio.built_in, None);
        assert_eq!(app.studio.alpha_name, "corner mark");
        assert!(Arc::ptr_eq(&app.studio.alpha, &loaded.alpha), "the picture was not taken up");
        assert!(app.status.contains("corner mark"), "{}", app.status);
        drop(app.studio_panel());

        // A file that is not a picture, and one that is not there, are each
        // refused with a reason and change nothing.
        let not_a_picture = directory.join("notes.png");
        std::fs::write(&not_a_picture, b"this is not a picture").unwrap();
        for path in [not_a_picture, directory.join("missing.png")] {
            let refused = read_stamp_image(&path);
            let why = refused.expect_err("something that is not a picture was read as one");
            assert!(!why.is_empty());
            update(&mut app, Message::Studio(StudioMessage::StampImageLoaded(Some(Err(why)))));
            assert!(app.status.contains("cannot be a stamp"), "{}", app.status);
            assert_eq!(app.studio.alpha_name, "corner mark", "a refusal replaced the stamp");
        }
        // A dismissed dialog is nothing at all.
        let status = app.status.clone();
        update(&mut app, Message::Studio(StudioMessage::StampImageLoaded(None)));
        assert_eq!(app.status, status);
        std::fs::remove_dir_all(&directory).ok();
    }

    /// The stamp, a studio operator and a classic brush are three things to
    /// have in hand, and taking one up puts the others away.
    #[test]
    fn the_stamp_an_operator_and_a_classic_brush_each_put_the_others_away() {
        let mut app = app();
        let set = |app: &mut Brokkr, message| update(app, Message::Studio(message));
        set(&mut app, StudioMessage::OperatorChosen(Some(FieldOp::Crease)));
        set(&mut app, StudioMessage::StampToggled(true));
        assert!(app.studio.stamping() && app.studio.op.is_none(), "the stamp kept the crease");
        assert!(app.status.contains("stamp"), "{}", app.status);
        set(&mut app, StudioMessage::OperatorChosen(Some(FieldOp::Crease)));
        assert!(!app.studio.stamping() && app.studio.op == Some(FieldOp::Crease));
        set(&mut app, StudioMessage::StampToggled(true));
        update(&mut app, Message::BrushKindChanged(BrushKind::Smooth));
        assert!(!app.studio.stamping(), "choosing a classic brush left the stamp up");
        assert_eq!(app.live_brush_name(), "SMOOTH");
        // Toggled off by its own button, a stroke is a stroke again.
        set(&mut app, StudioMessage::StampToggled(true));
        set(&mut app, StudioMessage::StampToggled(false));
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        stroke(&mut app);
        assert!(app.perf.stamps > 0, "with the stamp put away a drag laid no brush stamps");

        set(&mut app, StudioMessage::StampHeightChanged(5.0));
        assert_eq!(app.studio.stamp_height, *HEIGHT_RANGE.end());
        set(&mut app, StudioMessage::StampHeightChanged(f32::NAN));
        assert_eq!(app.studio.stamp_height, *HEIGHT_RANGE.start());
        set(&mut app, StudioMessage::StampToggled(true));
        drop(app.studio_panel());
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
