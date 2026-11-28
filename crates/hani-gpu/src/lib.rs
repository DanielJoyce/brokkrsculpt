// SPDX-License-Identifier: AGPL-3.0-only

//! GPU resources for HaniSculpt: the mesh buffer pool and the sculpt
//! renderer.
//!
//! Nothing here depends on a UI toolkit. The renderer takes a device, a queue
//! and a target format, and draws into whatever texture view it is handed, so
//! the Iced specific glue stays in `hani-app`.

pub mod frustum;
// Lights that can be set, and the surfaces they light. See `lights.rs`.
pub mod lights;
pub mod matcap;
pub mod mesh_pool;
pub mod overlay;
mod paints;
// Reference pictures behind the sculpt.
pub mod reference;
pub mod renderer;
pub mod spread;
// A boolean row tinted by what it does. See `tint.rs`.
pub mod thumbnail;
pub mod tint;

pub use frustum::Frustum;
pub use lights::{Light, LightBlock, MAX_LIGHTS, Material, Rig, Surface};
pub use mesh_pool::{
    INDEX_CAPACITY, MAX_BUFFERS, MaskPolarity, MeshPool, NodeId, PoolStats, SlotKey, THE_ONLY_BODY,
    TOTAL_INDEX_CAPACITY, TOTAL_VERTEX_CAPACITY, VERTEX_CAPACITY,
};
pub use overlay::{OverlayBatch, OverlayRenderer, OverlayVertex};
pub use paints::{PAINT_SLOTS, PaintBlock};
pub use reference::{
    MOST_PIXELS_ACROSS, REFERENCE_SLOTS, ReferencePicture, ReferenceQuad, ReferenceRenderer,
};
pub use renderer::{PALETTE_SLOTS, PixelRect, SculptRenderer, UNKNOWN_FILAMENT, Uniforms};
pub use thumbnail::{THUMBNAIL_BACKGROUND, THUMBNAIL_SIZE, ThumbnailAtlas, background_texel};
