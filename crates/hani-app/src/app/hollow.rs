// SPDX-License-Identifier: AGPL-3.0-only

//! Shell and Extract: hollowing a body to a wall, and thickening the masked
//! part of its surface into a body of its own.
//!
//! `hani_core::shell` and `hani_core::extract` have the geometry. Here
//! are the rows that reach them: a *Shell* row in FORM, and an EXTRACT
//! block in Sculpt, where the mask is painted. A child module of `app` for
//! the reason `form.rs` is (`docs/STUDIO-PLAN.md` decision D6).
//!
//! A shell is one undo entry holding the body as it was. An extract leaves
//! the body alone and adds the plate as a new row directly below it, one
//! undo entry, and makes it the active body.

use web_time::Instant;

use hani_core::extract::Extraction;
use hani_core::{Change, Entry, NodeId, Volume};
use iced::Element;
use iced::widget::{column, row};

use super::Hani;
use crate::message::{HollowMessage, Message};
use crate::theme;

pub(super) const WALL_RANGE: std::ops::RangeInclusive<f32> = 0.2..=20.0;
pub(super) const THICKNESS_RANGE: std::ops::RangeInclusive<f32> = 0.1..=20.0;
pub(super) const OFFSET_RANGE: std::ops::RangeInclusive<f32> = -10.0..=10.0;

/// What the application remembers for the two tools.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct HollowState {
    /// A shell's wall, in millimetres.
    pub wall_mm: f32,
    /// An extracted plate.
    pub extract: Extraction,
}

impl Default for HollowState {
    fn default() -> Self {
        Self {
            wall_mm: 1.5,
            extract: Extraction { thickness_mm: 1.0, inward: 0.0, offset_mm: 0.0 },
        }
    }
}

fn clamped(value: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if value.is_nan() { *range.start() } else { value.clamp(*range.start(), *range.end()) }
}

impl Hani {
    pub(super) fn hollow_changed(&mut self, message: HollowMessage) {
        match message {
            HollowMessage::WallChanged(mm) => self.hollow.wall_mm = clamped(mm, WALL_RANGE),
            HollowMessage::ShellApplied => self.shell_active(),
            HollowMessage::ThicknessChanged(mm) => {
                self.hollow.extract.thickness_mm = clamped(mm, THICKNESS_RANGE);
            }
            HollowMessage::InwardChanged(share) => {
                self.hollow.extract.inward = clamped(share, 0.0..=1.0);
            }
            HollowMessage::OffsetChanged(mm) => {
                self.hollow.extract.offset_mm = clamped(mm, OFFSET_RANGE);
            }
            HollowMessage::ExtractApplied => self.extract_active(),
        }
    }

    /// Hollow the active body, as one undo entry, or say why not.
    fn shell_active(&mut self) {
        let started = Instant::now();
        let body = self.doc.active();
        let name = self.active_body_name();
        let wall = self.hollow.wall_mm;
        let shelled = match self.doc.active_volume().shelled(wall) {
            Ok(shelled) => shelled,
            Err(why) => {
                self.status = format!("{name} not hollowed: {why}");
                return;
            }
        };
        if self.swap_in(body, shelled) {
            self.status = format!(
                "hollowed {name} to a {wall:.2} mm wall ({:.0} ms)",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Thicken the masked part of the active body into a new body.
    fn extract_active(&mut self) {
        let started = Instant::now();
        let source = self.doc.active();
        let name = self.active_body_name();
        let plate = match self.doc.active_volume().extracted(self.hollow.extract) {
            Ok(plate) => plate,
            Err(why) => {
                self.status = format!("nothing extracted from {name}: {why}");
                return;
            }
        };
        let plate_name = hani_core::name_that_fits(&format!("{name} plate")).to_string();
        let took = started.elapsed().as_secs_f64() * 1000.0;
        let note = format!(
            "extracted a {:.2} mm plate from {name} as {plate_name} ({took:.0} ms)",
            self.hollow.extract.thickness_mm
        );
        self.add_made_body(Some(source), plate_name, plate, note);
    }

    /// Add a body made by a tool, as one undo entry, and make it active.
    /// Directly below `beside` and at its depth when there is one, at the end
    /// otherwise. Refused, with the reason on the status line, when the
    /// document is full. Shared with the revolve (`revolve.rs`).
    pub(super) fn add_made_body(
        &mut self,
        beside: Option<NodeId>,
        name: String,
        volume: Volume,
        note: String,
    ) -> Option<NodeId> {
        if self.doc.body_count() >= hani_core::MAX_BODIES {
            self.status = format!(
                "could not add {name}: this document holds {} bodies, which is the limit",
                self.doc.body_count()
            );
            return None;
        }
        if self.doc.node_count() >= hani_core::MAX_NODES {
            self.status = format!(
                "could not add {name}: this document holds {} rows, which is the limit",
                self.doc.node_count()
            );
            return None;
        }
        let placed = beside.and_then(|id| {
            let at = self.doc.index_of(id)?;
            let depth = self.doc.node(id)?.depth();
            Some((at + 1, depth))
        });
        let (at, id) = match placed {
            Some((at, depth)) => (at, self.doc.insert_body(at, depth, name, volume)),
            None => {
                let id = self.doc.add_body(name, volume);
                (self.doc.node_count() - 1, id)
            }
        };
        let before = self.history.stats();
        self.history.push(Entry::new(vec![Change::NodeAdded { at, id }]));
        self.record_history(before);
        self.unsaved = true;
        self.select_body_saying(id, note);
        self.remesh_dirty();
        self.refresh_overlay();
        Some(id)
    }

    /// The shell's row, in FORM.
    pub(super) fn shell_row(&self) -> Element<'_, Message> {
        let changed = Message::Hollow;
        crate::ui::number(
            &self.drafts,
            "hollow.wall",
            "Shell",
            self.hollow.wall_mm,
            WALL_RANGE,
            move |mm| changed(HollowMessage::WallChanged(mm)),
        )
        .step(0.1)
        .shown(format!("{:.2} mm", self.hollow.wall_mm))
        .trailing(crate::ui::tip(
            crate::ui::button_secondary("Apply", Some(changed(HollowMessage::ShellApplied))),
            "Hollow the body to this wall",
        ))
        .into()
    }

    /// The EXTRACT block.
    pub(super) fn extract_panel(&self) -> Element<'_, Message> {
        let changed = Message::Hollow;
        let how = self.hollow.extract;
        column![
            crate::ui::number(
                &self.drafts,
                "extract.thickness",
                "Thickness",
                how.thickness_mm,
                THICKNESS_RANGE,
                move |mm| changed(HollowMessage::ThicknessChanged(mm)),
            )
            .step(0.1)
            .shown(format!("{:.2} mm", how.thickness_mm)),
            crate::ui::number(
                &self.drafts,
                "extract.inward",
                "Inward",
                how.inward,
                0.0..=1.0,
                move |share| changed(HollowMessage::InwardChanged(share)),
            )
            .step(0.05)
            .shown(format!("{:.0}%", how.inward * 100.0)),
            crate::ui::number(
                &self.drafts,
                "extract.offset",
                "Offset",
                how.offset_mm,
                OFFSET_RANGE,
                move |mm| changed(HollowMessage::OffsetChanged(mm)),
            )
            .step(0.1)
            .shown(format!("{:+.2} mm", how.offset_mm)),
            row![crate::ui::tip(
                crate::ui::button_secondary(
                    "Extract",
                    Some(changed(HollowMessage::ExtractApplied))
                ),
                "The masked area as a new body",
            )],
        ]
        .spacing(theme::S2)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// How far out the surface is along a direction from the middle of the
    /// active body, crossing in from outside.
    fn outer_radius(volume: &Volume, direction: Vec3) -> f32 {
        let mut radius = 60.0;
        while radius > 0.0 && volume.sample_world(direction * radius) >= 0.0 {
            radius -= 0.05;
        }
        radius
    }

    #[test]
    fn shell_hollows_the_active_body_and_undo_fills_it() {
        let mut app = app();
        let outer = outer_radius(app.doc.active_volume(), Vec3::X);
        assert!(app.doc.active_volume().sample_world(Vec3::ZERO) < 0.0, "starts solid");
        update(&mut app, Message::Hollow(HollowMessage::WallChanged(2.0)));
        update(&mut app, Message::Hollow(HollowMessage::ShellApplied));
        let volume = app.doc.active_volume();
        assert!(volume.sample_world(Vec3::ZERO) > 0.0, "{}", app.status);
        assert!(volume.sample_world(Vec3::X * (outer - 1.0)) < 0.0, "the wall is there");
        assert!(volume.sample_world(Vec3::X * (outer - 3.0)) > 0.0, "and two millimetres thick");
        assert!((outer_radius(volume, Vec3::X) - outer).abs() < 0.05, "outside unchanged");
        update(&mut app, Message::Undo);
        assert!(app.doc.active_volume().sample_world(Vec3::ZERO) < 0.0, "undo fills it");
    }

    #[test]
    fn extract_makes_the_masked_cap_a_new_body_below_it() {
        let mut app = app();
        let source = app.doc.active();
        let outer = outer_radius(app.doc.active_volume(), Vec3::Y);
        let cap = outer * 0.7;
        hani_core::extract::mask_where(app.doc.active_volume_mut(), |world| world.y > cap);
        let bodies = app.doc.body_count();
        update(&mut app, Message::Hollow(HollowMessage::ThicknessChanged(1.0)));
        update(&mut app, Message::Hollow(HollowMessage::InwardChanged(0.0)));
        update(&mut app, Message::Hollow(HollowMessage::ExtractApplied));
        assert_eq!(app.doc.body_count(), bodies + 1, "{}", app.status);
        let plate = app.doc.active();
        assert_ne!(plate, source, "the plate is made active");
        assert_eq!(app.doc.index_of(plate), app.doc.index_of(source).map(|at| at + 1));
        let volume = app.doc.volume(plate).expect("a body");
        assert!(volume.sample_world(Vec3::Y * (outer + 0.5)) < 0.0, "the plate sits on the cap");
        assert!(volume.sample_world(Vec3::Y * (outer - 0.5)) > 0.0, "all of it outward");
        assert!(volume.sample_world(-Vec3::Y * outer) > 0.0, "the unmasked side is not in it");
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.body_count(), bodies, "undo takes the plate away");
    }

    #[test]
    fn extract_with_nothing_masked_says_so_and_adds_nothing() {
        let mut app = app();
        let bodies = app.doc.body_count();
        update(&mut app, Message::Hollow(HollowMessage::ExtractApplied));
        assert_eq!(app.doc.body_count(), bodies);
        assert!(app.status.contains("nothing is masked"), "{}", app.status);
        drop(app.extract_panel());
        drop(app.shell_row());
    }
}
