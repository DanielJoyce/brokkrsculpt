// SPDX-License-Identifier: AGPL-3.0-only

//! Which way up an insert goes on, and how deep: a picture of it standing
//! on the surface, turned by hand.
//!
//! Asked for by the maintainer on 2026-11-27: "for insert brushes in the
//! settings view, the up direction on the monitor should map to the
//! surface normal. So by rotating the object in its preview how it inserts
//! relative to the surface changes when it's used. Also need some quick
//! xyz/-xyz to flip insert meshes around the Cardinal directions quickly.
//! Finally there is a translucent or transparent plane in the view and a
//! simple z slider lets you quickly preview set how deep the mesh will be
//! embedded when drawn."
//!
//! # The turn
//!
//! A shape or a lent body is set down in a frame the surface gives it
//! (`hani_core::solid::Placed::on`): its z out along the normal, its y the
//! way the drag went. The turn is a rotation in that frame, applied after
//! it, so it says which of the insert's own axes stands out of the surface
//! and which way it faces, wherever on the model it is set down. Identity
//! is the insert as it always went on.
//!
//! # The picture
//!
//! At the top of ADD, for a shape or a body: the insert as it will go on,
//! traced on the processor ([`picture`]) against its own distance field,
//! the one the join reads, so the picture is the insert and not a drawing
//! of it. Up the picture is out of the surface. The surface is the
//! translucent plane, and the part of the insert under it, which will be
//! in the model, is drawn darker. A drag on the picture turns the insert
//! (across: about the normal; up and down: tipped toward or away);
//! a right drag looks round it without turning it. *+X* to *-Z* stand
//! that axis of the insert out of the surface; *Reset* is as it came.
//! The slider under it, *Held off*, which was there, moves the plane.
//!
//! A brush kept with a turn keeps it (`turn:` in its file, `presets.rs`).

use glam::{Mat3, Quat, Vec3};
use hani_core::solid::{Lent, Placed, Solid};
use iced::widget::{column, image, mouse_area, row};
use iced::{Element, Length, Point};

use super::Hani;
use super::adding::Insert;
use crate::message::{Message, TurnMessage};
use crate::theme;

/// The picture's size, in pixels.
const WIDTH: u32 = 232;
const HEIGHT: u32 = 160;

/// How far the picture sees across, in the insert's own halves.
const SPAN: f32 = 6.0;

/// How much a drag of a pixel turns, in radians.
const PER_PIXEL: f32 = 0.012;

/// The six axes an insert can stand on, in the order of their buttons.
pub const AXES: [(&str, Vec3); 6] = [
    ("+X", Vec3::X),
    ("-X", Vec3::NEG_X),
    ("+Y", Vec3::Y),
    ("-Y", Vec3::NEG_Y),
    ("+Z", Vec3::Z),
    ("-Z", Vec3::NEG_Z),
];

/// The turn that stands `axis` of the insert out of the surface.
pub fn standing_on(axis: Vec3) -> Quat {
    Quat::from_rotation_arc(axis.normalize(), Vec3::Z)
}

/// The picture's view and the drag in progress.
#[derive(Debug, Clone)]
pub(super) struct TurnView {
    /// Round the normal, and above the surface, in radians.
    pub yaw: f32,
    pub pitch: f32,
    /// Where the pointer last was over the picture.
    at: Option<Point>,
    /// A drag in progress: `true` looks round, `false` turns.
    dragging: Option<bool>,
    /// The picture as last drawn, and what it was drawn for.
    pub picture: Option<image::Handle>,
    drawn_for: Option<(Quat, f32, f32, f32, u64)>,
}

impl Default for TurnView {
    fn default() -> Self {
        Self { yaw: -0.6, pitch: 0.35, at: None, dragging: None, picture: None, drawn_for: None }
    }
}

/// A shape or a body to picture: what is turned.
pub(super) enum Pictured<'a> {
    Shape(hani_core::solid::Form),
    Body(&'a hani_core::Volume, (Vec3, Vec3)),
}

/// The insert turned by `turn`, its middle `lift` halves off the surface,
/// as the camera at `yaw` and `pitch` sees it: `WIDTH` by `HEIGHT` RGBA.
pub fn picture(what: &Pictured<'_>, turn: Quat, lift: f32, yaw: f32, pitch: f32) -> Vec<u8> {
    with_solid(what, turn, lift, |solid| draw(solid, yaw, pitch))
        .unwrap_or_else(|| vec![0; (WIDTH * HEIGHT * 4) as usize])
}

/// The insert as a solid, turned and lifted, handed to `then`. Nothing
/// for one that cannot be set down.
fn with_solid<R>(
    what: &Pictured<'_>,
    turn: Quat,
    lift: f32,
    then: impl FnOnce(&dyn Solid) -> R,
) -> Option<R> {
    let (normal, toward, centre) = (turn * Vec3::Z, turn * Vec3::Y, Vec3::Z * lift);
    match what {
        Pictured::Shape(form) => {
            Placed::on(*form, centre, normal, toward, 1.0, 0.0).map(|placed| then(&placed))
        }
        Pictured::Body(source, extent) => {
            Lent::on(source, *extent, centre, normal, toward, 1.0, 0.0).map(|lent| then(&lent))
        }
    }
}

/// The camera at `yaw` and `pitch`: its forward, right and up.
fn camera(yaw: f32, pitch: f32) -> (Vec3, Vec3, Vec3) {
    let forward = -Vec3::new(pitch.cos() * yaw.sin(), -pitch.cos() * yaw.cos(), pitch.sin());
    let right = forward.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
    (forward, right, right.cross(forward))
}

/// Each pixel's ray, top row first: where it starts, and where along it
/// the insert is, if it is.
fn rays(solid: &dyn Solid, yaw: f32, pitch: f32) -> Vec<(Vec3, Option<f32>)> {
    let (forward, right, up) = camera(yaw, pitch);
    let per_pixel = SPAN / WIDTH as f32;
    let mut rays = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let across = (x as f32 + 0.5 - WIDTH as f32 * 0.5) * per_pixel;
            let upward = (HEIGHT as f32 * 0.5 - y as f32 - 0.5) * per_pixel;
            let origin = right * across + up * upward - forward * 4.0;
            rays.push((origin, trace(solid, origin, forward)));
        }
    }
    rays
}

fn draw(solid: &dyn Solid, yaw: f32, pitch: f32) -> Vec<u8> {
    let (forward, right, up) = camera(yaw, pitch);
    let light = (up * 0.8 - right * 0.5 - forward * 0.6).normalize();
    let background = [0x0b, 0x0d, 0x10];
    let plane = [0xff_u8, 0x7a, 0x3c];
    let mut rgba = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for (origin, hit) in rays(solid, yaw, pitch) {
        let mut colour = match hit {
            Some(t) => {
                let at = origin + forward * t;
                let normal = gradient(solid, at);
                let lit = 0.3 + 0.7 * normal.dot(light).max(0.0);
                // In the model once it is set down: darker.
                let clay = if at.z < 0.0 { [120.0, 112.0, 104.0] } else { [212.0, 204.0, 194.0] };
                clay.map(|channel| (channel * lit).min(255.0) as u8)
            }
            None => background,
        };
        // The surface, see-through, where it is in front of the insert.
        if forward.z.abs() > 1.0e-4 {
            let t = -origin.z / forward.z;
            let on = origin + forward * t;
            let in_front = hit.is_none_or(|hit| t < hit);
            if t > 0.0 && in_front && on.x.abs() < 1.9 && on.y.abs() < 1.9 {
                let edge = on.x.abs() > 1.86 || on.y.abs() > 1.86;
                let share = if edge { 0.8 } else { 0.22 };
                for (channel, tint) in colour.iter_mut().zip(plane) {
                    *channel = (*channel as f32 * (1.0 - share) + tint as f32 * share) as u8;
                }
            }
        }
        rgba.extend_from_slice(&[colour[0], colour[1], colour[2], 255]);
    }
    rgba
}

/// Where along the ray the insert's surface is, if it is.
fn trace(solid: &dyn Solid, origin: Vec3, ray: Vec3) -> Option<f32> {
    let (low, high) = solid.bounds();
    // Only through its box.
    let inverse = ray.recip();
    let (a, b) = ((low - origin) * inverse, (high - origin) * inverse);
    let near = a.min(b).max_element().max(0.0);
    let far = a.max(b).min_element();
    if near > far {
        return None;
    }
    let mut t = near;
    for _ in 0..160 {
        let distance = solid.distance(origin + ray * t);
        if distance < 1.0e-3 {
            return Some(t);
        }
        t += distance.max(2.0e-3);
        if t > far {
            return None;
        }
    }
    None
}

fn gradient(solid: &dyn Solid, at: Vec3) -> Vec3 {
    let e = 2.0e-3;
    let d = |offset: Vec3| solid.distance(at + offset) - solid.distance(at - offset);
    Vec3::new(d(Vec3::X * e), d(Vec3::Y * e), d(Vec3::Z * e)).try_normalize().unwrap_or(Vec3::Z)
}

impl Hani {
    /// The shape or body in hand, to picture, if what is in hand turns.
    fn pictured(&self) -> Option<Pictured<'_>> {
        match self.inserts.what? {
            Insert::Shape(form) => Some(Pictured::Shape(form)),
            Insert::Body(_) => {
                let (source, extent) = self.inserts.lent()?;
                Some(Pictured::Body(source, extent))
            }
            Insert::Laid | Insert::Hung | Insert::Map(_) => None,
        }
    }

    /// Draw the picture again if what it shows has changed.
    pub(super) fn refresh_turn_picture(&mut self) {
        let key = match self.inserts.what {
            Some(Insert::Shape(form)) => form as u64,
            Some(Insert::Body(id)) => 100 + id.0 as u64,
            _ => {
                self.inserts.view.picture = None;
                self.inserts.view.drawn_for = None;
                return;
            }
        };
        let view = &self.inserts.view;
        let wanted = (self.inserts.turn, self.inserts.lift, view.yaw, view.pitch, key);
        if view.drawn_for == Some(wanted) {
            return;
        }
        let Some(what) = self.pictured() else {
            return;
        };
        let rgba = picture(&what, wanted.0, wanted.1, wanted.2, wanted.3);
        let view = &mut self.inserts.view;
        view.picture = Some(image::Handle::from_rgba(WIDTH, HEIGHT, rgba));
        view.drawn_for = Some(wanted);
    }

    pub(super) fn turn_changed(&mut self, message: TurnMessage) {
        let view = &mut self.inserts.view;
        match message {
            TurnMessage::Pressed { look } => view.dragging = Some(look),
            TurnMessage::Released => view.dragging = None,
            TurnMessage::Moved(point) => {
                let last = view.at.replace(point);
                if let (Some(look), Some(last)) = (view.dragging, last) {
                    let (dx, dy) = (point.x - last.x, point.y - last.y);
                    if look {
                        view.yaw += dx * PER_PIXEL;
                        view.pitch = (view.pitch + dy * PER_PIXEL).clamp(-1.45, 1.45);
                    } else {
                        // Across turns it about the normal; up and down
                        // tips it about the picture's own across.
                        let (_, right, _) = camera(view.yaw, view.pitch);
                        let spin = Quat::from_axis_angle(Vec3::Z, dx * PER_PIXEL)
                            * Quat::from_axis_angle(right, dy * PER_PIXEL);
                        self.inserts.turn = (spin * self.inserts.turn).normalize();
                    }
                }
            }
            TurnMessage::Left => {
                view.at = None;
                view.dragging = None;
            }
            TurnMessage::Stood(at) => {
                if let Some((_, axis)) = AXES.get(at) {
                    self.inserts.turn = standing_on(*axis);
                }
            }
            TurnMessage::Reset => self.inserts.turn = Quat::IDENTITY,
        }
        self.refresh_turn_picture();
    }

    /// The picture and the six axes, for a shape or a body in hand.
    pub(super) fn turn_block(&self) -> Option<Element<'_, Message>> {
        let picture = self.inserts.view.picture.clone()?;
        let send = |message: TurnMessage| Message::Turn(message);
        let shown = crate::ui::tip(
            mouse_area(
                image(picture)
                    .width(Length::Fixed(WIDTH as f32))
                    .height(Length::Fixed(HEIGHT as f32)),
            )
            .on_press(send(TurnMessage::Pressed { look: false }))
            .on_right_press(send(TurnMessage::Pressed { look: true }))
            .on_release(send(TurnMessage::Released))
            .on_right_release(send(TurnMessage::Released))
            .on_move(move |point| send(TurnMessage::Moved(point)))
            .on_exit(send(TurnMessage::Left)),
            "Drag to turn it on the surface, right drag to look round it",
        );
        let standing = self.inserts.turn;
        let axes = AXES.iter().enumerate().fold(
            row![].spacing(theme::S1),
            |assembled, (at, (label, axis))| {
                let lit = (standing * *axis - Vec3::Z).length() < 1.0e-3;
                assembled.push(crate::ui::tip(
                    crate::ui::toggle(*label, lit, send(TurnMessage::Stood(at))),
                    "Stand this axis of it out of the surface",
                ))
            },
        );
        Some(
            column![
                shown,
                row![
                    axes.width(Length::Fill),
                    crate::ui::tip(
                        crate::ui::button_secondary("Reset", Some(send(TurnMessage::Reset))),
                        "Turn it back as it came",
                    )
                ]
                .spacing(theme::S1),
            ]
            .spacing(theme::S1)
            .into(),
        )
    }
}

/// The turn applied to a frame the surface gave: the insert's own axes in
/// the world.
pub fn turned(frame: Mat3, turn: Quat) -> Mat3 {
    frame * Mat3::from_quat(turn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hani_core::solid::Form;

    #[test]
    fn each_axis_stands_out_of_the_surface() {
        for (label, axis) in AXES {
            let turn = standing_on(axis);
            assert!((turn * axis - Vec3::Z).length() < 1.0e-5, "{label} does not stand up");
        }
    }

    #[test]
    fn a_turn_in_the_surface_frame_moves_the_insert_s_axes_and_not_its_place() {
        let seat = Placed::on(Form::Cylinder, Vec3::ZERO, Vec3::X, Vec3::Y, 2.0, 0.5).unwrap();
        // Standing on its +X: its own x is now out along the normal.
        let frame = turned(seat.frame, standing_on(Vec3::X));
        assert!((frame * Vec3::X - Vec3::X).length() < 1.0e-5, "x is not along the normal");
        assert!((frame.x_axis - Vec3::X).length() < 1.0e-5);
        // Identity is as it always went on.
        assert_eq!(turned(seat.frame, Quat::IDENTITY), seat.frame);
    }

    #[test]
    fn the_picture_shows_the_insert_and_the_surface_and_changes_with_the_turn() {
        let picture_of = |turn: Quat, lift: f32| {
            picture(&Pictured::Shape(Form::Cylinder), turn, lift, -0.6, 0.35)
        };
        let upright = picture_of(Quat::IDENTITY, 0.5);
        assert_eq!(upright.len(), (WIDTH * HEIGHT * 4) as usize);
        let lit =
            |rgba: &[u8]| rgba.chunks(4).filter(|pixel| pixel[0] > 100 && pixel[2] > 90).count();
        assert!(lit(&upright) > 2_000, "the insert is barely there: {}", lit(&upright));
        let laid = picture_of(standing_on(Vec3::X), 0.5);
        assert_ne!(upright, laid, "turning it changed nothing");
        let sunk = picture_of(Quat::IDENTITY, -0.8);
        // `HANI_DUMP_TURN=<directory>` writes the pictures out to look at.
        if let Some(directory) = std::env::var_os("HANI_DUMP_TURN") {
            for (name, rgba) in [("upright", &upright), ("laid", &laid), ("sunk", &sunk)] {
                let picture = ::image::RgbaImage::from_raw(WIDTH, HEIGHT, rgba.clone()).unwrap();
                picture.save(std::path::Path::new(&directory).join(format!("{name}.png"))).unwrap();
            }
        }
        assert_ne!(upright, sunk, "sinking it changed nothing");
        // Sunk, more of what is seen of it is under the surface.
        let under = |lift: f32| {
            with_solid(&Pictured::Shape(Form::Cylinder), Quat::IDENTITY, lift, |solid| {
                let (forward, _, _) = camera(-0.6, 0.35);
                rays(solid, -0.6, 0.35)
                    .into_iter()
                    .filter_map(|(origin, hit)| hit.map(|t| (origin + forward * t).z < 0.0))
                    .filter(|below| *below)
                    .count()
            })
            .unwrap()
        };
        assert!(under(-0.8) > 2 * under(0.5), "{} under sunk, {} raised", under(-0.8), under(0.5));
    }
}
