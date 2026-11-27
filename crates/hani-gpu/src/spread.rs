// SPDX-License-Identifier: AGPL-3.0-only
//
// How far each body is drawn from where it is: the hierarchy's Explode, which
// pushes the parts of a model apart on screen so that a small one buried
// between larger ones can be seen and picked. A VIEW offset and nothing else:
// the document's field never moves, and a body with no entry here is drawn
// exactly where it is.
//
// One entry per body in a small vertex buffer read per INSTANCE, so a body's
// draws pick their offset by the instance number they are issued with and the
// mesh itself is never touched. Entry 0 is always zero: every body that is not
// spread, and every thumbnail, draws with instance 0.
//
// The fourth component, which was padding, is how much of the body is
// screened away: the hierarchy's "others see-through", which draws every
// body but the active one translucent with the ring cut's 4x4 dither.
// Nought draws the body whole.
//
// And a body being moved by the transform gizmo is drawn where the drag has
// it, turned and scaled as well as moved, before its field is rebuilt
// there on the release (`app.rs`, `transform_to`). Asked for by the
// maintainer on 2026-11-27: "no kind of realtime update to see them being
// rotated / moved". So an entry carries a linear part too: three columns,
// stored as their difference from the identity so that a zeroed entry, as
// entry 0 and every unwritten one are, is still "drawn where it is".

use glam::{Affine3A, Mat3, Mat4, Vec3, Vec4};
use hani_core::NodeId;

/// Bytes per entry: the three columns of the linear part less the
/// identity's, each a `vec4` with nothing in `w`, and the offset with the
/// share screened away in `w`.
pub const SPREAD_BYTES: u64 = 64;

/// The offsets, and the buffer the shader reads them from.
#[derive(Debug)]
pub struct Spread {
    buffer: wgpu::Buffer,
    /// Body, and its offset with the share screened away in `w`: Explode's.
    /// At most `MAX_BODIES`, and almost always empty.
    entries: Vec<(NodeId, Vec4)>,
    /// The body the gizmo is dragging, and where the drag has it drawn.
    moving: Option<(NodeId, Mat4)>,
    /// What is in the buffer. A body's entry is its index here plus one.
    written: Vec<Entry>,
}

impl Spread {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hani spread"),
            size: (hani_core::MAX_BODIES as u64 + 1) * SPREAD_BYTES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            // Zeroed by wgpu, which is entry 0 and every entry not yet
            // written: no offset, and a linear part no different from the
            // identity's.
            mapped_at_creation: false,
        });
        Self {
            buffer,
            entries: Vec::with_capacity(hani_core::MAX_BODIES),
            moving: None,
            written: Vec::with_capacity(hani_core::MAX_BODIES),
        }
    }

    /// Replace every offset. Writes the buffer only when something changed, so
    /// a caller may publish the same set every frame for nothing.
    pub fn set(&mut self, queue: &wgpu::Queue, entries: &[(NodeId, Vec4)]) {
        debug_assert!(entries.len() <= hani_core::MAX_BODIES, "more offsets than bodies");
        if self.entries == entries {
            return;
        }
        self.entries.clear();
        self.entries.extend_from_slice(entries);
        self.write(queue);
    }

    /// Draw `body` through `map` as well as its offset, or no body so.
    pub fn set_moving(&mut self, queue: &wgpu::Queue, moving: Option<(NodeId, Mat4)>) {
        if self.moving == moving {
            return;
        }
        self.moving = moving;
        self.write(queue);
    }

    fn write(&mut self, queue: &wgpu::Queue) {
        self.written = combined(&self.entries, self.moving);
        if self.written.is_empty() {
            return;
        }
        queue.write_buffer(&self.buffer, SPREAD_BYTES, &bytes_of(&self.written));
    }

    /// The instance a body's draws are issued with, and the map that
    /// instance draws it through: `(0, IDENTITY)` for a body that is drawn
    /// where it is.
    pub fn of(&self, body: NodeId) -> (u32, Affine3A) {
        self.written
            .iter()
            .position(|entry| entry.body == body)
            .map_or((0, Affine3A::IDENTITY), |index| (index as u32 + 1, self.written[index].map))
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

/// One body drawn anywhere but where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Entry {
    body: NodeId,
    /// Where it is drawn: the gizmo's map, then Explode's offset.
    map: Affine3A,
    /// Explode's offset alone. The shader takes it back off for the
    /// body's world position, so that the overhang tint and the ring cut's
    /// see-through read a part where the gizmo has it and not where
    /// Explode draws it.
    explode: Vec3,
    /// The share screened away.
    screened: f32,
}

/// Every body that is drawn anywhere but where it is: Explode's offset after
/// the gizmo's map, and the share screened away.
fn combined(entries: &[(NodeId, Vec4)], moving: Option<(NodeId, Mat4)>) -> Vec<Entry> {
    let mut out: Vec<Entry> = entries
        .iter()
        .map(|(body, entry)| Entry {
            body: *body,
            map: Affine3A::from_translation(entry.truncate()),
            explode: entry.truncate(),
            screened: entry.w,
        })
        .collect();
    if let Some((body, map)) = moving {
        let map = Affine3A::from_mat4(map);
        match out.iter_mut().find(|entry| entry.body == body) {
            Some(entry) => entry.map *= map,
            None => out.push(Entry { body, map, explode: Vec3::ZERO, screened: 0.0 }),
        }
    }
    out
}

/// The buffer's bytes for these entries, from entry 1: three columns of
/// the linear part less the identity's, Explode's offset in their `w`, and
/// the whole offset with the share screened away.
fn bytes_of(written: &[Entry]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(written.len() * SPREAD_BYTES as usize);
    for entry in written {
        let linear = Mat3::from(entry.map.matrix3) - Mat3::IDENTITY;
        let columns = [
            linear.x_axis.extend(entry.explode.x),
            linear.y_axis.extend(entry.explode.y),
            linear.z_axis.extend(entry.explode.z),
            Vec3::from(entry.map.translation).extend(entry.screened),
        ];
        for column in columns {
            for value in column.to_array() {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    bytes
}

/// The box a body's box is drawn in, through `map`: the box round its eight
/// corners, so a frustum test on it is never too tight.
pub fn drawn_box(map: &Affine3A, low: Vec3, high: Vec3) -> (Vec3, Vec3) {
    let mut out = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for corner in 0..8_u32 {
        let at = Vec3::new(
            if corner & 1 == 0 { low.x } else { high.x },
            if corner & 2 == 0 { low.y } else { high.y },
            if corner & 4 == 0 { low.z } else { high.z },
        );
        let drawn = map.transform_point3(at);
        out = (out.0.min(drawn), out.1.max(drawn));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn body(n: u32) -> NodeId {
        NodeId(n)
    }

    /// An entry left zeroed is the identity; a written one carries its map
    /// less the identity's, and its offset and its screen; Explode's offset
    /// is applied after the gizmo's map, to the same entry.
    #[test]
    fn an_entry_is_its_map_less_the_identity() {
        assert!(bytes_of(&combined(&[], None)).is_empty());
        let turned = Mat4::from_rotation_translation(Quat::from_rotation_z(0.5), Vec3::X);
        let written =
            combined(&[(body(3), Vec4::new(0.0, 2.0, 0.0, 0.25))], Some((body(3), turned)));
        assert_eq!(written.len(), 1);
        let Entry { map, screened, explode, .. } = written[0];
        assert_eq!(screened, 0.25);
        assert_eq!(explode, Vec3::new(0.0, 2.0, 0.0));
        let at = Vec3::new(1.0, 1.0, 1.0);
        let want = turned.transform_point3(at) + Vec3::new(0.0, 2.0, 0.0);
        assert!(map.transform_point3(at).distance(want) < 1.0e-5);

        let floats: Vec<f32> = bytes_of(&written)
            .chunks(4)
            .map(|four| f32::from_le_bytes(four.try_into().unwrap()))
            .collect();
        assert_eq!(floats.len(), 16);
        let linear = Mat3::from_cols_slice(&[
            floats[0], floats[1], floats[2], floats[4], floats[5], floats[6], floats[8], floats[9],
            floats[10],
        ]) + Mat3::IDENTITY;
        assert!(linear.abs_diff_eq(Mat3::from(map.matrix3), 1.0e-6));
        assert_eq!([floats[3], floats[7], floats[11]], [0.0, 2.0, 0.0], "Explode's offset");
        assert_eq!(&floats[12..16], &[1.0, 2.0, 0.0, 0.25]);

        // A body only moving gets an entry of its own, screened not at all.
        let alone = combined(&[], Some((body(4), Mat4::from_scale(Vec3::splat(2.0)))));
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[0].screened, 0.0);
        assert_eq!(alone[0].explode, Vec3::ZERO);
    }

    /// The box round a turned box holds every corner of it.
    #[test]
    fn a_drawn_box_holds_the_turned_box() {
        let map = Affine3A::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let (low, high) = drawn_box(&map, Vec3::splat(-1.0), Vec3::splat(1.0));
        let reach = std::f32::consts::SQRT_2;
        assert!((high.x - reach).abs() < 1.0e-5 && (low.y + reach).abs() < 1.0e-5);
        assert!((high.z - 1.0).abs() < 1.0e-6);
    }
}
