// SPDX-License-Identifier: AGPL-3.0-only

//! What routing a stamp through the brush engine costs against not doing so.
//!
//! `brush::engine` implements the six stamping brushes a second time, as
//! kernels behind one generic loop. Its tests hold it to the same field as
//! [`Brush::apply`], bit for bit. This holds it to the same time, because a
//! composition that is correct and slower would be paid for on every stamp of
//! every stroke, in the loop the whole project is budgeted around.
//!
//! **A gate, like the budget harness: it exits non-zero.** What it gates is a
//! ratio and not a duration. The budget harness already holds [`Brush::apply`]
//! to milliseconds on the machine those milliseconds were measured on; this
//! asks only that the engine is not slower than the path it stands beside, on
//! whatever machine runs it.
//!
//! Each row is the fastest of several rounds, and the two paths take turns
//! within a round. Contention only ever adds time, so the minimum is the
//! honest estimate of the work, and alternating keeps a busy minute from
//! landing on one path alone.
//!
//! Run with `cargo bench -p hani-core --bench engine`.

use std::time::Duration;
use web_time::Instant;

use glam::Vec3;
use hani_core::brush::engine::{BrushDef, FieldOp};
use hani_core::{Brush, BrushDirection, BrushKind, BrushScratch, Stamp, Volume};

/// Voxels across the model, matching the budget harness.
const EFFECTIVE_RESOLUTION: f32 = 256.0;

/// Brush radii in voxels: the 3 mm, 10 mm and 20 mm settings at the 0.25 mm
/// voxel the application ships, as in the budget harness.
const RADII: [f32; 3] = [12.0, 40.0, 80.0];

/// Stamps in one timed stroke.
const STAMPS: usize = 48;

/// Fewest timed strokes per path per row. The fastest counts.
const MIN_ROUNDS: usize = 5;

/// Most timed strokes per path per row.
const MAX_ROUNDS: usize = 60;

/// How long a row keeps taking rounds for, per path, once it has its minimum.
///
/// The small radii are where the noise is: a stamp of a hundred microseconds
/// is moved ten percent by one scheduler tick, and five rounds of it measured
/// the same code anywhere from 96 to 119 microseconds on a shared four core
/// machine. They are also the rows that cost nothing to repeat, so they are
/// repeated until the minimum has had this long to settle. The large radii
/// pass it in their first five rounds and are unaffected.
const SETTLE: Duration = Duration::from_millis(400);

/// How much slower than [`Brush::apply`] the engine may measure before this
/// fails.
///
/// Not zero, because two runs of the SAME code differ by a few percent on a
/// machine doing anything else, and a gate that fails on noise is a gate
/// people learn to rerun. Ten percent is outside what a settled minimum moves
/// by and well inside what a per-voxel indirect call or a lost inlining would
/// cost, which are the regressions this exists to catch.
const ALLOWED_RATIO: f64 = 1.10;

/// Below this a row is too quick for a ratio to mean anything, and the gate
/// allows this much on top instead.
const NOISE_FLOOR: Duration = Duration::from_micros(15);

fn kinds() -> [(BrushKind, FieldOp); 6] {
    [
        (BrushKind::Draw, FieldOp::Draw),
        (BrushKind::Clay, FieldOp::Clay),
        (BrushKind::Smooth, FieldOp::Smooth),
        (BrushKind::Inflate, FieldOp::Inflate),
        (BrushKind::Pinch, FieldOp::Pinch),
        (BrushKind::Flatten, FieldOp::Flatten),
    ]
}

/// One stroke round the equator of a fresh ball, returning the median time of
/// a stamp.
///
/// A fresh ball each time, so both paths and every round stamp exactly the
/// same field: the engine is bit-identical to the brush, so the second stamp
/// of one meets what the second stamp of the other met.
fn stroke(
    radius: f32,
    stamp_with: &mut dyn FnMut(&mut Volume, &Stamp, &mut BrushScratch),
) -> Duration {
    let centre = Vec3::splat(EFFECTIVE_RESOLUTION * 0.5);
    let ball = EFFECTIVE_RESOLUTION * 0.47;
    let mut volume = Volume::new(1.0);
    volume.seed_sphere(centre, ball);
    volume.take_dirty(&mut Vec::new());

    let mut scratch = BrushScratch::new();
    let mut times = Vec::with_capacity(STAMPS);
    volume.begin_stroke();
    for step in 0..STAMPS {
        // A quarter of the radius apart, which is the spacing a real stroke
        // lays its stamps at.
        let angle = step as f32 * (radius * 0.25) / ball;
        let outward = Vec3::new(angle.cos(), 0.3, angle.sin()).normalize();
        let stamp = Stamp::new(centre + outward * ball, outward, BrushDirection::Add);
        let started = Instant::now();
        stamp_with(&mut volume, &stamp, &mut scratch);
        times.push(started.elapsed());
    }
    // Ended and dropped outside the timing: the undo entry is the same either
    // way and is not what is being compared.
    drop(volume.end_stroke());

    times.sort();
    times[times.len() / 2]
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1.0e6
}

fn main() {
    println!(
        "HaniSculpt engine harness: {}^3 effective volume, {STAMPS} stamps a stroke, \
         fastest of {MIN_ROUNDS} to {MAX_ROUNDS} rounds",
        EFFECTIVE_RESOLUTION as u32
    );
    println!(
        "{:<9} {:>7} {:>12} {:>12} {:>8}",
        "brush", "radius", "brush us", "engine us", "ratio"
    );

    let mut failures = Vec::new();
    for (kind, op) in kinds() {
        for radius in RADII {
            let brush = Brush { kind, radius, strength: 0.5, ..Brush::default() };
            let def = BrushDef::from_legacy(&brush).expect("a stamping kind");
            assert_eq!(def.op, op, "the table above pairs a kind with the wrong operator");

            let mut old = Duration::MAX;
            let mut new = Duration::MAX;
            let mut spent = Duration::ZERO;
            for round in 0..MAX_ROUNDS {
                if round >= MIN_ROUNDS && spent >= SETTLE {
                    break;
                }
                let one = stroke(radius, &mut |v, s, scratch| brush.apply(v, s, scratch));
                let other = stroke(radius, &mut |v, s, scratch| def.apply(v, s, scratch));
                old = old.min(one);
                new = new.min(other);
                // The median of a stroke times its stamps is near enough what
                // the stroke took, and the brush's alone is the clock: it is
                // the path that is not under test.
                spent += one * STAMPS as u32;
            }

            let ratio = micros(new) / micros(old);
            println!(
                "{:<9} {:>7} {:>12.1} {:>12.1} {:>8.3}",
                kind.label(),
                radius,
                micros(old),
                micros(new),
                ratio
            );
            let allowed = old.mul_f64(ALLOWED_RATIO) + NOISE_FLOOR;
            if new > allowed {
                failures.push(format!(
                    "{} at radius {radius}: engine {:.1} us against {:.1} us, allowed {:.1}",
                    kind.label(),
                    micros(new),
                    micros(old),
                    micros(allowed)
                ));
            }
        }
    }

    if failures.is_empty() {
        println!("engine: within {ALLOWED_RATIO}x of the brush on every row");
    } else {
        for failure in &failures {
            eprintln!("OVER: {failure}");
        }
        std::process::exit(1);
    }
}
