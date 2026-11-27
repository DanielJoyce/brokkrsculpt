// SPDX-License-Identifier: AGPL-3.0-only

//! What a drybrush would catch, drawn under the brush before a stroke.
//!
//! Asked for by the maintainer on 2026-11-25: "dry brushes need to show how
//! high you are above a surface so you can see what parts you are gonna
//! hit." A drybrush here is not held at a height: it catches what stands
//! up into it by as much as its *Catch* says, from the sharpest edges at
//! the least to every raised form at the most. So what is shown is the
//! answer itself: while the pointer is over the model with the Drybrush
//! touch and no button down, each point of the surface under the ring that
//! a stroke would leave paint on is tinted, the more it would take the
//! stronger, asked as a stroke asks it
//! ([`hani_core::Volume::drybrush_would_catch`]). Moving *Catch* moves
//! what is tinted.
//!
//! The Sponge is shown the same way (asked for in the same request, and
//! done on 2026-11-28): each point under the ring is tinted by the share
//! of it the sponge would cover ([`hani_core::Volume::sponge_would_catch`]),
//! so edges, recesses and faces from the light read before a dab. Where
//! each speck falls is drawn afresh each stroke, so the share is shown and
//! not the specks.

use glam::Vec3;
use hani_core::paint::touch::TouchScratch;

use super::Hani;
use crate::message::PaintTouch;

/// The tint: the brush ring's own orange, so that it reads over any paint.
const TINT: [f32; 3] = [1.0, 0.45, 0.1];

/// The scratch the question is asked in, kept from one move to the next.
#[derive(Debug, Default)]
pub(super) struct DryPreview {
    scratch: TouchScratch,
}

impl Hani {
    /// Whether the catch is to be shown: the paint brush with the Drybrush
    /// or the Sponge touch, over the model, with no stroke under way.
    fn shows_the_catch(&self) -> bool {
        self.effective_brush().kind == hani_core::BrushKind::Paint
            && matches!(self.touch.kind, PaintTouch::Drybrush | PaintTouch::Sponge)
            && self.drag.is_none()
            && self.hover.is_some()
    }

    /// Tint what the drybrush or the sponge would catch under the ring.
    pub(super) fn add_catch_preview(&mut self, batch: &mut hani_gpu::OverlayBatch) {
        if !self.shows_the_catch() {
            return;
        }
        let Some(centre) = self.hover else {
            return;
        };
        let radius = self.effective_brush().radius;
        let voxel = self.doc.voxel_size();
        // The view's gain, as a stroke's coat takes it. See `app/paints.rs`.
        let gain = super::CAVITY_FEATURE_MM / voxel / 4.0;
        let (kind, dry, sponge) = (self.touch.kind, self.touch.dry, self.touch.sponge);
        let caught = {
            let Hani { doc, dry_preview, .. } = self;
            let volume = doc.active_volume();
            let scratch = &mut dry_preview.scratch;
            if kind == PaintTouch::Sponge {
                volume.sponge_would_catch(centre, radius, sponge, gain, scratch)
            } else {
                volume.drybrush_would_catch(centre, radius, dry, gain, scratch)
            }
        };
        // A sponge tints the whole ring, a point a voxel, and seen on a
        // screen squares wider than a voxel stacked their tint until every
        // share looked the same: its squares tile instead. And its share is
        // a third or so on a flat and less where it fades, which the
        // drybrush's floor of tint hid: its ramp starts lower and climbs
        // further.
        let (half, least, span) = if kind == PaintTouch::Sponge {
            (voxel * 0.5, 0.1, 1.6)
        } else {
            (voxel * 0.6, 0.25, 0.6)
        };
        for point in caught {
            let (across, along) = point.normal.any_orthonormal_pair();
            let middle = point.at + point.normal * voxel * 0.6;
            let corner = |a: f32, b: f32| middle + (across * a + along * b) * half;
            let (p, q, r, s): (Vec3, Vec3, Vec3, Vec3) =
                (corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0));
            let colour = [TINT[0], TINT[1], TINT[2], (least + span * point.amount).min(0.85)];
            batch.push_triangle(p, q, r, colour);
            batch.push_triangle(p, r, s, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, TouchSetting};

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// Over the model with the Drybrush, something is tinted only when the
    /// brush can catch anything there, and more catch tints no less; with
    /// another touch nothing is.
    #[test]
    fn the_drybrush_shows_what_it_would_catch() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Paint));
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Drybrush));
        // A box for a body, and the pointer on one of its edges: a smooth
        // ball stands up nowhere, and a drybrush rightly catches nothing on
        // it.
        let voxel = app.doc.voxel_size();
        let half = 8.0;
        let mut body = hani_core::Volume::new(voxel);
        let reach = Vec3::splat(half + 3.0);
        body.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(half);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / voxel).clamp(hani_core::brick::INSIDE, hani_core::brick::OUTSIDE)
        });
        app.doc.replace_active_volume(body);
        app.hover = Some(Vec3::new(half, half, 0.0));
        let shown = |app: &mut Hani| {
            let mut batch = hani_gpu::OverlayBatch::default();
            app.add_catch_preview(&mut batch);
            batch.surfaces.len()
        };
        let mut last = 0;
        for catch in [0.2, 0.6, 1.0] {
            update(&mut app, Message::PaintTouchChanged(TouchSetting::DryCatch(catch)));
            let now = shown(&mut app);
            assert!(now >= last, "catch {catch} tinted {now}, less than {last}");
            last = now;
        }
        assert!(last > 0, "a drybrush at full catch tints nothing on a ball");
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Loaded));
        assert_eq!(shown(&mut app), 0);
        app.hover = Some(Vec3::new(0.0, half, 0.0));
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Drybrush));
        update(&mut app, Message::PaintTouchChanged(TouchSetting::DryCatch(0.4)));
        assert_eq!(shown(&mut app), 0, "the middle of a face was tinted");
    }

    /// With the Sponge, the ring is tinted wherever the sponge would lay
    /// anything, and a sponge that wants the light wholly tints nothing on
    /// the face of a box turned from it.
    #[test]
    fn the_sponge_shows_what_it_would_catch() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Paint));
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Sponge));
        let voxel = app.doc.voxel_size();
        let half = 8.0;
        let mut body = hani_core::Volume::new(voxel);
        let reach = Vec3::splat(half + 3.0);
        body.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(half);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / voxel).clamp(hani_core::brick::INSIDE, hani_core::brick::OUTSIDE)
        });
        app.doc.replace_active_volume(body);
        let shown = |app: &mut Hani| {
            let mut batch = hani_gpu::OverlayBatch::default();
            app.add_catch_preview(&mut batch);
            batch.surfaces.len()
        };
        // The light is overhead, up the Y axis, unless a zenithal says
        // otherwise: the top face is lit and the bottom is not.
        let light = app.touch.sponge.light;
        app.hover = Some(light * half);
        assert!(shown(&mut app) > 0, "the lit face showed nothing");
        update(&mut app, Message::PaintTouchChanged(TouchSetting::SpongeLight(1.0)));
        app.hover = Some(-light * half);
        assert_eq!(shown(&mut app), 0, "the face from the light was tinted");
        app.hover = Some(light * half);
        assert!(shown(&mut app) > 0, "the lit face showed nothing wanting the light");
    }

    /// What showing the catch costs a pointer move, on the edge of a box,
    /// at brush sizes from small to large. A measurement, run by hand:
    /// `cargo test --release -p hani-app -- --ignored what_the_catch_costs --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn what_the_catch_costs() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Paint));
        update(&mut app, Message::PaintTouchChosen(PaintTouch::Drybrush));
        let voxel = app.doc.voxel_size();
        let half = 12.0;
        let mut body = hani_core::Volume::new(voxel);
        let reach = Vec3::splat(half + 3.0);
        body.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(half);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / voxel).clamp(hani_core::brick::INSIDE, hani_core::brick::OUTSIDE)
        });
        app.doc.replace_active_volume(body);
        app.hover = Some(Vec3::new(half, half, 0.0));
        for radius in [2.0, 5.0, 10.0] {
            app.brush.radius = radius;
            let mut batch = hani_gpu::OverlayBatch::default();
            app.add_catch_preview(&mut batch);
            let runs = 10;
            let started = web_time::Instant::now();
            for _ in 0..runs {
                batch.surfaces.clear();
                app.add_catch_preview(&mut batch);
            }
            let each = started.elapsed().as_secs_f32() * 1000.0 / runs as f32;
            eprintln!("radius {radius} mm: {each:.2} ms, {} triangles", batch.surfaces.len() / 3);
        }
    }
}
