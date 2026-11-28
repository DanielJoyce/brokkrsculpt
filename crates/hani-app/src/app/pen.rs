// SPDX-License-Identifier: AGPL-3.0-only

//! The pen's samples, kept beside the moves they belong to.
//!
//! [`crate::pen`] hands the interface each step of the pen as a pointer
//! event, and ahead of each publishes how the pen was held at it
//! ([`Message::PenSample`]). A pointer's moves are not acted on as they
//! come: they wait for the frame and are handled together
//! ([`Hani::pointer_in`]). So a sample that was simply taken as it came
//! would be the last of the frame by the time the first move of the frame
//! laid its stamps, and every stamp of a frame would be pressed alike. A
//! sample waits with the move it was published ahead of, and is taken as
//! that move is handled.

use iced::Task;

use super::Hani;
use crate::message::Message;
use crate::tablet::PenState;

/// The samples waiting on moves, and whether the pen has been asked for.
#[derive(Debug, Default)]
pub(super) struct PenTrail {
    /// The compositor has been asked, or it has been found there is none to
    /// ask. Once.
    asked: bool,
    /// Each with how many moves were waiting when it came: it is taken
    /// ahead of the move of that number.
    waiting: Vec<(usize, PenState)>,
}

impl Hani {
    /// Ask the compositor for the pen, once, when there is a window to ask
    /// for. See [`crate::pen::attach`].
    pub(super) fn ask_for_the_pen(&mut self) -> Task<Message> {
        if self.pen.asked {
            return Task::none();
        }
        self.pen.asked = true;
        #[cfg(target_os = "linux")]
        {
            let tablet = self.tablet.clone();
            iced::window::latest()
                .and_then(move |id| {
                    let tablet = tablet.clone();
                    iced::window::run(id, move |window| crate::pen::attach(window, &tablet))
                })
                .discard()
        }
        #[cfg(not(target_os = "linux"))]
        Task::none()
    }

    /// How the pen was held at the step whose event comes next.
    ///
    /// Past every guard of [`Hani::update`], and leaving the moves that
    /// wait where they are: it is no action of the user's, it changes
    /// nothing but what the next stamp is pressed with, and two hundred of
    /// them a second must not each end a rename or handle a frame's moves
    /// one by one.
    pub(super) fn pen_sample(&mut self, state: PenState) {
        if self.moves.is_empty() {
            self.tablet.feed(state);
        } else {
            self.pen.waiting.push((self.moves.len(), state));
        }
    }

    /// The pen as it was at the move about to be handled, of those that
    /// waited together. See [`Hani::handle_moves`].
    pub(super) fn pen_at_move(&mut self, index: usize) {
        let due = self.pen.waiting.iter().take_while(|(before, _)| *before <= index).count();
        if let Some((_, state)) = self.pen.waiting.drain(..due).next_back() {
            self.tablet.feed(state);
        }
    }

    /// And after the last of them, as it was last said to be.
    pub(super) fn pen_after_moves(&mut self) {
        if let Some((_, state)) = self.pen.waiting.drain(..).next_back() {
            self.tablet.feed(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::PointerEvent;
    use glam::Vec2;

    fn held(pressure: f32) -> PenState {
        PenState { in_proximity: true, pressure, eraser: false, tilt: Vec2::ZERO }
    }

    fn moved(x: f32) -> Message {
        Message::Pointer(PointerEvent::Moved {
            position: iced::Vector::new(x, 100.0),
            size: iced::Vector::new(800.0, 600.0),
        })
    }

    #[test]
    fn a_sample_with_no_move_waiting_is_the_pen_at_once() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        drop(app.update(Message::PenSample(held(0.4))));
        assert_eq!(app.tablet.state().pressure, 0.4);
    }

    /// What the trail is for. Three samples and their moves come in one
    /// frame; each move must be handled at its own sample's pressure.
    #[test]
    fn each_move_of_a_frame_is_handled_at_its_own_pressure() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        for (index, pressure) in [0.2, 0.5, 0.9].into_iter().enumerate() {
            drop(app.update(Message::PenSample(held(pressure))));
            drop(app.update(moved(100.0 + index as f32)));
        }
        assert_eq!(app.moves.len(), 3, "the moves wait for the frame");
        assert_eq!(app.tablet.state().pressure, 0.2, "and so do the samples after the first");

        let mut seen = Vec::new();
        for index in 0..3 {
            app.pen_at_move(index);
            seen.push(app.tablet.state().pressure);
        }
        assert_eq!(seen, [0.2, 0.5, 0.9]);
        assert!(app.pen.waiting.is_empty());
    }

    #[test]
    fn the_frame_leaves_the_pen_as_it_was_last_said_to_be() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        drop(app.update(Message::PenSample(held(0.3))));
        drop(app.update(moved(10.0)));
        drop(app.update(Message::PenSample(held(0.6))));
        drop(app.update(moved(11.0)));
        // The pen lifted away after the last move, with no move of its own.
        drop(app.update(Message::PenSample(PenState::NONE)));
        drop(app.update(Message::Frame));
        assert!(app.moves.is_empty());
        assert!(app.pen.waiting.is_empty());
        assert_eq!(app.tablet.state(), PenState::NONE);
    }

    /// A sample is not an action: it does not end what any other message
    /// would.
    #[test]
    fn a_sample_does_not_handle_the_moves_that_wait() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        drop(app.update(moved(10.0)));
        drop(app.update(Message::PenSample(held(0.5))));
        assert_eq!(app.moves.len(), 1);
    }
}
