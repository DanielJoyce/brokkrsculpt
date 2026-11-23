// SPDX-License-Identifier: AGPL-3.0-only

//! The fit test: a small STL to print, to find the clearance a printer
//! needs for the parting keys.
//!
//! [`brokkr_core::part::coupon`] makes it and says why it can be trusted.
//! This asks where to put the file, writes it, and says what to do with
//! the print. Nothing in the document changes: the pieces are made beside
//! it and go straight to the file.
//!
//! The test is of the key as it is set in the panel (its kind, its radius
//! and its depth) at the document's own voxel size, because that is the
//! lattice the real keys will be cut on.

use brokkr_core::part::coupon::{CLEARANCES_MM, coupon};
use iced::Task;

use super::Brokkr;
use crate::message::Message;

/// Ask where the fit test goes.
async fn pick_fit_test_target() -> Option<std::path::PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Save a fit test to print")
        .add_filter("STL", &["stl"])
        .set_file_name("fit-test.stl")
        .save_file()
        .await
        .map(|handle| {
            let path = handle.path().to_path_buf();
            if path.extension().is_some() { path } else { path.with_extension("stl") }
        })
}

impl Brokkr {
    /// The first half: ask for a path.
    pub(super) fn fit_test_requested(&self) -> Task<Message> {
        Task::perform(pick_fit_test_target(), Message::FitTestChosen)
    }

    /// The second half: make the test for the key as it is set, and write
    /// it to `path` as an STL.
    pub(super) fn write_fit_test(&mut self, path: &std::path::Path) {
        let made = match coupon(self.key_spec, self.doc.voxel_size()) {
            Ok(made) => made,
            Err(why) => {
                self.status = format!("no fit test: {why}");
                return;
            }
        };
        let (plate, plate_report) = made.plate.export_mesh();
        let peg = made.peg.as_ref().map(|peg| peg.export_mesh());
        // The same verdict an export of the model is held to: nothing is
        // written that would not print.
        let sound = plate_report.is_printable()
            && peg.as_ref().is_none_or(|(_, report)| report.is_printable());
        if !sound {
            self.status = "no fit test: the pieces did not close".to_string();
            log::error!("a fit test for {:?} did not close", self.key_spec);
            return;
        }
        let mut parts: Vec<(&str, &brokkr_core::ExportMesh)> = vec![("Fit test plate", &plate)];
        if let Some((mesh, _)) = &peg {
            parts.push(("Fit test peg", mesh));
        }
        let write = || -> std::io::Result<()> {
            let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
            brokkr_core::export::stl::write_all(&parts, &mut file)?;
            std::io::Write::flush(&mut file)
        };
        self.status = match write() {
            Ok(()) => format!(
                "fit test written to {}: {} {} from {:.2} to {:.2} mm, tightest at the end with \
                 the corner cut off. Print it, and set Clearance to the one that fits",
                path.display(),
                CLEARANCES_MM.len(),
                match self.key_spec.kind {
                    brokkr_core::part::KeyKind::Peg => "sockets and a peg,",
                    brokkr_core::part::KeyKind::PinHole => "pin holes,",
                    brokkr_core::part::KeyKind::MagnetPocket => "magnet pockets,",
                },
                CLEARANCES_MM[0],
                CLEARANCES_MM[CLEARANCES_MM.len() - 1],
            ),
            Err(error) => format!("could not write {}: {error}", path.display()),
        };
        log::info!("{}", self.status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::KeySetting;
    use brokkr_core::part::KeyKind;

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Brokkr {
        Brokkr::with_tablet(crate::tablet::Tablet::inert())
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("brokkr-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    /// How many triangles a binary STL says it holds, and whether it is as
    /// long as that many triangles are.
    fn triangles(path: &std::path::Path) -> usize {
        let bytes = std::fs::read(path).expect("the fit test should be there");
        let count = u32::from_le_bytes(bytes[80..84].try_into().expect("four bytes")) as usize;
        assert_eq!(bytes.len(), 84 + 50 * count, "the STL is not the length it says");
        count
    }

    /// The fit test is written for the key as it is set, changes nothing
    /// in the document, and says what to do with it. A pin hole's has no
    /// peg in it, so it is a smaller file.
    #[test]
    fn a_fit_test_is_written_for_the_key_as_set_and_the_document_is_left_alone() {
        let directory = scratch("fit");
        let path = directory.join("fit-test.stl");
        let mut app = app();
        let bodies = app.doc.body_count();
        let history = app.history.stats();

        update(&mut app, Message::FitTestChosen(Some(path.clone())));
        assert!(app.status.starts_with("fit test written to"), "{}", app.status);
        assert!(app.status.contains("6 sockets and a peg, from 0.05 to 0.40 mm"), "{}", app.status);
        let with_a_peg = triangles(&path);
        assert!(with_a_peg > 5_000, "only {with_a_peg} triangles");
        // Both pieces are in the file: it is as many triangles as the two
        // of them, made again here, are.
        let again = coupon(app.key_spec, app.doc.voxel_size()).expect("the default key");
        let plate = again.plate.export_mesh().0.triangles.len();
        let peg = again.peg.expect("a peg key has a peg").export_mesh().0.triangles.len();
        assert_eq!(with_a_peg, plate + peg, "a plate of {plate} and a peg of {peg}");
        assert_eq!((app.doc.body_count(), app.history.stats()), (bodies, history));
        assert!(!app.unsaved, "writing a fit test dirtied the document");

        update(&mut app, Message::KeySettingChanged(KeySetting::Kind(KeyKind::PinHole)));
        update(&mut app, Message::FitTestChosen(Some(path.clone())));
        assert!(app.status.contains("6 pin holes, from"), "{}", app.status);
        let holes_only = triangles(&path);
        assert!(holes_only < with_a_peg, "{holes_only} triangles against {with_a_peg}");

        // A bigger key is a bigger test: the setting is the one used.
        update(&mut app, Message::KeySettingChanged(KeySetting::Radius(4.0)));
        update(&mut app, Message::FitTestChosen(Some(path.clone())));
        assert!(triangles(&path) > holes_only * 2, "{} against {holes_only}", triangles(&path));

        // The dialog dismissed is nothing done.
        std::fs::remove_file(&path).expect("removing the fit test");
        let said = app.status.clone();
        update(&mut app, Message::FitTestChosen(None));
        assert!(!path.exists());
        assert_eq!(app.status, said);

        // Somewhere it cannot be written is said, and is not a panic.
        update(&mut app, Message::FitTestChosen(Some(directory.join("no/such/place.stl"))));
        assert!(app.status.starts_with("could not write"), "{}", app.status);
        std::fs::remove_dir_all(&directory).ok();
    }

    /// A key too narrow for the document's lattice gets no fit test, and
    /// is told the voxel size that would give it one.
    #[test]
    fn a_key_too_narrow_for_the_lattice_gets_no_fit_test_and_is_told_why() {
        let directory = scratch("narrow");
        let path = directory.join("fit-test.stl");
        let mut app = app();
        // The narrowest the panel allows, on a lattice too coarse for it.
        app.key_spec.radius_mm = 0.5;
        update(&mut app, Message::Resample(0.5));
        assert_eq!(app.doc.voxel_size(), 0.5);
        update(&mut app, Message::FitTestChosen(Some(path.clone())));
        assert!(app.status.starts_with("no fit test: that key is too narrow"), "{}", app.status);
        assert!(app.status.contains("0.33 mm"), "{}", app.status);
        assert!(!path.exists(), "a fit test was written anyway");
        std::fs::remove_dir_all(&directory).ok();
    }
}
