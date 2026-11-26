// SPDX-License-Identifier: AGPL-3.0-only

//! Decals and stamps: a picture dabbed onto the model along the view, as a
//! waterslide decal or a rubber stamp is on a real one. Stage 5 of
//! `docs/PAINT-RACK.md`, section 6.
//!
//! A dab is a picture laid on a plane square to the view, through the point
//! aimed at, sized and turned, and carried along the view onto the surface
//! it meets: it wraps that surface, and stops where the surface turns away
//! or lies further back than the picture is wide.
//!
//! - A **decal** keeps its own colours, by the picture's own alpha.
//! - A **stamp** is a stencil in the paint in hand: dark is paint, light is
//!   none, and grey is part of it.
//!
//! A dab is laid over the paint as it was before the stroke, and a voxel it
//! has stopped reaching is given that paint back: so the application can
//! dab again, larger or turned, at every move of the pointer while the
//! button is down, and what is left when it comes up is one dab. See
//! `app/decal.rs`.

use glam::{IVec3, Vec2, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashSet;

use super::field::{PaintField, PaintVisit, bricks_between};
use super::stroke::{Mode, Pigment, laid};
use crate::brick::{BRICK_VOXELS, Brick, BrickCoord, brick_index};
use crate::mask::PROTECTED;
use crate::volume::Volume;

/// A picture: sRGB colour and straight alpha, row by row from the top.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<[u8; 4]>,
}

impl Picture {
    /// A picture whose pixel at (u, v), each 0..1 across and down from the
    /// top left, is `f(u, v)`.
    pub fn from_fn(width: usize, height: usize, f: impl Fn(f32, f32) -> [u8; 4]) -> Self {
        let mut rgba = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let u = (x as f32 + 0.5) / width as f32;
                let v = (y as f32 + 0.5) / height as f32;
                rgba.push(f(u, v));
            }
        }
        Self { width, height, rgba }
    }

    /// The picture at (u, v), blended from the four pixels round it, as
    /// premultiplied linear-ish floats: colour times alpha, and alpha.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 4];
        }
        let x = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let pixel = |x: usize, y: usize| {
            let [r, g, b, a] = self.rgba[y * self.width + x].map(f32::from);
            let a = a / 255.0;
            [r * a, g * a, b * a, a]
        };
        let (p00, p10, p01, p11) = (pixel(x0, y0), pixel(x1, y0), pixel(x0, y1), pixel(x1, y1));
        std::array::from_fn(|c| {
            let top = p00[c] + (p10[c] - p00[c]) * fx;
            let bottom = p01[c] + (p11[c] - p01[c]) * fx;
            top + (bottom - top) * fy
        })
    }

    /// How tall it is for each unit wide.
    fn aspect(&self) -> f32 {
        if self.width == 0 { 1.0 } else { self.height as f32 / self.width as f32 }
    }
}

/// Where a dab goes and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dab {
    /// The point of the surface aimed at.
    pub at: Vec3,
    /// The way the view looks, toward the surface.
    pub along: Vec3,
    /// The screen's right and up, square to `along`.
    pub right: Vec3,
    pub up: Vec3,
    /// How wide the picture is, world units.
    pub size: f32,
    /// How far it is turned, radians, anticlockwise on the screen.
    pub turn: f32,
    /// How much of its size its edge fades over, 0 for a hard edge, up to
    /// a half.
    pub soft: f32,
    /// `Some` for a stamp, in this paint; `None` for a decal, in its own
    /// colours.
    pub stamp: Option<Pigment>,
}

impl Dab {
    /// Where on the picture a point of the world is, (u, v), or `None` off
    /// it.
    fn uv(&self, picture: &Picture, offset: Vec3) -> Option<Vec2> {
        let flat = Vec2::new(offset.dot(self.right), offset.dot(self.up));
        let (sin, cos) = (-self.turn).sin_cos();
        let turned = Vec2::new(flat.x * cos - flat.y * sin, flat.x * sin + flat.y * cos);
        let height = self.size * picture.aspect();
        let u = turned.x / self.size + 0.5;
        let v = 0.5 - turned.y / height;
        ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some(Vec2::new(u, v))
    }

    /// How much of the picture an edge leaves, 0..=1.
    fn edge(&self, uv: Vec2) -> f32 {
        if self.soft <= 0.0 {
            return 1.0;
        }
        let nearest = uv.x.min(1.0 - uv.x).min(uv.y).min(1.0 - uv.y);
        (nearest / self.soft.min(0.5)).clamp(0.0, 1.0)
    }

    /// The colour a point of the picture at `uv` lays, and how much of it,
    /// where the surface faces the view `facing` squarely (0..=1).
    fn take(&self, picture: &Picture, uv: Vec2, facing: f32) -> ([u8; 3], f32) {
        let [r, g, b, a] = picture.sample(uv.x, uv.y);
        let edge = self.edge(uv) * facing;
        match self.stamp {
            // Dark is paint: the picture's darkness, where it is not
            // see-through.
            Some(pigment) => {
                let light = if a > 0.0 {
                    (0.2126 * r + 0.7152 * g + 0.0722 * b) / (255.0 * a)
                } else {
                    1.0
                };
                (pigment.colour, (1.0 - light).clamp(0.0, 1.0) * a * edge)
            }
            None => {
                let colour = if a > 0.0 {
                    [r / a, g / a, b / a].map(|c| c.round().clamp(0.0, 255.0) as u8)
                } else {
                    [0, 0, 0]
                };
                (colour, a * edge)
            }
        }
    }

    /// The colour a point of the picture at `uv` shows and how much of it,
    /// face on: what a preview of the dab draws there. See `app/decal.rs`.
    pub fn shows(&self, picture: &Picture, uv: Vec2) -> ([u8; 3], f32) {
        self.take(picture, uv, 1.0)
    }

    /// Where on the picture a point `offset` from the point aimed at is,
    /// or `None` off it: what a preview of the dab is laid out by.
    pub fn picture_at(&self, picture: &Picture, offset: Vec3) -> Option<Vec2> {
        self.uv(picture, offset)
    }

    /// The point `uv` of the picture on the plane through the point aimed
    /// at, square to the view: the inverse of [`Self::picture_at`] on it.
    pub fn on_plane(&self, picture: &Picture, uv: Vec2) -> Vec3 {
        let height = self.size * picture.aspect();
        let turned = Vec2::new((uv.x - 0.5) * self.size, (0.5 - uv.y) * height);
        let (sin, cos) = self.turn.sin_cos();
        let flat = Vec2::new(turned.x * cos - turned.y * sin, turned.x * sin + turned.y * cos);
        self.at + self.right * flat.x + self.up * flat.y
    }

    /// The box of voxels it can reach.
    pub fn reach(&self, picture: &Picture, voxel_size: f32) -> (IVec3, IVec3) {
        let half = self.size * 0.5 * (1.0 + picture.aspect()).max(1.0);
        let lo = ((self.at - Vec3::splat(half)) / voxel_size).floor().as_ivec3();
        let hi = ((self.at + Vec3::splat(half)) / voxel_size).ceil().as_ivec3();
        (lo, hi)
    }
}

/// What each voxel of a brick takes of a dab: a colour, and how much.
type BrickTakes = Vec<([u8; 3], f32)>;

impl Volume {
    /// Dab `picture` as `dab` says, over the paint as it was before the
    /// stroke. Every voxel of `also` it does not reach is given that paint
    /// back. Returns the box it reached, for the next dab of the same
    /// gesture to pass as `also`.
    pub fn dab(
        &mut self,
        picture: &Picture,
        dab: &Dab,
        also: Option<(IVec3, IVec3)>,
    ) -> (IVec3, IVec3) {
        let voxel_size = self.voxel_size();
        let along = dab.along.normalize_or_zero();
        let (lo, hi) = dab.reach(picture, voxel_size);

        // What each voxel the picture reaches takes, worked out first, a
        // brick at a time and the bricks across every core: the facing
        // reads the field, which the edit below holds. A brick that is empty
        // space, or solid or empty throughout, holds no paint and is passed
        // over whole. The first version asked every voxel of the box round
        // the picture, solid and empty alike, with the trilinear gradient:
        // 50 ms for one move of a decal 21 mm across, measured by
        // `what_a_decal_costs` in the application.
        let coords: Vec<BrickCoord> =
            bricks_between(BrickCoord::containing(lo).0, BrickCoord::containing(hi).0).collect();
        let takes: Vec<(BrickCoord, BrickTakes)> = if along == Vec3::ZERO || dab.size <= 0.0 {
            Vec::new()
        } else {
            let this = &*self;
            coords
                .into_par_iter()
                .filter_map(|coord| {
                    let values = match this.brick(coord)? {
                        Brick::Uniform(value) if !PaintField::paintable(*value) => return None,
                        brick => brick,
                    };
                    let origin = coord.origin();
                    let (from, to) = (lo.max(origin), hi.min(coord.max_voxel()));
                    if from.cmpgt(to).any() {
                        return None;
                    }
                    let mut take = vec![([0_u8; 3], 0.0_f32); BRICK_VOXELS];
                    let mut any = false;
                    for z in from.z..=to.z {
                        for y in from.y..=to.y {
                            for x in from.x..=to.x {
                                let voxel = IVec3::new(x, y, z);
                                let local = voxel - origin;
                                let index = brick_index(
                                    local.x as usize,
                                    local.y as usize,
                                    local.z as usize,
                                );
                                let distance = match values {
                                    Brick::Uniform(value) => *value,
                                    Brick::Dense(data) => data[index],
                                };
                                if !PaintField::paintable(distance) {
                                    continue;
                                }
                                let offset = voxel.as_vec3() * voxel_size - dab.at;
                                if offset.dot(along).abs() > dab.size * 0.5 {
                                    continue;
                                }
                                let Some(uv) = dab.uv(picture, offset) else {
                                    continue;
                                };
                                // The normal by central differences of the
                                // stored field: six lookups, where the
                                // trilinear gradient is forty-eight.
                                let sample = |step: IVec3| this.sample_voxel(voxel + step);
                                let normal = Vec3::new(
                                    sample(IVec3::X) - sample(-IVec3::X),
                                    sample(IVec3::Y) - sample(-IVec3::Y),
                                    sample(IVec3::Z) - sample(-IVec3::Z),
                                )
                                .normalize_or_zero();
                                let facing = ((-normal.dot(along) - 0.05) / 0.25).clamp(0.0, 1.0);
                                if facing <= 0.0 {
                                    continue;
                                }
                                let laid_here = dab.take(picture, uv, facing);
                                if laid_here.1 > 1e-3 {
                                    take[index] = laid_here;
                                    any = true;
                                }
                            }
                        }
                    }
                    any.then_some((coord, take))
                })
                .collect()
        };
        let opacity = dab.stamp.map_or(1.0, |pigment| pigment.opacity);
        let mode = dab.stamp.map_or(Mode::Paint, |pigment| pigment.mode);
        // Laid a brick at a time across every core, as a stroke is
        // (`Volume::edit_paint_across_cores`).
        let done: FxHashSet<BrickCoord> = takes.iter().map(|(coord, _)| *coord).collect();
        let mut visits: Vec<PaintVisit<BrickTakes>> = takes
            .into_iter()
            .map(|(coord, take)| {
                let origin = coord.origin();
                PaintVisit { coord, lo: lo.max(origin), hi: hi.min(coord.max_voxel()), state: take }
            })
            .collect();
        self.edit_paint_across_cores(&mut visits, |take, _, index, _, _, before, protection| {
            let (colour, take) = take[index];
            if take <= 0.0 {
                return before;
            }
            let free = 1.0 - f32::from(protection) / f32::from(PROTECTED);
            laid(before, Pigment { colour, opacity, mode }, (take * free).clamp(0.0, 1.0))
        });
        // What the last dab of the gesture reached and this one does not is
        // given its paint back, as is the rest of a brick this one is in.
        if let Some((was_lo, was_hi)) = also {
            let (lo_all, hi_all) = (lo.min(was_lo), hi.max(was_hi));
            for coord in
                bricks_between(BrickCoord::containing(lo_all).0, BrickCoord::containing(hi_all).0)
            {
                if done.contains(&coord) {
                    continue;
                }
                let origin = coord.origin();
                let (from, to) = (lo_all.max(origin), hi_all.min(coord.max_voxel()));
                if from.cmpgt(to).any() {
                    continue;
                }
                self.edit_paint_over(from, to, |_, _, _, before, _| before);
            }
        }
        (lo, hi)
    }
}

/// The pictures that come with the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltInPicture {
    /// A white star outlined in black: a decal.
    Star,
    /// An eye, white, an iris and a pupil: a decal.
    Eye,
    /// Three rings, blue, white and red: a decal.
    Roundel,
    /// Diagonal stripes: a stamp.
    Hazard,
    /// Blotches: a stamp.
    Camo,
    /// A grid of dots: a stamp.
    Dots,
}

impl BuiltInPicture {
    pub const ALL: [BuiltInPicture; 6] = [
        BuiltInPicture::Star,
        BuiltInPicture::Eye,
        BuiltInPicture::Roundel,
        BuiltInPicture::Hazard,
        BuiltInPicture::Camo,
        BuiltInPicture::Dots,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BuiltInPicture::Star => "Star",
            BuiltInPicture::Eye => "Eye",
            BuiltInPicture::Roundel => "Roundel",
            BuiltInPicture::Hazard => "Hazard",
            BuiltInPicture::Camo => "Camo",
            BuiltInPicture::Dots => "Dots",
        }
    }

    /// Whether it is made to be a stamp rather than a decal.
    pub fn is_stamp(self) -> bool {
        matches!(self, BuiltInPicture::Hazard | BuiltInPicture::Camo | BuiltInPicture::Dots)
    }

    pub fn picture(self) -> Picture {
        const SIZE: usize = 128;
        const CLEAR: [u8; 4] = [0, 0, 0, 0];
        let centred = |u: f32, v: f32| Vec2::new(u - 0.5, v - 0.5) * 2.0;
        match self {
            BuiltInPicture::Star => Picture::from_fn(SIZE, SIZE, |u, v| {
                let p = centred(u, v);
                let angle = p.y.atan2(p.x) + std::f32::consts::FRAC_PI_2;
                // `rem_euclid` and not `fract`, which keeps the sign of a
                // negative angle and lost one of the five points: seen on a
                // screen.
                let point = (angle * 5.0 / std::f32::consts::TAU).rem_euclid(1.0);
                let spike = 1.0 - (point - 0.5).abs() * 2.0;
                let outer = 0.45 + 0.5 * spike.powf(2.2);
                let r = p.length();
                if r < outer - 0.08 {
                    [245, 245, 240, 255]
                } else if r < outer {
                    [15, 15, 18, 255]
                } else {
                    CLEAR
                }
            }),
            BuiltInPicture::Eye => Picture::from_fn(SIZE, SIZE / 2, |u, v| {
                let p = Vec2::new(u - 0.5, (v - 0.5) * 0.5) * 2.0;
                let lid = (p.x * p.x) + (p.y * 4.0).powi(2);
                let r = p.length();
                if lid > 1.0 {
                    CLEAR
                } else if r < 0.12 {
                    [8, 8, 10, 255]
                } else if r < 0.3 {
                    [60, 120, 50, 255]
                } else if lid > 0.85 {
                    [20, 12, 10, 255]
                } else {
                    [240, 235, 225, 255]
                }
            }),
            BuiltInPicture::Roundel => Picture::from_fn(SIZE, SIZE, |u, v| {
                let r = centred(u, v).length();
                if r > 1.0 {
                    CLEAR
                } else if r > 0.66 {
                    [25, 45, 110, 255]
                } else if r > 0.36 {
                    [240, 240, 235, 255]
                } else {
                    [190, 25, 30, 255]
                }
            }),
            BuiltInPicture::Hazard => Picture::from_fn(SIZE, SIZE, |u, v| {
                let stripe = ((u + v) * 4.0).fract() < 0.5;
                if stripe { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
            }),
            BuiltInPicture::Camo => Picture::from_fn(SIZE, SIZE, |u, v| {
                let wave = (u * 9.0 + (v * 5.0).sin() * 1.3).sin()
                    + (v * 7.0 + (u * 6.0).cos() * 1.7).sin()
                    + ((u + v) * 13.0).sin() * 0.35;
                if wave > 0.4 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
            }),
            BuiltInPicture::Dots => Picture::from_fn(SIZE, SIZE, |u, v| {
                let cell = Vec2::new((u * 6.0).fract(), (v * 6.0).fract()) - Vec2::splat(0.5);
                if cell.length() < 0.28 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::field::BARE;
    use proptest::prelude::*;

    fn ball() -> Volume {
        let mut volume = Volume::new(0.5);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        volume
    }

    /// A dab from +z at the front of the ball.
    fn front(size: f32, stamp: Option<Pigment>) -> Dab {
        Dab {
            at: Vec3::new(0.0, 0.0, 10.0),
            along: -Vec3::Z,
            right: Vec3::X,
            up: Vec3::Y,
            size,
            turn: 0.0,
            soft: 0.0,
            stamp,
        }
    }

    const MIDDLE: IVec3 = IVec3::new(0, 0, 20);

    fn red_square() -> Picture {
        Picture::from_fn(8, 8, |_, _| [200, 20, 20, 255])
    }

    /// A decal lays its own colour where it lands and nowhere behind.
    #[test]
    fn a_decal_lays_its_colours_on_the_surface_it_meets() {
        let mut volume = ball();
        volume.begin_stroke();
        volume.dab(&red_square(), &front(6.0, None), None);
        assert_eq!(volume.paint().at(MIDDLE), [200, 20, 20, 255]);
        assert_eq!(volume.paint().at(-MIDDLE), BARE, "the back of the ball");
        assert_eq!(volume.paint().at(IVec3::new(0, 14, 13)), BARE, "outside the square");
    }

    /// A stamp lays the paint in hand where the picture is dark, and none
    /// where it is light.
    #[test]
    fn a_stamp_is_a_stencil_in_the_paint_in_hand() {
        let half =
            Picture::from_fn(
                16,
                16,
                |u, _| {
                    if u < 0.5 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
                },
            );
        let blue = Pigment { colour: [20, 40, 220], opacity: 1.0, mode: Mode::Paint };
        let mut volume = ball();
        volume.begin_stroke();
        volume.dab(&half, &front(8.0, Some(blue)), None);
        assert_eq!(volume.paint().at(IVec3::new(-4, 0, 19)), [20, 40, 220, 255]);
        assert_eq!(volume.paint().at(IVec3::new(4, 0, 19)), BARE);
    }

    /// Dabbed again smaller within one stroke, the first dab's reach that
    /// the second does not reach is given its paint back.
    #[test]
    fn a_smaller_dab_gives_back_what_the_larger_took() {
        let mut volume = ball();
        let rim = IVec3::new(0, 6, 18);
        volume.begin_stroke();
        let reached = volume.dab(&red_square(), &front(10.0, None), None);
        assert_ne!(volume.paint().at(rim), BARE);
        volume.dab(&red_square(), &front(4.0, None), Some(reached));
        assert_eq!(volume.paint().at(rim), BARE, "the larger dab's rim stayed");
        assert_ne!(volume.paint().at(MIDDLE), BARE);
        let edit = volume.end_stroke().expect("a stroke");
        volume.apply_edit(edit);
        assert!(volume.paint().is_empty(), "the dab was not one undo");
    }

    /// The mask keeps a dab off.
    #[test]
    fn the_mask_keeps_a_dab_off() {
        let mut volume = ball();
        let (lo, hi) = (IVec3::splat(-30), IVec3::splat(30));
        volume.edit_mask(lo, hi, |_, at, _| if at.x > 0.0 { PROTECTED } else { 0 });
        volume.begin_stroke();
        volume.dab(&red_square(), &front(8.0, None), None);
        assert_eq!(volume.paint().at(IVec3::new(4, 0, 19)), BARE);
        assert_ne!(volume.paint().at(IVec3::new(-4, 0, 19)), BARE);
    }

    /// The star has five points: five runs of its outline round a circle
    /// through them.
    #[test]
    fn the_star_has_five_points() {
        let star = BuiltInPicture::Star.picture();
        let inked = |angle: f32| {
            let (u, v) = (0.5 + 0.4 * angle.cos(), 0.5 + 0.4 * angle.sin());
            let (x, y) = ((u * star.width as f32) as usize, (v * star.height as f32) as usize);
            star.rgba[y * star.width + x][3] > 0
        };
        let steps = 720;
        let runs = (0..steps)
            .filter(|step| {
                let angle = |s: usize| s as f32 / steps as f32 * std::f32::consts::TAU;
                inked(angle(*step)) && !inked(angle((*step + steps - 1) % steps))
            })
            .count();
        assert_eq!(runs, 5);
    }

    /// Every picture that comes with the application has something in it.
    #[test]
    fn every_built_in_picture_has_something_in_it() {
        for which in BuiltInPicture::ALL {
            let picture = which.picture();
            assert!(picture.rgba.iter().any(|pixel| pixel[3] > 0), "{which:?}");
            if which.is_stamp() {
                assert!(picture.rgba.iter().any(|pixel| pixel[0] < 50), "{which:?} has no dark");
                assert!(picture.rgba.iter().any(|pixel| pixel[0] > 200), "{which:?} has no light");
            }
        }
    }

    proptest! {
        /// A soft edge never lays more than a hard one, and the middle of a
        /// picture is never faded by its edge.
        #[test]
        fn a_soft_edge_only_fades(u in 0.0f32..1.0, v in 0.0f32..1.0, soft in 0.0f32..0.5) {
            let hard = front(4.0, None);
            let softer = Dab { soft, ..hard };
            let uv = Vec2::new(u, v);
            prop_assert!(softer.edge(uv) <= hard.edge(uv));
            prop_assert!((softer.edge(Vec2::splat(0.5)) - 1.0).abs() < 1e-6 || soft > 0.5);
        }

        /// Turning a dab a full turn lands every point where it was.
        #[test]
        fn a_point_of_the_picture_on_the_plane_is_that_point(
            u in 0.01f32..0.99,
            v in 0.01f32..0.99,
            turn in -3.0f32..3.0,
            size in 0.5f32..20.0,
        ) {
            let picture = Picture::from_fn(8, 4, |_, _| [255, 0, 0, 255]);
            let dab = Dab { turn, size, ..front(4.0, None) };
            let point = dab.on_plane(&picture, Vec2::new(u, v));
            let back = dab.picture_at(&picture, point - dab.at).expect("on the picture");
            prop_assert!((back - Vec2::new(u, v)).length() < 1e-3, "{:?} came back {:?}", (u, v), back);
        }

        #[test]
        fn a_full_turn_is_no_turn(x in -2.0f32..2.0, y in -2.0f32..2.0, turn in -3.0f32..3.0) {
            let picture = red_square();
            let one = Dab { turn, ..front(4.0, None) };
            let round = Dab { turn: turn + std::f32::consts::TAU, ..one };
            let offset = Vec3::new(x, y, 0.0);
            match (one.uv(&picture, offset), round.uv(&picture, offset)) {
                (Some(a), Some(b)) => prop_assert!((a - b).length() < 1e-3),
                (None, None) => {}
                (a, b) => prop_assert!(
                    a.or(b).is_some_and(|uv| uv.x.min(1.0 - uv.x).min(uv.y).min(1.0 - uv.y) < 1e-3),
                    "one is on the picture and the other not, away from its edge"
                ),
            }
        }
    }
}
