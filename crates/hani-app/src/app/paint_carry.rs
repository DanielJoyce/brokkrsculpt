// SPDX-License-Identifier: AGPL-3.0-only

//! Sculpting over paint carries it onto the surface the stroke makes.
//!
//! Asked for by the maintainer on 2026-11-27. The work is the core's,
//! `hani_core::paint::carry`; this says where, after each pointer event's
//! stamps: in the box each stamp's ball reaches, and in its mirrors' when
//! symmetry is on, since a field brush changes nothing outside its ball.

use glam::Vec3;
use hani_core::{Brush, Stamp, Symmetry};

use super::{Hani, MIRROR_CENTRE};

impl Hani {
    /// Carry paint onto what this event's stamps, `brush` at `pressure`,
    /// brought into the surface's band.
    pub(super) fn carry_paint_after_stamps(&mut self, brush: Brush, pressure: f32) {
        if self.stamp_centres.is_empty() {
            return;
        }
        // The ball a stamp may change: the brush as the studio sizes it,
        // and no smaller than the brush as set.
        let reach = self.studio.sized(brush, pressure).radius.max(brush.radius);
        // One box for the stamps and one for each mirror of them: a box
        // round a stamp and its mirror would take in the body between.
        let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
        let mut twins = [Stamp::new(Vec3::ZERO, Vec3::Z, hani_core::BrushDirection::Add);
            Symmetry::MAX_MIRRORS];
        for centre in &self.stamp_centres {
            let stamp = Stamp::new(*centre, Vec3::Z, hani_core::BrushDirection::Add);
            let count = if self.symmetry.is_off() {
                0
            } else {
                self.framed_symmetry().mirrors(&stamp, MIRROR_CENTRE, &mut twins)
            };
            let places =
                std::iter::once(*centre).chain(twins[..count].iter().map(|twin| twin.centre));
            for (index, at) in places.enumerate() {
                let (low, high) = (at - Vec3::splat(reach), at + Vec3::splat(reach));
                match boxes.get_mut(index) {
                    Some(held) => *held = (held.0.min(low), held.1.max(high)),
                    None => boxes.push((low, high)),
                }
            }
        }
        let volume = self.doc.active_volume_mut();
        for (low, high) in boxes {
            let (lo, hi) = volume.voxel_bounds(low, high);
            volume.carry_paint_onto_new_surface(lo, hi);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, PaintingMessage, PointerButton, PointerEvent};
    use hani_core::BrushKind;
    use hani_core::brick::NARROW_BAND;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// One stroke of the brush in hand, wiggled about the middle of the view.
    fn stroke(app: &mut Hani) {
        let middle = Vector::new(SIZE.x * 0.5, SIZE.y * 0.5);
        app.on_pointer(PointerEvent::Moved { position: middle, size: SIZE });
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: middle,
            size: SIZE,
        });
        for step in 0..40 {
            let wiggle = if step % 2 == 0 { 3.0 } else { -3.0 };
            app.on_pointer(PointerEvent::Moved {
                position: Vector::new(middle.x + wiggle, middle.y),
                size: SIZE,
            });
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
    }

    /// Where the surface is under the middle of the view, and the paint
    /// shown at the voxel there.
    fn under_the_middle(app: &Hani) -> (Vec3, [u8; 4]) {
        let at = app.surface_under(glam::Vec2::new(SIZE.x * 0.5, SIZE.y * 0.5)).expect("the ball");
        let voxel = (at / app.doc.voxel_size()).round().as_ivec3();
        (at, app.doc.active_volume().display_paint().at(voxel))
    }

    /// A ball coated red, then drawn out over the middle further than the
    /// paint's band reaches: the raised surface is red, not clay.
    #[test]
    fn drawing_over_paint_keeps_it_painted() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.viewport_size = glam::Vec2::new(SIZE.x, SIZE.y);
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        update(&mut app, Message::Painting(PaintingMessage::Colour([200, 0, 0])));
        update(&mut app, Message::Painting(PaintingMessage::Coated));
        let (before, painted) = under_the_middle(&app);
        assert!(painted[3] > 0 && painted[0] > painted[1], "the coat: {painted:?}");

        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        update(&mut app, Message::BrushStrengthChanged(1.0));
        let band = NARROW_BAND * app.doc.voxel_size();
        let mut strokes = 0;
        while under_the_middle(&app).0.distance(before) < band * 2.0 {
            stroke(&mut app);
            strokes += 1;
            assert!(strokes < 60, "sixty strokes did not raise the surface past the band");
        }
        let (after, shown) = under_the_middle(&app);
        assert!(
            shown[3] > 0,
            "{strokes} strokes raised it {:.2} mm and it is bare",
            after.distance(before)
        );
        assert_eq!(shown, painted, "raised {:.2} mm, it shows {shown:?}", after.distance(before));
    }

    /// A coated ball, its surface under the middle of the view and the paint
    /// shown there.
    fn coated() -> (Hani, Vec3, [u8; 4]) {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        app.viewport_size = glam::Vec2::new(SIZE.x, SIZE.y);
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        update(&mut app, Message::Painting(PaintingMessage::Colour([200, 0, 0])));
        update(&mut app, Message::Painting(PaintingMessage::Coated));
        let (at, painted) = under_the_middle(&app);
        (app, at, painted)
    }

    /// Bulk keeps the coat on the surface it moves, past the band's depth.
    /// It always did (`hani_core::form`, which pulls each new surface voxel's
    /// paint from where it came from); this holds it from the application.
    #[test]
    fn a_bulk_keeps_the_paint_on_what_it_moves() {
        use crate::message::FormMessage;
        let (mut app, before, painted) = coated();
        update(&mut app, Message::Form(FormMessage::AmountChanged(2.0)));
        update(&mut app, Message::Form(FormMessage::BulkApplied));
        let (after, shown) = under_the_middle(&app);
        assert!(
            after.distance(before) > NARROW_BAND * app.doc.voxel_size(),
            "the bulk did not move it"
        );
        assert_eq!(shown, painted, "bulked {:.2} mm", after.distance(before));
    }

    /// The Move brush, pulling the ball's edge out past where it was, far
    /// enough to lock again on its way, keeps the coat on what it pulls out.
    /// It always did: Move warps the paint with the material
    /// (`paint_travels_with_the_material_under_a_move` in the core).
    #[test]
    fn a_long_move_keeps_the_paint_on_what_it_pulls() {
        let (mut app, _, painted) = coated();
        // The ball's edge along the middle row of the view.
        let row = SIZE.y * 0.5;
        let edge = (0..SIZE.x as i32 / 2)
            .map(|step| SIZE.x * 0.5 + step as f32)
            .find(|x| app.surface_under(glam::Vec2::new(*x, row)).is_none())
            .expect("the ball ends in the view");
        let beyond = glam::Vec2::new(edge + 30.0, row);
        assert!(app.surface_under(beyond).is_none(), "the fixture: past the edge is the ball");
        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        update(&mut app, Message::BrushRadiusChanged(6.0));
        let from = Vector::new(edge - 12.0, row);
        app.on_pointer(PointerEvent::Moved { position: from, size: SIZE });
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: from,
            size: SIZE,
        });
        for step in 1..=40 {
            app.on_pointer(PointerEvent::Moved {
                position: Vector::new(from.x + step as f32 * 2.0, row),
                size: SIZE,
            });
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        let at = app.surface_under(beyond).expect("the move pulled nothing out past the edge");
        let voxel = (at / app.doc.voxel_size()).round().as_ivec3();
        let shown = app.doc.active_volume().display_paint().at(voxel);
        assert_eq!(shown, painted, "the pulled material at {at} shows {shown:?}");
    }

    /// What carrying costs a pointer event of a Draw stroke over a coated
    /// ball, at the brush's starting size and at 10 mm: the time in
    /// `carry_paint_after_stamps` against the event's whole edit. Run with
    /// `--ignored --nocapture`; the numbers go in `docs/PERFORMANCE.md`.
    #[test]
    #[ignore]
    fn what_carrying_paint_costs() {
        for radius in [None, Some(10.0)] {
            let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
            app.viewport_size = glam::Vec2::new(SIZE.x, SIZE.y);
            update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
            update(&mut app, Message::Painting(PaintingMessage::Colour([200, 0, 0])));
            update(&mut app, Message::Painting(PaintingMessage::Coated));
            update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
            if let Some(radius) = radius {
                update(&mut app, Message::BrushRadiusChanged(radius));
            }
            let middle = Vector::new(SIZE.x * 0.5, SIZE.y * 0.5);
            app.on_pointer(PointerEvent::Moved { position: middle, size: SIZE });
            app.on_pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: middle,
                size: SIZE,
            });
            let (mut carrying, mut editing, mut events) = (0.0_f64, 0.0_f64, 0);
            for step in 0..60 {
                let x = middle.x + (step % 20) as f32 * 4.0 - 40.0;
                let started = std::time::Instant::now();
                app.on_pointer(PointerEvent::Moved {
                    position: Vector::new(x, middle.y),
                    size: SIZE,
                });
                let whole = started.elapsed().as_secs_f64() * 1000.0;
                let brush = app.effective_brush();
                let timed = std::time::Instant::now();
                app.carry_paint_after_stamps(brush, 1.0);
                carrying += timed.elapsed().as_secs_f64() * 1000.0;
                editing += whole;
                events += 1;
            }
            app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
            println!(
                "radius {:.1} mm: carrying {:.2} ms an event, the event's whole handling {:.2} ms, over {events}",
                app.brush.radius,
                carrying / f64::from(events),
                editing / f64::from(events),
            );
        }
    }
}
