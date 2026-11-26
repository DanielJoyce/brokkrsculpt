// SPDX-License-Identifier: AGPL-3.0-only

//! Revolve: a vertical line on the screen through the point the view turns
//! about, a line drawn beside it with the left button, and the solid that
//! line makes spun round it, added as a body of its own.
//!
//! `brokkr_core::revolve` has the geometry. Here are the button in REVOLVE
//! (Transform), the layer that draws the axis and what is being drawn (its
//! mirror faint on the other side, which is the half of the solid the user
//! is not drawing), and the press, move and release that record it.
//!
//! # Where the axis is in the world
//!
//! On the screen it is upright through the view's target. In the world it
//! is the camera's up through the target, and the line drawn lies in the
//! plane through the target that faces the camera: each pixel is taken onto
//! that plane, and its distance across and along the axis there is the
//! profile, in millimetres. So what is drawn is what is made, at the size it
//! was drawn, and turning the view first chooses which way the axis lies.
//!
//! The view can still be turned and panned with the other buttons while the
//! tool is out; the axis follows the target.

use glam::Vec2;
use iced::widget::{canvas, column, row};
use iced::{Element, Length, Point};

use super::Brokkr;
use super::measure::to_pixels;
use crate::camera::OrbitCamera;
use crate::message::{Message, PointerButton, RevolveMessage};
use crate::theme;

/// The fewest pixels between two points kept while drawing.
const MIN_STEP_PX: f32 = 2.0;

/// What the application remembers of a revolve.
#[derive(Debug, Default, Clone)]
pub(super) struct RevolveState {
    /// The axis is up and the next left drag draws a profile.
    pub armed: bool,
    /// The pixels of the line being drawn, while the button is down.
    pub drawn: Vec<Vec2>,
    pub drawing: bool,
}

/// The layer that draws the axis and the line.
struct AxisLayer {
    axis_x: f32,
    drawn: Vec<Vec2>,
}

impl<Message> canvas::Program<Message> for AxisLayer {
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
        let stroke = |colour: iced::Color, width: f32, dashed: bool| canvas::Stroke {
            style: canvas::Style::Solid(colour),
            width,
            line_cap: canvas::LineCap::Round,
            line_dash: if dashed {
                canvas::LineDash { segments: &[8.0, 6.0], offset: 0 }
            } else {
                canvas::LineDash::default()
            },
            ..canvas::Stroke::default()
        };
        let axis = canvas::Path::line(
            Point::new(self.axis_x, 0.0),
            Point::new(self.axis_x, bounds.height),
        );
        frame.stroke(&axis, stroke(theme::ACCENT, 1.5, true));
        if self.drawn.len() >= 2 {
            let line = |mirror: bool| {
                canvas::Path::new(|path| {
                    for (index, pixel) in self.drawn.iter().enumerate() {
                        let x = if mirror { 2.0 * self.axis_x - pixel.x } else { pixel.x };
                        let at = Point::new(x, pixel.y);
                        if index == 0 { path.move_to(at) } else { path.line_to(at) }
                    }
                })
            };
            frame.stroke(&line(false), stroke(theme::TEXT, 2.0, false));
            frame.stroke(&line(true), stroke(theme::TEXT_DIM, 1.0, false));
        }
        vec![frame.into_geometry()]
    }
}

impl Brokkr {
    pub(super) fn revolve_changed(&mut self, message: RevolveMessage) {
        match message {
            RevolveMessage::Armed(armed) => {
                self.revolve = RevolveState { armed, ..RevolveState::default() };
                if armed {
                    self.status = "revolve: draw a line beside the axis".to_string();
                }
            }
        }
    }

    /// Escape: put the axis away, and say whether it was up.
    pub(super) fn revolve_escaped(&mut self) -> bool {
        let armed = self.revolve.armed;
        if armed {
            self.revolve = RevolveState::default();
            self.status = "revolve put away".to_string();
        }
        armed
    }

    /// A press on the viewport: whether the revolve took it.
    pub(super) fn revolve_pressed(&mut self, button: PointerButton, pixel: Vec2) -> bool {
        if !self.revolve.armed || button != PointerButton::Left {
            return false;
        }
        if crate::navcube::pick(&self.camera, self.viewport_size, pixel).is_some() {
            return false;
        }
        self.revolve.drawn = vec![pixel];
        self.revolve.drawing = true;
        true
    }

    /// The pointer moved: the line being drawn follows it.
    pub(super) fn revolve_moved(&mut self, pixel: Vec2) {
        if !self.revolve.drawing {
            return;
        }
        let far_enough =
            self.revolve.drawn.last().is_none_or(|last| last.distance(pixel) >= MIN_STEP_PX);
        if far_enough {
            self.revolve.drawn.push(pixel);
        }
    }

    /// A release: the line drawn is spun into a body. Whether it was the
    /// revolve's.
    pub(super) fn revolve_released(&mut self, button: PointerButton) -> bool {
        if !self.revolve.drawing || button != PointerButton::Left {
            return false;
        }
        self.revolve.drawing = false;
        let pixels = std::mem::take(&mut self.revolve.drawn);
        self.revolve_drawn(&pixels);
        true
    }

    /// Where the axis is drawn across the viewport, in pixels.
    fn revolve_axis_x(&self) -> Option<f32> {
        to_pixels(&self.camera, self.camera.target, self.viewport_size).map(|at| at.x)
    }

    /// A pixel taken onto the plane through the target facing the camera, as
    /// millimetres across and along the axis.
    fn on_revolve_plane(camera: &OrbitCamera, size: Vec2, pixel: Vec2) -> Option<Vec2> {
        let ndc = OrbitCamera::ndc_from_pixels(pixel, size);
        let (origin, direction) = camera.ray(ndc, size.x / size.y.max(1.0));
        let facing = (camera.target - camera.eye()).try_normalize()?;
        let square = direction.dot(facing);
        if square.abs() < 1e-6 {
            return None;
        }
        let along_ray = (camera.target - origin).dot(facing) / square;
        let offset = origin + direction * along_ray - camera.target;
        Some(Vec2::new(offset.dot(camera.right()), offset.dot(camera.up())))
    }

    /// Spin a drawn line into a body.
    fn revolve_drawn(&mut self, pixels: &[Vec2]) {
        let started = web_time::Instant::now();
        let voxel_size = self.doc.voxel_size();
        let profile: Vec<Vec2> = pixels
            .iter()
            .filter_map(|pixel| Self::on_revolve_plane(&self.camera, self.viewport_size, *pixel))
            .collect();
        let profile = brokkr_core::revolve::thinned(&profile, voxel_size);
        let how = brokkr_core::revolve::Revolve {
            origin: self.camera.target,
            axis: self.camera.up(),
            profile,
        };
        match brokkr_core::revolve::revolve(voxel_size, &how) {
            Ok(volume) => {
                let note = format!(
                    "revolved a line of {} points into a body ({:.0} ms)",
                    how.profile.len(),
                    started.elapsed().as_secs_f64() * 1000.0
                );
                self.revolve.armed = false;
                self.add_made_body(None, "Revolved".to_string(), volume, note);
            }
            Err(why) => self.status = format!("nothing revolved: {why}"),
        }
    }

    /// The layer over the viewport with the axis, while the tool is out.
    pub(super) fn revolve_layer(&self) -> Option<Element<'_, Message>> {
        if !self.revolve.armed {
            return None;
        }
        let axis_x = self.revolve_axis_x()?;
        Some(
            canvas(AxisLayer { axis_x, drawn: self.revolve.drawn.clone() })
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        )
    }

    /// The REVOLVE block.
    pub(super) fn revolve_panel(&self) -> Element<'_, Message> {
        let armed = self.revolve.armed;
        column![row![crate::ui::tip(
            crate::ui::toggle(
                "Draw a profile",
                armed,
                Message::Revolve(RevolveMessage::Armed(!armed))
            ),
            "A line beside the axis, spun round it into a new body",
        )]]
        .spacing(theme::S2)
        .into()
    }
}

/// The world axis a revolve would use with this camera: for tests.
#[cfg(test)]
pub(super) fn axis_of(camera: &OrbitCamera) -> (glam::Vec3, glam::Vec3) {
    (camera.target, camera.up())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::PointerEvent;
    use glam::Vec3;

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    fn at(x: f32, y: f32) -> iced::Vector {
        iced::Vector::new(x, y)
    }

    fn draw(app: &mut Brokkr, pixels: &[(f32, f32)]) {
        app.on_pointer(PointerEvent::Moved { position: at(pixels[0].0, pixels[0].1), size: SIZE });
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: at(pixels[0].0, pixels[0].1),
            size: SIZE,
        });
        for (x, y) in &pixels[1..] {
            app.on_pointer(PointerEvent::Moved { position: at(*x, *y), size: SIZE });
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
    }

    #[test]
    fn a_line_beside_the_axis_becomes_a_body_round_it() {
        let mut app = app();
        let bodies = app.doc.body_count();
        let before = app.doc.active_volume().sample_world(Vec3::ZERO);
        drop(app.update(Message::Revolve(RevolveMessage::Armed(true))));
        app.on_pointer(PointerEvent::Moved { position: at(400.0, 300.0), size: SIZE });
        let axis_x = app.revolve_axis_x().expect("the target is on screen");
        assert!(app.revolve_layer().is_some(), "the axis is drawn");
        // An upright line 60 pixels right of the axis, from 150 down to 450.
        let line: Vec<(f32, f32)> =
            (0..=30).map(|step| (axis_x + 60.0, 150.0 + step as f32 * 10.0)).collect();
        draw(&mut app, &line);
        assert_eq!(app.doc.body_count(), bodies + 1, "{}", app.status);
        assert!(!app.revolve.armed, "put away once it has made one");
        let made = app.doc.volume(app.doc.active()).expect("a body");
        let (target, up) = axis_of(&app.camera);
        // Sixty pixels across, in millimetres on the plane.
        let across = Brokkr::on_revolve_plane(
            &app.camera,
            Vec2::new(SIZE.x, SIZE.y),
            Vec2::new(axis_x + 60.0, 300.0),
        )
        .expect("on the plane")
        .x;
        let right = app.camera.right();
        let facing = (app.camera.target - app.camera.eye()).normalize();
        assert!(made.sample_world(target + right * across * 0.9) < 0.0, "inside the drawn line");
        assert!(made.sample_world(target + right * across * 1.1) > 0.0, "outside it");
        assert!(made.sample_world(target - right * across * 0.9) < 0.0, "and on the far side");
        assert!(made.sample_world(target + facing * across * 0.9) < 0.0, "and all the way round");
        assert!(made.sample_world(target + up * across * 10.0) > 0.0, "it ends");
        // The model it was drawn over is untouched: no stroke was made.
        assert_eq!(
            app.doc.volume(app.doc.nodes()[0].id).expect("a body").sample_world(Vec3::ZERO),
            before
        );
        drop(app.revolve_panel());
    }

    #[test]
    fn a_dot_makes_nothing_and_escape_puts_it_away() {
        let mut app = app();
        let bodies = app.doc.body_count();
        drop(app.update(Message::Revolve(RevolveMessage::Armed(true))));
        draw(&mut app, &[(500.0, 300.0)]);
        assert_eq!(app.doc.body_count(), bodies);
        assert!(app.status.starts_with("nothing revolved"), "{}", app.status);
        assert!(app.revolve_escaped());
        assert!(!app.revolve.armed);
    }
}
