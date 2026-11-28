// SPDX-License-Identifier: AGPL-3.0-only

//! Filling: a paint poured onto an area, which runs until something stops
//! it.
//!
//! A base coat at the bench is area by area. This panel in blue, that strap
//! in brown. With a brush that is a lot of careful strokes up to a line, and
//! the line is nearly always one the model already has: the groove round
//! the panel, the edge of the strap. So the fill here is a flood from the
//! point under the cursor, and what stops it is the model's own shape.
//!
//! # What stops a fill
//!
//! * **Another paint.** A fill replaces the paint it started on and spreads
//!   only through that paint, as a bucket does in any painting program. On
//!   an unpainted model that is everything, and the next two are what bound
//!   it.
//! * **A crease**, when asked: a groove, a panel line, the foot of a rivet.
//!   The fill runs up to it from one side and does not cross.
//! * **An edge**, when asked: the rim of a plate, the corner of a block.
//! * **The mask**, always, on the half line every paint stamp uses.
//! * **A limit on how much**, so that a click on a model with no lines on it
//!   is an answer and not a wait.
//!
//! A fill that starts ON a crease or an edge it was asked to stop at does
//! the opposite: it runs along the line and stays inside it. That is panel
//! lining, a dark paint in every groove, in a click a groove.
//!
//! The crease and the edge are measured at the gain the viewport draws them
//! with, so switching "show creases" on is how to see where a fill will go
//! before pouring it.
//!
//! They are NOT the same number the viewport shades with, and the difference
//! was found by a fill going where it should not. The viewport's signal is
//! the sum of the surface's two curvatures, which is what a wash wants. But
//! where a groove runs off the edge of a part a concave line meets a convex
//! one, the sum passes through nothing, and a fill asked to stop at edges
//! walked through that point onto the next face.
//!
//! The two curvatures taken separately, from the field's second
//! derivatives, were tried next and are not here. They are right along a
//! line and weak exactly where three faces meet, which is where the hole
//! was: 0.25 of a full edge at the voxel that mattered, under any threshold
//! worth setting.
//!
//! What a fill reads is how far the surface's NORMAL turns between a voxel
//! and each of its six neighbours, along the step between them. Turning
//! apart is an edge and turning together is a crease, a voxel can be both,
//! and where three faces meet the normals of neighbouring voxels point three
//! ways, which is the strongest reading there is and not the weakest.
//!
//! # A panel is painted up to the middle of its lines
//!
//! A line has a width: two or three voxels. A fill that stopped at the
//! first voxel of it would leave every panel with a bare margin, and two
//! panels filled side by side with a bare stripe between them. So once the
//! flood has run, it goes on [`INTO_THE_LINE`] voxels further into the lines
//! that stopped it, and no further and nowhere else.
//!
//! # On the surface first, and then through the band
//!
//! Paint is in the whole narrow band and not only on the surface, so that it
//! is still there when the surface moves a voxel. A flood cannot run through
//! the whole band, though, and the first two versions of this did.
//!
//! The first judged each voxel where it stood, and ran under a groove
//! through the voxels a little inside the model. The second judged each
//! voxel by the point of the surface nearest it, which is how a drybrush
//! judges ([`super::touch`]) and is right for a drybrush. It still leaked,
//! at every sharp edge: inside a sharp convex edge NO voxel's nearest point
//! is the edge. The voxels on one side of the diagonal are nearest one face
//! and those on the other are nearest the other, and the flood stepped from
//! the first to the second without meeting the line at all. Measured, on
//! this module's fixture, as 3,538 voxels on a face it should not have
//! reached.
//!
//! So the flood runs in the SHELL: the voxels within [`SHELL`] of the
//! surface, one or two deep, where a line is a row of voxels it has to
//! cross. Then every other voxel of the band takes the answer of the shell
//! voxel it leads down to, by stepping to whichever neighbour is nearer the
//! surface until it is in the shell.

use std::collections::{HashMap, HashSet, VecDeque};

use contracts::debug_ensures;
use glam::{IVec3, Vec3};

use crate::colour::ColourField;
use crate::mask::PROTECTED;
use crate::paint::field::Rgba;
use crate::paint::stroke::{Pigment, laid};
use crate::volume::Volume;

/// A voxel at least this protected takes no paint. The line every paint
/// stamp draws: see [`super::touch`], which holds its own copy to upstream's.
const PAINT_PROTECTED: u8 = PROTECTED / 2 + 1;

/// How many voxels into a line that stopped it a fill goes on painting.
///
/// Two, because a line reads over about three voxels and two panels meeting
/// on it are then painted to the middle from each side. More would put a
/// panel's paint on the far side of a thin line; it never leaves the line,
/// so the worst that does is paint the whole of it.
pub const INTO_THE_LINE: usize = 2;

/// How far from the surface, in voxels, the flood itself runs.
///
/// One: the slab of voxels within one of a plane is two thick, which is
/// thick enough that six-connected steps can follow a surface at any angle
/// (a slab thinner than the square root of three can be stepped through
/// diagonally), and thin enough that a sharp line cannot be passed
/// underneath.
pub const SHELL: f32 = 1.0;

/// What a fill is asked to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    /// The paint to pour, or 0 to strip the area back to unpainted.
    pub slot: u8,
    /// Stop at a recess at least this deep, as a share of a full crease,
    /// 0..=1. `None` to run across creases.
    pub stop_at_creases: Option<f32>,
    /// Stop at an edge at least this sharp, likewise. `None` to run over
    /// edges.
    pub stop_at_edges: Option<f32>,
    /// How many units of the crease signal a full crease is: the viewport's
    /// gain.
    pub crease_gain: f32,
    /// The most voxels of the shell one fill may flood.
    pub limit: usize,
    /// How many voxels into a line that stopped it the fill goes on
    /// painting. [`INTO_THE_LINE`] unless there is a reason.
    pub into_the_line: usize,
}

/// What a fill did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filled {
    /// How many voxels changed paint.
    pub voxels: usize,
    /// Whether it stopped because it reached [`Fill::limit`] and not because
    /// it ran out of area. What was painted up to then stays painted.
    pub reached_the_limit: bool,
}

impl Filled {
    /// A fill that had nowhere to go.
    pub const NOTHING: Filled = Filled { voxels: 0, reached_the_limit: false };
}

impl Volume {
    /// Pour `fill.slot` from the surface nearest `seed`, a world position.
    ///
    /// Nothing happens, and `Filled::NOTHING` comes back, when the seed is
    /// not on the model, is protected by the mask, or already holds the
    /// paint. One call is one edit: with an undo recorder armed it is one
    /// entry, like a stroke.
    ///
    /// [`Fill::limit`] counts the voxels of the shell, which is what the
    /// flood costs. [`Filled::voxels`] counts every voxel painted, through
    /// the band.
    pub fn fill_paint(&mut self, seed: Vec3, fill: Fill) -> Filled {
        let seed = (seed / self.voxel_size()).round().as_ivec3();
        let replacing = self.colour().at(seed);
        if replacing == fill.slot {
            return Filled::NOTHING;
        }
        let same = |volume: &Volume, voxel: IVec3| volume.colour().at(voxel) == replacing;
        let Some(flood) = self.flooded(seed, fill, &same) else {
            return Filled::NOTHING;
        };
        let slot = fill.slot;
        let reached = &flood.reached;
        self.edit_colour(
            flood.lo,
            flood.hi,
            |voxel, _, held, _| {
                if reached.contains(&voxel) { slot } else { held }
            },
        );
        Filled { voxels: flood.reached.len(), reached_the_limit: flood.reached_the_limit }
    }

    /// Pour a colour from the surface nearest `seed`, a world position: the
    /// colour brush's fill ([`crate::paint::stroke`]).
    ///
    /// It runs as [`Volume::fill_paint`] runs, over the surface until the
    /// model's own lines stop it, and over paint LIKE the paint where it
    /// began: within `tolerance` levels of it, in colour and in coverage.
    /// A slot is the same as another or it is not, and a colour is more or
    /// less like another, so a fill of colour has to be told how alike is
    /// alike. Bare clay is like bare clay, whatever colour it last was.
    ///
    /// What it reaches takes the pigment at full strength, by the
    /// pigment's own opacity and in its own mode, less what the mask holds
    /// off. `lines` gives the lines to stop at and the limit; its slot is
    /// not read.
    ///
    /// Nothing happens when the seed is not on the model, is protected, or
    /// would not be changed by the pigment.
    #[debug_ensures(ret.voxels > 0 || !ret.reached_the_limit, "a fill that reached its limit reached something")]
    pub fn fill_colour(
        &mut self,
        seed: Vec3,
        pigment: Pigment,
        tolerance: u8,
        lines: Fill,
    ) -> Filled {
        let seed = (seed / self.voxel_size()).round().as_ivec3();
        let like = self.paint().at(seed);
        if laid(like, pigment, 1.0) == like {
            return Filled::NOTHING;
        }
        let same = |volume: &Volume, voxel: IVec3| alike(volume.paint().at(voxel), like, tolerance);
        let Some(flood) = self.flooded(seed, lines, &same) else {
            return Filled::NOTHING;
        };
        let reached = &flood.reached;
        self.edit_paint(flood.lo, flood.hi, |voxel, _, held, protection| {
            if !reached.contains(&voxel) {
                return held;
            }
            let free = 1.0 - f32::from(protection) / f32::from(PROTECTED);
            laid(held, pigment, free)
        });
        Filled { voxels: flood.reached.len(), reached_the_limit: flood.reached_the_limit }
    }

    /// The voxels a fill from `seed` reaches: over the shell until a line
    /// stops it, into the line, and through the band. `same` says whether
    /// a voxel holds what the fill is replacing. `None` when the seed is
    /// not on the model or is protected.
    fn flooded(
        &self,
        seed: IVec3,
        fill: Fill,
        same: &dyn Fn(&Volume, IVec3) -> bool,
    ) -> Option<Flood> {
        if self.sample_voxel(seed).abs() > SHELL || self.mask().at(seed) >= PAINT_PROTECTED {
            return None;
        }

        let mut judge = Judge { volume: self, fill, same, normals: HashMap::new() };
        // A fill that starts ON a line it was asked to stop at runs along
        // the line and stays in it. Otherwise a click in a groove would
        // paint one voxel, and a click in a groove means the groove: that is
        // panel lining, which at the bench is a fine brush and a steady
        // hand.
        let in_the_line = judge.stops(seed);
        let mut reached: HashSet<IVec3> = HashSet::new();
        let mut frontier = VecDeque::new();
        // The voxels of a line that stopped the flood, to go on into after.
        let mut shore: Vec<IVec3> = Vec::new();
        reached.insert(seed);
        frontier.push_back(seed);
        let mut reached_the_limit = false;
        'flood: while let Some(voxel) = frontier.pop_front() {
            for step in STEPS {
                let next = voxel + step;
                if reached.contains(&next) || !judge.in_the_shell(next) {
                    continue;
                }
                if judge.stops(next) != in_the_line {
                    if !in_the_line {
                        shore.push(next);
                    }
                    continue;
                }
                if reached.len() >= fill.limit {
                    reached_the_limit = true;
                    break 'flood;
                }
                reached.insert(next);
                frontier.push_back(next);
            }
        }

        // Into the lines, a ring at a time, through line and nothing else.
        for _ in 0..fill.into_the_line {
            if reached_the_limit {
                break;
            }
            let ring = std::mem::take(&mut shore);
            for voxel in ring {
                if reached.contains(&voxel) {
                    continue;
                }
                if reached.len() >= fill.limit {
                    reached_the_limit = true;
                    break;
                }
                reached.insert(voxel);
                for step in STEPS {
                    let next = voxel + step;
                    if !reached.contains(&next) && judge.in_the_shell(next) && judge.stops(next) {
                        shore.push(next);
                    }
                }
            }
        }

        // Through the band. A voxel off the shell is painted when the
        // neighbour it leads down to is: the one nearest the surface of all
        // twenty-six. Twenty-six and not six, because a field cut by a plane
        // is flat along the diagonal inside a concave corner, and a voxel
        // there has no face neighbour nearer the surface than itself. That
        // was measured as 242 voxels of the fixture left bare by a fill
        // with nothing to stop it. The voxels to ASK are found by face
        // steps, which is enough: such a voxel is still beside two that are
        // painted.
        let mut frontier: VecDeque<IVec3> = reached.iter().copied().collect();
        while let Some(voxel) = frontier.pop_front() {
            for step in STEPS {
                let next = voxel + step;
                if reached.contains(&next) || !judge.takes(next) {
                    continue;
                }
                let depth = judge.volume.sample_voxel(next).abs();
                if depth <= SHELL {
                    // A shell voxel the flood did not reach is one it was
                    // stopped from reaching.
                    continue;
                }
                // How near the surface its nearest neighbour is, and then
                // whether any neighbour that near is painted. "Any", because
                // along a straight edge three neighbours in a row are
                // exactly as near, and taking the first of them made a voxel
                // beside the mask follow a masked one and stay bare.
                let around_next = |around: i32| {
                    next + IVec3::new(around % 3 - 1, (around / 3) % 3 - 1, around / 9 - 1)
                };
                let nearest = (0..27)
                    .map(|around| judge.volume.sample_voxel(around_next(around)).abs())
                    .fold(depth, f32::min);
                let led_down = nearest < depth
                    && (0..27).any(|around| {
                        let neighbour = around_next(around);
                        judge.volume.sample_voxel(neighbour).abs() <= nearest + 1.0e-4
                            && reached.contains(&neighbour)
                    });
                if led_down {
                    reached.insert(next);
                    frontier.push_back(next);
                }
            }
        }

        let (mut lo, mut hi) = (seed, seed);
        for voxel in &reached {
            lo = lo.min(*voxel);
            hi = hi.max(*voxel);
        }
        Some(Flood { reached, lo, hi, reached_the_limit })
    }
}

/// What a flood reached: the voxels, the box they are in, and whether it
/// stopped at its limit and not for want of anywhere to go.
struct Flood {
    reached: HashSet<IVec3>,
    lo: IVec3,
    hi: IVec3,
    reached_the_limit: bool,
}

/// Whether two paints are alike enough for a fill of colour to run from
/// one onto the other: their coverage within `tolerance` levels, and their
/// colours too, unless there is so little of either that its colour is
/// not to be seen.
#[debug_ensures(!(one == other) || ret, "a paint is like itself")]
pub fn alike(one: Rgba, other: Rgba, tolerance: u8) -> bool {
    let apart = |channel: usize| one[channel].abs_diff(other[channel]);
    if apart(3) > tolerance {
        return false;
    }
    let faint = one[3] <= tolerance && other[3] <= tolerance;
    faint || (0..3).all(|channel| apart(channel) <= tolerance)
}

/// What a fill asks of a voxel, with the normal at each voxel worked out
/// once.
struct Judge<'a> {
    volume: &'a Volume,
    fill: Fill,
    /// Whether a voxel holds what the fill is replacing.
    same: &'a dyn Fn(&Volume, IVec3) -> bool,
    /// The unit normal of the field at a voxel, or `None` where the voxel
    /// is outside the band or the field is flat there.
    normals: HashMap<IVec3, Option<Vec3>>,
}

impl Judge<'_> {
    /// Whether a voxel can be painted by this fill at all: it is in the
    /// band, it holds the paint being replaced, and the mask leaves it free.
    fn takes(&self, voxel: IVec3) -> bool {
        ColourField::paintable(self.volume.sample_voxel(voxel))
            && (self.same)(self.volume, voxel)
            && self.volume.mask().at(voxel) < PAINT_PROTECTED
    }

    /// Whether the flood itself can stand on a voxel: it can be painted,
    /// and it is at the surface.
    fn in_the_shell(&self, voxel: IVec3) -> bool {
        self.volume.sample_voxel(voxel).abs() <= SHELL && self.takes(voxel)
    }

    fn normal(&mut self, voxel: IVec3) -> Option<Vec3> {
        let volume = self.volume;
        *self.normals.entry(voxel).or_insert_with(|| {
            if !ColourField::paintable(volume.sample_voxel(voxel)) {
                return None;
            }
            Vec3::new(
                volume.sample_voxel(voxel + IVec3::X) - volume.sample_voxel(voxel - IVec3::X),
                volume.sample_voxel(voxel + IVec3::Y) - volume.sample_voxel(voxel - IVec3::Y),
                volume.sample_voxel(voxel + IVec3::Z) - volume.sample_voxel(voxel - IVec3::Z),
            )
            .try_normalize()
        })
    }

    /// How sharp a crease and how sharp an edge the surface is at a voxel,
    /// each in 1/voxel and never negative.
    ///
    /// Along a step from this voxel to a neighbour, the normal either leans
    /// further the way the step goes, which is the surface turning away from
    /// itself, an edge; or leans back against it, a crease. The sharpest of
    /// each over the six steps is the answer. On a ball of radius `R` voxels
    /// both steps along the surface read `1 / R` as an edge, which is its
    /// curvature.
    fn bends(&mut self, voxel: IVec3) -> (f32, f32) {
        let Some(here) = self.normal(voxel) else {
            return (0.0, 0.0);
        };
        let (mut crease, mut edge) = (0.0_f32, 0.0_f32);
        for step in STEPS {
            if let Some(there) = self.normal(voxel + step) {
                let turn = (there - here).dot(step.as_vec3());
                edge = edge.max(turn);
                crease = crease.max(-turn);
            }
        }
        (crease, edge)
    }

    /// Whether a voxel is on a crease or an edge the fill was asked to stop
    /// at.
    fn stops(&mut self, voxel: IVec3) -> bool {
        if self.fill.stop_at_creases.is_none() && self.fill.stop_at_edges.is_none() {
            return false;
        }
        let (crease, edge) = self.bends(voxel);
        let gain = self.fill.crease_gain;
        self.fill.stop_at_creases.is_some_and(|depth| crease * gain >= depth.max(f32::EPSILON))
            || self.fill.stop_at_edges.is_some_and(|sharp| edge * gain >= sharp.max(f32::EPSILON))
    }
}

/// The six steps to a voxel's face neighbours, in the fixed order every
/// loop here takes them in.
const STEPS: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::{INSIDE, NARROW_BAND, OUTSIDE};
    use crate::clip::ClipPlane;

    const VOXEL: f32 = 0.5;
    const HALF: f32 = 12.0;
    /// The slot is 6 mm wide and 5 mm deep, down the middle of the +z face.
    const SLOT_HALF: f32 = 3.0;
    const FLOOR: f32 = HALF - 5.0;

    fn slotted_cube() -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(HALF + NARROW_BAND * VOXEL * 2.0);
        volume.edit_box(-reach, reach, move |position, _| {
            let outside = position.abs() - Vec3::splat(HALF);
            let distance = outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0);
            (distance / VOXEL).clamp(INSIDE, OUTSIDE)
        });
        volume.clip_convex(&[
            ClipPlane { point: Vec3::new(-SLOT_HALF, 0.0, 0.0), normal: Vec3::X },
            ClipPlane { point: Vec3::new(SLOT_HALF, 0.0, 0.0), normal: -Vec3::X },
            ClipPlane { point: Vec3::new(0.0, 0.0, FLOOR), normal: Vec3::Z },
        ]);
        volume
    }

    fn fill(slot: u8) -> Fill {
        Fill {
            slot,
            stop_at_creases: None,
            stop_at_edges: None,
            crease_gain: 1.0,
            limit: usize::MAX,
            into_the_line: INTO_THE_LINE,
        }
    }

    /// Every paintable voxel of the fixture, with where it is and its slot.
    fn voxels(volume: &Volume) -> Vec<(Vec3, u8)> {
        let reach = ((HALF / VOXEL) as i32) + 8;
        let mut out = Vec::new();
        for z in -reach..=reach {
            for y in -reach..=reach {
                for x in -reach..=reach {
                    let voxel = IVec3::new(x, y, z);
                    if ColourField::paintable(volume.sample_voxel(voxel)) {
                        out.push((voxel.as_vec3() * VOXEL, volume.colour().at(voxel)));
                    }
                }
            }
        }
        out
    }

    /// Whether each voxel `voxels` lists is in the shell, in the same order.
    fn shell_of(volume: &Volume) -> Vec<bool> {
        voxels(volume)
            .iter()
            .map(|(at, _)| volume.sample_voxel((*at / VOXEL).round().as_ivec3()).abs() <= SHELL)
            .collect()
    }

    /// The middle of the slot's floor, the middle of the top face to one
    /// side of the slot, and the corner where the floor meets a wall.
    const ON_THE_FLOOR: Vec3 = Vec3::new(0.0, 0.0, FLOOR);
    const ON_THE_TOP: Vec3 = Vec3::new(-8.0, 0.0, HALF);
    const IN_THE_CORNER: Vec3 = Vec3::new(-SLOT_HALF, 0.0, FLOOR);

    /// How far the band reaches either side of a face, in millimetres.
    const BAND: f32 = NARROW_BAND * VOXEL;

    /// With nothing to stop it a fill takes the whole body, and with a limit
    /// it takes that many voxels and says it was stopped.
    #[test]
    fn a_fill_with_nothing_to_stop_it_takes_the_body_or_its_limit() {
        let mut volume = slotted_cube();
        let all = voxels(&volume).len();
        let filled = volume.fill_paint(ON_THE_TOP, fill(4));
        assert_eq!(filled, Filled { voxels: all, reached_the_limit: false });
        assert!(voxels(&volume).iter().all(|(_, slot)| *slot == 4));

        // The limit is on the shell, which is what the flood costs, and the
        // band under what was reached is painted with it.
        let mut volume = slotted_cube();
        let filled = volume.fill_paint(ON_THE_TOP, Fill { limit: 1000, ..fill(4) });
        assert!(filled.reached_the_limit, "a fill of a thousand took the whole body");
        assert!(
            filled.voxels >= 1000 && filled.voxels < 4000,
            "a shell of 1000 voxels painted {} through the band",
            filled.voxels
        );
        let on_the_shell = voxels(&volume)
            .iter()
            .zip(shell_of(&volume))
            .filter(|((_, slot), shell)| *slot == 4 && *shell)
            .count();
        assert_eq!(on_the_shell, 1000, "the limit is not a count of the shell");
        let painted = voxels(&volume).iter().filter(|(_, slot)| *slot == 4).count();
        assert_eq!(painted, filled.voxels, "what was reached before the limit was not kept");
    }

    /// Asked to stop at creases and edges, a fill poured on the slot's floor
    /// paints the floor: up to the two walls, out to the two ends, and not
    /// up a wall, onto the top, or under either through the band.
    #[test]
    fn a_fill_stops_at_the_lines_the_model_has() {
        let mut volume = slotted_cube();
        let bounded = Fill { stop_at_creases: Some(0.3), stop_at_edges: Some(0.3), ..fill(4) };
        let filled = volume.fill_paint(ON_THE_FLOOR, bounded);
        assert!(!filled.reached_the_limit);
        let painted: Vec<Vec3> =
            voxels(&volume).into_iter().filter(|(_, slot)| *slot == 4).map(|(at, _)| at).collect();
        assert_eq!(painted.len(), filled.voxels);
        // The floor is 6 mm by 24 mm: 12 by 48 voxels, less the lines it
        // stops short of, through a band 6 deep.
        assert!(painted.len() > 1500, "the floor took only {} voxels", painted.len());
        // The floor, the band above and below it, and no more than
        // `INTO_THE_LINE` voxels into the lines round it: so nothing up a
        // wall, and nothing down an end face.
        let into = INTO_THE_LINE as f32 * VOXEL;
        for at in &painted {
            assert!(at.x.abs() <= SLOT_HALF + BAND, "{at} is past a wall");
            assert!(at.z < FLOOR + SHELL * VOXEL + into + 0.01, "{at} is up a wall");
            assert!(at.z > FLOOR - BAND, "{at} is below the floor's band");
            assert!(at.y.abs() <= HALF + BAND, "{at} is past the end of the slot");
            // Down an end face: beyond the end and below the floor's line.
            assert!(
                !(at.y.abs() > HALF - 0.01 && at.z < FLOOR - SHELL * VOXEL - into - 0.01),
                "{at} is down an end face"
            );
        }
        // It did reach from wall to wall and from end to end, less a
        // couple of voxels at each line.
        let reach = |pick: fn(&Vec3) -> f32| {
            painted
                .iter()
                .map(pick)
                .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
        };
        let (x, y) = (reach(|at| at.x), reach(|at| at.y));
        assert!(x.0 < -SLOT_HALF + 1.5 && x.1 > SLOT_HALF - 1.5, "x only {x:?}");
        assert!(y.0 < -HALF + 1.5 && y.1 > HALF - 1.5, "y only {y:?}");

        // The control: the same pour with nothing asked runs everywhere.
        let mut open = slotted_cube();
        let everywhere = open.fill_paint(ON_THE_FLOOR, fill(4));
        assert!(everywhere.voxels > painted.len() * 10, "the unbounded control did not spread");
    }

    /// What counts as a line is the caller's gain. At a tenth of it the
    /// cube's edges and the slot's corners are a tenth as sharp, nothing is
    /// a line, and the pour that painted the floor paints the body.
    #[test]
    fn the_gain_decides_what_is_sharp_enough_to_stop_at() {
        let bounded = Fill { stop_at_creases: Some(0.3), stop_at_edges: Some(0.3), ..fill(4) };
        let mut volume = slotted_cube();
        let all = voxels(&volume).len();
        let floor = volume.fill_paint(ON_THE_FLOOR, bounded).voxels;
        assert!(floor * 10 < all, "the floor is {floor} of {all} voxels");

        let mut volume = slotted_cube();
        let dull = volume.fill_paint(ON_THE_FLOOR, Fill { crease_gain: 0.1, ..bounded });
        assert_eq!(dull.voxels, all, "at a tenth of the gain something still stopped the fill");
    }

    /// The paint that goes on into a line stays in the line, however far it
    /// is told to go: six voxels into the corner's line is still the corner
    /// and no way up the wall.
    #[test]
    fn going_further_into_a_line_never_leaves_it() {
        let deep = Fill {
            stop_at_creases: Some(0.3),
            stop_at_edges: Some(0.3),
            into_the_line: 6,
            ..fill(4)
        };
        let mut volume = slotted_cube();
        let further = volume.fill_paint(ON_THE_FLOOR, deep).voxels;
        let highest = voxels(&volume)
            .iter()
            .zip(shell_of(&volume))
            // On the shell, which is where paint shows. A voxel deeper in
            // the band takes the answer of whichever shell voxel it is
            // nearest, and beside a line that can be one a step along.
            .filter(|(_, on_the_shell)| *on_the_shell)
            .map(|(voxel, _)| voxel)
            // Away from the slot's ends. There the corner's line meets the
            // edges of the end faces, which are lines too, and paint told to
            // go six voxels into a line does go six voxels along those:
            // measured, to z = 9.5. That is the line, and is why two is the
            // default.
            .filter(|(at, slot)| {
                *slot == 4 && at.x.abs() > SLOT_HALF - BAND && at.y.abs() < HALF / 2.0
            })
            .map(|(at, _)| at.z)
            .fold(f32::MIN, f32::max);
        // The line is the corner's voxel and one each side of it. Six rings
        // let loose would be three millimetres up the wall.
        assert!(
            highest < FLOOR + 1.5,
            "paint going into the line climbed the wall to z = {highest} from a floor at {FLOOR}"
        );
        // And it is more than none: the control that the setting is read.
        let mut volume = slotted_cube();
        let none = volume.fill_paint(ON_THE_FLOOR, Fill { into_the_line: 0, ..deep }).voxels;
        assert!(further > none, "going into the line painted nothing more ({further}, {none})");
    }

    /// Creases alone do not bound the floor: the fill goes out of the ends
    /// of the slot, over the cube's edges and round. And edges alone do not
    /// either: it climbs the walls from the floor. Each stop is its own.
    #[test]
    fn creases_and_edges_are_asked_for_separately() {
        let on_top = |volume: &Volume| {
            voxels(volume)
                .iter()
                .filter(|(at, slot)| *slot == 4 && at.z > HALF - BAND && at.x < -SLOT_HALF - 2.0)
                .count()
        };
        let on_a_wall = |volume: &Volume| {
            voxels(volume)
                .iter()
                .filter(|(at, slot)| {
                    *slot == 4
                        && (at.x + SLOT_HALF).abs() < BAND
                        && at.z > FLOOR + 2.0
                        && at.z < HALF - 2.0
                })
                .count()
        };

        let mut creases = slotted_cube();
        creases.fill_paint(ON_THE_FLOOR, Fill { stop_at_creases: Some(0.3), ..fill(4) });
        assert!(on_top(&creases) > 500, "with edges open the fill did not get round to the top");

        let mut edges = slotted_cube();
        edges.fill_paint(ON_THE_FLOOR, Fill { stop_at_edges: Some(0.3), ..fill(4) });
        assert!(on_a_wall(&edges) > 200, "with creases open the fill did not climb the wall");
        assert_eq!(on_top(&edges), 0, "with edges stopping it the fill got onto the top");
    }

    /// A fill that starts in a groove stays in the groove and runs its
    /// length: panel lining.
    #[test]
    fn a_fill_begun_in_a_crease_lines_the_crease() {
        let mut volume = slotted_cube();
        let lining = Fill { stop_at_creases: Some(0.3), ..fill(7) };
        let filled = volume.fill_paint(IN_THE_CORNER, lining);
        let painted: Vec<Vec3> =
            voxels(&volume).into_iter().filter(|(_, slot)| *slot == 7).map(|(at, _)| at).collect();
        assert_eq!(painted.len(), filled.voxels);
        assert!(painted.len() > 100, "the line took only {} voxels", painted.len());
        for at in &painted {
            let from_the_line = Vec3::new(at.x + SLOT_HALF, 0.0, at.z - FLOOR).length();
            assert!(from_the_line < BAND + VOXEL, "{at} is {from_the_line} mm from the corner");
        }
        let ends = painted
            .iter()
            .map(|at| at.y)
            .fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
        assert!(ends.0 < -HALF + 2.5 && ends.1 > HALF - 2.5, "the line ran only {ends:?}");
        // The other corner is the same kind of line and is not this line.
        assert!(painted.iter().all(|at| at.x < 0.0), "the fill jumped to the other corner");
    }

    /// A fill replaces the paint it started on and spreads through that
    /// paint only. Poured where the paint already is, it does nothing.
    #[test]
    fn a_fill_spreads_through_the_paint_it_started_on_and_no_other() {
        let mut volume = slotted_cube();
        volume.edit_colour(
            IVec3::splat(-40),
            IVec3::splat(40),
            |_, at, _, _| {
                if at.y < 0.0 { 2 } else { 0 }
            },
        );
        let before = voxels(&volume);
        let filled = volume.fill_paint(Vec3::new(-8.0, 6.0, HALF), fill(3));
        let after = voxels(&volume);
        let mut changed = 0;
        for ((at, was), (_, now)) in before.iter().zip(&after) {
            if at.y < 0.0 {
                assert_eq!((*was, *now), (2, 2), "the fill crossed into another paint at {at}");
            } else {
                assert_eq!((*was, *now), (0, 3), "the fill missed {at}");
                changed += 1;
            }
        }
        assert_eq!(filled.voxels, changed);

        assert_eq!(volume.fill_paint(Vec3::new(-8.0, 6.0, HALF), fill(3)), Filled::NOTHING);
        // And slot 0 strips an area back, on the same rules.
        let stripped = volume.fill_paint(Vec3::new(-8.0, 6.0, HALF), fill(0));
        assert_eq!(stripped.voxels, changed);
        assert_eq!(voxels(&volume), before);
    }

    /// The mask stops a fill, a protected seed fills nothing, and so does a
    /// seed that is not on the model at all.
    #[test]
    fn a_fill_respects_the_mask_and_needs_a_surface_to_start_on() {
        let mut volume = slotted_cube();
        volume.edit_mask(IVec3::splat(-40), IVec3::splat(40), |_, at, _| {
            if at.y < 0.0 { PROTECTED } else { 0 }
        });
        let filled = volume.fill_paint(Vec3::new(-8.0, 6.0, HALF), fill(3));
        assert!(filled.voxels > 5000);
        for (at, slot) in voxels(&volume) {
            assert_eq!(slot == 3, at.y >= 0.0, "at {at}");
        }
        assert_eq!(volume.fill_paint(Vec3::new(-8.0, -6.0, HALF), fill(5)), Filled::NOTHING);
        assert_eq!(volume.fill_paint(Vec3::splat(40.0), fill(5)), Filled::NOTHING);
        assert_eq!(volume.fill_paint(Vec3::ZERO, fill(5)), Filled::NOTHING, "deep inside");
    }

    /// A smooth ball has no lines on it, so a fill asked to stop at lines
    /// takes all of it: the control for every "stops" above. A measure that
    /// saw lines in the lattice's own grain would leave it in patches.
    #[test]
    fn a_ball_has_no_lines_and_a_bounded_fill_takes_all_of_it() {
        let mut volume = Volume::new(VOXEL);
        volume.seed_sphere(Vec3::ZERO, 12.0);
        let bounded = Fill { stop_at_creases: Some(0.3), stop_at_edges: Some(0.3), ..fill(4) };
        let filled = volume.fill_paint(Vec3::new(0.0, 0.0, 12.0), bounded);
        assert!(!filled.reached_the_limit);
        let all = voxels(&volume);
        let bare = all.iter().filter(|(_, slot)| *slot != 4).count();
        assert!(all.len() > 40_000, "the ball is only {} voxels", all.len());
        assert_eq!(bare, 0, "{bare} of {} voxels of a smooth ball were left bare", all.len());
        assert_eq!(filled.voxels, all.len());
    }

    /// The field is not touched, as by every paint operation.
    #[test]
    fn a_fill_never_moves_the_field() {
        let untouched = slotted_cube();
        let mut volume = slotted_cube();
        volume.fill_paint(
            ON_THE_FLOOR,
            Fill { stop_at_creases: Some(0.3), stop_at_edges: Some(0.3), ..fill(4) },
        );
        crate::testing::assert_same_field(&volume, &untouched, "a fill moved the field");
    }

    const RED: [u8; 3] = [200, 30, 20];

    fn paint(colour: [u8; 3], opacity: f32) -> Pigment {
        Pigment { colour, opacity, mode: crate::paint::stroke::Mode::Paint }
    }

    /// Every paintable voxel of the fixture, with where it is and its
    /// paint.
    fn paints(volume: &Volume) -> Vec<(Vec3, Rgba)> {
        voxels(volume)
            .iter()
            .map(|(at, _)| (*at, volume.paint().at((*at / VOXEL).round().as_ivec3())))
            .collect()
    }

    /// A fill of colour runs exactly where a fill of a slot runs, on a body
    /// with nothing painted on it: the same flood, laying something else.
    #[test]
    fn a_fill_of_colour_goes_where_a_fill_of_a_slot_goes() {
        for (seed, creases, edges) in [
            (ON_THE_FLOOR, Some(0.3), Some(0.3)),
            (ON_THE_TOP, Some(0.3), None),
            (IN_THE_CORNER, Some(0.3), Some(0.3)),
            (ON_THE_TOP, None, None),
        ] {
            let lines = Fill { stop_at_creases: creases, stop_at_edges: edges, ..fill(4) };
            let mut by_slot = slotted_cube();
            let poured = by_slot.fill_paint(seed, lines);
            let mut by_colour = slotted_cube();
            let coloured = by_colour.fill_colour(seed, paint(RED, 1.0), 0, lines);
            assert_eq!(coloured, poured, "from {seed}");
            assert!(poured.voxels > 100, "the fixture filled {} voxels", poured.voxels);
            for ((at, slot), (_, rgba)) in voxels(&by_slot).iter().zip(paints(&by_colour)) {
                let wanted = if *slot == 4 { [200, 30, 20, 255] } else { [0; 4] };
                assert_eq!(rgba, wanted, "at {at}, from {seed}");
            }
            // It moved no clay and painted no slot.
            assert!(voxels(&by_colour).iter().all(|(_, slot)| *slot == 0));
        }
    }

    /// It spreads over paint like the paint it began on and stops at paint
    /// that is not, by as much as it is told is alike.
    #[test]
    fn a_fill_of_colour_spreads_over_paint_like_where_it_began() {
        const GREEN: Rgba = [30, 160, 40, 255];
        const BLUE: Rgba = [20, 40, 210, 255];
        let two = || {
            let mut volume = slotted_cube();
            volume.edit_paint(IVec3::splat(-40), IVec3::splat(40), |_, at, _, _| {
                if at.y < 0.0 { GREEN } else { BLUE }
            });
            volume
        };
        let from_green = Vec3::new(-8.0, -6.0, HALF);
        let mut volume = two();
        let filled = volume.fill_colour(from_green, paint(RED, 1.0), 10, fill(0));
        assert!(filled.voxels > 5000, "{filled:?}");
        for (at, rgba) in paints(&volume) {
            let wanted = if at.y < 0.0 { [200, 30, 20, 255] } else { BLUE };
            assert_eq!(rgba, wanted, "at {at}");
        }
        // Told that anything is alike, it takes everything.
        let mut volume = two();
        volume.fill_colour(from_green, paint(RED, 1.0), 255, fill(0));
        assert!(paints(&volume).iter().all(|(_, rgba)| *rgba == [200, 30, 20, 255]));

        // Across a shade that changes evenly, it goes as far as the shade
        // is alike and no further: a level a voxel along x, from 100.
        let mut volume = slotted_cube();
        volume.edit_paint(IVec3::splat(-40), IVec3::splat(40), |voxel, _, _, _| {
            [(100 + voxel.x) as u8, 50, 50, 255]
        });
        // From the slot's floor, where x is nothing.
        volume.fill_colour(ON_THE_FLOOR, paint(RED, 1.0), 6, fill(0));
        for (at, rgba) in paints(&volume) {
            let along = (at.x / VOXEL).round() as i32;
            let wanted = if along.abs() <= 6 {
                [200, 30, 20, 255]
            } else {
                [(100 + along) as u8, 50, 50, 255]
            };
            assert_eq!(rgba, wanted, "at {at}");
        }
    }

    /// What it reaches takes the pigment as a stroke at full strength would
    /// lay it: by its opacity, in its mode, and less what the mask holds
    /// off. And it is one step of the undo history.
    #[test]
    fn a_fill_of_colour_lays_the_pigment_as_a_stroke_would() {
        use crate::paint::stroke::Mode;
        let mut volume = slotted_cube();
        volume.begin_stroke();
        let half = volume.fill_colour(ON_THE_TOP, paint(RED, 0.5), 0, fill(0));
        let edit = volume.end_stroke().expect("a fill that painted");
        assert!(half.voxels > 10_000);
        assert!(paints(&volume).iter().all(|(_, rgba)| *rgba == [200, 30, 20, 128]));
        // A full coat of grey over that covers it, the red and the clay
        // the red left bare alike; an eraser takes all of it off again.
        let grey = Pigment { colour: [128, 128, 128], opacity: 1.0, mode: Mode::Paint };
        volume.fill_colour(ON_THE_TOP, grey, 0, fill(0));
        assert!(paints(&volume).iter().all(|(_, rgba)| *rgba == [128, 128, 128, 255]));
        let eraser = Pigment { colour: RED, opacity: 1.0, mode: Mode::Erase };
        volume.fill_colour(ON_THE_TOP, eraser, 0, fill(0));
        assert!(
            paints(&volume).iter().all(|(_, rgba)| *rgba == [0; 4]),
            "an eraser poured over everything left paint"
        );
        volume.apply_edit(edit);

        // Under a mask: nothing where it is half protected or more, and
        // less where it is protected less than that.
        let mut volume = slotted_cube();
        volume.edit_mask(IVec3::splat(-40), IVec3::splat(40), |_, at, _| {
            if at.x < -4.0 {
                PROTECTED
            } else if at.x < 4.0 {
                51
            } else {
                0
            }
        });
        volume.fill_colour(Vec3::new(8.0, 0.0, HALF), paint(RED, 1.0), 0, fill(0));
        let (mut held, mut part, mut free) = (0, 0, 0);
        for (at, rgba) in paints(&volume) {
            if at.x < -4.0 {
                assert_eq!(rgba, [0; 4], "paint under the mask at {at}");
                held += 1;
            } else if at.x < 4.0 {
                assert_eq!(rgba, [200, 30, 20, 204], "a fifth protected at {at}");
                part += 1;
            } else {
                assert_eq!(rgba, [200, 30, 20, 255], "free at {at}");
                free += 1;
            }
        }
        assert!(held > 1000 && part > 1000 && free > 1000, "{held} {part} {free}");
    }

    /// A fill of colour that has nothing to do does nothing: off the
    /// model, under the mask, or where the pigment would change nothing.
    #[test]
    fn a_fill_of_colour_with_nothing_to_do_does_nothing() {
        let mut volume = slotted_cube();
        let red = paint(RED, 1.0);
        assert_eq!(volume.fill_colour(Vec3::splat(40.0), red, 0, fill(0)), Filled::NOTHING);
        assert_eq!(volume.fill_colour(Vec3::ZERO, red, 0, fill(0)), Filled::NOTHING);
        assert!(volume.paint().is_empty());
        volume.fill_colour(ON_THE_TOP, red, 0, fill(0));
        // Red over red changes nothing, and is not done.
        volume.begin_stroke();
        assert_eq!(volume.fill_colour(ON_THE_TOP, red, 0, fill(0)), Filled::NOTHING);
        assert!(volume.end_stroke().is_none());
        volume.mask_mut().write((ON_THE_TOP / VOXEL).round().as_ivec3(), PROTECTED);
        let blue = paint([20, 40, 210], 1.0);
        assert_eq!(volume.fill_colour(ON_THE_TOP, blue, 0, fill(0)), Filled::NOTHING);
    }
}
