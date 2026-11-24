// SPDX-License-Identifier: AGPL-3.0-only

//! One stroke, over and over, for a profiler to look at.
//!
//! The benches say how long a stamp takes and not where in it the time
//! goes. This is the smallest program that spends its time the way a stroke
//! does: a ball, a ring of stamps round it, and a remesh after each, with
//! the timers of both printed. Run it under whatever profiler there is:
//!
//! ```text
//! cargo build --profile bench -p brokkr-core --example stroke_profile
//! valgrind --tool=callgrind target/release/examples/stroke_profile draw 12 1
//! callgrind_annotate callgrind.out.* | head -60
//! ```
//!
//! Arguments: the brush (draw, inflate, smooth, pinch, flatten, clay), its
//! radius in voxels, and how many times round. With no profiler it prints
//! milliseconds a stamp and a remesh, on every core and on one.

use std::time::Instant;

use brokkr_core::{
    BrickMesh, Brush, BrushDirection, BrushKind, BrushScratch, FalloffCurve, Pattern, Stamp, Volume,
};
use glam::Vec3;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let kind = match arguments.first().map(String::as_str).unwrap_or("draw") {
        "inflate" => BrushKind::Inflate,
        "smooth" => BrushKind::Smooth,
        "pinch" => BrushKind::Pinch,
        "flatten" => BrushKind::Flatten,
        "clay" => BrushKind::Clay,
        _ => BrushKind::Draw,
    };
    let radius_voxels: f32 = arguments.get(1).and_then(|text| text.parse().ok()).unwrap_or(48.0);
    let rounds: usize = arguments.get(2).and_then(|text| text.parse().ok()).unwrap_or(3);

    let voxel = 0.0625;
    let ball = 20.0;
    let mut volume = Volume::new(voxel);
    volume.seed_sphere(Vec3::ZERO, ball);
    let mut dirty = Vec::new();
    volume.take_dirty(&mut dirty);

    let brush = Brush {
        kind,
        radius: radius_voxels * voxel,
        strength: 0.5,
        falloff: FalloffCurve::Smooth,
        pattern: Pattern::default(),
    };
    let mut scratch = BrushScratch::new();
    let mut meshes: Vec<BrickMesh> = Vec::new();
    let stamps = 48;

    let (mut edit, mut mesh, mut meshed) = (0.0f64, 0.0f64, 0usize);
    volume.begin_stroke();
    for step in 0..stamps * rounds {
        let turn = step as f32 / stamps as f32 * std::f32::consts::TAU;
        let normal = Vec3::new(turn.cos(), turn.sin(), 0.35).normalize();
        let stamp = Stamp::new(normal * ball, normal, BrushDirection::Add);

        let began = Instant::now();
        brush.apply(&mut volume, &stamp, &mut scratch);
        edit += began.elapsed().as_secs_f64();

        volume.take_dirty(&mut dirty);
        while meshes.len() < dirty.len() {
            meshes.push(BrickMesh::default());
        }
        let began = Instant::now();
        volume.mesh_bricks(&dirty, &mut meshes[..dirty.len()]);
        mesh += began.elapsed().as_secs_f64();
        meshed += dirty.len();
    }
    let _ = volume.end_stroke();

    let count = (stamps * rounds) as f64;
    println!(
        "{kind}, radius {radius_voxels} voxels, {} stamps: edit {:.3} ms a stamp, remesh {:.3} ms \
         a stamp ({:.1} bricks, {:.1} us a brick)",
        stamps * rounds,
        edit * 1000.0 / count,
        mesh * 1000.0 / count,
        meshed as f64 / count,
        mesh * 1.0e6 / meshed.max(1) as f64,
    );
}
