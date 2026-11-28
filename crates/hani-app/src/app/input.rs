// SPDX-License-Identifier: AGPL-3.0-only

//! Input: a pointer event and the moves batched to the frame, a key press,
//! whether a modal owns the input, the sizing gesture, and the puck. Moved
//! out of `app.rs` (issue 189).

use super::*;

/// How fast a hold-and-drag moves the brush numbers.
///
/// Radius is in log space so the same drag is the same proportion at any size;
/// about 250 px covers the whole range, which is a comfortable sweep.
const RADIUS_PER_PIXEL: f32 = 0.006;

const STRENGTH_PER_PIXEL: f32 = 0.002;

/// How many pointer moves are kept for the next frame before they are handled
/// without one. See [`Hani::pointer_in`].
const MOST_MOVES_KEPT: usize = 64;

const CLICK_MS: u128 = 250;

impl Hani {
    /// A pointer event, counted into the frame being timed. A stroke is
    /// noted on both sides of it, so that the press and the release are in a
    /// stroke's frames with the moves between them. See `app/phases.rs`.
    ///
    /// **A move waits for the frame.** A mouse reports five hundred or a
    /// thousand times a second and a pen two hundred, and a frame is drawn
    /// sixty times: several moves arrive between two frames, and one move of
    /// the pointer has been seen to arrive as three events at the same place.
    /// Each used to be a whole step of a stroke: its stamps, then a remesh,
    /// then a second ray march for the ring, then the ring. Only the stamps
    /// are owed to each move. So moves are kept as they come in, and handled
    /// together by [`Hani::handle_moves`] before the next frame or before
    /// anything else is acted on, whichever is first: every move's stamps in
    /// order, and one remesh, one pick and one ring for the lot.
    pub(super) fn pointer_in(&mut self, event: PointerEvent) {
        self.phases.count(|frame| frame.events += 1);
        if matches!(event, PointerEvent::Moved { .. }) {
            self.moves.push(event);
            // Frames stop coming to a window nothing is shown of, and moves
            // may not. They are not kept without limit.
            if self.moves.len() >= MOST_MOVES_KEPT {
                self.handle_moves();
            }
            return;
        }
        self.handle_moves();
        self.note_a_stroke();
        self.on_pointer(event);
        self.note_a_stroke();
    }

    /// Handle the moves kept since the last frame. See [`Hani::pointer_in`].
    pub(super) fn handle_moves(&mut self) {
        if self.moves.is_empty() {
            return;
        }
        let mut moves = std::mem::take(&mut self.moves);
        self.note_a_stroke();
        let last = moves.len() - 1;
        for (index, event) in moves.drain(..).enumerate() {
            self.batching = index < last;
            // The pen as it was at this move. See `app/pen.rs`.
            self.pen_at_move(index);
            self.on_pointer(event);
        }
        self.pen_after_moves();
        self.batching = false;
        self.note_a_stroke();
        // The allocation goes back for the next frame's moves.
        self.moves = moves;
    }

    /// Say that a stroke is down in the frame being timed.
    pub(super) fn note_a_stroke(&self) {
        if matches!(
            self.drag.map(|drag| drag.kind),
            Some(DragKind::Sculpt(_)) | Some(DragKind::Masking)
        ) {
            self.phases.count(|frame| frame.stroke = true);
        }
    }

    /// What one puck button press does.
    ///
    /// Split out of [`Hani::drive_from_spacemouse`] so a test can press a
    /// button without a `/dev/input` node: the press queue is filled by a
    /// reader thread, and the two camera arms below are exactly the kind of
    /// code that goes wrong unobserved because nothing in CI can reach it.
    pub(super) fn run_puck_button(&mut self, action: ButtonAction) {
        match action {
            ButtonAction::None => {}
            ButtonAction::Undo => self.undo(),
            ButtonAction::Redo => self.redo(),
            // Recentre and refit without losing which way the model is being
            // looked at, which is what makes this useful mid sculpt.
            //
            // **Through `frame_active`, not a fresh `framing`.** Assigning one
            // resets `voxel_size` to the 0.25 mm default and `content_radius`
            // to the hardcoded 30 mm starting ball, and both are now
            // load-bearing: the first is the zoom floor and the second is both
            // the far plane and the length of every pick ray. Measured on a
            // document resampled to 0.05, the floor went from 0.1 to 0.5 -- a
            // puck button silently making the camera five times coarser than
            // the lattice deserves, which is the precise complaint this work
            // exists to fix. On a large model the same assignment collapses
            // `far()` to well inside the geometry, so the far half of it stops
            // being drawn and stops being pickable at once. `frame_active`
            // frames the body that is actually there and refreshes the lattice
            // from the document, which is the whole job.
            ButtonAction::FrameModel => self.frame_active(),
            // Everything back to the start, roll included. This is the way out
            // of a view that has been twisted somewhere confusing.
            ButtonAction::ResetView => {
                let start = OrbitCamera::default();
                self.camera.yaw = start.yaw;
                self.camera.pitch = start.pitch;
                self.camera.roll = start.roll;
                // The angles are the reset; the framing is `frame_active`'s,
                // for the reason above.
                self.frame_active();
                self.status = "reset the view".to_string();
            }
            ButtonAction::ToggleSymmetry => {
                // Through the same gate the strip uses, and not a bare
                // `toggled`: see `toggle_mirror`.
                self.toggle_mirror(MirrorAxis::X);
            }
        }
    }

    /// Apply one frame of puck motion, and whatever its buttons asked for.
    pub(super) fn drive_from_spacemouse(&mut self, elapsed_ms: f32) {
        for action in self.spacemouse.take_presses() {
            self.run_puck_button(action);
        }

        let motion = self.spacemouse.motion();
        let moved = self.spacemouse.config.apply(
            &motion,
            elapsed_ms,
            &mut self.camera,
            self.viewport_size.y,
        );
        if moved {
            self.publish_camera();
        }
    }

    pub(super) fn configure_spacemouse(&mut self, setting: SpaceMouseSetting) {
        let config = &mut self.spacemouse.config;
        // Whether this change should reach the disk now. A slider mid drag
        // should not: it sends a message per step, and `Save` follows on
        // release.
        let mut persist = true;
        match setting {
            SpaceMouseSetting::Mode(mode) => config.mode = mode,
            SpaceMouseSetting::Deadzone(value) => {
                config.deadzone = value;
                persist = false;
            }
            SpaceMouseSetting::PanSens(value) => {
                config.pan_sens = value;
                persist = false;
            }
            SpaceMouseSetting::ZoomSens(value) => {
                config.zoom_sens = value;
                persist = false;
            }
            SpaceMouseSetting::StaleMs(value) => {
                config.stale_ms = value.clamp(20.0, 1000.0) as u64;
                persist = false;
            }
            SpaceMouseSetting::OrbitSens(value) => {
                config.orbit_sens = value;
                persist = false;
            }
            SpaceMouseSetting::Binding(action, source) => {
                let invert = config.binding(action).invert;
                config.set_binding(action, AxisBinding { source, invert });
            }
            SpaceMouseSetting::Invert(action, invert) => {
                let source = config.binding(action).source;
                config.set_binding(action, AxisBinding { source, invert });
            }
            SpaceMouseSetting::Button(index, action) => {
                if let Some(slot) = config.buttons.get_mut(index) {
                    *slot = action;
                }
            }
            SpaceMouseSetting::InvertAll => config.invert_all(),
            SpaceMouseSetting::Reset => *config = SpaceMouseConfig::default(),
            SpaceMouseSetting::Save => {}
        }
        if persist {
            self.spacemouse.config.save();
        }
    }

    /// Move the number a sizing gesture is holding, from how far the pointer
    /// has travelled since it began.
    ///
    /// Horizontal only, and absolute against where the drag started, so out and
    /// back returns to exactly the original value.
    fn apply_sizing(&mut self, to: Vec2) {
        let Some(sizing) = self.sizing else {
            return;
        };
        let travel = to.x - sizing.from_pixel.x;
        match sizing.what {
            // Multiplicative, matching the [ and ] keys: the radius spans fifty
            // to one, so a fixed amount per pixel would crawl at one end and
            // jump at the other.
            SizingTarget::Radius => {
                let factor = (travel * RADIUS_PER_PIXEL).exp();
                self.brush.radius =
                    (sizing.original * factor).clamp(MIN_RADIUS_MM, self.max_radius());
            }
            // Additive: strength is already a small linear range.
            SizingTarget::Strength => {
                self.brush.strength = (sizing.original + travel * STRENGTH_PER_PIXEL)
                    .clamp(MIN_STRENGTH, self.max_strength());
            }
        }
    }

    /// Whether a modal card is up, and therefore owns the input.
    ///
    /// One list, read by both halves of the guard: `on_key` for the keyboard
    /// and `on_pointer` for the pointer. Before this existed the pointer had
    /// its own inline pair of checks and the keyboard had none at all, so
    /// `ctrl+Z` under the unsaved-work prompt changed the very volume the
    /// prompt was asking about, `x` flipped a mirror plane behind the card,
    /// and `1`-`6` swapped the brush. The bug-report dialog was in neither
    /// list, so a press beside its card sculpted.
    ///
    /// # Adding to this list is not automatically right
    ///
    /// It answers "is the document unreachable right now", and every card here
    /// is a question the user must answer before anything else happens.
    ///
    /// **This comment used to say that the split preview belonged in the
    /// keyboard guard and NOT the pointer one, and that whoever added it should
    /// split this function in two.** That was written against a split preview
    /// whose gesture was dragging a plane across the model. The split that
    /// landed is split by loose parts, and its card is a question with two
    /// buttons and no gesture at all -- so it belongs in both guards, as the
    /// plan for increment 16 asks, and the function stays one list.
    ///
    /// The warning still stands for whatever modeless overlay does eventually
    /// take the pointer: **split this function in two at that point rather than
    /// widening it and quietly killing the drag.**
    pub(super) fn modal_open(&self) -> bool {
        self.welcome
            || self.preferences
            || self.shortcuts
            // See `app/help.rs`.
            || self.help.open
            // See `app/about.rs`.
            || self.about_open
            // See `app/shelf.rs`.
            || self.shelf.open
            // See `app/snip.rs`.
            || self.snipping()
            || self.confirm.is_some()
            || self.orient_prompt.is_some()
            || self.bug_report.is_some()
            || self.pending_delete.is_some()
            || self.pending_merge.is_some()
            || self.pending_split.is_some()
            // See `app/designer.rs`.
            || self.painting.designer.is_some()
    }

    /// What a key press means, now that nothing in the widget tree wanted it.
    ///
    /// The decode itself is `viewport::shortcut`, which is a pure function of
    /// the key and its modifiers so that it can be tested without a window —
    /// this is the half that needs the application, and it exists so that
    /// there is exactly one place where a key turns into a change.
    pub(super) fn on_key(
        &mut self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Task<Message> {
        match key {
            // Escape is the one key a modal must still see, so it sits above
            // the guard: it is how the unsaved-work prompt is cancelled and
            // the orientation prompt declined. It does NOT dismiss the bug
            // report, and that is on purpose -- the card holds a description
            // the user typed, and throwing it away on a stray Escape is the
            // same class of loss the confirm prompt exists to prevent.
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                // A snip being chosen, over everything. See `app/snip.rs`.
                if self.snip_escaped() {
                    return Task::none();
                }
                // The welcome screen first, and unlike the bug report it DOES
                // go on Escape: it holds nothing the user typed, so there is
                // nothing to lose by dismissing it, and a modal with no
                // keyboard exit is the one thing worse than no modal.
                if self.welcome {
                    self.welcome = false;
                    return Task::none();
                }
                // The same for the two cards of the menus, which hold
                // nothing typed either.
                if self.shortcuts || self.preferences {
                    self.shortcuts = false;
                    self.preferences = false;
                    return Task::none();
                }
                // Help, as those. See `app/help.rs`.
                if self.help_escaped() {
                    return Task::none();
                }
                // About, as those. See `app/about.rs`.
                if self.about_escaped() {
                    return Task::none();
                }
                // And the browser's card. See `app/shelf.rs`.
                if self.shelf_escaped() {
                    return Task::none();
                }
                self.update(Message::MenuClosed)
            }
            iced::keyboard::Key::Character(character) => {
                // THE keyboard modal guard. Everything a shortcut can reach --
                // undo, the mirror planes, the brush numbers -- edits the
                // document or the tool behind a card that is asking about it.
                if self.modal_open() {
                    return Task::none();
                }
                let Some(message) = crate::viewport::shortcut(
                    character.as_str(),
                    modifiers.command(),
                    modifiers.shift(),
                    modifiers.alt(),
                ) else {
                    return Task::none();
                };
                // A key that means nothing in the open mode does nothing: W
                // is Transform's, C is Sculpt's and Print's. See
                // `app/workspace.rs`.
                if !self.desk.now.allows(&message) {
                    return Task::none();
                }
                // One level of recursion and never more: `shortcut` returns
                // ordinary messages and cannot return another `KeyPressed`.
                self.update(message)
            }
            // Tab: back to the workspace before this one, the one key that
            // makes Sculpt and Paint and Sculpt again a flick. Under a card
            // it does nothing, as every shortcut does.
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab)
                if !self.modal_open() && !modifiers.command() =>
            {
                self.update(Message::WorkspaceBack)
            }
            iced::keyboard::Key::Named(_) | iced::keyboard::Key::Unidentified => Task::none(),
        }
    }

    pub(super) fn on_pointer(&mut self, event: PointerEvent) {
        // Fingers become the mouse's events, or the view's: `app/touch.rs`.
        if let PointerEvent::Touch { finger, phase, position, size } = event {
            self.touched(finger, phase, Vec2::new(position.x, position.y), size);
            return;
        }
        // While a modal prompt is up, the pointer belongs to it. The scrim in
        // `view` now genuinely swallows presses -- it is wrapped in
        // `iced::widget::opaque`, which captures them, where the bare styled
        // container it used to be forwarded every one -- and this is the other
        // half of that guarantee, because the viewport is a shader widget that
        // sees events wherever the cursor is. Both halves or neither: without
        // this a press behind the card would sculpt into a document the user
        // is about to discard, into a model they are about to turn, or into
        // the very state a bug report is describing.
        if self.modal_open() {
            // ...but a row picked up before the card went up has to be put
            // down, or the list stays armed and the NEXT press in the panel
            // arrives with a drag already in flight. Dropped rather than
            // performed: a move committed from behind a scrim is exactly the
            // edit this guard exists to refuse.
            if matches!(event, PointerEvent::Released { .. }) {
                self.row_drag = None;
            }
            return;
        }
        self.last_activity = Instant::now();
        match event {
            // Taken off at the top of this function.
            PointerEvent::Touch { .. } => {}
            PointerEvent::Modifiers { shift, control, alt } => {
                self.shift = shift;
                self.control = control;
                self.alt = alt;
                // Both change what a press would do, so the ring has to say so
                // before the press rather than after.
                self.refresh_overlay();
            }
            PointerEvent::Pressed { button, position, size } => {
                self.viewport_size = Vec2::new(size.x, size.y);
                let position = Vec2::new(position.x, position.y);
                self.cursor = Some(position);

                // **A second button down during a live transform abandons it.**
                //
                // There was no `self.drag.is_some()` guard here, so a right or
                // middle press assigned an Orbit or Pan drag straight over a
                // live `Transforming` one while `gizmo.grabbed` and the partial
                // `placement` survived. The right release then took the Orbit
                // arm, so the Transforming arm never ran and the left release
                // found no drag at all: the handle stayed lit, the preview box
                // went on drawing, and the gizmo visibly floated away from its
                // body.
                //
                // The abandoned gesture was not merely cosmetic. It is not
                // committed by the next `disarm_gizmo` -- that pushes the
                // pre-arm base against whatever the document already holds --
                // but by the NEXT gizmo drag's release, because that press
                // latches `pinned = placement` and adopts the stray placement
                // as its starting point.
                //
                // Cancelled exactly as Escape cancels, rather than ignoring the
                // press: ignoring it leaves the user in a modal drag with no
                // exit that anything on screen tells them about.
                if let Some(live) = &mut self.gizmo
                    && live.grabbed.is_some()
                    && self.drag.is_some_and(|drag| drag.button != button)
                {
                    live.placement = live.pinned;
                    live.grabbed = None;
                    self.drag = None;
                    self.refresh_gizmo();
                    self.status = "cancelled the drag".to_string();
                    return;
                }

                // An open menu swallows the next press: closing it is what the
                // click was for, and sculpting as well would be a surprise.
                if self.menu.take().is_some()
                    || self.top_menu.take().is_some()
                    || self.cube_menu.take().is_some()
                    || self.close_palette()
                {
                    return;
                }

                // A click while a measurement is being taken is one of its
                // two points, and not a stroke. See `app/measure.rs`.
                if self.measure_press(button, position) {
                    return;
                }
                // A press while the bodies are apart picks one. See
                // `app/explode.rs`.
                if self.explode_pressed(button, position) {
                    return;
                }
                // A press with the revolve's axis up draws its line. See
                // `app/revolve.rs`.
                if self.revolve_pressed(button, position) {
                    return;
                }

                // A press on the navigation cube belongs to the cube. Checked
                // before anything else, or clicking it would also carve a divot
                // out of the model behind it.
                //
                // Both buttons are taken, not just the left one. A right press
                // that fell through would start an orbit and then, on release,
                // open the brush menu on top of the cube -- and the cube's own
                // menu is what a right click there is for.
                if let Some(part) = navcube::pick(&self.camera, self.viewport_size, position) {
                    match button {
                        PointerButton::Left => self.fly_to(part),
                        PointerButton::Right => {
                            // Faces only. An edge or a corner points along no
                            // axis, and a quarter turn is the only kind of
                            // re-orientation offered, so there is nothing the
                            // menu could truthfully offer for one.
                            if part.extremes == 1
                                && let Some(facing) = hani_core::Facing::nearest(part.direction)
                            {
                                self.cube_menu = Some(CubeMenu { at: position, facing });
                            }
                        }
                        PointerButton::Middle => {}
                    }
                    return;
                }

                // **Before every other left-press rule, and bounds-checked
                // before it may claim anything.** ONLY BOUNDS-CHECKED EVENTS
                // MAY CAPTURE: `gizmo::pick` returns `None` for anything
                // outside the gizmo's own disc, so a press on the model still
                // reaches the sculpt below. It sits above the tool match rather
                // than inside `Tool::Transform`'s arm so that the gizmo is
                // reachable while the pointer is over another body -- selecting
                // a different body out from under an armed gizmo is exactly the
                // surprise the press ordering exists to remove.
                //
                // A held resize still outranks it, which is the precedence the
                // press dispatch below already settled between sizing and the
                // cut: a resize whose press turned into something else is the
                // worse of the two surprises, and the pointer belongs to that
                // gesture whether or not a button is down.
                if button == PointerButton::Left
                    && self.sizing.is_none()
                    && let Some(gizmo) = self.gizmo
                    && let Some(handle) =
                        gizmo::pick(&self.camera, self.viewport_size, &gizmo, position)
                {
                    if let Some(live) = &mut self.gizmo {
                        // Latched at the press: `drag` is absolute from this
                        // pixel and measures about the placement as it stands
                        // now, so both have to be held for the whole gesture.
                        live.pinned = live.placement;
                        live.grabbed = Some(handle);
                    }
                    self.drag = Some(Drag {
                        button,
                        kind: DragKind::Transforming(handle),
                        origin: position,
                        pressed_at: Instant::now(),
                        moved: false,
                        pivot: None,
                    });
                    self.refresh_gizmo();
                    return;
                }

                let kind = match button {
                    // Left sculpts -- unless a hold-and-drag resize is in
                    // progress, in which case the pointer belongs to that
                    // gesture and a press must not lay down a stroke.
                    //
                    // **This now outranks an armed cut, where it used to lose
                    // to one.** The cut and the mask collapsed into one match
                    // when `cut_armed` became a `Tool`, and the merged arm takes
                    // MASK's precedence rather than the cut's: a destructive
                    // one-shot may reasonably outrank a resize, an ongoing paint
                    // may not, and a resize whose press turned into a stroke is
                    // the worse of the two surprises.
                    PointerButton::Left if self.sizing.is_some() => DragKind::Sizing,
                    PointerButton::Left => match self.tool {
                        // The mode was chosen deliberately and the next left
                        // drag is the line, not a stroke.
                        Tool::Cut => {
                            // The press is the first point of the path, and the
                            // clear is what stops a cancelled gesture leaking
                            // into the next one.
                            self.cut_path.clear();
                            self.cut_path.push(position);
                            // Or the middle of a ring. See `app/ring_cut.rs`.
                            if self.rings() {
                                self.ring_pressed(position);
                            }
                            DragKind::Cutting
                        }
                        // Deliberately does NOT pick: masking paints the ACTIVE
                        // body, and a press that silently moved the target
                        // mid-mask would paint protection onto a body the user
                        // was not looking at. `refresh_hover` puts the ring on
                        // the active body while this tool is live, for the same
                        // reason and so that the two agree.
                        Tool::Mask => DragKind::Masking,
                        Tool::Sculpt => self.sculpt_press(position),
                        // The press missed every handle -- the gizmo claims its
                        // own above. Choosing a body is the useful thing left,
                        // and it re-arms on the new one, which is what makes
                        // "move this one now" one press rather than three.
                        Tool::Transform => {
                            let chosen = self.pick(position).map(|(body, _)| body);
                            match chosen {
                                Some(body) if body != self.doc.active() => {
                                    self.disarm_gizmo(true);
                                    // See `app/group_gizmo.rs`.
                                    self.body_picked();
                                    self.select_body(body);
                                    if let Err(why) = self.arm_gizmo() {
                                        self.status = why;
                                        self.tool = Tool::Sculpt;
                                    }
                                    DragKind::Selecting
                                }
                                _ => DragKind::Refused,
                            }
                        }
                    },
                    // Right and middle move the camera. Shift slides instead of
                    // turning.
                    PointerButton::Right | PointerButton::Middle => {
                        if self.shift {
                            DragKind::Pan
                        } else {
                            DragKind::Orbit
                        }
                    }
                };
                self.drag = Some(Drag {
                    button,
                    kind,
                    origin: position,
                    pressed_at: Instant::now(),
                    moved: false,
                    // `refresh_hover` ran on the motion event that brought the
                    // pointer here, so this costs a field read and no march.
                    //
                    // **The hover, with no fallback.** Orbiting about a point
                    // MOVES the target, so an anchor off the model walks the
                    // target sideways on every drag that started over empty
                    // background -- measured at 68 mm from one twenty step drag
                    // on a 30 mm ball, and it compounds. `None` here means
                    // "turn about the target", which is what orbiting has
                    // always done. [`Hani::navigation_anchor`] refuses an
                    // off-model anchor for the wheel for the same reason,
                    // arrived at separately and measured separately. Pinned by
                    // `an_orbit_drag_that_starts_off_the_model_turns_about_the_target`.
                    pivot: (kind == DragKind::Orbit).then_some(self.hover).flatten(),
                });

                let timed = self.phases.enter(phases::Phase::Edit);
                if let DragKind::Sculpt(direction) = kind {
                    self.sculpt_to(position, direction, true);
                }
                if kind == DragKind::Masking {
                    self.mask_to(position, true);
                }
                self.phases.leave(timed);
                self.refresh_hover(position);
                self.refresh_overlay();
            }
            PointerEvent::Released { button } => {
                // The revolve's line let go: it becomes a body. See
                // `app/revolve.rs`.
                if self.revolve_released(button) {
                    self.refresh_overlay();
                    return;
                }
                // **The one commit path for a row drag**, and it is the global
                // release rather than the row's own `on_release`: a row only
                // sees a release while the pointer is still over it, so a drag
                // that ended anywhere else would leave the list armed. See
                // `RowDrag`.
                if button == PointerButton::Left {
                    self.finish_row_drag();
                }
                if let Some(drag) = self.drag.filter(|drag| drag.button == button) {
                    if matches!(drag.kind, DragKind::Cutting) {
                        if self.rings() {
                            // The ring let go. See `app/ring_cut.rs`.
                            self.cut_path.clear();
                            self.ring_released();
                        } else {
                            // The drag already recorded where the button went down.
                            self.finish_cut(drag.origin);
                        }
                    }
                    // **The one place the field is rebuilt**, and it is the
                    // release rather than the motion: a bake per frame would be
                    // hundreds of trilinear passes for one gesture, and the
                    // preview box is what stands in for the model in the
                    // meantime.
                    if matches!(drag.kind, DragKind::Transforming(_)) {
                        if let Some(live) = &mut self.gizmo {
                            live.grabbed = None;
                        }
                        self.rebake_gizmo();
                        self.refresh_gizmo();
                    }
                    // A mask gesture ends the same way a sculpt one does, and
                    // through the same function: both opened the same recorder,
                    // and one gesture is one entry whichever of the two maps it
                    // wrote.
                    if matches!(drag.kind, DragKind::Sculpt(_) | DragKind::Masking) {
                        // What a drag that adds does when the button comes
                        // up: a cable's far end, or a body of its own.
                        self.add_released();
                        self.finish_stroke();
                    }
                    // A right press that neither moved nor lingered was a click,
                    // and a click opens the tool's own controls where the hand
                    // already is. Anything else was an orbit or a pan.
                    if button == PointerButton::Right
                        && !drag.moved
                        && drag.pressed_at.elapsed().as_millis() < CLICK_MS
                    {
                        self.menu = Some(drag.origin);
                    }
                    self.drag = None;
                }
                self.refresh_overlay();
            }
            PointerEvent::Moved { position, size } => {
                self.viewport_size = Vec2::new(size.x, size.y);
                let position = Vec2::new(position.x, position.y);
                let delta = self.cursor.map(|previous| position - previous).unwrap_or(Vec2::ZERO);
                self.cursor = Some(position);
                // The loose end of a measurement follows the pointer.
                self.measure_hover(position);
                // The revolve's line follows the pointer. See `app/revolve.rs`.
                self.revolve_moved(position);

                // Past the slop this is a drag, not a click, and it stays one
                // even if it comes back: otherwise an orbit that ends where it
                // began would open the menu.
                if let Some(drag) = &mut self.drag
                    && position.distance(drag.origin) > CLICK_SLOP_PX
                {
                    drag.moved = true;
                }

                // A held sizing key owns the pointer whether or not a button
                // is down, so this comes before the drag cases.
                if self.sizing.is_some() {
                    self.apply_sizing(position);
                    self.refresh_overlay();
                    return;
                }

                // The gizmo's hover, whether or not a button is down: it is
                // what says which handle a press would take, and it is a
                // handful of projections rather than a raycast.
                if let Some(gizmo) = self.gizmo {
                    let hovered = (self.drag.is_none())
                        .then(|| gizmo::pick(&self.camera, self.viewport_size, &gizmo, position))
                        .flatten();
                    if let Some(live) = &mut self.gizmo
                        && live.hovered != hovered
                    {
                        live.hovered = hovered;
                        self.refresh_gizmo();
                    }
                }

                match self.drag.map(|drag| drag.kind) {
                    Some(DragKind::Transforming(handle)) => {
                        self.transform_to(handle, position);
                        // Nothing else on this event: a gizmo drag owns the
                        // pointer, and the brush ring under it would be a ring
                        // on a surface that is about to move.
                        return;
                    }
                    Some(DragKind::Sculpt(direction)) => {
                        let timed = self.phases.enter(phases::Phase::Edit);
                        self.sculpt_to(position, direction, false);
                        self.phases.leave(timed);
                    }
                    Some(DragKind::Masking) => {
                        let timed = self.phases.enter(phases::Phase::Edit);
                        self.mask_to(position, false);
                        self.phases.leave(timed);
                    }
                    Some(DragKind::Orbit) => {
                        // About the surface the press landed on, so turning to
                        // look at the other side of a detail keeps the detail
                        // on screen instead of swinging it out of frame.
                        //
                        // **The fallback is the TARGET**, which is what
                        // orbiting has always turned about. An anchor off the
                        // model would put the pivot wherever the cursor
                        // happened to be over empty background, and since
                        // orbiting about it moves the target, every drag that
                        // started off the model would walk the target sideways
                        // for no benefit at all.
                        match self.drag.and_then(|drag| drag.pivot) {
                            Some(pivot) => self.camera.orbit_about(delta, pivot),
                            None => self.camera.orbit(delta),
                        }
                        self.publish_camera();
                    }
                    Some(DragKind::Pan) => {
                        self.camera.pan(delta, self.viewport_size.y);
                        self.publish_camera();
                    }
                    // The cut accumulates its path here and redraws the
                    // preview; the model is not touched until the button comes
                    // up, so the shape can be adjusted freely and what is drawn
                    // is what will go. A press that chose a body is over:
                    // dragging must not turn it into a stroke on a body the
                    // user has only just arrived at.
                    // A ring's radius. See `app/ring_cut.rs`.
                    Some(DragKind::Cutting) if self.rings() => self.ring_moved(position),
                    Some(DragKind::Cutting) => {
                        // Spaced rather than every event, so an identical
                        // looking stroke reduces to the same shape whether it
                        // was drawn quickly or slowly.
                        if self
                            .cut_path
                            .last()
                            .is_none_or(|last| last.distance(position) >= cut::CUT_PATH_SPACING_PX)
                        {
                            self.cut_path.push(position);
                            self.refresh_overlay();
                        }
                    }
                    Some(DragKind::Sizing)
                    | Some(DragKind::Selecting)
                    | Some(DragKind::Refused)
                    | None => {}
                }

                // Over the cube: light the part under the pointer, and draw no
                // brush ring, because a press there will not sculpt.
                let over_cube = navcube::pick(&self.camera, self.viewport_size, position);
                if over_cube != self.cube_hover {
                    self.cube_hover = over_cube;
                    self.refresh_cube();
                }
                if over_cube.is_some() {
                    self.hover = None;
                    self.hover_body = None;
                    self.refresh_overlay();
                    return;
                }

                // The ring follows the pointer, and the surface under it moves
                // as a stroke cuts into it.
                self.refresh_hover(position);
                self.refresh_overlay();
            }
            PointerEvent::Scrolled { amount, position, size } => {
                self.viewport_size = Vec2::new(size.x, size.y);
                let position = Vec2::new(position.x, position.y);
                // The anchor rule lives in `navigation_anchor`, and it is the
                // whole of the fix for "I cannot zoom in to work on a detail"
                // and for the drift that fixing it originally introduced.
                let factor = OrbitCamera::zoom_factor(amount);
                let anchor = self.navigation_anchor(factor);
                self.camera.zoom_by_about(factor, anchor);
                self.publish_camera();
                // The ring is pushed onto the surface it sits over and the
                // pick that placed it was taken at the old camera, so a zoom
                // leaves it stale. Cheap, and only on a real wheel notch.
                self.refresh_hover(position);
                self.refresh_overlay();
            }
        }
    }
}
