// SPDX-License-Identifier: AGPL-3.0-only

//! The Decal touch: a picture dabbed onto the model. Stage 5 of
//! `docs/PAINT-RACK.md`, section 6; the dab itself is
//! [`hani_core::paint::decal`].
//!
//! Press where it goes; drag away to size it and round to turn it, and it
//! is dabbed when the button comes up. While it is dragged it is drawn
//! over the model and not laid: the picture on a grid cast along the view
//! onto the surface, each cell in the picture's colour. Asked for by the
//! maintainer on 2026-11-25 ("render as decal before actually stamping
//! onto paint layer"): it was dabbed again at every move, which grew with
//! the cube of its size, 50 ms a move at 21 mm across. A click dabs it at
//! the brush's size. It goes on the active layer if that is a decal layer, or on a new
//! one over it named for the picture, and the new layer and the dab are
//! one undo entry.
//!
//! Pictures are the six that come with the application, and any picture
//! taken in, which is kept in `decals/` in the data directory as the
//! brushes are. Any picture can go on as a decal, in its own colours, or
//! as a stamp, a stencil in the paint in hand.

// A picked file has no path in a browser. See `web.rs`.
#[cfg(target_arch = "wasm32")]
use crate::web::NoPath;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec2;
use hani_core::paint::decal::{BuiltInPicture, Dab, Picture};
use hani_core::{Change, Entry, NodeId};
use iced::widget::{button, column, row, text};
use iced::{Element, Length, Task};

use super::Hani;
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
    /// What is drawn of it until it is laid: triangles on the surface, each
    /// with its colour.
    preview: Vec<([glam::Vec3; 3], [f32; 4])>,
}

/// How many cells across the preview's grid is.
const PREVIEW_CELLS: usize = 48;

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
    /// How wide the last dab was, world units, for a measurement.
    #[cfg(test)]
    pub last_size: f32,
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
            #[cfg(test)]
            last_size: 0.0,
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

impl Hani {
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
        self.touch.kind == PaintTouch::Decal
    }

    /// A press or a move of the Decal touch: where it goes, how big and
    /// how turned, drawn and not laid. See the module documentation.
    pub(super) fn decal_to(&mut self, pixel: Vec2, start: bool) {
        if start {
            self.painting.decal.gesture = None;
            let Some(at) = self.surface_under(pixel) else {
                return;
            };
            let (_, along) = self.ray_through(pixel);
            let across = |offset: Vec2| {
                let (_, ray) = self.ray_through(pixel + offset);
                (ray - along * ray.dot(along)).normalize_or_zero()
            };
            let (right, up) = (across(Vec2::new(10.0, 0.0)), across(Vec2::new(0.0, -10.0)));
            let stamp =
                self.painting.decal.as_stamp.then(|| self.pigment(hani_core::BrushDirection::Add));
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
            let preview = self.decal_preview(&dab);
            self.painting.decal.gesture = Some(Gesture { pressed: pixel, dab, preview });
            self.refresh_overlay();
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
        let preview = self.decal_preview(&dab);
        self.painting.decal.gesture = Some(Gesture { dab, preview, ..gesture });
        self.refresh_overlay();
    }

    /// Lay the dab dragged out, when the button comes up: on a decal layer,
    /// in the stroke whose entry the layer joins. Called at the end of a
    /// stroke, before its entry is made.
    pub(super) fn set_the_decal_down(&mut self) {
        let Some(gesture) = self.painting.decal.gesture.as_ref() else {
            return;
        };
        let dab = gesture.dab;
        #[cfg(test)]
        {
            self.painting.decal.last_size = dab.size;
        }
        let (_, picture) = self.painting.decal.picture();
        self.decal_layer();
        self.arm_recorder();
        let _ = self.doc.active_volume_mut().dab(&picture, &dab, None);
        if let Some(gesture) = self.painting.decal.gesture.as_mut() {
            gesture.preview.clear();
        }
        self.remesh_dirty();
        self.refresh_overlay();
    }

    /// The picture on a grid laid along the view onto the surface, each
    /// cell in the colour the dab would lay there: what is drawn while the
    /// dab is dragged.
    fn decal_preview(&self, dab: &Dab) -> Vec<([glam::Vec3; 3], [f32; 4])> {
        let (_, picture) = self.painting.decal.picture();
        let volume = self.doc.active_volume();
        let voxel = volume.voxel_size();
        let along = dab.along.normalize_or_zero();
        let cells = PREVIEW_CELLS;
        // Each corner of the grid, cast along the view from in front of the
        // picture's plane onto the surface, and lifted a voxel toward the
        // eye so the surface does not hide it.
        let corner = |i: usize, j: usize| {
            let uv = Vec2::new(i as f32 / cells as f32, j as f32 / cells as f32);
            let mut at = dab.on_plane(&picture, uv) - along * dab.size;
            let mut travelled = 0.0;
            for _ in 0..96 {
                let distance = volume.sample_world(at) * voxel;
                if distance < voxel * 0.1 {
                    return Some(at - along * voxel);
                }
                let step = distance.max(voxel * 0.25);
                at += along * step;
                travelled += step;
                if travelled > dab.size * 2.0 {
                    return None;
                }
            }
            None
        };
        // Across every core: a cast is a few dozen samples of the field,
        // and 2401 of them on one core were most of a move of a large decal
        // (17 ms at 41 mm across on four cores, the same on one).
        let corners: Vec<Option<glam::Vec3>> = {
            use rayon::prelude::*;
            (0..(cells + 1) * (cells + 1))
                .into_par_iter()
                .map(|at| corner(at % (cells + 1), at / (cells + 1)))
                .collect()
        };
        let at = |i: usize, j: usize| corners[j * (cells + 1) + i];
        let mut triangles = Vec::new();
        for j in 0..cells {
            for i in 0..cells {
                let (Some(a), Some(b), Some(c), Some(d)) =
                    (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1))
                else {
                    continue;
                };
                let middle =
                    Vec2::new((i as f32 + 0.5) / cells as f32, (j as f32 + 0.5) / cells as f32);
                let (colour, amount) = dab.shows(&picture, middle);
                if amount < 0.02 {
                    continue;
                }
                let colour = [
                    f32::from(colour[0]) / 255.0,
                    f32::from(colour[1]) / 255.0,
                    f32::from(colour[2]) / 255.0,
                    amount.min(1.0) * 0.9,
                ];
                triangles.push(([a, b, c], colour));
                triangles.push(([a, c, d], colour));
            }
        }
        triangles
    }

    /// The dab being dragged, drawn over the model.
    pub(super) fn add_decal_preview(&self, batch: &mut hani_gpu::OverlayBatch) {
        let Some(gesture) = &self.painting.decal.gesture else {
            return;
        };
        for ([a, b, c], colour) in &gesture.preview {
            batch.push_triangle(*a, *b, *c, *colour);
        }
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
            .add_layer(format!("Decal: {name}"), hani_core::paint::layers::Blend::Normal)
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
        edit: Option<hani_core::StrokeEdit>,
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
            crate::ui::tip(
                button(text(label).size(theme::LABEL_SIZE).width(Length::Fill))
                    .width(Length::Fill)
                    .padding(theme::PAD_CHOICE)
                    .style(style)
                    .on_press(Message::Decal(DecalMessage::Chosen(pick))),
                "Dab this picture on where you click",
            )
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
            crate::ui::tip(
                crate::ui::button_secondary("Picture\u{2026}", Some(send(DecalMessage::TakeIn))),
                "Take in a picture of your own"
            ),
            crate::ui::segmented_tips(
                [
                    (false, "Decal", "The picture in its own colours"),
                    (true, "Stamp", "The picture's shape in the paint in hand"),
                ],
                decal.as_stamp,
                |on| Message::Decal(DecalMessage::AsStamp(on)),
            ),
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
    use glam::IVec3;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };
    const CENTRE: Vector = Vector { x: 400.0, y: 300.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Paint));
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Decal));
        app
    }

    fn painted(app: &Hani) -> usize {
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

    fn dab(app: &mut Hani, to: Vector) {
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

    /// What one move of a decal being dragged to size costs, and its
    /// release, at sizes from a small one to one across most of the ball.
    /// A measurement, run by hand in release:
    /// `cargo test --release -p hani-app -- --ignored what_a_decal_costs --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn what_a_decal_costs() {
        for pulled in [20.0, 60.0, 120.0, 240.0] {
            let mut app = app();
            update(
                &mut app,
                Message::Pointer(PointerEvent::Pressed {
                    button: PointerButton::Left,
                    position: CENTRE,
                    size: SIZE,
                }),
            );
            update(&mut app, Message::Frame);
            let moves = 8;
            let started = web_time::Instant::now();
            for step in 0..moves {
                let at = Vector { x: CENTRE.x + pulled + step as f32, y: CENTRE.y };
                update(
                    &mut app,
                    Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }),
                );
                update(&mut app, Message::Frame);
            }
            let per_move = started.elapsed().as_secs_f32() * 1000.0 / moves as f32;
            let started = web_time::Instant::now();
            update(
                &mut app,
                Message::Pointer(PointerEvent::Released { button: PointerButton::Left }),
            );
            update(&mut app, Message::Frame);
            let release = started.elapsed().as_secs_f32() * 1000.0;
            let size = app.painting.decal.last_size;
            eprintln!(
                "pulled {pulled} px, {size:.1} mm across: {per_move:.1} ms a move, {release:.1} ms the release, {} voxels",
                painted(&app)
            );
        }
    }
}
