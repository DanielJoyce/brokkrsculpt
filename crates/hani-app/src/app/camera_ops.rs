// SPDX-License-Identifier: AGPL-3.0-only

//! The camera: what it is told, what it may aim at, framing, the navigation
//! cube and its flights, and turning the model from the cube's menu. Moved
//! out of `app.rs` (issue 189).

use super::*;

/// How long a click on the navigation cube takes to arrive.
const FLIGHT_MS: f32 = 260.0;

impl Hani {
    pub(super) fn publish_camera(&mut self) {
        // **The one place a camera becomes legal.** Every gesture, every
        // flight, every restore passes through here, so containing the target
        // here rather than in each of them makes "the camera is lost" a state
        // the application cannot reach rather than one it has to detect. See
        // [`Hani::contain_target`] for why that is worth a check per frame of
        // a drag.
        self.contain_target();
        self.shared.set_camera(self.camera);

        // **A live transform drag has to be re-pinned when the camera moves.**
        //
        // `gizmo::drag` rebuilds both rays from the CURRENT camera, so moving
        // the camera under a held handle re-interprets the pixel the button
        // went down on and the body walks with nothing touching the pointer.
        // Measured on the standard fixture with a stationary pointer: the
        // X-axis offset drifted 5.4696 mm to 6.5737 mm for a 0.4 radian yaw
        // nudge. Three paths reach this without a second button going down --
        // the SpaceMouse, the scroll wheel, and a navigation-cube flight still
        // in the air when the drag started -- so guarding the pointer handler
        // would have missed all three. This is the one place a camera becomes
        // legal, so it is the one place that has to know.
        //
        // Re-pinned rather than frozen: the gesture stays continuous and the
        // two-handed SpaceMouse-and-stylus workflow the application is built
        // around survives. Freezing the puck mid-drag is a larger behavioural
        // change than the bug.
        let repin = self.cursor.filter(|_| {
            self.drag.is_some_and(|drag| matches!(drag.kind, DragKind::Transforming(_)))
                && self.gizmo.is_some_and(|gizmo| gizmo.grabbed.is_some())
        });
        if let Some(cursor) = repin {
            if let Some(drag) = self.drag.as_mut() {
                drag.origin = cursor;
            }
            if let Some(live) = self.gizmo.as_mut() {
                live.pinned = live.placement;
            }
        }

        // The cube shows the camera's orientation, so it is stale the moment the
        // camera moves. This is the one place that knows that happened.
        self.refresh_cube();
        // And the gizmo is held at a constant SIZE on screen, so it is stale
        // for the same reason. See `refresh_gizmo` for why it cannot ride
        // `refresh_overlay` instead.
        self.refresh_gizmo();
    }

    /// The ball the camera is allowed to aim inside: everything the document
    /// holds, plus nothing.
    ///
    /// Over [`Document::world_bounds`] and not the active body, because a user
    /// panning across a two body document is doing something reasonable and
    /// must not be fenced into whichever one happens to be selected. The union
    /// spans the gaps between bodies, which for a camera bound is the safe
    /// direction to be loose in -- unlike sizing a primitive, where the same
    /// looseness compounds and is refused.
    ///
    /// Falls back to the starting ball when the document is empty, so a new
    /// file still has somewhere legal to look.
    pub(super) fn document_ball(&self) -> (Vec3, f32) {
        match self.doc.world_bounds() {
            Some((low, high)) => ((low + high) * 0.5, ((high - low).length() * 0.5).max(1.0e-3)),
            None => (Vec3::ZERO, MODEL_RADIUS_MM),
        }
    }

    /// Pull the camera's target back inside [`Hani::document_ball`].
    ///
    /// **The fence is what makes the rest of the system's assumptions true.**
    /// Two subsystems take the eye-to-target distance as the eye-to-MODEL
    /// distance and would break silently without it: [`OrbitCamera::far`] is
    /// both the far plane and the length of every pick ray, and
    /// [`OrbitCamera::near`] is a hundredth of the distance. Let the target
    /// wander a long way from the model and the model goes behind the far plane
    /// -- measured, at 2405 mm from the eye against a far plane of 2318: it is
    /// not drawn and it is not pickable, and nothing says so.
    ///
    /// So the fence stays the document's own ball, in millimetres. **Widening
    /// it in proportion to the distance was tried and is what produced that
    /// measurement** -- the idea being that a target far from the model is
    /// harmless when the eye is even further away, which is true of the camera
    /// alone and false of the two derived quantities above.
    ///
    /// It is [`Document::world_bounds`] and not the active body, because a user
    /// panning across a two body document is doing something reasonable and
    /// must not be fenced into whichever one happens to be selected.
    ///
    /// The clamp is to the surface of the ball rather than to its centre, so
    /// nothing jumps: a gesture pushing outward simply stops moving the target,
    /// and every gesture pulling back inward is free.
    ///
    /// **What keeps the fence from being felt is that the wheel no longer
    /// pushes the target at it**; see [`Hani::navigation_anchor`].
    fn contain_target(&mut self) {
        let (centre, radius) = self.document_ball();
        if !self.camera.target.is_finite() {
            self.camera.target = centre;
            return;
        }
        let offset = self.camera.target - centre;
        if offset.length() > radius {
            self.camera.target = centre + offset.normalize() * radius;
        }
    }

    /// Tell the camera the lattice and extent it should measure itself against.
    ///
    /// The zoom floor is a number of voxels and the far plane a multiple of the
    /// content radius, so both go stale the moment the document is resampled,
    /// imported into, or grown. Called from the same places
    /// [`Hani::refresh_model_radius`] is.
    pub(super) fn refresh_camera_lattice(&mut self) {
        let radius = self
            .doc
            .world_bounds()
            .map_or(MODEL_RADIUS_MM, |(low, high)| ((high - low).length() * 0.5).max(1.0e-3));
        self.camera.set_lattice(self.doc.voxel_size(), radius);

        // **The bounds moved, so the distance has to be brought inside them.**
        //
        // `set_lattice` writes the two figures the floor and the ceiling are
        // derived from and never re-checks the distance against them, so a
        // camera already sitting on the old floor is left UNDER the new one.
        // One press of "coarser" from 0.25 mm takes the floor from 0.500 to
        // 2.000 with the camera still at 0.500, and the next wheel notch INWARD
        // then jumps it out to 2.000 and drags the target four times away from
        // whatever it was anchored on -- a zoom in that flies backwards.
        //
        // The clamp alone would be incomplete in the way this project has been
        // bitten before: neither `resample` nor `remove_body` publishes a
        // camera, and `Message::Frame` only publishes when a flight is running,
        // so the corrected distance would sit in `self.camera` while
        // `SharedFrame` still held the old one. `publish_camera` is what closes
        // that, and it is safe to call from here because it does not itself
        // call back into this function.
        let clamped =
            self.camera.distance.clamp(self.camera.min_distance(), self.camera.max_distance());
        if clamped != self.camera.distance {
            self.camera.distance = clamped;
            self.publish_camera();
        }
    }

    /// Frame the active body, which is the one keystroke that recovers from any
    /// camera at all.
    ///
    /// **A camera free enough to be useful is a camera that can get lost**, and
    /// the answer to that tension is not to restrict the freedom but to make
    /// coming back cost one key. `contain_target` stops the target wandering
    /// out of the document; this is what puts it back on the thing the user
    /// meant when it has merely wandered somewhere unhelpful.
    ///
    /// The angles are kept, exactly as [`Hani::apply_view`] keeps them: a
    /// user who has turned the model to see the back of its head has not asked
    /// to be spun round to the front.
    pub(super) fn frame_active(&mut self) {
        let volume = self.doc.active_volume();
        let (centre, radius) = match (volume.world_bounds(), volume.content_radius()) {
            (Some((low, high)), Some(radius)) => ((low + high) * 0.5, radius.max(1.0e-3)),
            _ => (Vec3::ZERO, MODEL_RADIUS_MM),
        };
        let framed = OrbitCamera::framing(centre, radius);
        self.camera.target = framed.target;
        self.refresh_camera_lattice();
        // Clamped, because a body smaller than a couple of voxels frames at a
        // distance under its own floor and the camera would sit inside its own
        // limit until the next scroll pushed it back out.
        self.camera.distance =
            framed.distance.clamp(self.camera.min_distance(), self.camera.max_distance());
        self.publish_camera();
        self.status = "framed the active body".to_string();
    }

    /// The world point a wheel notch should zoom about.
    ///
    /// **The surface under the cursor when moving in, the target otherwise**,
    /// and both halves of that were arrived at by measuring rather than by
    /// taste.
    ///
    /// Anchoring the way IN is the feature Thomas asked for. The eye travels
    /// toward the target, and after a file is opened the target is the middle
    /// of the model -- so a target-centred zoom walks the camera INTO the
    /// geometry, and by the time an eyelid fills the view the eye is inside the
    /// skull. Lowering the distance floor only chooses how deep in you may get.
    /// Anchoring on the surface under the cursor is what Nomad and Blender do
    /// and what makes "closer" mean closer to the detail. It costs nothing:
    /// `update_hover` already ran a full pick on the last pointer move, and a
    /// wheel notch does not move the pointer.
    ///
    /// Everything else is the target, because an anchor that is NOT on the
    /// model pushes the target off the model in exact proportion to the
    /// distance. `(target - anchor) / distance` is invariant across
    /// [`OrbitCamera::zoom_by_about`] -- that invariance is exactly why the
    /// anchor stays pinned under the cursor -- so the drift is unbounded in the
    /// distance, and it breaks three things in rising order of quietness:
    ///
    /// - It walks the target into [`Hani::contain_target`]'s fence, and that
    ///   clamp is not invertible. Measured through the real wheel path, cursor
    ///   over empty background on the default ball: four notches out and four
    ///   back returned the target to within 0.00 mm, eight and eight left it
    ///   25.62 mm away on the opposite side, twelve and twelve left it 69.06 mm
    ///   away -- the whole fence -- with the model no longer under the middle of
    ///   the viewport. Two flicks of a wheel.
    /// - Widening the fence to admit the drift is worse, not better. With the
    ///   target far from the model, [`OrbitCamera::far`] -- the far plane AND
    ///   the length of every pick ray -- stops reaching the model. Measured at
    ///   2405 mm from the eye against a far plane of 2318: the model is not
    ///   drawn and not pickable, and nothing says so.
    /// - [`OrbitCamera::near`] is a hundredth of the distance, so far enough
    ///   out the same drift clips the near side too.
    ///
    /// Two anchors are off the model, and both are handled by falling back:
    ///
    /// - **Zooming out, wherever the cursor is.** Anchoring outward is the same
    ///   arithmetic run backwards and drifts the target away from the cursor.
    /// - **Zooming in with the cursor over background.** The fallback this
    ///   replaced was the ray's crossing of the plane through the target, argued
    ///   for as being continuous across a silhouette -- but that point is not on
    ///   the model, so it drifts too: twelve notches out and twelve back in over
    ///   a corner puts the target 188 mm from a 69 mm document, i.e. hard into
    ///   the fence. Continuity across the silhouette is not worth losing the
    ///   sculpt for.
    ///
    /// What this gives up is stated plainly: zooming in on a detail and back out
    /// restores the distance and the angles exactly but leaves the view centred
    /// on that detail rather than on wherever it started. That is not a camera
    /// that failed to come back -- the model is whole and on screen, at the
    /// distance and angle it was -- and it is the price of the target never
    /// being anywhere the rest of the system does not expect it. **The target
    /// cannot leave the document ball through the wheel at all now:** the only
    /// anchor that moves it is a point on the model, the ball is convex, and a
    /// zoom-in leaves the target on the segment between two points inside it.
    pub(super) fn navigation_anchor(&self, factor: f32) -> Vec3 {
        match self.hover {
            Some(hover) if factor < 1.0 => hover,
            _ => self.camera.target,
        }
    }

    /// Rebuild the navigation cube and hand it to the renderer.
    pub(super) fn refresh_cube(&mut self) {
        let mut batch = std::mem::take(&mut self.cube);
        navcube::build(&mut batch, &self.camera, self.cube_hover);
        self.shared.swap_cube(&mut batch);
        self.cube = batch;
    }

    /// Start an animated move to a cube part's orientation.
    pub(super) fn fly_to(&mut self, part: navcube::Part) {
        // **Not clamped short of the pole any more, and that is the point.**
        // A flight interpolates the angle fields directly, so while `look_at`
        // built the view matrix a top view had to stop 0.02 rad short or arrive
        // at a collapsed matrix -- which meant clicking "top" on the cube gave
        // a view that was very nearly but visibly not the top. The composed
        // basis is orthonormal at the pole, so the cube now lands on it.
        let (yaw, pitch) = navcube::orientation(part.direction, self.camera.yaw);
        // The shortest way round, so a click never spins the model several times
        // to reach a heading a few degrees away.
        let yaw = self.camera.yaw + OrbitCamera::shortest_angle_delta(self.camera.yaw, yaw);
        self.flight = Some(Flight {
            from: (self.camera.yaw, self.camera.pitch, self.camera.roll),
            to: (yaw, pitch, 0.0),
            elapsed_ms: 0.0,
        });
    }

    /// Advance a camera flight. Returns whether the camera moved.
    pub(super) fn advance_flight(&mut self, elapsed_ms: f32) -> bool {
        let Some(mut flight) = self.flight else {
            return false;
        };
        flight.elapsed_ms += elapsed_ms.clamp(0.0, 50.0);
        let t = (flight.elapsed_ms / FLIGHT_MS).clamp(0.0, 1.0);
        // Smoothstep, so it eases out of the old view and into the new one
        // rather than starting and stopping abruptly.
        let eased = t * t * (3.0 - 2.0 * t);
        let lerp = |from: f32, to: f32| from + (to - from) * eased;

        self.camera.yaw = lerp(flight.from.0, flight.to.0);
        self.camera.pitch = lerp(flight.from.1, flight.to.1);
        self.camera.roll = lerp(flight.from.2, flight.to.2);

        self.flight = (t < 1.0).then_some(flight);
        true
    }

    /// Turn the whole model, so that a face the user picked becomes the
    /// direction they asked for.
    ///
    /// Exact, unlike [`Hani::resample`]: a quarter turn maps voxels onto
    /// voxels, so nothing is resampled and turning back returns the same bits.
    /// That is also why there is no undo entry -- see below.
    ///
    /// **Every body turns, and turning only the active one would be wrong.**
    /// Bodies share one lattice and have no transform, so where a body sits IS
    /// its brick occupancy and the arrangement of the bodies is the only
    /// positional state the document has. Turning one of them would scatter
    /// that arrangement with nothing to put it back, and it would make this
    /// function's own status line -- turn it back the same way to undo -- a
    /// lie, because the bodies that did not turn would come back somewhere new.
    pub(super) fn orient(&mut self, rotation: hani_core::AxisRotation) {
        if rotation.is_identity() {
            return;
        }

        let started = Instant::now();
        self.doc.rotate(rotation);
        self.unsaved = true;
        // Every entry names a brick of a volume that no longer exists, exactly
        // as after a resample. Snapshotting the whole field instead would
        // exceed the history budget on any real model, and it is not needed:
        // turning back is the undo, and it is exact.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.rebuild_everything();
        self.refresh_overlay();

        self.status = format!(
            "turned the model, {:.0} ms -- turn it back the same way to undo",
            started.elapsed().as_secs_f64() * 1000.0
        );
        log::info!("{}", self.status);
    }
}

/// The camera Thomas asked for: free to move, still focused on the sculpt.
///
/// These are the tests that measure the OUTCOME rather than the mechanism.
/// Everything in `camera.rs` proves the arithmetic; this proves that driving
/// the application the way a user drives it gets you next to a detail and back
/// again without ending up inside the model or lost in space.
#[cfg(test)]
mod free_camera_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// A wheel notch, delivered the way the widget delivers one: the pointer
    /// has moved there first, so the hover is current.
    fn scroll(app: &mut Hani, at: Vector, amount: f32) {
        app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
        app.on_pointer(PointerEvent::Scrolled { amount, position: at, size: SIZE });
    }

    /// Off centre, so an anchored zoom and a target-centred one visibly differ.
    fn off_centre() -> Vector {
        Vector::new(SIZE.x / 2.0 + 70.0, SIZE.y / 2.0 - 50.0)
    }

    /// **The complaint, in one test.** "Now I can't zoom in close to the sculpt
    /// to work on details."
    ///
    /// The eye travels toward the TARGET, and after a file is opened the target
    /// is the centre of the model -- so scrolling in walks the camera into the
    /// geometry, and every millimetre of "closer" is a millimetre further
    /// inside. Raising or lowering the distance floor cannot fix that; it only
    /// chooses how deep in you are allowed to get. What fixes it is zooming
    /// toward the surface under the cursor.
    ///
    /// The assertion is the one that matters to a person: the eye is OUTSIDE
    /// the solid, and it is a small number of voxels from the surface it was
    /// pointed at.
    #[test]
    fn zooming_in_on_a_detail_ends_up_beside_the_surface_and_not_inside_it() {
        let mut app = app();
        let at = off_centre();
        app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
        let detail = app.hover.expect("the test point has to be on the model");

        for _ in 0..60 {
            scroll(&mut app, at, 1.0);
        }

        let eye = app.camera.eye();
        let field = app.doc.active_volume().sample_world(eye);
        assert!(field > 0.0, "the camera ended up inside the model: field {field} at {eye:?}");
        assert!(
            eye.distance(detail) < MODEL_RADIUS_MM * 0.25,
            "the camera stopped {} mm from the detail it was aimed at",
            eye.distance(detail)
        );
        assert!(
            app.camera.distance <= app.camera.min_distance() * 1.01,
            "sixty notches did not reach the floor: {}",
            app.camera.distance
        );
        // And close means close: two voxels, not a hundredth of the model.
        assert!(
            app.camera.distance < 1.0,
            "the closest the camera can get is {} mm",
            app.camera.distance
        );
    }

    /// The other half of the sentence: "but still focused on the sculpt".
    ///
    /// **The assertion is the one a person would make, and the one this test
    /// used to make could be satisfied while the model was invisible.** It
    /// asserted only that the target stayed inside the document ball -- a
    /// statement about a number the user cannot see. What matters is that the
    /// sculpt is still there to be worked on, so that is what is measured: the
    /// middle of the document still lands inside the widget, and a pick at that
    /// pixel still finds it. The second assertion is not redundant: with the
    /// target drifting, the model went behind `far()`, which is the far plane
    /// AND the pick reach, while still being inside the frustum sideways.
    ///
    /// **This is a guard rather than a reproduction.** It passes against the
    /// code as it was, because the tight fence held the model inside `far()` by
    /// accident. It exists because the first attempted fix -- widening the
    /// fence in proportion to the distance -- broke it, and nothing else would
    /// have noticed.
    #[test]
    fn a_long_scroll_out_over_empty_space_keeps_the_model_on_screen() {
        let mut app = app();
        // A corner, well clear of a sphere drawn in the middle.
        let empty = Vector::new(20.0, 20.0);
        app.on_pointer(PointerEvent::Moved { position: empty, size: SIZE });
        assert!(app.hover.is_none(), "the corner was supposed to miss the model");

        let (centre, _) = app.document_ball();
        let pixel_of = |app: &Hani| {
            let clip = app.camera.view_projection(SIZE.x / SIZE.y) * centre.extend(1.0);
            (clip.w, Vec2::new(clip.x / clip.w, clip.y / clip.w))
        };
        for step in 0..200 {
            scroll(&mut app, empty, -1.0);
            let (w, ndc) = pixel_of(&app);
            assert!(
                w > 0.0 && ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0,
                "after {} notches out the middle of the document is at {ndc:?}, off screen",
                step + 1
            );
        }

        // And it is not merely inside the frustum sideways: the cursor still
        // finds it there, which is what says it is inside the depth range too.
        let (_, ndc) = pixel_of(&app);
        let pixel = Vector::new((ndc.x + 1.0) * 0.5 * SIZE.x, (1.0 - ndc.y) * 0.5 * SIZE.y);
        app.on_pointer(PointerEvent::Moved { position: pixel, size: SIZE });
        assert!(app.hover.is_some(), "the model is drawn at {pixel:?} but nothing is pickable");
    }

    /// **The cost of the anchor rule, asserted rather than merely claimed.**
    /// Zooming in is anchored on the surface under the cursor and zooming out
    /// is anchored on the target, so a scroll in on a detail and back out
    /// restores the distance exactly but leaves the view centred on that detail
    /// rather than on wherever it started. That is the trade
    /// [`Hani::navigation_anchor`] argues for, so the distance is asserted to
    /// come back and the target deliberately is not.
    ///
    /// **This is a guard, not a reproduction**: it passes against the code as
    /// it was, because in-then-out over the MODEL was never the broken order.
    /// The broken one is
    /// `scrolling_out_over_a_corner_and_back_in_returns_exactly`.
    #[test]
    fn scrolling_in_and_back_out_returns_the_distance_and_leaves_the_model_in_view() {
        for notches in [4, 8, 12, 20, 30] {
            let mut app = app();
            let at = off_centre();
            app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
            assert!(app.hover.is_some(), "the test point has to be on the model");
            let before = app.camera.distance;

            for _ in 0..notches {
                scroll(&mut app, at, 1.0);
            }
            for _ in 0..notches {
                scroll(&mut app, at, -1.0);
            }

            assert!(
                (app.camera.distance - before).abs() < 1.0e-2,
                "{notches} notches in and back left the distance at {} instead of {before}",
                app.camera.distance
            );
            // The complaint restated: the sculpt is under the middle of the
            // viewport, where a person would look for it.
            let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
            app.on_pointer(PointerEvent::Moved { position: centre, size: SIZE });
            assert!(
                app.hover.is_some(),
                "after {notches} in and back the model is not under the middle of the viewport"
            );
        }
    }

    /// Scrolling out is anchored on the target, so it cannot move the target at
    /// all -- which is the property the drift used to violate and the one the
    /// containment fence should never have to enforce.
    #[test]
    fn scrolling_out_does_not_move_the_target() {
        let mut app = app();
        for at in [off_centre(), Vector::new(20.0, 20.0)] {
            app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
            let before = app.camera.target;
            for _ in 0..40 {
                scroll(&mut app, at, -1.0);
            }
            assert!(
                app.camera.target.distance(before) < 1.0e-4,
                "scrolling out at {at:?} moved the target {} mm",
                app.camera.target.distance(before)
            );
        }
    }

    /// A scroll with the cursor over nothing is the commonest gesture in the
    /// application, and it must be exactly the target-centred zoom it has
    /// always been -- in BOTH directions, so that it is perfectly reversible.
    ///
    /// The fallback this pins replaces the ray's crossing of the plane through
    /// the target. That point is off the model, so zooming in about it drifts
    /// the target off the model in proportion to the distance: measured, twelve
    /// notches out and twelve back in over a corner left the target 188 mm from
    /// a 69 mm document, hard into the fence and unrecoverable by the inverse
    /// gesture.
    #[test]
    fn a_scroll_over_empty_background_is_the_zoom_it_has_always_been() {
        let mut app = app();
        let empty = Vector::new(20.0, 20.0);
        app.on_pointer(PointerEvent::Moved { position: empty, size: SIZE });
        assert!(app.hover.is_none(), "the corner was supposed to miss the model");
        let before = (app.camera.target, app.camera.distance);

        for amount in [1.0_f32, -1.0, -1.0, 1.0, 3.0, -3.0] {
            let expected = app.camera.distance * OrbitCamera::zoom_factor(amount);
            let target = app.camera.target;
            scroll(&mut app, empty, amount);
            assert!(
                app.camera.target.distance(target) < 1.0e-6,
                "a scroll over background moved the target to {:?}",
                app.camera.target
            );
            assert!(
                (app.camera.distance - expected).abs() < 1.0e-4,
                "the distance went to {} rather than {expected}",
                app.camera.distance
            );
        }
        assert!(app.camera.target.distance(before.0) < 1.0e-6);
        assert!((app.camera.distance - before.1).abs() < 1.0e-4);
    }

    /// The reported failure, in its exact shape: N notches out over empty
    /// background then N back in, with the cursor parked in a corner. Twelve
    /// and twelve used to leave the target 69.06 mm out -- the whole fence --
    /// and the model no longer under the middle of the viewport.
    ///
    /// The target tolerance is float noise and not a budget: the fixed gesture
    /// does not touch the target at all, at either end, so the honest claim is
    /// that it is unchanged.
    #[test]
    fn scrolling_out_over_a_corner_and_back_in_returns_exactly() {
        for notches in [4, 8, 12, 20, 30] {
            let mut app = app();
            let empty = Vector::new(20.0, 20.0);
            app.on_pointer(PointerEvent::Moved { position: empty, size: SIZE });
            assert!(app.hover.is_none(), "the corner was supposed to miss the model");
            let before = (app.camera.target, app.camera.distance);

            for _ in 0..notches {
                scroll(&mut app, empty, -1.0);
            }
            for _ in 0..notches {
                scroll(&mut app, empty, 1.0);
            }

            assert!(
                app.camera.target.distance(before.0) < 1.0e-4,
                "{notches} out and back left the target {} mm away",
                app.camera.target.distance(before.0)
            );
            assert!(
                (app.camera.distance - before.1).abs() < 1.0e-2,
                "{notches} out and back left the distance at {} instead of {}",
                app.camera.distance,
                before.1
            );
            let centre = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
            app.on_pointer(PointerEvent::Moved { position: centre, size: SIZE });
            assert!(
                app.hover.is_some(),
                "after {notches} out and back the model is not under the middle of the viewport"
            );
        }
    }

    /// The fence still has a job, and this is it: a target nowhere near the
    /// document, at a distance that cannot see it, is pulled back.
    #[test]
    fn a_target_the_camera_could_not_possibly_see_is_pulled_back() {
        let mut app = app();
        app.camera.distance = 1.0;
        app.camera.target = Vec3::new(-435.18, -12.09, 95.65);
        app.publish_camera();

        let (centre, radius) = app.document_ball();
        assert!(
            app.camera.target.distance(centre) <= radius + 1.0e-3,
            "the target is still {} mm from a document of radius {radius}",
            app.camera.target.distance(centre)
        );
    }

    /// The arithmetic of [`OrbitCamera::zoom_by_about`] is exactly invertible,
    /// which is what lets the wheel arm anchor the way in without the way out
    /// having to undo anything by hand.
    ///
    /// **It is a statement about the camera and NOT about the gesture, and
    /// reading it as the latter hid a real bug.** It calls the camera directly,
    /// so it never goes through `publish_camera` and never meets
    /// `contain_target`; the containment fence could be deleted outright and
    /// this would stay green. What the user actually does is pinned by
    /// `scrolling_in_and_back_out_returns_the_distance_and_leaves_the_model_in_view`
    /// and `scrolling_out_does_not_move_the_target`.
    #[test]
    fn zooming_in_and_back_out_about_the_same_point_is_reversible() {
        let mut app = app();
        let at = off_centre();
        app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
        let anchor = app.navigation_anchor(OrbitCamera::zoom_factor(1.0));
        let before = (app.camera.target, app.camera.distance);

        for _ in 0..10 {
            app.camera.zoom_by_about(OrbitCamera::zoom_factor(1.0), anchor);
        }
        for _ in 0..10 {
            app.camera.zoom_by_about(OrbitCamera::zoom_factor(-1.0), anchor);
        }
        assert!(
            app.camera.target.distance(before.0) < 1.0e-2,
            "the target came back to {:?} instead of {:?}",
            app.camera.target,
            before.0
        );
        assert!((app.camera.distance - before.1).abs() < 1.0e-2);
    }

    /// One keystroke back from anywhere. This is the affordance that makes
    /// letting the camera move freely safe rather than reckless.
    #[test]
    fn framing_the_active_body_recovers_from_a_target_far_outside_the_model() {
        let mut app = app();
        app.camera.yaw = 1.234;
        app.camera.pitch = -0.5;
        app.camera.target = Vec3::new(-435.18, -12.09, 95.65);
        app.camera.distance = 0.3;

        update(&mut app, Message::CameraFramedOnActive);

        let radius = app.doc.active_volume().content_radius().expect("the ball has bounds");
        // Within a voxel of the origin, not exactly on it: the framing centre
        // is the middle of the BRICK box, which for a sphere on the lattice
        // sits half a voxel off.
        assert!(
            app.camera.target.length() < VOXEL_SIZE_MM,
            "the camera is still aimed at {:?}",
            app.camera.target
        );
        assert!(
            app.camera.distance > radius,
            "the whole body has to fit: distance {} against radius {radius}",
            app.camera.distance
        );
        // The angles are the user's and are kept, exactly as a restore keeps
        // them: framing is not a request to be spun round to the front.
        assert!((app.camera.yaw - 1.234).abs() < 1.0e-4);
        assert!((app.camera.pitch + 0.5).abs() < 1.0e-4);
        assert!(app.status.contains("framed"), "nothing said the camera had moved");
    }

    /// The key it is on, decoded the way a key press is decoded.
    #[test]
    fn the_f_key_frames_the_active_body() {
        assert!(matches!(
            crate::viewport::shortcut("f", false, false, false),
            Some(Message::CameraFramedOnActive)
        ));
        // And it must not fire as part of a chord meant for something else.
        assert!(crate::viewport::shortcut("f", true, false, false).is_none());
        assert!(crate::viewport::shortcut("f", false, false, true).is_none());
    }

    /// Orbiting turns about the surface the press landed on, so turning to see
    /// the side of a detail keeps the detail on screen.
    #[test]
    fn an_orbit_drag_turns_about_the_surface_the_press_landed_on() {
        let mut app = app();
        let at = off_centre();
        app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
        let pivot = app.hover.expect("the test point has to be on the model");

        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Right,
            position: at,
            size: SIZE,
        });
        assert_eq!(
            app.drag.and_then(|drag| drag.pivot),
            Some(pivot),
            "the press did not latch the surface under it"
        );

        let aspect = SIZE.x / SIZE.y;
        let screen = |app: &Hani, point: Vec3| {
            let clip = app.camera.view_projection(aspect) * point.extend(1.0);
            Vec2::new(clip.x / clip.w, clip.y / clip.w)
        };
        let before = screen(&app, pivot);
        for step in 0..25 {
            let to = Vector::new(at.x + step as f32 * 6.0, at.y - step as f32 * 3.0);
            app.on_pointer(PointerEvent::Moved { position: to, size: SIZE });
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Right });

        let after = screen(&app, pivot);
        assert!(
            before.distance(after) < 1.0e-2,
            "the pivot slid from {before:?} to {after:?} across the drag"
        );
    }

    /// A pointer that never touched the model still has to orbit, and about
    /// the target rather than about any off-model point, which would walk the
    /// target sideways on every background drag.
    #[test]
    fn an_orbit_drag_that_starts_off_the_model_turns_about_the_target() {
        let mut app = app();
        let empty = Vector::new(20.0, 20.0);
        app.on_pointer(PointerEvent::Moved { position: empty, size: SIZE });
        assert!(app.hover.is_none());

        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Right,
            position: empty,
            size: SIZE,
        });
        let target = app.camera.target;
        for step in 1..20 {
            let to = Vector::new(empty.x + step as f32 * 8.0, empty.y + step as f32 * 4.0);
            app.on_pointer(PointerEvent::Moved { position: to, size: SIZE });
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Right });

        assert!(
            app.camera.target.distance(target) < 1.0e-3,
            "a background drag walked the target from {target:?} to {:?}",
            app.camera.target
        );
    }

    /// The camera's floor and far plane are in the document's units, so a
    /// resample has to move them or the floor means a different closeness than
    /// it did a moment ago.
    #[test]
    fn resampling_moves_the_zoom_floor_with_the_lattice() {
        let mut app = app();
        let before = app.camera.min_distance();
        update(&mut app, Message::Resample(VOXEL_SIZE_MM / 2.0));
        assert!(
            (app.camera.min_distance() - app.doc.voxel_size() * 2.0).abs() < 1.0e-6,
            "the floor is {} on a {} mm lattice",
            app.camera.min_distance(),
            app.doc.voxel_size()
        );
        assert!(app.camera.min_distance() < before, "a finer lattice must let the camera closer");
    }

    /// **A close-in view saved on any lattice but 0.25 mm used to be thrown
    /// away on every reopen, with a status line blaming the file.**
    ///
    /// `apply_view` built its accept/reject bounds from
    /// `OrbitCamera::framing(centre, radius)`, and `framing` sets the content
    /// radius but never the voxel size -- so it inherited the 0.25 mm default
    /// and the loader's floor was 0.5 mm for every document ever opened, while
    /// the live floor is `doc.voxel_size() * 2.0`. Measured: resample to 0.05,
    /// eighty notches in, the camera legitimately reaches 0.1 -- and the
    /// restore gave back 158.10 mm. This is the same class as the bug fa6e234
    /// fixed, a floor computed from the wrong source, and the suite could not
    /// see it because `VOXEL_SIZE_MM` happens to equal the camera's default.
    ///
    /// **So this test resamples first, and that is the whole point of it.**
    /// The fixture it sits beside runs at 0.25 and therefore proves nothing.
    #[test]
    fn a_close_in_view_survives_a_reopen_on_a_lattice_finer_than_the_default() {
        let mut app = app();
        update(&mut app, Message::Resample(0.05));
        assert!(
            (app.doc.voxel_size() - 0.05).abs() < 1.0e-6,
            "the resample did not take: {}",
            app.doc.voxel_size()
        );

        let at = off_centre();
        for _ in 0..80 {
            scroll(&mut app, at, 1.0);
        }
        let close = app.camera.distance;
        assert!(
            close < 0.5,
            "the point of the test is a distance under the old 0.5 floor: {close}"
        );

        let view = app.current_view();
        app.status = "opened".to_string();
        app.apply_view(&view);

        assert!(
            (app.camera.distance - close).abs() < 1.0e-6,
            "the restore moved the camera from {close} to {}",
            app.camera.distance
        );
        assert!(
            !app.status.contains("re-framed"),
            "a view the live camera reached was called unusable: {}",
            app.status
        );
    }

    /// The same disagreement at the other end of the range. The loader's
    /// ceiling came from the ACTIVE body's content radius while the live one
    /// comes from the document ball -- 5520 against 6906 on the starting
    /// document -- so a distance the application itself drove the camera into
    /// was rejected as corrupt by its own loader.
    #[test]
    fn a_view_scrolled_out_to_the_live_ceiling_survives_a_reopen() {
        let mut app = app();
        let at = off_centre();
        for _ in 0..400 {
            scroll(&mut app, at, -1.0);
        }
        let far_out = app.camera.distance;
        assert!(
            (far_out - app.camera.max_distance()).abs() < 1.0e-3,
            "the test did not actually reach the ceiling: {far_out}"
        );

        let view = app.current_view();
        app.status = "opened".to_string();
        app.apply_view(&view);

        assert!(
            (app.camera.distance - far_out).abs() < 1.0e-3,
            "the restore moved the camera from {far_out} to {}",
            app.camera.distance
        );
        assert!(
            !app.status.contains("re-framed"),
            "a distance the live clamp produced was called unusable: {}",
            app.status
        );
    }

    /// **A puck button used to undo the whole point of this work.** Both camera
    /// actions assigned a fresh `OrbitCamera::framing(Vec3::ZERO, 30.0)`, which
    /// resets `voxel_size` to the 0.25 mm default and `content_radius` to the
    /// hardcoded starting ball -- and both fields are now load-bearing. The
    /// first is the zoom floor, so on a 0.05 mm lattice the floor jumped from
    /// 0.1 to 0.5 and the camera became five times coarser than the lattice
    /// deserves. The second is `far()`, which is the far plane AND the length
    /// of every pick ray, so on a large model the far half of it went both
    /// unclipped-into-nothing and silently unpickable.
    #[test]
    fn the_spacemouse_camera_buttons_keep_the_lattice_the_document_is_on() {
        use crate::spacemouse::ButtonAction;

        for action in [ButtonAction::FrameModel, ButtonAction::ResetView] {
            let mut app = app();
            update(&mut app, Message::Resample(0.05));
            let floor = app.camera.min_distance();
            let reach = app.camera.content_radius;

            app.run_puck_button(action);

            assert!(
                (app.camera.min_distance() - floor).abs() < 1.0e-6,
                "{action:?} moved the zoom floor from {floor} to {}",
                app.camera.min_distance()
            );
            assert!(
                app.camera.content_radius >= reach - 1.0e-3,
                "{action:?} shrank the pick reach from {reach} to {}",
                app.camera.content_radius
            );
        }
    }

    /// **The one place the two halves of this work pull against each other**,
    /// measured rather than reasoned about.
    ///
    /// `orbit_about` moves the target to keep the pivot fixed, and
    /// `contain_target` refuses to let the target leave the document's ball.
    /// A target rotating about a pivot `p` from the centre travels on a sphere
    /// of radius `|target - p|`, so it can reach `2p` from the centre -- and
    /// if that exceeds the ball, the clamp engages and the pivot slides out
    /// from under the cursor.
    ///
    /// Measured on the starting ball: two hundred motion events of a single
    /// drag, more than half a turn, take the target to 51 mm of a 69 mm leash
    /// and the pivot slips by 8e-6 in normalised device coordinates -- which is
    /// float noise, not the clamp. The headroom comes from the ball being the
    /// half-diagonal of the BRICK box, which rounds out past the surface.
    ///
    /// **The leash was deliberately not widened to make this impossible in
    /// every geometry.** Doubling it would admit a target 1.9 content radii
    /// out, and the real poisoned file was at 1.9 -- the fence would then stop
    /// short of the failure it exists to prevent. On a model whose surface
    /// reaches further into its own box than a sphere does, a very long orbit
    /// can still meet the leash, and what happens then is that the target stops
    /// while the drag continues. That is the leash working.
    #[test]
    fn a_long_orbit_drag_does_not_run_into_the_containment_leash() {
        let mut app = app();
        let at = off_centre();
        app.on_pointer(PointerEvent::Moved { position: at, size: SIZE });
        let pivot = app.hover.expect("the test point has to be on the model");
        let (centre, radius) = app.document_ball();

        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Right,
            position: at,
            size: SIZE,
        });
        let aspect = SIZE.x / SIZE.y;
        let screen = |app: &Hani, point: Vec3| {
            let clip = app.camera.view_projection(aspect) * point.extend(1.0);
            Vec2::new(clip.x / clip.w, clip.y / clip.w)
        };
        let before = screen(&app, pivot);

        let mut worst_slip = 0.0_f32;
        let mut worst_reach = 0.0_f32;
        for step in 1..200 {
            app.on_pointer(PointerEvent::Moved {
                position: Vector::new(at.x + step as f32 * 4.0, at.y),
                size: SIZE,
            });
            worst_slip = worst_slip.max(before.distance(screen(&app, pivot)));
            worst_reach = worst_reach.max(app.camera.target.distance(centre));
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Right });

        assert!(worst_slip < 1.0e-3, "the pivot slipped {worst_slip} across the drag");
        assert!(
            worst_reach < radius,
            "the drag reached {worst_reach} of a {radius} mm leash, so the clamp engaged"
        );
    }

    /// `update` returns a `Task`; tests do not run the iced runtime.
    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }
}
