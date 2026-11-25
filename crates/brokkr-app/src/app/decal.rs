// SPDX-License-Identifier: AGPL-3.0-only

//! The Decal touch: a picture dabbed onto the model. Stage 5 of
//! `docs/PAINT-RACK.md`, section 6; the dab itself is
//! [`brokkr_core::paint::decal`].
//!
//! Press where it goes; drag away to size it and round to turn it, and it
//! is dabbed again at each move, over the paint as it was, so what is left
//! when the button comes up is one dab. A click dabs it at the brush's
//! size. It goes on the active layer if that is a decal layer, or on a new
//! one over it named for the picture, and the new layer and the dab are
//! one undo entry.
//!
//! Pictures are the six that come with the application, and any picture
//! taken in, which is kept in `decals/` in the data directory as the
//! brushes are. Any picture can go on as a decal, in its own colours, or
//! as a stamp, a stencil in the paint in hand.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use brokkr_core::paint::decal::{BuiltInPicture, Dab, Picture};
use brokkr_core::{Change, Entry, NodeId};
use glam::{IVec3, Vec2};
use iced::widget::{button, column, row, text};
use iced::{Element, Length, Task};

use super::Brokkr;
use super::layers::Before;
use crate::message::{DecalMessage, DecalPick, Message, PaintTouch};
use crate::theme;

/// The directory of the user's own pictures, in the data directory.
const DIRECTORY: &str = "decals";
/// A drag shorter than this, in pixels, is a click.
const CLICK_PX: f32 = 4.0;
/// How soft an edge may be, as a share of the picture's size.
const SOFT_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.5;

/// A dab under way.
#[derive(Debug, Clone)]
struct Gesture {
    pressed: Vec2,
    dab: Dab,
    /// The box the last dab of it reached.
    reach: Option<(IVec3, IVec3)>,
}

/// The Decal touch's settings, and a dab under way.
#[derive(Debug)]
pub(super) struct DecalState {
    pub chosen: DecalPick,
    /// As a stamp in the paint in hand, rather than in its own colours.
    pub as_stamp: bool,
    pub soft: f32,
    /// The user's own pictures, by name.
    pub own: Vec<(String, Arc<Picture>)>,
    gesture: Option<Gesture>,
    /// The stack before a dab made its decal layer, for the one entry.
    made_layer: Option<Before>,
}

impl Default for DecalState {
    fn default() -> Self {
        let own =
            if cfg!(test) { Vec::new() } else { directory().map(load_all).unwrap_or_default() };
        Self {
            chosen: DecalPick::BuiltIn(BuiltInPicture::Star),
            as_stamp: false,
            soft: 0.0,
            own,
            gesture: None,
            made_layer: None,
        }
    }
}

impl DecalState {
    /// The picture chosen, and its name.
    fn picture(&self) -> (String, Arc<Picture>) {
        match self.chosen {
            DecalPick::BuiltIn(which) => (which.label().to_string(), Arc::new(which.picture())),
            DecalPick::Own(at) => match self.own.get(at) {
                Some((name, picture)) => (name.clone(), picture.clone()),
                None => ("Star".to_string(), Arc::new(BuiltInPicture::Star.picture())),
            },
        }
    }
}

fn directory() -> Option<PathBuf> {
    crate::paths::data_file(DIRECTORY)
}

/// A picture file as a picture, and its name.
fn read_picture(path: &Path) -> Result<(String, Arc<Picture>), String> {
    let image = image::open(path).map_err(|error| error.to_string())?.into_rgba8();
    let (width, height) = (image.width() as usize, image.height() as usize);
    if width == 0 || height == 0 {
        return Err("the picture is empty".to_string());
    }
    let rgba = image.pixels().map(|pixel| pixel.0).collect();
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "picture".to_string());
    Ok((name, Arc::new(Picture { width, height, rgba })))
}

/// Every picture in `directory`, by name.
fn load_all(directory: PathBuf) -> Vec<(String, Arc<Picture>)> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut pictures: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| {
                matches!(
                    extension.to_string_lossy().to_lowercase().as_str(),
                    "png" | "jpg" | "jpeg"
                )
            })
        })
        .filter_map(|path| read_picture(&path).ok())
        .collect();
    pictures.sort_by(|a, b| a.0.cmp(&b.0));
    pictures
}

/// Ask for a picture, read it, and keep a copy among the user's own.
async fn take_in() -> Option<Result<(String, Arc<Picture>), String>> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Decal picture")
        .add_filter("Picture", &["png", "jpg", "jpeg"])
        .pick_file()
        .await?;
    let read = read_picture(handle.path());
    if read.is_ok()
        && let (Some(directory), Some(name)) = (directory(), handle.path().file_name())
    {
        let _ = std::fs::create_dir_all(&directory);
        let _ = std::fs::copy(handle.path(), directory.join(name));
    }
    Some(read)
}

impl Brokkr {
    pub(super) fn decal_changed(&mut self, message: DecalMessage) -> Task<Message> {
        let decal = &mut self.painting.decal;
        match message {
            DecalMessage::Chosen(pick) => {
                decal.chosen = pick;
                if let DecalPick::BuiltIn(which) = pick {
                    decal.as_stamp = which.is_stamp();
                }
            }
            DecalMessage::AsStamp(on) => decal.as_stamp = on,
            DecalMessage::Soft(soft) => {
                decal.soft = soft.clamp(*SOFT_RANGE.start(), *SOFT_RANGE.end())
            }
            DecalMessage::TakeIn => {
                return Task::perform(take_in(), |taken| {
                    Message::Decal(DecalMessage::TakenIn(taken))
                });
            }
            DecalMessage::TakenIn(None) => {}
            DecalMessage::TakenIn(Some(Err(why))) => {
                self.status = format!("the picture could not be read: {why}");
            }
            DecalMessage::TakenIn(Some(Ok((name, picture)))) => {
                decal.own.retain(|(held, _)| *held != name);
                decal.own.push((name.clone(), picture));
                decal.chosen = DecalPick::Own(decal.own.len() - 1);
                self.status = format!("took in {name}");
            }
        }
        Task::none()
    }

    /// Whether a drag of the paint brush is a decal's.
    pub(super) fn dabbing(&self) -> bool {
        self.painting_colour() && self.touch.kind == PaintTouch::Decal
    }

    /// A press or a move of the Decal touch.
    pub(super) fn decal_to(&mut self, pixel: Vec2, start: bool) {
        if start {
            self.painting.decal.gesture = None;
            let Some(at) = self.surface_under(pixel) else {
                return;
            };
            let (_, picture) = self.painting.decal.picture();
            let (_, along) = self.ray_through(pixel);
            let across = |offset: Vec2| {
                let (_, ray) = self.ray_through(pixel + offset);
                (ray - along * ray.dot(along)).normalize_or_zero()
            };
            let (right, up) = (across(Vec2::new(10.0, 0.0)), across(Vec2::new(0.0, -10.0)));
            self.decal_layer();
            self.arm_recorder();
            let stamp = self
                .painting
                .decal
                .as_stamp
                .then(|| self.painting.pigment(brokkr_core::BrushDirection::Add));
            let dab = Dab {
                at,
                along,
                right,
                up,
                size: self.brush.radius * 2.0,
                turn: 0.0,
                soft: self.painting.decal.soft,
                stamp,
            };
            let reach = Some(self.doc.active_volume_mut().dab(&picture, &dab, None));
            self.painting.decal.gesture = Some(Gesture { pressed: pixel, dab, reach });
            self.remesh_dirty();
            return;
        }
        let Some(gesture) = self.painting.decal.gesture.clone() else {
            return;
        };
        let pulled = pixel - gesture.pressed;
        if pulled.length() < CLICK_PX {
            return;
        }
        // Pixels to the world at the depth of the point aimed at.
        let away = gesture.dab.at.distance(self.camera.eye());
        let per_pixel =
            2.0 * away * (self.camera.fov_y * 0.5).tan() / self.viewport_size.y.max(1.0);
        let dab = Dab {
            size: (pulled.length() * per_pixel * 2.0).max(self.doc.voxel_size()),
            turn: (-pulled.y).atan2(pulled.x),
            ..gesture.dab
        };
        let (_, picture) = self.painting.decal.picture();
        let reach = self.doc.active_volume_mut().dab(&picture, &dab, gesture.reach);
        self.painting.decal.gesture = Some(Gesture { reach: Some(reach), dab, ..gesture });
        self.remesh_dirty();
    }

    /// Make the active layer a decal layer, or a new one named for the
    /// picture over it, before a dab. Its entry waits for the dab's.
    fn decal_layer(&mut self) {
        let layers = self.doc.active_volume().layers();
        let active = layers.metas()[layers.active()].clone();
        if active.name.starts_with("Decal") {
            return;
        }
        let before = Before::Added(layers.metas(), layers.active_id());
        let (name, _) = self.painting.decal.picture();
        if self
            .doc
            .active_volume_mut()
            .add_layer(format!("Decal: {name}"), brokkr_core::paint::layers::Blend::Normal)
            .is_some()
        {
            self.painting.decal.made_layer = Some(before);
        }
    }

    /// The entry a stroke ends as: the stroke, and a decal layer it made
    /// first, as one. `None` when there is nothing to keep.
    pub(super) fn stroke_entry(
        &mut self,
        body: NodeId,
        edit: Option<brokkr_core::StrokeEdit>,
    ) -> Option<Entry> {
        self.painting.decal.gesture = None;
        let made = self.painting.decal.made_layer.take();
        let added = match made {
            Some(Before::Added(had, active)) => {
                let ids = self
                    .doc
                    .active_volume()
                    .layers()
                    .metas()
                    .into_iter()
                    .filter(|meta| !had.iter().any(|was| was.id == meta.id))
                    .map(|meta| meta.id)
                    .collect();
                Some(Change::LayersAdded { body, ids, active })
            }
            _ => None,
        };
        let changes: Vec<Change> =
            added.into_iter().chain(edit.map(|edit| Change::Bricks { body, edit })).collect();
        (!changes.is_empty()).then(|| Entry::new(changes))
    }

    /// The Decal touch's settings, under the touches.
    pub(super) fn decal_settings(&self) -> Element<'_, Message> {
        let decal = &self.painting.decal;
        let send = Message::Decal;
        let tile = |label: String, pick: DecalPick| {
            let (style, _) = theme::tool_toggle(decal.chosen == pick);
            button(text(label).size(theme::LABEL_SIZE).width(Length::Fill))
                .width(Length::Fill)
                .padding(theme::PAD_CHOICE)
                .style(style)
                .on_press(Message::Decal(DecalMessage::Chosen(pick)))
        };
        let mut picks: Vec<(String, DecalPick)> = BuiltInPicture::ALL
            .into_iter()
            .map(|which| (which.label().to_string(), DecalPick::BuiltIn(which)))
            .collect();
        picks.extend(
            decal.own.iter().enumerate().map(|(at, (name, _))| (name.clone(), DecalPick::Own(at))),
        );
        let mut grid = column![].spacing(theme::S1);
        for line in picks.chunks(3) {
            grid = grid.push(
                line.iter().fold(row![].spacing(theme::S1), |assembled, (label, pick)| {
                    assembled.push(tile(label.clone(), *pick))
                }),
            );
        }
        column![
            grid,
            crate::ui::button_secondary("Picture\u{2026}", Some(send(DecalMessage::TakeIn))),
            crate::ui::segmented([(false, "Decal"), (true, "Stamp")], decal.as_stamp, |on| {
                Message::Decal(DecalMessage::AsStamp(on))
            },),
            crate::ui::number(
                &self.drafts,
                "decal.soft",
                "Soft edge",
                decal.soft,
                SOFT_RANGE,
                |v| { Message::Decal(DecalMessage::Soft(v)) }
            )
            .shown(format!("{:.0}%", decal.soft * 100.0)),
        ]
        .spacing(theme::S2)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };
    const CENTRE: Vector = Vector { x: 400.0, y: 300.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(brokkr_core::BrushKind::Paint));
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Decal));
        app
    }

    fn painted(app: &Brokkr) -> usize {
        let volume = app.doc.active_volume();
        let mut count = 0;
        for coord in volume.brick_coords() {
            let origin = coord.origin();
            for index in 0..32 * 32 * 32 {
                let cell = origin + IVec3::new(index % 32, (index / 32) % 32, index / 1024);
                count += usize::from(volume.paint().at(cell)[3] > 0);
            }
        }
        count
    }

    fn dab(app: &mut Brokkr, to: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: CENTRE,
                size: SIZE,
            }),
        );
        update(app, Message::Pointer(PointerEvent::Moved { position: to, size: SIZE }));
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A dab makes a decal layer named for its picture, the dab dragged
    /// larger covers more, and the layer and the dab are one undo; a
    /// second dab goes on the same decal layer.
    #[test]
    fn a_dab_is_its_layer_and_one_undo() {
        let mut app = app();
        dab(&mut app, Vector { x: CENTRE.x + 20.0, y: CENTRE.y });
        let small = painted(&app);
        let names: Vec<String> =
            app.doc.active_volume().layers().metas().into_iter().map(|meta| meta.name).collect();
        assert_eq!(names, ["Paint", "Decal: Star"]);
        assert!(small > 0, "the dab laid nothing");
        update(&mut app, Message::Undo);
        assert!(app.doc.active_volume().layers().is_plain(), "the layer was not in the dab's undo");
        assert_eq!(painted(&app), 0);
        dab(&mut app, Vector { x: CENTRE.x + 60.0, y: CENTRE.y });
        assert!(painted(&app) > small * 3, "a dab dragged three times as far was not larger");
        dab(&mut app, Vector { x: CENTRE.x + 2.0, y: CENTRE.y });
        assert_eq!(app.doc.active_volume().layers().len(), 2, "a second dab made another layer");
    }

    /// A built-in stamp is chosen as a stamp, and any picture can be either.
    #[test]
    fn a_stamp_picture_is_chosen_as_a_stamp() {
        let mut app = app();
        update(
            &mut app,
            Message::Decal(DecalMessage::Chosen(DecalPick::BuiltIn(BuiltInPicture::Hazard))),
        );
        assert!(app.painting.decal.as_stamp);
        update(&mut app, Message::Decal(DecalMessage::AsStamp(false)));
        assert!(!app.painting.decal.as_stamp);
        let picture = Arc::new(Picture::from_fn(4, 4, |_, _| [1, 2, 3, 255]));
        update(
            &mut app,
            Message::Decal(DecalMessage::TakenIn(Some(Ok(("Aquila".to_string(), picture))))),
        );
        assert_eq!(app.painting.decal.chosen, DecalPick::Own(0));
        assert_eq!(app.painting.decal.picture().0, "Aquila");
        update(&mut app, Message::Decal(DecalMessage::Soft(9.0)));
        assert_eq!(app.painting.decal.soft, 0.5);
        drop(app.decal_settings());
    }
}
