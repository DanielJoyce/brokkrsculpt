// SPDX-License-Identifier: AGPL-3.0-only

//! Reading sizes off the screen: a scale bar, and the distance between two
//! points.
//!
//! `docs/STUDIO-PLAN.md` section 17. Two small things a model made to be
//! printed at a size wants, and that the viewport did not have:
//!
//! * **A scale bar** in the bottom left of the viewport: a bar, and the
//!   round number of millimetres it stands for, which changes as the view
//!   is zoomed, as a map's does.
//! * **A measure.** *Measure*, then two clicks on the model: the straight
//!   line between the two points is drawn, with its length on it.
//!
//! # What the scale bar measures
//!
//! Under a perspective view a millimetre is not one size everywhere on the
//! screen: nearer the eye it is bigger. The bar stands for its length **at
//! the point the view turns about** ([`crate::camera::OrbitCamera::target`]),
//! which is where the model being looked at is, and across the view, which
//! is the one direction in which a length there is not foreshortened. It
//! is right for whatever is at that depth and a little out for what is
//! nearer or further, and it says "at the middle of the view" under it so
//! that nobody takes it for more.
//!
//! The measure has no such caveat: it is the distance between two points
//! in the model's own millimetres, whatever the view.
//!
//! # Drawn over the viewport, in the application
//!
//! One transparent layer over the viewport, drawn by iced and not by the
//! renderer: a canvas, which the icons already cost
//! (`crates/brokkr-app/Cargo.toml`). The renderer's own overlay is tested
//! against the model's depth, and the straight line between two points on
//! a ball is inside the ball. A line across the screen is what is wanted:
//! always in front, a constant width, with its length written on it.
//!
//! The layer takes no pointer events (a canvas whose program has no
//! `update` and no `mouse_interaction` does not), so a drag that begins on
//! top of the scale bar still sculpts.
//!
//! # A child module, and what `app.rs` holds for it
//!
//! Decision D6, add and do not modify: one field, one message arm, and two
//! lines in the pointer handler, one that lets a click while measuring
//! take a point and not start a stroke, and one that moves the loose end
//! of the line with the pointer.

use glam::{Vec2, Vec3};
use iced::widget::{button, canvas, column, container, row, text};
use iced::{Alignment, Element, Length, Point};

use super::Brokkr;
use crate::camera::OrbitCamera;
use crate::message::{MeasureMessage, Message, PointerButton};
use crate::theme;

/// The longest the scale bar is drawn, in pixels. The bar is the longest
/// round number of millimetres that fits in this.
const MOST_BAR_PX: f32 = 140.0;

/// How far the scale bar is from the left of the viewport, in pixels, and
/// how far its line is from the bottom: clear of the Measure button under
/// it.
const BAR_LEFT: f32 = 18.0;
const BAR_UP: f32 = 64.0;

/// How wide the words under the scale bar are, in pixels: what its plate
/// is at least as wide as.
const PLATE_WORDS: f32 = 152.0;

/// How tall the ticks at the ends of the scale bar are, in pixels.
const TICK: f32 = 5.0;

/// What the application remembers of a measurement.
#[derive(Debug, Default)]
pub(super) struct MeasureState {
    /// The next left clicks on the model are points of a measurement and
    /// not strokes.
    pub armed: bool,
    /// The first point clicked.
    pub from: Option<Vec3>,
    /// The second, once it has been.
    pub to: Option<Vec3>,
    /// With one point down, the model under the pointer: the loose end of
    /// the line.
    pub hover: Option<Vec3>,
}

impl MeasureState {
    /// The two ends of the line to draw, and whether the second is fixed.
    fn line(&self) -> Option<(Vec3, Vec3, bool)> {
        let from = self.from?;
        match (self.to, self.hover) {
            (Some(to), _) => Some((from, to, true)),
            (None, Some(loose)) => Some((from, loose, false)),
            (None, None) => None,
        }
    }

    /// The distance measured, once both points are down.
    pub(super) fn distance(&self) -> Option<f32> {
        Some(self.from?.distance(self.to?))
    }
}

/// The length a scale bar stands for, in millimetres, and how long it is
/// drawn, in pixels: the longest of 1, 2 or 5 times a power of ten that is
/// no longer than `most_px` on a screen where a millimetre is `px_per_mm`
/// pixels. Nothing when a millimetre has no size on the screen.
pub(super) fn scale(px_per_mm: f32, most_px: f32) -> Option<(f32, f32)> {
    if !(px_per_mm.is_finite() && px_per_mm > 0.0 && most_px.is_finite() && most_px > 0.0) {
        return None;
    }
    let most_mm = most_px / px_per_mm;
    let exponent = most_mm.log10().floor();
    // A power of ten worked out by `powf` can come out a hair over the
    // number its exponent was the floor of the logarithm of. One step down
    // is then the power wanted.
    let mut decade = 10.0_f32.powf(exponent);
    if decade > most_mm {
        decade /= 10.0;
    }
    let millimetres =
        [5.0, 2.0, 1.0].into_iter().map(|step| step * decade).find(|mm| *mm <= most_mm)?;
    let pixels = millimetres * px_per_mm;
    (millimetres.is_finite() && millimetres > 0.0 && pixels.is_finite())
        .then_some((millimetres, pixels))
}

/// A round length as the scale bar says it: `5 mm`, `0.2 mm`, `200 mm`.
pub(super) fn round_length(millimetres: f32) -> String {
    if millimetres >= 1.0 {
        format!("{millimetres:.0} mm")
    } else {
        // As many places as the number needs: 0.5, 0.02, 0.001.
        let places = (-millimetres.log10()).ceil().clamp(1.0, 6.0) as usize;
        format!("{millimetres:.places$} mm")
    }
}

/// A measured length as it is read out: `12.34 mm`.
pub(super) fn measured_length(millimetres: f32) -> String {
    format!("{millimetres:.2} mm")
}

/// Where a point of the model is on a viewport of `size` pixels, or
/// nothing for a point behind the eye.
///
/// The other way round from [`OrbitCamera::ray`]:
/// `a_point_is_drawn_where_its_ray_came_from` holds the two together.
pub(super) fn to_pixels(camera: &OrbitCamera, point: Vec3, size: Vec2) -> Option<Vec2> {
    let clip = camera.view_projection(size.x / size.y.max(1.0)) * point.extend(1.0);
    if clip.w <= 1.0e-6 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(Vec2::new((ndc.x + 1.0) * 0.5 * size.x, (1.0 - ndc.y) * 0.5 * size.y))
}

/// How many pixels a millimetre is at the point the view turns about,
/// measured across the view.
pub(super) fn px_per_mm(camera: &OrbitCamera, size: Vec2) -> Option<f32> {
    let at = to_pixels(camera, camera.target, size)?;
    let beside = to_pixels(camera, camera.target + camera.right(), size)?;
    Some(at.distance(beside))
}

/// What the layer over the viewport draws: the scale bar, and the line of
/// a measurement if there is one.
struct Rulers {
    camera: OrbitCamera,
    /// The two ends of the measured line, and whether the second is fixed.
    line: Option<(Vec3, Vec3, bool)>,
}

impl<Message> canvas::Program<Message> for Rulers {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let size = Vec2::new(bounds.width, bounds.height);
        let stroke = |colour: iced::Color, width: f32| canvas::Stroke {
            style: canvas::Style::Solid(colour),
            width,
            line_cap: canvas::LineCap::Round,
            ..canvas::Stroke::default()
        };
        let label = |content: String, at: Point, colour: iced::Color, size: f32| canvas::Text {
            content,
            position: at,
            color: colour,
            size: iced::Pixels(size),
            font: theme::MONO,
            ..canvas::Text::default()
        };

        // The scale bar: a line with a tick at each end, the length it
        // stands for over it, and where that length holds under it.
        if let Some((millimetres, pixels)) =
            px_per_mm(&self.camera, size).and_then(|px| scale(px, MOST_BAR_PX))
        {
            let (left, base) = (BAR_LEFT, bounds.height - BAR_UP);
            // On a dark plate: zoomed in, the corner of the viewport is
            // over the model, and the model is pale. Seen on a screen
            // without one, the bar could not be read.
            let plate = canvas::Path::rounded_rectangle(
                Point::new(left - 8.0, base - TICK - 22.0),
                iced::Size::new(pixels.max(PLATE_WORDS) + 16.0, TICK + 44.0),
                iced::border::Radius::new(4.0),
            );
            frame.fill(&plate, iced::Color { a: 0.72, ..theme::BG_DEEP });
            let bar = canvas::Path::new(|path| {
                path.move_to(Point::new(left, base - TICK));
                path.line_to(Point::new(left, base));
                path.line_to(Point::new(left + pixels, base));
                path.line_to(Point::new(left + pixels, base - TICK));
            });
            frame.stroke(&bar, stroke(theme::TEXT_DIM, 1.5));
            frame.fill_text(label(
                round_length(millimetres),
                Point::new(left, base - TICK - 16.0),
                theme::TEXT_DIM,
                theme::TEXT_SIZE_SMALL,
            ));
            frame.fill_text(label(
                "at the middle of the view".to_string(),
                Point::new(left, base + 4.0),
                theme::TEXT_MUTE,
                theme::CAPTION_SIZE,
            ));
        }

        // The measurement: the line, a dot at each end, and its length on
        // it once both ends are down.
        if let Some((from, to, fixed)) = self.line
            && let (Some(a), Some(b)) =
                (to_pixels(&self.camera, from, size), to_pixels(&self.camera, to, size))
        {
            let (a, b) = (Point::new(a.x, a.y), Point::new(b.x, b.y));
            let colour = if fixed { theme::ACCENT } else { theme::ACCENT_HOT };
            frame.stroke(&canvas::Path::line(a, b), stroke(colour, if fixed { 2.0 } else { 1.0 }));
            for end in [a, b] {
                frame.fill(&canvas::Path::circle(end, 3.5), colour);
            }
            let words = measured_length(from.distance(to));
            // On a dark plate, over the middle of the line, so that it
            // reads over a pale model.
            let middle = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0 - 22.0);
            let wide = words.chars().count() as f32 * 7.8 + 12.0;
            let plate = canvas::Path::rounded_rectangle(
                Point::new(middle.x - wide / 2.0, middle.y - 3.0),
                iced::Size::new(wide, 20.0),
                iced::border::Radius::new(4.0),
            );
            frame.fill(&plate, iced::Color { a: 0.85, ..theme::BG_DEEP });
            frame.fill_text(canvas::Text {
                align_x: iced::alignment::Horizontal::Center.into(),
                ..label(words, middle, theme::TEXT, 13.0)
            });
        }

        vec![frame.into_geometry()]
    }
}

impl Brokkr {
    /// Begin a measurement, put one away, or clear what was measured.
    pub(super) fn measure_changed(&mut self, message: MeasureMessage) {
        match message {
            MeasureMessage::Armed(true) => {
                self.measure = MeasureState { armed: true, ..MeasureState::default() };
                self.status = "measure: click two points on the model".to_string();
            }
            MeasureMessage::Armed(false) => {
                self.measure.armed = false;
                self.measure.hover = None;
                // Half a measurement is none.
                if self.measure.to.is_none() {
                    self.measure.from = None;
                }
            }
            MeasureMessage::Cleared => self.measure = MeasureState::default(),
        }
    }

    /// Escape: put away a measurement that is being taken, and say whether
    /// there was one. One that is finished stays on the screen.
    pub(super) fn measure_escaped(&mut self) -> bool {
        let taking = self.measure.armed;
        if taking {
            self.measure_changed(MeasureMessage::Armed(false));
            self.status = "measure put away".to_string();
        }
        taking
    }

    /// A press on the viewport while a measurement is being taken: whether
    /// the measurement took it, in which case it is not a stroke.
    ///
    /// Only the left button, and not on the navigation cube: the view can
    /// still be turned and panned between the two clicks, which on a model
    /// of any size is how the second point is reached.
    pub(super) fn measure_press(&mut self, button: PointerButton, pixel: Vec2) -> bool {
        if !self.measure.armed || button != PointerButton::Left {
            return false;
        }
        if crate::navcube::pick(&self.camera, self.viewport_size, pixel).is_some() {
            return false;
        }
        let Some((_, hit)) = self.pick(pixel) else {
            self.status = "measure: that is not on the model".to_string();
            return true;
        };
        match self.measure.from {
            None => {
                self.measure.from = Some(hit.position);
                self.status = "measure: and the second point".to_string();
            }
            Some(from) => {
                self.measure.to = Some(hit.position);
                self.measure.hover = None;
                self.measure.armed = false;
                self.status = format!("measured {}", measured_length(from.distance(hit.position)));
            }
        }
        true
    }

    /// The pointer moved: with one point of a measurement down, the loose
    /// end of its line follows the model under the pointer.
    pub(super) fn measure_hover(&mut self, pixel: Vec2) {
        if self.measure.armed && self.measure.from.is_some() {
            self.measure.hover = self.pick(pixel).map(|(_, hit)| hit.position);
        }
    }

    /// The layer over the viewport that draws the scale bar and the
    /// measured line. It takes no pointer events.
    pub(super) fn rulers(&self) -> Element<'_, Message> {
        canvas(Rulers { camera: self.camera, line: self.measure.line() })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// The Measure button, in the bottom left of the viewport under the
    /// scale bar, and beside it what was measured and a button to clear
    /// it.
    pub(super) fn measure_controls(&self) -> Element<'_, Message> {
        let armed = self.measure.armed;
        let mut controls = row![
            button(text("Measure").size(theme::CAPTION_SIZE))
                .padding(theme::S2)
                .style(if armed { theme::tool_button_active } else { theme::tool_button })
                .on_press(Message::Measure(MeasureMessage::Armed(!armed))),
        ]
        .spacing(theme::S2)
        .align_y(Alignment::Center);
        if armed {
            controls = controls.push(
                text(if self.measure.from.is_none() {
                    "click two points on the model"
                } else {
                    "and the second point"
                })
                .size(theme::CAPTION_SIZE)
                .color(theme::TEXT_DIM),
            );
        }
        if let Some(distance) = self.measure.distance() {
            controls = controls
                .push(
                    text(measured_length(distance))
                        .size(theme::TEXT_SIZE_SMALL)
                        .font(theme::MONO)
                        .color(theme::TEXT),
                )
                .push(
                    button(text("Clear").size(theme::CAPTION_SIZE))
                        .padding(theme::S2)
                        .style(theme::tool_button)
                        .on_press(Message::Measure(MeasureMessage::Cleared)),
                );
        }
        // On a card, so that the words read over a pale model. Neither it
        // nor the `Fill` container round it takes presses, as the one that
        // holds the bug button in the other corner does not: only the
        // buttons do, inside their own bounds. See `Brokkr::overlay`.
        let card = container(controls).padding(theme::S1).style(theme::overlay_card);
        container(column![card])
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Start)
            .align_y(Alignment::End)
            .padding(theme::S4)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::PointerEvent;

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    /// An application that has had a pointer event, and so knows how big
    /// its viewport is.
    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        move_to(&mut app, Vec2::new(5.0, 5.0));
        app
    }

    fn move_to(app: &mut Brokkr, pixel: Vec2) {
        let position = iced::Vector::new(pixel.x, pixel.y);
        update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
    }

    fn click(app: &mut Brokkr, button: PointerButton, pixel: Vec2) {
        let position = iced::Vector::new(pixel.x, pixel.y);
        update(app, Message::Pointer(PointerEvent::Pressed { button, position, size: SIZE }));
        update(app, Message::Pointer(PointerEvent::Released { button }));
    }

    fn same_field(one: &Brokkr, other: &Brokkr) -> bool {
        let (a, b) = (one.doc.active_volume(), other.doc.active_volume());
        let (low, high) = a.world_bounds().expect("the body has bricks");
        let (lo, hi) = a.voxel_bounds(low - Vec3::splat(4.0), high + Vec3::splat(4.0));
        (lo.z..=hi.z).all(|z| {
            (lo.y..=hi.y).all(|y| {
                (lo.x..=hi.x).all(|x| {
                    let voxel = glam::IVec3::new(x, y, z);
                    a.sample_voxel(voxel).to_bits() == b.sample_voxel(voxel).to_bits()
                })
            })
        })
    }

    /// Two clicks on the model with the measure armed are the two ends of
    /// a line, its length is the distance between them, and neither click
    /// is a stroke. After the second the pointer is the brush's again.
    #[test]
    fn two_clicks_measure_the_distance_between_two_points_and_sculpt_nothing() {
        let mut app = app();
        let (first, second) = (Vec2::new(330.0, 260.0), Vec2::new(480.0, 350.0));
        let on_the_model = |app: &Brokkr, pixel: Vec2| {
            app.pick(pixel).map(|(_, hit)| hit.position).expect("the model is under the pixel")
        };
        let (a, b) = (on_the_model(&app, first), on_the_model(&app, second));
        assert!(a.distance(b) > 5.0, "the fixture's two points are {} mm apart", a.distance(b));

        update(&mut app, Message::Measure(MeasureMessage::Armed(true)));
        assert!(app.measure.armed && app.measure.line().is_none());
        assert_eq!(app.status, "measure: click two points on the model");
        drop(app.measure_controls());

        click(&mut app, PointerButton::Left, first);
        assert_eq!(app.measure.from, Some(a));
        assert!(app.measure.armed && app.measure.distance().is_none());
        // The loose end follows the pointer, over the model and off it.
        move_to(&mut app, second);
        assert_eq!(app.measure.line(), Some((a, b, false)));
        move_to(&mut app, Vec2::new(4.0, 4.0));
        assert_eq!(app.measure.line(), None, "a line was drawn to a point off the model");
        drop(app.measure_controls());
        drop(app.rulers());

        click(&mut app, PointerButton::Left, second);
        assert_eq!(app.measure.line(), Some((a, b, true)));
        assert_eq!(app.measure.distance(), Some(a.distance(b)));
        assert_eq!(app.status, format!("measured {}", measured_length(a.distance(b))));
        assert!(!app.measure.armed, "the measure stayed armed after its second point");
        drop(app.measure_controls());
        drop(app.rulers());

        // Neither click was a stroke: the model is as it was, and there is
        // nothing to undo.
        assert!(same_field(&app, &self::app()), "a click of the measure sculpted");
        assert!(!app.history.can_undo(), "a click of the measure left something to undo");

        // The next click is the brush's, and the line stays where it is.
        click(&mut app, PointerButton::Left, first);
        assert!(!same_field(&app, &self::app()), "after the measure, a click did not sculpt");
        assert_eq!(app.measure.distance(), Some(a.distance(b)));
        // The pointer moving no longer moves the line's end.
        move_to(&mut app, first);
        assert_eq!(app.measure.line(), Some((a, b, true)));

        update(&mut app, Message::Measure(MeasureMessage::Cleared));
        assert!(app.measure.line().is_none() && app.measure.distance().is_none());
    }

    /// A click off the model while measuring is no point and no stroke;
    /// the other buttons still turn the view; putting the measure away
    /// with one point down leaves no half of a line; and beginning again
    /// begins from nothing.
    #[test]
    fn a_measurement_can_be_put_away_and_begun_again() {
        let mut app = app();
        let on = Vec2::new(400.0, 300.0);
        update(&mut app, Message::Measure(MeasureMessage::Armed(true)));
        click(&mut app, PointerButton::Left, Vec2::new(3.0, 3.0));
        assert_eq!(app.measure.from, None);
        assert!(app.measure.armed);
        assert_eq!(app.status, "measure: that is not on the model");

        // The right button is not the measure's: it turns the view.
        assert!(!app.measure_press(PointerButton::Right, on));
        assert!(!app.measure_press(PointerButton::Middle, on));
        assert_eq!(app.measure.from, None);

        click(&mut app, PointerButton::Left, on);
        assert!(app.measure.from.is_some());
        update(&mut app, Message::Measure(MeasureMessage::Armed(false)));
        assert!(!app.measure.armed && app.measure.from.is_none() && app.measure.line().is_none());
        // Not armed, a press is not the measure's.
        assert!(!app.measure_press(PointerButton::Left, on));

        // A finished one put away stays on the screen; begun again, it
        // goes.
        update(&mut app, Message::Measure(MeasureMessage::Armed(true)));
        click(&mut app, PointerButton::Left, on);
        click(&mut app, PointerButton::Left, Vec2::new(450.0, 330.0));
        update(&mut app, Message::Measure(MeasureMessage::Armed(false)));
        assert!(app.measure.distance().is_some());
        update(&mut app, Message::Measure(MeasureMessage::Armed(true)));
        assert!(app.measure.armed && app.measure.from.is_none() && app.measure.to.is_none());
    }

    /// The scale bar stands for fewer millimetres as the view is zoomed in
    /// and more as it is zoomed out, and is always a length that fits.
    #[test]
    fn the_scale_bar_follows_the_zoom() {
        let app = app();
        let size = Vec2::new(SIZE.x, SIZE.y);
        let bar = |camera: &OrbitCamera| {
            scale(px_per_mm(camera, size).expect("the middle of the view is on screen"), 140.0)
                .expect("a millimetre has a size on the screen")
        };
        let (as_it_is, _) = bar(&app.camera);
        let mut near = app.camera;
        near.zoom_by(0.2);
        let mut far = app.camera;
        far.zoom_by(5.0);
        let ((close, close_px), (wide, wide_px)) = (bar(&near), bar(&far));
        assert!(close < as_it_is && as_it_is < wide, "{close} mm, {as_it_is} mm, {wide} mm");
        assert!(close_px <= 140.0 && wide_px <= 140.0);
        // A millimetre across the middle of the view is as many pixels as
        // the two points a millimetre apart there are apart on the screen.
        let per = px_per_mm(&app.camera, size).expect("on screen");
        let (a, b) = (app.camera.target, app.camera.target + app.camera.right() * 10.0);
        let apart = to_pixels(&app.camera, a, size)
            .zip(to_pixels(&app.camera, b, size))
            .map(|(a, b)| a.distance(b))
            .expect("on screen");
        assert!((apart - per * 10.0).abs() < 0.01, "{apart} px for 10 mm at {per} px a mm");
        // The middle of the view is drawn at the middle of the viewport.
        let middle = to_pixels(&app.camera, app.camera.target, size).expect("on screen");
        assert!((middle - size / 2.0).length() < 0.01, "{middle}");
        // And a point behind the eye is drawn nowhere.
        let behind = app.camera.eye() + (app.camera.eye() - app.camera.target);
        assert_eq!(to_pixels(&app.camera, behind, size), None);
    }

    /// Lengths as they are written.
    #[test]
    fn lengths_are_written_as_a_reader_expects() {
        assert_eq!(round_length(5.0), "5 mm");
        assert_eq!(round_length(200.0), "200 mm");
        assert_eq!(round_length(0.5), "0.5 mm");
        assert_eq!(round_length(0.02), "0.02 mm");
        assert_eq!(round_length(0.001), "0.001 mm");
        assert_eq!(measured_length(12.3456), "12.35 mm");
        assert_eq!(measured_length(0.0), "0.00 mm");
        // No size on the screen, no bar.
        for nothing in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(scale(nothing, 140.0), None);
            assert_eq!(scale(10.0, nothing), None);
        }
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Whatever the zoom and however long the bar may be: it
            /// stands for 1, 2 or 5 times a power of ten millimetres, is
            /// no longer than it may be, and is the longest such, which is
            /// to say more than two fifths of what it may be.
            #[test]
            fn the_scale_bar_is_a_round_length_that_fits(
                px_per_mm in 0.001_f32..5000.0,
                most_px in 40.0_f32..400.0,
            ) {
                let (millimetres, pixels) = scale(px_per_mm, most_px).expect("a bar");
                prop_assert!(pixels <= most_px * 1.0001, "{} px of {}", pixels, most_px);
                prop_assert!(pixels > most_px * 0.3999, "{} px of {}", pixels, most_px);
                prop_assert!((pixels - millimetres * px_per_mm).abs() <= pixels * 1.0e-5);
                let decade = 10.0_f32.powf(millimetres.log10().floor());
                let leading = millimetres / decade;
                prop_assert!(
                    [1.0_f32, 2.0, 5.0, 10.0].iter().any(|round| (leading - round).abs() < 1.0e-3),
                    "{} mm is not a round length",
                    millimetres
                );
                // And it can be written.
                prop_assert!(round_length(millimetres).ends_with(" mm"));
            }

            /// A point is drawn where its ray came from: a point along the
            /// ray through a pixel is drawn at that pixel.
            #[test]
            fn a_point_is_drawn_where_its_ray_came_from(
                x in 1.0_f32..799.0,
                y in 1.0_f32..599.0,
                along in 5.0_f32..300.0,
                turn in (-3.0_f32..3.0, -1.4_f32..1.4),
            ) {
                let size = Vec2::new(SIZE.x, SIZE.y);
                let mut camera = OrbitCamera::framing(Vec3::new(3.0, -2.0, 1.0), 32.0);
                camera.orbit_radians(Vec2::new(turn.0, turn.1));
                let pixel = Vec2::new(x, y);
                let (origin, ray) =
                    camera.ray(OrbitCamera::ndc_from_pixels(pixel, size), size.x / size.y);
                let drawn = to_pixels(&camera, origin + ray * along, size).expect("in front");
                prop_assert!((drawn - pixel).length() < 0.05, "{} drawn at {}", pixel, drawn);
            }
        }
    }
}
