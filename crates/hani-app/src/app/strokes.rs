// SPDX-License-Identifier: AGPL-3.0-only

//! A stroke: the brush's size and strength, the press, the sculpt, mask,
//! paint and move drags, the pen, and the stroke's end. Moved out of
//! `app.rs` (issue 189).

use super::*;

impl Hani {
    /// Count the stamps just laid into the frame being timed.
    fn count_stamps(&self) {
        let stamps = self.perf.stamps as u32;
        self.phases.count(|frame| frame.stamps += stamps);
    }

    /// Keep the brush a constant fraction of the model when asked to.
    ///
    /// ZBrush calls this Dynamic. Without it a brush tuned on a 60 mm ball is
    /// the wrong size the moment the model is resampled or grows.
    ///
    /// **This must never run because the SELECTION changed.** It exists to
    /// survive a *resample*, where the geometry moved under a fixed brush;
    /// choosing a different body changes no geometry at all. Once
    /// `model_radius` means the active body's, clicking a 5 mm rivet beside a
    /// 200 mm bust with Dynamic on would take a 5 mm brush to `5 * 0.025 =
    /// 0.125`, clamped up to [`MIN_RADIUS_MM`], and clicking back would take it
    /// to `0.25 * 40 = 10 mm` -- the brush doubled and the user never touched
    /// the slider. Worse, the press that resized it is the same press that
    /// selects, so the next press carves at a radius they never chose.
    /// Selection changes go through [`Hani::refresh_model_radius`] instead,
    /// which re-measures for the camera and the mirror plane and leaves the
    /// brush alone.
    pub(super) fn rescale_radius(&mut self, previous_model_radius: f32) {
        if !self.dynamic_radius || previous_model_radius <= 0.0 || self.model_radius <= 0.0 {
            return;
        }
        let factor = self.model_radius / previous_model_radius;
        if (factor - 1.0).abs() > 1.0e-4 {
            self.brush.radius =
                (self.brush.radius * factor).clamp(MIN_RADIUS_MM, self.max_radius());
        }
    }

    /// The largest brush radius that is usable at the current voxel size.
    ///
    /// The lower of the millimetre ceiling and the voxel one. See
    /// [`MAX_RADIUS_VOXELS`] for why the second exists: at a resin voxel the
    /// millimetre ceiling alone reaches a brush that takes a quarter of a
    /// second per stamp.
    pub(crate) fn max_radius(&self) -> f32 {
        MAX_RADIUS_MM.min(MAX_RADIUS_VOXELS * self.doc.voxel_size()).max(MIN_RADIUS_MM)
    }

    /// Open the undo recorder on the body this stroke is about to write to, if
    /// a press has not already opened it.
    ///
    /// **This is called where the edit happens and not where the button goes
    /// down, and that ordering is the whole point.** Recording used to open in
    /// the `Pressed` arm before anything had been raycast, which with more than
    /// one body means opening it on the wrong one -- and `record_for_undo` does
    /// nothing at all when the volume it is called on has no recorder, so the
    /// carve would land, no history entry would be pushed, `unsaved` would stay
    /// false, and quitting would not even raise the discard prompt. One press,
    /// unrecoverable work.
    ///
    /// Lazy rather than moved into the press for one behaviour worth keeping: a
    /// press that starts off the model and drags onto it sculpts, and there is
    /// no body to open a recorder on until it arrives.
    pub(super) fn arm_recorder(&mut self) {
        let volume = self.doc.active_volume_mut();
        if !volume.is_recording() {
            volume.begin_stroke();
        }
    }

    /// Apply the brush along the stroke path up to the point under the cursor.
    ///
    /// The stroke walks from its previous stamp to the new one at a fixed
    /// spacing, so a fast drag lays a continuous cut instead of a dotted trail.
    /// The stamps are applied one after another rather than batched, because
    /// each one has to see the field the previous one left behind.
    ///
    /// Move is the exception, and takes none of that path. It locked the field
    /// when the button went down and re-warps that same copy by the whole drag
    /// so far, so it wants the raw pointer position and one pass, not a trail
    /// of stamps that each build on the last. See `MoveStroke`.
    /// Index of a brush in the per-kind strength table.
    pub(super) fn strength_slot(kind: BrushKind) -> usize {
        BrushKind::ALL.iter().position(|candidate| *candidate == kind).unwrap_or(0)
    }

    /// Where the LIVE tool's strength lives in the table.
    ///
    /// The mask has a slot of its own; the cut has none and borrows the
    /// brush's, which is correct because arming the cut must not disturb what
    /// the brush was set to and a cut has no strength to remember.
    pub(super) fn live_strength_slot(&self) -> usize {
        match self.tool {
            Tool::Mask => MASK_STRENGTH_SLOT,
            Tool::Sculpt | Tool::Cut | Tool::Transform => Self::strength_slot(self.brush.kind),
        }
    }

    /// The highest strength the live tool allows. See [`MAX_MASK_STRENGTH`].
    pub(crate) fn max_strength(&self) -> f32 {
        match self.tool {
            Tool::Mask => MAX_MASK_STRENGTH,
            Tool::Sculpt | Tool::Cut | Tool::Transform => MAX_STRENGTH,
        }
    }

    /// What the right-click menu's heading should read.
    ///
    /// The menu is contextual on the live tool, so in mask mode it has to say
    /// MASK -- and it has to say BLUR while shift is held, which is the same
    /// live substitution the strip makes and the reason this is not just
    /// `self.tool`.
    pub(crate) fn live_tool_label(&self) -> String {
        match (self.tool, self.shift) {
            (Tool::Mask, true) => "MASK — BLUR".to_string(),
            (Tool::Mask, false) => "MASK".to_string(),
            // Shape-aware, through the same live substitution the mask's BLUR
            // uses: the tool card has to say what the gesture in progress is
            // going to do, and "PLANE CUT" over a lasso is a lie about which
            // material is about to go. Before a drag starts there is no shape
            // yet and it says what an unqualified cut is.
            // Alt keeps both sides, and the card says so WHILE it is held,
            // for the reason shift's preview is live: a verb read from a key
            // at the release has to be visible before the release.
            (Tool::Cut, _) if self.parting_cut() => "PART — KEEP BOTH".to_string(),
            // See `app/ring_cut.rs`.
            (Tool::Cut, _) if self.rings() => "RING CUT".to_string(),
            (Tool::Cut, _) => self.live_cut_shape().label().to_string(),
            (Tool::Transform, _) => "MOVE".to_string(),
            (Tool::Sculpt, _) => self.effective_brush().kind.label().to_uppercase(),
        }
    }

    /// Put the live strength away and bring out the one the incoming tool or
    /// brush was last set to.
    ///
    /// One function and two callers -- a brush change and a tool change --
    /// because the swap has to happen on both and two copies of it would drift.
    ///
    /// **Deliberately does not clamp on the way out.** `MAX_STRENGTH` is the
    /// slider's ceiling and not a hard one: `BrushKind::Move`'s own default is
    /// 1.0, deliberately above it, because for Move strength is the fraction of
    /// the drag the surface follows and a grab tool at 0.15 crawls. Clamping
    /// here would silently cap Move at 0.80 on every tool change, which
    /// `each_brush_remembers_its_own_strength` catches. Nothing needs the
    /// clamp anyway: the mask's 1.0 lives in its own slot and is never loaded
    /// into a brush's.
    pub(super) fn swap_strength(&mut self, into: impl FnOnce(&mut Self)) {
        self.strengths[self.live_strength_slot()] = self.brush.strength;
        into(self);
        self.brush.strength = self.strengths[self.live_strength_slot()];
    }

    /// Where the pointer is, in the plane through `through` that faces the
    /// camera.
    ///
    /// This is what makes a grab feel like a grab. Raycasting the pointer onto
    /// the surface instead gives a target that crawls ALONG the form: drag
    /// sideways across a ball and the hit point slides around its curve, so the
    /// vector from the grab point stays short and keeps turning, and the result
    /// is a smear rather than a pull. It is also a feedback loop, because the
    /// surface being raycast is the one currently being deformed.
    ///
    /// Projecting into the view plane instead means the surface follows the
    /// cursor the way the hand expects, in the plane of the screen, and it
    /// keeps working when the cursor is dragged off the model entirely -- which
    /// is exactly when a grab is most useful.
    fn view_plane_point(&self, pixel: Vec2, through: Vec3) -> Vec3 {
        let (origin, ray) = self.ray_through(pixel);
        let Some(facing) = (self.camera.target - self.camera.eye()).try_normalize() else {
            return through;
        };
        let denominator = ray.dot(facing);
        // The ray is parallel to the plane, which cannot happen for a pointer
        // inside the viewport but is cheap to refuse.
        if denominator.abs() < 1.0e-6 {
            return through;
        }
        origin + ray * ((through - origin).dot(facing) / denominator)
    }

    /// One event of a Move gesture.
    ///
    /// Kept apart from `sculpt_to` because Move shares almost none of it: no
    /// stroke interpolation, no per-stamp normals, no trail of stamps building
    /// on each other. It locks the field once and re-warps that copy by the
    /// whole drag so far.
    fn move_to(&mut self, pixel: Vec2, start: bool) {
        let brush = self.effective_brush();
        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let started = Instant::now();

        // The anchor is the one thing that needs the surface, and it is taken
        // once. After that the gesture is about the pointer.
        if start || !self.move_stroke.is_active() {
            let Some(point) = self.surface_under(pixel) else {
                return;
            };
            // Past the last early return, so the recorder opens on the body
            // that is about to be warped and not a moment sooner.
            self.arm_recorder();
            self.move_stroke.begin(
                self.doc.active_volume(),
                &brush,
                point,
                self.framed_symmetry(),
                MIRROR_CENTRE,
            );
            self.move_grab = Some(point);
        }
        let Some(grab) = self.move_grab else {
            return;
        };

        let target = self.view_plane_point(pixel, grab);
        self.move_stroke.drag_to(self.doc.active_volume_mut(), target, pressure);

        // Re-anchor once this lock has warped as far as it safely can, and let
        // the drag carry on from where the material now is.
        //
        // Without this a gesture stops dead at `Brush::max_drag`, which is a
        // quarter of the fold threshold and only a millimetre and a half for
        // the default 3 mm brush -- the surface simply refuses to follow the
        // cursor any further, which is exactly the "it does something, but not
        // what I expected" complaint. The cap itself cannot just be raised: past
        // the fold threshold a domain warp turns the field back through itself.
        // Chaining fold-safe warps gives unlimited reach without ever crossing
        // it. ZBrush can drag as far as it likes because it moves mesh vertices,
        // which stretch; a field has to be re-locked instead.
        //
        // Safe inside one stroke because `record_for_undo` captures a brick the
        // first time it is touched and not again, so a chain of locks is still
        // one undo entry.
        if self.move_stroke.is_at_the_limit() {
            let carried = grab + self.move_stroke.applied();
            self.move_stroke.begin(
                self.doc.active_volume(),
                &brush,
                carried,
                self.framed_symmetry(),
                MIRROR_CENTRE,
            );
            self.move_grab = Some(carried);
        }

        self.perf.stamps = 1;
        self.count_stamps();
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.remesh_dirty();
        self.hover = Some(grab);
        self.refresh_overlay();
    }

    /// What a left press means while the sculpt tool is live.
    ///
    /// Lifted out of the press dispatch when the cut and the mask collapsed
    /// into one match on [`Tool`]: three arms of body-resolution reasoning
    /// nested inside a `match` on the tool inside a `match` on the button is
    /// three levels deep and reads as none of them.
    ///
    /// **The body is resolved from the raycast BEFORE anything opens a
    /// recorder**; see `arm_recorder`. A press over a body that is not the
    /// active one selects it and carves nothing, and the press after that
    /// sculpts -- the same rule Photoshop's layer panel has, applied to the
    /// model itself so that direct manipulation acts on what is drawn. A press
    /// over a HIDDEN body picks nothing, because `Document::pick` never marches
    /// one.
    pub(super) fn sculpt_press(&mut self, position: Vec2) -> DragKind {
        match self.pick(position) {
            Some((body, _)) if body != self.doc.active() => {
                self.select_body(body);
                DragKind::Selecting
            }
            Some(_) => {
                self.warn_if_masked(position);
                DragKind::Sculpt(self.stroke_direction())
            }
            // Nothing drawn under the pointer. That is normally a press beside
            // the model, which is allowed to become a stroke the moment it is
            // dragged onto one -- but not when the body it would carve is itself
            // hidden. See `active_is_drawn`: the stroke's own raycast does not
            // consult the eye, so without this the press carves a body nobody
            // can see. Saying which body is the honest half; a press that
            // silently does nothing is its own puzzle.
            //
            // **And it must say which of the two hid it.** Under solo, the
            // active body can be undrawn with its own eye plainly open --
            // clicking a row outside the scope selects it, and `select_body`
            // does not veto that, because a view mode never vetoes a structural
            // operation. "Show it before sculpting it" would then be advice for
            // a state the user is not in, pointing at an eye that is already on.
            None if !self.active_is_drawn() => {
                let active = self.doc.active();
                let name =
                    self.doc.node(active).map_or("the active body", |node| node.name.as_str());
                self.status = if self.in_solo_scope(active) {
                    format!("{name} is hidden — show it before sculpting it")
                } else {
                    format!("{name} is outside the solo scope — escape leaves solo")
                };
                DragKind::Refused
            }
            None => DragKind::Sculpt(self.stroke_direction()),
        }
    }

    /// Say so when a sculpt press lands on protected material.
    ///
    /// **The one thing that stops a mask reading as a broken brush.** A stroke
    /// over a fully protected surface leaves the field bit-identical -- that is
    /// the whole point of the mask -- and with nothing on screen saying why, the
    /// brush looks broken. The mask was very possibly painted minutes ago, or on
    /// another body, so there is nothing under the cursor for the user to
    /// connect it to. The cut has the same line for the same reason; see
    /// `finish_cut`.
    ///
    /// Answered at the press only, from the ONE voxel under the cursor, and
    /// both halves are the point: a per-stamp check would be a map probe in the
    /// stroke loop, and a claim about the whole footprint would need a pass over
    /// it. So it is deliberately narrow -- "the surface you pressed on is fully
    /// protected" -- and stays silent about a stroke that is merely weakened,
    /// which the ring and the tint report instead.
    fn warn_if_masked(&mut self, pixel: Vec2) {
        let volume = self.doc.active_volume();
        if volume.mask().is_free() {
            return;
        }
        let Some(point) = self.surface_under(pixel) else {
            return;
        };
        let cell = (point / self.doc.voxel_size()).round().as_ivec3();
        if self.doc.active_volume().mask().at(cell) != hani_core::PROTECTED {
            return;
        }
        let active = self.doc.active();
        let name = self.doc.node(active).map_or("this body", |node| node.name.as_str()).to_string();
        self.status = format!("the mask on {name} protects this — m, then ctrl drag, unmasks");
    }

    /// Which way a mask stroke is going, and whether it is blurring instead.
    ///
    /// **Its own computation, and NOT `stroke_direction`.** That one gates on
    /// `brush.kind.is_directional()`, which answers false for Smooth, Flatten
    /// and Move -- so reusing it would make ctrl+drag in mask mode *mask*
    /// instead of unmasking for three of the seven brushes, with nothing on
    /// screen to say which three. This is two lines that never consult
    /// `brush.kind`, and the stylus eraser end falls out unmasking for free.
    ///
    /// Shift outranks the pair, which mirrors the field side: shift is Smooth
    /// there whatever the invert modifier says, and it is Blur here.
    pub(super) fn mask_op(&self) -> MaskOp {
        if self.shift {
            return MaskOp::Blur;
        }
        // Either modifier, and they do NOT compound -- holding both is still one
        // inversion, exactly as `stroke_direction` reads them.
        let unmask = (self.control || self.alt) != self.eraser_in_use();
        if unmask { MaskOp::Lower } else { MaskOp::Raise }
    }

    /// Paint the mask along the stroke path up to the point under the cursor.
    ///
    /// The sculpt path's twin, and deliberately its shape rather than a new
    /// one: the same [`Stroke`] walker at the same spacing, the same lazy
    /// recorder, the same pressure sample once per event. Masking is per body
    /// and lands on the ACTIVE one, so it asks `surface_under` rather than
    /// picking -- the same rule a live sculpt stroke follows once it has begun.
    ///
    /// Move has no analogue here and needs none: the mask is a value at a
    /// lattice cell, not a shape to warp, and there is nothing for a locked copy
    /// to be a copy of.
    pub(super) fn mask_to(&mut self, pixel: Vec2, start: bool) {
        let Some(point) = self.surface_under(pixel) else {
            // The cursor ran off the model. The stroke stays live so coming back
            // onto it continues rather than restarting, but nothing is painted
            // in mid air.
            return;
        };

        // Past the early return, for the same reason the sculpt path is: the
        // recorder opens on the body about to be painted and only once there is
        // something to paint. `end_stroke` then produces ONE entry carrying the
        // mask bricks, which is what makes a mask-only gesture undoable and
        // what sets `unsaved`.
        self.arm_recorder();

        let started = Instant::now();
        self.stamp_centres.clear();
        if start {
            self.stroke.begin(point, &mut self.stamp_centres);
            // A stroke's own record of what it gave: see `mask_stroke`.
            self.mask_stroke.begin();
        } else {
            let spacing = self.brush.spacing(self.doc.voxel_size());
            self.stroke.advance(point, spacing, &mut self.stamp_centres);
        }

        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let op = self.mask_op();
        // `self.brush` and not `effective_brush`: shift means Blur here, and
        // asking for the effective brush would hand the mask painter Smooth's
        // kind, which it does not read, and Smooth's strength, which it must
        // not -- the mask keeps its own.
        let brush = self.brush;
        let lean = self.pen_lean();
        let tangent = self.stroke.direction().unwrap_or(Vec3::ZERO);

        for index in 0..self.stamp_centres.len() {
            let centre = self.stamp_centres[index];
            let normal = lean_normal(self.doc.active_volume().gradient_world(centre), lean);
            // The direction a mask stamp carries is never read -- protection has
            // no add and subtract, `op` says which way this goes -- so `Add` is
            // the value that cannot be wrong rather than a choice.
            let stamp = Stamp::new(centre, normal, BrushDirection::Add)
                .with_pressure(pressure)
                .with_tangent(tangent);
            // By the stroke, so it fades to nothing at the radius rather
            // than settling solid nearly to it: `hani_core::mask_stroke`.
            let symmetry = self.framed_symmetry();
            self.mask_stroke.stamp(
                &brush,
                self.doc.active_volume_mut(),
                &stamp,
                op,
                symmetry,
                MIRROR_CENTRE,
                &mut self.brush_scratch,
            );
        }

        self.perf.stamps = self.stamp_centres.len();
        self.count_stamps();
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.remesh_dirty();
    }

    /// Paint the chosen filament slot along the stroke path up to the point
    /// under the cursor.
    ///
    /// `mask_to`'s twin, and the same shape for the same reasons: the same
    /// stroke walker at the same spacing, the same lazy recorder, and the
    /// active body only. The inverted direction is the eraser, which writes
    /// [`hani_core::colour::UNPAINTED`]; there is no third operation.
    fn paint_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        let Some(point) = self.surface_under(pixel) else {
            return;
        };
        // The eyedropper, before anything is armed: taking a colour is not
        // an edit. See `app/painting.rs`.
        if self.eyedropper_takes(point, start) {
            return;
        }
        // A hidden or locked layer takes no paint. See `app/layers.rs`.
        if self.layer_refuses_paint() {
            return;
        }
        // The airbrush sprays by the frame, not by the stamp. See
        // `app/airbrush.rs`.
        if self.airbrushing() {
            self.airbrush_to(pixel, direction, start);
            return;
        }
        // A decal is dabbed, not stroked. See `app/decal.rs`.
        if self.dabbing() {
            self.decal_to(pixel, start);
            return;
        }
        self.arm_recorder();

        let started = Instant::now();
        self.stamp_centres.clear();
        if start {
            self.stroke.begin(point, &mut self.stamp_centres);
        } else {
            let spacing = self.brush.spacing(self.doc.voxel_size());
            self.stroke.advance(point, spacing, &mut self.stamp_centres);
        }

        // A fill is not a stroke: the paint is poured where the pointer is.
        // See `app/painting.rs`.
        if self.touch.kind == crate::message::PaintTouch::Fill {
            self.pour_colour(point, direction);
            self.remesh_dirty();
            return;
        }
        let lean = self.pen_lean();
        let tangent = self.stroke.direction().unwrap_or(Vec3::ZERO);
        // Pressure is sampled so the readout stays honest; the brush does not
        // read it for strength, because a slot has no half. See
        // `BrushKind::Paint`. It can size the brush: see `app/studio.rs`.
        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let brush = self.studio.sized(self.brush, pressure);

        for index in 0..self.stamp_centres.len() {
            let centre = self.stamp_centres[index];
            let normal = lean_normal(self.doc.active_volume().gradient_world(centre), lean);
            let stamp =
                Stamp::new(centre, normal, direction).with_pressure(pressure).with_tangent(tangent);
            // A loaded brush is upstream's stamp, reached through the coat;
            // a drybrush or a sponge is the same stamp asked one more
            // question per voxel. See `app/paints.rs`.
            let coat = self.coat_for(centre, start && index == 0);
            // A stamp of the colour brush. See `app/painting.rs`.
            self.lay_colour(&brush, &stamp, coat, direction, start && index == 0);
        }

        self.perf.stamps = self.stamp_centres.len();
        self.count_stamps();
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.remesh_dirty();
    }

    pub(super) fn sculpt_to(&mut self, pixel: Vec2, direction: BrushDirection, start: bool) {
        // A shape or a cable being added is a gesture of its own too. See
        // `app/adding.rs`.
        if self.inserting() {
            self.add_to(pixel, direction, start);
            return;
        }
        // A stamp is a gesture of its own and not a stroke of any brush.
        // See `app/studio.rs`.
        if self.studio.stamping() {
            self.stamp_to(pixel, direction, start);
            return;
        }
        // Move is handled before anything else, because it wants the pointer
        // rather than the surface under the pointer, and it must keep working
        // once the cursor has been dragged off the model.
        if self.effective_brush().kind == BrushKind::Move {
            self.move_to(pixel, start);
            return;
        }
        // After Move, which wants the pointer itself: every stamping brush
        // can be led. See `app/studio.rs`.
        let pixel = self.studio.follow(pixel, start);
        // Paint has no field path either. `effective_brush` so that shift
        // still smooths with the paint brush selected, as it does with every
        // other.
        if self.effective_brush().kind == BrushKind::Paint {
            self.paint_to(pixel, direction, start);
            return;
        }

        let Some(point) = self.surface_under(pixel) else {
            // The cursor ran off the model. The stroke stays live so coming
            // back onto it continues rather than restarting, but nothing is
            // stamped in mid air.
            return;
        };

        // Past the early return, so the recorder opens on the body that is
        // about to be carved and only once there is something to carve. See
        // `arm_recorder`.
        self.arm_recorder();
        // A stroke that lays one layer is aimed at the surface as it was
        // before the stroke, and faces the way that did. See
        // `app/studio.rs`.
        let point = self.layer_aim(pixel, point);

        let started = Instant::now();
        self.stamp_centres.clear();
        if start {
            self.stroke.begin(point, &mut self.stamp_centres);
        } else {
            // Upstream's spacing, or the studio's for a brush set down in
            // dabs. See `app/studio.rs`.
            let spacing = self.stroke_spacing();
            self.stroke.advance(point, spacing, &mut self.stamp_centres);
        }

        // Sampled once for the whole event rather than per stamp: the pen has
        // not moved between the stamps that one pointer event interpolates, so
        // re-reading it would only add jitter.
        let pressure = self.tablet.stamp_pressure(self.pressure_enabled, self.pressure_curve);
        let brush = self.effective_brush();

        {
            // A locked copy of the field is only good while nothing else writes
            // over it, and this is about to.
            self.move_stroke.end();
            self.move_grab = None;

            // Not for a brush that levels to a plane: `app/pen_strokes.rs`.
            let lean = self.stroke_lean(self.pen_lean());
            // Which way the drag is going, for the patterns that comb. Zero
            // until the stroke has moved far enough to have a direction, which
            // the pattern copes with by picking any direction across the
            // surface.
            let tangent = self.stroke.direction().unwrap_or(Vec3::ZERO);

            for index in 0..self.stamp_centres.len() {
                let centre = self.stamp_centres[index];
                // Take the normal from the field at each stamp rather than
                // reusing the one from the raycast, so a stroke curving around
                // a form stays oriented to the surface it is actually on.
                // Leaning the pen rotates the direction the brush pushes in,
                // which steers every brush at once because they all read this
                // normal.
                let normal = lean_normal(self.stamp_facing(centre), lean);
                let stamp = Stamp::new(centre, normal, direction)
                    .with_pressure(pressure)
                    .with_tangent(tangent);
                // Upstream's stamp, or the studio's generic brush. See
                // `app/studio.rs`.
                self.stamp_field(brush, &stamp);
            }
            self.perf.stamps = self.stamp_centres.len();
            self.count_stamps();
        }
        // Paint carried onto the surface the stamps made: `app/paint_carry.rs`.
        self.carry_paint_after_stamps(brush, pressure);
        self.perf.pressure = pressure;
        self.perf.edit_ms = started.elapsed().as_secs_f32() * 1000.0;

        self.remesh_dirty();
    }

    /// The brush a stamp actually runs with.
    ///
    /// Holding shift smooths, which is the convention every sculpting tool
    /// uses and the single biggest ergonomic win available here. Shift already
    /// modified right drag into a pan, and this is left drag, so there is no
    /// clash.
    ///
    /// Deliberately does NOT swap `self.brush.kind` and swap it back: nothing
    /// then has to notice a key being released while the window is unfocused,
    /// and the tool strip does not flicker between two highlights during a
    /// stroke. The selection is never touched, so there is nothing to restore.
    ///
    /// **Shift over the paint brush smooths at Smooth's OWN remembered
    /// strength**, not the paint brush's. Paint's strength is a fixed 1.0 the
    /// panel hides -- a slot has no half -- and carrying it into Smooth meant
    /// shift-drag smoothed at 2.5x Smooth's default and above the slider's
    /// ceiling, with no control on screen to say so. Every other brush keeps
    /// its own strength under shift, as before.
    pub(super) fn effective_brush(&self) -> Brush {
        if !self.shift {
            return self.brush;
        }
        let strength = if self.brush.kind == BrushKind::Paint {
            self.strengths[Self::strength_slot(BrushKind::Smooth)]
        } else {
            self.brush.strength
        };
        Brush { kind: BrushKind::Smooth, strength, ..self.brush }
    }

    /// Direction for a new stroke, honouring the invert modifier, the eraser
    /// end of the stylus, and the fact that some brushes have no opposite.
    ///
    /// The two inverts combine rather than override: holding the modifier while
    /// using the eraser gives back the additive brush, which is the same
    /// behaviour every drawing application has.
    pub(super) fn stroke_direction(&self) -> BrushDirection {
        // Either modifier inverts. Alt is what ZBrush and Nomad both use, and
        // control is what this had first; keeping both costs nothing and means
        // neither habit is wrong. They do NOT compound -- holding both is still
        // one inversion, because a user pressing both means "invert", not
        // "invert twice".
        let modifier = self.control || self.alt;
        let inverted = modifier != self.eraser_in_use();
        if inverted && self.brush.kind.is_directional() {
            BrushDirection::Subtract
        } else {
            BrushDirection::Add
        }
    }

    /// Whether the eraser end of the stylus is the one in range.
    fn eraser_in_use(&self) -> bool {
        let pen = self.tablet.state();
        pen.in_proximity && pen.eraser
    }

    /// The world space lean of the pen, as a vector whose length is the tilt
    /// angle in radians.
    ///
    /// Tilt arrives in the tablet's own frame, which lines up with the screen,
    /// so it has to be carried into world space through the camera basis before
    /// it can steer anything.
    pub(super) fn pen_lean(&self) -> Vec3 {
        if !self.tilt_enabled {
            return Vec3::ZERO;
        }
        let pen = self.tablet.state();
        if !pen.in_proximity {
            return Vec3::ZERO;
        }

        let magnitude = pen.tilt.length().min(1.0);
        if magnitude < 1.0e-4 {
            return Vec3::ZERO;
        }

        // Screen y grows downward and the camera's up axis grows upward, and a
        // positive tilt on that axis means the pen is leaning toward the user,
        // which is toward the bottom of the screen. Hence the subtraction.
        let direction =
            (self.camera.right() * pen.tilt.x - self.camera.up() * pen.tilt.y).normalize_or_zero();
        direction * (magnitude * MAX_TILT)
    }

    pub(super) fn finish_stroke(&mut self) {
        let timed = self.phases.enter(phases::Phase::Finish);
        self.end_the_stroke();
        self.phases.leave(timed);
    }

    /// The end of a stroke itself. [`Hani::finish_stroke`] is this, timed.
    fn end_the_stroke(&mut self) {
        self.stroke.end();
        // Releases the locked field. One gesture is one lock, so a new drag
        // starts from the surface as it now stands rather than re-warping the
        // last one's copy.
        self.move_stroke.end();
        self.move_grab = None;
        let body = self.doc.active();
        // A decal dragged out is laid as the button comes up, and its new
        // layer goes in the same entry as its dab. See `app/decal.rs`.
        self.set_the_decal_down();
        let edit = self.doc.active_volume_mut().end_stroke();
        if let Some(entry) = self.stroke_entry(body, edit) {
            let before = self.history.stats();
            self.history.push(entry);
            self.record_history(before);
            // Inside the guard on purpose: a press and release that never
            // touched the model produces no edit, and must not raise a
            // "discard your work?" prompt on the way out.
            self.unsaved = true;
            // And in the same guard, for the same reason, the picture. This is
            // the "on stroke END, never per stamp" half of the refresh rule --
            // `remesh_dirty` deliberately skips a sculpt drag, so a five second
            // stroke queues exactly one render rather than one per pointer
            // event.
            self.thumbs.geometry_changed(body);
        }
        // What a stroke's remeshes left unmeasured. See `remesh_dirty`.
        if self.measures_stale {
            self.measure_document();
        }
    }
}

/// The paint brush through the application: the digit, the drag, the eraser,
/// the undo, and what the shader is told.
#[cfg(test)]
mod paint_brush_tests {
    use super::*;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// The paint brush in hand. The colour brush's own tests are in
    /// `app/painting.rs`.
    fn app() -> Hani {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
    }

    fn centre_of_viewport() -> Vector {
        Vector::new(SIZE.x / 2.0, SIZE.y / 2.0)
    }

    fn press(app: &mut Hani, at: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
    }

    fn drag_to(app: &mut Hani, at: Vector) {
        update(app, Message::Pointer(PointerEvent::Moved { position: at, size: SIZE }));
    }

    fn release(app: &mut Hani) {
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// A short drag across the middle of the model.
    fn paint_drag(app: &mut Hani) {
        let centre = centre_of_viewport();
        press(app, centre);
        drag_to(app, Vector::new(centre.x + 20.0, centre.y));
        drag_to(app, Vector::new(centre.x + 40.0, centre.y));
        release(app);
    }

    /// Which change of the active body's paint it is on: what a view toggle
    /// must not move.
    fn painted_bricks(app: &Hani) -> u64 {
        app.doc.active_volume().paint().revision()
    }

    #[test]
    fn a_paint_drag_moves_no_field_value() {
        let mut app = app();
        let before: Vec<f32> = {
            let volume = app.doc.active_volume();
            let mut coords: Vec<_> = volume.brick_coords().collect();
            coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
            coords
                .iter()
                .flat_map(|coord| {
                    let origin = coord.origin();
                    (0..32).map(move |i| volume.sample_voxel(origin + glam::IVec3::new(i, i, i)))
                })
                .collect()
        };
        paint_drag(&mut app);
        let after: Vec<f32> = {
            let volume = app.doc.active_volume();
            let mut coords: Vec<_> = volume.brick_coords().collect();
            coords.sort_by_key(|coord| (coord.0.z, coord.0.y, coord.0.x));
            coords
                .iter()
                .flat_map(|coord| {
                    let origin = coord.origin();
                    (0..32).map(move |i| volume.sample_voxel(origin + glam::IVec3::new(i, i, i)))
                })
                .collect()
        };
        assert_eq!(before.len(), after.len(), "a paint stroke added or removed field bricks");
        assert!(
            before.iter().zip(&after).all(|(a, b)| a.to_bits() == b.to_bits()),
            "a paint stroke moved the field"
        );
    }

    /// Shift over the paint brush smooths at Smooth's own strength, within
    /// the slider's ceiling. Paint's 1.0 is not a sculpting strength.
    #[test]
    fn shift_over_the_paint_brush_smooths_at_smooths_own_strength() {
        let mut app = app();
        assert_eq!(app.brush.strength, 1.0, "the fixture expects paint's fixed 1.0");
        app.shift = true;
        let effective = app.effective_brush();
        assert_eq!(effective.kind, BrushKind::Smooth);
        assert!(effective.strength <= MAX_STRENGTH, "shift-smooth ran at {}", effective.strength);
        assert_eq!(effective.strength, BrushKind::Smooth.default_strength());
    }

    /// A palette change recolours the viewport through the uniforms every
    /// message; the thumbnails are drawn on brick changes, so they have to be
    /// told.
    #[test]
    fn syncing_the_palette_marks_every_thumbnail_stale() {
        let mut app = app();
        paint_drag(&mut app);
        app.thumbs.settle_now();
        while let Some(body) = app.thumbs.next_stale() {
            app.thumbs.requested(body);
        }
        assert_eq!(app.thumbs.stale_count(), 0, "the fixture should start with nothing stale");

        let reported = vec![crate::printer::Filament {
            vendor: "Different".to_string(),
            material: "PLA".to_string(),
            sub_type: String::new(),
            colour: "#123456".to_string(),
            present: true,
        }];
        update(&mut app, Message::PaletteSynced(Ok(Some(reported))));
        assert!(app.thumbs.stale_count() > 0, "a palette change left the thumbnails as they were");
    }

    /// The toggle reaches the shader and touches nothing else: no brick dirty,
    /// no document change, no slot moved.
    #[test]
    fn the_show_paint_toggle_reaches_the_renderer_and_dirties_nothing() {
        let mut app = app();
        paint_drag(&mut app);
        let bricks = painted_bricks(&app);
        app.unsaved = false;
        let mut dirty = Vec::new();
        app.doc.active_volume_mut().take_dirty(&mut dirty);

        assert!(app.shared.paint_view().shown, "paint should start shown");
        update(&mut app, Message::ShowPaintToggled);
        assert!(!app.shared.paint_view().shown, "the toggle did not reach the shared frame");
        assert!(!app.unsaved, "a view toggle dirtied the document");
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "a view toggle dirtied bricks");
        assert_eq!(painted_bricks(&app), bricks, "a view toggle changed the paint");

        // And the palette the shader sees is the machine's, slot by slot.
        let palette = app.shared.paint_view().palette;
        assert_eq!(palette[1], app.palette.slots[0].swatch().into_linear());
    }

    /// The crease toggle reaches the shader AND the meshes, and still touches
    /// nothing a save or an undo would notice.
    ///
    /// The meshes are the half a uniform-only toggle would get wrong: the
    /// signal is measured at mesh time, so a toggle that only wrote the
    /// uniform would switch the shader on over vertices that carry nothing and
    /// the model would not change at all. That shipped twice in this project's
    /// history for other features, as a control that was green and dead.
    #[test]
    fn the_show_creases_toggle_remeshes_with_the_signal_and_changes_nothing_else() {
        let mut app = app();
        app.unsaved = false;
        let undo_before = app.history.stats();
        assert_eq!(app.shared.cavity_view().strength, 0.0, "creases should start hidden");
        let published_off = app.shared.pending_cavity_for_tests();
        assert!(
            published_off.iter().any(|(vertices, _)| *vertices > 0),
            "the fixture published no geometry at all"
        );
        assert!(
            published_off.iter().all(|(_, cavity)| *cavity == 0),
            "a mesh carried cavity bytes before anything asked for them"
        );

        update(&mut app, Message::ShowCavityToggled);
        let view = app.shared.cavity_view();
        assert_eq!(view.strength, CAVITY_STRENGTH, "the toggle did not reach the shared frame");
        assert_eq!(
            view.gain,
            CAVITY_FEATURE_MM / app.doc.voxel_size() / 4.0,
            "the gain is not the feature size over the voxel size"
        );
        let published_on = app.shared.pending_cavity_for_tests();
        let fresh = &published_on[published_off.len()..];
        let with_geometry = fresh.iter().filter(|(vertices, _)| *vertices > 0).count();
        assert!(with_geometry > 0, "switching creases on remeshed nothing");
        assert!(
            fresh.iter().all(|(vertices, cavity)| vertices == cavity),
            "a mesh published after the toggle does not carry one cavity byte per vertex"
        );

        assert!(!app.unsaved, "a view toggle dirtied the document");
        assert_eq!(app.history.stats(), undo_before, "a view toggle reached the history");
        assert_eq!(app.doc.active_volume().dirty_count(), 0, "the remesh left bricks dirty");

        // And off again stops paying for it.
        update(&mut app, Message::ShowCavityToggled);
        assert_eq!(app.shared.cavity_view().strength, 0.0, "the toggle did not switch back off");
        let published_again = app.shared.pending_cavity_for_tests();
        let fresh = &published_again[published_on.len()..];
        assert!(fresh.iter().any(|(vertices, _)| *vertices > 0), "switching off remeshed nothing");
        assert!(
            fresh.iter().all(|(_, cavity)| *cavity == 0),
            "meshes still carry cavity bytes after the toggle was switched off"
        );
    }
}
