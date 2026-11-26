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

use glam::Vec3;
use hani_core::NodeId;

/// Bytes per entry: a `vec3` padded to 16 so the stride is the one a uniform
/// array would use and the buffer can be read as `Float32x3`.
pub const SPREAD_BYTES: u64 = 16;

/// The offsets, and the buffer the shader reads them from.
#[derive(Debug)]
pub struct Spread {
    buffer: wgpu::Buffer,
    /// Body and offset; the body's entry in the buffer is its index here plus
    /// one. At most `MAX_BODIES`, and almost always empty.
    entries: Vec<(NodeId, Vec3)>,
}

impl Spread {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hani spread"),
            size: (hani_core::MAX_BODIES as u64 + 1) * SPREAD_BYTES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            // Zeroed by wgpu, which is entry 0 and every entry not yet written.
            mapped_at_creation: false,
        });
        Self { buffer, entries: Vec::with_capacity(hani_core::MAX_BODIES) }
    }

    /// Replace every offset. Writes the buffer only when something changed, so
    /// a caller may publish the same set every frame for nothing.
    pub fn set(&mut self, queue: &wgpu::Queue, entries: &[(NodeId, Vec3)]) {
        debug_assert!(entries.len() <= hani_core::MAX_BODIES, "more offsets than bodies");
        if self.entries == entries {
            return;
        }
        self.entries.clear();
        self.entries.extend_from_slice(entries);
        if entries.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(entries.len() * SPREAD_BYTES as usize);
        for (_, offset) in entries {
            for value in [offset.x, offset.y, offset.z, 0.0] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        queue.write_buffer(&self.buffer, SPREAD_BYTES, &bytes);
    }

    /// The instance a body's draws are issued with, and the offset that
    /// instance carries: `(0, ZERO)` for a body that is drawn where it is.
    pub fn of(&self, body: NodeId) -> (u32, Vec3) {
        self.entries
            .iter()
            .position(|(id, _)| *id == body)
            .map_or((0, Vec3::ZERO), |index| (index as u32 + 1, self.entries[index].1))
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}
