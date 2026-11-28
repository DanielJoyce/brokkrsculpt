// SPDX-License-Identifier: AGPL-3.0-only

//! What is drawn over the model and what the pointer is over: the brush
//! ring, the mirror planes, the live cut's outline, the hover and the pick.
//! Moved out of `app.rs` (issue 189).

use super::*;

impl Hani {
    /// Rebuild the brush ring and mirror planes and hand them to the renderer.
    ///
    /// Called from the places that change what it looks like -- pointer motion,
    /// a brush or mirror setting, a resample -- and deliberately NOT from the
    /// frame handler: the geometry is world space, so a camera moving under it
    /// needs no rebuild, and a raycast per frame would be waste.
    pub(super) fn refresh_overlay(&mut self) {
        // One of several moves handled together: the last one draws the
        // ring. See `pointer_in`.
        if self.batching {
            return;
        }
        let timed = self.phases.enter(phases::Phase::Overlay);
        self.build_overlay();
        self.phases.leave(timed);
    }

    /// The overlay itself. [`Hani::refresh_overlay`] is this, timed.
    fn build_overlay(&mut self) {
        // Moved out and back rather than borrowed in place: `build` also needs
        // the volume and the brush, which are fields of the same `self`. The
        // two buffers then rotate -- this frame's goes to the renderer and last
        // frame's comes back with its capacity -- so nothing allocates once
        // warm.
        let mut batch = std::mem::take(&mut self.overlay);
        // The body the pick returned, and not the active one. The ring is
        // pushed onto the surface by sampling the field it is over, so building
        // it from a body that does not contain the hover point draws a
        // confident ring in mid air; see `cursor::build`.
        let hovered = self.hover_body.unwrap_or_else(|| self.doc.active());
        let volume = self.doc.volume(hovered).unwrap_or_else(|| self.doc.active_volume());
        let selecting = self.hover_body.is_some_and(|body| body != self.doc.active());
        // Mask mode never selects -- see the press dispatch -- so the ring must
        // not say a press would.
        let masking = (self.tool == Tool::Mask).then(|| self.mask_op());
        cursor::build(
            &mut batch,
            volume,
            &self.effective_brush(),
            self.framed_symmetry(),
            MIRROR_CENTRE,
            self.hover,
            cursor::mood(
                self.stroke_direction(),
                self.sizing.is_some(),
                selecting && masking.is_none(),
                masking,
            ),
            self.model_radius,
        );
        self.add_cut_preview(&mut batch);
        // A shape being dragged out, drawn coarse. See `app/insert_preview.rs`.
        self.add_insert_preview(&mut batch);
        // The decal being dragged. See `app/decal.rs`.
        self.add_decal_preview(&mut batch);
        // What a drybrush would catch. See `app/dry_preview.rs`.
        self.add_catch_preview(&mut batch);
        // The ring cut. See `app/ring_cut.rs`.
        self.add_ring_preview(&mut batch);
        // The floor. See `app/references.rs`.
        self.add_floor(&mut batch);
        self.shared.swap_overlay(&mut batch);
        self.overlay = batch;
    }

    /// Add the live cut's outline to the overlay, if a cut is being drawn.
    ///
    /// Appended after the ring rather than instead of it, because the batch is
    /// shared -- and the ring is already gone by the time this runs, taken away
    /// by `refresh_hover`'s `Tool::Cut` arm.
    ///
    /// The projection puts the shape on the plane through the camera's target,
    /// facing the camera. That is where the model being looked at is, and it is
    /// the one depth at which a screen-space region is honest about its size.
    pub(super) fn add_cut_preview(&self, batch: &mut hani_gpu::OverlayBatch) {
        if self.tool != Tool::Cut || self.cut_path.is_empty() {
            return;
        }
        let Some(gesture) = cut::read_stroke(&self.cut_path, CLICK_SLOP_PX) else {
            return;
        };
        let eye = self.camera.eye();
        let focus = self.camera.target;
        let Some(facing) = (focus - eye).try_normalize() else {
            return;
        };
        // **In FRONT of the model, not at its centre.** The preview is a
        // projection of screen points along the view rays, so sliding it toward
        // the camera changes its world size and not one pixel of where it
        // appears -- it is screen-exact at every depth. What it does change is
        // whether the depth test keeps it: drawn on the plane through the
        // camera's target, the outline of a loop over the model sits INSIDE the
        // model and is occluded by the very surface it is describing. The line
        // cut hid this for a while, because its shaded band reaches three model
        // radii out and the part beyond the silhouette has nothing in front of
        // it -- so the band appeared over the background and vanished over the
        // model, which reads as a rendering quirk rather than as the bug it is.
        //
        // Just clear of the bounding sphere, and never closer than twice the
        // near plane, so it cannot be clipped away instead.
        let depth =
            ((focus - eye).length() - self.model_radius * 1.05).max(self.camera.near() * 2.0);
        let plane_at = eye + facing * depth;
        // Taken from the planes the cut is ACTUALLY about to use, so the shaded
        // side and the removed side cannot disagree. `cutter_from_hull` is the
        // one place the sign is settled, and asking it again here is cheaper
        // than a second derivation that could be wrong on its own.
        let doomed =
            self.cutter_from_hull(&gesture.hull).and_then(|planes| match planes.as_slice() {
                [only] => Some(only.normal),
                _ => None,
            });
        // A straight drag is infinite whatever shift says, so it is always
        // the denser shading; a shaped cut is bounded unless shift asks
        // otherwise.
        let through = self.shift || gesture.shape == cut::CutShape::Line;
        let at = |pixel: Vec2| {
            let (origin, ray) = self.ray_through(pixel);
            // Where the ray crosses the plane through the target. The dot
            // product cannot be zero for a ray inside the frustum, but a
            // fallback costs nothing and a division by it would put a
            // vertex at infinity.
            let denominator = ray.dot(facing);
            if denominator.abs() < 1.0e-6 {
                return origin + ray * depth;
            }
            origin + ray * ((plane_at - origin).dot(facing) / denominator)
        };
        cut_preview::build(
            batch,
            &gesture.hull,
            gesture.shape,
            self.model_radius,
            doomed,
            through,
            at,
        );

        // **The twins, one per enabled mirror**, because the cut takes them
        // too and a preview that showed one side of a two-sided cut would be
        // lying about half of what is about to go. Each outline is reflected
        // through the plane exactly as the cutter is, then brought back onto
        // the preview plane along the eye's own rays -- so it sits in front of
        // the model like the original rather than inside it, where the depth
        // test would hide the very shape it is describing. That projection
        // is exact at the focus depth and the same approximation the preview
        // already makes everywhere else.
        let mut flips = [hani_core::Flip::IDENTITY; Symmetry::MAX_MIRRORS];
        let count = self.framed_symmetry().flips(MIRROR_CENTRE, &mut flips);
        for flip in &flips[..count] {
            let mirrored_at = |pixel: Vec2| {
                let reflected = flip.point(at(pixel));
                let along = reflected - eye;
                let denominator = along.dot(facing);
                if denominator <= 1.0e-6 {
                    // Behind the eye: left where it is, and the near plane
                    // clips it rather than a division sending it to infinity.
                    return reflected;
                }
                eye + along * (depth / denominator)
            };
            cut_preview::build(
                batch,
                &gesture.hull,
                gesture.shape,
                self.model_radius,
                doomed.map(|normal| flip.direction(normal)),
                through,
                mirrored_at,
            );
        }
    }

    /// Where the pointer meets the surface, and on which body, remembered for
    /// the cursor ring and read by the press.
    fn update_hover(&mut self, pixel: Vec2) {
        match self.pick(pixel) {
            Some((body, hit)) => {
                self.hover = Some(hit.position);
                self.hover_body = Some(body);
            }
            None => {
                self.hover = None;
                self.hover_body = None;
            }
        }
    }

    /// The same, except that a live stroke owns the ring.
    ///
    /// **While a stroke is running the ring belongs to the body being carved
    /// and to no other.** [`Hani::update_hover`] picks across every drawn
    /// body, so dragging a stroke over a second body moved the ring onto that
    /// body's surface -- and [`Hani::refresh_overlay`] builds the ring out of
    /// the hovered body's field and colours it as `CursorMood::Selecting` the
    /// moment the hovered body is not the active one. The cursor was telling
    /// the user "a press here would select", on the wrong surface, during a
    /// press that was carving something else. The carve itself was never
    /// affected, because the stroke asks [`Hani::surface_under`]; only the
    /// overlay lied, which is worse than it sounds for a tool whose whole
    /// feedback loop is that ring.
    ///
    /// Asking `surface_under` here costs one march of one body, where the pick
    /// it replaces marched every drawn one, so the stroke path got cheaper
    /// rather than dearer.
    pub(super) fn refresh_hover(&mut self, pixel: Vec2) {
        // **Mask mode is the same case as a live stroke**, and for the same
        // reason rather than by analogy: a mask press does not pick, so the
        // paint lands on the active body wherever the cursor is. A ring built
        // from another body's field at a point on that other body's surface is
        // a confident ring in mid air over a body that is not being painted.
        // **The gizmo takes the ring away entirely**, which is not the same
        // case as the two below. A brush ring says "a press here would carve
        // this much of that surface", and under the gizmo a press does no such
        // thing -- it grabs a handle, or it chooses a body. Drawing one would
        // be the same lie the live-stroke case below exists to stop, told about
        // a different gesture.
        //
        // **The cut takes it away too, and for exactly the same reason.** With
        // the cut armed a press does not carve, it draws a line -- and a ring
        // saying "a press here would remove this much of that surface" is the
        // same lie told about a different gesture. It is worse here than under
        // the gizmo, because a cut IS destructive and the ring is what a user
        // reads to decide whether it is safe to press.
        // Apart, nothing is where it is drawn. See `app/explode.rs`.
        if self.tool == Tool::Transform || self.tool == Tool::Cut || self.exploded() {
            self.hover = None;
            self.hover_body = None;
            return;
        }
        // One of several moves handled together: the ring is wanted where
        // the last of them left the pointer, and that move finds it. See
        // `pointer_in`.
        if self.batching {
            return;
        }
        let owns_the_ring = self.tool == Tool::Mask
            || matches!(self.drag.map(|drag| drag.kind), Some(DragKind::Sculpt(_)));
        if !owns_the_ring {
            self.update_hover(pixel);
            return;
        }
        self.hover = self.surface_under(pixel);
        // Paired with the position rather than set unconditionally: a stroke
        // dragged off the model has no ring, and a `hover_body` naming a body
        // no ring is being drawn against would be a fact with no owner.
        self.hover_body = self.hover.map(|_| self.doc.active());
    }

    /// The nearest DRAWN surface under a point in widget pixels, and the body
    /// it belongs to.
    ///
    /// The visibility mask is the one `publish_visibility` already keeps, so
    /// the pick, the renderer and the panel cannot disagree about what is on
    /// screen — and a body the user cannot see cannot be picked, hovered or
    /// carved.
    pub(super) fn pick(&self, pixel: Vec2) -> Option<(NodeId, hani_core::Hit)> {
        let (origin, ray) = self.ray_through(pixel);
        let timed = self.phases.enter(phases::Phase::Pick);
        // Less the bodies drawn see-through. See `app/explode.rs`.
        let hit = self.doc.pick(origin, ray, self.camera.far(), self.pick_mask());
        self.phases.leave(timed);
        hit
    }

    /// Whether the body edits land on is one of the ones being drawn.
    ///
    /// **The pick alone does not answer this**, and that is the whole reason
    /// this exists. [`Document::pick`] refuses a hidden body, but a stroke
    /// takes its surface from [`Document::pick_body`], which asks one named
    /// body and deliberately does not consult the eye -- a live stroke keeps
    /// carving the body it started on. So a press while the ACTIVE body is
    /// hidden picks nothing, falls through to a sculpt, and then happily
    /// marches the invisible body and carves it.
    ///
    /// The mask is `publish_visibility`'s, which is recomputed after every
    /// message, so it is never stale by the time a pointer event reads it. A
    /// body that is not in the document at all reads as not drawn, which is
    /// the safe direction: the answer only ever refuses.
    pub(super) fn active_is_drawn(&self) -> bool {
        self.doc
            .index_of(self.doc.active())
            .is_some_and(|index| self.shown.get(index).copied().unwrap_or(false))
    }

    /// The world space ray through a point in widget pixels.
    pub(super) fn ray_through(&self, pixel: Vec2) -> (Vec3, Vec3) {
        let aspect = self.viewport_size.x / self.viewport_size.y.max(1.0);
        let ndc = OrbitCamera::ndc_from_pixels(pixel, self.viewport_size);
        self.camera.ray(ndc, aspect)
    }

    /// Where the cursor meets the ACTIVE body's surface, if it does.
    ///
    /// A live stroke keeps carving the body it started on however many others
    /// the cursor passes over, so this asks one body rather than picking. It
    /// goes through [`Document::pick_body`] rather than calling `raycast`
    /// directly, so that it shares the box gate and the box advance with the
    /// picker: two marches that disagree about where the surface is would put
    /// the ring in one place and the stroke in another.
    pub(super) fn surface_under(&self, pixel: Vec2) -> Option<Vec3> {
        let (origin, ray) = self.ray_through(pixel);
        let timed = self.phases.enter(phases::Phase::Pick);
        let hit = self
            .doc
            .pick_body(self.doc.active(), origin, ray, self.camera.far())
            .map(|hit| hit.position);
        self.phases.leave(timed);
        hit
    }
}
