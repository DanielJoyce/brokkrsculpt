// SPDX-License-Identifier: AGPL-3.0-only

//! Fingers on a touch screen: the tool with one, the view with two.
//!
//! Asked for by the maintainer on 2026-11-28: "the wasm build, I can't zoom
//! or pan on my touch tablet so we need some kind of gestures support (for
//! wasm and native tablet builds)". Until this a touch was the left button
//! and nothing else (`viewport.rs`, where a Windows pen arrives as a touch
//! too), and a second finger was refused, so a tablet with no mouse could
//! sculpt and could not turn, slide or zoom the view at all.
//!
//! Now:
//!
//! * **One finger on the model** is the left button, as it was. It is held
//!   back until it has moved a few pixels or a moment has passed
//!   ([`SLOP_PX`], [`HOLD_MS`]), so the first finger of a pinch does not
//!   sculpt before the second arrives. A finger lifted before either is a
//!   tap: a click.
//! * **One finger off the model** turns the view, as the right button does.
//! * **Two fingers** slide the view by how far their middle moves and zoom
//!   it by how far apart they move, toward what is under their middle. A
//!   stroke or a turn under way when the second finger lands is ended there.
//!   Once two have been down, what is left down does nothing until every
//!   finger is up, so lifting one of a pinch does not start a stroke.
//!
//! A pen hovering over the surface, which Windows reports as a finger
//! moving with none down, is a hover, as it was.

use glam::Vec2;
use iced::Vector;
use web_time::Instant;

use super::Hani;
use crate::message::{PointerButton, PointerEvent, TouchPhase};

/// How far a finger on the model moves before it is taken for a stroke, in
/// pixels.
pub(super) const SLOP_PX: f32 = 8.0;

/// How long a finger on the model is held before it is taken for a stroke
/// though it has not moved, in milliseconds.
pub(super) const HOLD_MS: u128 = 120;

/// What the fingers on the screen are doing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) enum Doing {
    /// No finger down.
    #[default]
    Nothing,
    /// One finger on the model, not yet a stroke: where it went down, when.
    Waiting { at: Vec2, since: Instant },
    /// One finger: the left button, held.
    Tool,
    /// One finger off the model: the right button, held, turning the view.
    Turning,
    /// Two fingers: where their middle was and how far apart they were.
    Pinching { middle: Vec2, apart: f32 },
    /// A pinch over, a finger still down: nothing until all are up.
    Done,
}

/// The fingers down and what they are doing.
#[derive(Debug, Clone, Default)]
pub(super) struct Touches {
    down: Vec<(u64, Vec2)>,
    doing: Doing,
    size: Vector,
}

impl Touches {
    fn middle_and_apart(&self) -> Option<(Vec2, f32)> {
        let [(_, a), (_, b), ..] = self.down[..] else {
            return None;
        };
        Some(((a + b) * 0.5, a.distance(b)))
    }
}

fn vector(point: Vec2) -> Vector {
    Vector::new(point.x, point.y)
}

impl Hani {
    /// A finger down, moved or lifted, in widget pixels.
    pub(super) fn touched(&mut self, finger: u64, phase: TouchPhase, at: Vec2, size: Vector) {
        self.touches.size = size;
        // Under a card, a finger is the card's: nothing starts here.
        if self.modal_open() && phase == TouchPhase::Down {
            return;
        }
        match phase {
            TouchPhase::Down => {
                self.touches.down.retain(|(id, _)| *id != finger);
                self.touches.down.push((finger, at));
                match self.touches.down.len() {
                    1 => {
                        self.touches.doing = if self.pick(at).is_some() {
                            Doing::Waiting { at, since: Instant::now() }
                        } else {
                            self.pointer(PointerEvent::Pressed {
                                button: PointerButton::Right,
                                position: vector(at),
                                size,
                            });
                            Doing::Turning
                        };
                    }
                    2 => {
                        // Whatever one finger began ends where the second lands.
                        match self.touches.doing {
                            Doing::Tool => {
                                self.pointer(PointerEvent::Released { button: PointerButton::Left })
                            }
                            Doing::Turning => self
                                .pointer(PointerEvent::Released { button: PointerButton::Right }),
                            _ => {}
                        }
                        if let Some((middle, apart)) = self.touches.middle_and_apart() {
                            self.touches.doing = Doing::Pinching { middle, apart };
                        }
                    }
                    _ => {}
                }
            }
            TouchPhase::Moved => {
                let Some(slot) = self.touches.down.iter_mut().find(|(id, _)| *id == finger) else {
                    // A pen in range and off the surface: a hover.
                    self.pointer(PointerEvent::Moved { position: vector(at), size });
                    return;
                };
                slot.1 = at;
                match self.touches.doing {
                    Doing::Waiting { at: start, .. } => {
                        if start.distance(at) >= SLOP_PX {
                            self.begin_the_stroke(start);
                            self.pointer(PointerEvent::Moved { position: vector(at), size });
                        }
                    }
                    Doing::Tool | Doing::Turning => {
                        self.pointer(PointerEvent::Moved { position: vector(at), size });
                    }
                    Doing::Pinching { middle, apart } => {
                        let Some((now_middle, now_apart)) = self.touches.middle_and_apart() else {
                            return;
                        };
                        self.camera.pan(now_middle - middle, self.viewport_size.y);
                        if apart > 1.0 && now_apart > 1.0 {
                            // Apart is closer: the distance shrinks by as much
                            // as the spread grows.
                            let factor = apart / now_apart;
                            let anchor = self
                                .pick(now_middle)
                                .map_or(self.camera.target, |(_, hit)| hit.position);
                            self.camera.zoom_by_about(factor, anchor);
                        }
                        self.publish_camera();
                        self.touches.doing =
                            Doing::Pinching { middle: now_middle, apart: now_apart };
                    }
                    Doing::Nothing | Doing::Done => {}
                }
            }
            TouchPhase::Up => {
                self.touches.down.retain(|(id, _)| *id != finger);
                match self.touches.doing {
                    // Lifted before it was a stroke: a click where it went down.
                    Doing::Waiting { at: start, .. } => {
                        self.begin_the_stroke(start);
                        self.pointer(PointerEvent::Released { button: PointerButton::Left });
                    }
                    Doing::Tool => {
                        self.pointer(PointerEvent::Released { button: PointerButton::Left })
                    }
                    Doing::Turning => {
                        self.pointer(PointerEvent::Released { button: PointerButton::Right })
                    }
                    Doing::Pinching { .. } | Doing::Done | Doing::Nothing => {}
                }
                self.touches.doing =
                    if self.touches.down.is_empty() { Doing::Nothing } else { Doing::Done };
                // Back to a pinch if two are still down.
                if let Some((middle, apart)) = self.touches.middle_and_apart() {
                    self.touches.doing = Doing::Pinching { middle, apart };
                }
            }
        }
    }

    /// On each frame: a finger held still on the model long enough is a
    /// stroke, though it has not moved.
    pub(super) fn touch_held(&mut self) {
        if let Doing::Waiting { at, since } = self.touches.doing
            && since.elapsed().as_millis() >= HOLD_MS
        {
            self.begin_the_stroke(at);
        }
    }

    fn begin_the_stroke(&mut self, at: Vec2) {
        let size = self.touches.size;
        self.pointer(PointerEvent::Moved { position: vector(at), size });
        self.pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: vector(at),
            size,
        });
        self.touches.doing = Doing::Tool;
    }

    /// The mouse's events, as a finger stands in for them.
    fn pointer(&mut self, event: PointerEvent) {
        self.on_pointer(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn fresh() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        app
    }

    fn touch(app: &mut Hani, finger: u64, phase: TouchPhase, x: f32, y: f32) {
        app.on_pointer(PointerEvent::Touch {
            finger,
            phase,
            position: Vector::new(x, y),
            size: SIZE,
        });
    }

    /// One finger dragged on the model sculpts; off it, it turns the view
    /// and leaves the model as it was.
    #[test]
    fn one_finger_sculpts_on_the_model_and_turns_the_view_off_it() {
        let mut app = fresh();
        touch(&mut app, 1, TouchPhase::Down, 400.0, 300.0);
        for step in 1..=10 {
            touch(&mut app, 1, TouchPhase::Moved, 400.0 + step as f32 * 3.0, 300.0);
        }
        touch(&mut app, 1, TouchPhase::Up, 430.0, 300.0);
        assert!(app.unsaved, "a finger dragged on the model did not sculpt");

        let mut app = fresh();
        let before = app.camera.orientation();
        touch(&mut app, 1, TouchPhase::Down, 20.0, 20.0);
        for step in 1..=10 {
            touch(&mut app, 1, TouchPhase::Moved, 20.0 + step as f32 * 10.0, 20.0);
        }
        touch(&mut app, 1, TouchPhase::Up, 120.0, 20.0);
        assert!(!app.unsaved, "a finger off the model changed the model");
        assert_ne!(
            app.camera.orientation(),
            before,
            "a finger off the model did not turn the view"
        );
    }

    /// Two fingers spread apart zoom in, slid together they slide the view,
    /// and neither sculpts, though the first landed on the model.
    #[test]
    fn two_fingers_zoom_and_slide_and_do_not_sculpt() {
        let mut app = fresh();
        let (distance, target) = (app.camera.eye().distance(app.camera.target), app.camera.target);
        touch(&mut app, 1, TouchPhase::Down, 380.0, 300.0);
        touch(&mut app, 2, TouchPhase::Down, 420.0, 300.0);
        for step in 1..=10 {
            let spread = step as f32 * 8.0;
            touch(&mut app, 1, TouchPhase::Moved, 380.0 - spread, 300.0);
            touch(&mut app, 2, TouchPhase::Moved, 420.0 + spread, 300.0);
        }
        let closer = app.camera.eye().distance(app.camera.target);
        assert!(closer < distance * 0.8, "spread apart, the view went from {distance} to {closer}");
        for step in 1..=10 {
            let slide = step as f32 * 6.0;
            touch(&mut app, 1, TouchPhase::Moved, 300.0, 300.0 + slide);
            touch(&mut app, 2, TouchPhase::Moved, 500.0, 300.0 + slide);
        }
        assert_ne!(app.camera.target, target, "slid together, the view did not slide");
        // One lifted, the other moved on: nothing, until both are up.
        touch(&mut app, 2, TouchPhase::Up, 500.0, 360.0);
        for step in 1..=10 {
            touch(&mut app, 1, TouchPhase::Moved, 300.0 + step as f32 * 5.0, 360.0);
        }
        touch(&mut app, 1, TouchPhase::Up, 350.0, 360.0);
        assert!(!app.unsaved, "a pinch sculpted");
    }

    /// A finger held still on the model becomes a stroke on the next frame
    /// past the hold; one lifted at once is a click, not a stroke.
    #[test]
    fn a_held_finger_starts_a_stroke_and_a_tap_is_a_click() {
        let mut app = fresh();
        touch(&mut app, 1, TouchPhase::Down, 400.0, 300.0);
        assert!(matches!(app.touches.doing, Doing::Waiting { .. }));
        if let Doing::Waiting { at, .. } = app.touches.doing {
            app.touches.doing = Doing::Waiting {
                at,
                since: Instant::now() - std::time::Duration::from_millis(HOLD_MS as u64 + 5),
            };
        }
        drop(app.update(Message::Frame));
        assert_eq!(app.touches.doing, Doing::Tool, "held, it did not become a stroke");
        touch(&mut app, 1, TouchPhase::Up, 400.0, 300.0);
        assert_eq!(app.touches.doing, Doing::Nothing);

        let mut app = fresh();
        touch(&mut app, 1, TouchPhase::Down, 400.0, 300.0);
        touch(&mut app, 1, TouchPhase::Up, 400.0, 300.0);
        assert!(app.drag.is_none(), "a tap left a drag behind");
    }

    /// A finger moving with none down is a hover, as a pen in range is.
    #[test]
    fn a_finger_moving_with_none_down_is_a_hover() {
        let mut app = fresh();
        touch(&mut app, 9, TouchPhase::Moved, 400.0, 300.0);
        assert!(app.hover.is_some(), "a pen in range did not hover");
        assert_eq!(app.touches.doing, Doing::Nothing);
    }
}
