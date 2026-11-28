// SPDX-License-Identifier: AGPL-3.0-only

//! A stroke through the application: sculpting and its undo, symmetry, the
//! sizing gesture, Move, the body under the pointer, the mirror planes a
//! body allows, the dynamic radius, the modifiers and the pen. Moved out of
//! `app.rs`'s `mod tests` (issue 189).

use super::test_support::*;
use super::*;
use crate::tablet::PenState;
use hani_core::{BrushKind, Change};
use iced::Vector;

/// The whole input to geometry path, with no window and no GPU: a press at
/// the centre of the viewport must raycast onto the sphere, stamp the
/// brush, and leave meshed bricks waiting for upload.
#[test]
fn a_press_at_the_centre_of_the_viewport_changes_the_model() {
    let mut app = app();
    let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let before = app.doc.active_volume().sample_world(front);

    press(&mut app, centre_of_viewport());

    assert!(app.perf.edit_ms > 0.0, "no edit was timed, so the raycast missed the sphere");
    assert!(app.perf.dirty_bricks > 0, "the stroke dirtied nothing");
    assert!(
        app.doc.active_volume().sample_world(front) < before,
        "adding clay should have pushed the field negative at the surface"
    );
}

/// The stats readout starts collapsed, and the icon is what brings it back.
///
/// Pinned because "it is not covering the model" is the whole point of the
/// control: a default flipped by an unrelated edit to the constructor would
/// put seven lines of monospace back across the corner of the sculpt with
/// nothing failing anywhere.
#[test]
fn the_stats_readout_starts_collapsed_and_the_icon_toggles_it() {
    let mut app = app();
    assert!(!app.stats_open, "the readout should not be over the model on launch");

    update(&mut app, Message::StatsToggled);
    assert!(app.stats_open, "pressing the info icon should show the numbers");

    update(&mut app, Message::StatsToggled);
    assert!(!app.stats_open, "pressing it again should put them away");
}

#[test]
fn the_history_budget_is_reported_before_anything_is_drawn() {
    let app = app();
    assert!(
        app.history_stats.budget_bytes > 0,
        "the overlay would show a zero byte history budget until the first stroke"
    );
}

#[test]
fn a_press_that_misses_the_model_changes_nothing() {
    let mut app = app();
    let bricks_before = app.doc.active_volume().brick_count();

    // The far corner of the viewport looks past the sphere into empty space.
    press(&mut app, Vector::new(2.0, 2.0));

    assert_eq!(app.doc.active_volume().brick_count(), bricks_before, "a miss must not allocate");
    assert_eq!(app.perf.dirty_bricks, 0, "a miss must not schedule a remesh");
}

#[test]
fn releasing_a_different_button_does_not_cancel_a_stroke() {
    let mut app = app();
    press(&mut app, centre_of_viewport());
    app.on_pointer(PointerEvent::Released { button: PointerButton::Right });
    assert!(app.drag.is_some(), "the left button drag should still be live");

    release(&mut app);
    assert!(app.drag.is_none());
}

#[test]
fn a_finished_stroke_becomes_exactly_one_undo_entry() {
    let mut app = app();
    press(&mut app, centre_of_viewport());
    for offset in 1..8 {
        app.on_pointer(PointerEvent::Moved {
            position: centre_of_viewport() + Vector::new(offset as f32 * 6.0, 0.0),
            size: SIZE,
        });
    }
    assert_eq!(app.history_stats.undo_entries, 0, "history should wait for the button up");

    release(&mut app);
    assert_eq!(
        app.history_stats.undo_entries, 1,
        "a whole drag is one entry, not one per pointer event"
    );
}

/// The eviction that has to speak for itself.
///
/// A per-operation prompt cannot catch this: two folder deletes that each
/// pass the reclaim allowance on their own go on to evict each other, and
/// the only moment anything knows the body has become unrecoverable is the
/// eviction. The delete gesture itself does not exist yet, so the entry is
/// built here by hand -- what is under test is the path from
/// `HistoryStats::dropped_bodies` to something the user can read.
#[test]
fn an_eviction_that_takes_a_deleted_body_with_it_reaches_the_status_line() {
    let mut app = app();
    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 10.0);
    let doomed = app.doc.add_body("Body 2", second);
    app.remesh_dirty();

    // An allowance of one byte, so the next push evicts the delete while
    // the stroke budget is nowhere near its ceiling.
    app.history = History::with_budgets(hani_core::DEFAULT_HISTORY_BUDGET, 1);
    let at = app.doc.index_of(doomed).expect("the second body");
    let node = app.doc.remove(at);
    app.history.push(Entry::new(vec![Change::NodeRemoved { at, node: Box::new(node) }]));
    assert!(app.status.is_empty(), "nothing has been evicted yet");

    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(app.history_stats.dropped_bodies, 1, "the delete was not evicted");
    assert!(
        app.status.contains("deleted body"),
        "the eviction left the user nothing to read: {:?}",
        app.status
    );
}

/// A body that comes back from the undo stack has to reach the GPU, and
/// the count the remesh reports is the only thing in the application that
/// says it did.
///
/// `perf.dirty_bricks` and not `perf.remesh_ms`: the first is written
/// before `remesh_dirty`'s early return and the second only past it, so a
/// restored body that scheduled nothing would leave `remesh_ms` reading
/// whatever the last real remesh cost.
///
/// The remesh before the delete is load-bearing -- it is what drains the
/// dirty set the second body was seeded with, so that the only thing that
/// can mark its bricks afterwards is `Document::insert`.
#[test]
fn undoing_a_body_delete_schedules_every_one_of_its_bricks_for_remesh() {
    let mut app = app();
    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(60.0, 0.0, 0.0), 10.0);
    let doomed = app.doc.add_body("Body 2", second);
    app.remesh_dirty();

    let bricks = app.doc.volume(doomed).expect("the second body").brick_count();
    assert!(bricks > 0, "the fixture body must have bricks or this asserts nothing");

    // The delete itself, which has no interface yet: the node moves out of
    // the document and into the entry.
    let at = app.doc.index_of(doomed).expect("the second body");
    let node = app.doc.remove(at);
    app.history.push(Entry::new(vec![Change::NodeRemoved { at, node: Box::new(node) }]));

    update(&mut app, Message::Undo);

    assert!(app.doc.volume(doomed).is_some(), "the body did not come back");
    assert!(
        app.perf.dirty_bricks >= bricks,
        "the restored body scheduled {} bricks, not its {bricks}, so it never reaches the screen",
        app.perf.dirty_bricks
    );
}

#[test]
fn undo_returns_the_model_to_where_it_started() {
    let mut app = app();
    let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let before = app.doc.active_volume().sample_world(front);

    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert_ne!(app.doc.active_volume().sample_world(front), before);

    update(&mut app, Message::Undo);
    assert_eq!(
        app.doc.active_volume().sample_world(front),
        before,
        "undo did not restore the field"
    );
    assert!(app.perf.dirty_bricks > 0, "undo must schedule a remesh or the screen goes stale");

    update(&mut app, Message::Redo);
    assert_ne!(
        app.doc.active_volume().sample_world(front),
        before,
        "redo did not reapply the stroke"
    );
}

#[test]
fn a_drag_stamps_more_than_once_along_its_path() {
    // Without interpolation a fast drag leaves a dotted trail, so check the
    // stroke actually produced several stamps from one pointer event.
    let mut app = app();
    app.brush.radius = 1.0;
    press(&mut app, centre_of_viewport());

    app.on_pointer(PointerEvent::Moved {
        position: centre_of_viewport() + Vector::new(80.0, 0.0),
        size: SIZE,
    });
    assert!(
        app.perf.stamps > 1,
        "one long pointer move should interpolate, got {} stamps",
        app.perf.stamps
    );
}

#[test]
fn symmetry_sculpts_both_sides_at_once() {
    let mut app = app();
    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
    // Nothing has told the application how big the viewport is yet, and
    // the ray depends on it.
    app.viewport_size = Vec2::new(SIZE.x, SIZE.y);

    // Aim off to one side so the mirrored half lands somewhere distinct.
    let off_centre = Vector::new(SIZE.x * 0.38, SIZE.y * 0.5);
    let hit = app
        .surface_under(Vec2::new(off_centre.x, off_centre.y))
        .expect("the test needs a point that is on the model");
    let mirrored = Vec3::new(-hit.x, hit.y, hit.z);
    let before = app.doc.active_volume().sample_world(mirrored);

    press(&mut app, off_centre);
    release(&mut app);

    assert!(
        app.doc.active_volume().sample_world(mirrored) < before,
        "the mirrored half of the stroke never landed"
    );
}

/// Six copies round Z: one press lays the stroke at every sixth of a
/// turn round the axis through the middle, and not only where it was
/// pressed. See `app/radial.rs`.
#[test]
fn radial_symmetry_lays_a_copy_at_every_turn() {
    let mut app = app();
    update(&mut app, Message::Radial(crate::app::RadialMessage::Copies(6.0)));
    update(&mut app, Message::Radial(crate::app::RadialMessage::Axis(MirrorAxis::Z)));
    app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
    let off_centre = Vector::new(SIZE.x * 0.5, SIZE.y * 0.3);
    let hit = app
        .surface_under(Vec2::new(off_centre.x, off_centre.y))
        .expect("the test needs a point that is on the model");
    let copies: Vec<Vec3> = (1..6)
        .map(|k| glam::Quat::from_rotation_z(std::f32::consts::TAU * k as f32 / 6.0) * hit)
        .collect();
    let before: Vec<f32> =
        copies.iter().map(|at| app.doc.active_volume().sample_world(*at)).collect();
    press(&mut app, off_centre);
    release(&mut app);
    for (k, (at, was)) in copies.iter().zip(before).enumerate() {
        let now = app.doc.active_volume().sample_world(*at);
        assert!(now < was, "copy {} at {at} never landed: {was} then {now}", k + 1);
    }
}

/// The ZBrush convention, and the biggest ergonomic win in this round.
#[test]
fn holding_shift_sculpts_with_smooth_without_changing_the_selection() {
    let mut app = app();
    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    assert_eq!(app.effective_brush().kind, BrushKind::Draw);

    update(
        &mut app,
        Message::Pointer(PointerEvent::Modifiers { shift: true, control: false, alt: false }),
    );
    assert_eq!(app.effective_brush().kind, BrushKind::Smooth, "shift should smooth");
    assert_eq!(
        app.brush.kind,
        BrushKind::Draw,
        "the selection itself must not change, or the tool strip would flicker \
         and a key released out of focus would strand the wrong brush"
    );

    update(
        &mut app,
        Message::Pointer(PointerEvent::Modifiers { shift: false, control: false, alt: false }),
    );
    assert_eq!(app.effective_brush().kind, BrushKind::Draw, "releasing shift should restore");
}

#[test]
fn holding_shift_actually_smooths_the_model_rather_than_drawing_on_it() {
    let mut app = app();
    // A move first, so the application knows the viewport size and
    // `surface_under` agrees with where a press will actually land. The
    // default size is not SIZE, and disagreeing here probes a point the
    // brush never touches.
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    let probe = app
        .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
        .expect("the centre of the viewport should hit the sphere");
    let flat = app.doc.active_volume().sample_world(probe);

    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    for _ in 0..4 {
        press(&mut app, centre_of_viewport());
        release(&mut app);
    }
    let raised = app.doc.active_volume().sample_world(probe);
    assert!(raised < flat, "drawing should have pushed the surface out past the probe");

    // Now the same gesture with shift held. Nothing about the selection
    // changes, but the strokes must smooth rather than pile on more.
    update(
        &mut app,
        Message::Pointer(PointerEvent::Modifiers { shift: true, control: false, alt: false }),
    );
    for _ in 0..12 {
        press(&mut app, centre_of_viewport());
        release(&mut app);
    }
    let smoothed = app.doc.active_volume().sample_world(probe);

    assert_ne!(smoothed, raised, "holding shift changed nothing at all");
    assert!(
        smoothed > raised,
        "shift should have flattened the bump back, not built on it: \
         {flat} flat, {raised} raised, {smoothed} after smoothing"
    );
    assert_eq!(app.brush.kind, BrushKind::Draw, "the selection must be untouched");
}

#[test]
fn a_fresh_application_starts_with_every_mirror_plane_off() {
    let app = app();
    assert!(app.symmetry_settings().is_off(), "symmetry was {:?}", app.symmetry_settings());
    for axis in MirrorAxis::ALL {
        assert!(!app.symmetry_settings().axis(axis), "{} was on at startup", axis.label());
    }
}

#[test]
fn each_mirror_axis_reaches_the_opposite_side() {
    for (axis, probe) in [
        (MirrorAxis::X, Vec3::new(-1.0, 1.0, 1.0)),
        (MirrorAxis::Y, Vec3::new(1.0, -1.0, 1.0)),
        (MirrorAxis::Z, Vec3::new(1.0, 1.0, -1.0)),
    ] {
        let mut app = app();
        update(&mut app, Message::SymmetryAxisToggled(axis));
        assert!(app.symmetry_settings().axis(axis), "{} did not turn on", axis.label());

        // On the surface, and off all three planes, so only the axis
        // under test can carry the stroke to the probe point.
        let at = Vec3::new(14.0, 14.0, 18.0).normalize() * MODEL_RADIUS_MM;
        let normal = app.doc.active_volume().gradient_world(at);
        let before = app.doc.active_volume().sample_world(at * probe);

        app.doc.active_volume_mut().begin_stroke();
        let symmetry = app.symmetry_settings();
        app.brush.apply_symmetric(
            app.doc.active_volume_mut(),
            &Stamp::new(at, normal, BrushDirection::Add),
            symmetry,
            MIRROR_CENTRE,
            &mut app.brush_scratch,
        );

        assert!(
            app.doc.active_volume().sample_world(at * probe) < before,
            "{} symmetry never reached the other side",
            axis.label()
        );

        // ...and toggling it again turns it back off.
        update(&mut app, Message::SymmetryAxisToggled(axis));
        assert!(!app.symmetry_settings().axis(axis));
    }
}

/// The fast path for radius in every sculpting tool is a drag, not a
/// slider. Absolute against where the drag began, so out and back returns
/// to exactly the original value rather than drifting.
#[test]
fn holding_the_sizing_key_and_dragging_scales_the_radius_reversibly() {
    let mut app = app();
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    let original = app.brush.radius;

    update(&mut app, Message::SizingStarted(SizingTarget::Radius));
    assert!(app.sizing.is_some(), "the gesture did not start");

    let at = |dx: f32| Vector::new(SIZE.x / 2.0 + dx, SIZE.y / 2.0);
    app.on_pointer(PointerEvent::Moved { position: at(120.0), size: SIZE });
    let grown = app.brush.radius;
    assert!(grown > original, "dragging right should grow the brush");

    app.on_pointer(PointerEvent::Moved { position: at(-120.0), size: SIZE });
    assert!(app.brush.radius < original, "dragging left should shrink it");

    // Back where it started: absolute, not accumulated.
    app.on_pointer(PointerEvent::Moved { position: at(0.0), size: SIZE });
    assert!(
        (app.brush.radius - original).abs() < 1.0e-4,
        "the gesture drifted: {original} became {}",
        app.brush.radius
    );

    update(&mut app, Message::SizingEnded);
    assert!(app.sizing.is_none());
    // ...and the pointer goes back to sculpting.
    app.on_pointer(PointerEvent::Moved { position: at(200.0), size: SIZE });
    assert!((app.brush.radius - original).abs() < 1.0e-4, "sizing outlived its key");
}

/// The failure this would otherwise cause is the worst kind: a drag that
/// silently carves the model while the user thinks they are resizing.
#[test]
fn a_sizing_drag_does_not_sculpt() {
    let mut app = app();
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    let probe = app
        .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
        .expect("the centre should hit the sphere");
    let before = app.doc.active_volume().sample_world(probe);

    update(&mut app, Message::SizingStarted(SizingTarget::Radius));
    press(&mut app, centre_of_viewport());
    for step in 1..=8 {
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x / 2.0 + step as f32 * 10.0, SIZE.y / 2.0),
            size: SIZE,
        });
    }
    release(&mut app);

    assert_eq!(
        app.doc.active_volume().sample_world(probe),
        before,
        "a sizing drag cut into the model"
    );
    assert!(!app.history.can_undo(), "a sizing drag recorded an undo entry");
}

#[test]
fn the_sizing_gesture_stays_inside_the_slider_bounds() {
    // The radius ceiling now depends on the voxel size, so it is asked of a
    // real app rather than read off a constant.
    let radius_ceiling = app().max_radius();
    for (target, low, high) in [
        (SizingTarget::Radius, MIN_RADIUS_MM, radius_ceiling),
        (SizingTarget::Strength, MIN_STRENGTH, MAX_STRENGTH),
    ] {
        for direction in [-1.0f32, 1.0] {
            let mut app = app();
            app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
            update(&mut app, Message::SizingStarted(target));
            app.on_pointer(PointerEvent::Moved {
                position: Vector::new(SIZE.x / 2.0 + direction * 5000.0, SIZE.y / 2.0),
                size: SIZE,
            });
            let value = match target {
                SizingTarget::Radius => app.brush.radius,
                SizingTarget::Strength => app.brush.strength,
            };
            assert!((low..=high).contains(&value), "{target:?} left its range at {value}");
        }
    }
}

///
/// One shared number made Move look broken: for Move, strength is the
/// fraction of the drag the surface follows, so Draw's 0.15 meant the form
/// crawled at a seventh of the pointer.
#[test]
fn each_brush_remembers_its_own_strength() {
    let mut app = app();
    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    assert!((app.brush.strength - 0.15).abs() < 1.0e-6, "draw: {}", app.brush.strength);

    update(&mut app, Message::BrushKindChanged(BrushKind::Move));
    assert!(
        app.brush.strength > 0.9,
        "a grab tool at {} would follow a seventh of the drag",
        app.brush.strength
    );

    // A deliberate change sticks, and survives a round trip through another
    // brush.
    update(&mut app, Message::BrushStrengthChanged(0.5));
    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    assert!((app.brush.strength - 0.15).abs() < 1.0e-6, "draw was overwritten");
    update(&mut app, Message::BrushKindChanged(BrushKind::Move));
    assert!((app.brush.strength - 0.5).abs() < 1.0e-6, "move forgot its setting");
}

/// A Move drag must follow the pointer across the SCREEN, not crawl around
/// the form.
///
/// Raycasting the pointer onto the surface gives a target that slides along
/// the curve, so the vector from the grab point stays short and keeps
/// turning. Dragging past the silhouette is the case that separates the two:
/// there is no surface under the cursor at all there, and a raycast version
/// simply stops.
#[test]
fn a_move_drag_keeps_working_past_the_edge_of_the_model() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    update(&mut app, Message::BrushKindChanged(BrushKind::Move));
    update(&mut app, Message::BrushRadiusChanged(10.0));

    let probe = Vec3::new(0.0, 0.0, 30.0);
    let before = app.doc.active_volume().sample_world(probe);

    press(&mut app, centre_of_viewport());
    // Well past the silhouette of the ball.
    for step in 1..=40 {
        app.on_pointer(PointerEvent::Moved {
            position: centre_of_viewport() + Vector::new(step as f32 * 12.0, 0.0),
            size: SIZE,
        });
    }
    release(&mut app);

    assert_ne!(
        app.doc.active_volume().sample_world(probe),
        before,
        "the drag stopped at the silhouette, so it is still raycasting the surface"
    );
    assert!(app.history_stats.undo_entries > 0, "a long drag recorded no undo entry");
}

/// Move through the APPLICATION, not through `Brush::apply`.
///
/// Every existing Move test builds a `Brush` and a `Stamp` by hand, which
/// tests the arithmetic and says nothing about whether a drag in the
/// viewport ever reaches it.
#[test]
fn dragging_with_move_selected_actually_changes_the_model() {
    let mut app = app();
    update(&mut app, Message::BrushKindChanged(BrushKind::Move));
    update(&mut app, Message::BrushStrengthChanged(1.0));
    update(&mut app, Message::BrushRadiusChanged(10.0));

    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    let probe =
        app.surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0)).expect("the centre hits the ball");
    let before = app.doc.active_volume().sample_world(probe);

    press(&mut app, centre_of_viewport());
    for step in 1..=12 {
        app.on_pointer(PointerEvent::Moved {
            position: centre_of_viewport() + Vector::new(step as f32 * 4.0, 0.0),
            size: SIZE,
        });
    }
    release(&mut app);

    assert!(
        app.history_stats.undo_entries > 0,
        "a Move drag recorded no undo entry, so it did nothing at all"
    );
    assert_ne!(
        app.doc.active_volume().sample_world(probe),
        before,
        "a Move drag across the model left the field untouched"
    );
}

/// Reach and cost together, since Move trades one against the other.
///
/// Printed rather than asserted: these are the numbers the reach cap was
/// chosen against, and the point is that a future change can re-run them.
/// `the_surface_follows_the_pointer_through_the_application` is the one
/// that fails if the reach collapses again.
#[test]
fn measure_move_reach_and_cost() {
    for (radius, strength) in
        [(3.0f32, 0.15f32), (3.0, 1.0), (10.0, 0.15), (10.0, 1.0), (20.0, 0.15), (20.0, 1.0)]
    {
        let mut app = app();
        aim_at_the_front(&mut app);
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        update(&mut app, Message::BrushRadiusChanged(radius.min(6.0)));
        update(&mut app, Message::BrushStrengthChanged(0.8));
        for _ in 0..6 {
            press(&mut app, centre_of_viewport());
            release(&mut app);
        }

        update(&mut app, Message::BrushKindChanged(BrushKind::Move));
        update(&mut app, Message::BrushRadiusChanged(radius));
        update(&mut app, Message::BrushStrengthChanged(strength));

        let before = bump_x(&app);
        app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
        let from = app.surface_under(pixel(centre_of_viewport())).expect("the centre hits");

        press(&mut app, centre_of_viewport());
        let mut worst = 0.0f32;
        let mut total_ms = 0.0f32;
        let mut to = from;
        for step in 1..=30 {
            let at = centre_of_viewport() + Vector::new(step as f32 * 5.0, 0.0);
            app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
            worst = worst.max(app.perf.edit_ms);
            total_ms += app.perf.edit_ms;
            if let Some(point) = app.surface_under(pixel(at)) {
                to = point;
            }
        }
        release(&mut app);

        let moved = bump_x(&app) - before;
        eprintln!(
            "radius {radius:>5.1} strength {strength:>4.2}: pointer travelled \
             {:>5.1} mm, surface followed {moved:>+6.2} mm (cap {:>5.2} mm), \
             worst event {worst:>6.2} ms, whole drag {total_ms:>7.2} ms",
            from.distance(to),
            Brush { kind: BrushKind::Move, radius, ..app.brush }.max_drag(),
        );
    }
}

/// Where the bump raised at the front of the ball sits along X, weighted by
/// how much material is standing proud of the sphere at each slice.
///
/// A drag has to carry this along with it, and it is measured rather than
/// assumed to have stayed put -- which is the whole difference between
/// "something changed" and "the surface followed the pointer".
fn bump_x(app: &Hani) -> f32 {
    let mut weighted = 0.0;
    let mut total = 0.0;
    for step in -120..=120 {
        let x = step as f32 * 0.25;
        // Where the surface crosses along +Z at this slice.
        let mut z = 60.0f32;
        while z > 0.0 {
            if app.doc.active_volume().sample_world(Vec3::new(x, 0.0, z)) < 0.0 {
                break;
            }
            z -= 0.05;
        }
        let raised = (z - (MODEL_RADIUS_MM * MODEL_RADIUS_MM - x * x).max(0.0).sqrt()).max(0.0);
        weighted += x * raised;
        total += raised;
    }
    assert!(total > 0.0, "there is no bump to measure");
    weighted / total
}

fn pixel(at: Vector) -> Vec2 {
    Vec2::new(at.x, at.y)
}

/// Look straight down -Z, so a horizontal drag on screen is a drag along
/// world X and the measurements above are in a frame anyone can check.
fn aim_at_the_front(app: &mut Hani) {
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
}

/// The reach the old incremental Move could not deliver.
///
/// It moved the surface **0.02 mm** for a full viewport drag at the default
/// radius, which is why it was reported as doing nothing at all. This
/// asserts a real magnitude rather than "something changed", because
/// "something changed" is exactly what the broken version also did.
#[test]
fn the_surface_follows_the_pointer_through_the_application() {
    let mut app = app();
    aim_at_the_front(&mut app);
    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    update(&mut app, Message::BrushRadiusChanged(6.0));
    update(&mut app, Message::BrushStrengthChanged(0.8));
    for _ in 0..6 {
        press(&mut app, centre_of_viewport());
        release(&mut app);
    }

    update(&mut app, Message::BrushKindChanged(BrushKind::Move));
    update(&mut app, Message::BrushRadiusChanged(10.0));
    update(&mut app, Message::BrushStrengthChanged(1.0));

    let before = bump_x(&app);
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    let from = app.surface_under(pixel(centre_of_viewport())).expect("the centre hits");

    press(&mut app, centre_of_viewport());
    let mut to = from;
    for step in 1..=10 {
        let at = centre_of_viewport() + Vector::new(step as f32 * 5.0, 0.0);
        app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
        if let Some(point) = app.surface_under(pixel(at)) {
            to = point;
        }
    }
    release(&mut app);

    let pointer = from.distance(to);
    let moved = bump_x(&app) - before;
    assert!(pointer > 3.0, "the test did not drag far enough to mean anything: {pointer} mm");
    assert!(
        moved > pointer * 0.4,
        "a {pointer:.1} mm drag moved the surface {moved:.2} mm, which is the order of \
         magnitude the incremental version failed at"
    );
}

/// The property locking the field buys, seen from the application.
#[test]
fn a_drag_out_and_back_through_the_application_returns_the_form() {
    let mut app = app();
    aim_at_the_front(&mut app);
    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    update(&mut app, Message::BrushRadiusChanged(6.0));
    update(&mut app, Message::BrushStrengthChanged(0.8));
    for _ in 0..6 {
        press(&mut app, centre_of_viewport());
        release(&mut app);
    }

    let probes: Vec<Vec3> =
        (-40..=40).map(|step| Vec3::new(step as f32 * 0.5, 0.0, MODEL_RADIUS_MM - 2.0)).collect();
    let before: Vec<f32> =
        probes.iter().map(|p| app.doc.active_volume().sample_world(*p)).collect();

    update(&mut app, Message::BrushKindChanged(BrushKind::Move));
    update(&mut app, Message::BrushRadiusChanged(10.0));
    update(&mut app, Message::BrushStrengthChanged(1.0));

    press(&mut app, centre_of_viewport());
    let mut path: Vec<f32> = (1..=8).map(|step| step as f32 * 4.0).collect();
    path.extend((0..=8).rev().map(|step| step as f32 * 4.0));
    for offset in path {
        app.on_pointer(PointerEvent::Moved {
            position: centre_of_viewport() + Vector::new(offset, 0.0),
            size: SIZE,
        });
    }
    release(&mut app);

    let worst = probes
        .iter()
        .zip(&before)
        .map(|(probe, was)| (app.doc.active_volume().sample_world(*probe) - was).abs())
        .fold(0.0f32, f32::max);
    // Looser than the core test's thousandth, and for two reasons that are
    // the application's rather than the algorithm's. The world point comes
    // from a raycast against the surface as it currently stands, so coming
    // back to the same pixel is not quite coming back to the same point;
    // and a drag that has shrunk to within a quarter voxel of where it
    // already was is skipped rather than redone. Both are bounded by a
    // fraction of a voxel, which is what this asserts -- the values are in
    // voxels, so a twentieth here is a hundredth of a voxel.
    assert!(worst < 0.05, "a drag out and back left the surface {worst} different");
}

/// Two bodies, one in front of the other, both drawn.
///
/// The second ball is placed on the line from the origin to the eye, so a
/// press at the centre of the viewport meets it before the starting sphere.
/// `publish_visibility` is called by hand because the fixture builds the
/// document directly rather than through a message, and the pick reads the
/// mask the application publishes.
fn two_bodies_in_a_line(app: &mut Hani) -> (NodeId, NodeId) {
    let front = app.camera.eye().normalize() * 55.0;
    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(front, 8.0);
    second.mark_everything_dirty();
    let other = app.doc.add_body("Body 2", second);
    app.publish_visibility();
    app.remesh_dirty();
    (app.doc.active(), other)
}

/// **The press ordering, which is the one that loses work if it is wrong.**
///
/// Recording used to open in the `Pressed` arm before anything had been
/// raycast. With two bodies that opens it on the wrong one, and
/// `record_for_undo` does nothing at all when the volume it is called on
/// has no recorder — so the carve lands, no entry is pushed, `unsaved` stays
/// false, and quitting does not even raise the discard prompt.
///
/// The rule that falls out is the one Photoshop's layer list has: a press
/// on something that is not selected selects it, and the press after that
/// works on it.
#[test]
fn a_press_over_another_body_selects_it_and_carves_nothing_until_the_next_press() {
    let mut app = app();
    let (first, other) = two_bodies_in_a_line(&mut app);

    // The surfaces the camera meets first: the starting ball at its own
    // radius, and the second ball at its centre plus its radius, because
    // the ray comes inwards from the eye.
    let front_of_first = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let front_of_other = app.camera.eye().normalize() * 63.0;
    let first_before = app.doc.volume(first).expect("a live body").sample_world(front_of_first);
    let other_before = app.doc.volume(other).expect("a live body").sample_world(front_of_other);

    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(app.doc.active(), other, "the press did not choose the body it landed on");
    assert!(!app.history.can_undo(), "a press that only selected recorded a stroke");
    assert!(!app.unsaved, "a press that only selected dirtied the document");
    assert_eq!(
        app.doc.volume(first).expect("a live body").sample_world(front_of_first),
        first_before,
        "the press carved the body it was leaving"
    );
    assert_eq!(
        app.doc.volume(other).expect("a live body").sample_world(front_of_other),
        other_before,
        "the press that selected also carved"
    );

    // And now it sculpts, on the body it chose.
    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(app.doc.active(), other, "the second press moved the selection again");
    assert!(app.history.can_undo(), "the second press recorded no undo entry");
    assert!(app.unsaved, "the second press left the document looking saved");
    assert_ne!(
        app.doc.volume(other).expect("a live body").sample_world(front_of_other),
        other_before,
        "the second press did not carve the selected body"
    );
    assert_eq!(
        app.doc.volume(first).expect("a live body").sample_world(front_of_first),
        first_before,
        "the stroke landed on the body that was NOT selected"
    );
}

/// Hiding is a draw-time skip, so the depth buffer where a hidden body sits
/// is empty. A press there must go through it: carving something invisible
/// would set `unsaved`, push an entry, pay a remesh and an upload, and
/// change not one pixel.
#[test]
fn a_press_over_a_hidden_body_carves_nothing_and_reaches_what_is_behind_it() {
    let mut app = app();
    let (first, other) = two_bodies_in_a_line(&mut app);

    let mut meta = app.doc.meta(other).expect("the second body");
    meta.visible = false;
    app.doc.set_meta(&meta);
    app.publish_visibility();

    let front_of_first = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let front_of_other = app.camera.eye().normalize() * 63.0;
    let other_before = app.doc.volume(other).expect("a live body").sample_world(front_of_other);
    let first_before = app.doc.volume(first).expect("a live body").sample_world(front_of_first);

    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(app.doc.active(), first, "a hidden body was selected by a press over it");
    assert_eq!(
        app.doc.volume(other).expect("a live body").sample_world(front_of_other),
        other_before,
        "the press carved a body that is not on screen"
    );
    assert_ne!(
        app.doc.volume(first).expect("a live body").sample_world(front_of_first),
        first_before,
        "the press stopped at a body nothing is drawing"
    );
}

/// The same rule, for the case the pick cannot answer on its own.
///
/// [`Document::pick`] masks on visibility, so it refuses a hidden body --
/// but a stroke takes its surface from `pick_body`, which asks one named
/// body and never consults the eye. So a press while the ACTIVE body is
/// hidden picked nothing, fell through to a sculpt, and carved the
/// invisible body: an undo entry pushed, `unsaved` set, a remesh and an
/// upload paid for, and not one pixel changed. Reachable today only by
/// opening a file whose active row is hidden; a one-click bug the moment
/// the panel ships an eye.
#[test]
fn a_press_while_the_active_body_is_hidden_carves_nothing_and_says_which_body() {
    let mut app = app();
    let active = app.doc.active();
    let mut meta = app.doc.meta(active).expect("the starting body");
    meta.visible = false;
    app.doc.set_meta(&meta);
    app.publish_visibility();

    let front = app.camera.eye().normalize() * MODEL_RADIUS_MM;
    let before = app.doc.active_volume().sample_world(front);

    press(&mut app, centre_of_viewport());
    app.on_pointer(PointerEvent::Moved {
        position: centre_of_viewport() + Vector::new(20.0, 0.0),
        size: SIZE,
    });
    release(&mut app);

    assert_eq!(
        app.doc.active_volume().sample_world(front),
        before,
        "the press carved the body that is not on screen"
    );
    assert!(!app.history.can_undo(), "a press that carved nothing recorded a stroke");
    assert!(!app.unsaved, "a press that carved nothing dirtied the document");
    assert!(
        app.status.contains("hidden") && app.status.contains(Document::FIRST_BODY_NAME),
        "nothing said why the press did nothing: {:?}",
        app.status
    );

    // Shown again, the very same press works: the refusal is about the
    // eye and nothing else.
    meta.visible = true;
    app.doc.set_meta(&meta);
    app.publish_visibility();
    press(&mut app, centre_of_viewport());
    release(&mut app);
    assert_ne!(
        app.doc.active_volume().sample_world(front),
        before,
        "the press stopped working on a body that is drawn"
    );
}

/// **A live stroke owns the ring.**
///
/// The `Moved` arm carved the active body and then picked across every
/// drawn body anyway, so a stroke dragged over a second body moved
/// `hover_body` onto that second body -- and the ring is built from the
/// hovered body's field and coloured `CursorMood::Selecting` whenever the
/// hovered body is not the active one. The cursor said "a press here would
/// select", on the wrong surface, during a press that was carving
/// something else.
#[test]
fn the_ring_stays_on_the_body_a_live_stroke_is_carving() {
    let mut app = app();
    let (first, other) = two_bodies_in_a_line(&mut app);

    // The control. With no button down the centre of the viewport hovers
    // the SECOND body, which is the whole reason a stroke dragged there
    // could move the ring; without this the test would also pass on a
    // build where the hover never reaches that body at all.
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    assert_eq!(app.hover_body, Some(other), "the fixture never hovers the second body");

    // Off to the side, where only the first body is, so the press starts a
    // stroke instead of choosing the other body.
    let beside = Vector::new(610.0, 300.0);
    press(&mut app, beside);
    assert_eq!(app.doc.active(), first, "the press beside the second ball chose a body");
    assert!(
        matches!(app.drag.map(|drag| drag.kind), Some(DragKind::Sculpt(_))),
        "the press beside the second ball did not start a stroke: {:?}",
        app.drag.map(|drag| drag.kind)
    );

    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    assert_eq!(
        app.hover_body,
        Some(first),
        "mid-stroke the ring hopped onto a body the stroke is not carving"
    );
    assert!(app.hover.is_some(), "the ring vanished from the body being carved");

    // And it comes back the moment the stroke ends.
    release(&mut app);
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    assert_eq!(app.hover_body, Some(other), "the hover never returned to the pick");
}

/// **The mitigation for the mirror centre being the lattice origin.**
///
/// With the centre pinned there, mirroring a body that sits wholly to one
/// side of a plane writes its twin out in empty space: a free-floating
/// shell that exports and that no slicer can print. The axis is refused
/// rather than enabled-and-ignored, so there is one answer to "is X on".
#[test]
fn a_mirror_the_active_body_does_not_straddle_is_refused_with_a_reason() {
    let mut app = app();

    // The starting ball is centred on the origin, so every plane crosses it
    // and every axis is allowed. This half is what stops the refusal from
    // being "always refuse".
    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
    assert!(
        app.symmetry_settings().axis(MirrorAxis::X),
        "a body on the origin was refused its mirror"
    );
    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
    assert!(!app.symmetry_settings().axis(MirrorAxis::X));

    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
    second.mark_everything_dirty();
    let other = app.doc.add_body("Body 2", second);
    app.publish_visibility();
    app.remesh_dirty();
    app.select_body(other);

    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
    assert!(
        !app.symmetry_settings().axis(MirrorAxis::X),
        "a mirror was enabled that misses the body"
    );
    assert!(
        app.status.contains('X') && app.status.contains("Body 2"),
        "the refusal said nothing useful: {:?}",
        app.status
    );

    // The other two planes DO cross it, and must not be refused along with
    // the one that does not.
    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::Y));
    assert!(app.symmetry_settings().axis(MirrorAxis::Y), "a plane the body straddles was refused");
}

/// **A refusal wired into one event site is not a refusal.**
///
/// The symmetry strip is not the only way to turn X mirroring on: a
/// SpaceMouse button bound to `ToggleSymmetry` did it with a bare
/// `Symmetry::toggled`, so a puck user on a body at x = +80 got every
/// stroke's twin written into empty space, with the strip highlight and
/// the drawn plane both agreeing that nothing was wrong. This drives the
/// application's real button path rather than calling the gate, because
/// the gate was never what was broken.
#[test]
fn the_spacemouse_symmetry_button_is_refused_the_mirror_the_strip_is_refused() {
    let mut app = app();
    let first = app.doc.active();
    app.spacemouse.config.buttons[0] = ButtonAction::ToggleSymmetry;

    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
    second.mark_everything_dirty();
    let other = app.doc.add_body("Body 2", second);
    app.publish_visibility();
    app.remesh_dirty();
    app.select_body(other);

    app.spacemouse.simulate_press(0);
    app.drive_from_spacemouse(16.0);

    assert!(
        !app.symmetry_settings().axis(MirrorAxis::X),
        "the puck turned on a mirror plane the body sits entirely to one side of"
    );
    assert!(
        app.status.contains('X') && app.status.contains("Body 2"),
        "the puck's refusal said nothing: {:?}",
        app.status
    );

    // And it still works where the mirror is legitimate, so the fix is a
    // gate and not a disconnected wire.
    app.select_body(first);
    app.spacemouse.simulate_press(0);
    app.drive_from_spacemouse(16.0);
    assert!(
        app.symmetry_settings().axis(MirrorAxis::X),
        "the puck stopped turning on a mirror the body does straddle"
    );
}

/// A view out of a file is a third way for a mirror to come on, and a
/// restored one carves twins into empty space exactly as an enabled one
/// does -- the file may have been written by a build without the refusal,
/// or with a different body active.
#[test]
fn a_mirror_restored_from_a_view_the_body_does_not_straddle_is_turned_off() {
    let mut app = app();
    let mut off_centre = Volume::new(app.doc.voxel_size());
    off_centre.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
    off_centre.mark_everything_dirty();
    app.doc = Document::from_volume(off_centre);
    app.publish_visibility();
    app.remesh_dirty();

    let mut view = app.current_view();
    view.mirror = [true, true, true];
    app.apply_view(&view);

    assert!(
        !app.symmetry_settings().axis(MirrorAxis::X),
        "a file turned on a mirror the body misses"
    );
    assert!(
        app.symmetry_settings().axis(MirrorAxis::Y) && app.symmetry_settings().axis(MirrorAxis::Z),
        "the two planes the body does straddle were dropped as well"
    );
}

/// Each body has its own planes (`app/symmetry_frame.rs`): choosing a
/// body away from the first shows that body's, which are off, and the
/// first keeps its own. Until each body had its own settings the plane
/// was the session's, and choosing a body it missed turned it off and
/// said so.
#[test]
fn selecting_another_body_shows_its_own_planes_and_the_first_keeps_its() {
    let mut app = app();
    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
    assert!(app.symmetry_settings().axis(MirrorAxis::X));

    let mut second = Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(80.0, 0.0, 0.0), 10.0);
    second.mark_everything_dirty();
    let other = app.doc.add_body("Body 2", second);
    app.publish_visibility();
    app.remesh_dirty();

    let first = app.doc.active();
    app.select_body(other);
    assert!(
        !app.symmetry_settings().axis(MirrorAxis::X),
        "the first body's mirror showed on the second"
    );
    assert!(app.doc.symmetry(first).axis(MirrorAxis::X), "the first body lost its mirror");
    app.select_body(first);
    assert!(app.symmetry_settings().axis(MirrorAxis::X), "the first body's mirror is not shown");
}

#[test]
fn a_dynamic_radius_keeps_its_proportion_to_the_model() {
    let proportion = |state: &Hani| state.brush.radius / state.model_radius;

    // Off by default: the radius means a physical size, so a resample must
    // leave it alone.
    let mut fixed = app();
    let before = fixed.brush.radius;
    update(&mut fixed, Message::Resample(VOXEL_SIZE_MM / 2.0));
    assert_eq!(fixed.brush.radius, before, "a fixed radius must survive a resample");

    // On, and the model doubles: the brush should follow it.
    let mut dynamic = app();
    update(&mut dynamic, Message::DynamicRadiusToggled(true));
    let before = proportion(&dynamic);
    dynamic.doc.active_volume_mut().seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM * 2.0);
    dynamic.doc.active_volume_mut().mark_everything_dirty();
    dynamic.remesh_dirty();

    assert!(
        dynamic.brush.radius > fixed.brush.radius,
        "a dynamic radius should have grown with the model"
    );
    assert!(
        (proportion(&dynamic) - before).abs() < 0.15 * before,
        "the brush lost its proportion: {before} became {}",
        proportion(&dynamic)
    );
}

/// The same ball, moved bodily away from the origin, must report the same
/// size.
///
/// This is what `content_radius` replaced `bounding_radius` for. The old
/// measure was taken from the WORLD ORIGIN, so a model 128 mm out reported
/// roughly 2.6 times its own radius and the Dynamic brush went with it --
/// and nothing in the interface would have said why the brush had grown.
///
/// 128 mm rather than a round 100 because at the 0.25 mm voxel that is
/// exactly sixteen bricks, so the sphere's brick footprint is an exact
/// translation of the one at the origin and the two radii are identical to
/// the last bit. That is the shared-lattice property this whole design
/// rests on, exercised from the application's side.
#[test]
fn a_body_far_from_the_origin_does_not_resize_the_dynamic_brush() {
    let mut app = app();
    update(&mut app, Message::DynamicRadiusToggled(true));
    let radius = app.brush.radius;
    let at_origin = app.model_radius;

    let mut moved = hani_core::Volume::new(app.doc.voxel_size());
    moved.seed_sphere(Vec3::new(128.0, 0.0, 0.0), MODEL_RADIUS_MM);
    moved.mark_everything_dirty();
    app.doc.replace_active_volume(moved);
    app.remesh_dirty();

    assert_eq!(
        app.model_radius, at_origin,
        "the same ball measured {} at the origin and {} at 128 mm",
        at_origin, app.model_radius
    );
    assert_eq!(app.brush.radius, radius, "moving the model resized the brush");
}

/// Selecting a different body changes no geometry, so it must not resize
/// the Dynamic brush -- not on the selection, and not on the next remesh
/// either, which is where a naive implementation would catch up on it a
/// frame later.
///
/// The fixture is the one from the decision: a small body beside a large
/// one. Under a plain `previous == self.model_radius` comparison this took
/// a 3 mm brush to the 0.25 mm floor and then to 10 mm on the way back --
/// and the press that resizes it is the same press that selects, so the
/// next press would carve at a radius nobody chose.
#[test]
fn changing_the_active_body_does_not_resize_the_dynamic_brush() {
    let mut app = app();
    update(&mut app, Message::DynamicRadiusToggled(true));
    let radius = app.brush.radius;

    let mut rivet = hani_core::Volume::new(app.doc.voxel_size());
    rivet.seed_sphere(Vec3::new(128.0, 0.0, 0.0), 2.5);
    rivet.mark_everything_dirty();
    let second = app.doc.add_body("Body 2", rivet);
    app.remesh_dirty();
    assert_eq!(app.brush.radius, radius, "ADDING a body resized the brush");

    app.doc.set_active(second);
    // Something then dirties a brick -- a stroke, an undo, a resample,
    // anything -- and the remesh that follows measures the rivet where the
    // last measurement was of the ball. That comparison is the whole
    // hazard, so the test has to reach it rather than land on
    // `remesh_dirty`'s empty-set early return.
    app.doc.active_volume_mut().mark_everything_dirty();
    app.remesh_dirty();
    assert_eq!(app.brush.radius, radius, "selecting a smaller body resized the brush");
    assert!(
        app.model_radius < MODEL_RADIUS_MM,
        "the framing radius did not follow the selection: {}",
        app.model_radius
    );

    let first = app.doc.nodes()[0].id;
    app.doc.set_active(first);
    app.doc.active_volume_mut().mark_everything_dirty();
    app.remesh_dirty();
    assert_eq!(app.brush.radius, radius, "selecting back resized the brush");
}

/// A number typed in the menu, as in the panel and the bar: the value
/// moves on enter and not before, held to the slider's range, and junk
/// leaves it alone. The menu's own fields applied every keystroke that
/// parsed; they are the kit's `ui::number` now (`docs/WORKSPACES.md`).
#[test]
fn a_typed_radius_arrives_on_enter_and_still_clamps() {
    let mut app = app();
    let set = |app: &mut Hani, carried: Message| {
        update(app, Message::FieldSet("brush.radius", Box::new(carried)));
    };
    for typed in ["", "2", "2.", "2.5"] {
        update(&mut app, Message::FieldEdited("brush.radius", typed.to_string()));
    }
    assert_ne!(app.brush.radius, 2.5, "typing alone moved the value");
    set(&mut app, Message::BrushRadiusChanged(2.5));
    assert!((app.brush.radius - 2.5).abs() < 1.0e-5, "the value never arrived");
    assert!(app.drafts.is_empty());
    // Out of range is held to the slider's range by the field, which
    // is what sends it.
    let submitted = |app: &Hani, key: &'static str, typed: &str| {
        let mut drafts = crate::ui::Drafts::new();
        drafts.insert(key, typed.to_string());
        match key {
            "brush.radius" => crate::ui::number(
                &drafts,
                key,
                "Radius",
                app.brush.radius,
                MIN_RADIUS_MM..=app.max_radius(),
                Message::BrushRadiusChanged,
            )
            .submitted(),
            _ => crate::ui::number(
                &drafts,
                key,
                "Strength",
                app.brush.strength,
                MIN_STRENGTH..=app.max_strength(),
                Message::BrushStrengthChanged,
            )
            .submitted(),
        }
    };
    let message = submitted(&app, "brush.radius", "500");
    update(&mut app, message);
    assert_eq!(app.brush.radius, app.max_radius());
    let message = submitted(&app, "brush.strength", "-5");
    update(&mut app, message);
    assert_eq!(app.brush.strength, MIN_STRENGTH);
    // And junk leaves the value where it was.
    let message = submitted(&app, "brush.radius", "banana");
    update(&mut app, message);
    assert_eq!(app.brush.radius, app.max_radius());
}

#[test]
fn the_cursor_ring_appears_where_the_pointer_meets_the_model() {
    let mut app = app();
    assert!(app.hover.is_none(), "nothing has been pointed at yet");

    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    assert!(app.hover.is_some(), "the centre of the viewport should hit the sphere");
    // The geometry lives in the shared frame, not in `self.overlay`: the
    // application swaps its buffer over to the renderer and keeps the empty
    // one back, which is what stops either side allocating.
    let drawn = app.shared.overlay_snapshot();
    assert!(!drawn.lines.is_empty(), "no ring was handed to the renderer");

    // A corner of the viewport misses the sphere, so there is nothing to
    // draw and that absence is the signal a press would do nothing.
    app.on_pointer(PointerEvent::Moved { position: Vector::new(2.0, 2.0), size: SIZE });
    assert!(app.hover.is_none(), "a corner should miss the model");
    assert!(
        app.shared.overlay_snapshot().lines.is_empty(),
        "a ring was drawn for a pointer that is off the model"
    );
}

/// Undo has to walk the brush ring back onto the surface it restored.
///
/// The ring is not a screen space circle drawn over the model: every one
/// of its points is pushed onto the field by `cursor::onto_surface`, so
/// the geometry the renderer is holding is only correct for the field it
/// was built from. Undo changes the field without the pointer moving,
/// which is the one way the two can come apart -- and before
/// [`Hani::after_history_step`] refreshed the overlay, ctrl+Z left the
/// ring standing on the surface the stroke had made, floating off the
/// model until some later pointer motion happened to rebuild it.
///
/// **Measured rather than argued.** With the default sphere and a
/// full strength Draw at the centre of the viewport, the stale ring sits
/// 0.49 voxels off the restored field and a refreshed one sits 0.0016
/// voxels off it, against the 0.1 voxel tolerance `onto_surface` stops
/// walking at -- five times the tolerance apart in one direction and
/// sixty times inside it in the other. The first assertion is there
/// because a fixture whose stroke did not actually move the surface under
/// the ring would pass the second one while testing nothing.
///
/// Deleting the `refresh_overlay` call from `after_history_step` fails
/// this test and, as of writing, nothing else in the workspace.
#[test]
fn undoing_a_stroke_walks_the_brush_ring_back_onto_the_surface_it_restores() {
    // What `cursor::onto_surface` treats as "on the surface", in voxels.
    const ON_SURFACE: f32 = 0.1;

    let ring = |app: &Hani| -> Vec<Vec3> {
        app.shared
            .overlay_snapshot()
            .lines
            .iter()
            .map(|vertex| Vec3::from_array(vertex.position))
            .collect()
    };
    let worst_off_surface = |app: &Hani, points: &[Vec3]| -> f32 {
        points
            .iter()
            .map(|point| app.doc.active_volume().sample_world(*point).abs())
            .fold(0.0f32, f32::max)
    };

    let mut app = app();
    app.on_pointer(PointerEvent::Moved { position: centre_of_viewport(), size: SIZE });
    assert!(app.hover.is_some(), "the centre of the viewport should hit the sphere");

    // Deep enough to move the surface the ring is standing on. The pointer
    // does not move again after this, so nothing but undo can rebuild the
    // overlay.
    app.brush.strength = 1.0;
    press(&mut app, centre_of_viewport());
    release(&mut app);

    let stale = ring(&app);
    assert!(!stale.is_empty(), "no ring was handed to the renderer");

    update(&mut app, Message::Undo);

    let stale_error = worst_off_surface(&app, &stale);
    assert!(
        stale_error > ON_SURFACE,
        "the fixture asserts nothing: the stroke left the ring only {stale_error} voxels \
         off the surface undo restored, which a stale ring would pass"
    );

    let error = worst_off_surface(&app, &ring(&app));
    assert!(
        error <= ON_SURFACE,
        "undo left the brush ring {error} voxels off the surface it restored, so the \
         overlay was never rebuilt"
    );
}

#[test]
fn the_radius_keys_stay_inside_the_range_the_slider_offers() {
    let mut app = app();
    for _ in 0..200 {
        update(&mut app, Message::BrushRadiusScaled(1.5));
    }
    assert_eq!(app.brush.radius, MAX_RADIUS_MM);
    for _ in 0..200 {
        update(&mut app, Message::BrushRadiusScaled(1.0 / 1.5));
    }
    assert_eq!(app.brush.radius, MIN_RADIUS_MM);
}

/// ZBrush and Nomad both invert on alt; this had control first. Both work,
/// and holding both is still one inversion rather than two.
#[test]
fn either_modifier_inverts_and_holding_both_is_not_a_double_negative() {
    let mut app = app();
    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    assert_eq!(app.stroke_direction(), BrushDirection::Add);

    let modifiers = |app: &mut Hani, control, alt| {
        app.on_pointer(PointerEvent::Modifiers { shift: false, control, alt });
    };

    modifiers(&mut app, true, false);
    assert_eq!(app.stroke_direction(), BrushDirection::Subtract, "control should invert");

    modifiers(&mut app, false, true);
    assert_eq!(app.stroke_direction(), BrushDirection::Subtract, "alt should invert");

    modifiers(&mut app, true, true);
    assert_eq!(
        app.stroke_direction(),
        BrushDirection::Subtract,
        "both held is one inversion, not two"
    );

    modifiers(&mut app, false, false);
    assert_eq!(app.stroke_direction(), BrushDirection::Add, "releasing should restore");
}

/// The eraser end still combines with a modifier the way it always did:
/// inverting an inverted stroke gives back the additive brush.
#[test]
fn alt_combines_with_the_eraser_the_way_control_does() {
    for (control, alt) in [(true, false), (false, true)] {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
        app.tablet.simulate(pen(glam::Vec2::ZERO, true));
        assert_eq!(app.stroke_direction(), BrushDirection::Subtract, "the eraser alone");

        app.on_pointer(PointerEvent::Modifiers { shift: false, control, alt });
        assert_eq!(
            app.stroke_direction(),
            BrushDirection::Add,
            "a modifier over the eraser should give back the additive brush"
        );
    }
}

#[test]
fn control_does_not_invert_a_brush_that_has_no_opposite() {
    let mut app = app();
    app.control = true;

    update(&mut app, Message::BrushKindChanged(BrushKind::Draw));
    assert_eq!(app.stroke_direction(), BrushDirection::Subtract);

    update(&mut app, Message::BrushKindChanged(BrushKind::Smooth));
    assert_eq!(
        app.stroke_direction(),
        BrushDirection::Add,
        "smooth has no opposite, so inverting it should do nothing"
    );
}

fn pen(tilt: glam::Vec2, eraser: bool) -> PenState {
    PenState { in_proximity: true, pressure: 1.0, eraser, tilt }
}

#[test]
fn the_eraser_end_inverts_the_brush() {
    let mut app = app();
    assert_eq!(app.stroke_direction(), BrushDirection::Add);

    app.tablet.simulate(pen(glam::Vec2::ZERO, true));
    assert_eq!(app.stroke_direction(), BrushDirection::Subtract);

    // The modifier and the eraser combine rather than override, so holding
    // one while using the other gives back the additive brush.
    app.control = true;
    assert_eq!(app.stroke_direction(), BrushDirection::Add);
}

#[test]
fn the_eraser_does_nothing_to_a_brush_with_no_opposite() {
    let mut app = app();
    update(&mut app, Message::BrushKindChanged(BrushKind::Smooth));
    app.tablet.simulate(pen(glam::Vec2::ZERO, true));
    assert_eq!(app.stroke_direction(), BrushDirection::Add);
}

#[test]
fn a_pen_that_is_away_cannot_erase() {
    let app = app();
    app.tablet.simulate(PenState { eraser: true, ..PenState::NONE });
    assert_eq!(app.stroke_direction(), BrushDirection::Add);
}

#[test]
fn leaning_the_pen_produces_a_world_space_lean_along_the_camera_axes() {
    let mut app = app();
    assert_eq!(app.pen_lean(), Vec3::ZERO, "no pen means no lean");

    app.tablet.simulate(pen(glam::Vec2::new(1.0, 0.0), false));
    let lean = app.pen_lean();
    assert!(
        (lean.length() - MAX_TILT).abs() < 1.0e-4,
        "a fully tilted pen should lean by the maximum angle, got {}",
        lean.length()
    );
    assert!(
        lean.normalize().dot(app.camera.right()) > 0.999,
        "tilt on the x axis should lean along the camera's right axis"
    );

    // Tilt on the y axis leans down the screen, which is away from up.
    app.tablet.simulate(pen(glam::Vec2::new(0.0, 1.0), false));
    assert!(app.pen_lean().normalize().dot(app.camera.up()) < -0.999);

    update(&mut app, Message::TiltToggled(false));
    assert_eq!(app.pen_lean(), Vec3::ZERO, "turning tilt off must disable it entirely");
}

#[test]
fn leaning_the_pen_moves_where_the_clay_lands() {
    // The end to end statement of what tilt is for: the same stroke on the
    // same spot puts material somewhere else when the pen is leaned.
    //
    // Measured as the difference between the two sides of the stroke rather
    // than against an upright stroke. Leaning also reduces how far the
    // brush pushes outward, by the cosine of the angle, and that term is
    // larger than the sideways one at this scale. Comparing left against
    // right cancels it, because it applies to both equally.
    let sculpt = |tilt: glam::Vec2| {
        let mut app = app();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        app.brush.strength = 0.8;
        app.brush.radius = 6.0;
        app.tablet.simulate(pen(tilt, false));

        let hit = app
            .surface_under(Vec2::new(SIZE.x / 2.0, SIZE.y / 2.0))
            .expect("the centre of the view is on the model");
        let sideways = app.camera.right() * 4.0;

        // One stamp moves the surface by a fraction of a voxel, so the
        // stroke has to be laid down repeatedly to be measurable.
        for _ in 0..8 {
            press(&mut app, centre_of_viewport());
            release(&mut app);
        }
        (
            app.doc.active_volume().sample_world(hit + sideways),
            app.doc.active_volume().sample_world(hit - sideways),
        )
    };

    let (right_upright, left_upright) = sculpt(glam::Vec2::ZERO);
    assert!(
        (right_upright - left_upright).abs() < 0.02,
        "an upright pen should build up evenly on both sides: \
         {right_upright} against {left_upright}"
    );

    let (right_leaned, left_leaned) = sculpt(glam::Vec2::new(1.0, 0.0));
    assert!(
        right_leaned < left_leaned - 0.02,
        "leaning right should pile material to the right: \
         right {right_leaned}, left {left_leaned}"
    );

    let (right_other_way, left_other_way) = sculpt(glam::Vec2::new(-1.0, 0.0));
    assert!(
        left_other_way < right_other_way - 0.02,
        "leaning left should pile material to the left: \
         right {right_other_way}, left {left_other_way}"
    );
}

#[test]
fn every_brush_can_be_driven_from_the_interface_without_panicking() {
    // Cheap breadth: each brush goes through the whole application path
    // once, which is where a bad plane or a zero normal would surface.
    for kind in BrushKind::ALL {
        let mut app = app();
        update(&mut app, Message::BrushKindChanged(kind));
        update(&mut app, Message::BrushStrengthChanged(0.6));
        press(&mut app, centre_of_viewport());
        app.on_pointer(PointerEvent::Moved {
            position: centre_of_viewport() + Vector::new(30.0, 12.0),
            size: SIZE,
        });
        release(&mut app);
        assert_eq!(app.history_stats.undo_entries, 1, "{kind} recorded no undo entry");
    }
}
