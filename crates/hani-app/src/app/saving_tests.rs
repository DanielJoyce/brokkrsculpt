// SPDX-License-Identifier: AGPL-3.0-only

//! Saving: the unsaved flag, the view a file keeps, the recent list, the
//! crash net and the checked save. Moved out of `app.rs`'s `mod tests`
//! (issue 189).

use super::test_support::*;
use super::*;
use hani_core::BrushKind;
use iced::Vector;
use std::time::Duration;

/// ZBrush calls this Dynamic. Without it a brush tuned on one model is the
/// wrong size the moment the model changes scale.
/// **A saved view that cannot see the model is re-framed rather than
/// restored, and the near plane comes from the body rather than from the
/// starting ball.**
///
/// This is the regression for a real file. `Hani.hani` -- a 133 mm
/// model centred on the origin -- was saved carrying `target` 435 mm away
/// and `distance` 0.3 mm, and reported as "I cannot navigate in this file:
/// not the SpaceMouse, not the scroll wheel, not the navigation cube."
/// All three at once is the tell that nothing was wrong with any of them:
/// pan, zoom and orbit all scale with `distance`, and 0.3 mm makes every
/// one of them move by nothing you can see.
///
/// 0.3 is exactly `near * 10.0` for a 30 mm ball, which is what
/// `apply_view` used to spread over every restore regardless of what was
/// in the file. So the clamp that should have stopped at 1.33 mm let the
/// camera inside the model and then pinned it there, and the next save
/// wrote that back to disk.
///
/// Both halves are asserted, because either alone leaves the file unusable:
/// the floor has to follow the body, and a poisoned view already on disk
/// has to recover.
#[test]
fn a_saved_view_that_cannot_see_the_model_is_re_framed() {
    let mut app = app();
    app.doc.active_volume_mut().seed_sphere(Vec3::ZERO, 133.0);
    app.rebuild_everything();

    let poisoned = hani_core::View {
        camera_target: Vec3::new(-435.18, -12.09, 95.65),
        camera_distance: 0.3,
        camera_yaw: 42.6,
        camera_pitch: -0.05,
        ..hani_core::View::default()
    };
    app.apply_view(&poisoned);

    let radius = app.doc.active_volume().content_radius().expect("the ball has bounds");
    // **The floor assertion changed with the unit it is measured in, and
    // the behaviour it pinned is the thing being fixed.** It used to demand
    // `near * 10.0 > 1.0` -- 1.33 mm on this model -- as proof that the
    // floor had been computed from the body rather than from the 30 mm
    // starting ball. The floor is now two voxels, so it is the same
    // closeness on every model by construction and cannot come from the
    // wrong body at all; a fraction-of-the-radius floor was itself the
    // reason detail work was impossible. What is still worth pinning is
    // that the floor is a sane multiple of the lattice and that the
    // restored distance clears it.
    assert!(
        (app.camera.min_distance() - app.doc.voxel_size() * 2.0).abs() < 1.0e-6,
        "the zoom floor is {} on a {} mm lattice",
        app.camera.min_distance(),
        app.doc.voxel_size()
    );
    assert!(
        app.camera.target.distance(Vec3::ZERO) <= radius,
        "the camera is still aimed {} mm from a model of radius {radius}",
        app.camera.target.distance(Vec3::ZERO)
    );
    assert!(
        app.camera.distance >= app.camera.min_distance(),
        "the restored distance {} is below its own floor {}",
        app.camera.distance,
        app.camera.min_distance()
    );
    assert!(
        (app.camera.yaw - 42.6).abs() < 1.0e-3,
        "re-framing threw away the angles the user had set"
    );

    // And the recovery is visible rather than silent.
    assert!(app.status.contains("re-framed"), "the view moved and nothing said so: {}", app.status);

    // The whole point: a scroll now moves the camera by something you can see.
    let before = app.camera.distance;
    app.camera.zoom_by(OrbitCamera::zoom_factor(1.0));
    assert!(
        (app.camera.distance - before).abs() > 1.0,
        "one notch of scroll moved {} mm",
        (app.camera.distance - before).abs()
    );
}

/// The whole point of the format: sculpt, save, reopen, and get the same
/// thing back. Driven through the application's own open and save rather
/// than through the format directly, because the interesting failures are
/// in the wiring -- a stale mesh pool, history that outlives its model, a
/// camera that does not follow.
#[test]
fn a_sculpt_survives_a_save_and_a_reopen() {
    let directory = std::env::temp_dir().join(format!("hani-roundtrip-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("sculpt.hani");

    let mut app = app();
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    let probe = app
        .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
        .expect("the centre should hit the sphere");

    // Make something worth keeping, and move the camera and settings so the
    // session state has something to prove.
    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    for _ in 0..4 {
        press(&mut app, centre_of_viewport());
        release(&mut app);
    }
    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::Y));
    update(&mut app, Message::BrushRadiusChanged(5.5));
    app.camera.yaw = 1.25;
    let sculpted = app.doc.active_volume().sample_world(probe);
    let expected_bricks: usize = app.doc.active_volume().brick_coords().count();

    app.save_project(&path);
    assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
    assert_eq!(app.project_path.as_deref(), Some(path.as_path()));

    // A different session entirely, as reopening after quitting would be.
    let mut reopened = Hani::with_tablet(crate::tablet::Tablet::inert());
    reopened.open_project(&path);
    assert!(reopened.status.starts_with("opened"), "open reported: {}", reopened.status);

    assert_eq!(
        reopened.doc.active_volume().sample_world(probe),
        sculpted,
        "the field came back different"
    );
    assert_eq!(reopened.doc.active_volume().brick_coords().count(), expected_bricks);
    assert!((reopened.camera.yaw - 1.25).abs() < 1.0e-5, "the camera did not follow");
    assert!((reopened.brush.radius - 5.5).abs() < 1.0e-5, "the brush did not follow");
    assert!(reopened.symmetry_settings().axis(MirrorAxis::Y), "the mirror planes did not follow");

    // And it was drawn and is printable, which is what "loaded" means.
    // Note `open_project` meshes immediately, so the dirty set is already
    // drained by now -- what proves it happened is how much it meshed.
    assert!(
        reopened.perf.dirty_bricks > 0,
        "the reopened model was never meshed, so it would load into an empty screen"
    );
    let (_, report) = reopened.doc.active_volume().export_mesh();
    assert!(report.is_printable(), "the reopened model is not printable: {}", report.summary());

    // History belongs to the model that went away.
    assert!(!reopened.history.can_undo(), "undo outlived the model it belonged to");

    std::fs::remove_dir_all(&directory).ok();
}

// --- the unsaved marker --------------------------------------------------
//
// Quitting with unsaved work used to lose it silently. These pin the flag
// that the confirm prompt reads, and in particular the three places it must
// NOT be set, since a flag that is always on is the same as no flag at all.

#[test]
fn a_stroke_marks_the_document_unsaved_and_a_save_clears_it() {
    let directory = std::env::temp_dir().join(format!("hani-unsaved-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("sculpt.hani");

    let mut app = app();
    assert!(!app.unsaved, "a freshly opened sphere is not unsaved work");

    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert!(app.unsaved, "a stroke did not mark the document unsaved");

    app.save_project(&path);
    assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
    assert!(!app.unsaved, "a successful save did not clear the marker");

    std::fs::remove_dir_all(&directory).ok();
}

/// A press and release that never met the model produces no undo entry, so
/// it must produce no prompt either. Without the guard inside
/// `finish_stroke` every stray click in empty space would arm the marker.
#[test]
fn a_press_that_misses_the_model_does_not_mark_it_unsaved() {
    let mut app = app();
    press(&mut app, Vector::new(4.0, 4.0));
    release(&mut app);
    assert!(
        !app.unsaved,
        "a click into empty space armed the unsaved marker, so every stray \
         click would raise a discard prompt"
    );
    assert!(
        !app.history.can_undo(),
        "the corner of the viewport hit the model after all, so this test \
         proves nothing -- move the press"
    );

    // The control: the same gesture at the centre does arm it, so the
    // assertion above is about the miss and not about a press path that
    // silently does nothing.
    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert!(app.unsaved, "the control press missed too, so the test is vacuous");
}

/// A write that failed did not save anything, so the marker has to survive
/// it. Clearing it in the wrong place lets the next quit go through with no
/// prompt and no file.
#[test]
fn a_failed_save_leaves_it_unsaved() {
    let mut app = app();
    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert!(app.unsaved);

    let nowhere = std::env::temp_dir()
        .join(format!("hani-absent-{}", std::process::id()))
        .join("no")
        .join("such")
        .join("directory")
        .join("sculpt.hani");
    app.save_project(&nowhere);

    assert!(app.status.contains("could not"), "a failed save reported: {}", app.status);
    assert!(app.unsaved, "a failed save cleared the unsaved marker");
    assert!(app.project_path.is_none(), "a failed save claimed the file");
}

/// The puck's buttons call `undo`/`redo` directly rather than sending
/// `Message::Undo`, so the marker has to be set in the methods. Calling them
/// the way `drive_from_spacemouse` does is what proves that.
#[test]
fn undo_and_redo_mark_it_unsaved_through_the_direct_call_path() {
    let mut app = app();
    press(&mut app, centre_of_viewport());
    release(&mut app);
    app.unsaved = false;

    app.undo();
    assert!(app.unsaved, "undo did not mark the document unsaved");

    app.unsaved = false;
    app.redo();
    assert!(app.unsaved, "redo did not mark the document unsaved");
}

/// Undo with nothing to undo changes nothing, so it must not arm the marker
/// either -- the flag lives inside the `history.undo` guard for this.
#[test]
fn undo_with_an_empty_history_does_not_mark_it_unsaved() {
    let mut app = app();
    assert!(!app.history.can_undo(), "the fixture already had something to undo");
    app.undo();
    assert!(!app.unsaved, "an undo that did nothing armed the unsaved marker");

    // The control. Without it this test asserts the default value and would
    // pass just as happily if `undo` did nothing at all, in either case.
    press(&mut app, centre_of_viewport());
    release(&mut app);
    app.unsaved = false;
    app.undo();
    assert!(
        app.unsaved,
        "an undo with something to undo failed to mark it, so the \
                          assertion above was vacuous"
    );
}

/// `resample` returns early when the size is already in use. The marker sits
/// past that return, so the no-op stays a no-op.
#[test]
fn resampling_to_the_current_size_leaves_the_flag_alone() {
    let mut app = app();
    app.resample(app.doc.voxel_size());
    assert!(!app.unsaved, "a resample that did nothing armed the unsaved marker");

    app.resample(app.doc.voxel_size() * 2.0);
    assert!(app.unsaved, "a real resample did not mark the document unsaved");
}

#[test]
fn the_title_carries_the_file_name_and_the_unsaved_star() {
    let mut app = app();
    assert_eq!(app.title(), "untitled — HaniSculpt");

    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert_eq!(app.title(), "untitled* — HaniSculpt");

    app.project_path = Some(std::path::PathBuf::from("/tmp/dragon.hani"));
    assert_eq!(app.title(), "dragon.hani* — HaniSculpt");

    app.unsaved = false;
    assert_eq!(app.title(), "dragon.hani — HaniSculpt");
}

#[test]
fn saving_and_opening_both_record_into_the_recent_list() {
    let directory = scratch("recent-record");
    let path = directory.join("sculpt.hani");

    let mut app = app_with_unsaved_work();
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
    app.save_project(&path);
    assert_eq!(app.recent.paths().len(), 1, "a save did not record the file");

    let mut reopened = Hani::with_tablet(crate::tablet::Tablet::inert());
    reopened.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
    reopened.open_project(&path);
    assert!(reopened.status.starts_with("opened"), "open reported: {}", reopened.status);
    assert_eq!(reopened.recent.paths()[0], std::path::absolute(&path).unwrap());

    std::fs::remove_dir_all(&directory).ok();
}

/// A file that has been moved or deleted should stop being offered, rather
/// than sitting in the menu failing every time it is clicked.
#[test]
fn opening_a_missing_recent_file_drops_it_from_the_list() {
    let directory = scratch("recent-missing");
    let gone = directory.join("gone.hani");

    let mut app = app();
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
    app.recent.record(&gone);
    assert_eq!(app.recent.paths().len(), 1);

    app.open_project(&gone);
    assert!(app.status.contains("could not"), "reported: {}", app.status);
    assert!(app.recent.is_empty(), "a file that cannot be opened stayed in the list");

    std::fs::remove_dir_all(&directory).ok();
}

/// A crash net this build cannot read is kept, not overwritten.
///
/// **This is a data-loss guard, and the loss is silent.** The updater
/// supports downgrade, so an older build can meet a newer build's autosave;
/// it refuses to open it, says so, and then the 120-second timer replaces
/// the only copy of the user's work with whatever is on screen.
///
/// The unreadable file is manufactured by bumping the FIELD version in the
/// header of a real autosave, which is exactly the shape a downgrade
/// produces -- rather than by writing rubbish, which would prove only that
/// rubbish is refused.
#[test]
fn an_autosave_this_build_cannot_read_is_kept_rather_than_overwritten() {
    let directory = scratch("autosave-unreadable");
    let path = directory.join("autosave.hani");

    // A genuine autosave first, so the bytes are real in every other way.
    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(path.clone());
    app.write_autosave();
    let good = std::fs::read(&path).expect("the autosave should be there");

    // Bytes 10..12 are the field version. One past what this build can READ
    // is a file from a newer build -- and that is NOT one past what it just
    // wrote: an unpainted, unmasked document deliberately stamps the OLDEST
    // encoding that can carry it, which is the whole backward-compatibility
    // mechanism. So the boundary is found rather than assumed, which also
    // stops this test rotting at the next field bump.
    let mut from_the_future = good.clone();
    let mut version = u16::from_le_bytes([good[10], good[11]]);
    loop {
        version += 1;
        assert!(version < 64, "no field version in range was refused");
        from_the_future[10..12].copy_from_slice(&version.to_le_bytes());
        if hani_core::project::read_outline(&mut from_the_future.as_slice()).is_err() {
            break;
        }
    }
    std::fs::write(&path, &from_the_future).expect("write failed");

    let mut older = Hani::with_tablet(crate::tablet::Tablet::inert());
    older.autosave_file = Some(path.clone());
    let aside = older.preserve_unreadable_autosave().expect("it should have been kept");

    assert!(!path.exists(), "the unreadable autosave was left where the timer would clobber it");
    assert!(aside.exists(), "it was moved somewhere that does not exist");
    assert_eq!(
        std::fs::read(&aside).expect("read failed"),
        from_the_future,
        "the kept file is not the bytes that were there"
    );
    assert_eq!(aside.file_name().unwrap(), "autosave-unreadable-1.hani");

    // A second one does not overwrite the first.
    std::fs::write(&path, &from_the_future).expect("write failed");
    let second = older.preserve_unreadable_autosave().expect("the second should be kept too");
    assert_eq!(second.file_name().unwrap(), "autosave-unreadable-2.hani");
    assert!(aside.exists(), "the second refusal destroyed the first one");

    // And a file this build CAN read is left exactly where it is.
    std::fs::write(&path, &good).expect("write failed");
    assert_eq!(older.preserve_unreadable_autosave(), None, "a good autosave was moved aside");
    assert!(path.exists(), "a readable autosave was taken away from File > Recover");

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn the_autosave_round_trips_but_is_not_a_save() {
    let directory = scratch("autosave");
    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(directory.join("autosave.hani"));
    let probe = Vec3::ZERO;
    let sculpted = app.doc.active_volume().sample_world(probe);

    app.write_autosave();

    assert!(app.has_autosave(), "nothing was written");
    assert!(app.unsaved, "the autosave cleared the unsaved marker, so quitting would not ask");
    assert!(app.project_path.is_none(), "the autosave claimed to be the document's file");

    let mut recovered = Hani::with_tablet(crate::tablet::Tablet::inert());
    recovered.autosave_file = app.autosave_file.clone();
    recovered.recover_autosave();

    assert_eq!(
        recovered.doc.active_volume().sample_world(probe),
        sculpted,
        "the field came back different"
    );
    assert!(
        recovered.project_path.is_none(),
        "a recovered autosave adopted its own path, so the next Save would write back into \
         the crash net instead of asking for a real file"
    );
    assert!(recovered.unsaved, "a recovered autosave must still want saving somewhere real");
    assert!(recovered.recent.is_empty(), "the crash net was listed as a recent document");

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn an_explicit_save_throws_the_crash_net_away() {
    let directory = scratch("autosave-cleared");
    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(directory.join("autosave.hani"));
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
    app.write_autosave();
    assert!(app.has_autosave());

    app.save_project(&directory.join("sculpt.hani"));
    assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
    assert!(
        !app.has_autosave(),
        "a stale crash net survived a real save, so File > Recover would offer to restore \
         something older than the file on disk"
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// **A failed save must leave the file it was replacing exactly as it
/// was.** Until this increment the save opened the user's real file with
/// `File::create`, which truncates it to zero before a byte of the new
/// document is written -- so a failure part way through left the work in
/// neither place.
///
/// The failure is forced by putting a DIRECTORY where the temporary file
/// wants to be, which is the cheapest way to make `File::create` fail
/// without depending on permissions, a full disk or a mocked filesystem.
#[test]
fn a_save_that_fails_leaves_the_previous_file_and_the_crash_net_alone() {
    let directory = scratch("save-atomic");
    let path = directory.join("sculpt.hani");
    std::fs::write(&path, b"the previous save, which must survive").unwrap();

    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(directory.join("autosave.hani"));
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
    app.write_autosave();
    assert!(app.has_autosave(), "the fixture needs a crash net to protect");

    // Where `save_project` will try to put its temporary.
    std::fs::create_dir_all(path.with_extension("hani.tmp")).unwrap();

    app.save_project(&path);

    assert!(app.status.contains("could not"), "a failed save reported: {}", app.status);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"the previous save, which must survive",
        "a failed save destroyed the file it was replacing"
    );
    assert!(app.unsaved, "a failed save cleared the unsaved marker");
    assert!(app.project_path.is_none(), "a failed save adopted the path anyway");
    assert!(app.has_autosave(), "a failed save deleted the crash net");

    std::fs::remove_dir_all(&directory).ok();
}

/// A save that works leaves the file and nothing else -- no temporary
/// beside it for the user to find and wonder about.
#[test]
fn a_save_that_works_leaves_no_temporary_behind() {
    let directory = scratch("save-tidy");
    let path = directory.join("sculpt.hani");
    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(directory.join("autosave.hani"));
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));

    app.save_project(&path);

    assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
    assert!(path.is_file(), "the file was not written");
    assert!(
        !path.with_extension("hani.tmp").exists(),
        "the temporary the save writes through was left on disk"
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// The save reads its own output back before it deletes the crash net.
///
/// `clear_autosave` used to run on the writer's word alone: a write that
/// reported success and produced a file this build will not open took the
/// user's work, cleared the unsaved marker and deleted the one copy that
/// could have recovered it. The check is the header and the node table
/// only, which is a few hundred bytes whatever the sculpt weighs.
///
/// Driven here by changing the document *after* the file is written, which
/// is the same disagreement a partial write would produce and the only one
/// a test can stage without a mocked filesystem.
#[test]
fn the_save_verification_notices_a_file_that_is_not_this_document() {
    let directory = scratch("save-verify");
    let path = directory.join("sculpt.hani");
    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(directory.join("autosave.hani"));
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));

    app.save_project(&path);
    assert!(app.status.starts_with("saved"), "save reported: {}", app.status);
    assert!(app.verify_written(&path).is_ok(), "a file just written failed its own check");

    // One more body than the file on disk knows about.
    app.doc.add_body("Body 2", hani_core::Volume::new(app.doc.voxel_size()));
    let problem = app.verify_written(&path).expect_err("a stale file passed the check");
    assert!(problem.contains("2"), "the mismatch did not say what it found: {problem}");

    std::fs::remove_dir_all(&directory).ok();
}

/// **`save_project` really returns on that verification** -- the test above
/// proves the check works, not that anything calls it.
///
/// That distinction was measured, not assumed: deleting the whole
/// `if let Err(problem) = self.verify_written(&temporary)` gate from
/// `save_project` left all 279 tests in this crate green. A repair that is
/// real, correct, unit-tested and never called is a shape this project has
/// shipped before, and the gate is the one standing between an unreadable
/// file and `clear_autosave`.
///
/// Driving it needs a temporary that swallows every write and hands back
/// something else on the read, and `/dev/null` is exactly that: the save
/// opens the symlink, `project::write` reports success at every step, and
/// the read-back finds an empty file. Nothing is written through the link
/// that survives, and the cleanup unlinks the LINK -- `remove_file` does
/// not follow one -- so `/dev/null` itself is never touched.
///
/// Rejected first: making the document disagree with the file, the trick
/// the test above uses. It cannot work here, because `save_project` writes
/// and verifies against the same `self.doc` within one call, so the two can
/// never disagree without an injection point that does not exist.
#[test]
// The trick IS the symlink: a link to `/dev/null` is what makes the write
// succeed and the read-back come up empty. Windows has neither piece, and
// the behaviour under test -- keep the previous file when a save cannot be
// read back -- is platform-independent, so it is covered here and taken on
// trust there rather than faked with a second, weaker fixture.
#[cfg(unix)]
fn a_save_that_cannot_read_its_own_output_back_keeps_the_previous_file() {
    let directory = scratch("save-unreadable");
    let path = directory.join("sculpt.hani");
    std::fs::write(&path, b"the previous save, which must survive").unwrap();

    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(directory.join("autosave.hani"));
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
    app.write_autosave();
    assert!(app.has_autosave(), "the fixture needs a crash net to protect");

    // Where `save_project` will put its temporary, pointed somewhere that
    // accepts the write and gives nothing back.
    let temporary = path.with_extension("hani.tmp");
    std::os::unix::fs::symlink("/dev/null", &temporary).unwrap();

    app.save_project(&path);

    assert!(
        app.status.contains("could not write"),
        "a file that read back empty reported: {}",
        app.status
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"the previous save, which must survive",
        "an unverifiable save replaced the file it could not prove it had written"
    );
    assert!(app.unsaved, "an unverifiable save cleared the unsaved marker");
    assert!(app.project_path.is_none(), "an unverifiable save adopted the path anyway");
    assert!(
        app.has_autosave(),
        "an unverifiable save deleted the crash net, which is the whole failure the \
         verification exists to stop"
    );
    assert!(
        std::fs::symlink_metadata(&temporary).is_err(),
        "the temporary was left on disk after the verification refused it"
    );
    assert!(
        std::path::Path::new("/dev/null").exists(),
        "the cleanup followed the symlink instead of unlinking it"
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// The last failure arm, and the only one reached with a good file in hand:
/// the write worked, the read-back agreed, and the rename still failed.
///
/// It is here for two reasons. It is the one arm that runs *after* the
/// verification, so reaching it at all proves execution got past that gate
/// rather than around it. And it is the arm where the temporary holds the
/// user's only new copy, so leaving it behind, or clearing `unsaved`
/// because the bytes did reach a disk somewhere, would both be wrong.
///
/// The failure is forced by putting a DIRECTORY at the TARGET, which makes
/// `rename` fail with `EISDIR` while leaving every step before it -- create,
/// write, verify -- entirely successful.
#[test]
fn a_save_that_cannot_replace_the_target_keeps_the_crash_net() {
    let directory = scratch("save-rename");
    let path = directory.join("sculpt.hani");
    std::fs::create_dir_all(&path).unwrap();

    let mut app = app_with_unsaved_work();
    app.autosave_file = Some(directory.join("autosave.hani"));
    app.recent = crate::recent::Recent::load_from(Some(directory.join("recent")));
    app.write_autosave();
    assert!(app.has_autosave(), "the fixture needs a crash net to protect");

    app.save_project(&path);

    assert!(app.status.contains("could not replace"), "a failed rename reported: {}", app.status);
    assert!(app.unsaved, "a failed rename cleared the unsaved marker");
    assert!(app.project_path.is_none(), "a failed rename adopted the path anyway");
    assert!(app.has_autosave(), "a failed rename deleted the crash net");
    assert!(
        !path.with_extension("hani.tmp").exists(),
        "a failed rename left the temporary behind, where the user would find a file \
         holding work the application says is unsaved"
    );

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn the_autosave_tick_waits_for_the_interval_and_for_the_stroke_to_end() {
    let directory = scratch("autosave-tick");
    let mut app = app();
    app.autosave_file = Some(directory.join("autosave.hani"));

    // Clean document: nothing to protect, so nothing is written however
    // long it has been.
    app.last_autosave = Instant::now() - Duration::from_secs(600);
    app.maybe_autosave();
    assert!(!app.has_autosave(), "a clean document was autosaved");

    // Dirty, but the interval has not elapsed.
    press(&mut app, centre_of_viewport());
    release(&mut app);
    app.last_autosave = Instant::now();
    app.maybe_autosave();
    assert!(!app.has_autosave(), "it autosaved before the interval was up");

    // Dirty and overdue, but mid-drag: a write here reads as a stutter in
    // the brush.
    app.last_autosave = Instant::now() - Duration::from_secs(600);
    press(&mut app, centre_of_viewport());
    app.maybe_autosave();
    assert!(!app.has_autosave(), "it autosaved in the middle of a stroke");

    // Released, dirty and overdue -- but the pointer only just stopped, so
    // the write still waits for the pause.
    release(&mut app);
    app.maybe_autosave();
    assert!(
        !app.has_autosave(),
        "it autosaved the instant the pointer stopped, which is where a 400 ms write \
         lands as a visible hitch"
    );

    // Once the pointer has been still long enough, it writes: a piece a
    // tick, on another thread. See `app/autosave.rs`.
    app.last_activity = Instant::now() - Duration::from_secs(60);
    app.maybe_autosave();
    while app.autosave.in_flight() {
        app.maybe_autosave();
    }
    app.autosave.settle();
    assert!(app.has_autosave(), "an overdue autosave never happened after a pause");

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn a_bad_file_reports_and_changes_nothing() {
    let directory = std::env::temp_dir().join(format!("hani-bad-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("not-a-sculpt.hani");
    std::fs::write(&path, b"this is not a sculpt").unwrap();

    let mut app = app();
    let before = app.doc.active_volume().brick_coords().count();
    app.open_project(&path);

    assert!(app.status.contains("not a HaniSculpt file"), "reported: {}", app.status);
    assert_eq!(
        app.doc.active_volume().brick_coords().count(),
        before,
        "a failed open lost the model"
    );
    assert!(app.project_path.is_none(), "a failed open claimed the file");

    app.open_project(&directory.join("does-not-exist.hani"));
    assert!(app.status.contains("could not open"), "reported: {}", app.status);
    assert_eq!(app.doc.active_volume().brick_coords().count(), before);

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn resetting_discards_history_that_refers_to_the_old_model() {
    // Undoing into a volume the entry was not recorded against would splice
    // pieces of the discarded model back in.
    let mut app = app();
    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert!(app.history.can_undo());

    // Reset now goes through the unsaved-work prompt, because it discards
    // the document. Answering Discard is what the button used to do on its
    // own; the history assertions below are unchanged.
    update(&mut app, Message::ResetSphere);
    update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Discard));

    assert!(!app.history.can_undo(), "reset must clear history");
    assert!(!app.history.can_redo());
}
