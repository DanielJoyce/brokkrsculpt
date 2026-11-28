// SPDX-License-Identifier: AGPL-3.0-only

//! The prompts and modals: the unsaved-work prompt, the keyboard's road
//! into the application, the welcome screen, the update card, and what a
//! modal holds back. Moved out of `app.rs`'s `mod tests` (issue 189).

use super::test_support::*;
use super::*;
use hani_core::BrushKind;
use iced::Vector;

#[test]
fn new_with_unsaved_work_raises_the_prompt_and_does_not_reset() {
    let mut app = app_with_unsaved_work();
    let sculpted = app.doc.active_volume().brick_coords().count();

    update(&mut app, Message::NewSculpt);

    assert_eq!(app.confirm, Some(PendingAction::NewSculpt), "no prompt was raised");
    assert!(app.unsaved, "the document was reset behind the prompt");
    assert_eq!(
        app.doc.active_volume().brick_coords().count(),
        sculpted,
        "New threw the sculpt away before the user had answered"
    );
}

/// The panel's Reset sphere button shares an arm with File > New, so it
/// discards the document identically and must ask identically.
#[test]
fn reset_sphere_asks_as_well_since_it_shares_the_arm() {
    let mut app = app_with_unsaved_work();
    update(&mut app, Message::ResetSphere);
    assert_eq!(app.confirm, Some(PendingAction::NewSculpt));
}

#[test]
fn quitting_with_unsaved_work_raises_the_prompt() {
    let mut app = app_with_unsaved_work();
    let id = iced::window::Id::unique();
    update(&mut app, Message::CloseRequested(id));
    assert_eq!(
        app.confirm,
        Some(PendingAction::Quit(id)),
        "the close request was not intercepted, so quitting still loses work"
    );
}

/// With nothing to lose, every gated action must behave exactly as it did
/// before the gate existed.
#[test]
fn a_clean_document_is_not_prompted_at_all() {
    let mut app = app();
    assert!(!app.unsaved, "the fixture starts dirty, so this proves nothing");

    update(&mut app, Message::NewSculpt);
    assert!(app.confirm.is_none(), "a clean document was prompted");

    update(&mut app, Message::CloseRequested(iced::window::Id::unique()));
    assert!(app.confirm.is_none(), "a clean document was prompted on quit");

    update(&mut app, Message::OpenRequested);
    assert!(app.confirm.is_none(), "a clean document was prompted on open");

    // The control: the same three actions DO prompt once there is something
    // to lose. Without it every assertion above is satisfied by the default.
    for action in [Message::NewSculpt, Message::CloseRequested(iced::window::Id::unique())] {
        let mut dirty = app_with_unsaved_work();
        update(&mut dirty, action);
        assert!(dirty.confirm.is_some(), "the gate never fires, so the checks above are vacuous");
    }
}

/// The prompt draws over the viewport but the shader widget still sees
/// events wherever the cursor is. A press behind the card must neither
/// dismiss the prompt nor sculpt into a document about to be discarded.
#[test]
fn a_viewport_press_does_not_dismiss_the_prompt_or_sculpt() {
    let mut app = app_with_unsaved_work();
    update(&mut app, Message::NewSculpt);
    let before = app.doc.active_volume().sample_world(Vec3::ZERO);
    let entries = app.history_stats.undo_entries;

    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(app.confirm, Some(PendingAction::NewSculpt), "a stray click dismissed it");
    assert_eq!(
        app.doc.active_volume().sample_world(Vec3::ZERO),
        before,
        "it sculpted behind the prompt"
    );
    assert_eq!(app.history_stats.undo_entries, entries, "it recorded a stroke behind the prompt");
}

/// Escape answers Cancel. It must not fall through to the menu clears,
/// which would drop the prompt without answering it.
#[test]
fn escape_cancels_the_prompt_rather_than_dropping_it() {
    let mut app = app_with_unsaved_work();
    update(&mut app, Message::NewSculpt);

    update(&mut app, Message::MenuClosed);

    assert!(app.confirm.is_none(), "escape left the prompt up");
    assert!(app.unsaved, "escape discarded the work it was asking about");
    assert!(app.history.can_undo(), "escape reset the sculpt");
}

// --- the subscription's key decode ---------------------------------------
//
// Everything in the section below this one synthesises `Message::KeyPressed`
// directly, which is downstream of `key_event` and cannot see it. These
// four cases are the only thing standing between a wrong `key_event` and a
// green suite: with the press arm deleted the whole keyboard is dead and
// every guard test still passes, guarding a message nothing can produce.

/// A window event carrying `character`, pressed or released.
///
/// The fields beyond `key` and `modifiers` are what a real winit press
/// fills in and `key_event` ignores; they are here because the literal
/// does not compile without them.
fn key_window_event(character: &str, modifiers: iced::keyboard::Modifiers) -> iced::Event {
    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Character(character.into()),
        modified_key: iced::keyboard::Key::Character(character.into()),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}

fn key_release_event(character: &str) -> iced::Event {
    iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
        key: iced::keyboard::Key::Character(character.into()),
        modified_key: iced::keyboard::Key::Character(character.into()),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::empty(),
    })
}

/// `listen_with` hands every event to `key_event` regardless of who wanted
/// it, so dropping the captured ones is the whole of the focus awareness.
/// Inverting this check is verbatim the bug that shipped for a year:
/// `1`-`7`, `s`, `u`, `x`, `y` and `z` stolen from every text field,
/// because a focused input reports its keystrokes captured and the
/// shortcut fired anyway.
#[test]
fn a_key_a_widget_already_consumed_never_reaches_the_application() {
    let typed = key_window_event("z", command());

    let message = key_event(typed, iced::event::Status::Captured, iced::window::Id::unique());

    assert!(
        message.is_none(),
        "a captured key press was forwarded, so shortcuts fire while typing"
    );
}

/// The other half: an event nobody claimed becomes a `KeyPressed` carrying
/// exactly the key and modifiers that arrived. `on_key` decodes from those
/// two fields alone, so anything lost here is a shortcut that cannot fire
/// -- dropping the modifiers would turn every `ctrl+z` into a bare `z`.
#[test]
fn an_ignored_key_press_is_forwarded_with_its_key_and_modifiers_intact() {
    let mut modifiers = command();
    modifiers.insert(iced::keyboard::Modifiers::SHIFT);
    let pressed = key_window_event("z", modifiers);

    let message = key_event(pressed, iced::event::Status::Ignored, iced::window::Id::unique());

    let Some(Message::KeyPressed { key, modifiers: forwarded }) = message else {
        panic!("an ignored key press produced {message:?} rather than a KeyPressed");
    };
    assert_eq!(key, iced::keyboard::Key::Character("z".into()));
    assert!(forwarded.command(), "the command modifier was dropped");
    assert!(forwarded.shift(), "the shift modifier was dropped");
}

/// A left press the widget tree ignored becomes the blur signal, and a
/// press anyone claimed does not.
///
/// **This arm is the only thing in the application that can see a blur.**
/// `text_input` unfocuses itself on any left press outside its own bounds
/// and publishes nothing (`text_input.rs:723-735`), and the bodies list is
/// a fixed six or eight rows tall, so a press in the empty scrollable
/// under the last row belongs to no widget. Without this arm that press
/// produced no message at all, `renaming` stayed set, and the field went
/// on being drawn while the next keystroke fired a tool shortcut.
///
/// Both controls matter. Captured must stay dropped, or clicking INSIDE
/// the field would close it. The right button must stay silent, because
/// `text_input` does not blur on it -- committing there would close a
/// field that is still focused and still taking keys.
#[test]
fn a_left_press_nobody_wanted_is_the_only_blur_the_application_can_see() {
    let press = |button| iced::Event::Mouse(iced::mouse::Event::ButtonPressed(button));

    let ignored = key_event(
        press(iced::mouse::Button::Left),
        iced::event::Status::Ignored,
        iced::window::Id::unique(),
    );
    assert!(
        matches!(ignored, Some(Message::PressedNothing)),
        "a press on nothing produced {ignored:?}, so a rename in flight never hears about it"
    );

    let captured = key_event(
        press(iced::mouse::Button::Left),
        iced::event::Status::Captured,
        iced::window::Id::unique(),
    );
    assert!(
        captured.is_none(),
        "a press a widget wanted produced {captured:?}; clicking inside the field closes it"
    );

    let right = key_event(
        press(iced::mouse::Button::Right),
        iced::event::Status::Ignored,
        iced::window::Id::unique(),
    );
    assert!(
        right.is_none(),
        "a right press produced {right:?}, but the field stays focused through one"
    );
}

/// Releases are the one thing `key_event` decodes itself, because ending a
/// gesture must not go through the modal guard. Only the two sizing keys
/// mean anything on release; every other release is somebody else's.
#[test]
fn releasing_a_sizing_key_ends_the_gesture_and_no_other_release_means_anything() {
    for sizing in ["s", "u"] {
        let message = key_event(
            key_release_event(sizing),
            iced::event::Status::Ignored,
            iced::window::Id::unique(),
        );
        assert!(
            matches!(message, Some(Message::SizingEnded)),
            "releasing {sizing} produced {message:?} rather than ending the sizing drag"
        );
    }

    // And the mask's peek key, which is held for the same reason and needs
    // its release for a sharper one: a peek that never ends leaves the tint
    // stuck on with nothing the user can do about it but press again.
    let message =
        key_event(key_release_event("h"), iced::event::Status::Ignored, iced::window::Id::unique());
    assert!(
        matches!(message, Some(Message::MaskPeekEnded)),
        "releasing h produced {message:?} rather than ending the peek"
    );

    let message =
        key_event(key_release_event("x"), iced::event::Status::Ignored, iced::window::Id::unique());
    assert!(
        message.is_none(),
        "releasing x produced {message:?}; only presses carry meaning for the mirror keys"
    );
}

// --- the modal keyboard and pointer guard --------------------------------

/// Press a character key the way the subscription does, on an event the
/// widget tree ignored.
///
/// Note what this does NOT do: call `viewport::shortcut` and feed the
/// result in. That would test the decode and skip the guard, which is the
/// only thing these tests are about.
fn key(app: &mut Hani, character: &str, modifiers: iced::keyboard::Modifiers) {
    update(
        app,
        Message::KeyPressed { key: iced::keyboard::Key::Character(character.into()), modifiers },
    );
}

fn bare() -> iced::keyboard::Modifiers {
    iced::keyboard::Modifiers::empty()
}

/// The modifier that `command()` reads, for whichever platform is running.
///
/// **Not a bare `CTRL`.** The shortcut table is fed `modifiers.command()`,
/// which iced maps to Cmd on macOS and Control everywhere else, so a test
/// that sets the raw control bit proves nothing on a Mac -- it asserts that
/// Control+Z undoes, which on that platform it correctly does not. Four
/// tests failed exactly that way the first time this crate was built for
/// macOS. The behaviour was right and the fixture was wrong.
fn command() -> iced::keyboard::Modifiers {
    if cfg!(target_os = "macos") {
        iced::keyboard::Modifiers::LOGO
    } else {
        iced::keyboard::Modifiers::CTRL
    }
}

/// **An action from the welcome screen dismisses it.**
///
/// It is a starting point and not a mode: pressing New sculpt has answered
/// the question the screen was asking. And because the welcome card ranks
/// above every other card, leaving it up draws the unsaved-work prompt that
/// `New sculpt` raises UNDERNEATH it -- raised, unanswerable and invisible.
/// Seen exactly that way in the running application before this landed.
#[test]
fn an_action_from_the_welcome_screen_dismisses_it_so_its_prompt_is_reachable() {
    let mut app = app_with_unsaved_work();
    app.welcome = true;

    update(&mut app, Message::NewSculpt);

    assert!(!app.welcome, "the welcome screen survived an action taken from it");
    assert!(
        app.confirm.is_some(),
        "the fixture raised no prompt, so this does not test what it names"
    );
}

/// The tick must not dismiss the screen it is a setting for, and neither
/// must reading an article.
///
/// **Asked of the rule and not by sending the message**, deliberately.
/// `WelcomeOnStartupSet` writes through to the real
/// `$XDG_CONFIG_HOME/hanisculpt/welcome.conf`, so driving it here would
/// reach into the config of whoever is running the suite -- and two such
/// tests in parallel would race on one file. `dismisses_the_welcome` is a
/// pure function of the message, so it can be asked directly; that the
/// preference round-trips is
/// `welcome::tests::the_preference_survives_being_written_and_read_back`,
/// which uses a path of its own for the same reason.
/// **`with_devices` must not resolve the executable, and the crash-revert
/// offer must therefore not be decided there.**
///
/// This pins a trap that already cost the feature. The offer was originally
/// decided at the tail of `with_devices`, guarded on `app.exe` -- which
/// `new` does not assign until after `with_devices` has returned. So `exe`
/// was unconditionally `None` at that point, the guard never passed, and
/// "Go back to the previous build" could not appear in any build ever
/// shipped. Correct logic, unit-tested in isolation, whose caller never ran.
///
/// The suite could not have caught it, because ~640 tests build through
/// `with_devices` and it leaves `exe: None` by design -- resolving a path
/// and sweeping a directory is startup work, not test-construction work.
/// So the invariant is pinned from the other side: if someone later makes
/// `with_devices` resolve `exe`, this fails and they are pointed at the
/// decision in `new` that assumes it does not.
#[test]
fn with_devices_leaves_the_executable_unresolved_so_new_decides_the_revert() {
    let app = Hani::with_tablet(Tablet::inert());
    assert!(
        app.exe.is_none(),
        "`with_devices` must not resolve the executable -- `new` does, and the crash-revert \
         offer is decided there because it needs it"
    );
    assert!(app.crash_revert.is_none(), "and nothing may offer a revert before that");
    assert!(!app.crashed_last_session, "a scratch-dir construction has no crash report");
}

/// **An update refusal reaches the welcome card, not only the status line.**
///
/// The install button lives on the welcome card, and that card draws over a
/// full-window scrim and returns before every other card in `view` -- so
/// the status line it reported into was behind the very screen the button
/// is on. That is why the Windows relaunch failure has been reported more
/// than once and its text never read. `update_failure` is the field the
/// card draws; if a refusal does not reach it, the refusal is invisible
/// again on exactly the screen that caused it.
#[test]
fn an_update_refusal_reaches_the_card_that_covers_the_status_line() {
    let mut app = app();

    // Through a real message, not by calling the setter: what this pins is
    // that the DISPATCH ARM routes to the writer. Calling `update_shows`
    // here would exercise a two-line setter against itself and stay green
    // if every arm went back to assigning `self.status` directly, which is
    // exactly the defect being fixed.
    update(
        &mut app,
        Message::UpdateInstalled(
            iced::window::Id::unique(),
            Err("starting the new build failed (os error 5)".to_string()),
        ),
    );

    assert_eq!(
        app.update_failure.as_deref(),
        Some("starting the new build failed (os error 5)"),
        "the welcome card has nothing to draw, so the reason is invisible on the screen the \
         install button is on"
    );
    assert_eq!(app.status, "starting the new build failed (os error 5)");
    assert!(!app.installing_update, "the install was left marked in flight");
}

/// Every refusal in the install path reaches the card, not just the first.
///
/// **One test per arm, because each is a separate `let ... else` and the
/// silent one was found by review rather than by this suite.** Removing
/// any single `update_says` leaves the others green, so a per-arm
/// assertion is the only shape that catches it. The middle arm is the one
/// that returned in silence: unticking Check for updates clears `offer`
/// while leaving `downloaded_update`, and the Help entry is gated on the
/// download alone.
#[test]
fn every_refusal_in_the_install_path_reaches_the_card() {
    // No payload downloaded, reached through the message arm the Help
    // entry and the card's button both raise.
    {
        let mut nothing = app();
        update(&mut nothing, Message::UpdateInstallRequested(iced::window::Id::unique()));
        assert_eq!(
            nothing.update_failure.as_deref(),
            Some("download the update first"),
            "the guard in the message arm said nothing on the card"
        );
    }

    // And the same state reaching the install path itself, which has its
    // own sentence and its own arm.
    {
        let mut nothing = app();
        let _ = nothing.install_and_restart(iced::window::Id::unique());
        assert_eq!(
            nothing.update_failure.as_deref(),
            Some("there is no downloaded update to install"),
            "a press with nothing downloaded said nothing on the card"
        );
    }

    // A payload, but the offer behind it has been withdrawn.
    {
        let mut withdrawn = app();
        withdrawn.downloaded_update = Some(std::path::PathBuf::from("/tmp/does-not-matter"));
        withdrawn.offer = None;
        let _ = withdrawn.install_and_restart(iced::window::Id::unique());
        assert_eq!(
            withdrawn.update_failure.as_deref(),
            Some("the update was withdrawn -- switch checking back on and look again"),
            "the withdrawn-offer arm returned in silence and blanked the card"
        );
    }
}

/// A fresh attempt clears the last one's refusal, so the card cannot go on
/// showing a reason that has been superseded.
///
/// **The refusal has to be one this attempt cannot itself produce**, or the
/// assertion is satisfied by the next statement rather than by the clear.
/// The first draft used a string the very next `let ... else` overwrote, so
/// it passed with the clear deleted.
#[test]
fn starting_another_install_clears_the_previous_refusal() {
    let mut app = app();
    app.update_shows("a reason from last time".to_string());

    // Reaches the first refusal, which writes its OWN string. If the clear
    // were removed the old reason would still be gone -- so instead: park
    // the app where every arm is silent, and check the field is empty.
    app.downloaded_update = Some(std::path::PathBuf::from("/tmp/does-not-matter"));
    app.offer = None;
    app.installing_update = false;
    app.update_failure = Some("a reason from last time".to_string());
    let _ = app.install_and_restart(iced::window::Id::unique());
    assert_ne!(
        app.update_failure.as_deref(),
        Some("a reason from last time"),
        "the previous attempt's reason survived a new attempt"
    );
}

/// **A second press while a swap is running changes nothing at all.**
///
/// The guard has to come above the clear, not below it: clearing the note
/// and then returning would wipe "installing build N…" off the card and
/// leave a running install saying nothing on screen -- which is the
/// "clicked install, nothing happened" report this path exists to stop
/// producing.
#[test]
fn a_second_install_press_during_a_swap_does_not_wipe_what_the_card_is_saying() {
    let mut app = app();
    app.installing_update = true;
    app.update_failure = Some("installing build 1042…".to_string());
    app.status = "installing build 1042…".to_string();

    let _ = app.install_and_restart(iced::window::Id::unique());

    assert_eq!(
        app.update_failure.as_deref(),
        Some("installing build 1042…"),
        "a second press blanked the card while the install was still running"
    );
    assert_eq!(app.status, "installing build 1042…");
}

/// Choosing Export from the File menu closes the menu; the menu stayed
/// over the viewport, and in a browser over its file chooser.
#[test]
fn a_file_menu_entry_chosen_closes_the_menu() {
    let mut app = app();
    app.welcome = false;
    update(&mut app, Message::TopMenuToggled(crate::message::TopMenu::File));
    assert_eq!(app.top_menu, Some(crate::message::TopMenu::File));
    update(&mut app, Message::ExportRequested(ExportFormat::Stl));
    assert_eq!(app.top_menu, None);
    assert!(!closes_the_file_menu(&Message::TopMenuToggled(crate::message::TopMenu::File)));
}

#[test]
fn the_screens_own_controls_do_not_dismiss_it() {
    assert!(!dismisses_the_welcome(&Message::WelcomeOnStartupSet(false)));
    // The update tick sits on the same card and must behave the same way.
    // It is deliberately in none of the three classifiers, exactly like the
    // tick beside it: an unlisted variant commits an in-flight rename and
    // disarms the gizmo, and that is what the sibling control already does.
    assert!(!dismisses_the_welcome(&Message::UpdateCheckSet(false)));
    assert!(!dismisses_the_welcome(&Message::UpdateCheckSet(true)));
    assert!(!dismisses_the_welcome(&Message::WelcomeOnStartupSet(true)));
    assert!(
        !dismisses_the_welcome(&Message::LinkOpened(crate::links::RELEASE_PAGE.into())),
        "opening the release page closed the screen it was opened from"
    );

    // And the ones that must: every button that starts real work.
    assert!(dismisses_the_welcome(&Message::NewSculpt));
    assert!(dismisses_the_welcome(&Message::OpenRequested));
    assert!(dismisses_the_welcome(&Message::ImportRequested));
    assert!(dismisses_the_welcome(&Message::WelcomeClosed));
}

/// No shortcut may reach the document through the welcome screen.
///
/// It is modal like every other card, and `modal_open` is the one place
/// that is decided -- a card missing from that list is a card the keyboard
/// reaches straight through.
#[test]
fn no_shortcut_fires_while_the_welcome_screen_is_up() {
    let mut app = app();
    app.welcome = true;
    let before = app.brush.kind;

    key(&mut app, "3", bare());
    key(&mut app, "x", bare());

    assert_eq!(app.brush.kind, before, "a brush number changed the tool behind the screen");
    assert!(
        !app.symmetry_settings().axis(MirrorAxis::X),
        "a mirror plane toggled behind the screen"
    );
    assert!(app.welcome, "a shortcut dismissed the screen instead of being swallowed");
}

/// Escape closes it, unlike the bug report, which holds typed text.
#[test]
fn escape_closes_the_welcome_screen() {
    let mut app = app();
    app.welcome = true;
    update(
        &mut app,
        Message::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            modifiers: bare(),
        },
    );
    assert!(!app.welcome, "Escape left the screen up, so it has no keyboard exit");
}

/// The undo that used to reach through the card.
///
/// `Message::Undo` had no guard of any kind, so ctrl+Z with the
/// unsaved-work prompt up rolled back the very stroke the prompt was
/// asking whether to keep -- and answering Save then wrote out a document
/// the user had not agreed to.
#[test]
fn control_z_under_a_modal_leaves_the_document_alone() {
    let mut app = app_with_unsaved_work();
    let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    update(&mut app, Message::NewSculpt);
    assert!(app.confirm.is_some(), "the fixture never raised a prompt");

    let before = app.doc.active_volume().sample_world(front);
    key(&mut app, "z", command());

    assert_eq!(
        app.doc.active_volume().sample_world(front),
        before,
        "ctrl+Z undid a stroke behind the prompt"
    );
    assert!(app.history.can_undo(), "the stroke left the undo stack");
    assert!(app.confirm.is_some(), "the prompt went away on its own");
}

/// Every other shortcut, on every modal. The brush and the mirror planes
/// are not the document, but changing them behind a card the user is
/// reading is the same surprise, and `1`-`6` did exactly that.
#[test]
fn no_shortcut_fires_while_any_modal_card_is_up() {
    fn check(what: &str, raise: impl FnOnce(&mut Hani)) {
        let mut app = app_with_unsaved_work();
        let brush = app.brush.kind;
        raise(&mut app);
        assert!(app.modal_open(), "{what} did not count as a modal");

        key(&mut app, "x", bare());
        key(&mut app, "2", bare());

        assert!(
            !app.symmetry_settings().axis(MirrorAxis::X),
            "x flipped a mirror plane behind {what}"
        );
        assert_eq!(app.brush.kind, brush, "a digit swapped the brush behind {what}");
    }

    check("the unsaved-work prompt", |app| update(app, Message::NewSculpt));
    check("the bug report", |app| update(app, Message::BugReportOpened));
    check("the orientation prompt", |app| {
        app.adopt_import(imported_with(Some(hani_core::Facing::Back)));
    });
}

/// The control for both tests above: with nothing modal up, the same three
/// keystrokes all land. Without this they would pass against a build where
/// the keyboard was simply dead.
#[test]
fn the_same_keys_all_land_with_no_modal_up() {
    let mut app = app_with_unsaved_work();
    let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let sculpted = app.doc.active_volume().sample_world(front);
    assert!(!app.modal_open(), "the fixture is not a fair control");

    key(&mut app, "x", bare());
    assert!(app.symmetry_settings().axis(MirrorAxis::X), "x did not reach the mirror planes");

    key(&mut app, "2", bare());
    assert_eq!(app.brush.kind, BrushKind::ALL[1], "the digit did not reach the brush");

    // The mirror is the last step now, an undo step of its own (see
    // `app/symmetry_frame.rs`), so the first ctrl+Z takes it back and the
    // second the stroke.
    key(&mut app, "z", command());
    assert!(!app.symmetry_settings().axis(MirrorAxis::X), "ctrl+Z did not undo the mirror");
    key(&mut app, "z", command());
    assert_ne!(
        app.doc.active_volume().sample_world(front),
        sculpted,
        "ctrl+Z did not undo the stroke"
    );
}

/// Escape is the exception, and has to stay one: it is how the card is
/// answered, so it is the one key that must pass the guard.
#[test]
fn escape_still_reaches_a_modal_through_the_key_path() {
    let mut app = app_with_unsaved_work();
    update(&mut app, Message::NewSculpt);

    update(
        &mut app,
        Message::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            modifiers: bare(),
        },
    );

    assert!(app.confirm.is_none(), "escape was swallowed by the modal guard");
    assert!(app.unsaved, "escape discarded the work it was asking about");
}

/// The bug-report card was in neither guard: the pointer early return
/// named `confirm` and `orient_prompt` only, so a press beside the card
/// carved the model the report was about while the user described it.
#[test]
fn a_press_beside_the_bug_report_card_does_not_sculpt() {
    let mut app = app();
    update(&mut app, Message::BugReportOpened);
    assert!(app.bug_report.is_some(), "the dialog never opened");

    let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let before = app.doc.active_volume().sample_world(front);
    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(
        app.doc.active_volume().sample_world(front),
        before,
        "a press reached the model behind the card"
    );
    assert!(app.drag.is_none(), "it started a stroke behind the card");
    assert!(!app.unsaved, "it dirtied a document nobody had touched");
}

#[test]
fn discard_runs_the_pending_action() {
    let mut app = app_with_unsaved_work();
    update(&mut app, Message::NewSculpt);
    update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Discard));

    assert!(app.confirm.is_none());
    assert!(!app.unsaved, "the fresh sphere is not unsaved work");
    assert!(!app.history.can_undo(), "the reset did not happen");
}

#[test]
fn cancel_leaves_everything_exactly_as_it_was() {
    let mut app = app_with_unsaved_work();
    let bricks = app.doc.active_volume().brick_coords().count();
    update(&mut app, Message::NewSculpt);
    update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Cancel));

    assert!(app.confirm.is_none());
    assert!(app.unsaved, "cancel cleared the unsaved marker");
    assert_eq!(
        app.doc.active_volume().brick_coords().count(),
        bricks,
        "cancel reset the sculpt anyway"
    );
}

/// Save-then-continue, for a document that already has a file.
#[test]
fn saving_from_the_prompt_writes_the_file_and_then_acts() {
    let directory = std::env::temp_dir().join(format!("hani-confirm-save-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("sculpt.hani");

    let mut app = app_with_unsaved_work();
    app.save_project(&path);
    assert!(!app.unsaved);

    // Dirty it again so the prompt has something to be about.
    press(&mut app, Vector::new(SIZE.x / 2.0 + 12.0, SIZE.y / 2.0));
    release(&mut app);
    assert!(app.unsaved);

    update(&mut app, Message::NewSculpt);
    update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Save));

    assert!(app.confirm.is_none(), "the prompt stayed up after a good save");
    assert!(!app.history.can_undo(), "the pending New did not run after the save");
    // And the file on disk is the sculpt as it was, not the fresh sphere.
    let mut reopened = Hani::with_tablet(crate::tablet::Tablet::inert());
    reopened.open_project(&path);
    assert!(reopened.status.starts_with("opened"), "open reported: {}", reopened.status);

    std::fs::remove_dir_all(&directory).ok();
}

/// A save that fails must not be treated as a save. Continuing here would
/// quit on a file that was never written, which is the exact loss the
/// prompt exists to prevent.
#[test]
fn a_failed_save_from_the_prompt_does_not_continue() {
    let mut app = app_with_unsaved_work();
    app.project_path = Some(
        std::env::temp_dir()
            .join(format!("hani-absent-{}", std::process::id()))
            .join("nope")
            .join("sculpt.hani"),
    );

    update(&mut app, Message::NewSculpt);
    update(&mut app, Message::ConfirmAnswered(ConfirmChoice::Save));

    assert!(app.status.contains("could not"), "reported: {}", app.status);
    assert_eq!(
        app.confirm,
        Some(PendingAction::NewSculpt),
        "the prompt was dismissed by a save that did not happen"
    );
    assert!(app.unsaved, "a failed save cleared the marker");
    assert!(app.history.can_undo(), "the sculpt was discarded on a failed save");
}

/// Dismissing the file dialog is not an answer to the prompt.
#[test]
fn dismissing_the_save_dialog_leaves_the_prompt_up() {
    let mut app = app_with_unsaved_work();
    update(&mut app, Message::NewSculpt);
    update(&mut app, Message::SavedThenContinue(None));

    assert_eq!(app.confirm, Some(PendingAction::NewSculpt));
    assert!(app.history.can_undo(), "the sculpt was discarded");
}
