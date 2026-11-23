// SPDX-License-Identifier: AGPL-3.0-only

//! The form tools: thicken or thin a body, soften or exaggerate its detail,
//! and keep what is on its surface either way.
//!
//! `brokkr_core::form` has the argument and the measurements. Here are the
//! three sliders and two buttons that reach it, a child module of `app` for
//! the reason `paints.rs` and `studio.rs` are (`docs/STUDIO-PLAN.md`
//! decision D6).
//!
//! Both tools work on the whole of the active body, less whatever the mask
//! protects, and each is one undo entry holding the body as it was.
//!
//! # They run where they are called, and they are not instant
//!
//! A bulk is a pyramid of the body, one pass over its band and a
//! redistance: measured at 1.8 s in all for the starting ball at a quarter
//! millimetre, in release. That is on the update thread, like an export,
//! and like an export it should move to a task. It is owed.

use std::time::Instant;

use brokkr_core::form::{FormError, level_for};
use brokkr_core::{Change, Entry, FormPyramid, Volume};
use iced::widget::{button, column, row, slider, text};
use iced::{Element, Length};

use super::Brokkr;
use crate::message::{FormMessage, Message};
use crate::theme;

/// The sliders' ranges, which are also what a setting is clamped to.
pub(super) const DETAIL_RANGE: std::ops::RangeInclusive<f32> = 0.2..=10.0;
pub(super) const AMOUNT_RANGE: std::ops::RangeInclusive<f32> = -5.0..=5.0;
pub(super) const GAIN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=3.0;

/// What the application remembers for the form tools.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FormState {
    /// What counts as detail, in millimetres: features smaller than this
    /// ride along, larger ones are the form.
    pub detail_mm: f32,
    /// How far a bulk moves the surface, in millimetres. Negative thins.
    pub amount_mm: f32,
    /// What the detail is multiplied by. One is as it is.
    pub gain: f32,
}

impl Default for FormState {
    fn default() -> Self {
        Self { detail_mm: 2.0, amount_mm: 0.5, gain: 1.5 }
    }
}

/// Into a slider's range, with a NaN sent to the bottom of it.
fn clamped(value: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if value.is_nan() { *range.start() } else { value.clamp(*range.start(), *range.end()) }
}

/// Which of the two tools.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Reshape {
    Bulk,
    DetailGain,
}

impl Brokkr {
    pub(super) fn form_changed(&mut self, message: FormMessage) {
        match message {
            FormMessage::DetailSizeChanged(mm) => {
                self.form.detail_mm = clamped(mm, DETAIL_RANGE);
            }
            FormMessage::AmountChanged(mm) => self.form.amount_mm = clamped(mm, AMOUNT_RANGE),
            FormMessage::GainChanged(gain) => self.form.gain = clamped(gain, GAIN_RANGE),
            FormMessage::BulkApplied => self.reshape(Reshape::Bulk),
            FormMessage::DetailGainApplied => self.reshape(Reshape::DetailGain),
        }
    }

    /// Rebuild the active body through one of the form tools, as one undo
    /// entry, or say why not and change nothing.
    fn reshape(&mut self, what: Reshape) {
        let started = Instant::now();
        let body = self.doc.active();
        let voxel_size = self.doc.voxel_size();
        let reshaped = {
            let volume = self.doc.active_volume();
            let wanted = level_for(voxel_size, self.form.detail_mm);
            let pyramid = FormPyramid::build(volume, wanted + 1);
            // A model smaller than the detail size has no form that coarse.
            // The coarsest it does have is the nearest thing to what was
            // asked, and the status line says which was used.
            let Some(level) = pyramid.level_count().checked_sub(1).map(|last| wanted.min(last))
            else {
                self.status = FormError::NoSuchLevel.to_string();
                return;
            };
            let used_mm = voxel_size * (1_u32 << (level + 1)) as f32;
            let result = match what {
                Reshape::Bulk => volume.bulked(&pyramid, level, self.form.amount_mm),
                Reshape::DetailGain => volume.with_detail_gain(&pyramid, level, self.form.gain),
            };
            result.map(|volume| (volume, used_mm))
        };
        let (mut reshaped, used_mm) = match reshaped {
            Ok(done) => done,
            Err(why) => {
                self.status = format!("not done: {why}");
                return;
            }
        };
        // A warp leaves the gradient off one, and every brush after it reads
        // the gradient. Gated on measured drift, so it costs one pass when
        // there is nothing to fix.
        reshaped.redistance();

        let Some(previous) = self.doc.replace_volume(body, reshaped) else {
            return;
        };
        // The bricks the old field occupied, marked in the new one, or the
        // renderer goes on drawing triangles nothing will overwrite.
        let stale: Vec<brokkr_core::BrickCoord> = previous.brick_coords().collect();
        if let Some(landed) = self.doc.volume_mut(body) {
            landed.mark_everything_dirty();
            for coord in stale {
                landed.mark_dirty(coord);
            }
        }
        self.keep_for_undo(body, previous);
        self.unsaved = true;
        self.thumbs.geometry_changed(body);
        self.remesh_dirty();
        let did = match what {
            Reshape::Bulk if self.form.amount_mm < 0.0 => {
                format!("slimmed by {:.2} mm", -self.form.amount_mm)
            }
            Reshape::Bulk => format!("bulked by {:.2} mm", self.form.amount_mm),
            Reshape::DetailGain => format!("detail at {:.2} of what it was", self.form.gain),
        };
        self.status = format!(
            "{did}, keeping detail finer than {used_mm:.2} mm ({:.0} ms)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    /// One undo entry holding the body as it was.
    fn keep_for_undo(&mut self, body: brokkr_core::NodeId, previous: Volume) {
        let before = self.history.stats();
        self.history
            .push(Entry::new(vec![Change::WholeVolume { body, volume: Box::new(previous) }]));
        self.record_history(before);
    }

    /// The form controls, under the studio block.
    pub(super) fn form_panel(&self) -> Element<'_, Message> {
        let reading = |label: String| text(label).size(theme::CAPTION_SIZE).color(theme::TEXT_DIM);
        let changed = |message: FormMessage| Message::Form(message);
        let form = self.form;
        column![
            text("FORM").size(theme::CAPTION_SIZE).color(theme::TEXT_MUTE),
            reading(format!("Detail is finer than  {:.1} mm", form.detail_mm)),
            slider(DETAIL_RANGE, form.detail_mm, move |mm| {
                changed(FormMessage::DetailSizeChanged(mm))
            })
            .step(0.1_f32),
            reading(format!("Bulk  {:+.2} mm", form.amount_mm)),
            row![
                slider(AMOUNT_RANGE, form.amount_mm, move |mm| {
                    changed(FormMessage::AmountChanged(mm))
                })
                .step(0.05_f32),
                button(text("Apply").size(theme::CAPTION_SIZE))
                    .style(theme::tool_button)
                    .on_press(changed(FormMessage::BulkApplied)),
            ]
            .spacing(theme::S2),
            reading(format!("Detail  x {:.2}", form.gain)),
            row![
                slider(GAIN_RANGE, form.gain, move |gain| changed(FormMessage::GainChanged(gain)))
                    .step(0.05_f32),
                button(text("Apply").size(theme::CAPTION_SIZE))
                    .style(theme::tool_button)
                    .on_press(changed(FormMessage::DetailGainApplied)),
            ]
            .spacing(theme::S2),
            text("the whole body, less what the mask protects")
                .size(theme::CAPTION_SIZE)
                .color(theme::TEXT_MUTE)
                .width(Length::Fill),
        ]
        .spacing(theme::S1)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{PointerButton, PointerEvent};
    use glam::Vec3;

    const SIZE: iced::Vector = iced::Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    fn set(app: &mut Brokkr, message: FormMessage) {
        update(app, Message::Form(message));
    }

    /// How far out the surface is along a direction from the middle of the
    /// model, in millimetres.
    fn radius_along(app: &Brokkr, direction: Vec3) -> f32 {
        let volume = app.doc.active_volume();
        let direction = direction.normalize();
        let mut radius = 10.0;
        while radius < 60.0 && volume.sample_world(direction * radius) < 0.0 {
            radius += 0.01;
        }
        radius
    }

    /// A short Draw stroke with a small brush across the middle of the
    /// model: a bump about two millimetres across, which is detail at a
    /// detail size of four.
    fn bump(app: &mut Brokkr) {
        update(app, Message::BrushRadiusChanged(1.0));
        update(app, Message::BrushStrengthChanged(0.6));
        let centre = iced::Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: centre,
                size: SIZE,
            }),
        );
        for _ in 0..3 {
            for step in [4.0, -4.0] {
                let position = iced::Vector::new(centre.x + step, centre.y);
                update(app, Message::Pointer(PointerEvent::Moved { position, size: SIZE }));
            }
        }
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    fn toward_camera() -> Vec3 {
        app().camera.eye().normalize()
    }

    /// Whether two applications' active bodies hold the same values.
    fn same_values(one: &Brokkr, other: &Brokkr) -> bool {
        let (a, b) = (one.doc.active_volume(), other.doc.active_volume());
        let (lo, hi) = a.voxel_bounds(Vec3::splat(-40.0), Vec3::splat(40.0));
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = glam::IVec3::new(x, y, z);
                    if a.sample_voxel(voxel).to_bits() != b.sample_voxel(voxel).to_bits() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// A bulk through the application: the body is that much bigger all
    /// round, a negative one that much smaller, each is one undo back to
    /// exactly what it was, and the status line says what was done.
    #[test]
    fn a_bulk_makes_the_body_bigger_all_round_and_is_one_undo() {
        let fresh = app();
        let radius = radius_along(&fresh, Vec3::X);
        for amount in [1.0_f32, -0.75] {
            let mut app = app();
            set(&mut app, FormMessage::AmountChanged(amount));
            let undo_before = app.history.stats();
            set(&mut app, FormMessage::BulkApplied);
            for direction in [Vec3::X, -Vec3::Y, Vec3::new(1.0, 2.0, -1.5)] {
                let grew = radius_along(&app, direction) - radius;
                assert!(
                    (grew - amount).abs() < 0.06,
                    "a bulk of {amount} mm moved the surface {grew} mm along {direction}"
                );
            }
            assert!(app.unsaved, "a bulk did not dirty the document");
            assert_ne!(app.history.stats(), undo_before, "a bulk left nothing to undo");
            let said = if amount > 0.0 { "bulked by 1.00 mm" } else { "slimmed by 0.75 mm" };
            assert!(app.status.contains(said), "{}", app.status);
            assert_eq!(app.doc.active_volume().dirty_count(), 0, "the remesh left bricks dirty");

            update(&mut app, Message::Undo);
            assert!(same_values(&app, &fresh), "undoing a bulk did not put the body back");
            update(&mut app, Message::Redo);
            assert!((radius_along(&app, Vec3::X) - radius - amount).abs() < 0.06, "redo");
        }
    }

    /// Asked for more than the detail size can carry, nothing is done and
    /// nothing is left to undo, and the status line says how much would be.
    #[test]
    fn a_bulk_too_far_for_its_detail_size_is_refused_and_changes_nothing() {
        let fresh = app();
        let mut app = app();
        set(&mut app, FormMessage::DetailSizeChanged(0.5));
        set(&mut app, FormMessage::AmountChanged(4.0));
        let undo_before = app.history.stats();
        set(&mut app, FormMessage::BulkApplied);
        assert!(app.status.starts_with("not done"), "{}", app.status);
        assert!(app.status.contains("mm at most"), "{}", app.status);
        assert!(same_values(&app, &fresh), "a refused bulk changed the body");
        assert_eq!(app.history.stats(), undo_before, "a refused bulk reached the history");
        assert!(!app.unsaved, "a refused bulk dirtied the document");
    }

    /// The detail gain on a sculpted bump: at nothing it is mostly gone, at
    /// more than one it stands further off, and the ball it sits on stays
    /// the size it was.
    #[test]
    fn the_detail_gain_flattens_or_raises_a_bump_and_leaves_the_ball_its_size() {
        let fresh = app();
        let ball = radius_along(&fresh, Vec3::X);
        let with_gain = |gain: f32| {
            let mut app = app();
            bump(&mut app);
            let before = radius_along(&app, toward_camera());
            // A detail size over the bump's own, so the bump is detail,
            // and well under the ball's, so the ball is form.
            set(&mut app, FormMessage::DetailSizeChanged(4.0));
            set(&mut app, FormMessage::GainChanged(gain));
            set(&mut app, FormMessage::DetailGainApplied);
            assert!(app.status.contains("detail at"), "{}", app.status);
            // The field is left a distance field, near enough for every
            // brush that reads its gradient afterwards. Measured: 0.57 at
            // twice the gain with the redistance, and 2.44 without it.
            let drift = app.doc.active_volume().gradient_drift();
            assert!(drift < 1.0, "a gain of {gain} left the gradient off by {drift}");
            // Away from the bump, on the far side, the ball has not moved.
            let far = radius_along(&app, -toward_camera()) - ball;
            // By no more than the pyramid's own grain at this detail size:
            // measured at 0.104 mm at worst, in `brokkr_core::form`. Without
            // the correction there it is 0.22 mm on average.
            assert!(far.abs() < 0.12, "a gain of {gain} moved the ball's far side by {far} mm");
            (before - ball, radius_along(&app, toward_camera()) - ball)
        };
        let (bump_height, flattened) = with_gain(0.0);
        assert!(bump_height > 0.25, "the fixture's bump is only {bump_height} mm high");
        assert!(
            flattened < bump_height * 0.6,
            "at no gain a bump of {bump_height} mm still stands {flattened} mm"
        );
        let (_, raised) = with_gain(2.0);
        assert!(
            raised > bump_height * 1.3,
            "at twice the gain a bump of {bump_height} mm stands {raised} mm"
        );

        // And it is one undo.
        let mut app = app();
        bump(&mut app);
        let bumped = radius_along(&app, toward_camera());
        set(&mut app, FormMessage::GainChanged(0.0));
        set(&mut app, FormMessage::DetailSizeChanged(4.0));
        set(&mut app, FormMessage::DetailGainApplied);
        update(&mut app, Message::Undo);
        assert_eq!(radius_along(&app, toward_camera()), bumped, "a detail gain was not one undo");
    }

    #[test]
    fn form_settings_land_where_they_say_and_are_clamped() {
        let mut app = app();
        set(&mut app, FormMessage::DetailSizeChanged(3.3));
        set(&mut app, FormMessage::AmountChanged(-1.2));
        set(&mut app, FormMessage::GainChanged(0.4));
        assert_eq!(app.form, FormState { detail_mm: 3.3, amount_mm: -1.2, gain: 0.4 });
        set(&mut app, FormMessage::DetailSizeChanged(0.0));
        set(&mut app, FormMessage::AmountChanged(99.0));
        set(&mut app, FormMessage::GainChanged(-2.0));
        assert_eq!(app.form.detail_mm, *DETAIL_RANGE.start(), "a detail size of nothing");
        assert_eq!(app.form.amount_mm, *AMOUNT_RANGE.end());
        assert_eq!(app.form.gain, 0.0);
        for nan in [
            FormMessage::DetailSizeChanged(f32::NAN),
            FormMessage::AmountChanged(f32::NAN),
            FormMessage::GainChanged(f32::NAN),
        ] {
            set(&mut app, nan);
        }
        assert_eq!(
            app.form,
            FormState {
                detail_mm: *DETAIL_RANGE.start(),
                amount_mm: *AMOUNT_RANGE.start(),
                gain: *GAIN_RANGE.start()
            }
        );
        assert!(!app.unsaved, "a setting dirtied the document");
        drop(app.form_panel());
    }
}
