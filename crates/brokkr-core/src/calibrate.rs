// SPDX-License-Identifier: AGPL-3.0-only

//! How many worker threads this machine's work wants: a short sculpting
//! and painting workload timed with 1, 2, 4 and so on up to every thread
//! the machine has, and the fewest threads within a few parts in a hundred
//! of the fastest.
//!
//! Asked by the maintainer on 2026-11-26: "is there a way to test how many
//! cores we should use on a given PC". Every thread is not always best: a
//! second thread on a core that already has one (hyper-threading) gives
//! far less than a core, a small efficiency core less than a large one,
//! and on some machines the last few make the work slower. That cannot be
//! known from the count of threads, so it is measured.
//!
//! The workload is what a stroke spends its time on: a ring of sculpting
//! stamps, each remeshed, and a ring of paint stamps, on a ball as fine as
//! a figure is worked at. Each count of threads is a pool of its own
//! ([`rayon::ThreadPool::install`]), so the global pool is not touched. The
//! first run is a warm-up and is not counted.

use std::time::Instant;

use glam::Vec3;

use crate::brush::{Brush, BrushDirection, BrushKind, FalloffCurve, Stamp};
use crate::paint::stroke::{Lay, Mode, PaintStroke, Pigment};
use crate::{BrickMesh, BrushScratch, Pattern, Volume};

/// One count of threads and how long the workload took with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trial {
    pub threads: usize,
    pub millis: f32,
}

/// How much slower than the fastest a count of threads may be and still be
/// chosen for using fewer: three parts in a hundred, about what one run
/// differs from the next on a quiet machine.
const WITHIN: f32 = 1.03;

/// The counts worth trying on a machine with `most` threads: each power of
/// two below it, and itself.
pub fn counts(most: usize) -> Vec<usize> {
    let most = most.max(1);
    let mut counts: Vec<usize> =
        std::iter::successors(Some(1_usize), |n| Some(n * 2)).take_while(|n| *n < most).collect();
    counts.push(most);
    counts
}

/// The fewest threads within [`WITHIN`] of the fastest of `trials`.
pub fn recommend(trials: &[Trial]) -> Option<usize> {
    let best = trials.iter().map(|trial| trial.millis).fold(f32::INFINITY, f32::min);
    if !best.is_finite() {
        return None;
    }
    trials.iter().filter(|trial| trial.millis <= best * WITHIN).map(|trial| trial.threads).min()
}

/// The workload: a ball at a sixteenth of a millimetre, as a figure is
/// worked at, and somewhere to keep its meshes.
struct Workload {
    volume: Volume,
    scratch: BrushScratch,
    meshes: Vec<BrickMesh>,
    dirty: Vec<crate::brick::BrickCoord>,
    stroke: PaintStroke,
}

const VOXEL: f32 = 0.0625;
const BALL: f32 = 12.0;

impl Workload {
    fn new() -> Self {
        let mut volume = Volume::new(VOXEL);
        volume.seed_sphere(Vec3::ZERO, BALL);
        let mut dirty = Vec::new();
        volume.take_dirty(&mut dirty);
        Self {
            volume,
            scratch: BrushScratch::new(),
            meshes: Vec::new(),
            dirty,
            stroke: PaintStroke::new(),
        }
    }

    /// One round: sculpting stamps with their remesh, then paint stamps.
    /// `round` turns the ring a little each time, so that each round
    /// works on fresh surface.
    fn run(&mut self, round: usize) {
        let stamps = 16;
        let brush = Brush {
            kind: BrushKind::Draw,
            radius: 3.0,
            strength: 0.2,
            falloff: FalloffCurve::Smooth,
            pattern: Pattern::default(),
        };
        let tilt = 0.2 + 0.07 * round as f32;
        let on = |step: usize| {
            let turn = step as f32 / stamps as f32 * std::f32::consts::TAU;
            let normal = Vec3::new(turn.cos(), turn.sin(), tilt).normalize();
            Stamp::new(normal * BALL, normal, BrushDirection::Add)
        };
        self.volume.begin_stroke();
        for step in 0..stamps {
            brush.apply(&mut self.volume, &on(step), &mut self.scratch);
            self.volume.take_dirty(&mut self.dirty);
            while self.meshes.len() < self.dirty.len() {
                self.meshes.push(BrickMesh::default());
            }
            self.volume.mesh_bricks(&self.dirty, &mut self.meshes[..self.dirty.len()]);
        }
        drop(self.volume.end_stroke());

        let paint = Brush { kind: BrushKind::Paint, radius: 5.0, ..brush };
        let pigment = Pigment { colour: [180, 40, 30], opacity: 0.8, mode: Mode::Paint };
        self.volume.begin_stroke();
        self.stroke.begin();
        for step in 0..stamps {
            self.stroke.stamp(&mut self.volume, &paint, &on(step), &Lay::loaded(pigment));
        }
        drop(self.volume.end_stroke());
        self.volume.take_dirty(&mut self.dirty);
    }
}

/// Time the workload with each count of threads in `counts`, after a
/// warm-up with the last. Takes a few seconds; not for the window's
/// thread.
pub fn measure(counts: &[usize]) -> Vec<Trial> {
    let mut work = Workload::new();
    let mut round = 0;
    let mut timed = |threads: usize, work: &mut Workload| -> Option<f32> {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().ok()?;
        let began = Instant::now();
        pool.install(|| work.run(round));
        round += 1;
        Some(began.elapsed().as_secs_f32() * 1000.0)
    };
    if let Some(last) = counts.last() {
        let _ = timed(*last, &mut work);
    }
    counts
        .iter()
        .filter_map(|threads| {
            timed(*threads, &mut work).map(|millis| Trial { threads: *threads, millis })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counts_are_powers_of_two_and_the_most() {
        assert_eq!(counts(1), [1]);
        assert_eq!(counts(4), [1, 2, 4]);
        assert_eq!(counts(12), [1, 2, 4, 8, 12]);
        assert_eq!(counts(0), [1]);
    }

    /// The fewest threads within a few parts in a hundred of the fastest:
    /// more threads that buy nothing are not chosen.
    #[test]
    fn the_fewest_threads_near_the_fastest_are_chosen() {
        let trial = |threads, millis| Trial { threads, millis };
        let flat =
            [trial(1, 100.0), trial(2, 52.0), trial(4, 30.0), trial(8, 29.5), trial(16, 31.0)];
        assert_eq!(recommend(&flat), Some(4));
        let scaling = [trial(1, 100.0), trial(2, 50.0), trial(4, 25.0)];
        assert_eq!(recommend(&scaling), Some(4));
        let worse = [trial(1, 40.0), trial(2, 45.0)];
        assert_eq!(recommend(&worse), Some(1));
        assert_eq!(recommend(&[]), None);
    }

    /// The workload runs, and says something on this machine.
    #[test]
    fn the_workload_is_timed_for_each_count() {
        let trials = measure(&[1, 2]);
        assert_eq!(trials.iter().map(|trial| trial.threads).collect::<Vec<_>>(), [1, 2]);
        assert!(trials.iter().all(|trial| trial.millis > 0.0));
    }
}
