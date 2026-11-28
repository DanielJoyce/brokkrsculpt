// SPDX-License-Identifier: AGPL-3.0-only

//! The voxel size and the working size: resampling, scaling the model to a
//! length, and what the resolution means for a print. Moved out of `app.rs`
//! (issue 189).

use super::*;

/// Roughly what a consumer resin printer resolves in XY, in millimetres.
///
/// A reference point for the detail advice, not a limit on anything. Mono LCD
/// panels in the 8K class land near this; the exact figure varies by machine
/// and it is used only to say "you are at or below this", which does not need
/// to be exact.
const RESIN_XY_MM: f32 = 0.035;

/// A typical filament nozzle's extrusion width, in millimetres. The other end
/// of the same comparison: detail finer than this cannot be printed by FDM at
/// all, so a voxel well under it is being spent on nothing unless the model is
/// bound for resin.
const FDM_LINE_MM: f32 = 0.4;

impl Hani {
    /// The voxel size a request actually lands on.
    pub(super) fn clamped_voxel_size(requested: f32) -> f32 {
        requested.clamp(FINEST_VOXEL_MM, COARSEST_VOXEL_MM)
    }

    /// Rebuild the volume at a different voxel size.
    pub(super) fn resample(&mut self, voxel_size: f32) {
        let mut voxel_size = Self::clamped_voxel_size(voxel_size);
        if (voxel_size - self.doc.voxel_size()).abs() < 1.0e-6 {
            return;
        }

        // A step the pool cannot hold LANDS ON THE FINEST THAT FITS instead
        // of refusing. The refusal named that size and then made the user go
        // there by hand -- which they could not: the button only halves, so
        // the message pointed at a rung the interface had no way to reach.
        // Observed live at 0.062 mm with 0.043 mm reachable and no path to it.
        let mut capped = None;
        if let Some((why, finest)) = self.too_fine_for_the_pool(voxel_size) {
            let fallback = Self::clamped_voxel_size(finest);
            if fallback < self.doc.voxel_size() * 0.98 {
                capped = Some(voxel_size);
                voxel_size = fallback;
            } else {
                // Already at the fit limit: there is nowhere finer to land.
                self.status = why;
                log::warn!("{}", self.status);
                return;
            }
        }

        let started = Instant::now();
        // Every body at once, because they share the lattice: the old bricks
        // have to be cleared out of the renderer's pool, and after a resample
        // their coordinates mean something different, so they are marked in the
        // new volume to remesh to nothing.
        self.doc.resample(voxel_size);
        // Past the early return above, so resampling to the size already in use
        // stays the no-op its test asserts it is.
        self.unsaved = true;
        // History refers to bricks of a volume that no longer exists, and at a
        // different resolution, so keeping it would splice nonsense back in.
        self.history.clear();
        self.history_stats = self.history.stats();
        self.rebuild_everything();
        self.refresh_detail_advice();

        self.status = match capped {
            Some(wanted) => format!(
                "resampled to {voxel_size:.3} mm -- the finest the mesh pool holds at this size \
                 ({wanted:.3} mm did not fit), {} dense bricks, {:.0} MB, {:.0} ms",
                self.doc_stats.dense_bricks,
                self.doc_stats.resident_bytes as f64 / (1024.0 * 1024.0),
                started.elapsed().as_secs_f64() * 1000.0
            ),
            None => format!(
                "resampled to {voxel_size:.3} mm, {} dense bricks, {:.0} MB, {:.0} ms",
                self.doc_stats.dense_bricks,
                self.doc_stats.resident_bytes as f64 / (1024.0 * 1024.0),
                started.elapsed().as_secs_f64() * 1000.0
            ),
        };
        log::info!("{}", self.status);
    }

    /// Why a resample to `wanted` would not fit the mesh pool, if it would not.
    ///
    /// This is the guard the import path has had all along and this one never
    /// did, and its absence was not theoretical: on the Nightwing dragon the
    /// "finer" button resampled correctly in 296 ms and then reserved 8,525,824
    /// vertices against a pool of 8,000,000. The volume was right, the model on
    /// screen lost parts of itself, and the only thing that said so was a line
    /// on stderr. From the user's chair the button did nothing.
    ///
    /// The prediction is a square law over what is ACTUALLY on the GPU right
    /// now, not an estimate from surface area. A surface has a fixed area, so
    /// halving the voxel size quadruples the vertices on it -- and starting
    /// from a measured reservation means the allocator's own padding is already
    /// baked into the figure rather than guessed at.
    fn too_fine_for_the_pool(&self, wanted: f32) -> Option<(String, f32)> {
        if wanted >= self.doc.voxel_size() {
            return None;
        }

        // System memory first, because the GPU pool is no longer the tighter
        // of the two. Measured on the dragon: 0.0565 mm fits the pool at 48%
        // and costs 4.15 GB of RAM, so a pool-only guard would happily walk a
        // machine into swap or the OOM killer. Same square law -- a surface
        // has fixed area, so halving the voxel quadruples the shell.
        let growth = (self.doc.voxel_size() / wanted).powi(2) as f64;
        let bytes = self.doc_stats.resident_bytes as f64 * growth;
        if bytes > MAX_VOLUME_BYTES {
            let headroom = MAX_VOLUME_BYTES / self.doc_stats.resident_bytes.max(1) as f64;
            let finest = self.doc.voxel_size() / headroom.sqrt() as f32 * 1.03;
            return Some((
                format!(
                    "could not resample to {wanted:.3} mm: it needs about {:.1} GB of memory \
                     against a {:.0} GB ceiling -- {finest:.3} mm is the finest that fits",
                    bytes / (1024.0 * 1024.0 * 1024.0),
                    MAX_VOLUME_BYTES / (1024.0 * 1024.0 * 1024.0),
                ),
                finest,
            ));
        }

        let pool = self.shared.stats();
        if pool.vertices_reserved == 0 {
            return None;
        }
        // Predicted from what is LIVE, because a resample resets the pool and
        // so starts from a clean bump pointer -- the watermark before the
        // reset says nothing about the space after it. Scaling `live` is
        // therefore the honest basis, and it is only honest BECAUSE of the
        // reset; before that existed this prediction was blind to
        // fragmentation and let the pool overflow.
        let vertices = pool.vertices_reserved as f64 * growth;
        let indices = pool.indices_reserved as f64 * growth;
        if vertices <= pool.vertex_capacity as f64 && indices <= pool.index_capacity as f64 {
            return None;
        }

        // The finest size that would fit, from the same square law, with three
        // percent on top because a prediction that lands at exactly 100% of a
        // ceiling helps nobody. Returned so `resample` can GO there rather
        // than tell the user to.
        let headroom = (pool.vertex_capacity as f64 / pool.vertices_reserved as f64)
            .min(pool.index_capacity as f64 / pool.indices_reserved as f64);
        let finest = self.doc.voxel_size() / headroom.sqrt() as f32 * 1.03;
        let why = format!(
            "could not resample to {wanted:.3} mm: it needs about {:.1}M vertices against a pool \
             of {:.1}M -- {finest:.3} mm is the finest that fits at this size",
            vertices / 1.0e6,
            pool.vertex_capacity as f64 / 1.0e6,
        );
        Some((why, finest))
    }

    /// Scale the model so its longest dimension is `longest_mm`.
    ///
    /// Free and lossless: distances are stored in voxels, so this is a change
    /// to `voxel_size` and nothing else, and the bricks are bit-identical
    /// afterwards. It buys **no** detail -- the model has the same number of
    /// voxels across it as before. What it changes is what one voxel measures,
    /// which is what decides whether the detail already there is enough for a
    /// given printer. [`Hani::detail_advice`] is what tells the user that.
    pub(super) fn set_working_size(&mut self, longest_mm: f32) {
        if !(longest_mm.is_finite() && longest_mm > 0.0) {
            self.status = "could not resize: that is not a length".into();
            return;
        }
        let Some((lo, hi)) = self.doc.active_volume().surface_bounds() else {
            self.status = "could not resize: there is no model".into();
            return;
        };
        let current = (hi - lo).max_element();
        if current <= 0.0 {
            self.status = "could not resize: the model has no size".into();
            return;
        }

        let factor = longest_mm / current;
        // The voxel size travels with the model, so the range that bounds it
        // bounds this too -- otherwise a resize could land somewhere the finer
        // and coarser buttons can never get back from.
        let wanted_voxel = self.doc.voxel_size() * factor;
        let clamped = Self::clamped_voxel_size(wanted_voxel);
        if (clamped - wanted_voxel).abs() > 1.0e-9 {
            self.status = format!(
                "could not resize to {longest_mm:.1} mm: that would put the voxel at {:.4} mm, \
                 outside the {FINEST_VOXEL_MM:.2}-{COARSEST_VOXEL_MM:.2} mm range",
                wanted_voxel,
            );
            return;
        }

        let previous_radius = self.model_radius;
        // Every body, because they share the lattice: scaling one alone would
        // hand it a lattice its siblings do not have.
        self.doc.rescale(factor);
        // The brush is in millimetres, so without this a resize leaves it the
        // wrong size relative to the model it is about to be used on.
        self.brush.radius = (self.brush.radius * factor).clamp(MIN_RADIUS_MM, self.max_radius());
        self.unsaved = true;
        // Every brick's world position moved, so everything has to be redrawn
        // even though not one voxel changed.
        self.doc.mark_everything_dirty();
        self.rebuild_everything();
        let _ = previous_radius;
        self.camera = OrbitCamera::framing(Vec3::ZERO, self.model_radius.max(1.0e-3));
        // `framing` leaves the default lattice behind, and the floor is a
        // number of voxels, so it has to be told the real one.
        self.refresh_camera_lattice();
        self.publish_camera();
        self.refresh_overlay();
        self.refresh_detail_advice();
        self.status = format!("working size {longest_mm:.1} mm, {}", self.detail_advice);
        log::info!("{}", self.status);
    }

    /// Recompute the cached readout. See the field for why it is cached.
    pub(super) fn refresh_detail_advice(&mut self) {
        self.detail_advice = self.measure_detail_advice();
    }

    /// What the current resolution actually means for a print, in one line.
    ///
    /// Detail is decided by how many voxels lie across the model, which does
    /// not change when it is scaled. What a printer cares about is what one
    /// voxel MEASURES. Saying both, side by side, is the whole point: it turns
    /// "is 0.25 mm fine enough?" from a guess into arithmetic.
    ///
    /// The resin figure is the comparison worth drawing because it is the
    /// demanding one -- a consumer resin printer resolves around 0.03 mm in XY,
    /// where a filament nozzle lays down 0.4 mm lines and cannot use anything
    /// like this much.
    fn measure_detail_advice(&self) -> String {
        let Some((lo, hi)) = self.doc.active_volume().surface_bounds() else {
            return "no model".into();
        };
        let voxel_size = self.doc.voxel_size();
        let longest = (hi - lo).max_element();
        let across = (longest / voxel_size).round() as i64;
        let verdict = if voxel_size <= RESIN_XY_MM {
            "at or below what a resin printer resolves"
        } else if voxel_size <= FDM_LINE_MM {
            "finer than a filament nozzle, coarser than resin"
        } else {
            "coarse: a filament nozzle would not see the difference"
        };
        format!(
            "{longest:.1} mm across, voxel {voxel_size:.3} mm, {across} voxels wide -- {verdict}"
        )
    }
}

#[cfg(test)]
mod working_size_tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    fn app() -> Hani {
        Hani::with_tablet(crate::tablet::Tablet::inert())
    }

    /// Scaling is a change to `voxel_size` and nothing else, so the field has
    /// to come through it untouched. If this ever fails, the operation stopped
    /// being free and started resampling.
    #[test]
    fn resizing_scales_the_model_without_touching_a_voxel() {
        let mut app = app();
        let before: Vec<_> = app
            .doc
            .active_volume()
            .brick_coords()
            .map(|c| (c, app.doc.active_volume().sample_voxel(c.origin())))
            .collect();
        let was =
            app.doc.active_volume().surface_bounds().expect("the starting ball has a surface");
        let longest_before = (was.1 - was.0).max_element();
        let voxel_before = app.doc.voxel_size();

        app.working_size_field = "30".into();
        update(&mut app, Message::WorkingSizeCommitted);

        let now = app.doc.active_volume().surface_bounds().expect("still a surface");
        let longest_after = (now.1 - now.0).max_element();
        assert!(
            (longest_after - 30.0).abs() < 0.5,
            "asked for 30 mm and got {longest_after:.2} mm (from {longest_before:.2})"
        );

        let factor = 30.0 / longest_before;
        assert!(
            (app.doc.voxel_size() - voxel_before * factor).abs() < 1.0e-6,
            "the voxel size should have scaled with the model"
        );
        for (coord, value) in before {
            assert_eq!(
                app.doc.active_volume().sample_voxel(coord.origin()),
                value,
                "a voxel changed, so this resampled instead of rescaling"
            );
        }
    }

    /// The detail did NOT improve, and the interface must not imply it did.
    #[test]
    fn resizing_buys_no_extra_detail() {
        let mut app = app();
        let bounds = app.doc.active_volume().surface_bounds().expect("a surface");
        let across_before =
            ((bounds.1 - bounds.0).max_element() / app.doc.voxel_size()).round() as i64;

        app.working_size_field = "12".into();
        update(&mut app, Message::WorkingSizeCommitted);

        let bounds = app.doc.active_volume().surface_bounds().expect("a surface");
        let across_after =
            ((bounds.1 - bounds.0).max_element() / app.doc.voxel_size()).round() as i64;
        assert_eq!(
            across_before, across_after,
            "the model has the same number of voxels across it, whatever size it is"
        );
    }

    /// A size that would push the voxel outside the range the finer and coarser
    /// buttons work in has to be refused, or the model lands somewhere those
    /// buttons can never bring it back from.
    #[test]
    fn a_size_that_would_put_the_voxel_out_of_range_is_refused() {
        let mut app = app();
        let voxel_before = app.doc.voxel_size();

        app.working_size_field = "0.001".into();
        update(&mut app, Message::WorkingSizeCommitted);

        assert_eq!(app.doc.voxel_size(), voxel_before, "the model was resized anyway");
        assert!(app.status.contains("could not resize"), "no refusal shown: {}", app.status);
    }

    #[test]
    fn nonsense_in_the_field_is_refused_rather_than_parsed_as_zero() {
        let mut app = app();
        let voxel_before = app.doc.voxel_size();
        app.working_size_field = "big".into();
        update(&mut app, Message::WorkingSizeCommitted);
        assert_eq!(app.doc.voxel_size(), voxel_before);
        assert!(app.status.contains("could not resize"), "{}", app.status);
    }

    /// The radius ceiling at the default voxel size must be **exactly what it
    /// always was**. The voxel rule is there to stop a fine lattice reaching an
    /// unusable brush, not to change the tool anyone is using today.
    #[test]
    fn the_voxel_rule_does_not_touch_the_brush_at_the_default_size() {
        let app = app();
        assert_eq!(app.doc.voxel_size(), VOXEL_SIZE_MM);
        assert_eq!(
            app.max_radius(),
            MAX_RADIUS_MM,
            "100 voxels is 25 mm at the default, above the millimetre ceiling, so nothing moves"
        );
    }

    /// And at a resin lattice it must bite, because the millimetre ceiling
    /// there is 640 voxels of radius -- measured at roughly a quarter of a
    /// second per stamp.
    ///
    /// The lattice is reached with [`Document::rescale`], which multiplies
    /// `voxel_size` and touches no voxel, rather than with `resample`, which
    /// rebuilds the default sphere's narrow band eight times finer.
    /// `max_radius` reads nothing but `doc.voxel_size()`, and `0.25 * 0.125`
    /// is `0.03125` to the bit, so the resample bought this assertion nothing
    /// and was refused rather than merely not chosen: measured on the built
    /// test binary it cost 4.6 s and 2.4 GB of peak RSS for one test, against
    /// 0.08 s and 75 MB through `rescale`. Cargo runs test binaries and their
    /// threads in parallel, so a 2.4 GB spike here is a spike beside the GPU
    /// offscreen suite on a CI runner. If a later change gives `max_radius` a
    /// reason to read actual voxels, this has to become a real resample --
    /// and then say so here.
    #[test]
    fn the_voxel_rule_caps_the_brush_at_a_resin_lattice() {
        let mut app = app();
        app.doc.rescale(0.125);
        assert!(
            (app.max_radius() - 3.125).abs() < 1.0e-6,
            "expected 100 voxels of radius, got {} mm",
            app.max_radius()
        );

        // And a request past it is actually clamped, not merely reported.
        drop(app.update(Message::BrushRadiusChanged(MAX_RADIUS_MM)));
        assert!(
            app.brush.radius <= app.max_radius() + 1.0e-6,
            "the slider handed out {} mm past a {} mm ceiling",
            app.brush.radius,
            app.max_radius()
        );
    }

    /// A finer step the pool cannot hold lands on the finest that fits,
    /// rather than refusing with directions to a rung the buttons cannot
    /// reach. The refusal is kept only for when there is nowhere finer left.
    #[test]
    fn a_step_that_does_not_fit_lands_on_the_finest_that_does() {
        let mut app = app();
        // A pool measured at 6M of 11M vertices: one halving would predict
        // 24M, far over, but there is real room above the current size.
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 6_000_000,
            indices_reserved: 33_000_000,
            vertex_capacity: 11_000_000,
            index_capacity: 66_000_000,
            ..Default::default()
        });

        update(&mut app, Message::Resample(VOXEL_SIZE_MM / 2.0));

        let expected = VOXEL_SIZE_MM / (11.0f32 / 6.0).sqrt() * 1.03;
        assert!(
            (app.doc.voxel_size() - expected).abs() < 1.0e-3,
            "expected to land near {expected:.3} mm, landed at {:.3}",
            app.doc.voxel_size()
        );
        assert!(
            app.status.contains("finest the mesh pool holds"),
            "the status must say the step was capped: {}",
            app.status
        );
    }

    /// Memory is checked before the pool, because since the pool grew itself
    /// buffers it is no longer the tighter of the two. A step the pool would
    /// happily hold can still be one that walks the machine into swap.
    #[test]
    fn a_step_that_would_exhaust_memory_is_capped_even_when_the_pool_has_room() {
        let mut app = app();
        // Plenty of pool...
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 1_000_000,
            indices_reserved: 6_000_000,
            vertex_capacity: hani_gpu::TOTAL_VERTEX_CAPACITY,
            index_capacity: hani_gpu::TOTAL_INDEX_CAPACITY,
            ..Default::default()
        });
        // ...but the volume is already 4 GB, so one halving would want 16.
        app.doc_stats.resident_bytes = 4 * 1024 * 1024 * 1024;
        let before = app.doc.voxel_size();

        update(&mut app, Message::Resample(before / 2.0));

        assert!(app.doc.voxel_size() < before, "it should still go finer, just not that fine");
        assert!(app.doc.voxel_size() > before / 2.0, "and not all the way to the requested size");
        assert!(
            app.status.contains("finest the mesh pool holds"),
            "the cap should be reported: {}",
            app.status
        );
    }

    /// And when the current size already sits at the fit limit, the refusal
    /// stays a refusal -- there is nowhere finer to land.
    #[test]
    fn at_the_fit_limit_the_refusal_remains() {
        let mut app = app();
        app.shared.set_stats_for_tests(hani_gpu::PoolStats {
            vertices_reserved: 10_800_000,
            indices_reserved: 64_000_000,
            vertex_capacity: 11_000_000,
            index_capacity: 66_000_000,
            ..Default::default()
        });
        let before = app.doc.voxel_size();

        update(&mut app, Message::Resample(before / 2.0));

        assert_eq!(app.doc.voxel_size(), before, "there was no room, so nothing should move");
        assert!(app.status.contains("could not resample"), "{}", app.status);
    }

    /// The readout has to name both numbers, because either alone is the
    /// scaling changed no detail, and voxels-across without millimetres cannot
    /// answer "is this enough for my printer".
    #[test]
    fn the_advice_names_both_the_resolution_and_what_it_measures() {
        let app = app();
        let advice = app.measure_detail_advice();
        assert!(advice.contains("voxels wide"), "no voxel count: {advice}");
        assert!(advice.contains("mm across"), "no physical size: {advice}");
        assert!(advice.contains("voxel "), "no voxel size: {advice}");
    }
}
