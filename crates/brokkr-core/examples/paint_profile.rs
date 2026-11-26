// SPDX-License-Identifier: AGPL-3.0-only

//! What a paint stamp costs and what its remesh costs, at three brush sizes
//! on a ball as fine as a figure is worked at:
//!
//! ```text
//! cargo run --release -p brokkr-core --example paint_profile
//! RAYON_NUM_THREADS=1 cargo run --release -p brokkr-core --example paint_profile
//! ```
//!
//! The remesh is a cost paint does not need: paint changes no shape, and
//! the surface is meshed again only to carry the new colours. See
//! `docs/PERFORMANCE.md`.

use std::time::Instant;

use brokkr_core::paint::stroke::{Lay, Mode, PaintStroke, Pigment};
use brokkr_core::{BrickMesh, Brush, BrushDirection, BrushKind, Stamp, Volume};
use glam::Vec3;
fn main() {
    let mut volume = Volume::new(0.0625);
    volume.seed_sphere(Vec3::ZERO, 12.0);
    let mut dirty = Vec::new();
    volume.take_dirty(&mut dirty);
    let mut meshes: Vec<BrickMesh> = Vec::new();
    for radius in [2.0_f32, 5.0, 10.0] {
        let brush = Brush { kind: BrushKind::Paint, radius, ..Brush::default() };
        let pigment = Pigment { colour: [180, 40, 30], opacity: 0.8, mode: Mode::Paint };
        let mut stroke = PaintStroke::new();
        volume.begin_stroke();
        stroke.begin();
        let (mut stamp_ms, mut mesh_ms, mut bricks) = (0.0, 0.0, 0);
        let n = 24;
        for step in 0..n {
            let turn = step as f32 / n as f32 * 1.5;
            let normal = Vec3::new(turn.cos(), turn.sin(), 0.3).normalize();
            let began = Instant::now();
            stroke.stamp(
                &mut volume,
                &brush,
                &Stamp::new(normal * 12.0, normal, BrushDirection::Add),
                &Lay::loaded(pigment),
            );
            stamp_ms += began.elapsed().as_secs_f64() * 1000.0;
            volume.take_dirty(&mut dirty);
            while meshes.len() < dirty.len() {
                meshes.push(BrickMesh::default());
            }
            let began = Instant::now();
            volume.mesh_bricks(&dirty, &mut meshes[..dirty.len()]);
            mesh_ms += began.elapsed().as_secs_f64() * 1000.0;
            bricks += dirty.len();
        }
        drop(volume.end_stroke());
        println!(
            "radius {radius} mm: stamp {:.2} ms, remesh {:.2} ms, {:.1} bricks a stamp",
            stamp_ms / n as f64,
            mesh_ms / n as f64,
            bricks as f64 / n as f64
        );
    }
}
