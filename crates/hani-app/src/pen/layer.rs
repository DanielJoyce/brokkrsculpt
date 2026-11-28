// SPDX-License-Identifier: AGPL-3.0-only

//! The widget that makes the pen a pointer for everything inside it.
//!
//! It goes round the whole interface and adds nothing to it: no size, no
//! state, no place in the tree of its own. On each frame it asks for the
//! pen's news ([`super::pump`]) and hands each step to what it holds as the
//! mouse event it stands for, with the pointer at the pen. And while the pen
//! was the last thing to move, every other event, and the drawing, is given
//! the pen's place for the pointer in place of the mouse's, so what is under
//! the pen is what is lit.
//!
//! # Overlays
//!
//! A menu let down from a list, or a tooltip, is not inside the widget that
//! raised it: iced asks the root for its overlay and deals with that first,
//! and gives what is under it no pointer where the overlay is. This is the
//! root, so it does as iced does (`iced_runtime::UserInterface::update`):
//! [`PenOverlay`] takes the pen's steps first and keeps them, with whether
//! it took each and whether the pen was over it, for the widget under it to
//! act on after.

use std::cell::RefCell;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::time::Instant;
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse, window};

use super::{Action, PenInput, Step};
use crate::tablet::PenState;

/// `content`, taking a pen as it takes a mouse.
///
/// `scale` is the interface's own, over the system's: the pen is said in the
/// window's pixels and the interface is laid out in its own. `on_sample` is
/// published ahead of each step's event with how the pen was held at it.
pub fn layer<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    scale: f32,
    on_sample: fn(PenState) -> Message,
) -> Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    Element::new(PenLayer { content: content.into(), scale, on_sample })
}

struct PenLayer<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    scale: f32,
    on_sample: fn(PenState) -> Message,
}

/// Whether an event is a pointer's own doing, the mouse's or a finger's, so
/// that the pointer is that one's from here on.
fn is_a_pointers(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(
            mouse::Event::CursorMoved { .. }
                | mouse::Event::ButtonPressed(_)
                | mouse::Event::WheelScrolled { .. }
        ) | Event::Touch(_)
    )
}

/// The instant of the redraw an event is, if it is one.
fn redraw_at(event: &Event) -> Option<Instant> {
    match event {
        Event::Window(window::Event::RedrawRequested(at)) => Some(*at),
        _ => None,
    }
}

/// The mouse event a step of the pen stands for.
fn event_of(input: &PenInput, scale: f32) -> Event {
    Event::Mouse(match input.action {
        Action::Moved => mouse::Event::CursorMoved {
            position: super::point(input.at.unwrap_or_default(), scale),
        },
        Action::Pressed(button) => mouse::Event::ButtonPressed(button),
        Action::Released(button) => mouse::Event::ButtonReleased(button),
        Action::Left => mouse::Event::CursorLeft,
    })
}

/// The pointer at a step: the pen where it was then.
fn pointer_at(input: &PenInput, scale: f32) -> mouse::Cursor {
    match input.at {
        Some(at) => mouse::Cursor::Available(super::point(at, scale)),
        None => mouse::Cursor::Unavailable,
    }
}

/// Hand one event the pen stands for to something, and pass on what came of
/// it, but for one thing: whether it was taken, which is returned and not
/// passed on. The event the frame is being updated for is the redraw, and a
/// press the pen made inside it is not the redraw being taken: iced drops
/// the overlay for the frame when the root says it took the event.
fn hand<Message>(shell: &mut Shell<'_, Message>, to: impl FnOnce(&mut Shell<'_, Message>)) -> bool {
    let mut messages = Vec::new();
    let mut local = Shell::new(&mut messages);
    to(&mut local);
    let captured = local.is_event_captured();
    if local.is_layout_invalid() {
        shell.invalidate_layout();
    }
    if local.are_widgets_invalid() {
        shell.invalidate_widgets();
    }
    shell.request_redraw_at(local.redraw_request());
    shell.input_method_mut().merge(local.input_method());
    for message in messages {
        shell.publish(message);
    }
    captured
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for PenLayer<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    // The tree is the content's own, handed straight through: this widget is
    // not in it.
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if is_a_pointers(event) {
            super::mouse_leads();
        }
        if let Some(at) = redraw_at(event) {
            for step in super::steps_for_base(at) {
                if !step.heard {
                    shell.publish((self.on_sample)(step.input.state));
                }
                if step.captured {
                    continue;
                }
                let stands_for = event_of(&step.input, self.scale);
                let pointer = if step.over {
                    mouse::Cursor::Unavailable
                } else {
                    pointer_at(&step.input, self.scale)
                };
                let content = &mut self.content;
                let _ = hand(shell, |shell| {
                    content.as_widget_mut().update(
                        tree,
                        &stands_for,
                        layout,
                        pointer,
                        renderer,
                        clipboard,
                        shell,
                        viewport,
                    );
                });
            }
        }
        self.content.as_widget_mut().update(
            tree,
            event,
            layout,
            super::pointer(cursor, self.scale, true),
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            tree,
            renderer,
            theme,
            style,
            layout,
            super::pointer(cursor, self.scale, true),
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            tree,
            layout,
            super::pointer(cursor, self.scale, true),
            viewport,
            renderer,
        )
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        let Some(inner) =
            self.content.as_widget_mut().overlay(tree, layout, renderer, viewport, translation)
        else {
            // Nothing is over what is under any more.
            super::PORT.with_borrow_mut(|port| port.over_overlay = false);
            return None;
        };
        Some(overlay::Element::new(Box::new(PenOverlay {
            nested: RefCell::new(overlay::Nested::new(inner)),
            scale: self.scale,
            on_sample: self.on_sample,
        })))
    }
}

/// The content's overlay, and every overlay nested in it, taking the pen.
///
/// `Nested` is iced's own way through an overlay's overlays, and wants
/// `&mut` where the trait gives `&`, to draw and to say what the pointer is
/// over: hence the cell.
struct PenOverlay<'a, Message, Theme, Renderer> {
    nested: RefCell<overlay::Nested<'a, Message, Theme, Renderer>>,
    scale: f32,
    on_sample: fn(PenState) -> Message,
}

impl<Message, Theme, Renderer> PenOverlay<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    /// Whether a pointer there is over the overlay, as iced asks it: by
    /// whether the overlay has anything to say of the pointer.
    fn is_under(&self, pointer: mouse::Cursor, layout: Layout<'_>, renderer: &Renderer) -> bool {
        pointer.position().is_some_and(|at| {
            self.nested.borrow_mut().mouse_interaction(
                layout,
                mouse::Cursor::Available(at),
                renderer,
            ) != mouse::Interaction::None
        })
    }
}

impl<Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for PenOverlay<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.nested.get_mut().layout(renderer, bounds)
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.nested.get_mut().operate(layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        if is_a_pointers(event) {
            super::mouse_leads();
        }
        if let Some(at) = redraw_at(event)
            && super::first_pass(at)
        {
            let mut kept = Vec::new();
            for input in super::pump() {
                shell.publish((self.on_sample)(input.state));
                let stands_for = event_of(&input, self.scale);
                let pointer = pointer_at(&input, self.scale);
                let nested = self.nested.get_mut();
                let captured = hand(shell, |shell| {
                    nested.update(&stands_for, layout, pointer, renderer, clipboard, shell);
                });
                let over = self.is_under(pointer, layout, renderer);
                kept.push(Step { input, heard: true, captured, over });
            }
            super::PORT.with_borrow_mut(|port| match &mut port.for_base {
                // What is under was not reached last frame. It still gets it.
                Some(waiting) => waiting.append(&mut kept),
                None => port.for_base = Some(kept),
            });
        }
        let pointer = super::pointer(cursor, self.scale, false);
        let over = self.is_under(pointer, layout, renderer);
        super::PORT.with_borrow_mut(|port| port.over_overlay = port.pen_leads && over);
        self.nested.get_mut().update(event, layout, pointer, renderer, clipboard, shell);
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.nested.borrow_mut().draw(
            renderer,
            theme,
            style,
            layout,
            super::pointer(cursor, self.scale, false),
        );
    }

    // Given the pointer it was asked about and not the pen's: iced asks this
    // of the mouse, to know whether the mouse is over the overlay.
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.nested.borrow_mut().mouse_interaction(layout, cursor, renderer)
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;
    use iced::advanced::clipboard;
    use iced::{Point, Theme};
    use iced_runtime::user_interface::{Cache, UserInterface};

    use super::*;
    use crate::pen::{Raw, forget, say};

    /// What the tests' widgets publish.
    #[derive(Debug, Clone, PartialEq)]
    enum Said {
        Sample(f32),
        Clicked,
        /// By the probe under everything: what it was handed and where the
        /// pointer was said to be.
        Under(mouse::Event, Option<Point>),
        /// By the overlay.
        Over(mouse::Event),
    }

    fn sample(state: PenState) -> Said {
        Said::Sample(state.pressure)
    }

    const WINDOW: Size = Size::new(800.0, 600.0);

    /// Fills what it is given and says every mouse event it is handed.
    struct Probe;

    impl Widget<Said, Theme, ()> for Probe {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fill, Length::Fill)
        }

        fn layout(&mut self, _: &mut Tree, _: &(), limits: &layout::Limits) -> layout::Node {
            layout::Node::new(limits.max())
        }

        fn draw(
            &self,
            _: &Tree,
            _: &mut (),
            _: &Theme,
            _: &renderer::Style,
            _: Layout<'_>,
            _: mouse::Cursor,
            _: &Rectangle,
        ) {
        }

        fn update(
            &mut self,
            _: &mut Tree,
            event: &Event,
            _: Layout<'_>,
            cursor: mouse::Cursor,
            _: &(),
            _: &mut dyn Clipboard,
            shell: &mut Shell<'_, Said>,
            _: &Rectangle,
        ) {
            if let Event::Mouse(event) = event {
                shell.publish(Said::Under(*event, cursor.position()));
            }
        }
    }

    /// A probe with something over the left of it: an overlay 200 wide and
    /// the window high, which takes every press on it.
    struct Covered;

    const COVER: Rectangle = Rectangle { x: 0.0, y: 0.0, width: 200.0, height: 600.0 };

    struct Cover;

    impl overlay::Overlay<Said, Theme, ()> for Cover {
        fn layout(&mut self, _: &(), _: Size) -> layout::Node {
            layout::Node::new(COVER.size())
        }

        fn draw(
            &self,
            _: &mut (),
            _: &Theme,
            _: &renderer::Style,
            _: Layout<'_>,
            _: mouse::Cursor,
        ) {
        }

        fn update(
            &mut self,
            event: &Event,
            layout: Layout<'_>,
            cursor: mouse::Cursor,
            _: &(),
            _: &mut dyn Clipboard,
            shell: &mut Shell<'_, Said>,
        ) {
            let Event::Mouse(event) = event else {
                return;
            };
            if cursor.is_over(layout.bounds()) {
                shell.publish(Said::Over(*event));
                if matches!(event, mouse::Event::ButtonPressed(_)) {
                    shell.capture_event();
                }
            }
        }

        fn mouse_interaction(
            &self,
            layout: Layout<'_>,
            cursor: mouse::Cursor,
            _: &(),
        ) -> mouse::Interaction {
            if cursor.is_over(layout.bounds()) {
                mouse::Interaction::Pointer
            } else {
                mouse::Interaction::None
            }
        }
    }

    impl Widget<Said, Theme, ()> for Covered {
        fn size(&self) -> Size<Length> {
            Probe.size()
        }

        fn layout(
            &mut self,
            tree: &mut Tree,
            renderer: &(),
            limits: &layout::Limits,
        ) -> layout::Node {
            Probe.layout(tree, renderer, limits)
        }

        fn draw(
            &self,
            _: &Tree,
            _: &mut (),
            _: &Theme,
            _: &renderer::Style,
            _: Layout<'_>,
            _: mouse::Cursor,
            _: &Rectangle,
        ) {
        }

        fn update(
            &mut self,
            tree: &mut Tree,
            event: &Event,
            layout: Layout<'_>,
            cursor: mouse::Cursor,
            renderer: &(),
            clipboard: &mut dyn Clipboard,
            shell: &mut Shell<'_, Said>,
            viewport: &Rectangle,
        ) {
            Probe.update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
        }

        fn overlay<'a>(
            &'a mut self,
            _: &'a mut Tree,
            _: Layout<'a>,
            _: &(),
            _: &Rectangle,
            _: Vector,
        ) -> Option<overlay::Element<'a, Said, Theme, ()>> {
            Some(overlay::Element::new(Box::new(Cover)))
        }
    }

    /// One pass of iced's own loop over `content` in the layer, as a frame
    /// is: the events given, then the redraw. What was published, in order.
    fn frame<'a>(
        content: impl Into<Element<'a, Said, Theme, ()>>,
        scale: f32,
        cache: &mut Cache,
        events: &[Event],
        mouse: mouse::Cursor,
    ) -> Vec<Said> {
        frame_at(content, scale, cache, events, mouse, Instant::now())
    }

    /// [`frame`], for the redraw at an instant given: iced's second and
    /// third passes over one redraw are at the instant of the first.
    fn frame_at<'a>(
        content: impl Into<Element<'a, Said, Theme, ()>>,
        scale: f32,
        cache: &mut Cache,
        events: &[Event],
        mouse: mouse::Cursor,
        at: Instant,
    ) -> Vec<Said> {
        let mut renderer = ();
        let mut said = Vec::new();
        let mut interface = UserInterface::build(
            layer(content, scale, sample),
            WINDOW,
            std::mem::take(cache),
            &mut renderer,
        );
        let mut events = events.to_vec();
        events.push(Event::Window(window::Event::RedrawRequested(at)));
        let _ = interface.update(&events, mouse, &mut renderer, &mut clipboard::Null, &mut said);
        *cache = interface.into_cache();
        said
    }

    fn pose(x: f32, y: f32, pressure: f32) {
        say(Raw::Pose { at: Vec2::new(x, y), pressure: Some(pressure), tilt: None });
    }

    fn a_button<'a>() -> Element<'a, Said, Theme, ()> {
        iced::widget::button(iced::widget::space().width(100).height(40))
            .padding(0)
            .on_press(Said::Clicked)
            .into()
    }

    #[test]
    fn a_pen_presses_a_button_as_a_mouse_does() {
        forget();
        let mut cache = Cache::default();
        say(Raw::In { eraser: false });
        pose(50.0, 20.0, 0.0);
        say(Raw::Frame);
        say(Raw::Down);
        pose(50.0, 20.0, 0.6);
        say(Raw::Frame);
        say(Raw::Up);
        pose(50.0, 20.0, 0.0);
        say(Raw::Frame);
        let said = frame(a_button(), 1.0, &mut cache, &[], mouse::Cursor::Unavailable);
        assert!(said.contains(&Said::Clicked), "{said:?}");
    }

    #[test]
    fn a_pen_off_the_button_does_not_press_it() {
        forget();
        let mut cache = Cache::default();
        say(Raw::Down);
        pose(300.0, 300.0, 0.6);
        say(Raw::Frame);
        say(Raw::Up);
        say(Raw::Frame);
        let said = frame(a_button(), 1.0, &mut cache, &[], mouse::Cursor::Unavailable);
        assert!(!said.contains(&Said::Clicked), "{said:?}");
    }

    /// The window's pixels are not the interface's when the interface is
    /// drawn larger: a button 100 wide is 200 of the window's across.
    #[test]
    fn the_pen_is_where_it_is_at_the_interfaces_own_scale() {
        for (x, hits) in [(150.0, true), (250.0, false)] {
            forget();
            let mut cache = Cache::default();
            say(Raw::Down);
            pose(x, 40.0, 0.5);
            say(Raw::Frame);
            say(Raw::Up);
            say(Raw::Frame);
            let said = frame(a_button(), 2.0, &mut cache, &[], mouse::Cursor::Unavailable);
            assert_eq!(said.contains(&Said::Clicked), hits, "at {x}: {said:?}");
        }
    }

    /// What `app/pen.rs` relies on: how the pen was held at a step comes
    /// ahead of what the step did.
    #[test]
    fn each_steps_sample_is_published_ahead_of_what_the_step_did() {
        forget();
        let mut cache = Cache::default();
        for (x, pressure) in [(10.0, 0.25), (20.0, 0.5), (30.0, 0.75)] {
            pose(x, 5.0, pressure);
            say(Raw::Frame);
        }
        let said = frame(Element::new(Probe), 1.0, &mut cache, &[], mouse::Cursor::Unavailable);
        let moved = |x: f32| {
            let at = Point::new(x, 5.0);
            Said::Under(mouse::Event::CursorMoved { position: at }, Some(at))
        };
        assert_eq!(
            said,
            [
                Said::Sample(0.25),
                moved(10.0),
                Said::Sample(0.5),
                moved(20.0),
                Said::Sample(0.75),
                moved(30.0),
            ]
        );
    }

    #[test]
    fn the_pointer_is_whichever_of_the_pen_and_the_mouse_moved_last() {
        forget();
        let mut cache = Cache::default();
        let mouse_at = Point::new(700.0, 500.0);
        let mouse = mouse::Cursor::Available(mouse_at);
        let click = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let under = |said: &[Said]| {
            said.iter()
                .rev()
                .find_map(|said| match said {
                    Said::Under(mouse::Event::ButtonReleased(_), at) => Some(*at),
                    _ => None,
                })
                .expect("the probe was handed the release")
        };

        // Nothing of a pen yet: the mouse's.
        let said = frame(Element::new(Probe), 1.0, &mut cache, std::slice::from_ref(&click), mouse);
        assert_eq!(under(&said), Some(mouse_at));

        // The pen moves. The mouse lies where it was and iced still says so,
        // and what is inside is told the pen's place.
        pose(40.0, 40.0, 0.0);
        say(Raw::Frame);
        let _ = frame(Element::new(Probe), 1.0, &mut cache, &[], mouse);
        let said = frame(Element::new(Probe), 1.0, &mut cache, std::slice::from_ref(&click), mouse);
        assert_eq!(under(&said), Some(Point::new(40.0, 40.0)));

        // The mouse moves, and has it back.
        let moved = Event::Mouse(mouse::Event::CursorMoved { position: mouse_at });
        let said = frame(Element::new(Probe), 1.0, &mut cache, &[moved, click], mouse);
        assert_eq!(under(&said), Some(mouse_at));
    }

    #[test]
    fn a_pen_that_leaves_takes_the_pointer_away_with_it() {
        forget();
        let mut cache = Cache::default();
        let mouse = mouse::Cursor::Available(Point::new(700.0, 500.0));
        pose(40.0, 40.0, 0.0);
        say(Raw::Frame);
        say(Raw::Out);
        say(Raw::Frame);
        let said = frame(Element::new(Probe), 1.0, &mut cache, &[], mouse);
        assert_eq!(said.last(), Some(&Said::Under(mouse::Event::CursorLeft, None)));
        // And nothing is under a pointer until one moves.
        let click = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let said = frame(Element::new(Probe), 1.0, &mut cache, &[click], mouse);
        assert_eq!(said, [Said::Under(mouse::Event::ButtonReleased(mouse::Button::Left), None)]);
    }

    /// iced deals with an overlay ahead of what is under it and gives what
    /// is under no pointer where the overlay is. The pen has to be dealt
    /// with the same, by this widget, since it is the root.
    #[test]
    fn a_press_on_an_overlay_is_the_overlays_and_not_what_is_under_its() {
        forget();
        let mut cache = Cache::default();
        say(Raw::Down);
        pose(100.0, 300.0, 0.5);
        say(Raw::Frame);
        let said = frame(Element::new(Covered), 1.0, &mut cache, &[], mouse::Cursor::Unavailable);
        let at = Point::new(100.0, 300.0);
        assert_eq!(
            said,
            [
                Said::Sample(0.5),
                Said::Over(mouse::Event::CursorMoved { position: at }),
                Said::Sample(0.5),
                Said::Over(mouse::Event::ButtonPressed(mouse::Button::Left)),
                // What is under is told the pen moved, and not where: it is
                // not under it. The press it is not told of at all.
                Said::Under(mouse::Event::CursorMoved { position: at }, None),
            ]
        );
    }

    #[test]
    fn a_press_beside_an_overlay_reaches_what_is_under() {
        forget();
        let mut cache = Cache::default();
        say(Raw::Down);
        pose(500.0, 300.0, 0.5);
        say(Raw::Frame);
        let said = frame(Element::new(Covered), 1.0, &mut cache, &[], mouse::Cursor::Unavailable);
        let at = Point::new(500.0, 300.0);
        assert_eq!(
            said,
            [
                Said::Sample(0.5),
                Said::Sample(0.5),
                Said::Under(mouse::Event::CursorMoved { position: at }, Some(at)),
                Said::Under(mouse::Event::ButtonPressed(mouse::Button::Left), Some(at)),
            ]
        );
    }

    /// iced updates again for the same redraw whenever an update for it
    /// published anything, and warns when the third still does. A pen in
    /// mid stroke has more to say every time it is asked.
    #[test]
    fn the_pen_is_asked_once_a_frame_however_often_the_frame_is_updated() {
        for content in [|| Element::new(Probe), || Element::new(Covered)] {
            forget();
            let mut cache = Cache::default();
            let nowhere = mouse::Cursor::Unavailable;
            let this_frame = Instant::now();
            pose(500.0, 300.0, 0.2);
            say(Raw::Frame);
            let first = frame_at(content(), 1.0, &mut cache, &[], nowhere, this_frame);
            assert!(first.contains(&Said::Sample(0.2)), "{first:?}");

            // More came while the first pass was acted on.
            pose(510.0, 300.0, 0.4);
            say(Raw::Frame);
            let second = frame_at(content(), 1.0, &mut cache, &[], nowhere, this_frame);
            assert_eq!(second, [], "the same redraw, updated again");

            // It is the next frame's.
            let next = frame(content(), 1.0, &mut cache, &[], nowhere);
            assert!(next.contains(&Said::Sample(0.4)), "{next:?}");
        }
    }

    /// The redraw itself is not taken when a press inside it is: iced drops
    /// the overlay for the frame when the root takes the event it was
    /// updated for.
    #[test]
    fn a_press_taken_inside_a_redraw_does_not_take_the_redraw() {
        forget();
        let mut renderer = ();
        let mut said = Vec::new();
        let mut interface = UserInterface::build(
            layer(a_button(), 1.0, sample),
            WINDOW,
            Cache::default(),
            &mut renderer,
        );
        say(Raw::Down);
        pose(50.0, 20.0, 0.6);
        say(Raw::Frame);
        let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));
        let (_, statuses) = interface.update(
            &[redraw],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut clipboard::Null,
            &mut said,
        );
        assert_eq!(statuses, [iced::event::Status::Ignored]);
    }

    /// What is inside keeps the state it had: the layer is not in the tree,
    /// so putting it round an interface moves nothing in it.
    #[test]
    fn the_layer_adds_nothing_to_the_tree() {
        let inside = a_button();
        let tag = inside.as_widget().tag();
        let children = inside.as_widget().children().len();
        let wrapped: Element<'_, Said, Theme, ()> = layer(inside, 1.0, sample);
        assert_eq!(wrapped.as_widget().tag(), tag);
        assert_eq!(wrapped.as_widget().children().len(), children);
    }
}
