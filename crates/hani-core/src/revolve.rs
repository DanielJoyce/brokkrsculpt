// SPDX-License-Identifier: AGPL-3.0-only

//! Revolve: a profile drawn beside an axis, spun round it into a solid.
//!
//! The profile is a line of points `(across, along)` in millimetres: how far
//! from the axis, and how far along it. Which side of the axis a point is on
//! does not matter, only how far, so a squiggle drawn either side, or across
//! the axis, spins into the same thing as its fold onto one side.
//!
//! # The shape
//!
//! The solid's meridian section is the region the profile bounds together
//! with its mirror image: the profile, a flat cap across the axis at its last
//! point, the profile mirrored and walked back, and a cap at its first. That
//! closed outline is symmetric about the axis, so a point at distance `r`
//! from the axis and `h` along it is as far from the solid as `(r, h)` is from
//! the outline, exactly: the nearest point of a solid of revolution lies in
//! the same half plane. Inside and outside is the even-odd rule, so a profile
//! that crosses itself still gives a solid.
//!
//! The outline's distance is worked out once on a grid of half a voxel in
//! `(r, h)` and read bilinearly per voxel, so the cost per voxel does not
//! grow with the number of points drawn.
//!
//! # Not done
//!
//! * **A profile is one line.** Holes, a second line inside the first, are
//!   not; a tube is drawn as a squiggle that goes up one side of the wall
//!   and down the other.
//! * **Not smoothed.** The points are used as given; a caller thins them.

use glam::{Vec2, Vec3};
use rayon::prelude::*;

use crate::brick::{BRICK_DIM, Brick, BrickCoord, INSIDE, NARROW_BAND, OUTSIDE, brick_index};
use crate::shell::{settled, voxel_of};
use crate::volume::Volume;

/// A revolve: the axis, as a point on it and its direction, and the profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Revolve {
    pub origin: Vec3,
    pub axis: Vec3,
    /// `(across, along)` in millimetres from `origin`.
    pub profile: Vec<Vec2>,
}

/// Why nothing was made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RevolveError {
    /// Fewer than two points.
    TooShort,
    /// Every point is within a voxel of the axis, or the line is shorter
    /// than one along it and across it both.
    TooThin,
}

impl std::fmt::Display for RevolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RevolveError::TooShort => write!(f, "draw a line beside the axis"),
            RevolveError::TooThin => write!(f, "the line is too close to the axis to make a solid"),
        }
    }
}

/// The closed outline of a profile's meridian section: the profile folded
/// onto one side, then mirrored and walked back.
fn outline(profile: &[Vec2]) -> Vec<Vec2> {
    let folded: Vec<Vec2> = profile.iter().map(|p| Vec2::new(p.x.abs(), p.y)).collect();
    let mirrored = folded.iter().rev().map(|p| Vec2::new(-p.x, p.y));
    folded.iter().copied().chain(mirrored).collect()
}

/// Signed distance from `p` to a closed polygon, negative inside by the
/// even-odd rule.
fn polygon_distance(polygon: &[Vec2], p: Vec2) -> f32 {
    let mut nearest = f32::INFINITY;
    let mut inside = false;
    let count = polygon.len();
    for i in 0..count {
        let a = polygon[i];
        let b = polygon[(i + 1) % count];
        let edge = b - a;
        let to = p - a;
        let along = (to.dot(edge) / edge.length_squared().max(1e-12)).clamp(0.0, 1.0);
        nearest = nearest.min((to - edge * along).length_squared());
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    let distance = nearest.sqrt();
    if inside { -distance } else { distance }
}

/// The outline's distance on a grid in `(r, h)`, millimetres.
struct Table {
    low: Vec2,
    step: f32,
    width: usize,
    height: usize,
    values: Vec<f32>,
}

impl Table {
    fn new(polygon: &[Vec2], low: Vec2, high: Vec2, step: f32) -> Self {
        let width = ((high.x - low.x) / step).ceil() as usize + 2;
        let height = ((high.y - low.y) / step).ceil() as usize + 2;
        let values = (0..width * height)
            .into_par_iter()
            .map(|index| {
                let at = low + Vec2::new((index % width) as f32, (index / width) as f32) * step;
                polygon_distance(polygon, at)
            })
            .collect();
        Table { low, step, width, height, values }
    }

    fn at(&self, p: Vec2) -> f32 {
        let grid = ((p - self.low) / self.step)
            .clamp(Vec2::ZERO, Vec2::new((self.width - 2) as f32, (self.height - 2) as f32));
        let base = grid.floor();
        let frac = grid - base;
        let (x, y) = (base.x as usize, base.y as usize);
        let value = |dx: usize, dy: usize| self.values[(y + dy) * self.width + x + dx];
        let bottom = value(0, 0) + (value(1, 0) - value(0, 0)) * frac.x;
        let top = value(0, 1) + (value(1, 1) - value(0, 1)) * frac.x;
        bottom + (top - bottom) * frac.y
    }
}

/// The solid a revolve makes, at `voxel_size`, every brick marked dirty.
pub fn revolve(voxel_size: f32, how: &Revolve) -> Result<Volume, RevolveError> {
    if how.profile.len() < 2 {
        return Err(RevolveError::TooShort);
    }
    let axis = how.axis.try_normalize().unwrap_or(Vec3::Y);
    let polygon = outline(&how.profile);
    let reach = polygon.iter().fold(0.0f32, |most, p| most.max(p.x.abs()));
    let low_h = polygon.iter().fold(f32::INFINITY, |least, p| least.min(p.y));
    let high_h = polygon.iter().fold(f32::NEG_INFINITY, |most, p| most.max(p.y));
    if reach < voxel_size {
        return Err(RevolveError::TooThin);
    }
    let band = NARROW_BAND * voxel_size;
    let margin = band + 2.0 * voxel_size;
    let table = Table::new(
        &polygon,
        Vec2::new(0.0, low_h - margin),
        Vec2::new(reach + margin, high_h + margin),
        voxel_size * 0.5,
    );
    let distance = |world: Vec3| {
        let q = world - how.origin;
        let h = q.dot(axis);
        let r = (q - h * axis).length();
        table.at(Vec2::new(r, h))
    };

    // The box round a cylinder of radius `reach + margin` along the axis.
    let middle = how.origin + axis * (low_h + high_h) * 0.5;
    let half_length = (high_h - low_h) * 0.5 + margin;
    let radius = reach + margin;
    let extent = Vec3::new(
        radius * (1.0 - axis.x * axis.x).max(0.0).sqrt() + half_length * axis.x.abs(),
        radius * (1.0 - axis.y * axis.y).max(0.0).sqrt() + half_length * axis.y.abs(),
        radius * (1.0 - axis.z * axis.z).max(0.0).sqrt() + half_length * axis.z.abs(),
    );
    let first = BrickCoord::containing(((middle - extent) / voxel_size).floor().as_ivec3()).0;
    let last = BrickCoord::containing(((middle + extent) / voxel_size).ceil().as_ivec3()).0;
    let mut coords = Vec::new();
    for z in first.z..=last.z {
        for y in first.y..=last.y {
            for x in first.x..=last.x {
                coords.push(BrickCoord::new(x, y, z));
            }
        }
    }
    let half_diagonal = (BRICK_DIM as f32 * 0.5) * 3f32.sqrt() * voxel_size;
    let built: Vec<(BrickCoord, Brick)> = coords
        .par_iter()
        .filter_map(|&coord| {
            let origin = coord.origin();
            let centre =
                (origin.as_vec3() + Vec3::splat((BRICK_DIM as f32 - 1.0) * 0.5)) * voxel_size;
            let middle = distance(centre);
            // The table is a distance, so it changes no faster than the
            // point does; half a voxel of grid error is in the slack.
            if middle - half_diagonal >= band + voxel_size {
                return None;
            }
            if middle + half_diagonal <= -band - voxel_size {
                return Some((coord, Brick::Uniform(INSIDE)));
            }
            let mut brick = Brick::dense_filled(OUTSIDE);
            let data = brick.make_dense();
            for z in 0..BRICK_DIM {
                for y in 0..BRICK_DIM {
                    for x in 0..BRICK_DIM {
                        let at = voxel_of(origin, x, y, z).as_vec3() * voxel_size;
                        data[brick_index(x, y, z)] =
                            (distance(at) / voxel_size).clamp(INSIDE, OUTSIDE);
                    }
                }
            }
            settled(brick).map(|brick| (coord, brick))
        })
        .collect();
    if built.is_empty() {
        return Err(RevolveError::TooThin);
    }
    let mut volume = Volume::new(voxel_size);
    for (coord, brick) in built {
        volume.insert_brick(coord, brick);
    }
    volume.mark_everything_dirty();
    Ok(volume)
}

/// `points` thinned so that no two in a row are nearer than `spacing`, the
/// last one always kept.
pub fn thinned(points: &[Vec2], spacing: f32) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let last = index + 1 == points.len();
        match out.last() {
            Some(previous) if previous.distance(*point) < spacing && !last => {}
            _ => out.push(*point),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn solid(volume: &Volume, at: Vec3) -> bool {
        volume.sample_world(at) < 0.0
    }

    #[test]
    fn a_line_beside_the_axis_spins_into_a_cylinder() {
        let how = Revolve {
            origin: Vec3::ZERO,
            axis: Vec3::Y,
            profile: vec![Vec2::new(5.0, -5.0), Vec2::new(5.0, 5.0)],
        };
        let volume = revolve(0.25, &how).expect("a solid");
        assert!(solid(&volume, Vec3::new(4.7, 0.0, 0.0)));
        assert!(!solid(&volume, Vec3::new(5.3, 0.0, 0.0)));
        assert!(solid(&volume, Vec3::new(0.0, 0.0, -4.7)), "all the way round");
        assert!(solid(&volume, Vec3::new(0.0, 4.7, 0.0)), "the cap");
        assert!(!solid(&volume, Vec3::new(0.0, 5.3, 0.0)), "and no further");
        assert!(solid(&volume, Vec3::new(0.1, 0.0, 0.0)), "solid on the axis");
        assert_eq!(volume.sample_world(Vec3::ZERO), INSIDE, "deep inside is saturated");
    }

    #[test]
    fn either_side_of_the_axis_is_the_same_solid() {
        let right = Revolve {
            origin: Vec3::new(1.0, 2.0, 3.0),
            axis: Vec3::new(0.0, 1.0, 1.0),
            profile: vec![Vec2::new(2.0, -4.0), Vec2::new(4.0, 0.0), Vec2::new(1.0, 4.0)],
        };
        let mut left = right.clone();
        for point in &mut left.profile {
            point.x = -point.x;
        }
        let one = revolve(0.25, &right).expect("a solid");
        let other = revolve(0.25, &left).expect("a solid");
        for coord in one.brick_coords() {
            let corner = coord.origin();
            assert_eq!(one.sample_voxel(corner), other.sample_voxel(corner));
        }
    }

    #[test]
    fn a_half_circle_spins_into_a_ball() {
        let profile: Vec<Vec2> = (0..=64)
            .map(|step| {
                let angle = std::f32::consts::PI * (step as f32 / 64.0 - 0.5);
                Vec2::new(6.0 * angle.cos(), 6.0 * angle.sin())
            })
            .collect();
        let how = Revolve { origin: Vec3::ZERO, axis: Vec3::Y, profile };
        let volume = revolve(0.25, &how).expect("a solid");
        for direction in [Vec3::X, Vec3::Y, -Vec3::Y, Vec3::new(1.0, 1.0, 1.0).normalize()] {
            assert!(solid(&volume, direction * 5.7), "inside along {direction}");
            assert!(!solid(&volume, direction * 6.3), "outside along {direction}");
        }
    }

    #[test]
    fn too_little_is_refused() {
        let point = Revolve { origin: Vec3::ZERO, axis: Vec3::Y, profile: vec![Vec2::ONE] };
        assert_eq!(revolve(0.25, &point).err(), Some(RevolveError::TooShort));
        let on_axis = Revolve { profile: vec![Vec2::new(0.1, 0.0), Vec2::new(-0.1, 5.0)], ..point };
        assert_eq!(revolve(0.25, &on_axis).err(), Some(RevolveError::TooThin));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn the_outline_distance_is_zero_on_the_profile(
            points in prop::collection::vec((-8.0f32..8.0, -8.0f32..8.0), 2..12),
            pick in 0usize..12,
            along in 0.0f32..1.0,
        ) {
            let profile: Vec<Vec2> = points.iter().map(|(x, y)| Vec2::new(*x, *y)).collect();
            let polygon = outline(&profile);
            let i = pick % (profile.len() - 1);
            let a = Vec2::new(profile[i].x.abs(), profile[i].y);
            let b = Vec2::new(profile[i + 1].x.abs(), profile[i + 1].y);
            let on = a + (b - a) * along;
            prop_assert!(polygon_distance(&polygon, on).abs() < 1e-3);
        }

        #[test]
        fn thinning_keeps_both_ends_and_the_spacing(
            points in prop::collection::vec((-8.0f32..8.0, -8.0f32..8.0), 2..40),
            spacing in 0.1f32..3.0,
        ) {
            let points: Vec<Vec2> = points.iter().map(|(x, y)| Vec2::new(*x, *y)).collect();
            let thin = thinned(&points, spacing);
            prop_assert_eq!(thin[0], points[0]);
            prop_assert_eq!(*thin.last().unwrap(), *points.last().unwrap());
            for pair in thin[..thin.len() - 1].windows(2) {
                prop_assert!(pair[0].distance(pair[1]) >= spacing);
            }
        }
    }
}
