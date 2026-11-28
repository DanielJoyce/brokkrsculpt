// SPDX-License-Identifier: AGPL-3.0-only

//! The cut through the application: the line, the lasso, a cut through,
//! the ctrl lasso's mask, parting with keys, and what a cut says. Moved out
//! of `app.rs`'s `mod tests` (issue 189).

use super::test_support::*;
use super::*;
use iced::Vector;

// --- the plane cut -------------------------------------------------------

/// Which side of the dragged line is removed, pinned by observation rather
/// than by reasoning about cross products on paper.
///
/// This is the one thing about the cut that cannot be got right by thinking
/// about it: the sign depends on the ray order, the handedness of the
/// camera basis, and whether the pixel-to-NDC step flips Y. Get it backwards
/// and the tool takes the half the user meant to keep -- which is
/// destructive, and only obvious after the fact.
/// **Alt turns the cut into a parting**: the same line, and the body
/// becomes two that register on each other instead of losing a side.
///
/// Driven through the pointer and not by calling the engine, because the
/// engine has its own tests and what can go wrong HERE is the wiring: the
/// modifier read at the wrong moment, the source's slots not released,
/// the tool left armed, the history given two entries or none.
#[test]
fn alt_parts_the_body_in_two_with_keys_and_one_undo_puts_it_back() {
    let mut app = app();
    // Parting is Print's cut (`docs/WORKSPACES.md`); in Sculpt a cut
    // only cuts.
    app.desk.now = crate::message::Workspace::Print;
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    let source = app.doc.active();
    let before = app.doc.active_volume().stats();
    let undo_before = app.history.stats().undo_entries;

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    app.alt = true;
    assert_eq!(app.live_tool_label(), "PART — KEEP BOTH", "the card does not say what alt does");
    let middle_y = SIZE.y / 2.0;
    press(&mut app, Vector::new(SIZE.x * 0.1, middle_y));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.9, middle_y),
        size: SIZE,
    });
    release(&mut app);
    app.alt = false;

    assert_ne!(app.tool, Tool::Cut, "the tool stayed armed after parting");
    assert!(app.status.starts_with("parted "), "{}", app.status);
    assert!(app.status.contains("key"), "the status line says nothing of keys: {}", app.status);
    assert!(app.unsaved, "a parting did not mark the document unsaved");
    assert_eq!(app.doc.body_count(), 2, "the body did not become two");
    assert!(app.doc.volume(source).is_none(), "the source is still in the document");
    assert_eq!(
        app.history.stats().undo_entries,
        undo_before + 1,
        "a parting should be exactly one undo entry"
    );

    // Both halves are real, printable, and on either side of the line.
    let mut visible = Vec::new();
    app.doc.saved_visibility(&mut visible);
    let bodies = app.doc.export_bodies(&visible);
    assert_eq!(bodies.len(), 2);
    for (meta, _, report) in &bodies {
        assert!(report.is_printable(), "{}: {}", meta.name, report.summary());
    }
    let above = Vec3::new(0.0, 12.0, 0.0);
    let below = Vec3::new(0.0, -12.0, 0.0);
    let holds = |at: Vec3| -> Vec<bool> {
        app.doc
            .nodes()
            .iter()
            .filter_map(|node| node.volume())
            .map(|volume| volume.sample_world(at) < 0.0)
            .collect()
    };
    assert_eq!(holds(above).iter().filter(|held| **held).count(), 1, "one part holds the top");
    assert_eq!(holds(below).iter().filter(|held| **held).count(), 1, "one part holds the bottom");
    assert_ne!(holds(above), holds(below), "one part holds both the top and the bottom");

    // One undo, and the body that was there is there again.
    update(&mut app, Message::Undo);
    assert_eq!(app.doc.body_count(), 1, "undoing a parting left {} bodies", app.doc.body_count());
    let after = app.doc.volume(source).expect("the source is back under its own id").stats();
    assert_eq!(after.dense_bricks, before.dense_bricks, "the body came back a different size");
}

/// The key settings reach the parting. Without this the controls could
/// move a number on the panel and change nothing that gets cut.
#[test]
fn the_key_settings_decide_what_a_parting_cuts() {
    use crate::message::KeySetting;
    use hani_core::part::KeyKind;

    let part = |app: &mut Hani| {
        // Parting is Print's cut (`docs/WORKSPACES.md`).
        app.desk.now = crate::message::Workspace::Print;
        app.camera.yaw = 0.0;
        app.camera.pitch = 0.0;
        app.publish_camera();
        update(app, Message::ToolChanged(Tool::Cut));
        // The line cut, not the ring. See `app/ring_cut.rs`.
        update(app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
        app.alt = true;
        let middle_y = SIZE.y / 2.0;
        press(app, Vector::new(SIZE.x * 0.1, middle_y));
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(SIZE.x * 0.9, middle_y),
            size: SIZE,
        });
        release(app);
        app.alt = false;
        assert!(app.status.starts_with("parted "), "{}", app.status);
    };
    // Solid volume of every body together, by counting voxels that read
    // inside.
    let solid = |app: &Hani| -> usize {
        app.doc
            .nodes()
            .iter()
            .filter_map(|node| node.volume())
            .map(|volume| {
                let (lo, hi) = volume.world_bounds().expect("a body has bricks");
                let (lo, hi) = volume.voxel_bounds(lo, hi);
                let mut inside = 0;
                for z in lo.z..=hi.z {
                    for y in lo.y..=hi.y {
                        for x in lo.x..=hi.x {
                            inside +=
                                usize::from(volume.sample_voxel(glam::IVec3::new(x, y, z)) < 0.0);
                        }
                    }
                }
                inside
            })
            .sum()
    };

    // A peg adds to one part what it takes from the other, less the
    // clearance. A pin hole only takes, from both.
    let mut pegged = app();
    part(&mut pegged);
    let mut pinned = app();
    update(&mut pinned, Message::KeySettingChanged(KeySetting::Kind(KeyKind::PinHole)));
    assert_eq!(pinned.key_spec.kind, KeyKind::PinHole, "the setting did not arrive");
    part(&mut pinned);
    assert!(
        solid(&pinned) + 50 < solid(&pegged),
        "pin holes left {} voxels solid and pegs {}: the kind did not reach the cut",
        solid(&pinned),
        solid(&pegged)
    );

    // And a setting from outside the sliders' range is clamped, not
    // passed on.
    update(&mut pinned, Message::KeySettingChanged(KeySetting::Radius(-3.0)));
    update(&mut pinned, Message::KeySettingChanged(KeySetting::Clearance(40.0)));
    assert_eq!(pinned.key_spec.radius_mm, 0.5);
    assert_eq!(pinned.key_spec.clearance_mm, 0.6);
}

/// A parting is one plane. A lasso with alt held parts nothing and says
/// why, where quietly cutting instead would take material the user asked
/// to keep.
#[test]
fn alt_over_a_lasso_parts_nothing_and_cuts_nothing() {
    let mut app = app();
    // Parting is Print's cut (`docs/WORKSPACES.md`); in Sculpt a cut
    // only cuts.
    app.desk.now = crate::message::Workspace::Print;
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    let before = app.doc.active_volume().stats();

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    app.alt = true;
    // A closed loop round the middle of the view.
    let (cx, cy) = (SIZE.x / 2.0, SIZE.y / 2.0);
    press(&mut app, Vector::new(cx + 80.0, cy));
    for step in 1..=24 {
        let angle = step as f32 / 24.0 * std::f32::consts::TAU;
        app.on_pointer(PointerEvent::Moved {
            position: Vector::new(cx + 80.0 * angle.cos(), cy + 80.0 * angle.sin()),
            size: SIZE,
        });
    }
    release(&mut app);
    app.alt = false;

    assert!(app.status.starts_with("not parted"), "{}", app.status);
    assert_eq!(app.doc.body_count(), 1);
    let after = app.doc.active_volume().stats();
    assert_eq!(after.dense_bricks, before.dense_bricks, "the lasso cut the body after all");
    assert_eq!(app.history.stats().undo_entries, 0, "a refused parting reached the history");
}

#[test]
fn a_left_to_right_drag_removes_a_consistent_side() {
    let mut app = app();
    // A known camera: looking straight down -Z at the origin.
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    let middle_y = SIZE.y / 2.0;
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    assert_eq!(app.tool, Tool::Cut, "the cut did not arm");

    // Drag left to right across the middle of the viewport.
    press(&mut app, Vector::new(SIZE.x * 0.1, middle_y));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.9, middle_y),
        size: SIZE,
    });
    release(&mut app);

    assert_ne!(app.tool, Tool::Cut, "the cut stayed armed after being used");
    assert!(app.unsaved, "a cut did not mark the document unsaved");

    let above = app.doc.active_volume().sample_world(Vec3::new(0.0, 12.0, 0.0));
    let below = app.doc.active_volume().sample_world(Vec3::new(0.0, -12.0, 0.0));
    assert_ne!(
        above < 0.0,
        below < 0.0,
        "the cut removed both halves or neither: above {above}, below {below}"
    );
    // Record which one it actually is, so a change of sign fails here
    // rather than in someone's sculpt. If this assertion is what fails
    // after a camera change, check the ray order in `finish_cut` before
    // editing the expectation.
    assert!(
        below < 0.0 && above > 0.0,
        "a left to right drag should keep the LOWER half on screen: \
         above {above}, below {below}"
    );
}

/// The pixel column whose view ray crosses the z = 0 plane at world `x`,
/// for a camera looking straight down -Z. A vertical drag there is a plane
/// that stands at that x where the model's equator is.
fn column_at_world_x(app: &Hani, x: f32) -> f32 {
    let x_at = |px: f32| {
        let (origin, ray) = app.ray_through(Vec2::new(px, SIZE.y / 2.0));
        origin.x + ray.x * (-origin.z / ray.z)
    };
    let (mut low, mut high) = (0.0_f32, SIZE.x);
    for _ in 0..40 {
        let middle = (low + high) / 2.0;
        if x_at(middle) < x {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

/// **The report this exists for**: with X mirroring on, a plane cut down
/// one side of the model took that side and left the other -- while every
/// stroke that session had landed on both. Now the cut takes both, as one
/// gesture and one undo entry.
#[test]
fn a_cut_with_x_mirror_on_takes_both_sides() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
    assert!(app.symmetry_settings().axis(MirrorAxis::X), "X did not turn on");

    // A plane standing at x = 10 on the equator, dragged top to bottom: the
    // left of the arrow is screen-right, so everything past x = 10 goes.
    let column = column_at_world_x(&app, 10.0);
    let sample = |app: &Hani, x: f32| app.doc.active_volume().sample_world(Vec3::new(x, 0.0, 0.0));
    assert!(sample(&app, 14.0) < 0.0 && sample(&app, -14.0) < 0.0, "the fixture is not a ball");
    let entries_before = app.history.stats().undo_entries;

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(column, SIZE.y * 0.1));
    app.on_pointer(PointerEvent::Moved { position: Vector::new(column, SIZE.y * 0.9), size: SIZE });
    release(&mut app);

    assert!(sample(&app, 14.0) > 0.0, "the drawn side stayed: {}", app.status);
    assert!(sample(&app, -14.0) > 0.0, "the mirrored side stayed: {}", app.status);
    assert!(sample(&app, 0.0) < 0.0, "the middle went too: {}", app.status);
    assert_eq!(
        app.history.stats().undo_entries,
        entries_before + 1,
        "a mirrored cut is one gesture and one entry"
    );
    assert!(app.status.contains("mirrored X"), "the status did not say so: {}", app.status);
}

/// And with the mirror off, the same drag takes one side only -- the cut
/// that has always shipped is untouched.
#[test]
fn a_cut_with_the_mirror_off_takes_one_side() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
    assert!(app.symmetry_settings().is_off());

    let column = column_at_world_x(&app, 10.0);
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(column, SIZE.y * 0.1));
    app.on_pointer(PointerEvent::Moved { position: Vector::new(column, SIZE.y * 0.9), size: SIZE });
    release(&mut app);

    let sample = |x: f32| app.doc.active_volume().sample_world(Vec3::new(x, 0.0, 0.0));
    assert!(sample(14.0) > 0.0, "the drawn side stayed: {}", app.status);
    assert!(sample(-14.0) < 0.0, "the other side went with no mirror on: {}", app.status);
    assert!(!app.status.contains("mirrored"), "{}", app.status);
}

/// What is shown is what goes, on both sides: with a mirror on, the
/// preview draws the twin as well as the outline that was drawn.
#[test]
fn the_cut_preview_draws_the_twin_when_a_mirror_is_on() {
    let mut app = app();
    // Looking straight down -Z, so world x and screen x agree and the
    // side assertion below means what it says.
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(SIZE.x * 0.65, SIZE.y * 0.2));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.65, SIZE.y * 0.8),
        size: SIZE,
    });

    let mut alone = hani_gpu::OverlayBatch::default();
    app.add_cut_preview(&mut alone);
    assert!(!alone.lines.is_empty(), "no preview at all");

    update(&mut app, Message::SymmetryAxisToggled(MirrorAxis::X));
    assert!(app.symmetry_settings().axis(MirrorAxis::X), "X did not turn on");
    let mut mirrored = hani_gpu::OverlayBatch::default();
    app.add_cut_preview(&mut mirrored);
    assert_eq!(mirrored.lines.len(), alone.lines.len() * 2, "the twin was not drawn");
    assert_eq!(mirrored.surfaces.len(), alone.surfaces.len() * 2, "the twin's side was not shaded");

    // The twin sits on the other side of the plane. The drag was to the
    // right of centre, so the outline is at positive x and its twin, once
    // reflected and brought back onto the preview plane, at negative x.
    let mean_x = |vertices: &[hani_gpu::OverlayVertex]| {
        vertices.iter().map(|vertex| vertex.position[0]).sum::<f32>() / vertices.len() as f32
    };
    assert!(mean_x(&alone.lines) > 0.0, "the fixture's outline is not at positive x");
    assert!(mean_x(&mirrored.lines[alone.lines.len()..]) < 0.0, "the twin is not across the plane");
}

/// A cut is destructive, so a click must never be one.
#[test]
fn a_click_with_the_cut_armed_does_nothing() {
    let mut app = app();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    let before = app.doc.active_volume().brick_coords().count();

    press(&mut app, centre_of_viewport());
    release(&mut app);

    assert_eq!(app.doc.active_volume().brick_coords().count(), before, "a click cut the model");
    assert!(!app.unsaved, "a click that cut nothing marked the document unsaved");
    assert!(app.status.contains("cancelled"), "reported: {}", app.status);
}

/// Add a sphere to whatever is already there.
///
/// **Not `seed_sphere`, which OVERWRITES.** Seeding clears every brick the
/// sphere's box touches before writing, so seeding a small sphere onto a
/// large one carves a moat around it rather than adding a lump to it: the
/// fixture ends up as a ball with a pit, and a test that asserts "the spur
/// went" passes for the wrong reason. A union is `min` of the two distance
/// fields, which is what this does.
fn union_sphere(volume: &mut hani_core::Volume, centre: Vec3, radius: f32) {
    let voxel_size = volume.voxel_size();
    let band = hani_core::NARROW_BAND * voxel_size;
    let (lo, hi) =
        volume.voxel_bounds(centre - (radius + band * 2.0), centre + (radius + band * 2.0));
    volume.edit_voxels(lo, hi, |_, position, value| {
        let outside = (position.distance(centre) - radius) / voxel_size;
        value.min(outside).clamp(hani_core::INSIDE, hani_core::OUTSIDE)
    });
}

/// Drag a closed loop, from a list of viewport positions.
fn drag_loop(app: &mut Hani, points: &[Vector]) {
    let first = *points.first().expect("a loop needs points");
    press(app, first);
    for point in points.iter().skip(1) {
        app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
    }
    app.on_pointer(PointerEvent::Moved { position: first, size: SIZE });
    release(app);
}

/// A circle of viewport positions around the middle of the screen.
fn loop_around(radius: f32, steps: usize) -> Vec<Vector> {
    let centre = centre_of_viewport();
    (0..steps)
        .map(|step| {
            let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
            centre + Vector::new(angle.cos() * radius, angle.sin() * radius)
        })
        .collect()
}

/// **The region cut is the region drawn**, checked in world space rather
/// than by looking at the screen.
///
/// This is the test whose absence let the apex bug survive a design review.
/// The cut used the ray's NEAR-PLANE point as the pyramid's apex instead of
/// the eye; with one plane that cancels exactly, and with several it
/// displaces every side by a different amount. It was invisible to any test
/// comparing the preview against the drawing, because the preview projects
/// the same screen points and is therefore screen-exact either way.
///
/// So this projects the RESULT back: every point that lost material must
/// fall inside the drawn polygon, and points well outside it must be
/// untouched. A displaced apex fails the second half.
#[test]
fn the_region_cut_matches_the_region_drawn() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));

    let radius = SIZE.y * 0.15;
    drag_loop(&mut app, &loop_around(radius, 24));
    assert!(app.unsaved, "the lasso did not cut: {}", app.status);

    // Probe a ring of world points at the depth the camera focuses on,
    // spanning well inside the loop and well outside it.
    let centre = centre_of_viewport();
    let volume = app.doc.active_volume();
    let mut inside_survived = 0;
    let mut outside_removed = 0;
    for step in 0..48 {
        let angle = step as f32 / 48.0 * std::f32::consts::TAU;
        for (scale, expect_gone) in [(0.45_f32, true), (2.2_f32, false)] {
            let pixel = centre + Vector::new(angle.cos(), angle.sin()) * (radius * scale);
            // Where that pixel meets the sphere's front face, before the
            // cut: the model is a ball of known radius at the origin.
            let (origin, ray) = app.ray_through(Vec2::new(pixel.x, pixel.y));
            let Some(hit) = ray_meets_ball(origin, ray, MODEL_RADIUS_MM) else {
                continue;
            };
            // A little inside the surface, so a grazing sample does not
            // decide the outcome.
            let probe = hit + ray * 0.5;
            let gone = volume.sample_world(probe) > 0.0;
            if expect_gone && !gone {
                inside_survived += 1;
            }
            if !expect_gone && gone {
                outside_removed += 1;
            }
        }
    }
    assert_eq!(inside_survived, 0, "material survived well inside the drawn loop");
    assert_eq!(outside_removed, 0, "material was removed well outside the drawn loop");
}

/// Where a ray first meets a ball at the origin, if it does.
fn ray_meets_ball(origin: Vec3, ray: Vec3, radius: f32) -> Option<Vec3> {
    let b = origin.dot(ray);
    let c = origin.length_squared() - radius * radius;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return None;
    }
    let t = -b - discriminant.sqrt();
    (t > 0.0).then(|| origin + ray * t)
}

/// **The preview must be drawn in FRONT of the model it describes.**
///
/// This is the one that `cargo test` could not see and a screenshot could.
/// The outline was first projected onto the plane through the camera's
/// target -- which is the middle of the model -- so the polygon sat INSIDE
/// the sphere and the depth test threw it away. Every headless assertion
/// passed: the geometry was built, the vertex counts were right, the shape
/// was correct, and the screen showed nothing at all.
///
/// The straight-line cut hid it for a while, because its band reaches three
/// model radii out and the part past the silhouette has nothing in front of
/// it. So the band appeared over the background and vanished over the
/// model, which reads as a rendering quirk rather than as a preview drawn
/// in the wrong place.
///
/// Depth is what is asserted, not pixels, because depth is the thing that
/// was wrong and it is checkable without a compositor.
#[test]
fn the_live_preview_is_drawn_in_front_of_the_model() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));

    let points = loop_around(SIZE.y * 0.15, 12);
    press(&mut app, points[0]);
    for point in points.iter().skip(1) {
        app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
    }
    app.on_pointer(PointerEvent::Moved { position: points[0], size: SIZE });

    assert!(!app.overlay.lines.is_empty(), "the live lasso drew no outline");

    // The nearest the model can be to the eye: the camera looks at the
    // origin and the body is a ball of MODEL_RADIUS_MM about it.
    let eye = app.camera.eye();
    let nearest_surface = eye.length() - MODEL_RADIUS_MM;
    for vertex in app.overlay.lines.iter().chain(&app.overlay.surfaces) {
        let away = Vec3::from_array(vertex.position).distance(eye);
        assert!(
            away < nearest_surface,
            "a preview vertex sits {away} from the eye, behind the model's near face at \
             {nearest_surface} -- the depth test will hide it"
        );
    }
}

/// **The depth cap spares the wall behind the spur.**
///
/// The fixture is the one the design argues about: a lump sticking out with
/// another surface close behind it, close enough that placing the cap one
/// band past the lump's back face would eat into the wall. Circling the
/// lump must take the lump and leave the wall EXACTLY as it was.
///
/// `assert_same_field` rather than a thickness measurement, because a
/// thickness measurement is what `is_printable` does not have: it counts
/// holes and winding, so a wall thinned from the front with a clean step is
/// still "printable" and no other assertion in this repo would notice.
#[test]
fn a_lasso_over_a_spur_leaves_the_wall_behind_it_alone() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    // Rebuilt as a spur in front of a wall, in camera-relative terms so the
    // fixture does not depend on which way yaw zero points.
    let eye = app.camera.eye();
    let toward_camera = (eye - app.camera.target).normalize();
    {
        let volume = app.doc.active_volume_mut();
        *volume = hani_core::Volume::new(volume.voxel_size());
        // The wall: a big ball centred behind the origin, so its near face
        // sits a little way behind the spur.
        volume.seed_sphere(-toward_camera * 34.0, 30.0);
        // The spur: small, in front, straddling the wall's near face so it
        // is attached rather than floating. UNIONED, because seeding it
        // would clear the wall's bricks around it and leave a moat.
        union_sphere(volume, toward_camera * 6.0, 7.0);
        volume.mark_everything_dirty();
    }
    app.rebuild_everything();

    let before = app.doc.active_volume().duplicated(glam::IVec3::ZERO);

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    drag_loop(&mut app, &loop_around(SIZE.y * 0.06, 20));
    assert!(app.unsaved, "the lasso did not cut: {}", app.status);

    // The wall's far side, well behind anything the spur occupies: it must
    // read exactly as it did.
    let deep = -toward_camera * 50.0;
    let after = app.doc.active_volume().sample_world(deep);
    assert_eq!(before.sample_world(deep), after, "the cut reached through to the back of the wall");
    // And the spur itself is gone.
    let spur = toward_camera * 10.0;
    assert!(
        app.doc.active_volume().sample_world(spur) > 0.0,
        "the spur survived the lasso that circled it"
    );
}

/// Shift asks for the old behaviour, and it must actually get it: the same
/// loop with shift held goes all the way through.
#[test]
fn shift_makes_the_same_lasso_cut_through() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    let eye = app.camera.eye();
    let toward_camera = (eye - app.camera.target).normalize();
    {
        let volume = app.doc.active_volume_mut();
        *volume = hani_core::Volume::new(volume.voxel_size());
        volume.seed_sphere(-toward_camera * 34.0, 30.0);
        union_sphere(volume, toward_camera * 6.0, 7.0);
        volume.mark_everything_dirty();
    }
    app.rebuild_everything();

    app.shift = true;
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    drag_loop(&mut app, &loop_around(SIZE.y * 0.06, 20));

    let deep = -toward_camera * 50.0;
    assert!(
        app.doc.active_volume().sample_world(deep) > 0.0,
        "shift did not cut through: {}",
        app.status
    );
}

/// **The hole is the same size where it goes in and where it comes out.**
///
/// The cutter is built along the view direction rather than from the eye,
/// so its sides are parallel. Built from the eye instead -- which is the
/// obvious construction and the one this shipped with first -- the removed
/// region is the view frustum's own wedge and widens exactly as the
/// projection does. At the framing this application picks that is not
/// subtle: the eye sits about 90 mm from a 30 mm ball, so the front face is
/// 60 mm away and the back 120, and **the exit hole comes out twice the
/// diameter of the entry hole**.
///
/// Measured rather than reasoned about, because the factor depends on the
/// framing rule, the field of view and the model's size all at once, and a
/// test that recomputed those would be asserting its own arithmetic.
#[test]
fn a_cut_through_leaves_the_same_hole_at_both_ends() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    let eye = app.camera.eye();
    let facing = (app.camera.target - eye).normalize();
    // Any direction across the view, to scan outward along.
    let across = facing.cross(Vec3::Y).normalize_or(Vec3::X);

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    // Shift, so the cut goes all the way through and there are two ends to
    // compare. With the depth cap on there is only one.
    app.shift = true;
    drag_loop(&mut app, &loop_around(SIZE.y * 0.10, 24));
    assert!(app.unsaved, "the lasso did not cut: {}", app.status);

    // How far the hole reaches from the axis, on a slab a little inside
    // each face so a grazing sample cannot decide it.
    let reach_at = |depth: f32| {
        let volume = app.doc.active_volume();
        let mut last_gone = 0.0_f32;
        let mut radius = 0.0_f32;
        while radius < 28.0 {
            let probe = facing * depth + across * radius;
            if volume.sample_world(probe) > 0.0 {
                last_gone = radius;
            } else {
                break;
            }
            radius += 0.25;
        }
        last_gone
    };

    let front = reach_at(-25.0);
    let back = reach_at(25.0);
    assert!(front > 1.0, "no hole at the front face: {front}");
    assert!(back > 1.0, "no hole at the back face: {back}");
    assert!(
        (back - front).abs() < front * 0.15,
        "the hole flares through the model: {front:.2} mm going in, {back:.2} mm coming \
         out -- the cutter is a pyramid from the eye rather than a prism"
    );
}

/// **Ctrl with a loop selects instead of cutting, and the selection becomes
/// a body.**
///
/// This is the whole "circle the lump and it becomes a layer" route, end to
/// end through the application: the same drag, the same hull, the same
/// classification, writing protection instead of removing material -- and
/// then the shipped masked split lifting it off.
///
/// The field must come through the mask step bit-identical. Ctrl and no
/// ctrl differ in which map they write, and a ctrl gesture that moved one
/// voxel of geometry would be a destructive operation wearing a
/// selection's clothes.
#[test]
fn a_ctrl_lasso_selects_a_region_and_it_can_be_split_off() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    let before = app.doc.active_volume().duplicated(glam::IVec3::ZERO);
    let bodies_before = app.doc.nodes().len();

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    app.control = true;
    drag_loop(&mut app, &loop_around(SIZE.y * 0.12, 20));
    app.control = false;

    assert!(
        !app.doc.active_volume().mask().is_free(),
        "a ctrl lasso protected nothing: {}",
        app.status
    );
    // Compared by sampling rather than through the core's own helper,
    // which is behind a test-only module this crate cannot reach.
    for probe in [
        Vec3::new(0.0, 0.0, MODEL_RADIUS_MM - 2.0),
        Vec3::new(8.0, 4.0, MODEL_RADIUS_MM - 6.0),
        Vec3::ZERO,
        Vec3::new(-12.0, 0.0, 0.0),
    ] {
        assert_eq!(
            before.sample_world(probe),
            app.doc.active_volume().sample_world(probe),
            "a ctrl lasso moved the field at {probe:?}"
        );
    }
    assert!(app.status.contains("lasso"), "the status does not name the shape: {}", app.status);

    // And the selection lifts off as its own row.
    update(&mut app, Message::BodySplitMasked);
    assert!(
        app.doc.nodes().len() > bodies_before,
        "the masked split produced no new body: {}",
        app.status
    );
}

/// **A cut that severs a body says so.**
///
/// Two shells inside one `Volume` look right on screen, export as one
/// object and arrive at the slicer in two pieces. Nothing before this said
/// a word about it.
#[test]
fn a_cut_that_severs_a_body_reports_the_loose_piece() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    // A dumbbell: two balls joined by a thin neck, arranged across the view
    // so a straight drag can part them.
    let eye = app.camera.eye();
    let facing = (app.camera.target - eye).normalize();
    let across = facing.cross(Vec3::Y).normalize_or(Vec3::X);
    {
        let volume = app.doc.active_volume_mut();
        *volume = hani_core::Volume::new(volume.voxel_size());
        volume.seed_sphere(across * -14.0, 10.0);
        union_sphere(volume, across * 14.0, 10.0);
        // The neck, as a chain of overlapping spheres: there is no capsule
        // primitive, and a union of them is the same solid.
        for step in 0..=56 {
            let t = step as f32 / 56.0;
            union_sphere(volume, across * (-14.0 + t * 28.0), 2.5);
        }
        volume.mark_everything_dirty();
    }
    app.rebuild_everything();
    assert_eq!(
        app.doc.split_plan(app.doc.active()).map(|plan| plan.parts.len()),
        Some(1),
        "the fixture is already in pieces before the cut"
    );

    // Cut the neck: a lasso around the middle, deep enough to part it.
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    app.shift = true;
    drag_loop(&mut app, &loop_around(SIZE.y * 0.035, 16));
    app.shift = false;

    assert!(app.status.contains("loose piece"), "a severed body was not reported: {}", app.status);
}

/// A lasso disarms the tool exactly as a line does. The one-shot property
/// belongs to the gesture, not to the shape of it.
#[test]
fn a_lasso_disarms_the_cut_like_a_line_does() {
    let mut app = app();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    drag_loop(&mut app, &loop_around(SIZE.y * 0.15, 20));
    assert_ne!(app.tool, Tool::Cut, "the cut stayed armed after a lasso");
}

/// Arm the cut, hold ctrl, and draw a loop: the mask-instead-of-cut gesture.
///
/// The release goes through `update` rather than `on_pointer`, because the
/// every-message tail is what refreshes the mask card -- and a status line
/// that names the card's verb row is only honest if that row is up by the
/// time anyone reads it.
fn ctrl_lasso(app: &mut Hani, radius: f32) {
    update(app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    app.control = true;
    let points = loop_around(radius, 24);
    let first = *points.first().expect("a loop needs points");
    press(app, first);
    for point in points.iter().skip(1) {
        app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
    }
    app.on_pointer(PointerEvent::Moved { position: first, size: SIZE });
    update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    app.control = false;
}

/// **The status must not send anyone to a panel with no such button.**
///
/// It used to say "split it off from the BODIES panel". The BODIES verb row
/// is five icons and its Split fires [`Message::BodySplit`], a different
/// operation, so a user who followed the sentence either found nothing or
/// found the wrong thing. A beta user hit it within an hour of the
/// repository going public.
#[test]
fn the_ctrl_lasso_status_names_a_surface_that_is_actually_on_screen() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    ctrl_lasso(&mut app, SIZE.y * 0.15);

    assert!(!app.doc.active_volume().mask().is_free(), "the ctrl lasso masked nothing");
    assert!(
        !app.status.contains("BODIES"),
        "the status still sends the user to a panel that has no such button: {}",
        app.status
    );
    assert!(
        app.status.contains("mask card"),
        "the status does not name the surface the verb is on: {}",
        app.status
    );
    assert!(
        app.mask_card.as_ref().is_some_and(MaskCard::names_the_active_body),
        "the status named the mask card's verb row and the card will not be showing one: {}",
        app.status
    );
}

/// **The advised operation must actually run from the state the gesture
/// leaves**, which is the half of the claim a test can carry.
///
/// Dispatching [`Message::BodySplitMasked`] with no `ToolChanged` in
/// between is what pressing the card's `split off` does, so this pins that
/// the advice is not merely well-worded but possible: the tool is disarmed,
/// the active body carries a partial mask, and `split_the_mask_off` accepts
/// it rather than refusing. Before this round the same dispatch would still
/// have worked -- the defect was that nothing on screen could send it.
///
/// **What this does NOT pin:** that the button exists in the widget tree.
/// iced offers no introspection, so no test in this suite can see a button.
/// That half is verified by driving the running application; see
/// `docs/DRIVING-THE-APP.md`.
#[test]
fn a_ctrl_lasso_leaves_the_split_it_advises_ready_to_run() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();
    let before = app.doc.body_count();

    ctrl_lasso(&mut app, SIZE.y * 0.15);
    assert!(app.status.contains("split off"), "the status did not advise a split");

    update(&mut app, Message::BodySplitMasked);

    assert!(
        app.status.starts_with("split "),
        "the split the status advised refused instead: {}",
        app.status
    );
    assert_eq!(
        app.doc.body_count(),
        before + 1,
        "the advised split did not leave two bodies where there was one"
    );
}

/// The advice points at the card only when the card will carry the verb,
/// and never reads as an offer to lift several bodies off at once.
#[test]
fn the_split_advice_points_at_the_card_only_when_the_card_will_carry_the_verb() {
    // The active body was masked, so the card's verb row is up.
    assert!(split_advice(true, 1).starts_with("split off on the mask card"));
    assert!(!split_advice(true, 1).contains("BODIES"));
    assert!(!split_advice(true, 1).contains("one body at a time"));

    // Several bodies masked: one press splits one of them.
    assert!(split_advice(true, 3).contains("one body at a time"));

    // The active body was missed, so BODIES is where a body gets chosen
    // first -- and the singular case must read as singular.
    assert!(split_advice(false, 1).contains("click it in BODIES"));
    assert!(split_advice(false, 3).contains("click one of them in BODIES"));
    for touched in [1, 3] {
        assert!(
            split_advice(false, touched).contains("mask card"),
            "the verb is still on the card even when a body must be chosen first"
        );
    }
}

/// **The sentence names the body it masked, not whichever body was active.**
///
/// The loop walks every DRAWN body while `split_the_mask_off` takes
/// `doc.active()`, and the old line read the second while describing the
/// first: mask a body that is not active and it confidently named a body
/// the gesture never went near. Hiding the active one is the cheap way to
/// make it undrawn -- placing two bodies so a centred loop reaches one and
/// misses the other is the fiddly way, and buys nothing extra.
#[test]
fn a_ctrl_lasso_names_the_body_it_masked_rather_than_the_one_that_was_active() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    let sphere = app.doc.active();
    let sphere_name = app.body_name(sphere);

    // A second body, which becomes active, and is then taken off screen so
    // the gesture cannot reach it. Hiding the active body moves the active
    // row to the first visible one -- deliberate, and documented in
    // `toggle_visibility` -- so it has to be selected back afterwards. That
    // reselection is the whole fixture: it is the only way to reach the
    // state where the active body is not the body a gesture can touch.
    update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
    let cube = app.doc.active();
    assert_ne!(cube, sphere, "the primitive did not become the active body");
    update(&mut app, Message::BodyVisibilityToggled(cube));
    update(&mut app, Message::BodySelected(cube));
    assert_eq!(app.doc.active(), cube, "the hidden body could not be made active again");

    ctrl_lasso(&mut app, SIZE.y * 0.15);

    assert!(
        app.status.contains(&sphere_name),
        "the status did not name the body the lasso actually masked: {}",
        app.status
    );
    assert!(
        app.status.contains("click it in BODIES"),
        "the status offered a card verb for a body that was not masked: {}",
        app.status
    );
}

/// The status names the shape, because "cut 812 bricks" over a lasso and
/// over a plane describe very different things happening to the model.
#[test]
fn the_status_says_which_shape_cut() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    drag_loop(&mut app, &loop_around(SIZE.y * 0.15, 24));
    assert!(app.status.contains("lasso"), "a lasso reported: {}", app.status);

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y * 0.3));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.9, SIZE.y * 0.3),
        size: SIZE,
    });
    release(&mut app);
    assert!(!app.status.contains("lasso"), "a straight drag reported: {}", app.status);
}

/// A loop thinner than the lattice is refused with its own sentence rather
/// than reporting a successful cut over a model that did not change.
#[test]
fn a_loop_thinner_than_the_voxel_grid_is_refused() {
    let mut app = app();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    let before = app.doc.active_volume().brick_coords().count();

    // Long, and about a pixel wide: out along the middle of the viewport
    // and back a hair below.
    let middle = SIZE.y * 0.5;
    let mut path = vec![Vector::new(SIZE.x * 0.2, middle)];
    for step in 1..=20 {
        path.push(Vector::new(SIZE.x * (0.2 + 0.03 * step as f32), middle));
    }
    for step in (0..=20).rev() {
        path.push(Vector::new(SIZE.x * (0.2 + 0.03 * step as f32), middle + 0.4));
    }
    drag_loop(&mut app, &path);

    assert_eq!(
        app.doc.active_volume().brick_coords().count(),
        before,
        "a sliver loop changed the model"
    );
    assert!(app.status.contains("cancelled"), "reported: {}", app.status);
}

/// Escape mid-drag throws the shape away and leaves the tool armed, which
/// is the treatment a live gizmo drag already gets.
#[test]
fn escape_mid_path_leaves_the_cut_armed() {
    let mut app = app();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(SIZE.x * 0.2, SIZE.y * 0.5));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.5, SIZE.y * 0.4),
        size: SIZE,
    });
    assert!(!app.cut_path.is_empty(), "the path did not accumulate");

    update(&mut app, Message::MenuClosed);

    assert_eq!(app.tool, Tool::Cut, "escape mid-path disarmed the tool");
    assert!(app.cut_path.is_empty(), "escape left the path behind");
    assert!(!app.unsaved, "escape mid-path still cut");
}

/// The tool card has to say what the gesture in progress will do. "PLANE
/// CUT" over a lasso is a lie about which material is about to go.
#[test]
fn the_live_label_follows_the_shape_being_drawn() {
    let mut app = app();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    assert_eq!(app.live_tool_label(), "PLANE CUT", "an unstarted cut is not a plane cut");

    let points = loop_around(SIZE.y * 0.15, 20);
    press(&mut app, points[0]);
    for point in points.iter().skip(1) {
        app.on_pointer(PointerEvent::Moved { position: *point, size: SIZE });
    }
    app.on_pointer(PointerEvent::Moved { position: points[0], size: SIZE });
    assert_eq!(app.live_tool_label(), "LASSO CUT", "a live loop still says plane cut");
}

#[test]
fn a_cut_is_undoable_through_the_application() {
    let mut app = app();
    let probe = Vec3::new(0.0, 12.0, 0.0);
    let before = app.doc.active_volume().sample_world(probe);

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
        size: SIZE,
    });
    release(&mut app);
    assert_ne!(app.doc.active_volume().sample_world(probe), before, "the cut did nothing to undo");

    app.undo();
    assert_eq!(app.doc.active_volume().sample_world(probe), before, "undo did not restore the cut");
}

/// A cut a mask blocked entirely must never report itself as a cut that
/// missed.
///
/// This is the one failure masking adds to a tool that had none: the cut
/// does nothing, and the reason is protection painted minutes ago, possibly
/// on a body that is not even the one being looked at. "The cut missed the
/// model" sends the user off to redraw a line that was never the problem,
/// so the message names the mask AND the body it is on.
#[test]
fn a_cut_a_mask_blocked_names_the_mask_and_the_body_it_is_on() {
    let mut app = app();
    let body = app.doc.active();
    let mut meta = app.doc.meta(body).expect("the active body");
    meta.name = "Left Ear".to_string();
    app.doc.set_meta(&meta);
    // Mask All: an empty map read inverted, so every voxel of the body is
    // protected and it costs nothing.
    app.doc.active_volume_mut().mask_mut().set_inverted(true);

    let before = app.doc.active_volume().brick_coords().count();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
        size: SIZE,
    });
    release(&mut app);

    assert_eq!(
        app.doc.active_volume().brick_coords().count(),
        before,
        "the mask did not stop the cut"
    );
    assert!(!app.unsaved, "a cut that changed nothing marked the document unsaved");
    assert!(
        !app.status.contains("missed"),
        "a cut a mask blocked reported that it missed: {}",
        app.status
    );
    assert!(app.status.contains("mask"), "the message does not name the mask: {}", app.status);
    assert!(
        app.status.contains("Left Ear"),
        "the message does not name the body the mask is on: {}",
        app.status
    );
}

/// Escape is the way out of every other mode, and a destructive one must
/// not be the exception.
#[test]
fn escape_disarms_the_cut() {
    let mut app = app();
    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    assert_eq!(app.tool, Tool::Cut);
    update(&mut app, Message::MenuClosed);
    assert_ne!(app.tool, Tool::Cut, "escape left a destructive mode armed");
}

/// **The plane cut crosses every VISIBLE body.**
///
/// A cut is a line the user draws across what they can see, so it acts on
/// what is drawn. `finish_cut` was single-volume before there was a
/// document, and the rename that gave it one made it silently mean "the
/// active body" -- which is exactly the kind of change that ships with
/// nothing failing. This was written as the executable statement of what it
/// owed and left `#[ignore]`d, alongside a `debug_assert!` at the top of
/// `finish_cut` saying the same thing from the other side. Both were
/// discharged by `Document::clip`, which brackets each visible body and
/// records ONE `Entry` of N `Change::Bricks`.
#[test]
fn a_cut_crosses_every_visible_body() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    // A second ball overlapping the first, so one screen-space line crosses
    // both of them.
    let mut second = hani_core::Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(10.0, 0.0, 0.0), MODEL_RADIUS_MM * 0.5);
    second.mark_everything_dirty();
    let other = app.doc.add_body("Body 2", second);
    app.remesh_dirty();

    let above = Vec3::new(10.0, 12.0, 0.0);
    let before = app.doc.volume(other).expect("a live body").sample_world(above);

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
        size: SIZE,
    });
    release(&mut app);

    assert_ne!(
        app.doc.volume(other).expect("a live body").sample_world(above),
        before,
        "the cut passed straight through the inactive body and left it whole"
    );
}

/// **Solo narrows the cut**, and that is decided rather than incidental.
///
/// The instinct is that a view mode must never narrow an operation. It does
/// not survive the case: eight bodies with three eye-hidden and four
/// solo-hidden means a line drawn across one body cuts five, and **nothing
/// on screen distinguishes an eye-hidden body from a solo-hidden one** --
/// in both cases it is simply not there. The rule that survives is narrower
/// and can be stated in one sentence: direct manipulation acts on what is
/// drawn; the file and the export act on `saved_visibility`.
#[test]
fn solo_narrows_the_cut_to_the_body_it_is_showing() {
    let mut app = app();
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.publish_camera();

    let mut second = hani_core::Volume::new(app.doc.voxel_size());
    second.seed_sphere(Vec3::new(10.0, 0.0, 0.0), MODEL_RADIUS_MM * 0.5);
    second.mark_everything_dirty();
    let other = app.doc.add_body("Body 2", second);
    app.remesh_dirty();

    let first = app.doc.nodes()[0].id;
    update(&mut app, Message::SoloEntered(first));

    let above = Vec3::new(10.0, 12.0, 0.0);
    let spared = app.doc.volume(other).expect("a live body").sample_world(above);
    let over_the_soloed = Vec3::new(0.0, 12.0, 0.0);
    let cut_away = app.doc.volume(first).expect("a live body").sample_world(over_the_soloed);

    update(&mut app, Message::ToolChanged(Tool::Cut));
    // The line cut, not the ring. See `app/ring_cut.rs`.
    update(&mut app, Message::Ring(crate::message::RingMessage::Way(CutWay::Line)));
    press(&mut app, Vector::new(SIZE.x * 0.1, SIZE.y / 2.0));
    app.on_pointer(PointerEvent::Moved {
        position: Vector::new(SIZE.x * 0.9, SIZE.y / 2.0),
        size: SIZE,
    });
    release(&mut app);

    assert_eq!(
        app.doc.volume(other).expect("a live body").sample_world(above),
        spared,
        "the cut went through a body solo was hiding"
    );
    // ...and the body solo IS showing really was cut, so this is not a test
    // of a cut that missed everything and passed for the wrong reason.
    assert_ne!(
        app.doc.volume(first).expect("a live body").sample_world(over_the_soloed),
        cut_away,
        "the cut missed the soloed body too: {}",
        app.status
    );
    assert!(
        app.status.contains("bricks") && !app.status.contains("bodies"),
        "the cut did not report exactly one body: {}",
        app.status
    );
}
