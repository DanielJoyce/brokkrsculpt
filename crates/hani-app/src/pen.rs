// SPDX-License-Identifier: AGPL-3.0-only

//! The pen, from the compositor.
//!
//! Asked for by the maintainer on 2026-11-27: "please fix tablet handling
//! using the octotablet crate, iced filters the events out."
//!
//! # What was wrong
//!
//! Nothing in iced or winit carries a pen. [`crate::tablet`] went round them
//! and read the kernel's event nodes, which wants the user in the `input`
//! group: on a desktop that does not put them there the pen had no pressure
//! and the panel said to change the system. And where the pen went was left
//! to the compositor turning it into a mouse, which KWin and wlroots do for
//! a window that does not speak the tablet protocol and GNOME does not do at
//! all, so there a pen over the window did nothing.
//!
//! # What this does
//!
//! On Wayland the compositor has the pen and will say everything about it to
//! a window that asks: where it is, how hard it presses, how it leans, which
//! end is down, which button is held. `octotablet` asks, over the connection
//! the window already has ([`seat`]). No device is opened and no group is
//! needed.
//!
//! **A window that asks is no longer sent a mouse for the pen.** That is the
//! protocol, on every compositor: the pen arrives as a pen or as a mouse,
//! never both. iced knows only the mouse, so the pen is made one here, for
//! the whole interface and not only the viewport: [`layer`] is a widget
//! around everything that hands the pen's steps to what is inside it as
//! pointer events, at the pen's place. Every button, slider and menu then
//! takes a pen as it takes a mouse, and the viewport sculpts with it by the
//! path it already had.
//!
//! # How a sample keeps its pressure
//!
//! A pen reports some two hundred times a second and a frame is drawn sixty:
//! several samples come in together. Each is handed on with
//! [`PenInput::state`] published ahead of the pointer event it belongs to,
//! and the application keeps that beside the move it queues
//! (`app/pen.rs`), so a stamp is pressed as hard as the pen was when it was
//! there, and not as hard as it was at the end of the frame.
//!
//! # Where it does not reach
//!
//! X11, where octotablet 0.1 has nothing: the X server makes the pen a mouse
//! itself and the kernel reader still supplies the pressure, as before.
//! Windows and macOS, which keep the readers they had. A browser.

use std::cell::RefCell;

use glam::Vec2;
use iced::mouse;

use crate::tablet::PenState;

pub mod layer;
#[cfg(target_os = "linux")]
mod seat;

pub use layer::layer;

/// How far a pen leans at the most this takes it to lean: the tilt at which
/// [`PenState::tilt`] reads 1. Wacom's pens stop at about this, and the
/// kernel reader's own scale ends at the device's limit, which is the same
/// place.
const FULL_LEAN: f32 = 60.0 * std::f32::consts::PI / 180.0;

/// The kernel's names for what a pen and a tablet's mouse have under the
/// finger, which is what the tablet protocol sends for a button.
mod code {
    pub const LEFT: u32 = 0x110;
    pub const RIGHT: u32 = 0x111;
    pub const MIDDLE: u32 = 0x112;
    pub const STYLUS: u32 = 0x14b;
    pub const STYLUS2: u32 = 0x14c;
}

/// One thing the compositor said of the pen, in no library's words, so that
/// what is made of it can be tested without a compositor.
// Said only by the seat, which only Linux has: elsewhere nothing makes one,
// and the translator and the widget that take them are compiled and tested
// all the same, so that the day another platform has a seat it has only to
// say these.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Raw {
    /// The pen came into range over the window.
    In { eraser: bool },
    /// It left: lifted away, or moved off the window.
    Out,
    /// The tip touched.
    Down,
    /// The tip lifted.
    Up,
    /// Where the pen is and how it is held. `at` is in the window's own
    /// pixels, before the interface's scale; `tilt` is in radians from
    /// upright; a pen with no pressure to report says `None`.
    Pose { at: Vec2, pressure: Option<f32>, tilt: Option<Vec2> },
    /// A button on the pen, by the kernel's code for it.
    Button { code: u32, pressed: bool },
    /// Everything since the last of these happened at once.
    Frame,
}

/// What a step of the pen is to the interface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Moved,
    Pressed(mouse::Button),
    Released(mouse::Button),
    /// Out of range. The pointer is nowhere until something moves.
    Left,
}

/// One step of the pen: what it did, where, and how it was held as it did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PenInput {
    pub state: PenState,
    /// In the window's own pixels. `None` once it has left.
    pub at: Option<Vec2>,
    pub action: Action,
}

/// What came in one frame of the pen's, held until the frame is closed.
#[derive(Debug, Default)]
struct Frame {
    entered: Option<bool>,
    left: bool,
    down: bool,
    up: bool,
    pose: Option<(Vec2, Option<f32>, Option<Vec2>)>,
    buttons: Vec<(u32, bool)>,
}

/// Makes pointer steps of what the compositor says.
///
/// The compositor groups what it says into frames, each a moment, in no
/// promised order within one. They are put in the order a hand does them:
/// the pen arrives, is somewhere, touches, lifts, leaves. So a touch lands
/// where the pen is in that frame and as hard as it pressed in it, and not
/// where it hovered the frame before at no pressure at all.
#[derive(Debug, Default)]
pub struct Translator {
    in_range: bool,
    eraser: bool,
    down: bool,
    at: Option<Vec2>,
    pressure: Option<f32>,
    tilt: Vec2,
    held: Vec<mouse::Button>,
    frame: Frame,
}

impl Translator {
    pub fn take(&mut self, raw: Raw, out: &mut Vec<PenInput>) {
        match raw {
            Raw::In { eraser } => self.frame.entered = Some(eraser),
            Raw::Out => self.frame.left = true,
            Raw::Down => self.frame.down = true,
            Raw::Up => self.frame.up = true,
            Raw::Pose { at, pressure, tilt } => self.frame.pose = Some((at, pressure, tilt)),
            // The button under the forefinger of a tablet's mouse is its tip.
            Raw::Button { code: code::LEFT, pressed: true } => self.frame.down = true,
            Raw::Button { code: code::LEFT, pressed: false } => self.frame.up = true,
            Raw::Button { code, pressed } => self.frame.buttons.push((code, pressed)),
            Raw::Frame => self.close_frame(out),
        }
    }

    fn close_frame(&mut self, out: &mut Vec<PenInput>) {
        let frame = std::mem::take(&mut self.frame);
        if let Some(eraser) = frame.entered {
            self.in_range = true;
            self.eraser = eraser;
        }
        // A compositor may say where a pen is, or that it touched, in the
        // frame it should have said it came: one change of state is all a
        // frame of the protocol holds. A pen that is somewhere is in range.
        if frame.pose.is_some() || frame.down {
            self.in_range = true;
        }
        if let Some((at, pressure, tilt)) = frame.pose {
            self.at = Some(at);
            self.pressure = pressure;
            if let Some(tilt) = tilt {
                self.tilt = (tilt / FULL_LEAN).clamp(Vec2::NEG_ONE, Vec2::ONE);
            }
            self.say(Action::Moved, out);
        }
        if frame.down && !self.down {
            self.down = true;
            self.say(Action::Pressed(mouse::Button::Left), out);
        }
        for (code, pressed) in frame.buttons {
            let Some(button) = button_of(code) else {
                continue;
            };
            let held = self.held.contains(&button);
            if pressed && !held {
                self.held.push(button);
                self.say(Action::Pressed(button), out);
            } else if !pressed && held {
                self.held.retain(|other| *other != button);
                self.say(Action::Released(button), out);
            }
        }
        if frame.up && self.down {
            self.down = false;
            self.say(Action::Released(mouse::Button::Left), out);
        }
        if frame.left {
            self.leave(out);
        }
    }

    /// The pen is gone. Whatever it held is let go first: a compositor takes
    /// a pen away with its tip still down when the window loses it, and a
    /// stroke left open would follow the mouse.
    fn leave(&mut self, out: &mut Vec<PenInput>) {
        if !self.in_range {
            return;
        }
        if self.down {
            self.down = false;
            self.say(Action::Released(mouse::Button::Left), out);
        }
        for button in std::mem::take(&mut self.held) {
            self.say(Action::Released(button), out);
        }
        self.in_range = false;
        self.pressure = None;
        self.tilt = Vec2::ZERO;
        self.at = None;
        self.say(Action::Left, out);
    }

    fn say(&self, action: Action, out: &mut Vec<PenInput>) {
        out.push(PenInput { state: self.state(), at: self.at, action });
    }

    fn state(&self) -> PenState {
        if !self.in_range {
            return PenState::NONE;
        }
        // A tool that reports no pressure, a tablet's mouse, presses fully
        // while it is down, as any mouse does.
        let pressure = self.pressure.unwrap_or(if self.down { 1.0 } else { 0.0 });
        PenState {
            in_proximity: true,
            pressure: pressure.clamp(0.0, 1.0),
            eraser: self.eraser,
            tilt: self.tilt,
        }
    }
}

/// Which button of a mouse a button of the pen is: the lower one on the
/// barrel the right, the upper the middle, as GTK and Qt read them.
fn button_of(code: u32) -> Option<mouse::Button> {
    match code {
        code::STYLUS | code::RIGHT => Some(mouse::Button::Right),
        code::STYLUS2 | code::MIDDLE => Some(mouse::Button::Middle),
        _ => None,
    }
}

/// A step that [`layer`]'s overlay has taken, kept for what is under it.
#[derive(Debug, Clone, Copy)]
struct Step {
    input: PenInput,
    /// The overlay has published the state and been handed the event.
    heard: bool,
    /// And took it, so what is under does not get it.
    captured: bool,
    /// The pen was over the overlay, so what is under has no pointer.
    over: bool,
}

/// Everything about the pen that outlives one building of the interface.
///
/// A thread's own, and the interface's thread is the only one that touches
/// it: the seat is made there and may not leave, and the widget is updated
/// and drawn there.
#[derive(Default)]
struct Port {
    #[cfg(target_os = "linux")]
    seat: Option<seat::Seat>,
    translator: Translator,
    raw: Vec<Raw>,
    /// The pen moved after the mouse did, so the pointer is the pen's.
    pen_leads: bool,
    /// Where the pen is, in the window's own pixels, while it is in range.
    at: Option<Vec2>,
    /// The pen is over an overlay: what is under has no pointer.
    over_overlay: bool,
    /// What the overlay took this frame, for what is under it.
    for_base: Option<Vec<Step>>,
    /// The redraw the pen was last asked at. See [`first_pass`].
    asked_at: Option<iced::time::Instant>,
}

thread_local! {
    static PORT: RefCell<Port> = RefCell::default();
}

/// Begin asking the compositor for the pen, once there is a window.
///
/// Says in the log what came of it and nothing to the user: a window on X11
/// is refused here, and is no fault.
#[cfg(target_os = "linux")]
pub fn attach(window: &dyn iced::window::Window, tablet: &crate::tablet::Tablet) {
    PORT.with_borrow_mut(|port| {
        if port.seat.is_some() {
            return;
        }
        match seat::Seat::open(window, tablet.clone()) {
            Ok(seat) => {
                log::info!("pen: asking the compositor for the tablet");
                port.seat = Some(seat);
            }
            Err(why) => log::info!("pen: not from the compositor ({why}), from the kernel"),
        }
    });
}

/// What the pen has done since this was last asked, as steps.
fn pump() -> Vec<PenInput> {
    PORT.with_borrow_mut(|port| {
        let Port { translator, raw, .. } = port;
        #[cfg(target_os = "linux")]
        if let Some(seat) = &mut port.seat {
            seat.pump(raw);
        }
        let mut steps = Vec::new();
        for said in raw.drain(..) {
            translator.take(said, &mut steps);
        }
        if let Some(last) = steps.last() {
            port.pen_leads = true;
            port.at = last.at;
        }
        // `RUST_LOG=hanisculpt::pen=debug` says each step as it is taken:
        // what `scripts/drive.py`'s pen is checked against.
        for step in &steps {
            log::debug!(
                "pen: {:?} at {:?} pressure {:.3} tilt {:+.2} {:+.2}{}",
                step.action,
                step.at.map(|at| (at.x, at.y)),
                step.state.pressure,
                step.state.tilt.x,
                step.state.tilt.y,
                if step.state.eraser { " eraser" } else { "" },
            );
        }
        steps
    })
}

/// Whether this is the first time the interface is updated for the redraw
/// at `at`, which is when the pen is asked.
///
/// **Once a frame, and iced updates for one redraw up to three times.** When
/// an update for a redraw publishes anything, iced acts on it, builds the
/// interface again and updates for the same redraw again, to see whether
/// that settled it, and gives up after the third with a warning in the log
/// (`iced_winit`'s "More than 3 consecutive RedrawRequested events produced
/// layout invalidation"). Asked on every pass, a pen in the middle of a
/// stroke had more to say on each: drawing the frame reads the connection,
/// and the pen has moved since. Twenty of those warnings were in the log of
/// the first stroke drawn with a real pen, on 2026-11-27, each a frame whose
/// interface was built three times over. The passes of one redraw are handed
/// the same instant, which is how they are told from the next frame's.
fn first_pass(at: iced::time::Instant) -> bool {
    PORT.with_borrow_mut(|port| {
        let first = port.asked_at != Some(at);
        port.asked_at = Some(at);
        first
    })
}

/// The steps for what is under the overlay: the ones the overlay kept, or,
/// with no overlay up, the pen's news itself.
fn steps_for_base(at: iced::time::Instant) -> Vec<Step> {
    let kept = PORT.with_borrow_mut(|port| port.for_base.take());
    if let Some(kept) = kept {
        return kept;
    }
    if !first_pass(at) {
        return Vec::new();
    }
    pump()
        .into_iter()
        .map(|input| Step { input, heard: false, captured: false, over: false })
        .collect()
}

/// The mouse, or a finger, moved: the pointer is its again.
fn mouse_leads() {
    PORT.with_borrow_mut(|port| port.pen_leads = false);
}

/// The pointer to draw and to act by: the one given, which is the mouse's,
/// unless the pen moved last.
fn pointer(given: mouse::Cursor, scale: f32, under_overlay: bool) -> mouse::Cursor {
    PORT.with_borrow(|port| {
        if !port.pen_leads {
            return given;
        }
        match port.at {
            Some(at) if !(under_overlay && port.over_overlay) => {
                mouse::Cursor::Available(point(at, scale))
            }
            _ => mouse::Cursor::Unavailable,
        }
    })
}

/// A place in the window's own pixels, in the interface's.
fn point(at: Vec2, scale: f32) -> iced::Point {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    iced::Point::new(at.x / scale, at.y / scale)
}

/// Something for `--tablets` to say of the session, when the kernel's
/// devices are not how the pen is read in it.
pub fn session_note() -> Option<&'static str> {
    let wayland = cfg!(target_os = "linux")
        && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|name| !name.is_empty());
    wayland.then_some(
        "This is a Wayland session. The application reads the pen from the compositor \
         there,\nand needs no access to /dev/input: what is above is the other way, used \
         under X11.",
    )
}

/// Say something of the pen as the compositor would, in a test.
#[cfg(test)]
pub(crate) fn say(raw: Raw) {
    PORT.with_borrow_mut(|port| port.raw.push(raw));
}

/// A pen that has never been, for a test that shares its thread.
#[cfg(test)]
pub(crate) fn forget() {
    PORT.with_borrow_mut(|port| *port = Port::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(said: &[Raw]) -> Vec<PenInput> {
        let mut translator = Translator::default();
        let mut out = Vec::new();
        for raw in said {
            translator.take(*raw, &mut out);
        }
        out
    }

    fn pose(x: f32, y: f32, pressure: f32) -> Raw {
        Raw::Pose { at: Vec2::new(x, y), pressure: Some(pressure), tilt: None }
    }

    /// The fault the frame is put in order for: the compositor says the tip
    /// is down ahead of where the pen is in that same moment.
    #[test]
    fn a_touch_lands_where_the_pen_is_in_that_frame_and_as_hard() {
        let out = steps(&[
            Raw::In { eraser: false },
            pose(10.0, 10.0, 0.0),
            Raw::Frame,
            Raw::Down,
            pose(40.0, 30.0, 0.5),
            Raw::Frame,
        ]);
        let actions: Vec<Action> = out.iter().map(|step| step.action).collect();
        assert_eq!(
            actions,
            [Action::Moved, Action::Moved, Action::Pressed(mouse::Button::Left)],
            "the move of the frame comes ahead of its press"
        );
        let press = out[2];
        assert_eq!(press.at, Some(Vec2::new(40.0, 30.0)));
        assert_eq!(press.state.pressure, 0.5);
        assert!(press.state.in_proximity);
    }

    #[test]
    fn every_sample_of_a_stroke_keeps_its_own_pressure() {
        let out = steps(&[
            Raw::In { eraser: false },
            Raw::Down,
            pose(0.0, 0.0, 0.2),
            Raw::Frame,
            pose(1.0, 0.0, 0.4),
            Raw::Frame,
            pose(2.0, 0.0, 0.9),
            Raw::Frame,
            Raw::Up,
            pose(2.0, 0.0, 0.0),
            Raw::Frame,
        ]);
        let moves: Vec<f32> = out
            .iter()
            .filter(|step| step.action == Action::Moved)
            .map(|step| step.state.pressure)
            .collect();
        assert_eq!(moves, [0.2, 0.4, 0.9, 0.0]);
        assert_eq!(out.last().map(|step| step.action), Some(Action::Released(mouse::Button::Left)));
    }

    #[test]
    fn a_pen_taken_away_with_its_tip_down_lets_go_first() {
        let out = steps(&[
            Raw::In { eraser: false },
            Raw::Down,
            pose(5.0, 5.0, 0.7),
            Raw::Button { code: code::STYLUS, pressed: true },
            Raw::Frame,
            Raw::Out,
            Raw::Frame,
        ]);
        let tail: Vec<Action> = out.iter().skip(3).map(|step| step.action).collect();
        assert_eq!(
            tail,
            [
                Action::Released(mouse::Button::Left),
                Action::Released(mouse::Button::Right),
                Action::Left
            ]
        );
        let last = out.last().expect("a pen that left says so");
        assert_eq!(last.state, PenState::NONE, "a pen away is no pen: a mouse is at full strength");
        assert_eq!(last.at, None);
    }

    #[test]
    fn the_barrel_buttons_are_the_right_and_the_middle() {
        let out = steps(&[
            Raw::In { eraser: false },
            pose(1.0, 1.0, 0.0),
            Raw::Button { code: code::STYLUS, pressed: true },
            Raw::Frame,
            Raw::Button { code: code::STYLUS, pressed: false },
            Raw::Button { code: code::STYLUS2, pressed: true },
            Raw::Frame,
            // One this has no meaning for.
            Raw::Button { code: 0x149, pressed: true },
            Raw::Frame,
        ]);
        let buttons: Vec<Action> =
            out.iter().map(|step| step.action).filter(|action| *action != Action::Moved).collect();
        assert_eq!(
            buttons,
            [
                Action::Pressed(mouse::Button::Right),
                Action::Released(mouse::Button::Right),
                Action::Pressed(mouse::Button::Middle),
            ]
        );
    }

    #[test]
    fn the_eraser_end_is_told_from_the_tip() {
        let out = steps(&[Raw::In { eraser: true }, pose(1.0, 1.0, 0.3), Raw::Frame]);
        assert!(out[0].state.eraser);
        let out = steps(&[Raw::In { eraser: false }, pose(1.0, 1.0, 0.3), Raw::Frame]);
        assert!(!out[0].state.eraser);
    }

    /// A tablet's mouse has a button and no pressure, and must not sculpt at
    /// nothing.
    #[test]
    fn a_tool_with_no_pressure_presses_fully_while_it_is_down() {
        let at = Vec2::new(3.0, 3.0);
        let out = steps(&[
            Raw::In { eraser: false },
            Raw::Pose { at, pressure: None, tilt: None },
            Raw::Frame,
            Raw::Button { code: code::LEFT, pressed: true },
            Raw::Frame,
            Raw::Button { code: code::LEFT, pressed: false },
            Raw::Frame,
        ]);
        assert_eq!(out[0].state.pressure, 0.0, "hovering");
        assert_eq!(out[1].action, Action::Pressed(mouse::Button::Left));
        assert_eq!(out[1].state.pressure, 1.0);
        assert_eq!(out[2].action, Action::Released(mouse::Button::Left));
    }

    #[test]
    fn a_lean_is_a_share_of_the_most_a_pen_leans_and_no_more() {
        let lean = |degrees: f32| {
            let tilt = Some(Vec2::new(degrees.to_radians(), -degrees.to_radians()));
            steps(&[Raw::Pose { at: Vec2::ZERO, pressure: Some(0.1), tilt }, Raw::Frame])[0]
                .state
                .tilt
        };
        assert!((lean(30.0) - Vec2::new(0.5, -0.5)).length() < 1e-5);
        assert_eq!(lean(80.0), Vec2::new(1.0, -1.0));
        assert_eq!(lean(0.0), Vec2::ZERO);
    }

    /// One change of state is all a frame of the protocol holds, so a pen
    /// that came and touched at once is only said to have touched.
    #[test]
    fn a_pen_never_said_to_have_come_is_in_range_once_it_is_somewhere() {
        let out = steps(&[Raw::Down, pose(2.0, 2.0, 0.6), Raw::Frame]);
        assert!(out.iter().all(|step| step.state.in_proximity));
        assert_eq!(out[1].action, Action::Pressed(mouse::Button::Left));
    }

    #[test]
    fn a_place_is_scaled_into_the_interfaces_pixels() {
        assert_eq!(point(Vec2::new(300.0, 150.0), 1.5), iced::Point::new(200.0, 100.0));
        assert_eq!(point(Vec2::new(300.0, 150.0), 0.0), iced::Point::new(300.0, 150.0));
    }
}
