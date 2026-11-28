// SPDX-License-Identifier: AGPL-3.0-only

//! What the tests of the application share: an application at the
//! viewport's size, a press and a release, a document with unsaved work, a
//! scratch directory and an imported mesh. Moved out of `app.rs`'s
//! `mod tests` (issue 189).

use super::*;
use iced::Vector;

/// `update` returns a `Task` now. Tests do not run the iced runtime, so
/// there is nothing to hand it to and dropping it is correct — but it must
/// be dropped deliberately rather than by `#[allow]`, or a test that should
/// have driven a dialog would pass silently.
pub(super) fn update(app: &mut Hani, message: Message) {
    let task = app.update(message);
    drop(task);
}

pub(super) const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

pub(super) fn centre_of_viewport() -> Vector {
    Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
}

pub(super) fn app() -> Hani {
    Hani::with_tablet(crate::tablet::Tablet::inert())
}

pub(super) fn press(app: &mut Hani, at: Vector) {
    app.on_pointer(PointerEvent::Pressed { button: PointerButton::Left, position: at, size: SIZE });
}

pub(super) fn release(app: &mut Hani) {
    app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
}

// --- the confirm-before-discard prompt -----------------------------------

/// An app with a stroke in it and nowhere to save it, which is the state
/// every one of these is about.
pub(super) fn app_with_unsaved_work() -> Hani {
    let mut app = app();
    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert!(app.unsaved, "the fixture did not actually dirty the document");
    app
}

// --- the recent list and the crash net -----------------------------------

/// A scratch directory for one test's recent list and autosave, so nothing
/// here can reach the real ones.
pub(super) fn scratch(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("hani-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// Build what a finished import delivers, with a chosen guess about which
/// way the mesh's own up pointed.
pub(super) fn imported_with(resting_up: Option<hani_core::Facing>) -> crate::message::Imported {
    let mut volume = hani_core::Volume::new(0.5);
    volume.seed_sphere(Vec3::new(0.0, 20.0, 0.0), 6.0);
    volume.mark_everything_dirty();
    crate::message::Imported {
        volume,
        report: hani_core::voxelise::VoxeliseReport::default(),
        source: std::path::PathBuf::from("nightwing.obj"),
        elapsed_ms: 0.0,
        resting_up,
    }
}
