// SPDX-License-Identifier: AGPL-3.0-only

//! Importing a mesh: what it replaces, the prompt it raises, and the
//! prompt to stand it up. Moved out of `app.rs`'s `mod tests` (issue 189).

use super::test_support::*;
use super::*;

// --- importing a mesh ----------------------------------------------------

/// Drives the import through the application rather than the voxeliser,
/// because the interesting failures are all in the wiring: a stale mesh
/// pool, history outliving its model, and above all a document that thinks
/// it belongs to the STL it came from.
#[test]
fn importing_a_mesh_replaces_the_model_without_claiming_its_file() {
    let directory = scratch("import");
    let path = directory.join("cube.stl");

    // Write a real STL through this project's own writer, so the test
    // exercises the reader rather than an assumption about the format.
    let mut source = hani_core::Volume::new(0.5);
    source.seed_sphere(Vec3::ZERO, 14.0);
    source.mark_everything_dirty();
    let (mesh, report) = source.export_mesh();
    assert!(report.is_printable(), "the fixture is not printable");
    let mut bytes = Vec::new();
    hani_core::export::stl::write(&mesh, &mut bytes).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    let mut app = app();
    app.project_path = Some(directory.join("previous.hani"));
    press(&mut app, centre_of_viewport());
    release(&mut app);

    let imported = hani_core::import::read_path(&path).expect("the STL should read");
    let options = hani_core::voxelise::VoxeliseOptions::at(app.doc.voxel_size());
    let (volume, voxel_report) =
        hani_core::voxelise::voxelise(&imported, &options).expect("it should voxelise");
    app.adopt_import(crate::message::Imported {
        volume,
        report: voxel_report,
        source: path.clone(),
        elapsed_ms: 0.0,
        resting_up: hani_core::resting_up(&imported.positions),
    });

    assert!(
        app.project_path.is_none(),
        "the import adopted the mesh's path, so the next plain Save would write a .hani \
         container straight over the user's .stl"
    );
    assert!(app.unsaved, "an imported model is unsaved work and quitting would lose it");
    assert!(!app.history.can_undo(), "history outlived the model it belonged to");
    assert!(app.perf.dirty_bricks > 0, "nothing was meshed, so the import is invisible");
    assert!(app.status.starts_with("imported"), "reported: {}", app.status);

    let (_, after) = app.doc.active_volume().export_mesh();
    assert!(after.is_printable(), "the imported model is not printable: {}", after.summary());

    std::fs::remove_dir_all(&directory).ok();
}

/// A file that cannot be read must leave the sculpt on screen alone, and
/// must say so in a way the header renders as an error.
#[test]
fn a_mesh_that_cannot_be_read_reports_and_changes_nothing() {
    let directory = scratch("import-bad");
    let path = directory.join("broken.stl");
    std::fs::write(&path, b"this is not an STL of any flavour").unwrap();

    let mut app = app();
    press(&mut app, centre_of_viewport());
    release(&mut app);
    let before = app.doc.active_volume().sample_world(Vec3::ZERO);

    let payload = crate::message::ImportPayload::new(
        hani_core::import::read_path(&path).map(|_| unreachable!("it should not parse")),
    );
    update(&mut app, Message::ImportLoaded(payload));

    assert!(
        app.status.contains("could not"),
        "the failure does not contain the substring the header colours as an error: {}",
        app.status
    );
    assert_eq!(
        app.doc.active_volume().sample_world(Vec3::ZERO),
        before,
        "a failed import changed the model"
    );
    assert!(app.history.can_undo(), "a failed import cleared history");

    std::fs::remove_dir_all(&directory).ok();
}

/// Import discards the document, so it goes through the same gate as Open.
#[test]
fn importing_with_unsaved_work_raises_the_prompt() {
    let mut app = app_with_unsaved_work();
    update(&mut app, Message::ImportRequested);
    assert_eq!(app.confirm, Some(PendingAction::Import));
}

/// **A saved multi-body document is still work, and import throws all of it
/// away.**
///
/// The gate consulted `unsaved` alone, so with a saved five-body project
/// open, File > Import replaced every body with the mesh and asked nothing:
/// no dialog, no prompt, no status line. The file survives on disk, which
/// is why this is a prompt rather than a refusal.
#[test]
fn importing_over_a_saved_document_of_several_bodies_asks_first() {
    let mut app = app();
    assert!(!app.unsaved, "the fixture must be clean or this measures the old gate");

    // One body and saved: nothing to lose that Open would not also cost, so
    // no prompt. This half is what says the guard did not simply become
    // "always ask".
    update(&mut app, Message::ImportRequested);
    assert_eq!(app.confirm, None, "a single-body import should not be interrupted");

    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 10.0);
    app.doc.add_body("Body 2", second);
    app.publish_visibility();
    assert!(!app.unsaved, "adding the fixture body must not be what raises the prompt");

    update(&mut app, Message::ImportRequested);
    assert_eq!(
        app.confirm,
        Some(PendingAction::Import),
        "importing discarded a second body without asking"
    );
}

/// The same import, with its sphere where the camera is looking.
///
/// `adopt_import` frames the ORIGIN at `MODEL_RADIUS_MM`, and
/// `imported_with`'s sphere is a 6 mm ball twenty millimetres above it: a
/// ray through the centre of the viewport misses it completely. A test
/// that presses there to prove the press was STOPPED therefore proves
/// nothing, which is exactly what
/// `a_press_behind_the_orientation_prompt_does_not_sculpt` was doing --
/// verified by removing the guard and watching it pass.
fn imported_under_the_cursor(resting_up: Option<hani_core::Facing>) -> crate::message::Imported {
    let mut volume = hani_core::Volume::new(0.5);
    volume.seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM);
    volume.mark_everything_dirty();
    crate::message::Imported { volume, ..imported_with(resting_up) }
}

#[test]
fn an_import_that_came_in_lying_down_raises_the_prompt() {
    let mut app = app();
    app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
    assert_eq!(
        app.orient_prompt,
        Some(hani_core::Facing::Back),
        "the model is on its back and nothing offered to stand it up"
    );
}

#[test]
fn an_import_that_is_already_upright_is_not_asked_about() {
    // The case that decides whether this feature is usable or a nuisance:
    // an STL exported for a slicer sits on the bed and arrives standing, so
    // the guess resolves to `Up` and must raise nothing at all. Getting
    // this wrong would put a dialog in front of nearly every print file.
    let mut app = app();
    app.adopt_import(imported_with(Some(hani_core::Facing::Up)));
    assert!(app.orient_prompt.is_none(), "an upright model was asked about");
}

#[test]
fn an_import_with_no_tell_is_not_guessed_at() {
    let mut app = app();
    app.adopt_import(imported_with(None));
    assert!(app.orient_prompt.is_none());
}

#[test]
fn accepting_the_prompt_stands_the_model_up() {
    let mut app = app();
    app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
    // The fixture's material is above the origin; a model whose own up
    // points backwards has to be turned for that to mean anything, so
    // check where it lands rather than that it merely moved.
    let turned = hani_core::AxisRotation::taking(hani_core::Facing::Back, hani_core::Facing::Up)
        .apply(Vec3::new(0.0, 20.0, 0.0));

    update(&mut app, Message::OrientPromptAnswered(true));

    assert!(app.orient_prompt.is_none(), "the prompt stayed up after being answered");
    assert!(
        app.doc.active_volume().sample_world(turned) < 0.0,
        "the model did not turn the way promised"
    );
}

#[test]
fn declining_the_prompt_leaves_the_model_exactly_as_imported() {
    let mut app = app();
    app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
    let above = Vec3::new(0.0, 20.0, 0.0);
    let before = app.doc.active_volume().sample_world(above);

    update(&mut app, Message::OrientPromptAnswered(false));

    assert!(app.orient_prompt.is_none());
    assert_eq!(
        app.doc.active_volume().sample_world(above),
        before,
        "declining still turned the model"
    );
}

#[test]
fn a_press_behind_the_orientation_prompt_does_not_sculpt() {
    // iced 0.14's `stack!` layers do not block what is underneath them, so
    // the scrim is the widget that swallows presses and this early return
    // is the guarantee behind it. Without both, a click behind the card
    // carves the model the user is being asked about.
    let mut app = app();
    app.adopt_import(imported_under_the_cursor(Some(hani_core::Facing::Back)));
    assert!(app.orient_prompt.is_some());

    let probe = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let before = app.doc.active_volume().sample_world(probe);
    let entries = app.history_stats.undo_entries;
    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(
        app.doc.active_volume().sample_world(probe),
        before,
        "a press reached the model behind it"
    );
    // The probe above is a fixed point on the ORIGINAL sphere, and an
    // imported model need not have surface there -- with the guard removed
    // by hand this assertion still passed, so it is not the one doing the
    // work. A stroke that lands anywhere at all records an undo entry.
    assert_eq!(app.history_stats.undo_entries, entries, "it recorded a stroke behind the prompt");
    assert!(app.drag.is_none());
}

#[test]
fn escape_declines_the_orientation_prompt() {
    let mut app = app();
    app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
    let above = Vec3::new(0.0, 20.0, 0.0);
    let before = app.doc.active_volume().sample_world(above);

    update(&mut app, Message::MenuClosed);

    assert!(app.orient_prompt.is_none(), "escape did not dismiss it");
    assert_eq!(app.doc.active_volume().sample_world(above), before, "escape turned the model");
}
