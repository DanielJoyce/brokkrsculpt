// SPDX-License-Identifier: AGPL-3.0-only

//! The airbrush touch, and the brush sizes of the Paint mode. Stage 3 of
//! `docs/PAINT-RACK.md`, section 5; the spray itself is
//! [`brokkr_core::paint::spray`].
//!
//! The airbrush sprays every frame the button is held, at the pointer,
//! along the view: paint builds where it dwells. How much a second is its
//! flow, times the pen's pressure. The spot is the brush's radius where
//! the surface is as far from the camera as what the camera turns about,
//! and wider further away, as an airbrush held back sprays wider.
//!
//! The sizes are radii in millimetres a painter knows by their brush:
//! five for a brush, and the drybrush's own three.

use brokkr_core::BrushDirection;
use brokkr_core::paint::spray::Spray;
use std::time::Instant;

use glam::{Vec2, Vec3};
use iced::Element;
use iced::widget::{checkbox, column};

use super::Brokkr;
use crate::message::{AirbrushMessage, Message, PaintTouch};
use crate::theme;

/// The brush sizes, by name and radius in millimetres.
pub(super) const SIZES: [(&str, f32); 5] =
    [("Detail", 0.3), ("Fine", 0.6), ("Standard", 1.2), ("Base", 2.5), ("Large", 5.0)];

/// The drybrush's sizes.
pub(super) const DRY_SIZES: [(&str, f32); 3] = [("Small", 1.5), ("Medium", 3.0), ("Large", 6.0)];

/// How much of a full coat a second of spray lays at the middle of the
/// spot, at most and at first.
pub(super) const FLOW_RANGE: std::ops::RangeInclusive<f32> = 0.1..=4.0;
const FIRST_FLOW: f32 = 1.0;

/// A wait longer than this between two sprays is sprayed as this long: a
/// stall should not flood the spot.
const LONGEST_WAIT_S: f32 = 0.25;

/// The most sprays one move of the pointer is shared out into.
const MOST_STEPS: usize = 16;

/// How much wider or narrower distance may make the spot.
const SPREAD: std::ops::RangeInclusive<f32> = 0.25..=4.0;

/// The airbrush's settings, and where it is held.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct AirbrushState {
    pub flow: f32,
    pub overspray: bool,
    /// The pointer while the button is held, and which way.
    held: Option<(Vec2, BrushDirection)>,
    /// When it last sprayed.
    last: Option<Instant>,
}

impl Default for AirbrushState {
    fn default() -> Self {
        Self { flow: FIRST_FLOW, overspray: true, held: None, last: None }
    }
}

impl Brokkr {
    pub(super) fn airbrush_changed(&mut self, message: AirbrushMessage) {
        match message {
            AirbrushMessage::Flow(flow) => {
                self.painting.airbrush.flow = flow.clamp(*FLOW_RANGE.start(), *FLOW_RANGE.end());
            }
            AirbrushMessage::Overspray(on) => self.painting.airbrush.overspray = on,
        }
    }

    /// Whether a drag of the paint brush is the airbrush's.
    pub(super) fn airbrushing(&self) -> bool {
        self.touch.kind == PaintTouch::Airbrush
    }

    /// A press or a move of the airbrush: spray where it is now for the
    /// time since it last sprayed, so a sweep lays a line however few
    /// frames it crosses. A press sprays a sixtieth of a second, so a
    /// click leaves a mark.
    ///
    /// Between two moves further apart than half a spot, the time is
    /// shared out along the way: the pointer's events come further apart
    /// than a spot is wide on a quick sweep, and sprayed only at each it
    /// left a row of dots. Seen on a screen.
    pub(super) fn airbrush_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        self.arm_recorder();
        let was = self.painting.airbrush.held.map(|(at, _)| at).filter(|_| !start);
        if start {
            self.painting.airbrush.held = Some((pixel, direction));
            self.spray_for(1.0 / 60.0);
            return;
        }
        let seconds = self.since_the_last_spray();
        let steps = match (was.and_then(|at| self.surface_under(at)), self.surface_under(pixel)) {
            (Some(from), Some(to)) => {
                let half_spot = (self.brush.radius * 0.5).max(1e-3);
                ((from.distance(to) / half_spot).ceil() as usize).clamp(1, MOST_STEPS)
            }
            _ => 1,
        };
        let from = was.unwrap_or(pixel);
        for step in 1..=steps {
            let along = step as f32 / steps as f32;
            self.painting.airbrush.held = Some((from.lerp(pixel, along), direction));
            self.spray_for(seconds / steps as f32);
        }
    }

    /// A frame while the airbrush is held: spray for the time since it last
    /// sprayed. Let go, it is put down.
    pub(super) fn airbrush_frame(&mut self) {
        if self.painting.airbrush.held.is_none() {
            return;
        }
        if self.drag.is_none() || !self.airbrushing() {
            self.painting.airbrush.held = None;
            self.painting.airbrush.last = None;
            return;
        }
        let seconds = self.since_the_last_spray();
        self.spray_for(seconds);
    }

    /// Seconds since the airbrush last sprayed, no more than a stall's
    /// worth; and now is when it last did.
    fn since_the_last_spray(&mut self) -> f32 {
        let now = Instant::now();
        let seconds =
            self.painting.airbrush.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.painting.airbrush.last = Some(now);
        seconds.min(LONGEST_WAIT_S)
    }

    /// Spray where the airbrush is held for `seconds`.
    pub(super) fn spray_for(&mut self, seconds: f32) {
        if self.painting.airbrush.last.is_none() {
            self.painting.airbrush.last = Some(Instant::now());
        }
        let Some((pixel, direction)) = self.painting.airbrush.held else {
            return;
        };
        let (origin, ray) = self.ray_through(pixel);
        let Some(at) = self
            .doc
            .pick_body(self.doc.active(), origin, ray, self.camera.far())
            .map(|hit| hit.position)
        else {
            return;
        };
        let away = at.distance(origin);
        let spread = (away / self.camera.distance.max(1e-3)).clamp(*SPREAD.start(), *SPREAD.end());
        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let spray = Spray {
            pigment: self.pigment(direction),
            radius: self.brush.radius * spread,
            overspray: self.painting.airbrush.overspray,
        };
        let amount = self.painting.airbrush.flow * seconds * pressure;
        let ray: Vec3 = ray;
        self.doc.active_volume_mut().spray(at, ray, &spray, amount);
        self.remesh_dirty();
    }

    /// The airbrush's own settings, under the touches.
    pub(super) fn airbrush_settings(&self) -> Element<'_, Message> {
        let airbrush = self.painting.airbrush;
        column![
            crate::ui::number(
                &self.drafts,
                "airbrush.flow",
                "Flow, coats a second",
                airbrush.flow,
                FLOW_RANGE,
                |flow| Message::Airbrush(AirbrushMessage::Flow(flow)),
            )
            .shown(format!("{:.2}", airbrush.flow)),
            checkbox(airbrush.overspray)
                .label("Overspray")
                .on_toggle(|on| Message::Airbrush(AirbrushMessage::Overspray(on)))
                .text_size(theme::LABEL_SIZE),
        ]
        .spacing(theme::S2)
        .into()
    }

    /// The brush sizes, or the drybrush's: one lit when the radius is it.
    pub(super) fn paint_sizes(&self) -> Element<'_, Message> {
        let sizes: &[(&str, f32)] =
            if self.touch.kind == PaintTouch::Drybrush { &DRY_SIZES } else { &SIZES };
        let radius = self.brush.radius;
        let lit = sizes.iter().position(|(_, size)| (size - radius).abs() < 1e-3);
        crate::ui::segmented(
            sizes.iter().enumerate().map(|(at, (name, _))| (Some(at), *name)),
            lit,
            move |at| Message::BrushRadiusChanged(at.map_or(radius, |at| sizes[at].1)),
        )
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
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Airbrush));
        app
    }

    fn coverage(app: &Brokkr) -> u32 {
        let volume = app.doc.active_volume();
        let mut total = 0;
        for coord in volume.brick_coords() {
            let origin = coord.origin();
            for index in 0..32 * 32 * 32 {
                let cell = origin + glam::IVec3::new(index % 32, (index / 32) % 32, index / 1024);
                total += u32::from(volume.paint().at(cell)[3]);
            }
        }
        total
    }

    fn press(app: &mut Brokkr) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: CENTRE,
                size: SIZE,
            }),
        );
    }

    fn release(app: &mut Brokkr) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// Held still, the airbrush lays more the longer it is held, the whole
    /// hold is one undo, and let go it lays no more.
    #[test]
    fn the_airbrush_builds_while_held_and_is_one_undo() {
        let mut app = app();
        press(&mut app);
        let first = coverage(&app);
        assert!(first > 0, "a press laid nothing");
        for _ in 0..10 {
            app.spray_for(0.016);
        }
        let held = coverage(&app);
        assert!(held > first * 3, "ten frames held laid {held} after {first}");
        release(&mut app);
        app.airbrush_frame();
        app.spray_for(0.016);
        assert_eq!(coverage(&app), held, "it sprayed after it was let go");
        update(&mut app, Message::Undo);
        assert_eq!(coverage(&app), 0, "the hold was not one undo");
    }

    /// A sweep lays a line, not a dot at each event of the pointer: the
    /// middle of a move far wider than the spot takes paint.
    #[test]
    fn a_sweep_lays_a_line() {
        let mut app = app();
        press(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let to = Vector { x: CENTRE.x + 160.0, y: CENTRE.y };
        update(&mut app, Message::Pointer(PointerEvent::Moved { position: to, size: SIZE }));
        release(&mut app);
        let middle = Vec2::new(CENTRE.x + 80.0, CENTRE.y);
        let at = app.surface_under(middle).expect("the ball is under the middle of the sweep");
        let voxel = (at / app.doc.voxel_size()).round().as_ivec3();
        let volume = app.doc.active_volume();
        let around = (-2..=2)
            .flat_map(|x| {
                (-2..=2).flat_map(move |y| (-2..=2).map(move |z| glam::IVec3::new(x, y, z)))
            })
            .map(|offset| u32::from(volume.paint().at(voxel + offset)[3]))
            .max()
            .unwrap_or(0);
        assert!(around > 0, "the middle of the sweep took no paint");
    }

    /// What a frame of the airbrush and a move of a loaded brush cost, by
    /// the brush's size. A measurement, run by hand in release: `cargo test
    /// --release -p brokkr-app -- --ignored what_painting_costs --nocapture`.
    #[test]
    #[ignore = "a measurement, not a claim: run by hand, in release"]
    fn what_painting_costs() {
        for radius in [2.0, 5.0, 10.0] {
            let mut app = app();
            update(&mut app, Message::BrushRadiusChanged(radius));
            press(&mut app);
            let started = std::time::Instant::now();
            for _ in 0..10 {
                app.spray_for(0.016);
            }
            let spray = started.elapsed().as_secs_f64() * 100.0;
            release(&mut app);

            let mut app = app;
            if std::env::var_os("BROKKR_LAYERED").is_some() {
                update(&mut app, Message::Coats(crate::message::CoatsMessage::Spongechop));
                update(&mut app, Message::Frame);
            }
            update(&mut app, Message::PaintTouchChosen(PaintTouch::Loaded));
            press(&mut app);
            let started = std::time::Instant::now();
            for step in 1..=10 {
                let to = Vector { x: CENTRE.x + step as f32 * 25.0, y: CENTRE.y };
                update(
                    &mut app,
                    Message::Pointer(PointerEvent::Moved { position: to, size: SIZE }),
                );
                // Moves are held until the next frame. See `pointer_in`.
                update(&mut app, Message::Frame);
            }
            let stroke = started.elapsed().as_secs_f64() * 100.0;
            release(&mut app);
            eprintln!(
                "radius {radius} mm: airbrush {spray:.1} ms a frame, brush {stroke:.1} ms a move"
            );
        }
    }

    /// The sizes set the radius and light the one it is.
    #[test]
    fn the_sizes_set_the_radius() {
        let mut app = app();
        update(&mut app, Message::BrushRadiusChanged(SIZES[1].1));
        assert_eq!(app.brush.radius, 0.6);
        drop(app.paint_sizes());
        update(&mut app, Message::Airbrush(AirbrushMessage::Flow(100.0)));
        assert_eq!(app.painting.airbrush.flow, *FLOW_RANGE.end());
        drop(app.airbrush_settings());
    }
}
