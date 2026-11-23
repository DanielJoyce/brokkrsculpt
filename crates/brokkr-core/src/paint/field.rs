// SPDX-License-Identifier: AGPL-3.0-only

//! Paint: a colour and a coverage for every voxel of the surface.
//!
//! This is what the painting engine paints on. Four bytes a voxel, sRGB red,
//! green and blue and how much of the voxel the paint covers, held per body
//! in a sparse map of bricks beside the distance field, the mask and the
//! filament slots.
//!
//! # Why a colour, where upstream chose a slot
//!
//! [`crate::colour`] holds one byte a voxel and says why: it names a
//! filament, a slicer wants a filament and not a colour, and a colour would
//! cost four times as much. All true, and all of it is the reasoning of a
//! tool that paints in order to print in four plastics.
//!
//! A painter needs any colour, and needs one colour to go over another
//! partly: a glaze that tints what is under it, a drybrush that catches
//! only the raised grain, an edge that fades out and does not stop. A slot
//! can be none of those. The first painting mode on this branch was built
//! on slots, with a recipe for each (`paint.rs`), and it showed the limit
//! every time it was looked at: 63 paints, each all or nothing, with a
//! stepped edge the size of a voxel.
//!
//! So this is the second field. The slots stay as they are, for a model
//! that came in with them and for whoever prints in filaments; nothing here
//! reads or writes one.
//!
//! # What a voxel holds
//!
//! `[red, green, blue, coverage]`. The colour is sRGB, as a picture's bytes
//! are. The coverage is how much of the surface there the paint hides:
//! 0 is bare, and is always stored as four zeros ([`BARE`]) so that a brick
//! nobody painted, or that was painted and wiped, collapses to nothing.
//! 255 is paint with nothing showing through. Between them the body's own
//! surface shows through in proportion, which is what a soft edge is.
//!
//! Not premultiplied: the colour is the paint's own colour whatever its
//! coverage, so a stroke that only thickens a coat does not have to touch
//! the colour, and a colour is not lost to rounding where the coat is thin.
//!
//! # The cost
//!
//! A dense brick is 131,072 bytes, the same as the distance brick it lies
//! beside, where a slot brick is a quarter of that. So a painted body is
//! about twice a bare one in memory and in what a stroke costs the undo
//! history. That is the price upstream declined, and it is paid only where
//! there is paint.
//!
//! # The same shape as the slot field, deliberately
//!
//! Paint lives in the narrow band and only the band is ever read, a brick
//! of one value costs no heap, absent means bare, and every operation that
//! moves a body (turn, shift, warp, resample, duplicate, merge, split, the
//! Move brush) carries its paint the way it carries its slots, by the same
//! code shape, with a test for each. The reasons are in
//! [`crate::colour`]'s header and are not repeated here.
//!
//! Sampling under a warp or a resample is nearest, as it is for slots.
//! For a colour, interpolating would be better, and is owed.

use contracts::{debug_ensures, debug_requires};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::brick::{BRICK_DIM, BRICK_VOXELS, Brick, BrickCoord, brick_index};
use crate::colour::gather_pad;
use crate::mask::MaskField;
use crate::orientation::AxisRotation;
use crate::similarity::Similarity;
use crate::volume::{Volume, VolumeStats};
use glam::{IVec3, Vec3};

/// One voxel's paint: sRGB red, green and blue, and coverage.
pub type Rgba = [u8; 4];

/// What an unpainted voxel holds. Any voxel with no coverage is stored as
/// this, whatever colour it was last given.
pub const BARE: Rgba = [0, 0, 0, 0];

/// `rgba` as it is stored: a voxel with no coverage has no colour either.
#[inline]
#[debug_ensures(ret[3] != 0 || ret == BARE, "a voxel with no coverage has no colour")]
pub fn stored(rgba: Rgba) -> Rgba {
    if rgba[3] == 0 { BARE } else { rgba }
}

/// The paint of one brick. Mirrors [`crate::colour::ColourBrick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaintBrick {
    Uniform(Rgba),
    Dense(Box<[Rgba; BRICK_VOXELS]>),
}

impl PaintBrick {
    pub(crate) fn dense_filled(rgba: Rgba) -> Self {
        let data: Box<[Rgba]> = vec![rgba; BRICK_VOXELS].into_boxed_slice();
        let data: Box<[Rgba; BRICK_VOXELS]> = data.try_into().expect("length is BRICK_VOXELS");
        PaintBrick::Dense(data)
    }

    pub(crate) fn make_dense(&mut self) -> &mut [Rgba; BRICK_VOXELS] {
        if let PaintBrick::Uniform(rgba) = *self {
            *self = PaintBrick::dense_filled(rgba);
        }
        match self {
            PaintBrick::Dense(data) => data,
            PaintBrick::Uniform(_) => unreachable!("just promoted to dense"),
        }
    }

    #[inline]
    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            PaintBrick::Uniform(_) => 0,
            PaintBrick::Dense(_) => BRICK_VOXELS * size_of::<Rgba>(),
        }
    }

    pub(crate) fn is_collapsible(&self) -> Option<Rgba> {
        match self {
            PaintBrick::Uniform(rgba) => Some(*rgba),
            PaintBrick::Dense(data) => {
                let first = data[0];
                data.iter().all(|rgba| *rgba == first).then_some(first)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PaintSlab<'a> {
    Bare,
    Uniform(Rgba),
    Dense(&'a [Rgba; BRICK_VOXELS]),
}

impl PaintSlab<'_> {
    #[inline]
    pub(crate) fn rgba_at(&self, index: usize) -> Rgba {
        match self {
            PaintSlab::Bare => BARE,
            PaintSlab::Uniform(rgba) => *rgba,
            PaintSlab::Dense(data) => data[index],
        }
    }
}

#[derive(Debug)]
pub(crate) enum PaintChange {
    Changed(Option<PaintBrick>),
    Unchanged,
}

#[derive(Debug, Default, Clone)]
pub struct PaintField {
    bricks: FxHashMap<BrickCoord, PaintBrick>,
    revision: u64,
}

impl PaintField {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bricks.is_empty()
    }

    #[inline]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    #[inline]
    pub fn paintable(distance: f32) -> bool {
        distance > crate::brick::INSIDE && distance < crate::brick::OUTSIDE
    }

    pub fn at(&self, cell: IVec3) -> Rgba {
        let coord = BrickCoord::containing(cell);
        let Some(brick) = self.bricks.get(&coord) else { return BARE };
        let local = cell - coord.origin();
        match brick {
            PaintBrick::Uniform(rgba) => *rgba,
            PaintBrick::Dense(data) => {
                data[brick_index(local.x as usize, local.y as usize, local.z as usize)]
            }
        }
    }

    /// The paint at a point between voxels, `at` in voxels: its colour as
    /// linear light ALREADY MULTIPLIED by its coverage, and the coverage,
    /// each blended from the eight voxels round the point.
    ///
    /// Multiplied first for the reason the shader multiplies before it
    /// interpolates: bare is stored as black, and a blend of straight
    /// colours across the edge of a painted patch would darken the paint
    /// on its way out. This is what a file is written from, where a
    /// vertex or a texel is not at a voxel; the mesh on screen reads the
    /// voxel of each vertex's cell and lets the triangles do the blending.
    #[debug_ensures((0.0..=1.0).contains(&ret[3]), "a coverage")]
    #[debug_ensures(
        ret[..3].iter().all(|channel| (0.0..=ret[3] + 1e-5).contains(channel)),
        "a colour no brighter than its coverage"
    )]
    pub fn sample(&self, at: Vec3) -> [f32; 4] {
        if self.bricks.is_empty() {
            return [0.0; 4];
        }
        let low = at.floor();
        let share = at - low;
        let low = low.as_ivec3();
        let mut sum = [0.0_f32; 4];
        for corner in 0..8 {
            let step = IVec3::new(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let weight = [share.x, share.y, share.z]
                .into_iter()
                .zip(step.to_array())
                .map(|(share, step)| if step == 1 { share } else { 1.0 - share })
                .product::<f32>();
            if weight <= 0.0 {
                continue;
            }
            let [r, g, b, a] = self.at(low + step);
            let coverage = f32::from(a) / 255.0 * weight;
            let colour = crate::paint::linear([r, g, b]);
            for channel in 0..3 {
                sum[channel] += colour[channel] * coverage;
            }
            sum[3] += coverage;
        }
        sum[3] = sum[3].min(1.0);
        sum
    }

    pub(crate) fn slab(&self, coord: BrickCoord) -> PaintSlab<'_> {
        if self.bricks.is_empty() {
            return PaintSlab::Bare;
        }
        match self.bricks.get(&coord) {
            None => PaintSlab::Uniform(BARE),
            Some(PaintBrick::Uniform(rgba)) => PaintSlab::Uniform(*rgba),
            Some(PaintBrick::Dense(data)) => PaintSlab::Dense(data),
        }
    }

    #[debug_requires(stored(rgba) == rgba, "paint is written as it is stored")]
    #[debug_ensures(self.at(cell) == rgba, "a voxel written reads back")]
    pub fn write(&mut self, cell: IVec3, rgba: Rgba) {
        let coord = BrickCoord::containing(cell);
        let local = cell - coord.origin();
        let index = brick_index(local.x as usize, local.y as usize, local.z as usize);
        self.touch();
        match self.bricks.get_mut(&coord) {
            Some(PaintBrick::Uniform(held)) if *held == rgba => {}
            Some(brick) => brick.make_dense()[index] = rgba,
            None if rgba == BARE => {}
            None => {
                let mut brick = PaintBrick::Uniform(BARE);
                brick.make_dense()[index] = rgba;
                self.bricks.insert(coord, brick);
            }
        }
    }

    #[debug_requires(
        lo.cmpge(coord.origin()).all() && hi.cmple(coord.max_voxel()).all(),
        "the box edited is inside the brick"
    )]
    pub(crate) fn edit_brick(
        &mut self,
        coord: BrickCoord,
        lo: IVec3,
        hi: IVec3,
        keep_prior: bool,
        paintable: &impl Fn(usize) -> bool,
        edit: &mut impl FnMut(IVec3, usize, Rgba) -> Rgba,
    ) -> PaintChange {
        let origin = coord.origin();
        let existed = self.bricks.contains_key(&coord);
        // A copy of the brick as it was, for the undo record, and only for
        // a caller that has somewhere to put it: the copy is 131 KB, and a
        // stroke asks for it once a brick and not once a stamp.
        let prior = if keep_prior { self.bricks.get(&coord).cloned() } else { None };
        let brick = self.bricks.entry(coord).or_insert(PaintBrick::Uniform(BARE));
        let was_uniform = matches!(brick, PaintBrick::Uniform(_));
        let uniform_rgba = match brick {
            PaintBrick::Uniform(rgba) => *rgba,
            PaintBrick::Dense(_) => BARE,
        };

        let data = brick.make_dense();
        let mut changed = false;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let voxel = IVec3::new(x, y, z);
                    let local = voxel - origin;
                    let index = brick_index(local.x as usize, local.y as usize, local.z as usize);
                    if !paintable(index) {
                        continue;
                    }
                    let rgba = edit(voxel, index, data[index]);
                    if rgba != data[index] {
                        data[index] = rgba;
                        changed = true;
                    }
                }
            }
        }

        if changed {
            self.touch();
            return PaintChange::Changed(prior);
        }
        if was_uniform {
            if existed {
                self.bricks.insert(coord, PaintBrick::Uniform(uniform_rgba));
            } else {
                self.bricks.remove(&coord);
            }
        }
        PaintChange::Unchanged
    }

    pub(crate) fn brick(&self, coord: BrickCoord) -> Option<&PaintBrick> {
        self.bricks.get(&coord)
    }

    pub(crate) fn restore_brick(&mut self, coord: BrickCoord, brick: Option<PaintBrick>) {
        self.touch();
        match brick {
            Some(brick) => {
                self.bricks.insert(coord, brick);
            }
            None => {
                self.bricks.remove(&coord);
            }
        }
    }

    #[debug_ensures(out.len() == old(out.len()) + cells.len(), "one paint for each cell")]
    pub(crate) fn rgba_at_cells(&self, coord: BrickCoord, cells: &[IVec3], out: &mut Vec<Rgba>) {
        if self.bricks.is_empty() {
            out.resize(out.len() + cells.len(), BARE);
            return;
        }

        let slab = self.slab(coord);
        let origin = coord.origin();
        let dim = BRICK_DIM as i32;
        out.reserve(cells.len());
        for cell in cells {
            let local = *cell - origin;
            let rgba = if local.min_element() >= 0 && local.max_element() < dim {
                slab.rgba_at(brick_index(local.x as usize, local.y as usize, local.z as usize))
            } else {
                self.at(*cell)
            };
            out.push(rgba);
        }
    }

    #[debug_ensures(
        self.bricks.values().all(|brick| *brick != PaintBrick::Uniform(BARE)),
        "a brick of nothing is no brick"
    )]
    pub(crate) fn collapse(&mut self) {
        self.bricks.retain(|_, brick| match brick.is_collapsible() {
            Some(BARE) => false,
            Some(rgba) => {
                *brick = PaintBrick::Uniform(rgba);
                true
            }
            None => true,
        });
    }

    pub(crate) fn add_to_stats(&self, stats: &mut VolumeStats) {
        let bytes: usize = self.bricks.values().map(PaintBrick::heap_bytes).sum();
        stats.resident_bytes += bytes + self.map_bytes();
    }

    fn map_bytes(&self) -> usize {
        self.bricks.capacity()
            * (std::mem::size_of::<BrickCoord>() + std::mem::size_of::<PaintBrick>())
    }

    pub(crate) fn translated(&self, offset_bricks: IVec3) -> PaintField {
        let mut copy = PaintField { bricks: FxHashMap::default(), revision: self.revision + 1 };
        copy.bricks.reserve(self.bricks.len());
        for (coord, brick) in &self.bricks {
            copy.bricks.insert(BrickCoord(coord.0 + offset_bricks), brick.clone());
        }
        copy
    }

    fn successor(&self) -> PaintField {
        PaintField { bricks: FxHashMap::default(), revision: self.revision + 1 }
    }

    pub(crate) fn rotated(&self, rotation: AxisRotation) -> PaintField {
        let map = rotation.axis_map();
        let last = BRICK_DIM - 1;
        let mut turned = self.successor();
        turned.bricks.reserve(self.bricks.len());
        for (coord, brick) in &self.bricks {
            let destination = BrickCoord(rotation.apply_voxel(coord.0));
            let moved = match brick {
                PaintBrick::Uniform(rgba) => PaintBrick::Uniform(*rgba),
                PaintBrick::Dense(data) => {
                    let mut moved = PaintBrick::dense_filled(BARE);
                    let out = moved.make_dense();
                    for z in 0..BRICK_DIM {
                        for y in 0..BRICK_DIM {
                            for x in 0..BRICK_DIM {
                                let from = [x, y, z];
                                let mut to = [0usize; 3];
                                for (axis, (into, flipped)) in map.into_iter().enumerate() {
                                    to[into] = if flipped { last - from[axis] } else { from[axis] };
                                }
                                out[brick_index(to[0], to[1], to[2])] = data[brick_index(x, y, z)];
                            }
                        }
                    }
                    moved
                }
            };
            turned.bricks.insert(destination, moved);
        }
        turned
    }

    pub(crate) fn shifted(&self, offset_voxels: IVec3) -> PaintField {
        let dim = BRICK_DIM as i32;
        if offset_voxels.rem_euclid(IVec3::splat(dim)) == IVec3::ZERO {
            return self.translated(offset_voxels / dim);
        }
        let mut wanted: FxHashSet<BrickCoord> = FxHashSet::default();
        for coord in self.bricks.keys() {
            let low = BrickCoord::containing(coord.origin() + offset_voxels).0;
            let high = BrickCoord::containing(coord.max_voxel() + offset_voxels).0;
            wanted.extend(bricks_between(low, high));
        }
        self.gathered(
            wanted,
            |coord| Some((coord.origin() - offset_voxels, coord.max_voxel() - offset_voxels)),
            |cell| cell - offset_voxels,
        )
    }

    pub(crate) fn resampled(&self, from_voxel: f32, to_voxel: f32) -> PaintField {
        let ratio = from_voxel / to_voxel;
        let inverse = to_voxel / from_voxel;
        let pad = IVec3::splat(gather_pad(ratio));
        let mut wanted: FxHashSet<BrickCoord> = FxHashSet::default();
        for coord in self.bricks.keys() {
            let low = (coord.origin().as_vec3() * ratio).floor().as_ivec3() - pad;
            let high = (coord.max_voxel().as_vec3() * ratio).ceil().as_ivec3() + pad;
            wanted.extend(bricks_between(
                BrickCoord::containing(low).0,
                BrickCoord::containing(high).0,
            ));
        }
        let source = move |cell: IVec3| (cell.as_vec3() * inverse).round().as_ivec3();
        self.gathered(
            wanted,
            |coord| Some((source(coord.origin()), source(coord.max_voxel()))),
            source,
        )
    }

    pub(crate) fn warped(&self, by: Similarity, voxel_size: f32) -> PaintField {
        let pad = IVec3::splat(gather_pad(by.scale.max_element()));
        let mut wanted: FxHashSet<BrickCoord> = FxHashSet::default();
        for coord in self.bricks.keys() {
            let source_low = coord.origin().as_vec3() * voxel_size;
            let source_high = coord.max_voxel().as_vec3() * voxel_size;
            let (low, high) = crate::transform::forward_bounds(by, source_low, source_high);
            let low = BrickCoord::containing((low / voxel_size).floor().as_ivec3() - pad).0;
            let high = BrickCoord::containing((high / voxel_size).ceil().as_ivec3() + pad).0;
            wanted.extend(bricks_between(low, high));
        }
        self.gathered(
            wanted,
            |_| None,
            |cell| {
                (by.inverse_transform_point(cell.as_vec3() * voxel_size) / voxel_size)
                    .round()
                    .as_ivec3()
            },
        )
    }

    fn gathered(
        &self,
        wanted: FxHashSet<BrickCoord>,
        source_box: impl Fn(&BrickCoord) -> Option<(IVec3, IVec3)> + Sync,
        source_of: impl Fn(IVec3) -> IVec3 + Sync,
    ) -> PaintField {
        let mut out = self.successor();
        if self.bricks.is_empty() {
            return out;
        }
        let coords: Vec<BrickCoord> = wanted.into_iter().collect();
        let built: Vec<(BrickCoord, PaintBrick)> = coords
            .par_iter()
            .filter_map(|coord| {
                if let Some((low, high)) = source_box(coord)
                    && let Some(rgba) = self.uniform_over(low, high)
                {
                    return (rgba != BARE).then_some((*coord, PaintBrick::Uniform(rgba)));
                }
                let origin = coord.origin();
                let mut brick = PaintBrick::dense_filled(BARE);
                let data = brick.make_dense();
                let mut cached: Option<(BrickCoord, PaintSlab<'_>)> = None;
                for z in 0..BRICK_DIM {
                    for y in 0..BRICK_DIM {
                        for x in 0..BRICK_DIM {
                            let cell = source_of(origin + IVec3::new(x as i32, y as i32, z as i32));
                            let source = BrickCoord::containing(cell);
                            let slab = match cached {
                                Some((held, slab)) if held == source => slab,
                                _ => {
                                    let slab = self.slab(source);
                                    cached = Some((source, slab));
                                    slab
                                }
                            };
                            let local = cell - source.origin();
                            data[brick_index(x, y, z)] = slab.rgba_at(brick_index(
                                local.x as usize,
                                local.y as usize,
                                local.z as usize,
                            ));
                        }
                    }
                }
                match brick.is_collapsible() {
                    Some(BARE) => None,
                    Some(rgba) => Some((*coord, PaintBrick::Uniform(rgba))),
                    None => Some((*coord, brick)),
                }
            })
            .collect();
        out.bricks.reserve(built.len());
        for (coord, brick) in built {
            out.bricks.insert(coord, brick);
        }
        out
    }

    fn uniform_over(&self, low: IVec3, high: IVec3) -> Option<Rgba> {
        let mut found: Option<Rgba> = None;
        for coord in bricks_between(BrickCoord::containing(low).0, BrickCoord::containing(high).0) {
            let rgba = match self.bricks.get(&coord) {
                None => BARE,
                Some(PaintBrick::Uniform(rgba)) => *rgba,
                Some(PaintBrick::Dense(_)) => return None,
            };
            match found {
                None => found = Some(rgba),
                Some(held) if held == rgba => {}
                Some(_) => return None,
            }
        }
        found
    }

    pub(crate) fn union_from(
        &mut self,
        other: &PaintField,
        admits: impl Fn(BrickCoord, usize) -> bool,
        mut record: impl FnMut(BrickCoord, Option<PaintBrick>),
    ) -> Vec<BrickCoord> {
        let mut changed = Vec::new();
        for (coord, incoming) in &other.bricks {
            if matches!(incoming, PaintBrick::Uniform(BARE)) {
                continue;
            }
            let theirs = other.slab(*coord);
            let mut brick = self.bricks.get(coord).cloned().unwrap_or(PaintBrick::Uniform(BARE));
            let data = brick.make_dense();
            let mut moved = false;
            for (index, held) in data.iter_mut().enumerate() {
                let rgba = theirs.rgba_at(index);
                if rgba != BARE && *held != rgba && admits(*coord, index) {
                    *held = rgba;
                    moved = true;
                }
            }
            if !moved {
                continue;
            }
            record(*coord, self.bricks.get(coord).cloned());
            // Dense, whatever it now holds: the end of the stroke collapses.
            self.bricks.insert(*coord, brick);
            self.touch();
            changed.push(*coord);
        }
        changed
    }

    pub(crate) fn bricks(&self) -> &FxHashMap<BrickCoord, PaintBrick> {
        &self.bricks
    }

    pub(crate) fn insert(&mut self, coord: BrickCoord, brick: PaintBrick) {
        self.touch();
        self.bricks.insert(coord, brick);
    }
}

pub(crate) fn bricks_between(low: IVec3, high: IVec3) -> impl Iterator<Item = BrickCoord> {
    (low.z..=high.z).flat_map(move |z| {
        (low.y..=high.y).flat_map(move |y| (low.x..=high.x).map(move |x| BrickCoord::new(x, y, z)))
    })
}

/// What an edit of the paint needs of a volume, borrowed apart. See
/// `Volume::paint_parts`.
pub(crate) struct PaintParts<'a> {
    pub bricks: &'a FxHashMap<BrickCoord, Brick>,
    pub mask: &'a MaskField,
    pub paint: &'a mut PaintField,
    pub recorder: &'a mut Option<FxHashMap<BrickCoord, Option<PaintBrick>>>,
}

/// Where voxel `index` of a brick is inside it.
fn local_of(index: usize) -> IVec3 {
    IVec3::new(
        (index % BRICK_DIM) as i32,
        ((index / BRICK_DIM) % BRICK_DIM) as i32,
        (index / (BRICK_DIM * BRICK_DIM)) as i32,
    )
}

impl Volume {
    /// Edit the paint of every paintable voxel in the inclusive voxel box
    /// `lo..=hi`.
    ///
    /// `edit` is handed each voxel, its world position, the paint it holds
    /// and the RESOLVED protection at it, and returns the paint it should
    /// hold. What it returns with no coverage is stored as [`BARE`].
    ///
    /// [`Volume::edit_colour`]'s shape, for its reasons: only a voxel in the
    /// narrow band is offered, a brick whose field is saturated throughout
    /// is never opened, and the open stroke records each brick it changes
    /// before it changes.
    pub fn edit_paint(
        &mut self,
        lo: IVec3,
        hi: IVec3,
        edit: impl Fn(IVec3, Vec3, Rgba, u8) -> Rgba,
    ) {
        self.edit_paint_over(lo, hi, |voxel, position, held, _, protection| {
            edit(voxel, position, held, protection)
        });
    }

    /// [`Volume::edit_paint`] for an edit that works from the paint as it
    /// was when the open stroke began, and may keep count as it goes.
    ///
    /// `edit` is handed one thing more, after the paint the voxel holds:
    /// the paint it held **before the stroke**. That is the stroke's own
    /// undo record read back, so it costs nothing to keep. A brick the
    /// stroke has not changed yet holds it still, and with no stroke open
    /// it is simply what the voxel holds.
    ///
    /// It is what lets a stroke lay its paint once however many stamps
    /// cross a voxel: each stamp blends over what was there before any of
    /// them, and not over the last one's result. See
    /// [`crate::paint::stroke`].
    pub fn edit_paint_over(
        &mut self,
        lo: IVec3,
        hi: IVec3,
        mut edit: impl FnMut(IVec3, Vec3, Rgba, Rgba, u8) -> Rgba,
    ) {
        let voxel_size = self.voxel_size();
        let b_min = BrickCoord::containing(lo).0;
        let b_max = BrickCoord::containing(hi).0;
        let parts = self.paint_parts();
        let mask = parts.mask;
        for coord in bricks_between(b_min, b_max) {
            let origin = coord.origin();
            let from = lo.max(origin);
            let to = hi.min(coord.max_voxel());
            if from.cmpgt(to).any() {
                continue;
            }
            let protection = mask.slab(coord);
            // What this brick held before the stroke, where the stroke has
            // changed it already: `None` for a brick it has not.
            let recorded = parts.recorder.as_ref().and_then(|recorder| recorder.get(&coord));
            // Whether a stroke is open and has yet to record this brick.
            let unrecorded = parts.recorder.is_some() && recorded.is_none();
            let mut edit = |voxel: IVec3, index: usize, held: Rgba| {
                let position = voxel.as_vec3() * voxel_size;
                let before = match recorded {
                    None => held,
                    Some(None) => BARE,
                    Some(Some(PaintBrick::Uniform(rgba))) => *rgba,
                    Some(Some(PaintBrick::Dense(data))) => data[index],
                };
                let byte = protection.byte_at(index);
                stored(edit(voxel, position, held, before, mask.resolve(byte)))
            };
            let outcome = match parts.bricks.get(&coord) {
                None => continue,
                Some(Brick::Uniform(value)) => {
                    if !PaintField::paintable(*value) {
                        continue;
                    }
                    parts.paint.edit_brick(coord, from, to, unrecorded, &|_| true, &mut edit)
                }
                Some(Brick::Dense(values)) => parts.paint.edit_brick(
                    coord,
                    from,
                    to,
                    unrecorded,
                    &|index| PaintField::paintable(values[index]),
                    &mut edit,
                ),
            };
            let PaintChange::Changed(prior) = outcome else {
                continue;
            };
            if unrecorded && let Some(recorder) = parts.recorder.as_mut() {
                recorder.insert(coord, prior);
            }
        }
        self.mark_dirty_voxel_range(lo, hi);
    }

    /// Take the paint of `source` that lies on THIS body's surface: what a
    /// half of a split keeps of the body it was split from.
    /// [`Volume::adopt_paint`] for the slots, and the same rule.
    pub(crate) fn adopt_rgba(&mut self, source: &PaintField) {
        let mut paint = PaintField::default();
        let coords: Vec<BrickCoord> = self.brick_coords().collect();
        for coord in coords {
            let Some(theirs) = source.brick(coord) else {
                continue;
            };
            if let Some(Brick::Uniform(value)) = self.brick(coord)
                && !PaintField::paintable(*value)
            {
                continue;
            }
            let origin = coord.origin();
            let mut mine = theirs.clone();
            let data = mine.make_dense();
            for (index, rgba) in data.iter_mut().enumerate() {
                if *rgba != BARE && !self.has_material_at(origin + local_of(index)) {
                    *rgba = BARE;
                }
            }
            paint.insert(coord, mine);
        }
        paint.collapse();
        *self.paint_mut() = paint;
    }

    /// Take `other`'s paint wherever `other` has surface: what a merge does
    /// with the paint of the body merged in.
    /// [`Volume::union_colour_from`] for the slots, and the same rule.
    pub(crate) fn union_paint_from(&mut self, other: &Volume) {
        let parts = self.paint_parts();
        let recorder = parts.recorder;
        let changed = parts.paint.union_from(
            other.paint(),
            |coord, index| other.has_material_at(coord.origin() + local_of(index)),
            |coord, prior| {
                if let Some(recorder) = recorder.as_mut() {
                    recorder.entry(coord).or_insert(prior);
                }
            },
        );
        for coord in changed {
            self.paint_changed_in(coord);
        }
    }

    /// Copy the paint of the inclusive voxel box `lo..=hi` into `region`:
    /// what a gesture that works from the surface as it WAS reads from.
    #[debug_ensures(
        region.values.len() == region.size.element_product() as usize,
        "a paint for each voxel of the box"
    )]
    pub fn snapshot_paint(&self, lo: IVec3, hi: IVec3, region: &mut PaintRegion) {
        region.lo = lo;
        region.size = (hi - lo + IVec3::ONE).max(IVec3::ZERO);
        region.values.clear();
        region.values.resize(region.size.element_product().max(0) as usize, BARE);
        if self.paint().is_empty() {
            return;
        }
        let size = region.size;
        for coord in bricks_between(BrickCoord::containing(lo).0, BrickCoord::containing(hi).0) {
            let slab = self.paint().slab(coord);
            if matches!(slab, PaintSlab::Uniform(BARE) | PaintSlab::Bare) {
                continue;
            }
            let origin = coord.origin();
            let from = lo.max(origin);
            let to = hi.min(coord.max_voxel());
            if from.cmpgt(to).any() {
                continue;
            }
            for z in from.z..=to.z {
                for y in from.y..=to.y {
                    for x in from.x..=to.x {
                        let at = IVec3::new(x, y, z);
                        let local = at - origin;
                        let index = (x - lo.x) + (y - lo.y) * size.x + (z - lo.z) * size.x * size.y;
                        region.values[index as usize] = slab.rgba_at(brick_index(
                            local.x as usize,
                            local.y as usize,
                            local.z as usize,
                        ));
                    }
                }
            }
        }
    }
}

/// The paint of a box of voxels, copied out. See [`Volume::snapshot_paint`].
#[derive(Debug, Default, Clone)]
pub struct PaintRegion {
    lo: IVec3,
    size: IVec3,
    values: Vec<Rgba>,
}

impl PaintRegion {
    pub fn new() -> Self {
        Self::default()
    }

    /// The paint `voxel` held when the copy was taken, or [`BARE`] for one
    /// outside the box.
    #[inline]
    pub fn get(&self, voxel: IVec3) -> Rgba {
        let local = voxel - self.lo;
        if local.min_element() < 0 || local.cmpge(self.size).any() {
            return BARE;
        }
        self.values
            [(local.x + local.y * self.size.x + local.z * self.size.x * self.size.y) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::Document;
    use crate::brush::{Brush, BrushKind, MoveStroke, Symmetry};
    use crate::mesh::{BrickMesh, MeshScratch};
    use crate::orientation::Facing;

    const RED: Rgba = [200, 30, 20, 255];
    const BLUE: Rgba = [20, 40, 210, 128];

    fn ball(radius: f32) -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::ZERO, radius);
        volume
    }

    /// A ball painted red where x is positive and a half-covering blue
    /// where it is not.
    fn painted() -> Volume {
        let mut volume = ball(12.0);
        volume.edit_paint(
            IVec3::splat(-20),
            IVec3::splat(20),
            |_, at, _, _| {
                if at.x > 0.0 { RED } else { BLUE }
            },
        );
        volume
    }

    /// The cells the surface passes through: the ones a mesh vertex reads.
    fn shell(volume: &Volume, reach: i32) -> Vec<IVec3> {
        let mut cells = Vec::new();
        for z in -reach..=reach {
            for y in -reach..=reach {
                for x in -reach..=reach {
                    let cell = IVec3::new(x, y, z);
                    if volume.sample_voxel(cell).abs() < 0.5 {
                        cells.push(cell);
                    }
                }
            }
        }
        cells
    }

    /// The contracts on this module are checked in a debug build, and one
    /// that is broken says which. (`write` is for paint as it is stored: a
    /// colour with no coverage is not that.)
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "paint is written as it is stored")]
    fn a_contract_is_checked_in_a_debug_build() {
        PaintField::default().write(IVec3::ZERO, [9, 9, 9, 0]);
    }

    /// An unpainted field costs nothing and reads as bare; a voxel painted
    /// reads back; one painted with no coverage is bare whatever colour it
    /// was handed; and a brick painted back to bare is dropped.
    #[test]
    fn a_field_holds_what_is_painted_and_nothing_where_nothing_is() {
        let mut field = PaintField::default();
        assert!(field.is_empty());
        assert_eq!(field.at(IVec3::new(5, -7, 300)), BARE);
        let before = field.revision();

        let cell = IVec3::new(3, 4, 5);
        field.write(cell, RED);
        assert_eq!(field.at(cell), RED);
        assert_eq!(field.at(cell + IVec3::X), BARE);
        assert!(field.revision() > before, "a write did not move the revision");
        // Across a brick boundary, and in a negative brick.
        for far in [IVec3::new(31, 0, 0), IVec3::new(32, 0, 0), IVec3::new(-1, -33, 64)] {
            field.write(far, BLUE);
            assert_eq!(field.at(far), BLUE, "{far}");
        }

        assert_eq!(stored([9, 9, 9, 0]), BARE);
        assert_eq!(stored([9, 9, 9, 1]), [9, 9, 9, 1]);

        // Writing bare where nothing is painted allocates nothing.
        let mut empty = PaintField::default();
        empty.write(cell, BARE);
        assert!(empty.is_empty());

        // A brick all one paint is a tile, and one wiped is gone.
        let mut brick = PaintBrick::dense_filled(RED);
        assert_eq!(brick.is_collapsible(), Some(RED));
        brick.make_dense()[77] = BLUE;
        assert_eq!(brick.is_collapsible(), None);
        assert_eq!(brick.heap_bytes(), BRICK_VOXELS * 4);
        assert_eq!(PaintBrick::Uniform(RED).heap_bytes(), 0);
        let mut wiped = PaintField::default();
        wiped.write(cell, RED);
        wiped.write(cell, BARE);
        wiped.collapse();
        assert!(wiped.is_empty(), "a brick painted back to bare was kept");
    }

    /// Only the surface is painted: a voxel deep inside or far outside is
    /// never offered, and painting a ball allocates nothing for its
    /// saturated interior. Paint handed back with no coverage is stored as
    /// bare, and the protection at each voxel is handed to the edit.
    #[test]
    fn an_edit_paints_the_surface_and_only_the_surface() {
        let mut volume = ball(80.0);
        volume.edit_paint(IVec3::splat(-90), IVec3::splat(90), |_, _, _, _| RED);
        assert_eq!(volume.paint().at(IVec3::ZERO), BARE, "the middle of the ball was painted");
        assert_eq!(volume.paint().at(IVec3::new(88, 0, 0)), BARE, "empty space was painted");
        assert_eq!(volume.paint().at(IVec3::new(80, 0, 0)), RED);
        assert_eq!(volume.paint().at(IVec3::new(78, 0, 0)), RED, "the band under the surface");
        // The first brick from the centre reaches 54 voxels out at its far
        // corner, and the surface is at 80: it is solid throughout.
        assert!(volume.paint().brick(BrickCoord::new(0, 0, 0)).is_none(), "an interior brick");

        volume.edit_paint(IVec3::splat(-90), IVec3::splat(90), |_, at, held, _| {
            if at.y > 0.0 { [77, 77, 77, 0] } else { held }
        });
        assert_eq!(volume.paint().at(IVec3::new(0, 80, 0)), BARE, "no coverage was stored");
        assert_eq!(volume.paint().at(IVec3::new(0, -80, 0)), RED);

        // The mask is handed over, resolved, for the edit to do as it likes.
        let mut masked = ball(12.0);
        masked.edit_mask(IVec3::splat(-20), IVec3::splat(20), |_, at, _| {
            if at.x > 0.0 { crate::PROTECTED } else { crate::UNMASKED }
        });
        masked.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, held, protection| {
            if protection > 0 { held } else { RED }
        });
        assert_eq!(masked.paint().at(IVec3::new(12, 0, 0)), BARE, "paint under the mask");
        assert_eq!(masked.paint().at(IVec3::new(-12, 0, 0)), RED);
    }

    /// A stroke that changes paint and nothing else is a stroke: it has an
    /// undo entry, the entry weighs what the bricks it holds weigh, undoing
    /// it takes the paint off and redoing it puts it back.
    #[test]
    fn a_stroke_of_paint_alone_is_recorded_and_undoes() {
        let mut volume = ball(12.0);
        volume.begin_stroke();
        volume.edit_paint(
            IVec3::splat(-20),
            IVec3::splat(20),
            |_, at, _, _| {
                if at.x > 0.0 { RED } else { BARE }
            },
        );
        let edit = volume.end_stroke().expect("a stroke that painted recorded nothing");
        assert_eq!(edit.len(), 0, "a paint stroke recorded bricks of the field");
        assert!(edit.paint_len() > 0 && !edit.is_empty());
        assert!(edit.bytes() > 0);
        let painted =
            shell(&volume, 16).iter().filter(|cell| volume.paint().at(**cell) == RED).count();
        assert!(painted > 200, "only {painted} cells of the shell were painted");

        // What the stroke itself marked is taken first, so that what is
        // found after the undo is the undo's.
        let mut dirty = Vec::new();
        volume.take_dirty(&mut dirty);
        assert!(!dirty.is_empty(), "painting marked nothing to remesh");
        dirty.clear();
        let redo = volume.apply_edit(edit);
        assert!(volume.paint().is_empty(), "undo left paint behind");
        assert!(redo.paint_len() > 0);
        volume.take_dirty(&mut dirty);
        assert!(!dirty.is_empty(), "undoing paint marked nothing to remesh");
        volume.apply_edit(redo);
        let again =
            shell(&volume, 16).iter().filter(|cell| volume.paint().at(**cell) == RED).count();
        assert_eq!(again, painted);

        // A second coat over the first: undo is the first coat, not bare.
        volume.begin_stroke();
        volume.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, held, _| {
            if held == RED { BLUE } else { held }
        });
        let second = volume.end_stroke().expect("the second coat");
        assert!(second.bytes() > edit_bytes_of_nothing(), "a recorded dense brick weighed nothing");
        assert_eq!(volume.paint().at(IVec3::new(12, 0, 0)), BLUE);
        volume.apply_edit(second);
        assert_eq!(volume.paint().at(IVec3::new(12, 0, 0)), RED);

        // And a stroke that painted nothing is no stroke.
        volume.begin_stroke();
        volume.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, held, _| held);
        assert!(volume.end_stroke().is_none());
        // Nor does it leave anything behind on a body with no paint, where
        // every brick it looked into had to be made to be looked into.
        let mut untouched = ball(12.0);
        untouched.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, held, _| held);
        assert!(untouched.paint().is_empty(), "an edit that changed nothing left bricks behind");

        // Two edits in one stroke are one undo, back to before the first:
        // what a brick held when the stroke began is recorded once.
        let before = volume.paint().at(IVec3::new(12, 0, 0));
        assert_eq!(before, RED);
        volume.begin_stroke();
        volume.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, _, _| BLUE);
        volume.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, _, _| [1, 2, 3, 4]);
        let both = volume.end_stroke().expect("two edits");
        volume.apply_edit(both);
        assert_eq!(volume.paint().at(IVec3::new(12, 0, 0)), RED, "undo went back one edit");
        assert_eq!(volume.paint().at(IVec3::new(-12, 0, 0)), BARE);

        // A stroke that wipes every voxel leaves no brick behind, and paint
        // is counted in what a body weighs.
        let weighed = volume.stats().resident_bytes;
        volume.begin_stroke();
        volume.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, _, _| BARE);
        volume.end_stroke().expect("the wipe");
        assert!(volume.paint().is_empty(), "a wiped body still holds paint bricks");
        assert!(
            volume.stats().resident_bytes + BRICK_VOXELS < weighed,
            "paint weighed nothing: {} bytes with it and {} without",
            weighed,
            volume.stats().resident_bytes
        );
    }

    /// The edit is handed the protection as the user sees it, whichever way
    /// the mask is read. `edit_colour_hands_over_the_resolved_protection`,
    /// for the paint.
    #[test]
    fn an_edit_is_handed_the_protection_as_it_is_resolved() {
        let mut volume = ball(24.0);
        let cell = IVec3::new(24, 0, 0);
        // And what it paints is marked to be drawn again.
        let mut dirty = Vec::new();
        volume.take_dirty(&mut dirty);
        dirty.clear();
        volume.edit_paint(cell, cell, |_, _, _, _| RED);
        volume.take_dirty(&mut dirty);
        assert!(!dirty.is_empty(), "painting a voxel marked nothing to draw again");

        volume.mask_mut().write(cell, crate::PROTECTED);
        let seen = std::cell::RefCell::new(Vec::new());
        volume.edit_paint(cell, cell, |_, _, held, protection| {
            seen.borrow_mut().push(protection);
            held
        });
        assert_eq!(*seen.borrow(), vec![crate::PROTECTED]);
        volume.mask_mut().set_inverted(true);
        volume.edit_paint(cell, cell, |_, _, held, protection| {
            seen.borrow_mut().push(protection);
            held
        });
        assert_eq!(*seen.borrow(), vec![crate::PROTECTED, crate::UNMASKED], "polarity not applied");

        // And the position in millimetres, not in voxels.
        let mut fine = Volume::new(0.5);
        fine.seed_sphere(Vec3::ZERO, 6.0);
        fine.edit_paint(IVec3::splat(-16), IVec3::splat(16), |voxel, position, _, _| {
            assert!(
                (position - voxel.as_vec3() * 0.5).abs().max_element() < 1e-5,
                "voxel {voxel} was handed {position}"
            );
            RED
        });
        assert_eq!(fine.paint().at(IVec3::new(12, 0, 0)), RED);
    }

    /// An edit that asks is told what each voxel held before the stroke
    /// that is open, whatever the stroke has done to it since and however
    /// its brick was stored: not there, one value, or dense. With no
    /// stroke open that is what it holds.
    #[test]
    fn an_edit_is_told_what_a_voxel_held_before_the_stroke() {
        let (lo, hi) = (IVec3::splat(-20), IVec3::splat(20));
        let seen = |volume: &mut Volume, lay: Rgba| {
            let mut pairs = std::collections::BTreeSet::new();
            volume.edit_paint_over(lo, hi, |_, _, held, before, _| {
                pairs.insert((held, before));
                lay
            });
            pairs
        };
        let only = |pair: (Rgba, Rgba)| std::collections::BTreeSet::from([pair]);

        // No stroke open: before is now.
        let mut volume = ball(12.0);
        assert_eq!(seen(&mut volume, RED), only((BARE, BARE)));
        assert_eq!(seen(&mut volume, BLUE), only((RED, RED)));

        // A brick that was not there, then one that was dense: half the
        // ball red and half blue, so that it is not one value.
        for start in [None, Some(())] {
            let mut volume = ball(12.0);
            let was = |at: Vec3| match start {
                None => BARE,
                Some(()) if at.x > 0.0 => RED,
                Some(()) => BLUE,
            };
            volume.edit_paint(lo, hi, |_, at, _, _| was(at));
            volume.begin_stroke();
            volume.edit_paint(lo, hi, |_, _, _, _| [1, 2, 3, 4]);
            let mut checked = 0;
            volume.edit_paint_over(lo, hi, |_, at, held, before, _| {
                assert_eq!(held, [1, 2, 3, 4]);
                assert_eq!(before, was(at), "at {at}");
                checked += 1;
                [5, 6, 7, 8]
            });
            assert!(checked > 1000);
            // And still, after a second change in the same stroke.
            volume.edit_paint_over(lo, hi, |_, at, held, before, _| {
                assert_eq!((held, before), ([5, 6, 7, 8], was(at)), "at {at}");
                held
            });
            volume.end_stroke().expect("a stroke");
            // The stroke over, before is now again.
            assert_eq!(seen(&mut volume, BARE), only(([5, 6, 7, 8], [5, 6, 7, 8])));
        }

        // A brick that was one value throughout.
        let mut volume = ball(12.0);
        for coord in volume.brick_coords().collect::<Vec<_>>() {
            volume.paint_mut().insert(coord, PaintBrick::Uniform(RED));
        }
        volume.begin_stroke();
        volume.edit_paint(lo, hi, |_, _, _, _| BLUE);
        assert_eq!(seen(&mut volume, BLUE), only((BLUE, RED)));
    }

    /// A brick that is one paint throughout is stored as that paint, and is
    /// turned and shifted like any other; and two of them side by side are
    /// not one.
    #[test]
    fn a_brick_of_one_paint_is_kept_small_and_moved_like_any_other() {
        // Where a voxel is in a brick, and back.
        for index in [0, 1, 31, 32, 33, 1023, 1024, 12_345, BRICK_VOXELS - 1] {
            let local = local_of(index);
            assert_eq!(
                brick_index(local.x as usize, local.y as usize, local.z as usize),
                index,
                "{local}"
            );
        }

        let mut field = PaintField::default();
        for index in 0..BRICK_VOXELS {
            field.write(local_of(index), RED);
        }
        let home = BrickCoord::new(0, 0, 0);
        assert_eq!(field.bricks()[&home].heap_bytes(), BRICK_VOXELS * 4);
        field.collapse();
        assert_eq!(field.bricks()[&home], PaintBrick::Uniform(RED));
        assert_eq!(field.bricks()[&home].heap_bytes(), 0);

        let rotation = AxisRotation::taking(Facing::Right, Facing::Up);
        let turned = field.rotated(rotation);
        let cell = IVec3::new(3, 4, 5);
        assert_eq!(turned.at(rotation.apply_voxel(cell)), RED, "a turn lost a brick of one paint");

        // Red beside blue, moved half a brick along: the brick between them
        // is half of each.
        field.insert(BrickCoord::new(1, 0, 0), PaintBrick::Uniform(BLUE));
        let shifted = field.shifted(IVec3::new(16, 0, 0));
        for (x, wanted) in [(15, BARE), (16, RED), (47, RED), (48, BLUE), (79, BLUE), (80, BARE)] {
            assert_eq!(shifted.at(IVec3::new(x, 7, 7)), wanted, "x = {x}");
        }
    }

    /// A change of scale reaches past the edge of the brick the paint was
    /// in, since a voxel there is the nearest to cells of the brick beside
    /// it, and leaves no brick behind that holds nothing.
    #[test]
    fn a_change_of_scale_reaches_past_its_brick_and_leaves_no_empty_ones() {
        let no_empty = |field: &PaintField, what: &str| {
            assert!(!field.is_empty(), "{what}: nothing came through");
            for (coord, brick) in field.bricks() {
                assert_ne!(brick.is_collapsible(), Some(BARE), "{what}: {coord:?} holds nothing");
            }
        };
        // One voxel, the first of its brick.
        let mut field = PaintField::default();
        field.write(IVec3::new(32, 5, 5), RED);
        // Four times finer, the nearest voxel to cells 126 to 129 is that
        // one, and the first two of those are in the brick before.
        let across = |field: &PaintField, what: &str| {
            for (x, wanted) in
                [(125, BARE), (126, RED), (127, RED), (128, RED), (129, RED), (130, BARE)]
            {
                assert_eq!(field.at(IVec3::new(x, 20, 20)), wanted, "{what}, x = {x}");
            }
            no_empty(field, what);
        };
        across(&field.resampled(1.0, 0.25), "a finer lattice");
        let grown =
            Similarity::about(Vec3::ZERO, glam::Quat::IDENTITY, Vec3::splat(4.0), Vec3::ZERO);
        across(&field.warped(grown, 1.0), "a body scaled up");

        // Half as fine: a block, since a lone voxel can fall between cells.
        let mut field = PaintField::default();
        for z in 4..8 {
            for y in 4..8 {
                for x in 32..36 {
                    field.write(IVec3::new(x, y, z), RED);
                }
            }
        }
        let coarser = field.resampled(0.5, 1.0);
        assert_eq!(coarser.at(IVec3::new(16, 2, 2)), RED);
        assert_eq!(coarser.at(IVec3::new(18, 2, 2)), BARE);
        no_empty(&coarser, "a coarser lattice");
    }

    /// What an entry that holds no brick's storage weighs at most: a few
    /// table rows.
    fn edit_bytes_of_nothing() -> usize {
        64 * size_of::<(BrickCoord, Option<PaintBrick>)>()
    }

    /// Everything that moves a body whole carries its paint to the same
    /// place on it: a copy, a shift, a quarter turn, a free turn, a
    /// resample.
    #[test]
    fn a_body_moved_whole_keeps_its_paint_where_it_was_on_it() {
        let volume = painted();
        let cells = shell(&volume, 16);
        assert!(cells.len() > 800);
        let (mut red, mut blue) = (0, 0);
        for cell in &cells {
            match volume.paint().at(*cell) {
                RED => red += 1,
                BLUE => blue += 1,
                other => panic!("{cell} of the fixture holds {other:?}"),
            }
        }
        assert!(red > 300 && blue > 300);

        // A copy, a whole number of bricks away.
        let copy = volume.duplicated(IVec3::new(1, 0, -2));
        let offset = IVec3::new(32, 0, -64);
        for cell in &cells {
            assert_eq!(copy.paint().at(*cell + offset), volume.paint().at(*cell));
        }
        assert_eq!(copy.paint().at(cells[0]), BARE, "the copy left paint where it came from");

        // A shift that is not a whole number of bricks.
        let offset = IVec3::new(5, -9, 40);
        let shifted = volume.shifted(offset);
        for cell in &cells {
            assert_eq!(shifted.paint().at(*cell + offset), volume.paint().at(*cell), "{cell}");
        }

        // A quarter turn: what faced +x is red, wherever it faces now.
        let rotation = AxisRotation::taking(Facing::Right, Facing::Up);
        let turned = volume.rotated(rotation);
        for cell in &cells {
            assert_eq!(turned.paint().at(rotation.apply_voxel(*cell)), volume.paint().at(*cell));
        }
        // Control: the turn moved the paint, so the check above was of
        // something.
        let moved = cells
            .iter()
            .filter(|cell| turned.paint().at(**cell) != volume.paint().at(**cell))
            .count();
        assert!(moved > 200, "a quarter turn changed the paint of only {moved} cells");

        // A free turn, which is a resample: nearest, so checked a little
        // way off the line between the two paints.
        let by = Similarity::about(
            Vec3::ZERO,
            glam::Quat::from_rotation_z(0.6),
            Vec3::ONE,
            Vec3::new(3.0, 0.0, 0.0),
        );
        let warped = volume.warped(by);
        let mut checked = 0;
        for cell in shell(&warped, 20) {
            let from = by.inverse_transform_point(cell.as_vec3());
            if from.x.abs() < 2.0 {
                continue;
            }
            let wanted = if from.x > 0.0 { RED } else { BLUE };
            assert_eq!(warped.paint().at(cell), wanted, "{cell} came from {from}");
            checked += 1;
        }
        assert!(checked > 600, "only {checked} cells of the turned ball were checked");

        // A resample to twice the detail.
        let finer = volume.resampled(0.5);
        let mut checked = 0;
        for cell in shell(&finer, 30) {
            if (cell.x as f32 * 0.5).abs() < 1.5 {
                continue;
            }
            let wanted = if cell.x > 0 { RED } else { BLUE };
            assert_eq!(finer.paint().at(cell), wanted, "{cell}");
            checked += 1;
        }
        assert!(checked > 2500, "only {checked} cells of the finer ball were checked");
    }

    /// Each half of a split keeps the paint that was on it and none of the
    /// other half's, and a body merged into another brings its paint, with
    /// an undo that takes it off again.
    #[test]
    fn a_split_and_a_merge_carry_the_paint_of_what_they_move() {
        // Split along the mask: the masked half is the red half.
        let mut volume = painted();
        volume.edit_mask(IVec3::splat(-20), IVec3::splat(20), |_, at, _| {
            if at.x > 0.0 { crate::PROTECTED } else { crate::UNMASKED }
        });
        let mut doc = Document::from_volume(volume);
        let source = doc.active();
        let outcome = doc.split_masked(source).expect("a half masked ball divides");
        let colours_on = |doc: &Document, body| {
            let volume = doc.volume(body).expect("a body");
            let mut found = std::collections::BTreeSet::new();
            for cell in shell(volume, 16) {
                found.insert(volume.paint().at(cell));
            }
            found
        };
        let masked = colours_on(&doc, outcome.masked);
        let rest = colours_on(&doc, outcome.rest);
        assert!(masked.contains(&RED) && !masked.contains(&BLUE), "{masked:?}");
        assert!(rest.contains(&BLUE) && !rest.contains(&RED), "{rest:?}");
        // And no half holds paint where it has no material, which the
        // other half's paint across the cut would be, nor a brick of
        // nothing where that paint was taken off.
        let tidy = |doc: &Document, body, what: &str| {
            let volume = doc.volume(body).expect("a body");
            assert!(!volume.paint().is_empty(), "{what} has no paint");
            for (coord, brick) in volume.paint().bricks() {
                assert_ne!(brick.is_collapsible(), Some(BARE), "{what}: nothing in {coord:?}");
                for index in 0..BRICK_VOXELS {
                    let cell = coord.origin() + local_of(index);
                    assert!(
                        volume.paint().at(cell) == BARE || volume.has_material_at(cell),
                        "{what} holds paint at {cell}, where it has no material"
                    );
                }
            }
        };
        tidy(&doc, outcome.masked, "the masked half");
        tidy(&doc, outcome.rest, "the rest");
        // Paint that is all off the body it is offered to leaves no brick
        // behind on it: here one voxel, in the band and a step clear of the
        // surface.
        let mut part = ball(8.0);
        let mut offered = PaintField::default();
        offered.write(IVec3::new(10, 0, 0), RED);
        part.adopt_rgba(&offered);
        assert!(part.paint().is_empty(), "paint off the body left a brick on it");
        offered.write(IVec3::new(8, 0, 0), RED);
        part.adopt_rgba(&offered);
        assert_eq!(part.paint().at(IVec3::new(8, 0, 0)), RED, "paint on the body was not taken");
        assert_eq!(part.paint().at(IVec3::new(10, 0, 0)), BARE);

        // Split into the pieces it is in: two balls, each with its own.
        // Near enough across a brick boundary that each one's band reaches
        // into the other's bricks, where the paint is all the other's.
        let mut two = Volume::new(1.0);
        for centre in [-6.0, 6.0] {
            let mut one = Volume::new(1.0);
            one.seed_sphere(Vec3::new(centre, 0.0, 0.0), 4.0);
            two.begin_stroke();
            two.union_from(&one);
            two.end_stroke().expect("a merge");
        }
        two.edit_paint(
            IVec3::splat(-20),
            IVec3::splat(20),
            |_, at, _, _| {
                if at.x < 0.0 { RED } else { BLUE }
            },
        );
        let mut doc = Document::from_volume(two);
        let plan = doc.split_plan(doc.active()).expect("the body is there");
        let parts = doc.split(plan).expect("two balls is a split");
        assert_eq!(parts.bodies.len(), 2);
        let mut seen = Vec::new();
        for body in &parts.bodies {
            let colours = colours_on(&doc, *body);
            assert_eq!(colours.len(), 1, "a piece is painted {colours:?}");
            seen.extend(colours);
            tidy(&doc, *body, "a piece");
        }
        seen.sort_unstable();
        assert_eq!(seen, vec![BLUE, RED], "each piece should have its own ball's paint");

        // Merge a painted ball into a bare one beside it.
        let mut into = ball(8.0);
        let mut from = Volume::new(1.0);
        from.seed_sphere(Vec3::new(10.0, 0.0, 0.0), 6.0);
        from.edit_paint(IVec3::splat(-30), IVec3::splat(30), |_, _, _, _| RED);
        let theirs = IVec3::new(16, 0, 0);
        assert_eq!(from.paint().at(theirs), RED, "the fixture's far side is not painted");
        into.begin_stroke();
        into.union_from(&from);
        let edit = into.end_stroke().expect("a merge that changed the field");
        assert_eq!(into.paint().at(theirs), RED, "the merged body's paint did not arrive");
        assert_eq!(into.paint().at(IVec3::new(-8, 0, 0)), BARE);
        assert!(edit.paint_len() > 0, "the merge recorded no paint");
        let mut dirty = Vec::new();
        into.take_dirty(&mut dirty);
        assert!(!dirty.is_empty());
        into.apply_edit(edit);
        assert!(into.paint().is_empty(), "undoing the merge left its paint");

        // Paint the incoming body left behind where it has no material is
        // not material and does not arrive.
        let stray = IVec3::new(40, 0, 0);
        from.paint_mut().write(stray, BLUE);
        let mut into = ball(8.0);
        into.begin_stroke();
        into.union_from(&from);
        into.end_stroke().expect("a merge");
        assert_eq!(into.paint().at(stray), BARE, "paint arrived where nothing is");
        assert_eq!(into.paint().at(theirs), RED);

        // And paint alone, onto a body of the same shape, marks what it
        // changed to be drawn again.
        let mut same = ball(8.0);
        let mut coat = ball(8.0);
        coat.edit_paint(IVec3::splat(-12), IVec3::splat(12), |_, _, _, _| RED);
        let mut dirty = Vec::new();
        same.take_dirty(&mut dirty);
        dirty.clear();
        same.union_paint_from(&coat);
        same.take_dirty(&mut dirty);
        assert!(!dirty.is_empty(), "paint merged in marked nothing to draw again");
        assert_eq!(same.paint().at(IVec3::new(8, 0, 0)), RED);

        // Where the incoming body is bare, what was there stays.
        // Bare and painted within one brick of the incoming body, which is
        // where a brick taken whole would bring its bare voxels with it.
        let mut part = ball(8.0);
        part.edit_paint(
            IVec3::splat(-12),
            IVec3::splat(12),
            |_, at, _, _| {
                if at.x > 4.0 { BLUE } else { BARE }
            },
        );
        let (painted, bare) = (IVec3::new(8, 0, 0), IVec3::new(2, 7, 3));
        assert_eq!((part.paint().at(painted), part.paint().at(bare)), (BLUE, BARE), "the fixture");
        assert_eq!(same.paint().at(bare), RED, "the fixture");
        same.union_paint_from(&part);
        assert_eq!(same.paint().at(painted), BLUE);
        assert_eq!(same.paint().at(bare), RED, "bare paint merged in as paint");
    }

    /// Paint under the mask stays where it is under a Move, as the material
    /// under it does. `paint_under_protection_stays_put_under_a_move`, for
    /// the paint.
    #[test]
    fn paint_under_the_mask_stays_put_under_a_move() {
        let brush = Brush { kind: BrushKind::Move, radius: 9.0, strength: 0.8, ..Brush::default() };
        let at = Vec3::new(20.0, 0.0, 0.0);
        let mut volume = ball(20.0);
        let (lo, hi) = (IVec3::new(8, -14, -14), IVec3::new(40, 14, 14));
        // Two paints side by side across the drag, so a shift would show.
        volume.edit_paint(lo, hi, |cell, _, _, _| if cell.x < 21 { RED } else { BLUE });
        volume.edit_mask(lo, hi, |_, _, _| crate::PROTECTED);
        let before: Vec<(IVec3, Rgba)> = (lo.x..=hi.x)
            .flat_map(|x| (-3..=3).flat_map(move |y| (-3..=3).map(move |z| IVec3::new(x, y, z))))
            .map(|cell| (cell, volume.paint().at(cell)))
            .collect();
        let (red, blue) = (
            before.iter().filter(|(_, paint)| *paint == RED).count(),
            before.iter().filter(|(_, paint)| *paint == BLUE).count(),
        );
        assert!(red > 100 && blue > 50, "the fixture: {red} red and {blue} blue");

        let mut stroke = MoveStroke::new();
        stroke.begin(&volume, &brush, at, Symmetry::OFF, Vec3::ZERO);
        stroke.drag_to(&mut volume, at + Vec3::X * 8.0, 1.0);
        stroke.end();
        for (cell, paint) in before {
            assert_eq!(volume.paint().at(cell), paint, "protected paint moved at {cell}");
        }

        // Half protected, the material goes about half as far and the paint
        // goes with it. Every voxel is painted with its own depth under the
        // surface, so that afterwards the paint says where it came from:
        // paint that had gone as far as an unprotected drag takes it would
        // be the paint of a voxel deeper than the one it is now on.
        let fresh = ball(20.0);
        let depth = |distance: f32| (128.0 + distance * 32.0).round().clamp(1.0, 255.0) as u8;
        let (lo, hi) = (IVec3::new(2, -18, -18), IVec3::new(44, 18, 18));
        let mut volume = ball(20.0);
        volume.edit_paint(lo, hi, |cell, _, _, _| [depth(fresh.sample_voxel(cell)), 9, 9, 255]);
        volume.edit_mask(lo, hi, |_, _, _| 128);
        // A longer drag with a wider brush than above, so that half of it
        // is still some voxels.
        let wide = Brush { radius: 14.0, strength: 1.0, ..brush };
        let mut stroke = MoveStroke::new();
        stroke.begin(&volume, &wide, at, Symmetry::OFF, Vec3::ZERO);
        stroke.drag_to(&mut volume, at + Vec3::X * 12.0, 1.0);
        stroke.end();

        let (mut judged, mut furthest) = (0, 0.0_f32);
        for x in 12..36 {
            for y in -4..=4 {
                for z in -4..=4 {
                    let cell = IVec3::new(x, y, z);
                    let now = volume.sample_voxel(cell);
                    // Near the surface, and where the drag moved it.
                    if now.abs() >= 1.5 || (now - fresh.sample_voxel(cell)).abs() < 0.75 {
                        continue;
                    }
                    let paint = volume.paint().at(cell);
                    assert_ne!(paint, BARE, "the surface at {cell} came away bare");
                    // Nearest and not blended, so out by up to the half
                    // diagonal of a voxel, 0.87, and no more.
                    let worn = (f32::from(paint[0]) - 128.0) / 32.0;
                    assert!(
                        (worn - now).abs() < 1.0,
                        "{cell} is {now} from the surface and wears the paint of {worn}"
                    );
                    judged += 1;
                    furthest = furthest.max((now - fresh.sample_voxel(cell)).abs());
                }
            }
        }
        // Controls: the drag moved enough of the surface to judge by, and
        // moved some of it by more than the comparison allows.
        assert!(judged > 150 && furthest > 1.4, "{judged} voxels judged, moved up to {furthest}");
    }

    /// Paint goes with the material under a Move, as the slots do: the
    /// cells the drag brought to the surface, which were not painted
    /// before it, are painted after it.
    #[test]
    fn paint_goes_with_the_material_under_a_move() {
        let brush = Brush { kind: BrushKind::Move, radius: 9.0, strength: 0.8, ..Brush::default() };
        let at = Vec3::new(20.0, 0.0, 0.0);
        let mut volume = ball(20.0);
        let (lo, hi) = (IVec3::new(8, -14, -14), IVec3::new(40, 14, 14));
        // Only the cells nearest the surface, and not the whole band: the
        // drag moves the surface about as far as the band is deep, and a
        // band painted throughout would be painted where it arrives whether
        // or not anything carried the paint there.
        let near = |volume: &Volume, cell: IVec3| volume.sample_voxel(cell).abs() < 1.5;
        let fresh = ball(20.0);
        volume.edit_paint(lo, hi, |cell, _, held, _| if near(&fresh, cell) { RED } else { held });
        let bare_before: Vec<IVec3> = (lo.x..=hi.x)
            .flat_map(|x| (-3..=3).flat_map(move |y| (-3..=3).map(move |z| IVec3::new(x, y, z))))
            .filter(|cell| volume.paint().at(*cell) == BARE)
            .collect();

        let mut stroke = MoveStroke::new();
        volume.begin_stroke();
        stroke.begin(&volume, &brush, at, Symmetry::OFF, Vec3::ZERO);
        stroke.drag_to(&mut volume, at + Vec3::X * 8.0, 1.0);
        stroke.end();
        let edit = volume.end_stroke().expect("the move changed the body");

        let arrived: Vec<IVec3> = bare_before
            .iter()
            .copied()
            .filter(|cell| volume.sample_voxel(*cell).abs() < 0.5)
            .collect();
        assert!(
            arrived.len() > 5,
            "the drag brought only {} bare cells to the surface",
            arrived.len()
        );
        for cell in &arrived {
            assert_eq!(
                volume.paint().at(*cell),
                RED,
                "material arrived at {cell} without its paint"
            );
        }
        // And the undo of the move is the undo of its paint.
        assert!(edit.paint_len() > 0, "the move recorded no paint");
        volume.apply_edit(edit);
        for cell in &arrived {
            assert_eq!(volume.paint().at(*cell), BARE, "undo left paint at {cell}");
        }
    }

    /// A mesh carries the paint of each vertex's own cell, one for each
    /// vertex, and a run of bare for a body nobody painted.
    #[test]
    fn a_mesh_carries_the_paint_of_each_vertex() {
        let volume = painted();
        let mut scratch = MeshScratch::default();
        let mut mesh = BrickMesh::default();
        let mut vertices = 0;
        for coord in volume.brick_coords() {
            volume.mesh_brick(coord, &mut scratch, &mut mesh);
            assert_eq!(mesh.paint.len(), mesh.vertices.len());
            for (vertex, (cell, paint)) in mesh.cells.iter().zip(&mesh.paint).enumerate() {
                assert_eq!(*paint, volume.paint().at(*cell), "vertex {vertex} of {coord:?}");
                let x = mesh.vertices[vertex].position[0];
                if x.abs() > 2.0 {
                    assert_eq!(*paint, if x > 0.0 { RED } else { BLUE });
                }
            }
            vertices += mesh.vertices.len();
        }
        assert!(vertices > 1000, "the ball meshed to {vertices} vertices");

        let bare = ball(12.0);
        for coord in bare.brick_coords() {
            bare.mesh_brick(coord, &mut scratch, &mut mesh);
            assert_eq!(mesh.paint.len(), mesh.vertices.len());
            assert!(mesh.paint.iter().all(|paint| *paint == BARE));
        }
    }

    /// A copy of a box of paint reads back what was there, and bare outside
    /// the box.
    #[test]
    fn a_copy_of_a_box_of_paint_reads_back_what_was_there() {
        let volume = painted();
        let (lo, hi) = (IVec3::new(-14, -3, -3), IVec3::new(14, 3, 3));
        let mut region = PaintRegion::new();
        volume.snapshot_paint(lo, hi, &mut region);
        let mut painted = 0;
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                for z in lo.z..=hi.z {
                    let cell = IVec3::new(x, y, z);
                    assert_eq!(region.get(cell), volume.paint().at(cell), "{cell}");
                    painted += usize::from(region.get(cell) != BARE);
                }
            }
        }
        assert!(painted > 100, "the box held only {painted} painted cells");
        assert_eq!(region.get(IVec3::new(0, 12, 0)), BARE, "a cell outside the box");
        assert_eq!(region.get(lo - IVec3::X), BARE);
        // One past the far side is outside, and is not the first cell of
        // the next row, which is painted.
        assert_ne!(region.get(IVec3::new(lo.x, 1, 0)), BARE, "the fixture");
        assert_eq!(region.get(IVec3::new(hi.x + 1, 0, 0)), BARE);

        // Reused for a body with no paint, it is all bare.
        ball(12.0).snapshot_paint(lo, hi, &mut region);
        assert_eq!(region.get(IVec3::new(12, 0, 0)), BARE);
    }
}
