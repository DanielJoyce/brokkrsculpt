// SPDX-License-Identifier: AGPL-3.0-only

//! Pressing a stamp into the surface: one image, placed with one drag.
//!
//! A stroke lays hundreds of stamps and each moves the surface by no more
//! than a voxel ([`super::engine`], rule 3), because they overlap and what
//! each one does compounds. A rivet is not a stroke. It is one image, as
//! tall as it is, put down once: press where it goes, drag to size it and
//! turn it, let go.
//!
//! That is a gesture, on the terms [`super::MoveStroke`] sets out for Move
//! and for the same reason: every pointer event of the drag has to be worked
//! out from the surface as it was when the button went down, not from what
//! the event before left behind, or the stamp would be pressed in again at
//! every size it passed through on the way.
//!
//! # What one event does
//!
//! The field under the stamp was copied when it was first needed. Every
//! voxel of that box is then rewritten from the copy:
//!
//! ```text
//! new(p) = locked(p - normal * height * alpha(u, v))
//! ```
//!
//! where `u, v` are where `p` is across the stamp's square. Reading the field
//! from `height` further in along the normal brings the inside out by that
//! much, so the surface under a texel of height 1 rises by the whole
//! `height`, and where the alpha is 0 the voxel is read from itself and is
//! exactly what it was. A negative height reads from further out and
//! engraves.
//!
//! Outside the stamp every voxel of the box is put back to the copy, which
//! is what lets the stamp shrink again as the drag comes back.
//!
//! # Only the near side moves
//!
//! Read as written, that formula shifts the whole column under a texel, all
//! the way through the model: a rivet pressed onto a breastplate would raise
//! the outside and dent the inside by the same amount. So the shift is
//! whole only near the surface and fades to nothing with depth:
//!
//! ```text
//! new(p) = locked(p - normal * height * alpha(u, v) * near(depth of p))
//! ```
//!
//! `near` is 1 from above the surface down to a band's depth under it (or
//! under the floor of an engraving), so the surface and the band that
//! carries it move rigidly and exactly. Below that it falls in a straight
//! line to 0 over twice the stamp's height, and everything deeper is not
//! touched at all. Twice, because the material in that stretch is being
//! squeezed or stretched to make up the difference, and a fade any shorter
//! than the height itself would fold it over.
//!
//! A wall thinner than that depth still feels the stamp on its far side,
//! less the thinner the stamp. There is no warp that raises one face of a
//! sheet and leaves the other alone when the sheet is thinner than the
//! rivet is tall; the material has to come from somewhere.
//!
//! # What this does to the field, said plainly
//!
//! It is a warp, so every value written is one the field already held and
//! the band stays a band. It is not an isometry. Up the side of a rivet the
//! field's gradient is steeper than one, as it is after a Move, and a stamp
//! with a vertical side leaves a step of the full band across one voxel
//! there. The surface is where it should be and meshes cleanly; a brush
//! dragged over the side afterwards meets distances that overstate
//! themselves, until [`crate::redistance`] is run.
//!
//! # Mirrors
//!
//! Each twin has its own copy, taken before anything was written. Where two
//! twins' boxes overlap, near a mirror plane, both ask every twin how high
//! its stamp is at the voxel and both take the highest, so they agree and
//! the order they are written in does not show.

use glam::{IVec3, Vec3};

use super::alpha::Alpha;
use super::{Flip, Symmetry};
use crate::brick::NARROW_BAND;
use crate::region::FieldRegion;
use crate::volume::{BrickVerdict, Volume};

/// The most voxels along one side of the box a stamp may lock.
///
/// A copy is a cube of this side, four bytes a voxel, and there is one for
/// each mirror: 320 is 131 MB at worst for one. A stamp wanting more than
/// that is a mistake in the drag, and is refused with the field put back.
pub const MAX_STAMP_SIDE: i32 = 320;

/// How much bigger than it needs a copy is taken, so that a drag growing a
/// pixel at a time does not copy the field again at every pixel.
const GROWTH: f32 = 1.25;

/// Where a stamp goes and how big it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The middle of the stamp, on the surface, in world space.
    pub centre: Vec3,
    /// The surface's outward normal there. The stamp is pressed along it.
    pub normal: Vec3,
    /// Which way the top of the image points, in world space. Only its part
    /// along the surface counts. With none, any direction across the
    /// surface is taken, and the same one each time.
    pub up: Vec3,
    /// Half the side of the stamp's square, in world units.
    pub radius: f32,
    /// How far a texel of height 1 raises the surface, in world units.
    /// Negative engraves.
    pub height: f32,
}

/// Why a stamp was not pressed. The field is as it was before the gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampError {
    /// The stamp is too big to lock: see [`MAX_STAMP_SIDE`].
    TooBig,
    /// The placement has no size, no direction to press in, or a number that
    /// is not one.
    Degenerate,
}

/// One stamp or one mirror of it: its frame, and its copy of the field.
#[derive(Debug, Default)]
struct Twin {
    centre: Vec3,
    normal: Vec3,
    right: Vec3,
    up: Vec3,
    /// The inclusive voxel box that is rewritten, which is the box the copy
    /// was taken for.
    lo: IVec3,
    hi: IVec3,
    locked: FieldRegion,
}

/// How much of a stamp's shift a point takes, by how far along the normal
/// it is from the surface the stamp was pressed on: `along` and everything
/// else in world units, positive outward.
///
/// 1 down to `band` below the surface, or below the floor of an engraving
/// when `height` is negative; then a straight line to 0 over twice the
/// height and a voxel. See the module documentation.
fn near(along: f32, height: f32, band: f32, voxel_size: f32) -> f32 {
    let whole_down_to = -band - (-height).max(0.0);
    let fade = 2.0 * height.abs() + voxel_size;
    ((along - whole_down_to) / fade + 1.0).clamp(0.0, 1.0)
}

/// A stamp being placed.
#[derive(Debug, Default)]
pub struct StampGesture {
    /// Kept allocated across gestures. Only `twins[..count]` is in use.
    twins: Vec<Twin>,
    count: usize,
    active: bool,
    /// The tallest stamp the copies in hand can serve. A read lands up to a
    /// stamp's height from the voxel it is for, so a copy padded for one
    /// height is not good for a taller stamp even inside the same box.
    lift_covered: f32,
}

impl StampGesture {
    pub fn new() -> Self {
        Self::default()
    }

    /// True from [`StampGesture::begin`] to [`StampGesture::end`].
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Start a gesture. Nothing is copied until there is a stamp to press.
    pub fn begin(&mut self) {
        self.count = 0;
        self.active = true;
    }

    /// Finish, leaving the field as the last [`StampGesture::apply`] left
    /// it. The storage is kept for the next gesture.
    pub fn end(&mut self) {
        self.count = 0;
        self.active = false;
    }

    /// Give the gesture up: the field is put back as it was, and the
    /// gesture is over.
    pub fn cancel(&mut self, volume: &mut Volume) {
        self.put_back(volume);
        self.end();
    }

    /// Write every twin's copy back over its box.
    fn put_back(&mut self, volume: &mut Volume) {
        for twin in &self.twins[..self.count] {
            let locked = &twin.locked;
            volume.edit_voxels_where(
                twin.lo,
                twin.hi,
                0,
                false,
                crate::Settled::NONE,
                |_| BrickVerdict::Whole,
                |voxel, _, _, _| locked.get(voxel),
            );
        }
    }

    /// Press `alpha` into the field at `placement`, and at its mirrors,
    /// replacing whatever this gesture pressed before.
    ///
    /// On an error the field is as it was before the gesture began, and the
    /// gesture is still active: a drag that has gone too far can come back.
    pub fn apply(
        &mut self,
        volume: &mut Volume,
        alpha: &Alpha,
        placement: &Placement,
        symmetry: Symmetry,
        mirror_centre: Vec3,
    ) -> Result<(), StampError> {
        debug_assert!(self.active, "a stamp applied outside a gesture");
        let voxel_size = volume.voxel_size();
        let Some(frame) = Frame::of(placement) else {
            self.put_back(volume);
            self.count = 0;
            return Err(StampError::Degenerate);
        };

        // The stamp and each of its mirrors.
        let mut flips = [Flip::IDENTITY; Symmetry::MAX_MIRRORS];
        let mirrors = if symmetry.is_off() { 0 } else { symmetry.flips(mirror_centre, &mut flips) };
        let frames: Vec<Frame> = std::iter::once(frame)
            .chain(flips[..mirrors].iter().map(|flip| frame.flipped(*flip)))
            .collect();

        // How far the stamp reaches from its centre. Across, to the corner
        // of its square. Along the normal, up to the top of the stamp and
        // its band, and down to where `near` reaches nothing: a band, an
        // engraving's depth, and the fade.
        let lift = placement.height.abs();
        let band = (NARROW_BAND + 1.0) * voxel_size;
        let reach = placement.radius * std::f32::consts::SQRT_2 + band + 3.0 * lift + voxel_size;
        let boxes: Vec<(IVec3, IVec3)> = frames
            .iter()
            .map(|frame| {
                volume.voxel_bounds(
                    frame.centre - Vec3::splat(reach),
                    frame.centre + Vec3::splat(reach),
                )
            })
            .collect();

        // The copies in hand are good while there are as many as there are
        // twins and each still covers what its twin needs.
        let covered = self.count == frames.len()
            && lift <= self.lift_covered
            && self.twins[..self.count]
                .iter()
                .zip(&boxes)
                .all(|(twin, (lo, hi))| twin.lo.cmple(*lo).all() && twin.hi.cmpge(*hi).all());
        if !covered {
            // Back to the surface as it was, and copy it again, larger.
            self.put_back(volume);
            self.count = 0;
            let grown = reach * GROWTH;
            let grown_boxes: Vec<(IVec3, IVec3)> = frames
                .iter()
                .map(|frame| {
                    volume.voxel_bounds(
                        frame.centre - Vec3::splat(grown),
                        frame.centre + Vec3::splat(grown),
                    )
                })
                .collect();
            // The whole stamp is refused if any copy is, and the smallest
            // box that would do is the one judged: the growth is a
            // convenience and must not be what makes a stamp too big.
            if boxes.iter().any(|(lo, hi)| (*hi - *lo + IVec3::ONE).max_element() > MAX_STAMP_SIDE)
            {
                return Err(StampError::TooBig);
            }
            while self.twins.len() < frames.len() {
                self.twins.push(Twin::default());
            }
            // A read lands up to the height away along the normal, and the
            // trilinear tap one voxel past that. Padded for a taller stamp
            // than this one, for the reason the box is grown.
            self.lift_covered = lift * GROWTH;
            let pad = IVec3::splat((self.lift_covered / voxel_size).ceil() as i32 + 2);
            for ((twin, (needed_lo, needed_hi)), (grown_lo, grown_hi)) in
                self.twins.iter_mut().zip(&boxes).zip(&grown_boxes)
            {
                // The grown box where it fits, the needed one where the
                // growth would take it past the limit.
                let fits = (*grown_hi - *grown_lo + IVec3::ONE).max_element() <= MAX_STAMP_SIDE;
                let (lo, hi) = if fits { (*grown_lo, *grown_hi) } else { (*needed_lo, *needed_hi) };
                twin.lo = lo;
                twin.hi = hi;
                volume.snapshot(lo - pad, hi + pad, &mut twin.locked);
            }
            self.count = frames.len();
        }
        for (twin, frame) in self.twins[..self.count].iter_mut().zip(&frames) {
            twin.centre = frame.centre;
            twin.normal = frame.normal;
            twin.right = frame.right;
            twin.up = frame.up;
        }

        let twins = &self.twins[..self.count];
        let (radius, height) = (placement.radius, placement.height);
        for twin in twins {
            let own = &twin.locked;
            volume.edit_voxels_where(
                twin.lo,
                twin.hi,
                0,
                true,
                crate::Settled::NONE,
                |_| BrickVerdict::Whole,
                |voxel, position, _, free| {
                    let was = own.get(voxel);
                    // The highest stamp over this voxel, of every twin's.
                    let mut highest = 0.0_f32;
                    let mut by = twin;
                    for other in twins {
                        let offset = position - other.centre;
                        let level = alpha.sample(
                            offset.dot(other.right) / radius,
                            offset.dot(other.up) / radius,
                        );
                        if level > highest {
                            highest = level;
                            by = other;
                        }
                    }
                    if highest <= 0.0 || free <= 0.0 {
                        return was;
                    }
                    // A voxel under a twin's stamp is inside that twin's
                    // box, so that twin's copy covers the read.
                    let along = (position - by.centre).dot(by.normal);
                    let shift = height * highest * near(along, height, band, voxel_size);
                    if shift == 0.0 {
                        return was;
                    }
                    let from = position - by.normal * shift;
                    let target = by.locked.sample(from / voxel_size);
                    if free >= 1.0 { target } else { was + (target - was) * free }
                },
            );
        }
        Ok(())
    }
}

/// A stamp's own axes.
#[derive(Debug, Clone, Copy)]
struct Frame {
    centre: Vec3,
    normal: Vec3,
    right: Vec3,
    up: Vec3,
}

impl Frame {
    fn of(placement: &Placement) -> Option<Self> {
        let finite = placement.centre.is_finite()
            && placement.normal.is_finite()
            && placement.up.is_finite()
            && placement.radius.is_finite()
            && placement.height.is_finite();
        if !finite || placement.radius <= 0.0 {
            return None;
        }
        let normal = placement.normal.try_normalize()?;
        // The top of the image, laid into the surface.
        let lying = placement.up - normal * placement.up.dot(normal);
        let up = lying.try_normalize().unwrap_or_else(|| normal.any_orthonormal_vector());
        // Seen from outside with `up` up the screen, this is to the right.
        let right = up.cross(normal);
        Some(Self { centre: placement.centre, normal, right, up })
    }

    /// The mirror image. Each axis is reflected, and `right` is NOT worked
    /// out again from the other two: a mirror turns a right hand into a
    /// left, and the cross product would quietly turn it back, pressing the
    /// image the wrong way round on the far side.
    fn flipped(self, flip: Flip) -> Self {
        Self {
            centre: flip.point(self.centre),
            normal: self.normal * flip.sign,
            right: self.right * flip.sign,
            up: self.up * flip.sign,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::{INSIDE, OUTSIDE};
    use crate::brush::alpha::BuiltIn;
    use crate::mask::PROTECTED;
    use crate::testing::assert_same_field;

    const VOXEL: f32 = 0.5;
    const HALF: f32 = 12.0;

    /// A block 24 mm on a side about the origin. Stamps go on its top, +z.
    fn block() -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(HALF + NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(HALF);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / VOXEL).clamp(INSIDE, OUTSIDE)
        });
        volume.take_dirty(&mut Vec::new());
        volume
    }

    fn on_top(x: f32, y: f32, radius: f32, height: f32) -> Placement {
        Placement { centre: Vec3::new(x, y, HALF), normal: Vec3::Z, up: Vec3::Y, radius, height }
    }

    /// One gesture of one stamp, start to finish.
    fn pressed(alpha: &Alpha, placement: Placement, symmetry: Symmetry) -> Volume {
        let mut volume = block();
        let mut gesture = StampGesture::new();
        gesture.begin();
        gesture.apply(&mut volume, alpha, &placement, symmetry, Vec3::ZERO).expect("a stamp");
        gesture.end();
        volume
    }

    /// How high the top surface is above the block's own top at `(x, y)`,
    /// in millimetres: where the field changes sign up the column.
    fn rise(volume: &Volume, x: f32, y: f32) -> f32 {
        let mut below = -6.0_f32;
        let mut step = 0.05_f32;
        // Walk up from well inside until the field is no longer negative.
        while below < 8.0 && volume.sample_world(Vec3::new(x, y, HALF + below + step)) < 0.0 {
            below += step;
        }
        // Then close in.
        for _ in 0..12 {
            step *= 0.5;
            if volume.sample_world(Vec3::new(x, y, HALF + below + step)) < 0.0 {
                below += step;
            }
        }
        below
    }

    /// An image with one bright blob up and to the right of its middle, and
    /// nothing anywhere else: which way round it lands is unmistakable.
    fn off_centre() -> Alpha {
        let size = 32;
        let mut heights = vec![0.0; size * size];
        for row in 0..size {
            for column in 0..size {
                let x = (column as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                let y = 1.0 - (row as f32 + 0.5) / size as f32 * 2.0;
                let from_blob = ((x - 0.45).powi(2) + (y - 0.45).powi(2)).sqrt();
                heights[row * size + column] = (1.0 - from_blob / 0.3).clamp(0.0, 1.0);
            }
        }
        Alpha::from_heights(size, size, &heights).expect("a square of heights")
    }

    /// A stamp raises the surface by its image: the full height where the
    /// image is white, nothing where it is black, and not a bit of the field
    /// outside its square.
    #[test]
    fn a_stamp_raises_the_surface_by_its_image_and_touches_nothing_else() {
        let rivet = Alpha::built_in(BuiltIn::Rivet);
        let before = block();
        let after = pressed(&rivet, on_top(1.0, -0.5, 4.0, 1.5), Symmetry::OFF);

        // The middle of a rivet is its flat top.
        assert!((rise(&after, 1.0, -0.5) - 1.5).abs() < 0.05, "{}", rise(&after, 1.0, -0.5));
        assert!((rise(&after, 2.0, 0.5) - 1.5).abs() < 0.05, "the top of a rivet is not flat");
        // Past its bevel, still inside the square, the surface has not moved.
        assert!(rise(&after, 1.0 + 3.8, -0.5).abs() < 0.02, "{}", rise(&after, 4.8, -0.5));
        // Half way up the bevel it is part way up. 0.6 to 0.85 of 4 mm is
        // 2.4 to 3.4 mm out, so 2.9 is the middle of the ramp.
        let on_the_bevel = rise(&after, 1.0 + 2.9, -0.5);
        assert!(on_the_bevel > 0.4 && on_the_bevel < 1.1, "the bevel is at {on_the_bevel}");

        // Outside the square nothing is so much as rounded differently.
        let (mut same, mut moved) = (0, 0);
        for z in -40..=40 {
            for y in -40..=40 {
                for x in -40..=40 {
                    let voxel = IVec3::new(x, y, z);
                    let (was, now) = (before.sample_voxel(voxel), after.sample_voxel(voxel));
                    assert!(now.is_finite() && (INSIDE..=OUTSIDE).contains(&now), "{voxel}: {now}");
                    let across = voxel.as_vec3() * VOXEL - Vec3::new(1.0, -0.5, 0.0);
                    if across.x.abs().max(across.y.abs()) >= 4.0 {
                        assert_eq!(was.to_bits(), now.to_bits(), "{voxel} is outside the stamp");
                        same += 1;
                    } else {
                        moved += usize::from(was != now);
                    }
                }
            }
        }
        assert!(same > 100_000 && moved > 500, "{same} compared, {moved} moved");
    }

    /// A negative height engraves, by as much.
    #[test]
    fn a_negative_height_engraves() {
        let rivet = Alpha::built_in(BuiltIn::Rivet);
        let after = pressed(&rivet, on_top(0.0, 0.0, 4.0, -1.5), Symmetry::OFF);
        assert!((rise(&after, 0.0, 0.0) + 1.5).abs() < 0.05, "{}", rise(&after, 0.0, 0.0));
        assert!(rise(&after, 3.8, 0.0).abs() < 0.02);
    }

    /// Every event of a drag is worked out from the surface as it was when
    /// the gesture began. So a stamp dragged out and back in is the stamp at
    /// the size it ended at and nothing of the sizes it passed through, and
    /// a stamp grown past its first copy is the same as one made that size
    /// at once.
    #[test]
    fn a_drag_replaces_the_stamp_and_never_presses_it_in_twice() {
        let bolt = Alpha::built_in(BuiltIn::Bolt);
        let dragged = |sizes: &[(f32, f32)]| {
            let mut volume = block();
            let mut gesture = StampGesture::new();
            gesture.begin();
            for (radius, height) in sizes {
                gesture
                    .apply(
                        &mut volume,
                        &bolt,
                        &on_top(0.5, 0.5, *radius, *height),
                        Symmetry::OFF,
                        Vec3::ZERO,
                    )
                    .expect("a stamp");
            }
            assert!(gesture.is_active());
            gesture.end();
            assert!(!gesture.is_active());
            volume
        };
        let small = dragged(&[(2.0, 1.0)]);
        assert_same_field(
            &dragged(&[(1.0, 0.5), (5.0, 2.0), (3.0, 1.0), (2.0, 1.0)]),
            &small,
            "a stamp dragged out and back is not the stamp it ended as",
        );
        // Grown well past what was first copied, in one step and in many.
        let large = dragged(&[(7.0, 2.5)]);
        assert_same_field(&dragged(&[(1.0, 0.3), (7.0, 2.5)]), &large, "grown in one step");
        let steps: Vec<(f32, f32)> =
            (1..=28).map(|step| (step as f32 * 0.25, step as f32 * 2.5 / 28.0)).collect();
        assert_same_field(&dragged(&steps), &large, "grown a quarter millimetre at a time");
        // And the control: those two sizes are different stamps.
        assert!((rise(&small, 0.5, 0.5) - rise(&large, 0.5, 0.5)).abs() > 1.0);
    }

    /// Cancelling puts the field back to the bit, and so does a stamp that
    /// is refused.
    #[test]
    fn a_cancelled_or_refused_stamp_leaves_the_field_as_it_was() {
        let dome = Alpha::built_in(BuiltIn::Dome);
        let untouched = block();

        let mut volume = block();
        let mut gesture = StampGesture::new();
        gesture.begin();
        gesture
            .apply(&mut volume, &dome, &on_top(0.0, 0.0, 3.0, 2.0), Symmetry::OFF, Vec3::ZERO)
            .unwrap();
        assert!(rise(&volume, 0.0, 0.0) > 1.5, "the stamp was not pressed in the first place");
        gesture.cancel(&mut volume);
        assert!(!gesture.is_active());
        assert_same_field(&volume, &untouched, "a cancelled stamp left a mark");

        // Refused part way through a drag: the stamp already there goes.
        gesture.begin();
        gesture
            .apply(&mut volume, &dome, &on_top(0.0, 0.0, 3.0, 2.0), Symmetry::OFF, Vec3::ZERO)
            .unwrap();
        let huge = on_top(0.0, 0.0, MAX_STAMP_SIDE as f32 * VOXEL, 1.0);
        assert_eq!(
            gesture.apply(&mut volume, &dome, &huge, Symmetry::OFF, Vec3::ZERO),
            Err(StampError::TooBig)
        );
        assert_same_field(&volume, &untouched, "a stamp too big to lock left the last one behind");
        assert!(gesture.is_active(), "a refusal ended the gesture, so the drag cannot come back");
        // And it can come back.
        gesture
            .apply(&mut volume, &dome, &on_top(0.0, 0.0, 3.0, 2.0), Symmetry::OFF, Vec3::ZERO)
            .unwrap();
        assert!(rise(&volume, 0.0, 0.0) > 1.5, "the drag came back and nothing was pressed");
        gesture.cancel(&mut volume);

        for broken in [
            Placement { radius: 0.0, ..on_top(0.0, 0.0, 3.0, 2.0) },
            Placement { radius: -2.0, ..on_top(0.0, 0.0, 3.0, 2.0) },
            Placement { radius: f32::NAN, ..on_top(0.0, 0.0, 3.0, 2.0) },
            Placement { height: f32::INFINITY, ..on_top(0.0, 0.0, 3.0, 2.0) },
            Placement { normal: Vec3::ZERO, ..on_top(0.0, 0.0, 3.0, 2.0) },
            Placement { centre: Vec3::splat(f32::NAN), ..on_top(0.0, 0.0, 3.0, 2.0) },
        ] {
            gesture.begin();
            assert_eq!(
                gesture.apply(&mut volume, &dome, &broken, Symmetry::OFF, Vec3::ZERO),
                Err(StampError::Degenerate),
                "{broken:?}"
            );
            gesture.end();
            assert_same_field(&volume, &untouched, "a stamp with no shape left a mark");
        }
    }

    /// The top of the image points where it is told to, and the image is
    /// not mirrored: its right is on the right, seen from outside.
    #[test]
    fn the_image_lands_the_way_up_it_is_turned_and_the_right_way_round() {
        let blob = off_centre();
        // The blob is at (0.45, 0.45) of the square, so with a 4 mm stamp it
        // is 1.8 mm right and 1.8 mm up from the middle.
        let raised = |volume: &Volume, x: f32, y: f32| rise(volume, x, y) > 0.8;
        let placed = |up: Vec3| {
            pressed(&blob, Placement { up, ..on_top(0.0, 0.0, 4.0, 1.5) }, Symmetry::OFF)
        };

        let upright = placed(Vec3::Y);
        assert!(raised(&upright, 1.8, 1.8), "the blob is not up and to the right");
        for (x, y) in [(-1.8, 1.8), (1.8, -1.8), (-1.8, -1.8)] {
            assert!(!raised(&upright, x, y), "the blob is also at {x} {y}");
        }
        // Top pointing along +x: a quarter turn clockwise seen from above,
        // which takes up-right to down-right.
        let turned = placed(Vec3::X);
        assert!(raised(&turned, 1.8, -1.8), "a quarter turn did not carry the blob round");
        assert!(!raised(&turned, 1.8, 1.8));
        // Upside down.
        let inverted = placed(-Vec3::Y);
        assert!(raised(&inverted, -1.8, -1.8));
        // Only the part of `up` along the surface counts, and with none the
        // stamp is still pressed, somewhere, the same way each time.
        assert_same_field(&placed(Vec3::new(0.0, 3.0, 9.0)), &upright, "the lift in `up` counted");
        assert_same_field(&placed(Vec3::Z), &placed(Vec3::ZERO), "no direction is not one answer");
    }

    /// A mirrored stamp is the mirror image, and where the two overlap they
    /// agree.
    #[test]
    fn a_mirrored_stamp_is_the_mirror_image_even_where_the_two_overlap() {
        let blob = off_centre();
        let symmetric = |volume: &Volume, what: &str| {
            let mut compared = 0;
            for z in 10..=40 {
                for y in -30..=30 {
                    for x in 1..=40 {
                        let (here, there) = (
                            volume.sample_voxel(IVec3::new(x, y, z)),
                            volume.sample_voxel(IVec3::new(-x, y, z)),
                        );
                        assert!(
                            (here - there).abs() < 1.0e-4,
                            "{what}: {x} {y} {z}: {here} {there}"
                        );
                        compared += 1;
                    }
                }
            }
            assert!(compared > 50_000);
        };

        // Well clear of the plane: the blob is on the outer side of each.
        let apart = pressed(&blob, on_top(6.0, 0.0, 4.0, 1.5), Symmetry::X);
        symmetric(&apart, "apart");
        assert!(rise(&apart, 6.0 + 1.8, 1.8) > 0.8, "the stamp itself is not there");
        assert!(rise(&apart, -6.0 - 1.8, 1.8) > 0.8, "the twin's blob is not the mirror image");
        assert!(rise(&apart, -6.0 + 1.8, 1.8) < 0.2, "the twin was pressed the same way round");
        // The control: without the mirror there is nothing on the far side.
        let alone = pressed(&blob, on_top(6.0, 0.0, 4.0, 1.5), Symmetry::OFF);
        assert!(rise(&alone, -6.0 - 1.8, 1.8).abs() < 0.02);

        // Close enough that the two squares overlap across the plane.
        let dome = Alpha::built_in(BuiltIn::Dome);
        let close = pressed(&dome, on_top(1.5, 0.0, 4.0, 2.0), Symmetry::X);
        symmetric(&close, "overlapping");
        // On the plane itself both domes are part way down their sides, and
        // the surface is at the higher of the two, not their sum and not
        // whichever was written last.
        let one = pressed(&dome, on_top(1.5, 0.0, 4.0, 2.0), Symmetry::OFF);
        assert!((rise(&close, 0.0, 0.0) - rise(&one, 0.0, 0.0)).abs() < 0.05);
        assert!(rise(&close, 0.0, 0.0) > 1.0);
    }

    /// A plate six millimetres thick, top at z = 3 and underside at z = -3.
    fn plate() -> Volume {
        let mut volume = Volume::new(VOXEL);
        let size = Vec3::new(HALF, HALF, 3.0);
        let reach = size + Vec3::splat(NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - size;
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / VOXEL).clamp(INSIDE, OUTSIDE)
        });
        volume.take_dirty(&mut Vec::new());
        volume
    }

    fn on_the_plate(radius: f32, height: f32) -> Placement {
        Placement { centre: Vec3::new(0.0, 0.0, 3.0), normal: Vec3::Z, up: Vec3::Y, radius, height }
    }

    /// A rivet on a plate raises the top and leaves the underside alone, to
    /// the bit: the shift fades out with depth. Raised or engraved, the
    /// surface itself still moves by the whole height.
    #[test]
    fn a_stamp_moves_the_near_side_and_not_the_far_side_of_a_plate() {
        let rivet = Alpha::built_in(BuiltIn::Rivet);
        let untouched = plate();
        for height in [0.5, -0.5] {
            let mut volume = plate();
            let mut gesture = StampGesture::new();
            gesture.begin();
            gesture
                .apply(&mut volume, &rivet, &on_the_plate(4.0, height), Symmetry::OFF, Vec3::ZERO)
                .unwrap();
            gesture.end();
            // The top moved by the whole height, in the middle of the rivet.
            let top = (0..200)
                .map(|step| 6.0 - step as f32 * 0.02)
                .find(|z| volume.sample_world(Vec3::new(0.0, 0.0, *z)) < 0.0)
                .expect("the plate has a top");
            assert!((top - (3.0 + height)).abs() < 0.05, "at {height}: the top is at {top}");
            // A band, the depth of an engraving, twice the height and a
            // voxel: 2 + 0.5 + 1 + 0.5 mm at most. Below z = -1 nothing.
            let mut compared = 0;
            for z in -14..=-2 {
                for y in -12..=12 {
                    for x in -12..=12 {
                        let voxel = IVec3::new(x, y, z);
                        assert_eq!(
                            volume.sample_voxel(voxel).to_bits(),
                            untouched.sample_voxel(voxel).to_bits(),
                            "at {height}: {voxel} is on the far side and moved"
                        );
                        compared += 1;
                    }
                }
            }
            assert!(compared > 5000);
        }
    }

    #[test]
    fn the_shift_is_whole_at_the_surface_and_gone_at_depth_and_never_folds() {
        let (band, voxel) = (2.0, 0.5);
        for height in [0.3_f32, 2.0, -0.3, -2.0] {
            // Whole from far above down to a band under the lower of the
            // old surface and the new one.
            let floor = height.min(0.0);
            for along in [9.0, height.max(0.0), 0.0, floor, floor - band] {
                assert_eq!(near(along, height, band, voxel), 1.0, "{height} at {along}");
            }
            let gone = floor - band - 2.0 * height.abs() - voxel;
            // The end of the fade is where a subtraction rounds: nothing to
            // within that, and exactly nothing a little past it.
            assert!(near(gone, height, band, voxel) < 1.0e-6, "{height} at {gone}");
            assert_eq!(near(gone - 5.0, height, band, voxel), 0.0);
            // Where a voxel reads from goes up as the voxel does: two
            // voxels never read past each other, which would fold the
            // field over.
            let reads_from = |along: f32| along - height * near(along, height, band, voxel);
            let mut last = reads_from(-12.0);
            for step in 1..=480 {
                let now = reads_from(-12.0 + step as f32 * 0.05);
                assert!(now > last, "{height}: the read went backwards at step {step}");
                last = now;
            }
        }
    }

    /// A copy taken for a low stamp is not used for a tall one, even when
    /// the tall one fits inside the same box: its reads land further away.
    /// Low and wide and then tall and narrow is the tall stamp, to the bit.
    #[test]
    fn a_taller_stamp_in_the_same_box_gets_a_copy_that_reaches_far_enough() {
        let dome = Alpha::built_in(BuiltIn::Dome);
        let dragged = |stamps: &[(f32, f32)]| {
            let mut volume = plate();
            let mut gesture = StampGesture::new();
            gesture.begin();
            for (radius, height) in stamps {
                gesture
                    .apply(
                        &mut volume,
                        &dome,
                        &on_the_plate(*radius, *height),
                        Symmetry::OFF,
                        Vec3::ZERO,
                    )
                    .unwrap();
            }
            gesture.end();
            volume
        };
        // The second is taller than the plate is thick, so its reads come
        // from the air on the far side by way of everything between.
        for tall in [(2.0, 4.0), (2.0, -4.0)] {
            assert_same_field(
                &dragged(&[(8.0, 0.1), tall]),
                &dragged(&[tall]),
                "a tall stamp after a low wide one is not the tall stamp",
            );
        }
    }

    /// The one place the copy's padding is all that stands between a stamp
    /// and a wrong answer: engraving under an overhang. An engraving reads
    /// from further OUT, past the top of its box, and under a ceiling what
    /// is out there is not air. A copy padded for a shallow stamp, reused
    /// for a deep one that fits the same box, read the ceiling as the rim
    /// of its own snapshot: measured, as a different field.
    #[test]
    fn a_deeper_engraving_under_an_overhang_reads_what_is_really_above_it() {
        let dome = Alpha::built_in(BuiltIn::Dome);
        // The plate, and a second slab over it with a gap of 19 mm between:
        // its underside is just past the top of the box a shallow stamp of
        // radius 8 locks.
        let under_a_ceiling = || {
            let mut volume = Volume::new(VOXEL);
            let size = Vec3::new(HALF, HALF, 3.0);
            let ceiling = Vec3::new(0.0, 0.0, 25.0);
            let reach = Vec3::new(HALF, HALF, 30.0) + Vec3::splat(NARROW_BAND * VOXEL * 2.0);
            volume.edit_box(-reach, reach, move |position, _| {
                let slab = |centre: Vec3| {
                    let outside = (position - centre).abs() - size;
                    outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0)
                };
                (slab(Vec3::ZERO).min(slab(ceiling)) / VOXEL).clamp(INSIDE, OUTSIDE)
            });
            volume.take_dirty(&mut Vec::new());
            volume
        };
        let dragged = |stamps: &[(f32, f32)]| {
            let mut volume = under_a_ceiling();
            let mut gesture = StampGesture::new();
            gesture.begin();
            for (radius, height) in stamps {
                gesture
                    .apply(
                        &mut volume,
                        &dome,
                        &on_the_plate(*radius, *height),
                        Symmetry::OFF,
                        Vec3::ZERO,
                    )
                    .unwrap();
            }
            gesture.end();
            volume
        };
        let deep = (2.0, -4.0);
        assert_same_field(
            &dragged(&[(8.0, -0.1), deep]),
            &dragged(&[deep]),
            "a deep engraving after a shallow wide one is not the deep engraving",
        );
    }

    /// The mask holds a stamp off, as it does every other edit.
    #[test]
    fn a_stamp_respects_the_mask() {
        let dome = Alpha::built_in(BuiltIn::Dome);
        let masked = || {
            let mut volume = block();
            volume.edit_mask(IVec3::splat(-60), IVec3::splat(60), |_, at, _| {
                if at.x < 0.0 { PROTECTED } else { 0 }
            });
            volume
        };
        let untouched = masked();
        let mut volume = masked();
        let mut gesture = StampGesture::new();
        gesture.begin();
        gesture
            .apply(&mut volume, &dome, &on_top(0.0, 0.0, 4.0, 2.0), Symmetry::OFF, Vec3::ZERO)
            .unwrap();
        gesture.end();
        assert!(rise(&volume, 1.0, 0.0) > 1.2, "the free half was not stamped");
        for z in 14..=40 {
            for y in -12..=12 {
                for x in -12..0 {
                    let voxel = IVec3::new(x, y, z);
                    assert_eq!(
                        volume.sample_voxel(voxel).to_bits(),
                        untouched.sample_voxel(voxel).to_bits(),
                        "{voxel} is protected and moved"
                    );
                }
            }
        }
    }
}
