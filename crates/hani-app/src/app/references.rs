// SPDX-License-Identifier: AGPL-3.0-only

//! Reference pictures on the front, side and top planes, and the floor.
//!
//! The pictures themselves, and where they stand, are the document's: they
//! are `hani_core::reference::References`, saved in the `.hani` file with
//! the sculpt. What is here is the panel that places them, the decoding of a
//! picture's bytes into texels for the renderer, and the floor's grid, which
//! is overlay geometry like a mirror plane's.
//!
//! The renderer draws the pictures in a pass of their own before the
//! sculpt's, so the model is always drawn over them. See
//! `hani-gpu/src/reference.rs`.

use std::sync::Arc;

use glam::Vec3;
use hani_core::reference::{Plane, Reference, References};
use hani_gpu::{MOST_PIXELS_ACROSS, OverlayBatch, ReferencePicture, ReferenceQuad};
use iced::widget::{column, row, text};
use iced::{Element, Task};

use super::Hani;
use crate::message::{Message, ReferenceMessage};
use crate::theme;

/// A picture taken in: its name, the file's bytes, and its texels.
pub type Taken = (String, Arc<[u8]>, Arc<ReferencePicture>);

/// What the panel is showing, and the pictures decoded.
#[derive(Debug)]
pub struct ReferencesState {
    /// Which plane the panel is placing.
    pub editing: Plane,
    /// Each plane's picture as texels, decoded from the document's bytes.
    /// Held so a change of placement does not decode again.
    decoded: [Option<Arc<ReferencePicture>>; 3],
}

impl Default for ReferencesState {
    fn default() -> Self {
        Self { editing: Plane::Front, decoded: [None, None, None] }
    }
}

/// A picture file's bytes as texels, no more than [`MOST_PIXELS_ACROSS`] on
/// a side.
pub fn decode(bytes: &[u8]) -> Result<ReferencePicture, String> {
    let picture = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let (width, height) = (picture.width(), picture.height());
    if width == 0 || height == 0 {
        return Err("the picture is empty".to_string());
    }
    let longest = width.max(height);
    let picture = if longest > MOST_PIXELS_ACROSS {
        let scale = MOST_PIXELS_ACROSS as f32 / longest as f32;
        let fit = |side: u32| ((side as f32 * scale).round() as u32).clamp(1, MOST_PIXELS_ACROSS);
        picture.resize_exact(fit(width), fit(height), image::imageops::FilterType::Triangle)
    } else {
        picture
    };
    let rgba = picture.into_rgba8();
    Ok(ReferencePicture { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

/// Ask for a picture, read and decode it.
async fn take_in() -> Option<Result<Taken, String>> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Reference picture")
        .add_filter("Picture", &["png", "jpg", "jpeg"])
        .pick_file()
        .await?;
    let name = handle.file_name();
    let bytes = handle.read().await;
    if bytes.len() > hani_core::reference::MAX_PICTURE_BYTES {
        return Some(Err("the picture is larger than 64 MB".to_string()));
    }
    Some(decode(&bytes).map(|picture| (name, Arc::from(bytes), Arc::new(picture))))
}

/// A picture's four corners, bottom left first, as seen from the view that
/// faces its plane.
pub fn corners(plane: Plane, reference: &Reference, aspect: f32) -> [Vec3; 4] {
    let (across, up) = plane.across_and_up();
    let middle =
        across * reference.centre[0] + up * reference.centre[1] + plane.normal() * reference.depth;
    let half_across = across * reference.width * 0.5;
    let half_up = up * reference.width / aspect.max(1.0e-3) * 0.5;
    [
        middle - half_across - half_up,
        middle + half_across - half_up,
        middle + half_across + half_up,
        middle - half_across + half_up,
    ]
}

/// A round number of millimetres between floor lines, so that a floor
/// `span` across has between ten and twenty-five of them.
pub fn floor_step(span: f32) -> f32 {
    let rough = (span / 12.0).max(0.1);
    let decade = 10f32.powf(rough.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|each| each * decade)
        .find(|step| *step >= rough)
        .unwrap_or(10.0 * decade)
}

/// Floor lines' colour: quiet, under the model.
const FLOOR_ALPHA: f32 = 0.22;
/// The two lines through the middle, a little stronger.
const FLOOR_AXIS_ALPHA: f32 = 0.45;

impl Hani {
    /// The references section of the panel.
    pub(super) fn references_panel(&self) -> Element<'_, Message> {
        let references = &self.references;
        let plane = self.reference_view.editing;
        let floor = iced::widget::checkbox(references.floor)
            .label("Floor")
            .on_toggle(|on| Message::Reference(ReferenceMessage::Floor(on)))
            .text_size(theme::LABEL_SIZE);
        let planes =
            crate::ui::segmented(Plane::ALL.map(|each| (each, each.label())), plane, |each| {
                Message::Reference(ReferenceMessage::Editing(each))
            });
        let mut block = column![floor, planes].spacing(theme::S2);
        let Some(reference) = references.on(plane) else {
            return block
                .push(crate::ui::button_secondary(
                    "Load picture…",
                    Some(Message::Reference(ReferenceMessage::Load)),
                ))
                .into();
        };
        let keys = drafts_keys(plane);
        let send = |make: fn(f32) -> ReferenceMessage| move |value| Message::Reference(make(value));
        let mm = |value: f32| format!("{value:.0} mm");
        block = block
            .push(text(reference.name.clone()).size(theme::LABEL_SIZE).color(theme::TEXT_DIM))
            .push(
                row![
                    iced::widget::checkbox(reference.shown)
                        .label("Shown")
                        .on_toggle(|on| Message::Reference(ReferenceMessage::Shown(on)))
                        .text_size(theme::LABEL_SIZE),
                    iced::widget::checkbox(reference.flipped)
                        .label("Flip")
                        .on_toggle(|on| Message::Reference(ReferenceMessage::Flipped(on)))
                        .text_size(theme::LABEL_SIZE),
                ]
                .spacing(theme::S3),
            )
            .push(
                crate::ui::number(
                    &self.drafts,
                    keys[0],
                    "Opacity",
                    reference.opacity,
                    0.0..=1.0,
                    send(ReferenceMessage::Opacity),
                )
                .step(0.05)
                .shown(format!("{:.0}%", reference.opacity * 100.0)),
            )
            .push(
                crate::ui::number(
                    &self.drafts,
                    keys[1],
                    "Width",
                    reference.width,
                    1.0..=2000.0,
                    send(ReferenceMessage::Width),
                )
                .step(1.0)
                .shown(mm(reference.width)),
            )
            .push(
                crate::ui::number(
                    &self.drafts,
                    keys[2],
                    "Across",
                    reference.centre[0],
                    -1000.0..=1000.0,
                    send(ReferenceMessage::Across),
                )
                .step(0.5)
                .shown(mm(reference.centre[0])),
            )
            .push(
                crate::ui::number(
                    &self.drafts,
                    keys[3],
                    "Up",
                    reference.centre[1],
                    -1000.0..=1000.0,
                    send(ReferenceMessage::Up),
                )
                .step(0.5)
                .shown(mm(reference.centre[1])),
            )
            .push(
                crate::ui::number(
                    &self.drafts,
                    keys[4],
                    "Depth",
                    reference.depth,
                    -1000.0..=1000.0,
                    send(ReferenceMessage::Depth),
                )
                .step(0.5)
                .shown(mm(reference.depth)),
            )
            .push(
                row![
                    crate::ui::button_secondary(
                        "Replace…",
                        Some(Message::Reference(ReferenceMessage::Load))
                    ),
                    crate::ui::button_secondary(
                        "Remove",
                        Some(Message::Reference(ReferenceMessage::Remove))
                    ),
                ]
                .spacing(theme::S2),
            );
        block.into()
    }

    pub(super) fn reference_changed(&mut self, message: ReferenceMessage) -> Task<Message> {
        let plane = self.reference_view.editing;
        match message {
            ReferenceMessage::Editing(each) => {
                self.reference_view.editing = each;
                return Task::none();
            }
            ReferenceMessage::Floor(on) => {
                self.references.floor = on;
                self.unsaved = true;
                self.refresh_overlay();
                return Task::none();
            }
            ReferenceMessage::Load => {
                return Task::perform(take_in(), move |taken| {
                    Message::Reference(ReferenceMessage::Loaded(plane, taken))
                });
            }
            ReferenceMessage::Loaded(onto, taken) => {
                match taken {
                    None => {}
                    Some(Err(error)) => {
                        self.status = format!("could not read the picture: {error}")
                    }
                    Some(Ok((name, bytes, picture))) => {
                        self.place_reference(onto, name, bytes, picture);
                        self.reference_view.editing = onto;
                    }
                }
                return Task::none();
            }
            ReferenceMessage::Remove => {
                self.references.planes[plane.index()] = None;
                self.reference_view.decoded[plane.index()] = None;
                self.shared.set_reference_picture(plane.index(), None);
                self.status = format!("removed the {} picture", plane.label().to_lowercase());
            }
            other => {
                let Some(reference) = self.references.on_mut(plane) else {
                    return Task::none();
                };
                match other {
                    ReferenceMessage::Shown(on) => reference.shown = on,
                    ReferenceMessage::Flipped(on) => reference.flipped = on,
                    ReferenceMessage::Opacity(value) => reference.opacity = value.clamp(0.0, 1.0),
                    ReferenceMessage::Width(value) => reference.width = value.max(1.0),
                    ReferenceMessage::Across(value) => reference.centre[0] = value,
                    ReferenceMessage::Up(value) => reference.centre[1] = value,
                    ReferenceMessage::Depth(value) => reference.depth = value,
                    _ => {}
                }
            }
        }
        self.unsaved = true;
        self.publish_reference_quads();
        Task::none()
    }

    /// A picture newly taken in, set on its plane: as tall as the model,
    /// centred on it, and standing just behind it as the plane's view sees.
    fn place_reference(
        &mut self,
        plane: Plane,
        name: String,
        bytes: Arc<[u8]>,
        picture: Arc<ReferencePicture>,
    ) {
        let aspect = picture.width as f32 / picture.height as f32;
        let (low, high) = self
            .doc
            .world_bounds()
            .unwrap_or((Vec3::splat(-super::MODEL_RADIUS_MM), Vec3::splat(super::MODEL_RADIUS_MM)));
        let (across, up) = plane.across_and_up();
        let middle = (low + high) * 0.5;
        let size = high - low;
        let tall = size.dot(up.abs()).max(1.0) * 1.2;
        let axis = plane.normal_axis();
        let behind = low[axis] - size.max_element() * 0.1;
        let mut reference = Reference::new(name.clone(), bytes, behind, tall * aspect);
        reference.centre = [middle.dot(across), middle.dot(up)];
        self.references.planes[plane.index()] = Some(reference);
        self.shared.set_reference_picture(plane.index(), Some(picture.clone()));
        self.reference_view.decoded[plane.index()] = Some(picture);
        self.unsaved = true;
        self.status = format!("{name} on the {} plane", plane.label().to_lowercase());
        self.publish_reference_quads();
    }

    /// Take a document's references as they were read: decode every picture
    /// and hand it to the renderer. A picture that will not decode is
    /// dropped, and the status says so.
    pub(super) fn adopt_references(&mut self, references: References) {
        self.references = references;
        let mut unreadable = Vec::new();
        for plane in Plane::ALL {
            let decoded =
                self.references.on(plane).and_then(|reference| match decode(&reference.picture) {
                    Ok(picture) => Some(Arc::new(picture)),
                    Err(_) => {
                        unreadable.push(reference.name.clone());
                        None
                    }
                });
            if decoded.is_none() {
                self.references.planes[plane.index()] = None;
            }
            self.shared.set_reference_picture(plane.index(), decoded.clone());
            self.reference_view.decoded[plane.index()] = decoded;
        }
        if !unreadable.is_empty() {
            self.status = format!("could not read the reference {}", unreadable.join(", "));
        }
        self.publish_reference_quads();
        self.refresh_overlay();
    }

    /// Where each shown picture is drawn, to the renderer.
    pub(super) fn publish_reference_quads(&self) {
        let quads: Vec<ReferenceQuad> = Plane::ALL
            .into_iter()
            .filter_map(|plane| {
                let reference = self.references.on(plane).filter(|reference| reference.shown)?;
                let picture = self.reference_view.decoded[plane.index()].as_ref()?;
                let aspect = picture.width as f32 / picture.height as f32;
                Some(ReferenceQuad {
                    slot: plane.index(),
                    corners: corners(plane, reference, aspect),
                    opacity: reference.opacity,
                    flipped: reference.flipped,
                })
            })
            .collect();
        self.shared.set_reference_quads(&quads);
    }

    /// The floor's grid, under the lowest point of the model, when it is on.
    pub(super) fn add_floor(&self, batch: &mut OverlayBatch) {
        if !self.references.floor {
            return;
        }
        let (low, high) = self
            .doc
            .world_bounds()
            .unwrap_or((Vec3::splat(-super::MODEL_RADIUS_MM), Vec3::splat(super::MODEL_RADIUS_MM)));
        let reach = ((high - low).max_element() * 0.5).max(5.0) * 1.6;
        let step = floor_step(reach * 2.0);
        // Lines on whole steps from the origin, so the floor does not slide
        // as the model grows.
        let first = |centre: f32| ((centre - reach) / step).floor() * step;
        let (x0, z0) = (first(0.0f32.min(low.x)), first(0.0f32.min(low.z)));
        let (x1, z1) = (0.0f32.max(high.x) + reach, 0.0f32.max(high.z) + reach);
        let y = low.y;
        let colour = theme::linear(theme::TEXT_DIM, FLOOR_ALPHA);
        let axis_colour = theme::linear(theme::TEXT_DIM, FLOOR_AXIS_ALPHA);
        let mut x = x0;
        while x <= x1 {
            let ink = if x.abs() < step * 0.01 { axis_colour } else { colour };
            batch.push_line(Vec3::new(x, y, z0), Vec3::new(x, y, z1), ink);
            x += step;
        }
        let mut z = z0;
        while z <= z1 {
            let ink = if z.abs() < step * 0.01 { axis_colour } else { colour };
            batch.push_line(Vec3::new(x0, y, z), Vec3::new(x1, y, z), ink);
            z += step;
        }
    }
}

/// The drafts keys of one plane's five numbers.
fn drafts_keys(plane: Plane) -> [&'static str; 5] {
    match plane {
        Plane::Front => [
            "reference.front.opacity",
            "reference.front.width",
            "reference.front.across",
            "reference.front.up",
            "reference.front.depth",
        ],
        Plane::Side => [
            "reference.side.opacity",
            "reference.side.width",
            "reference.side.across",
            "reference.side.up",
            "reference.side.depth",
        ],
        Plane::Top => [
            "reference.top.opacity",
            "reference.top.width",
            "reference.top.across",
            "reference.top.up",
            "reference.top.depth",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// A small PNG, as a file's bytes.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let picture = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 40, 40, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(picture)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("a png writes");
        bytes
    }

    fn taken(width: u32, height: u32) -> Taken {
        let bytes = png(width, height);
        let picture = decode(&bytes).expect("it decodes");
        ("front.png".to_string(), Arc::from(bytes), Arc::new(picture))
    }

    #[test]
    fn a_large_picture_is_brought_down_to_what_every_adapter_takes() {
        let picture = decode(&png(3000, 1500)).unwrap();
        assert_eq!((picture.width, picture.height), (MOST_PIXELS_ACROSS, MOST_PIXELS_ACROSS / 2));
        assert_eq!(picture.rgba.len(), (picture.width * picture.height * 4) as usize);
        assert!(decode(b"not a picture").is_err());
    }

    #[test]
    fn a_picture_taken_in_stands_behind_the_model_as_tall_as_it_and_is_published() {
        let mut app = app();
        update(
            &mut app,
            Message::Reference(ReferenceMessage::Loaded(Plane::Front, Some(Ok(taken(40, 20))))),
        );
        let reference = app.references.on(Plane::Front).expect("the picture was placed");
        let (low, high) = app.doc.world_bounds().unwrap();
        assert!(reference.depth < low.z, "the front picture is not behind the model");
        assert!(reference.width / 2.0 > (high.y - low.y), "not as tall as the model");
        assert!(app.unsaved);

        let quads = app.shared.reference_quads();
        assert_eq!(quads.len(), 1);
        let [bottom_left, bottom_right, top_right, _] = quads[0].corners;
        assert!((bottom_right - bottom_left).x > 0.0, "the front picture runs backwards");
        assert!((top_right - bottom_right).y > 0.0, "the front picture is upside down");
        assert!((bottom_right.x - bottom_left.x - reference.width).abs() < 1.0e-3);
    }

    #[test]
    fn hidden_or_removed_pictures_are_not_drawn() {
        let mut app = app();
        update(
            &mut app,
            Message::Reference(ReferenceMessage::Loaded(Plane::Side, Some(Ok(taken(8, 8))))),
        );
        assert_eq!(app.reference_view.editing, Plane::Side);
        update(&mut app, Message::Reference(ReferenceMessage::Shown(false)));
        assert!(app.shared.reference_quads().is_empty(), "a hidden picture was drawn");
        update(&mut app, Message::Reference(ReferenceMessage::Shown(true)));
        assert_eq!(app.shared.reference_quads().len(), 1);
        update(&mut app, Message::Reference(ReferenceMessage::Remove));
        assert!(app.references.on(Plane::Side).is_none());
        assert!(app.shared.reference_quads().is_empty());
    }

    #[test]
    fn references_are_saved_with_the_sculpt_and_come_back_on_open() {
        let directory =
            std::env::temp_dir().join(format!("hani-references-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("with-references.hani");

        let mut app = app();
        update(
            &mut app,
            Message::Reference(ReferenceMessage::Loaded(Plane::Top, Some(Ok(taken(16, 12))))),
        );
        update(&mut app, Message::Reference(ReferenceMessage::Opacity(0.8)));
        update(&mut app, Message::Reference(ReferenceMessage::Floor(true)));
        app.save_project(&path);

        let mut reopened = Hani::with_tablet(crate::tablet::Tablet::inert());
        reopened.open_project(&path);
        assert_eq!(reopened.references, app.references, "the references did not come back");
        assert!(reopened.references.floor);
        assert_eq!(reopened.shared.reference_quads().len(), 1, "the reopened picture is not drawn");

        // And a new sculpt starts with none.
        update(&mut reopened, Message::NewSculpt);
        if reopened.confirm.is_some() {
            drop(reopened.run_pending(super::super::PendingAction::NewSculpt));
        }
        assert!(reopened.references.is_empty(), "a new sculpt kept the references");
        assert!(reopened.shared.reference_quads().is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_floor_is_drawn_under_the_model_only_when_it_is_on() {
        let mut app = app();
        let mut batch = OverlayBatch::default();
        app.add_floor(&mut batch);
        assert!(batch.lines.is_empty());
        app.references.floor = true;
        app.add_floor(&mut batch);
        assert!(batch.lines.len() >= 20, "too few floor lines: {}", batch.lines.len() / 2);
        let low = app.doc.world_bounds().unwrap().0.y;
        assert!(batch.lines.iter().all(|vertex| (vertex.position[1] - low).abs() < 1.0e-4));
    }

    #[test]
    fn floor_steps_are_round_numbers() {
        for (span, step) in [(60.0, 5.0), (120.0, 10.0), (240.0, 20.0), (6.0, 0.5), (1000.0, 100.0)]
        {
            assert_eq!(floor_step(span), step, "span {span}");
        }
    }
}
