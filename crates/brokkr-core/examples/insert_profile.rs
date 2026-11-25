// SPDX-License-Identifier: AGPL-3.0-only

//! A shape dragged out of a ball, over and over, for a profiler to look at.
//!
//! What a brush that adds does on every move of the pointer: the body put
//! back as it was at the press, and the shape joined to it at the size the
//! drag now gives it, with a remesh after. `stroke_profile` is the same for
//! a brush on the field.
//!
//! ```text
//! cargo build --profile bench -p brokkr-core --example insert_profile
//! valgrind --tool=callgrind target/release/examples/insert_profile ball 40 3 1
//! ```
//!
//! Arguments: the form (ball, box, cylinder, cone, ring), how far from its
//! middle to its edge at the end of the drag, in voxels, how soft the join
//! is, in voxels, how many drags, and `cut` to take the shape out and not
//! add it. With no profiler it prints milliseconds a move for the join and
//! for the remesh, and a sum of the field about the shape as the last move
//! left it: the same sum before and after a change to the join is the same
//! field, to the bit.

use std::time::Instant;

use brokkr_core::solid::{Form, How, Join, Placed, Solid};
use brokkr_core::{BrickMesh, FieldRegion, Volume};
use glam::{IVec3, Vec3};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let name = arguments.first().map(String::as_str).unwrap_or("ball");
    let form = match name {
        "box" => Form::Box,
        "cylinder" => Form::Cylinder,
        "cone" => Form::Cone,
        "ring" => Form::Torus,
        _ => Form::Ball,
    };
    let half_voxels: f32 = arguments.get(1).and_then(|text| text.parse().ok()).unwrap_or(40.0);
    let blend: f32 = arguments.get(2).and_then(|text| text.parse().ok()).unwrap_or(3.0);
    let drags: usize = arguments.get(3).and_then(|text| text.parse().ok()).unwrap_or(2);

    // The ball the application opens with, at half its voxel size: what
    // `what_setting_a_shape_down_costs` drags a shape out of.
    let voxel = 0.125;
    let ball = 20.0;
    let mut volume = Volume::new(voxel);
    volume.seed_sphere(Vec3::ZERO, ball);
    let mut dirty = Vec::new();
    volume.take_dirty(&mut dirty);

    let cut = arguments.get(4).is_some_and(|text| text == "cut");
    let how = How { cut, blend };
    let mut join = Join::new();
    let mut meshes: Vec<BrickMesh> = Vec::new();
    let moves = 48;
    let normal = Vec3::new(0.3, 0.5, 0.81).normalize();
    let toward = Vec3::X;

    let (mut set, mut mesh, mut meshed) = (0.0f64, 0.0f64, 0usize);
    let mut sum = 0xcbf2_9ce4_8422_2325_u64;
    for _ in 0..drags {
        volume.begin_stroke();
        join.begin();
        for step in 1..=moves {
            let half = half_voxels * voxel * step as f32 / moves as f32;
            let seat = Placed::on(form, normal * ball, normal, toward, half, 0.0);
            let solids: Vec<&dyn Solid> = seat.iter().map(|seat| seat as &dyn Solid).collect();

            let began = Instant::now();
            join.set(&mut volume, &solids, how);
            set += began.elapsed().as_secs_f64();

            volume.take_dirty(&mut dirty);
            while meshes.len() < dirty.len() {
                meshes.push(BrickMesh::default());
            }
            let began = Instant::now();
            volume.mesh_bricks(&dirty, &mut meshes[..dirty.len()]);
            mesh += began.elapsed().as_secs_f64();
            meshed += dirty.len();
        }
        // The field about the shape, every bit of it, into one number.
        let middle = (normal * ball / voxel).round().as_ivec3();
        // No more of it than a copy of the field will hold.
        let reach = IVec3::splat(((half_voxels * 1.8) as i32 + 40).min(180));
        let mut region = FieldRegion::new();
        volume.snapshot(middle - reach, middle + reach, &mut region);
        for z in -reach.z..=reach.z {
            for y in -reach.y..=reach.y {
                for x in -reach.x..=reach.x {
                    let bits = region.get(middle + IVec3::new(x, y, z)).to_bits();
                    sum = (sum ^ u64::from(bits)).wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        // The drag undone, so that the next begins on the ball as it was.
        join.clear(&mut volume);
        let _ = volume.end_stroke();
        volume.take_dirty(&mut dirty);
    }

    let count = (moves * drags) as f64;
    println!(
        "{name}, {half_voxels} voxels at the end, a join {blend} voxels soft, {} moves: set down \
         {:.3} ms a move, remesh {:.3} ms a move ({:.1} bricks, {:.1} us a brick), field {sum:016x}",
        moves * drags,
        set * 1000.0 / count,
        mesh * 1000.0 / count,
        meshed as f64 / count,
        mesh * 1.0e6 / meshed.max(1) as f64,
    );
}
