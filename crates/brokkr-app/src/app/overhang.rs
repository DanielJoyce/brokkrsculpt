// SPDX-License-Identifier: AGPL-3.0-only

//! The overhang view: where the model, printed as it stands, would be
//! printed over nothing.
//!
//! [`brokkr_core::overhang`] is the rule and the reasons. This is the
//! switch, the one setting (how steep a lean the printer manages), and the
//! two numbers the shader is handed for them. A view and nothing else: no
//! stroke, no export and no file is changed by it, and it is not saved
//! with a document. The export does say how much overhangs when the view
//! is on, which is the one place the number is worth the pass over the
//! mesh that working it out costs.
//!
//! # The plate
//!
//! The shader needs the height nothing is an overhang below, which is a
//! first layer above the model's lowest point. Finding the lowest point is
//! a scan of every dense brick of every visible body
//! (`Volume::lowest_point`), which is a user-action cost and this is
//! asked for every frame. So it is kept, and found again only when the
//! undo history's counters or the document's rough bounds have changed:
//! every finished edit, undo, redo, hide and show moves the first, and a
//! new document moves the second. In the middle of a stroke it is the
//! plate from before the stroke.

use brokkr_core::overhang::{self, Overhangs, STEEPEST_RANGE, USUAL_STEEPEST};
use glam::Vec3;
use iced::Element;
use iced::widget::{checkbox, column};

use super::Brokkr;
use crate::message::{Message, OverhangMessage};
use crate::theme;

/// What the kept plate was found for.
type Found = (brokkr_core::HistoryStats, Option<(Vec3, Vec3)>);

/// The view's own state, in one field of `Brokkr`.
#[derive(Debug)]
pub(super) struct OverhangState {
    pub shown: bool,
    /// The steepest lean from upright the printer manages, in degrees.
    pub steepest: f32,
    /// The plate, and what it was found for.
    plate: Option<(Found, f32)>,
    /// How many times the plate has been looked for, which is what a test
    /// of "not every frame" has to count.
    #[cfg(test)]
    finds: u32,
}

impl Default for OverhangState {
    fn default() -> Self {
        Self {
            shown: false,
            steepest: USUAL_STEEPEST,
            plate: None,
            #[cfg(test)]
            finds: 0,
        }
    }
}

impl Brokkr {
    pub(super) fn overhang_changed(&mut self, message: OverhangMessage) {
        match message {
            OverhangMessage::Shown(shown) => {
                self.overhang.shown = shown;
                if shown {
                    self.status = "overhangs shown, for the model printed as it stands".to_string();
                }
            }
            OverhangMessage::Steepest(degrees) => {
                // Not a number is no setting at all, and is the usual one.
                self.overhang.steepest = if degrees.is_nan() {
                    USUAL_STEEPEST
                } else {
                    degrees.clamp(*STEEPEST_RANGE.start(), *STEEPEST_RANGE.end())
                };
            }
        }
    }

    /// The lowest point of what would be exported: of every body whose eye
    /// is open. `None` for a document with nothing in it.
    fn lowest_point(&self) -> Option<f32> {
        let mut visible = Vec::new();
        self.doc.saved_visibility(&mut visible);
        self.doc
            .nodes()
            .iter()
            .zip(&visible)
            .filter(|(_, shown)| **shown)
            .filter_map(|(node, _)| node.volume()?.lowest_point())
            .reduce(f32::min)
    }

    /// What the shader is handed: the limit and the plate, or two zeros
    /// with the view off, which draws no overhangs.
    pub(super) fn overhang_view(&mut self) -> [f32; 2] {
        if !self.overhang.shown {
            return [0.0, 0.0];
        }
        let found: Found = (self.history.stats(), self.doc.world_bounds());
        let plate = match self.overhang.plate {
            Some((kept, plate)) if kept == found => plate,
            _ => {
                // With nothing to stand on a plate there is no plate, and
                // nothing is drawn that could be under it.
                let plate = self.lowest_point().map_or(f32::MIN, overhang::plate);
                self.overhang.plate = Some((found, plate));
                #[cfg(test)]
                {
                    self.overhang.finds += 1;
                }
                plate
            }
        };
        [overhang::limit(self.overhang.steepest), plate]
    }

    /// What an export says about overhangs: how much of what it wrote
    /// overhangs, or nothing at all with the view off, which leaves the
    /// line an export writes as it was.
    pub(super) fn overhang_summary(&self, bodies: &[brokkr_core::ExportedBody]) -> String {
        if !self.overhang.shown {
            return String::new();
        }
        let lowest = bodies.iter().filter_map(|(_, mesh, _)| mesh.lowest()).reduce(f32::min);
        let Some(lowest) = lowest else {
            return String::new();
        };
        let (limit, plate) = (overhang::limit(self.overhang.steepest), overhang::plate(lowest));
        let found = bodies
            .iter()
            .map(|(_, mesh, _)| mesh.overhangs(limit, plate))
            .fold(Overhangs::default(), Overhangs::and);
        format!("; {}", found.summary(self.overhang.steepest))
    }

    /// The switch, and under it, when it is on, the one setting.
    pub(super) fn overhang_panel(&self) -> Element<'_, Message> {
        let changed = |message: OverhangMessage| Message::Overhang(message);
        let mut panel = column![
            checkbox(self.overhang.shown)
                .label("show overhangs")
                .on_toggle(move |shown| changed(OverhangMessage::Shown(shown)))
                .text_size(theme::LABEL_SIZE),
        ]
        .spacing(theme::S1);
        if self.overhang.shown {
            panel = panel.push(
                crate::ui::number(
                    &self.drafts,
                    "overhang.steepest",
                    "Leaning past upright by",
                    self.overhang.steepest,
                    STEEPEST_RANGE,
                    move |degrees| changed(OverhangMessage::Steepest(degrees)),
                )
                .step(1.0)
                .shown(format!("{:.0}°", self.overhang.steepest)),
            );
        }
        panel.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ExportFormat, FormMessage};

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn set(app: &mut Brokkr, message: OverhangMessage) {
        update(app, Message::Overhang(message));
    }

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    /// What the last frame told the shader.
    fn published(app: &mut Brokkr) -> [f32; 2] {
        app.publish_mask_view();
        let [_, _, limit, plate] = app.shared.cavity_view().packed();
        [limit, plate]
    }

    /// Off, the shader is told nothing. On, it is told the limit for the
    /// angle set and a plate one first layer over the bottom of the model.
    #[test]
    fn the_view_tells_the_shader_the_limit_and_the_plate_and_nothing_when_off() {
        let mut app = app();
        assert_eq!(published(&mut app), [0.0, 0.0]);
        drop(app.overhang_panel());

        set(&mut app, OverhangMessage::Shown(true));
        drop(app.overhang_panel());
        let [limit, plate] = published(&mut app);
        assert_eq!(limit, overhang::limit(45.0));
        let bottom = -super::super::MODEL_RADIUS_MM;
        assert!(
            (plate - (bottom + overhang::FIRST_LAYER_MM)).abs() < 0.3,
            "the plate is at {plate} mm under a ball whose bottom is at {bottom}"
        );

        // The angle is the slider's, kept to its range.
        for (asked, taken) in [(60.0, 60.0), (5.0, 20.0), (500.0, 80.0), (f32::NAN, 45.0)] {
            set(&mut app, OverhangMessage::Steepest(asked));
            assert_eq!(app.overhang.steepest, taken, "asked for {asked}");
            assert_eq!(published(&mut app)[0], overhang::limit(taken));
        }

        set(&mut app, OverhangMessage::Shown(false));
        assert_eq!(published(&mut app), [0.0, 0.0]);
        // The angle is remembered across the switch.
        set(&mut app, OverhangMessage::Shown(true));
        assert_eq!(published(&mut app)[0], overhang::limit(45.0));
    }

    /// The plate follows the model: found again after an edit that moves
    /// the bottom of it, and not found again when nothing has changed.
    #[test]
    fn the_plate_is_found_again_when_the_model_changes_and_only_then() {
        let mut app = app();
        set(&mut app, OverhangMessage::Shown(true));
        let before = published(&mut app)[1];
        let kept = app.overhang.plate;
        assert!(kept.is_some());
        // Frames go by and nothing changes: the same plate, from the same
        // finding of it. Finding it is a scan of the whole model.
        assert_eq!(app.overhang.finds, 1);
        for _ in 0..5 {
            assert_eq!(published(&mut app)[1], before);
        }
        assert_eq!((app.overhang.plate, app.overhang.finds), (kept, 1));

        // The body made 2 mm thicker all round: its bottom is 2 mm lower.
        update(&mut app, Message::Form(FormMessage::AmountChanged(2.0)));
        update(&mut app, Message::Form(FormMessage::BulkApplied));
        let after = published(&mut app)[1];
        assert!(
            (before - after - 2.0).abs() < 0.3,
            "the plate went from {before} mm to {after} under a body bulked by 2 mm"
        );
        // And back with the undo.
        update(&mut app, Message::Undo);
        assert!((published(&mut app)[1] - before).abs() < 0.05);

        // A second body, lower down: the plate is under that one. Hidden,
        // it is not printed, and the plate is under the first again.
        let mut low = brokkr_core::Volume::new(app.doc.voxel_size());
        low.seed_sphere(Vec3::new(0.0, -50.0, 0.0), 5.0);
        let low = app.doc.add_body("Low", low);
        let under_both = published(&mut app)[1];
        assert!((under_both - (-55.0 + overhang::FIRST_LAYER_MM)).abs() < 0.1, "{under_both}");
        update(&mut app, Message::BodyVisibilityToggled(low));
        assert!((published(&mut app)[1] - before).abs() < 0.05, "a hidden body held the plate");

        // The crease view's two numbers still go beside the overhang's two.
        update(&mut app, Message::ShowCavityToggled);
        app.publish_mask_view();
        let [strength, gain, limit, _] = app.shared.cavity_view().packed();
        assert!(strength > 0.0 && gain > 0.0 && limit == overhang::limit(45.0));
    }

    /// An export says how much overhangs when the view is on, and says
    /// nothing about it when the view is off.
    #[test]
    fn an_export_says_how_much_overhangs_only_when_the_view_is_on() {
        let directory = std::env::temp_dir().join(format!("brokkr-leaning-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        // (Not a path with the word in it: the status names the path.)
        let path = directory.join("ball.stl");
        let mut app = app();
        app.export(ExportFormat::Stl, &path);
        assert!(app.status.starts_with("exported"), "{}", app.status);
        assert!(!app.status.contains("overhang"), "{}", app.status);

        set(&mut app, OverhangMessage::Shown(true));
        app.export(ExportFormat::Stl, &path);
        // A ball 30 mm in radius past 45 degrees: the cap under the line
        // where it leans that far, (1 - sin 45) / 2 = 14.6% of a sphere,
        // less the 0.3 mm of it that stands on the plate, 0.5%.
        assert!(
            app.status.contains("overhangs past 45 degrees, 14% of the surface"),
            "{}",
            app.status
        );
        // Past 60: 6.7%, less the same.
        set(&mut app, OverhangMessage::Steepest(60.0));
        app.export(ExportFormat::Stl, &path);
        assert!(
            app.status.contains("overhangs past 60 degrees, 6% of the surface"),
            "{}",
            app.status
        );
        std::fs::remove_dir_all(&directory).ok();
    }
}
