// SPDX-License-Identifier: AGPL-3.0-only

//! The cut: the shape a drag draws, the cutter built from it, and what the
//! cut did. Moved out of `app.rs` (issue 189).

use super::*;

/// How far the pointer may travel during a right press and still count as a
/// click rather than an orbit.
///
/// Right drag orbits, so this is the whole difficulty of putting a menu on right
/// click: too tight and a click that wobbles a pixel opens nothing, too loose
/// and a small deliberate orbit opens a menu over the model. Four pixels and a
/// quarter second are a starting point to tune by feel, which is why both are
/// named.
/// The narrowest a cut region may be, in voxels, before it is refused.
///
/// Two, because one is not enough: a region exactly one voxel across lands on a
/// row of voxel centres or falls between two rows depending on where the
/// lattice sits under it, so it removes material in some places and not others
/// for reasons the user cannot see. Two guarantees at least one centre across
/// the narrow direction wherever the region happens to fall.
const MIN_CUT_WIDTH_VOXELS: f32 = 2.0;

/// Bricks a cut may walk looking for loose pieces before it gives up.
///
/// **Sized from a time budget rather than from a memory one.**
/// `Document::split_plan` measures 13 to 17 microseconds per brick -- 423
/// bricks in 7.2 ms, 7,665 in 103 ms, close to linear -- so a brick ceiling IS
/// a time ceiling once the slope is known. At 14 us a brick, 1,200 bricks is
/// about one 16 ms frame, which is the right order for something appended to a
/// status line.
///
/// The number this replaced was 24,000, chosen to sound like "a large model".
/// At the measured slope that is roughly 320 milliseconds -- twenty frames --
/// spent deciding whether to add five words to a sentence, on top of a cut that
/// costs five.
const MOST_BRICKS_TO_WALK: usize = 1_200;

impl Hani {
    /// How the path drawn so far would be read, for the label and the preview.
    ///
    /// [`cut::CutShape::Line`] before anything is drawn, which is both the safe
    /// default and the true one: a cut that has not been dragged yet is the cut
    /// this application has always had.
    pub(super) fn live_cut_shape(&self) -> cut::CutShape {
        cut::read_stroke(&self.cut_path, CLICK_SLOP_PX)
            .map_or(cut::CutShape::Line, |gesture| gesture.shape)
    }

    /// The narrowest width of a screen-space hull, in voxels, at the depth the
    /// camera is focused on.
    ///
    /// The narrowest and not the average: a long thin lens is wide in one
    /// direction and vanishing in the other, and it is the vanishing one that
    /// decides whether the cut lands on any voxel centres at all.
    ///
    /// Found by rotating-callipers over the hull's edges -- for a convex
    /// polygon the minimum width is always across some edge, so the furthest
    /// vertex from each edge in turn is the whole search. At most sixteen edges
    /// by sixteen vertices, once per gesture.
    fn hull_width_in_voxels(&self, hull: &[Vec2]) -> f32 {
        let voxel_size = self.doc.voxel_size();
        if hull.len() < 3 || voxel_size <= 0.0 {
            return f32::INFINITY;
        }
        // Millimetres per pixel where the camera is looking, which is where the
        // preview is drawn and where the user believes the shape is.
        let target = self.camera.target;
        let scale = {
            let centre = self.viewport_size * 0.5;
            let (_, straight) = self.ray_through(centre);
            let (_, offset) = self.ray_through(centre + Vec2::X);
            let depth = (target - self.camera.eye()).length();
            (straight * depth).distance(offset * depth)
        };

        let mut narrowest = f32::INFINITY;
        for index in 0..hull.len() {
            let from = hull[index];
            let to = hull[(index + 1) % hull.len()];
            let Some(along) = (to - from).try_normalize() else {
                continue;
            };
            // Perpendicular distance from the edge to the furthest vertex.
            let across = hull
                .iter()
                .map(|point| ((*point - from).x * along.y - (*point - from).y * along.x).abs())
                .fold(0.0, f32::max);
            narrowest = narrowest.min(across);
        }
        narrowest * scale / voxel_size
    }

    /// A far cap for the cutter, so it stops behind what it is cutting off
    /// instead of carrying on through the model.
    ///
    /// **This is the request, in one function.** "Something that makes it
    /// easier to cut off things that are sticking out of the main body" is
    /// unanswerable with an unbounded cutter: circling a spur on the near side
    /// of a scan also takes whatever is behind it, and the only defence is a
    /// hand-painted mask on the far side -- which is the workaround every
    /// comparable tool ships and the one this is meant to remove.
    ///
    /// Returns `None` when there should be no cap, which is a real outcome
    /// rather than a failure. Three ways to get it, and they are not the same:
    ///
    /// * A straight drag. One plane is an infinite half-space by definition;
    ///   that is what the plane cut has always been and what
    ///   `a_cut_passes_through_the_whole_model` asserts of it.
    /// * Nothing was hit, so there is no depth to measure from.
    /// * There is nothing behind worth sparing -- and `report` is set, so the
    ///   status can say so rather than let the tool look clever.
    ///
    /// # Why the cap is clamped against the NEXT surface
    ///
    /// Placing it one band past the deepest exit is the obvious rule and it is
    /// wrong. At a 0.25 mm voxel a band is 0.75 mm, so a wall half a millimetre
    /// behind a spur is thinned from the front by a quarter of a millimetre
    /// with a sharp step -- and `is_printable` returns true for the result,
    /// because it counts holes and winding and has no thickness term at all.
    /// So the cap is pulled back to stay a full band clear of whatever is
    /// behind, and where there is not room for both it declines and says so. A
    /// tool that quietly thins the wall behind is worse than one that admits it
    /// cannot help.
    fn depth_cap(
        &self,
        hull: &[Vec2],
        report: &mut Option<&'static str>,
    ) -> Option<(hani_core::ClipPlane, f32)> {
        if hull.len() < 3 {
            return None;
        }
        let eye = self.camera.eye();
        let facing = (self.camera.target - eye).try_normalize()?;
        let reach = self.camera.far();

        // Points known to be inside a CONVEX hull: its centroid, and points on
        // the way from the centroid to each vertex. Deliberately not a bounding
        // box grid, which would put rays outside a hull that is not square, and
        // deliberately not the centroid alone -- one ray down the middle is a
        // sample of one, and on a long thin loop it can miss the spur entirely.
        let centroid = hull.iter().copied().sum::<Vec2>() / hull.len() as f32;
        let mut probes = vec![centroid];
        for vertex in hull {
            for fraction in [0.45_f32, 0.8] {
                probes.push(centroid.lerp(*vertex, fraction));
            }
        }

        let bodies = self.drawn_bodies();
        let mut deepest_exit = f32::NEG_INFINITY;
        let mut shallowest_enter = f32::INFINITY;
        let mut nearest_behind = f32::INFINITY;
        for pixel in probes {
            let (origin, ray) = self.ray_through(pixel);
            // Distances come back along each ray from its own origin on the
            // near plane, so they are converted to DEPTH along the view
            // direction before being compared between rays. Without this a ray
            // at the edge of a wide hull reports a longer distance for the same
            // depth and drags the cap backwards -- which is the same class of
            // mistake as measuring the cutter's apex from the near plane.
            let scale = ray.dot(facing);
            if scale <= 0.0 {
                continue;
            }
            let base = (origin - eye).dot(facing);
            for body in &bodies {
                let Some(volume) = self.doc.volume(*body) else {
                    continue;
                };
                let spans = hani_core::first_solid_spans(volume, origin, ray, reach);
                if let Some(span) = spans.first {
                    deepest_exit = deepest_exit.max(base + span.exit * scale);
                    shallowest_enter = shallowest_enter.min(base + span.enter * scale);
                }
                if let Some(next) = spans.next_enter {
                    nearest_behind = nearest_behind.min(base + next * scale);
                }
            }
        }

        if deepest_exit == f32::NEG_INFINITY {
            // Every ray missed. There is no depth here, and the existing "the
            // cut missed the model" outcome is the one that applies.
            return None;
        }

        let margin = hani_core::NARROW_BAND * self.doc.voxel_size();
        let want = deepest_exit + margin;
        if nearest_behind.is_finite() {
            if nearest_behind - deepest_exit < 2.0 * margin {
                *report = Some("nothing behind it to spare — the cut went through");
                return None;
            }
            let limit = nearest_behind - margin;
            let at = want.min(limit);
            return hani_core::ClipPlane::new(eye + facing * at, -facing)
                .map(|plane| (plane, at - shallowest_enter));
        }
        hani_core::ClipPlane::new(eye + facing * want, -facing)
            .map(|plane| (plane, want - shallowest_enter))
    }

    /// How many loose pieces the cut left behind, as a phrase to append to the
    /// status, or nothing.
    ///
    /// **A cut that severs a body leaves two shells inside one `Volume` with
    /// nothing anywhere saying so.** The user gets a model that looks right,
    /// exports as one object, and arrives at the slicer in two pieces -- which
    /// is precisely the failure the research identified in the tools this one
    /// is meant to beat.
    ///
    /// `Document::split_plan` is read-only, parallel and already tested, so
    /// this is a report and not a second connectivity walk. It is specifically
    /// NOT a brick-level walk: `split.rs` measures one finding a single
    /// component on the reference dragon where the voxel walk finds 29, 47, 85
    /// and 182 at four voxel sizes, and fusing parts of a real four-part model.
    ///
    /// # It is expensive, so it is bounded three ways
    ///
    /// `split_plan` is a per-voxel union-find and costs roughly 13 to 17
    /// microseconds per brick -- more, on the bench fixture, than the entire
    /// cut that preceded it. Left unbounded this one status-line suffix is the
    /// most expensive thing on the commit path by a wide margin. So:
    ///
    /// 1. **Only bodies the cut actually changed.** They come from the outcome
    ///    rather than from asking each body whether anything is dirty, which
    ///    would be reading an answer off a set that merely has not been drained
    ///    yet. Walking a body the cutter never reached costs full price and can
    ///    only ever find what was already there before the gesture.
    /// 2. **One budget across all of them**, not one each: eight small bodies
    ///    are eight walks, and a per-body gate lets them through.
    /// 3. **A budget sized in bricks from a time target.** See
    ///    [`MOST_BRICKS_TO_WALK`].
    ///
    /// Going over budget stops and reports what was found so far rather than
    /// reporting nothing: pieces already counted are true, and throwing them
    /// away because a later body was large would hide a real severance behind
    /// an unrelated body's size.
    fn loose_pieces_note(&self, changed: &[NodeId]) -> String {
        let mut pieces = 0usize;
        let mut walked = 0usize;
        for body in changed {
            let Some(volume) = self.doc.volume(*body) else {
                continue;
            };
            // `brick_count` and not `brick_coords().count()`: the second walks
            // the map to answer a question the map already knows.
            walked += volume.brick_count();
            if walked > MOST_BRICKS_TO_WALK {
                break;
            }
            let Some(plan) = self.doc.split_plan(*body) else {
                continue;
            };
            // One part is the body itself and is not news.
            pieces += plan.parts.len().saturating_sub(1);
        }
        match pieces {
            0 => String::new(),
            1 => " — 1 loose piece".to_string(),
            many => format!(" — {many} loose pieces"),
        }
    }

    /// The DRAWN bodies, by node id -- the same set the cut acts on.
    fn drawn_bodies(&self) -> Vec<NodeId> {
        let visible = self.drawn_nodes();
        self.doc
            .nodes()
            .iter()
            .enumerate()
            .filter(|(index, _)| visible.get(*index).copied().unwrap_or(false))
            .map(|(_, node)| node.id)
            .collect()
    }

    /// Build the cutter's planes from a screen-space hull.
    ///
    /// Each edge of the hull becomes a plane containing that edge and the view
    /// direction, so the region removed is the PRISM the polygon sweeps out
    /// going away from the viewer -- the same width front and back. `None` if
    /// any edge is degenerate: all or nothing, because a polygon missing one
    /// side is not a smaller region, it is an open wedge that carries on for
    /// ever.
    ///
    /// The straight drag is the exception and keeps its plane through the eye:
    /// a single half-space has no aperture, so there is nothing for a taper to
    /// widen, and that construction is what ships and what the side-convention
    /// test observes.
    ///
    /// # The eye is `camera.eye()`, and that is a bug fix
    ///
    /// [`OrbitCamera::ray`] returns the point where the ray crosses the NEAR
    /// PLANE, not the eye -- and it must keep doing so, because every pick in
    /// the application starts there and a test pins it. The cut used that point
    /// as its apex, and for a single plane that is correct **by accident**: the
    /// near point lies on the first ray, and the normal is perpendicular to
    /// both rays, so the offset falls out of the dot product exactly.
    ///
    /// With several planes it stops cancelling. Plane `k`'s normal is not
    /// perpendicular to the ray the near point came from, so plane `k` is
    /// displaced by `(P0 - E) . n_k` -- a different amount for every side --
    /// and the pyramid's apex opens into a small polyhedral gap. The bound is
    /// `near()`, which is a hundredth of the camera distance and about 1.5 mm
    /// on a framed 100 mm model, six voxels at 0.25 mm. In practice it is less,
    /// because for a small hull all the rays are nearly parallel and the dot
    /// product is a fraction of that; it grows with the model and with how wide
    /// the gesture is.
    ///
    /// # The sign is measured, not assumed
    ///
    /// For a polygon the winding decides which side each plane removes, and
    /// getting it backwards inverts the cutter: instead of removing the inside
    /// of the loop it removes everything except a pyramid. Rather than reason
    /// about the handedness of the camera basis, the y-flip in the pixel-to-NDC
    /// step and the hull's winding all at once -- three chances to be wrong
    /// with one symptom -- the normals are built to one convention and then
    /// CHECKED against a point known to be inside the region. If the check
    /// fails they are all flipped. A sign that is verified costs one dot
    /// product per plane, once per gesture, and cannot be got wrong by a later
    /// change to any of the three.
    pub(super) fn cutter_from_hull(&self, hull: &[Vec2]) -> Option<Vec<hani_core::ClipPlane>> {
        let eye = self.camera.eye();

        // Two points name one plane and no region: this is the straight drag,
        // and it keeps its own construction so that the side convention a test
        // observes is untouched.
        if let [from, to] = hull {
            let (_, first) = self.ray_through(*from);
            let (_, second) = self.ray_through(*to);
            return hani_core::ClipPlane::new(eye, second.cross(first)).map(|plane| vec![plane]);
        }
        if hull.len() < 3 {
            return None;
        }

        // **A prism, not a pyramid**, and the difference is the loudest thing
        // about this tool in use. Building each side plane through the EYE
        // makes the removed region the view frustum's own wedge: it widens with
        // distance in exactly the proportion the projection does. At the
        // framing this application picks -- `camera.rs` puts the eye at
        // `radius / sin(fov/2) * 1.15`, about 90 mm for a 30 mm ball -- the
        // front face is 60 mm away and the back 120, so **the exit hole comes
        // out twice the diameter of the entry hole**. Circling a lump takes a
        // cone out of everything behind it.
        //
        // Building them along the VIEW DIRECTION instead makes the sides
        // parallel, so the cutter is the same width all the way through: what a
        // bandsaw does, and what a symmetric pair of cuts needs in order to
        // come out symmetric.
        //
        // The trade is real and worth stating. A pyramid is screen-exact at
        // every depth -- the region removed is precisely the screen area swept,
        // whatever it passes through. A prism is exact only at the focus depth,
        // which is where the preview is drawn; further away the region takes
        // slightly less than the outline appeared to cover, and nearer slightly
        // more. That is the better error to have: it is bounded by the model's
        // own depth rather than growing with it, and it errs toward the
        // recoverable direction on the far side.
        //
        // This is the orthographic-cut behaviour without an orthographic
        // camera. The camera is untouched; only the cutter is parallel.
        let facing = (self.camera.target - eye).try_normalize()?;
        let corner = |pixel: Vec2| {
            let (origin, ray) = self.ray_through(pixel);
            let denominator = ray.dot(facing);
            if denominator.abs() < 1.0e-6 {
                return origin;
            }
            origin + ray * ((self.camera.target - origin).dot(facing) / denominator)
        };

        let mut planes = Vec::with_capacity(hull.len());
        for index in 0..hull.len() {
            let here = corner(hull[index]);
            let next = corner(hull[(index + 1) % hull.len()]);
            planes.push(hani_core::ClipPlane::new(here, (next - here).cross(facing))?);
        }

        // A point genuinely inside the prism: the centroid of a CONVEX polygon
        // is inside it, so the point at the focus depth under the centroid is
        // in the middle of the region.
        let centroid = hull.iter().copied().sum::<Vec2>() / hull.len() as f32;
        let inside = corner(centroid);
        if planes.iter().any(|plane| plane.distance(inside) < 0.0) {
            for plane in &mut planes {
                plane.normal = -plane.normal;
            }
        }
        debug_assert!(
            planes.iter().all(|plane| plane.distance(inside) >= 0.0),
            "the cutter excludes its own centre, so the hull is not convex"
        );
        Some(planes)
    }

    ///
    /// **It cuts every body the user can SEE**, which is [`Document::clip`]'s
    /// whole job: solo narrows the set, a hidden body the line passes over is
    /// left bit-identical, and the whole gesture is one undo entry.
    ///
    /// **And it answers to the mask**, on the same principle: a cut acts on what
    /// is drawn, and a masked region is drawn as protected. A cut a mask blocked
    /// entirely produces the same zero as a cut that missed, so the zero case
    /// below tells the two apart and names the body -- see there for why that
    /// matters more than the count does.
    ///
    /// # The shape comes from the stroke
    ///
    /// [`cut::read_stroke`] decides whether the drag was a line, an open curve
    /// or a closed loop, and hands back a convex polygon in screen pixels. Each
    /// of its edges becomes a plane through the EYE, so the cutter is the
    /// pyramid the polygon sweeps out going away from the viewer -- which is
    /// what makes a screen-space gesture mean something in three dimensions.
    /// A two-point polygon is the straight drag that has always shipped, and it
    /// still produces exactly one plane and one cross product.
    ///
    /// **Which side goes is set by the drag direction**: the material to the
    /// LEFT of the arrow, as drawn on screen, is removed -- so a left to right
    /// drag takes the top half. Drag the other way to keep the other half.
    ///
    /// That sentence is copied from what
    /// `a_left_to_right_drag_removes_a_consistent_side` actually observes, not
    /// from reasoning about the cross product. The sign depends on the ray
    /// order, the handedness of the camera basis and whether the pixel-to-NDC
    /// step flips Y, and getting it backwards means the tool removes the half
    /// the user meant to keep. For a polygon the sign is not reasoned about at
    /// all -- see [`Hani::cutter_from_hull`].
    pub(super) fn finish_cut(&mut self, from: Vec2) {
        // The path is taken whatever happens next, so no outcome below can
        // leave it to leak into the next gesture.
        let mut path = std::mem::take(&mut self.cut_path);
        if let Some(to) = self.cursor {
            // The release position, which no motion event may have delivered if
            // the pointer stopped moving before the button came up.
            if path.last().is_none_or(|last| *last != to) {
                path.push(to);
            }
        }
        // Older gestures reached here with only the press position recorded.
        if path.is_empty() {
            path.push(from);
        }

        let Some(gesture) = cut::read_stroke(&path, CLICK_SLOP_PX) else {
            // A click is not a cut. Without this, arming the tool and clicking
            // once would take an arbitrary half of the model away. Ctrl only
            // changes which field the region is written into, so the same slop
            // applies and the word changes with it.
            //
            // It also catches a stroke that went somewhere and enclosed
            // nothing -- out and straight back -- which is not a click and is
            // just as much not a region. Hulling that into a sliver thinner
            // than the lattice would remove material in some places and not
            // others depending on where the voxels fell, while reporting
            // success.
            let what = if self.control { "mask" } else { "cut" };
            self.status = format!("{what} cancelled: drag a shape across the model");
            self.refresh_overlay();
            return;
        };

        let Some(planes) = self.cutter_from_hull(&gesture.hull) else {
            // All or nothing: dropping one side plane of a polygon does not
            // make a smaller cutter, it opens the region into an unbounded
            // wedge that carries on across the whole model.
            self.status = "cut cancelled: that shape has no direction".to_string();
            self.refresh_overlay();
            return;
        };

        // **The width guard is in VOXELS and not in pixels, and that is the
        // whole point of it.** A long thin loop clears any plausible area
        // threshold in pixels while enclosing a lens a fraction of a voxel
        // across, and whether any voxel centre falls inside such a lens depends
        // on where the lattice happens to sit. The slot then appears in some
        // places and not others, or nowhere at all -- after bricks were
        // classified, promoted, written and counted, so the status line reports
        // a successful cut over a model that did not visibly change.
        //
        // Measured at the focus depth, which is where the preview is drawn and
        // therefore where the user believes the shape is.
        if gesture.hull.len() >= 3 {
            let across = self.hull_width_in_voxels(&gesture.hull);
            if across < MIN_CUT_WIDTH_VOXELS {
                self.status = "cut cancelled: that loop is thinner than the voxel grid".to_string();
                self.refresh_overlay();
                return;
            }
        }

        // **Alt keeps both sides, and keys them.** The same drag as a cut,
        // read at the release like ctrl and shift: instead of one side going,
        // the body becomes two that register on each other. Before the depth
        // cap and the mirror, because a parting is one whole plane through one
        // body and neither applies to it.
        if self.parting_cut() {
            self.part_along(&planes, gesture.shape);
            // One shot per arming, as the cut and the mask are.
            self.tool = Tool::Sculpt;
            self.refresh_overlay();
            return;
        }

        // **Bounded by default, and shift is what asks for the old behaviour.**
        // The default is the one that answers the request -- circle the spur
        // and the spur goes -- and the unbounded cut is a modifier away rather
        // than the other way round, because a cut that takes more than it was
        // asked for is the complaint every surveyed tool has and a cut that
        // takes less is one more drag.
        //
        // Read at the RELEASE, like ctrl, and previewed while it is held, so
        // the verb is visible rather than inferred from a key state nobody can
        // see. Silent mode flips at release are named in the research as the
        // top pitfall of this whole family of tools.
        let mut depth_note: Option<&'static str> = None;
        let mut planes = planes;
        let mut depth_mm = None;
        if !self.shift
            && let Some((cap, thickness)) = self.depth_cap(&gesture.hull, &mut depth_note)
        {
            depth_mm = Some(thickness);
            planes.push(cap);
        }

        // **Ctrl turns the cut into a selection**, which is increment 25c's
        // "no new gesture": the same drag, the same region, the same brick
        // classification, writing protection instead of removing material. It
        // is checked here at the RELEASE rather than at the press, because that
        // is where the region exists and because a user who realises mid-drag
        // that they meant to select can still say so.
        //
        // EVERY shape routes here, not just the straight drag. `planes` is
        // whatever `cutter_from_hull` built -- the whole convex region and its
        // depth cap -- and `mask_cutter` reads the shape only to choose a word
        // for the status line. (This paragraph said the opposite for as long as
        // the shaped cut has existed, which is how its status line came to
        // describe a lasso and name a button that only a half-space user could
        // ever have found.)
        // **The cut honours the mirror the way every brush does.** With X on,
        // a stroke lands on both sides, so a cut with X on takes both sides:
        // the cutter the user drew and its reflection through each enabled
        // plane, depth cap included, as ONE gesture and ONE undo entry. The
        // report that prompted this had a user cut one side of a model with X
        // on and find the other side untouched, after a session in which every
        // stroke had gone to both. The preview draws the twin for the same
        // reason: what is shown is what goes, on both sides of the plane.
        let twins = self.framed_symmetry().mirrored_cutters(&planes, MIRROR_CENTRE);
        let mut cutters: Vec<&[hani_core::ClipPlane]> = vec![&planes];
        cutters.extend(twins.iter().map(Vec::as_slice));
        let mirrored = if twins.is_empty() {
            String::new()
        } else {
            format!(", mirrored {}", self.symmetry_settings().label())
        };

        if self.control {
            self.mask_cutter(&cutters, gesture.shape);
            // One shot per arming, exactly as the cut is: the assignment below
            // carries that property and this arm must not skip it.
            self.tool = Tool::Sculpt;
            self.refresh_overlay();
            return;
        }

        // **The cut crosses every VISIBLE body**, which is what
        // `Document::clip_cutters` is for: solo narrows the set, a hidden body
        // the gesture passes over comes back bit-identical, and the whole
        // gesture is ONE undo entry of N `Change::Bricks`. Direct manipulation
        // acts on what is drawn.
        let visible = self.drawn_nodes();
        let outcome = self.doc.clip_cutters(&cutters, &visible);
        if outcome.bricks > 0 {
            let before = self.history.stats();
            if let Some(entry) = outcome.entry {
                self.history.push(entry);
            }
            self.unsaved = true;
            let where_ = match gesture.shape {
                cut::CutShape::Line => String::new(),
                cut::CutShape::Curve => " inside the curve".to_string(),
                cut::CutShape::Lasso => " inside the lasso".to_string(),
            };
            // How deep it went, which is the whole difference between this and
            // the cut that shipped before it. A shaped cut that says only how
            // many bricks it took leaves the user to discover from the other
            // side whether it stopped.
            let how_deep = match (depth_mm, depth_note) {
                (_, Some(note)) => format!(" — {note}"),
                (Some(deep), None) => format!(", {deep:.0} mm deep"),
                (None, None) if gesture.shape == cut::CutShape::Line => String::new(),
                (None, None) => " — all the way through".to_string(),
            };
            let body_count = if outcome.bodies_cut.len() > 1 {
                format!(" across {} bodies", outcome.bodies_cut.len())
            } else {
                String::new()
            };
            self.status = format!(
                "cut {} bricks{where_}{body_count}{mirrored}{how_deep}{}",
                outcome.bricks,
                self.loose_pieces_note(&outcome.bodies_cut)
            );
            // After the cut's own message on purpose: an eviction that took
            // a deleted body with it outranks a count of bricks.
            self.record_history(before);
            self.remesh_dirty();
            self.refresh_overlay();
        } else {
            // Nothing changed, so nothing is recorded -- an undo entry for a
            // no-op would be worse than none. The three ways to get here read
            // very differently to a user, so they are told apart: a mask that
            // blocked the whole cut, a line that really did pass over a body
            // and found only empty space inside its box, and a line that went
            // nowhere near anything.
            self.status = if outcome.bricks_spared_by_mask > 0 {
                // **The mask comes first, and it names the body.** Without this
                // the single most confusing thing masking can do -- a cut that
                // silently does nothing because of protection painted minutes
                // ago, possibly on a body that is not even the active one --
                // reports itself as "the cut missed the model", which sends the
                // user off to redraw a line that was never the problem.
                match outcome.bodies_spared_by_mask.as_slice() {
                    [only] => format!(
                        "the mask on {} blocked the cut",
                        self.doc
                            .meta(*only)
                            .map_or_else(|| "that body".to_string(), |meta| meta.name)
                    ),
                    many => format!("the mask blocked the cut on {} bodies", many.len()),
                }
            } else if outcome.bodies_crossed > 0 {
                format!(
                    "the cut crossed {} bodies and found nothing to remove",
                    outcome.bodies_crossed
                )
            } else {
                "the cut missed the model".to_string()
            };
        }
        // One cut per arming. A destructive tool that stays live is how a
        // stray click removes half the model. **This is the assignment that
        // carries the one-shot property, not the one in the Escape arm** --
        // moving it there instead would drop it silently while the Escape test
        // still passed.
        self.tool = Tool::Sculpt;
    }
}
