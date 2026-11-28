// SPDX-License-Identifier: AGPL-3.0-only

//! The camera through the application: orbiting, the navigation cube's
//! flights and its menu, the brush's right-click menu, and turning the
//! model. Moved out of `app.rs`'s `mod tests` (issue 189).

use super::test_support::*;
use super::*;
use iced::Vector;

/// Run a camera flight to completion in controlled time.
///
/// `Message::Frame` scales by the real clock, and consecutive calls in a
/// test are microseconds apart, so driving it that way would never arrive.
fn finish_flight(app: &mut Hani) {
    for _ in 0..200 {
        if app.flight.is_none() {
            return;
        }
        app.advance_flight(16.0);
        app.publish_camera();
    }
    panic!("the flight never finished");
}

#[test]
fn orbiting_moves_the_camera_without_touching_the_model() {
    let mut app = app();
    let yaw = app.camera.yaw;
    let bricks = app.doc.active_volume().brick_count();

    app.on_pointer(PointerEvent::Pressed {
        button: PointerButton::Right,
        position: centre_of_viewport(),
        size: SIZE,
    });
    app.on_pointer(PointerEvent::Moved { position: Vector::new(460.0, 300.0), size: SIZE });

    assert_ne!(app.camera.yaw, yaw, "a right drag should have orbited");
    assert_eq!(app.doc.active_volume().brick_count(), bricks, "orbiting must not sculpt");
}

/// **A quarter turn turns the whole document.**
///
/// Bodies share one lattice and have no transform, so their arrangement is
/// the only positional state there is. `orient` turned `active_volume()`
/// alone, which with two bodies scatters that arrangement — and makes its
/// own status line, "turn it back the same way to undo", untrue for every
/// body that stayed put. The bit-exactness of four turns is measured in
/// `hani-core`; this is the wiring.
#[test]
fn turning_the_model_turns_every_body_and_not_just_the_active_one() {
    let mut app = app();
    // Off the axis the turn is about: a ball sitting ON that axis comes
    // back to the same box whether the turn reached it or not, and this
    // test would then pass with `orient` still turning one body.
    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(0.0, 50.0, 0.0), 8.0);
    second.mark_everything_dirty();
    let other = app.doc.add_body("Body 2", second);
    app.publish_visibility();
    app.remesh_dirty();

    let before = app.doc.volume(other).expect("a live body").world_bounds().expect("a seeded ball");
    app.orient(hani_core::AxisRotation::taking(hani_core::Facing::Front, hani_core::Facing::Up));
    let after = app.doc.volume(other).expect("a live body").world_bounds().expect("still a ball");

    assert_ne!(before, after, "the inactive body did not turn with the model");
    assert!(
        ((after.1 - after.0).length() - (before.1 - before.0).length()).abs() < 1.0e-3,
        "the inactive body changed size rather than orientation: {before:?} to {after:?}"
    );
}

/// Clicking the cube must move the camera and must NOT sculpt. Getting that
/// wrong takes a divot out of the model every time someone reaches for a
/// standard view.
#[test]
fn clicking_the_navigation_cube_flies_the_camera_without_sculpting() {
    let mut app = app();
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    let probe = app
        .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
        .expect("the centre should hit the sphere");
    let before_field = app.doc.active_volume().sample_world(probe);
    let before_yaw = app.camera.yaw;

    let (origin, size) = crate::navcube::corner_rect(Vec2::new(SIZE.x, SIZE.y));
    let middle = origin + size * 0.5;
    press(&mut app, Vector::new(middle.x, middle.y));
    release(&mut app);

    assert!(app.flight.is_some(), "clicking the cube did not start a move");
    assert_eq!(
        app.doc.active_volume().sample_world(probe),
        before_field,
        "clicking the cube carved the model"
    );
    assert!(!app.history.can_undo(), "clicking the cube recorded an undo entry");
    assert!(app.drag.is_none(), "clicking the cube started a drag");

    finish_flight(&mut app);
    assert!(app.flight.is_none(), "the move never finished");
    assert_ne!(app.camera.yaw, before_yaw, "the camera did not actually move");
    assert!(app.camera.view().to_cols_array().iter().all(|v| v.is_finite()));
}

/// A click on Top asks for a pitch of exactly ninety degrees, which is
/// exactly where `look_at` used to collapse.
///
/// **This replaces `flying_to_a_pole_stops_short_of_collapsing_the_view`,
/// and the inversion is the fix.** The old test asserted the camera did
/// NOT arrive at the pole -- because `PITCH_SAFE` stopped it 0.02 rad
/// short, so clicking "top" gave a view that was visibly not the top. The
/// composed basis is orthonormal there, so the assertion is now that the
/// camera lands exactly on the pole and the matrix survives it.
#[test]
fn flying_to_a_pole_arrives_at_the_pole_with_the_view_intact() {
    for direction in [Vec3::Y, Vec3::NEG_Y] {
        let mut app = app();
        app.fly_to(crate::navcube::Part { direction, extremes: 1 });
        finish_flight(&mut app);
        assert!(app.flight.is_none());
        assert!(
            (app.camera.pitch.abs() - std::f32::consts::FRAC_PI_2).abs() < 1.0e-4,
            "{direction:?} did not reach the pole: pitch {}",
            app.camera.pitch
        );
        // Straight down means the view direction is world up, exactly.
        let looking = (app.camera.target - app.camera.eye()).normalize();
        assert!(
            looking.dot(-direction).abs() > 0.9999,
            "{direction:?} did not end up looking along the axis: {looking:?}"
        );
        assert!(app.camera.view().to_cols_array().iter().all(|v| v.is_finite()));
        assert!((app.camera.right().length() - 1.0).abs() < 1.0e-4);
        assert!(app.camera.right().dot(app.camera.up()).abs() < 1.0e-4);
    }
}

/// The camera must not spin several times round to reach a heading a few
/// degrees away, which is what interpolating an unwrapped yaw would do.
#[test]
fn a_flight_takes_the_short_way_round_and_unwinds_any_roll() {
    let mut app = app();
    // Many turns of yaw, and a roll from the puck, as a session would
    // accumulate.
    app.camera.yaw = std::f32::consts::TAU * 3.0 + 0.2;
    app.camera.roll = 0.7;

    app.fly_to(crate::navcube::Part { direction: Vec3::Z, extremes: 1 });
    let flight = app.flight.expect("no flight started");
    let travel = (flight.to.0 - flight.from.0).abs();
    assert!(travel <= std::f32::consts::PI + 1.0e-4, "the flight would spin {travel} radians");

    finish_flight(&mut app);
    assert!(
        app.camera.roll.abs() < 1.0e-4,
        "a standard view should be level, roll was {}",
        app.camera.roll
    );
}

/// The flight is driven off the frame tick, so that link needs its own
/// test: the tests above supply their own time to get a result at all.
#[test]
fn the_frame_tick_is_what_advances_a_flight() {
    let mut app = app();
    app.fly_to(crate::navcube::Part { direction: Vec3::X, extremes: 1 });
    // The first frame has no previous one to measure against, so it
    // contributes nothing; the second is the one that moves.
    update(&mut app, Message::Frame);
    update(&mut app, Message::Frame);
    let elapsed = app.flight.expect("the flight ended early").elapsed_ms;
    assert!(elapsed > 0.0, "the frame tick did not advance the flight");
}

fn right_press(app: &mut Hani, at: Vector) {
    app.on_pointer(PointerEvent::Pressed {
        button: PointerButton::Right,
        position: at,
        size: SIZE,
    });
}

fn right_release(app: &mut Hani) {
    app.on_pointer(PointerEvent::Released { button: PointerButton::Right });
}

/// Right drag already orbits, so the menu has to be told apart from it by
/// movement and time. All three outcomes matter, and getting any of them
/// wrong makes either the menu or the orbit unusable.
#[test]
fn a_right_click_opens_the_menu_but_a_right_drag_orbits() {
    let centre = centre_of_viewport();
    let moved_to = |dx: f32| Vector::new(SIZE.x / 2.0 + dx, SIZE.y / 2.0);

    // A click that does not move: the menu.
    let mut click = app();
    right_press(&mut click, centre);
    right_release(&mut click);
    assert!(click.menu.is_some(), "a right click did not open the menu");

    // A drag: orbits, and opens nothing.
    let mut orbit = app();
    let before = orbit.camera.yaw;
    right_press(&mut orbit, centre);
    orbit.on_pointer(PointerEvent::Moved { position: moved_to(60.0), size: SIZE });
    right_release(&mut orbit);
    assert!(orbit.menu.is_none(), "an orbit opened the menu");
    assert_ne!(orbit.camera.yaw, before, "the orbit did not happen");

    // A drag that returns to where it started is still a drag.
    let mut returned = app();
    right_press(&mut returned, centre);
    for dx in [40.0, 0.0] {
        returned.on_pointer(PointerEvent::Moved { position: moved_to(dx), size: SIZE });
    }
    right_release(&mut returned);
    assert!(returned.menu.is_none(), "an orbit that came back opened the menu");

    // Inside the slop is still a click: a hand never holds perfectly still.
    let mut wobble = app();
    right_press(&mut wobble, centre);
    wobble.on_pointer(PointerEvent::Moved { position: moved_to(CLICK_SLOP_PX * 0.5), size: SIZE });
    right_release(&mut wobble);
    assert!(wobble.menu.is_some(), "a click that wobbled a pixel opened nothing");
}

#[test]
fn the_menu_closes_on_escape_and_on_a_click_elsewhere() {
    let mut app = app();
    right_press(&mut app, centre_of_viewport());
    right_release(&mut app);
    assert!(app.menu.is_some());

    update(&mut app, Message::MenuClosed);
    assert!(app.menu.is_none(), "escape did not close it");

    // Open again, then click elsewhere: the click closes it and must not
    // also sculpt, or dismissing a menu would carve the model.
    right_press(&mut app, centre_of_viewport());
    right_release(&mut app);
    let probe = app
        .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
        .expect("the centre should hit the sphere");
    let before = app.doc.active_volume().sample_world(probe);

    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert!(app.menu.is_none(), "a click elsewhere did not close the menu");
    assert_eq!(app.doc.active_volume().sample_world(probe), before, "dismissing the menu sculpted");
}

#[test]
fn hovering_the_cube_hides_the_brush_ring() {
    let mut app = app();
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    assert!(app.hover.is_some(), "the model should be under the pointer");

    let (origin, size) = crate::navcube::corner_rect(Vec2::new(SIZE.x, SIZE.y));
    let middle = origin + size * 0.5;
    app.on_pointer(PointerEvent::Moved { position: Vector::new(middle.x, middle.y), size: SIZE });

    assert!(app.cube_hover.is_some(), "the cube should be under the pointer");
    // No ring: a press here will not sculpt, and drawing one would promise
    // that it would.
    assert!(app.hover.is_none());
    assert!(app.shared.overlay_snapshot().lines.is_empty());
}

// --- re-orienting the model ------------------------------------------

/// The middle of the navigation cube, which always picks a face rather than
/// an edge or a corner.
fn cube_centre() -> Vector {
    let (origin, size) = crate::navcube::corner_rect(Vec2::new(SIZE.x, SIZE.y));
    let middle = origin + size * 0.5;
    Vector::new(middle.x, middle.y)
}

/// A model with material in one identifiable place, so a turn is visible.
///
/// The seeded sphere is centred and symmetric, so every rotation leaves it
/// looking exactly the same -- a test built on it would pass whether or not
/// anything turned.
fn lopsided(app: &mut Hani) {
    let mut volume = hani_core::Volume::new(app.doc.voxel_size());
    volume.seed_sphere(Vec3::new(0.0, 30.0, 0.0), 8.0);
    volume.mark_everything_dirty();
    app.doc.replace_active_volume(volume);
    app.remesh_dirty();
}

/// The collision this feature had to resolve. A right press on the cube
/// used to fall through to an orbit and then, on release, open the BRUSH
/// menu on top of the cube -- so the one gesture that should have asked
/// about orientation asked about brush radius instead.
#[test]
fn a_right_click_on_the_cube_opens_the_cubes_menu_and_not_the_brushs() {
    let mut app = app();
    right_press(&mut app, cube_centre());
    right_release(&mut app);

    assert!(app.cube_menu.is_some(), "the cube's own menu did not open");
    assert!(app.menu.is_none(), "the brush menu opened on top of the cube");
    assert!(app.drag.is_none(), "the press started an orbit");
    assert!(app.flight.is_none(), "a right click flew the camera as though it were a left one");
}

#[test]
fn a_left_click_on_the_cube_still_flies_the_camera() {
    // The half of the cube's behaviour that already existed, pinned
    // alongside the new half so a change to the routing cannot take it out
    // silently.
    let mut app = app();
    press(&mut app, cube_centre());
    release(&mut app);

    assert!(app.flight.is_some(), "the left click stopped moving the camera");
    assert!(app.cube_menu.is_none(), "a left click opened the orientation menu");
}

#[test]
fn the_next_press_closes_the_cube_menu_without_sculpting() {
    let mut app = app();
    let probe = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    right_press(&mut app, cube_centre());
    right_release(&mut app);
    assert!(app.cube_menu.is_some());

    let before = app.doc.active_volume().sample_world(probe);
    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert!(app.cube_menu.is_none(), "the menu stayed open");
    assert_eq!(
        app.doc.active_volume().sample_world(probe),
        before,
        "dismissing the menu also sculpted"
    );
}

#[test]
fn escape_closes_the_cube_menu() {
    let mut app = app();
    right_press(&mut app, cube_centre());
    right_release(&mut app);
    update(&mut app, Message::MenuClosed);
    assert!(app.cube_menu.is_none());
}

#[test]
fn choosing_a_direction_turns_the_model_and_leaves_the_camera_alone() {
    let mut app = app();
    lopsided(&mut app);
    let (yaw, pitch, roll) = (app.camera.yaw, app.camera.pitch, app.camera.roll);
    let above = Vec3::new(0.0, 30.0, 0.0);
    let in_front = Vec3::new(0.0, 0.0, 30.0);
    assert!(app.doc.active_volume().sample_world(above) < 0.0, "the fixture is wrong");

    app.cube_menu = Some(CubeMenu { at: Vec2::new(700.0, 20.0), facing: hani_core::Facing::Up });
    update(&mut app, Message::OrientFace(hani_core::Facing::Front));

    // The model moved...
    assert!(
        app.doc.active_volume().sample_world(in_front) < 0.0,
        "the material did not arrive in front of the origin"
    );
    assert!(
        app.doc.active_volume().sample_world(above) >= 0.0,
        "the material is still above the origin"
    );
    // ...and the camera did not. This is the whole difference between
    // turning the model and re-aiming the view, and it is what makes an
    // export land upright.
    assert_eq!((app.camera.yaw, app.camera.pitch, app.camera.roll), (yaw, pitch, roll));

    assert!(app.cube_menu.is_none(), "the menu stayed open over a model that had moved");
    assert!(app.unsaved, "a turned model is unsaved work");
    assert!(!app.history.can_undo(), "history outlived the volume it named bricks of");
    assert!(app.perf.dirty_bricks > 0, "nothing was remeshed, so the turn is invisible");
}

#[test]
fn turning_a_face_onto_itself_changes_nothing() {
    // The menu greys this one out, but the message is public and the guard
    // is in `orient`, not in the widget.
    let mut app = app();
    lopsided(&mut app);
    app.unsaved = false;
    let above = Vec3::new(0.0, 30.0, 0.0);
    let before = app.doc.active_volume().sample_world(above);

    app.cube_menu = Some(CubeMenu { at: Vec2::ZERO, facing: hani_core::Facing::Up });
    update(&mut app, Message::OrientFace(hani_core::Facing::Up));

    assert_eq!(app.doc.active_volume().sample_world(above), before);
    assert!(!app.unsaved, "a turn that did nothing still marked the document dirty");
}

#[test]
fn turning_back_returns_the_model() {
    // The recovery path, and the reason there is no undo entry: a quarter
    // turn is exact, so the other way round is a real undo rather than an
    // approximation of one.
    let mut app = app();
    lopsided(&mut app);
    let above = Vec3::new(0.0, 30.0, 0.0);
    let before = app.doc.active_volume().sample_world(above);

    app.cube_menu = Some(CubeMenu { at: Vec2::ZERO, facing: hani_core::Facing::Up });
    update(&mut app, Message::OrientFace(hani_core::Facing::Front));
    app.cube_menu = Some(CubeMenu { at: Vec2::ZERO, facing: hani_core::Facing::Front });
    update(&mut app, Message::OrientFace(hani_core::Facing::Up));

    assert_eq!(
        app.doc.active_volume().sample_world(above),
        before,
        "turning back did not return the model"
    );
}
