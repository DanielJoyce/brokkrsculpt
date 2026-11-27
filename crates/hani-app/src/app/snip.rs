// SPDX-License-Identifier: AGPL-3.0-only

//! Snip and report: a part of the window, drawn on and described, as a new
//! issue on GitHub.
//!
//! Asked for by the maintainer on 2026-11-26: *"Is it possible to have a key
//! like Ctrl-? And when I press it I want to be able to snapshot part of the
//! window ( drag a rectangle ) and once I finish the drag there is a popup
//! with the picture and a space for me to comment to describe how I want it
//! different. And maybe a way to kinda draw on the snapshot to annotate
//! more. And then have a button/link to create a new issue on GitHub with
//! the attached items?"*
//!
//! Ctrl and ? (Help, *Snip and report…*, too) freezes the window: iced takes
//! a screenshot and it is drawn over everything, so what is dragged across
//! is what was there. A drag chooses a part of it; a click with no drag takes
//! the whole window. The part comes up on a card, to draw on in one of three
//! inks, with a title and a comment.
//!
//! # Why the issue opens in the browser and is not made through the API
//!
//! GitHub's API makes no issue without a token, public repository or not,
//! and it has no way to attach a picture to an issue at all: the web page
//! uploads a pasted picture to its own store, which the API does not offer.
//! A token in the application would be a secret anyone could read out of the
//! binary. So *Open an issue* puts the picture, with what was drawn on it, on
//! the clipboard and opens GitHub's new-issue page with the title and the
//! comment already in it; one paste adds the picture.
//!
//! Not in a browser: iced reads a screenshot back by mapping a GPU buffer
//! and waiting for it, which WebGPU refuses. The pieces that would serve it
//! there (a download in place of the clipboard) are written, so that a
//! screenshot taken another way could use them.

// In a browser nothing starts a snip (see above), so its stages go unused
// there; natively every item is used.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

#[cfg(not(target_arch = "wasm32"))]
use std::borrow::Cow;

use iced::widget::{button, canvas, column, container, row, space, text, text_editor, text_input};
use iced::{Alignment, Color, Element, Length, Point, Rectangle, Size, Task, mouse};

use super::Hani;
use crate::message::{Message, SnipInk, SnipMessage};
use crate::theme;

/// The largest the picture is shown on the card, in logical pixels.
const SHOWN: Size = Size { width: 640.0, height: 400.0 };

/// A drag shorter than this either way is a click, which takes the whole
/// window.
const CLICK: f32 = 6.0;

/// Where snipping has got to.
#[derive(Debug, Default)]
pub(crate) enum Stage {
    #[default]
    Off,
    /// The screenshot has been asked for.
    Waiting,
    /// The window, frozen, with the part being dragged out.
    Choosing { shot: Shot, from: Option<Point>, to: Option<Point> },
    /// The part, to draw on and describe.
    Card(Box<Card>),
}

/// The frozen window.
#[derive(Debug)]
pub(crate) struct Shot {
    screenshot: iced::window::Screenshot,
    handle: iced::widget::image::Handle,
}

/// The card.
#[derive(Debug)]
pub(crate) struct Card {
    /// The part, as taken, RGBA.
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    handle: iced::widget::image::Handle,
    /// What has been drawn, in the picture's own pixels.
    strokes: Vec<Stroke>,
    drawing: bool,
    ink: SnipInk,
    title: String,
    comment: text_editor::Content,
}

#[derive(Debug, Clone)]
struct Stroke {
    ink: SnipInk,
    points: Vec<Point>,
}

/// Snip's state.
#[derive(Default)]
pub(crate) struct SnipState {
    pub(crate) stage: Stage,
    /// Kept for as long as the application runs: on Linux the clipboard is
    /// served by whoever wrote it, and a dropped writer can take the
    /// picture with it.
    #[cfg(not(target_arch = "wasm32"))]
    clipboard: Option<arboard::Clipboard>,
}

impl std::fmt::Debug for SnipState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnipState").field("stage", &self.stage).finish_non_exhaustive()
    }
}

fn colour(ink: SnipInk) -> [u8; 4] {
    match ink {
        SnipInk::Orange => [0xff, 0x7a, 0x3c, 0xff],
        SnipInk::White => [0xff, 0xff, 0xff, 0xff],
        SnipInk::Black => [0x10, 0x10, 0x10, 0xff],
    }
}

/// The part of a screenshot a drag from `from` to `to` chose, in its
/// physical pixels: the corners in either order, kept on the picture, and
/// the whole of it for a click.
pub(crate) fn region(from: Point, to: Point, size: Size<u32>, scale: f32) -> Rectangle<u32> {
    let whole = Rectangle { x: 0, y: 0, width: size.width, height: size.height };
    if (to.x - from.x).abs() < CLICK && (to.y - from.y).abs() < CLICK {
        return whole;
    }
    let physical = |value: f32, most: u32| ((value * scale).round().max(0.0) as u32).min(most);
    let (left, right) = (from.x.min(to.x), from.x.max(to.x));
    let (top, bottom) = (from.y.min(to.y), from.y.max(to.y));
    let (x0, x1) = (physical(left, size.width), physical(right, size.width));
    let (y0, y1) = (physical(top, size.height), physical(bottom, size.height));
    if x1 <= x0 || y1 <= y0 {
        return whole;
    }
    Rectangle { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
}

/// The picture with the strokes drawn into it, RGBA.
fn flattened(width: u32, height: u32, rgba: &[u8], strokes: &[Stroke]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    // About as thick as the line looked on the card, at any size.
    let radius = (width.max(height) as f32 / 260.0).max(2.0);
    let mut stamp = |at: Point, ink: [u8; 4]| {
        let reach = radius.ceil() as i64;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if (dx * dx + dy * dy) as f32 > radius * radius {
                    continue;
                }
                let (x, y) = (at.x.round() as i64 + dx, at.y.round() as i64 + dy);
                if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
                    continue;
                }
                let at = (y as usize * width as usize + x as usize) * 4;
                out[at..at + 4].copy_from_slice(&ink);
            }
        }
    };
    for stroke in strokes {
        let ink = colour(stroke.ink);
        let mut last: Option<Point> = None;
        for &point in &stroke.points {
            match last {
                None => stamp(point, ink),
                Some(from) => {
                    let steps = (from.distance(point) / 0.5).ceil().max(1.0) as usize;
                    for step in 1..=steps {
                        let t = step as f32 / steps as f32;
                        stamp(
                            Point::new(
                                from.x + (point.x - from.x) * t,
                                from.y + (point.y - from.y) * t,
                            ),
                            ink,
                        );
                    }
                }
            }
            last = Some(point);
        }
    }
    out
}

/// The longest comment put in the link: GitHub turns away a new-issue link
/// much past eight thousand characters, and a comment encodes to up to
/// three times its length.
const COMMENT_CHARS: usize = 2000;

/// GitHub's new-issue page with the title and the body in it.
pub(crate) fn issue_link(title: &str, comment: &str, about: &str) -> String {
    let title = if title.trim().is_empty() { "From a snip" } else { title.trim() };
    let comment: String = comment.trim().chars().take(COMMENT_CHARS).collect();
    let body =
        format!("{comment}\n\n(The picture is on the clipboard: paste it here.)\n\n---\n{about}");
    crate::links::new_issue(title, &body)
}

/// The application and the machine, in one line, for the foot of an issue.
fn about() -> String {
    format!(
        "HaniSculpt {} ({}), build {}, {} {}",
        env!("CARGO_PKG_VERSION"),
        super::build_commit(),
        super::build_number().map_or_else(|| "none".to_string(), |n| n.to_string()),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

impl Card {
    fn png(&self) -> Result<Vec<u8>, String> {
        let flat = flattened(self.width, self.height, &self.rgba, &self.strokes);
        let mut png = Vec::new();
        image::write_buffer_with_format(
            &mut std::io::Cursor::new(&mut png),
            &flat,
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|error| error.to_string())?;
        Ok(png)
    }
}

impl Hani {
    /// Whether snipping is under way, which holds every other key back.
    pub(super) fn snipping(&self) -> bool {
        !matches!(self.snip.stage, Stage::Off)
    }

    /// Escape: stop choosing a part, and say whether snipping was under way.
    /// The card stays, as the bug report's does: it holds what was typed.
    pub(super) fn snip_escaped(&mut self) -> bool {
        match self.snip.stage {
            Stage::Off => false,
            Stage::Card(_) => true,
            Stage::Waiting | Stage::Choosing { .. } => {
                self.snip.stage = Stage::Off;
                self.status = "snip put away".into();
                true
            }
        }
    }

    pub(super) fn snip_changed(&mut self, message: SnipMessage) -> Task<Message> {
        match message {
            // iced reads a screenshot back by mapping a buffer and waiting,
            // which WebGPU refuses ("getMappedRange failed", seen in
            // Chromium), so there is no snip in a browser.
            #[cfg(target_arch = "wasm32")]
            SnipMessage::Started => {
                self.status = "snip and report is in the desktop version".into();
            }
            #[cfg(not(target_arch = "wasm32"))]
            SnipMessage::Started => {
                self.top_menu = None;
                self.menu = None;
                self.snip.stage = Stage::Waiting;
                return iced::window::latest()
                    .and_then(iced::window::screenshot)
                    .map(|shot| Message::Snip(SnipMessage::Captured(shot)));
            }
            SnipMessage::Captured(screenshot) => {
                if !matches!(self.snip.stage, Stage::Waiting) {
                    return Task::none();
                }
                let handle = iced::widget::image::Handle::from_rgba(
                    screenshot.size.width,
                    screenshot.size.height,
                    screenshot.rgba.clone(),
                );
                self.snip.stage =
                    Stage::Choosing { shot: Shot { screenshot, handle }, from: None, to: None };
                self.status = "drag across a part of the window, or click for all of it".into();
            }
            SnipMessage::Corner(at) => {
                if let Stage::Choosing { from, to, .. } = &mut self.snip.stage {
                    *from = Some(at);
                    *to = Some(at);
                }
            }
            SnipMessage::Dragged(at) => {
                if let Stage::Choosing { from: Some(_), to, .. } = &mut self.snip.stage {
                    *to = Some(at);
                }
            }
            SnipMessage::Chosen => self.snip_chosen(),
            SnipMessage::Ink(ink) => {
                if let Stage::Card(card) = &mut self.snip.stage {
                    card.ink = ink;
                }
            }
            SnipMessage::StrokeBegun(at) => {
                if let Stage::Card(card) = &mut self.snip.stage {
                    card.strokes.push(Stroke { ink: card.ink, points: vec![at] });
                    card.drawing = true;
                }
            }
            SnipMessage::StrokeMoved(at) => {
                if let Stage::Card(card) = &mut self.snip.stage
                    && card.drawing
                    && let Some(stroke) = card.strokes.last_mut()
                {
                    stroke.points.push(at);
                }
            }
            SnipMessage::StrokeEnded => {
                if let Stage::Card(card) = &mut self.snip.stage {
                    card.drawing = false;
                }
            }
            SnipMessage::Undone => {
                if let Stage::Card(card) = &mut self.snip.stage {
                    card.strokes.pop();
                }
            }
            SnipMessage::TitleTyped(title) => {
                if let Stage::Card(card) = &mut self.snip.stage {
                    card.title = title;
                }
            }
            SnipMessage::Commented(action) => {
                if let Stage::Card(card) = &mut self.snip.stage {
                    card.comment.perform(action);
                }
            }
            SnipMessage::PictureCopied => {
                self.picture_out();
            }
            SnipMessage::PictureSaved => return self.save_picture(),
            SnipMessage::SaveChosen(path) => {
                if let Some(path) = path {
                    self.write_picture(&path);
                }
            }
            SnipMessage::IssueOpened => {
                let Stage::Card(card) = &self.snip.stage else {
                    return Task::none();
                };
                let link = issue_link(&card.title, &card.comment.text(), &about());
                if self.picture_out()
                    && let Err(why) = crate::links::open_in_browser(&link)
                {
                    self.status = why;
                }
            }
            SnipMessage::Closed => self.snip.stage = Stage::Off,
        }
        Task::none()
    }

    /// The drag is done: the part of the frozen window it chose, on a card.
    fn snip_chosen(&mut self) {
        let Stage::Choosing { shot, from: Some(from), to: Some(to) } = &self.snip.stage else {
            return;
        };
        let screenshot = &shot.screenshot;
        let part = region(*from, *to, screenshot.size, screenshot.scale_factor);
        let Ok(cropped) = screenshot.crop(part) else {
            self.status = "could not take that part of the window".into();
            self.snip.stage = Stage::Off;
            return;
        };
        let (width, height) = (cropped.size.width, cropped.size.height);
        let rgba = cropped.rgba.to_vec();
        let handle = iced::widget::image::Handle::from_rgba(width, height, rgba.clone());
        self.snip.stage = Stage::Card(Box::new(Card {
            width,
            height,
            rgba,
            handle,
            strokes: Vec::new(),
            drawing: false,
            ink: SnipInk::Orange,
            title: String::new(),
            comment: text_editor::Content::new(),
        }));
        self.status = format!("snipped {width} by {height}");
    }

    /// The picture, with what is drawn on it, to the clipboard; in a
    /// browser, downloaded. Says whether it went.
    fn picture_out(&mut self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Stage::Card(card) = &self.snip.stage else {
                return false;
            };
            let flat = flattened(card.width, card.height, &card.rgba, &card.strokes);
            let picture = arboard::ImageData {
                width: card.width as usize,
                height: card.height as usize,
                bytes: Cow::Owned(flat),
            };
            if self.snip.clipboard.is_none() {
                self.snip.clipboard = arboard::Clipboard::new().ok();
            }
            let copied = match self.snip.clipboard.as_mut() {
                Some(clipboard) => clipboard.set_image(picture).map_err(|error| error.to_string()),
                None => Err("there is no clipboard to write to".to_string()),
            };
            match copied {
                Ok(()) => {
                    self.status = "the picture is on the clipboard".into();
                    true
                }
                Err(why) => {
                    self.status = format!("could not copy the picture: {why}");
                    false
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if !matches!(self.snip.stage, Stage::Card(_)) {
                return false;
            }
            self.write_picture(std::path::Path::new("snip.png"));
            true
        }
    }

    /// Ask where the picture goes; in a browser it is a download.
    fn save_picture(&mut self) -> Task<Message> {
        #[cfg(target_arch = "wasm32")]
        {
            self.write_picture(std::path::Path::new("snip.png"));
            Task::none()
        }
        #[cfg(not(target_arch = "wasm32"))]
        Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .set_title("Save the picture")
                    .add_filter("PNG", &["png"])
                    .set_file_name("snip.png")
                    .save_file()
                    .await
                    .map(|handle| handle.path().to_path_buf())
            },
            |path| Message::Snip(SnipMessage::SaveChosen(path)),
        )
    }

    fn write_picture(&mut self, path: &std::path::Path) {
        let Stage::Card(card) = &self.snip.stage else {
            return;
        };
        let written = card.png().and_then(|png| {
            let mut out = crate::files::create(path).map_err(|error| error.to_string())?;
            out.write_all(&png).and_then(|()| out.flush()).map_err(|error| error.to_string())
        });
        self.status = match written {
            Ok(()) => format!("saved the picture to {}", path.display()),
            Err(why) => format!("could not save the picture: {why}"),
        };
    }

    /// The frozen window with the part being dragged out, over everything.
    pub(super) fn snip_overlay(&self) -> Option<Element<'_, Message>> {
        let Stage::Choosing { shot, from, to } = &self.snip.stage else {
            return None;
        };
        let chosen = from
            .zip(*to)
            .map(|(a, b)| Rectangle::new(a, Size::ZERO).union(&Rectangle::new(b, Size::ZERO)));
        // The picture as a widget under the canvas and not drawn in it: in
        // one layer iced draws images after paths, so the shading and the
        // outline went under the picture (seen on the screen). A stack puts
        // each layer over the one before.
        Some(
            iced::widget::stack![
                iced::widget::image(shot.handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Fill),
                canvas(Chooser { chosen }).width(Length::Fill).height(Length::Fill),
            ]
            .into(),
        )
    }

    /// The card, or nothing while there is none.
    pub(super) fn snip_card(&self) -> Option<Element<'_, Message>> {
        let Stage::Card(card) = &self.snip.stage else {
            return None;
        };
        let send = |message: SnipMessage| Message::Snip(message);
        let small = |label: &'static str, what: &'static str, message: SnipMessage| {
            crate::ui::tip(
                button(text(label).size(theme::LABEL_SIZE))
                    .padding(theme::PAD_BUTTON)
                    .style(theme::tool_button)
                    .on_press(send(message)),
                what,
            )
        };
        let scale =
            (SHOWN.width / card.width as f32).min(SHOWN.height / card.height as f32).min(1.0);
        let shown = Size::new(card.width as f32 * scale, card.height as f32 * scale);
        // The picture under the canvas, for the reason `snip_overlay` gives.
        let pad = iced::widget::stack![
            iced::widget::image(card.handle.clone())
                .width(Length::Fixed(shown.width))
                .height(Length::Fixed(shown.height))
                .content_fit(iced::ContentFit::Fill),
            canvas(Pad { card, scale })
                .width(Length::Fixed(shown.width))
                .height(Length::Fixed(shown.height)),
        ];
        let inks = crate::ui::segmented_tips(
            [
                (SnipInk::Orange, "Orange", "Mark the picture in orange"),
                (SnipInk::White, "White", "Mark the picture in white"),
                (SnipInk::Black, "Black", "Mark the picture in black"),
            ],
            card.ink,
            move |ink| send(SnipMessage::Ink(ink)),
        );
        let body = column![
            row![
                text("Snip and report").size(theme::TITLE_SIZE).color(theme::TEXT),
                space().width(Length::Fill),
                small("Close", "Put the snip away", SnipMessage::Closed),
            ]
            .align_y(Alignment::Center),
            container(pad).width(Length::Fill).center_x(Length::Fill),
            row![
                container(inks).width(Length::Fixed(240.0)),
                small("Undo", "Take back the last mark", SnipMessage::Undone),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
            text_input("Title", &card.title)
                .on_input(move |title| send(SnipMessage::TitleTyped(title)))
                .size(theme::LABEL_SIZE)
                .padding(theme::S2),
            text_editor(&card.comment)
                .placeholder("What should be different?")
                .on_action(move |action| send(SnipMessage::Commented(action)))
                .size(theme::LABEL_SIZE)
                .height(Length::Fixed(110.0)),
            row![
                small("Copy picture", "Copy the marked picture", SnipMessage::PictureCopied),
                small(
                    "Save picture…",
                    "Save the marked picture as a file",
                    SnipMessage::PictureSaved
                ),
                space().width(Length::Fill),
                crate::ui::tip(
                    button(text("Open an issue on GitHub").size(theme::LABEL_SIZE))
                        .padding(theme::PAD_BUTTON)
                        .style(theme::tool_button_active)
                        .on_press(send(SnipMessage::IssueOpened)),
                    "Open a new issue with the title and comment; paste the picture into it"
                ),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center),
        ]
        .spacing(theme::S3);
        let card = container(body).padding(theme::S5).max_width(720.0).style(theme::menu_card);
        Some(super::panel::modal_layer(card))
    }
}

use std::io::Write;

/// The frozen window and the part being dragged out of it.
struct Chooser {
    chosen: Option<Rectangle>,
}

/// Whether the button is down, which `CursorMoved` does not say.
#[derive(Default)]
struct Held(bool);

impl canvas::Program<Message> for Chooser {
    type State = Held;

    fn update(
        &self,
        held: &mut Held,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let at = cursor.position_in(bounds);
        let message = match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                held.0 = true;
                SnipMessage::Corner(at?)
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) if held.0 => {
                SnipMessage::Dragged(at?)
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if held.0 => {
                held.0 = false;
                SnipMessage::Chosen
            }
            _ => return None,
        };
        Some(canvas::Action::publish(Message::Snip(message)).and_capture())
    }

    fn draw(
        &self,
        _held: &Held,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let whole = Rectangle::new(Point::ORIGIN, bounds.size());
        let shade = Color { a: 0.45, ..Color::BLACK };
        match self.chosen {
            Some(part) => {
                // Everything but the part shaded, in four bands.
                let bands = [
                    Rectangle::new(Point::ORIGIN, Size::new(whole.width, part.y)),
                    Rectangle::new(
                        Point::new(0.0, part.y + part.height),
                        Size::new(whole.width, (whole.height - part.y - part.height).max(0.0)),
                    ),
                    Rectangle::new(Point::new(0.0, part.y), Size::new(part.x, part.height)),
                    Rectangle::new(
                        Point::new(part.x + part.width, part.y),
                        Size::new((whole.width - part.x - part.width).max(0.0), part.height),
                    ),
                ];
                for band in bands {
                    frame.fill_rectangle(band.position(), band.size(), shade);
                }
                frame.stroke(
                    &canvas::Path::rectangle(part.position(), part.size()),
                    canvas::Stroke::default().with_color(theme::ACCENT).with_width(2.0),
                );
            }
            None => frame.fill_rectangle(Point::ORIGIN, whole.size(), Color { a: 0.2, ..shade }),
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _held: &Held,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::Crosshair
    }
}

/// The picture, and what is drawn on it, to draw on.
struct Pad<'a> {
    card: &'a Card,
    /// Shown pixels per picture pixel.
    scale: f32,
}

impl canvas::Program<Message> for Pad<'_> {
    type State = Held;

    fn update(
        &self,
        held: &mut Held,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let at =
            cursor.position_in(bounds).map(|at| Point::new(at.x / self.scale, at.y / self.scale));
        let message = match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = at?;
                held.0 = true;
                SnipMessage::StrokeBegun(at)
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) if held.0 => {
                SnipMessage::StrokeMoved(at?)
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if held.0 => {
                held.0 = false;
                SnipMessage::StrokeEnded
            }
            _ => return None,
        };
        Some(canvas::Action::publish(Message::Snip(message)).and_capture())
    }

    fn draw(
        &self,
        _held: &Held,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let width =
            (self.card.width.max(self.card.height) as f32 / 260.0).max(2.0) * 2.0 * self.scale;
        for stroke in &self.card.strokes {
            let [r, g, b, _] = colour(stroke.ink);
            let path = canvas::Path::new(|line| {
                let mut points = stroke.points.iter();
                if let Some(first) = points.next() {
                    line.move_to(Point::new(first.x * self.scale, first.y * self.scale));
                    line.line_to(Point::new(first.x * self.scale + 0.01, first.y * self.scale));
                }
                for point in points {
                    line.line_to(Point::new(point.x * self.scale, point.y * self.scale));
                }
            });
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(Color::from_rgb8(r, g, b))
                    .with_width(width.max(1.5))
                    .with_line_cap(canvas::LineCap::Round)
                    .with_line_join(canvas::LineJoin::Round),
            );
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _held: &Held,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.welcome = false;
        app
    }

    fn snip(app: &mut Hani, message: SnipMessage) {
        drop(app.update(Message::Snip(message)));
    }

    /// A window of 200 by 100 logical pixels at a scale of two, each pixel's
    /// red its column and green its row, a quarter of the way.
    fn window() -> iced::window::Screenshot {
        let (width, height) = (400u32, 200u32);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[(x / 4) as u8, (y / 4) as u8, 0, 255]);
            }
        }
        iced::window::Screenshot::new(rgba, Size::new(width, height), 2.0)
    }

    #[test]
    fn a_drag_chooses_its_part_in_physical_pixels_and_a_click_the_whole() {
        let size = Size::new(400, 200);
        let part = region(Point::new(60.0, 50.0), Point::new(10.0, 20.0), size, 2.0);
        assert_eq!(part, Rectangle { x: 20, y: 40, width: 100, height: 60 });
        let past = region(Point::new(150.0, 50.0), Point::new(900.0, 900.0), size, 2.0);
        assert_eq!(part.y, 40);
        assert_eq!((past.x + past.width, past.y + past.height), (400, 200), "not kept on it");
        let click = region(Point::new(30.0, 30.0), Point::new(32.0, 31.0), size, 2.0);
        assert_eq!(click, Rectangle { x: 0, y: 0, width: 400, height: 200 });
    }

    #[test]
    fn a_link_carries_the_title_and_the_comment_and_is_one_the_application_opens() {
        let link =
            issue_link("Make it orange", "The ball & the cube\nshould be bigger?", "build 7");
        assert!(link.starts_with(crate::links::NEW_ISSUE));
        assert!(
            link.contains("title=Make%20it%20orange&body=The%20ball%20%26%20the%20cube%0Ashould")
        );
        assert!(link.contains("build%207"));
        assert!(crate::links::may_be_opened(&link), "{link}");
        assert!(issue_link("  ", "", "x").contains("title=From%20a%20snip"));
        let long = issue_link("t", &"é".repeat(5000), "x");
        assert!(long.len() < 8000 * 2, "{}", long.len());
    }

    #[test]
    fn a_stroke_is_drawn_into_the_picture_in_its_ink_and_nowhere_else() {
        let (width, height) = (40u32, 20u32);
        let rgba = vec![0u8; (width * height * 4) as usize];
        let stroke = Stroke {
            ink: SnipInk::Orange,
            points: vec![Point::new(5.0, 10.0), Point::new(35.0, 10.0)],
        };
        let flat = flattened(width, height, &rgba, &[stroke]);
        let at = |x: u32, y: u32| {
            let i = ((y * width + x) * 4) as usize;
            [flat[i], flat[i + 1], flat[i + 2], flat[i + 3]]
        };
        assert_eq!(at(20, 10), colour(SnipInk::Orange), "the middle of the line");
        assert_eq!(at(5, 10), colour(SnipInk::Orange), "its start");
        assert_eq!(at(20, 2), [0, 0, 0, 0], "far from it");
    }

    /// Started, frozen, dragged, drawn on, undone, described; Escape ends it.
    #[test]
    fn a_snip_is_frozen_dragged_drawn_on_and_closed() {
        let mut app = app();
        snip(&mut app, SnipMessage::Started);
        assert!(app.snipping() && app.modal_open());
        snip(&mut app, SnipMessage::Captured(window()));
        assert!(app.snip_overlay().is_some());
        snip(&mut app, SnipMessage::Corner(Point::new(10.0, 20.0)));
        snip(&mut app, SnipMessage::Dragged(Point::new(40.0, 30.0)));
        snip(&mut app, SnipMessage::Dragged(Point::new(60.0, 50.0)));
        snip(&mut app, SnipMessage::Chosen);
        let Stage::Card(card) = &app.snip.stage else { panic!("no card: {}", app.status) };
        assert_eq!((card.width, card.height), (100, 60));
        // The part's top left is the window's pixel (20, 40).
        assert_eq!(&card.rgba[..4], &[5, 10, 0, 255]);
        assert!(app.snip_card().is_some());

        snip(&mut app, SnipMessage::Ink(SnipInk::White));
        snip(&mut app, SnipMessage::StrokeBegun(Point::new(10.0, 10.0)));
        snip(&mut app, SnipMessage::StrokeMoved(Point::new(50.0, 30.0)));
        snip(&mut app, SnipMessage::StrokeEnded);
        snip(&mut app, SnipMessage::StrokeMoved(Point::new(90.0, 50.0)));
        snip(&mut app, SnipMessage::TitleTyped("Bigger".into()));
        let Stage::Card(card) = &app.snip.stage else { panic!() };
        assert_eq!(card.strokes.len(), 1);
        assert_eq!(card.strokes[0].points.len(), 2, "a move after the release was drawn");
        assert_eq!(card.strokes[0].ink, SnipInk::White);
        assert_eq!(card.title, "Bigger");
        assert!(card.png().is_ok_and(|png| png.starts_with(b"\x89PNG")));
        snip(&mut app, SnipMessage::Undone);
        let Stage::Card(card) = &app.snip.stage else { panic!() };
        assert!(card.strokes.is_empty());

        // Escape keeps the card, which holds what was typed; Close ends it.
        assert!(app.snip_escaped());
        assert!(app.snipping());
        snip(&mut app, SnipMessage::Closed);
        assert!(!app.snipping());
        // While choosing, Escape puts it away.
        snip(&mut app, SnipMessage::Started);
        snip(&mut app, SnipMessage::Captured(window()));
        assert!(app.snip_escaped() && !app.snipping());
    }

    #[test]
    fn control_and_question_mark_starts_a_snip() {
        for key in ["?", "/"] {
            assert!(matches!(
                crate::viewport::shortcut(key, true, true, false),
                Some(Message::Snip(SnipMessage::Started))
            ));
        }
        assert!(matches!(
            crate::viewport::shortcut("?", false, true, false),
            Some(Message::ShortcutsOpened)
        ));
    }
}
