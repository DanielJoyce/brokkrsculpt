// SPDX-License-Identifier: AGPL-3.0-only

//! Reference pictures on their planes, drawn behind the sculpt.
//!
//! # Why a pass of its own, before the sculpt's
//!
//! A reference is something to model against, so the model is always drawn
//! over it, wherever the plane stands: a picture set through the middle of a
//! head must not cut the head in two. So the pictures go first, in a pass
//! that clears depth and tests and writes it among themselves (two planes
//! that cross are drawn crossing), and then the sculpt's own pass clears
//! depth again and draws the model over whatever the pictures left. They
//! blend with the viewport's background by their opacity.
//!
//! One picture a plane at most, three slots. The pictures are decoded by the
//! application and arrive here as RGBA, at most [`MOST_PIXELS_ACROSS`] on a
//! side; this module keeps a texture per slot and a quad per slot a frame.

use glam::{Mat4, Vec3};

use crate::overlay::OverlayUniforms;

/// The largest picture side the application uploads.
///
/// 2048 is the side WebGPU and every desktop adapter guarantee, so a
/// picture this size uploads everywhere the viewport draws.
pub const MOST_PIXELS_ACROSS: u32 = 2048;

/// How many pictures can be drawn at once: one a plane.
pub const REFERENCE_SLOTS: usize = 3;

/// A decoded picture: `width * height` RGBA texels, sRGB, rows top first.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferencePicture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Where one slot's picture is drawn this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceQuad {
    pub slot: usize,
    /// The picture's corners: bottom left, bottom right, top right, top
    /// left, as the picture is seen from the view that faces it.
    pub corners: [Vec3; 4],
    pub opacity: f32,
    /// Drawn mirrored left to right.
    pub flipped: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ReferenceVertex {
    position: [f32; 3],
    uv: [f32; 2],
    opacity: f32,
}

/// The six vertices of one quad, two triangles, with the picture's texture
/// coordinates: v runs down the picture, so the top corners are v = 0.
fn quad_vertices(quad: &ReferenceQuad) -> [ReferenceVertex; 6] {
    let (left, right) = if quad.flipped { (1.0, 0.0) } else { (0.0, 1.0) };
    let uv = [[left, 1.0], [right, 1.0], [right, 0.0], [left, 0.0]];
    let corner = |at: usize| ReferenceVertex {
        position: quad.corners[at].to_array(),
        uv: uv[at],
        opacity: quad.opacity,
    };
    [corner(0), corner(1), corner(2), corner(0), corner(2), corner(3)]
}

#[derive(Debug)]
struct Slot {
    bind_group: wgpu::BindGroup,
    /// Kept so the texture lives as long as the group that binds it.
    _texture: wgpu::Texture,
}

/// Draws the reference pictures.
#[derive(Debug)]
pub struct ReferenceRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    picture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vertices: wgpu::Buffer,
    slots: [Option<Slot>; REFERENCE_SLOTS],
    /// The slots to draw this frame, in the order their quads sit in
    /// `vertices`.
    drawn: Vec<usize>,
    srgb_target: bool,
}

impl ReferenceRenderer {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hani reference uniforms"),
            size: size_of::<OverlayUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hani reference uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hani reference uniform group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let picture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hani reference picture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hani reference sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hani reference shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/reference.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hani reference pipeline layout"),
            bind_group_layouts: &[&uniform_layout, &picture_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hani reference pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<ReferenceVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x2,
                        2 => Float32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // Seen from either side: a picture behind the model is
                // looked at through its back when the camera goes round.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hani reference vertices"),
            size: (REFERENCE_SLOTS * 6 * size_of::<ReferenceVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            uniform_buffer,
            uniform_group,
            picture_layout,
            sampler,
            vertices,
            slots: [None, None, None],
            drawn: Vec::with_capacity(REFERENCE_SLOTS),
            srgb_target: target_format.is_srgb(),
        }
    }

    /// Put a picture in a slot, or empty it.
    pub fn set_picture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: usize,
        picture: Option<&ReferencePicture>,
    ) {
        let Some(held) = self.slots.get_mut(slot) else {
            return;
        };
        let Some(picture) = picture.filter(|picture| {
            picture.width > 0
                && picture.height > 0
                && picture.width <= MOST_PIXELS_ACROSS
                && picture.height <= MOST_PIXELS_ACROSS
                && picture.rgba.len() == (picture.width * picture.height * 4) as usize
        }) else {
            *held = None;
            return;
        };
        let size = wgpu::Extent3d {
            width: picture.width,
            height: picture.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hani reference picture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &picture.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(picture.width * 4),
                rows_per_image: Some(picture.height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hani reference picture group"),
            layout: &self.picture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        *held = Some(Slot { bind_group, _texture: texture });
    }

    /// Whether a slot holds a picture.
    pub fn holds(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(Option::is_some)
    }

    /// Replace this frame's quads and the matrix they are drawn with. A quad
    /// whose slot holds no picture, or which is wholly see-through, is not
    /// drawn.
    pub fn upload(&mut self, queue: &wgpu::Queue, quads: &[ReferenceQuad], view_projection: Mat4) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&OverlayUniforms {
                view_projection: view_projection.to_cols_array_2d(),
                srgb_target: u32::from(self.srgb_target),
                padding: [0; 3],
            }),
        );
        self.drawn.clear();
        let mut vertices = Vec::with_capacity(REFERENCE_SLOTS * 6);
        for quad in quads {
            if self.drawn.len() == REFERENCE_SLOTS
                || !self.holds(quad.slot)
                || quad.opacity <= 0.0
                || self.drawn.contains(&quad.slot)
            {
                continue;
            }
            self.drawn.push(quad.slot);
            vertices.extend_from_slice(&quad_vertices(quad));
        }
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
        }
    }

    /// Whether anything is to be drawn this frame.
    pub fn is_empty(&self) -> bool {
        self.drawn.is_empty()
    }

    /// Draw into a pass whose depth was cleared for it.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.drawn.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for (at, slot) in self.drawn.iter().enumerate() {
            let Some(Some(held)) = self.slots.get(*slot) else {
                continue;
            };
            pass.set_bind_group(1, &held.bind_group, &[]);
            let first = (at * 6) as u32;
            pass.draw(first..first + 6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_is_two_triangles_with_the_picture_upright() {
        let quad = ReferenceQuad {
            slot: 0,
            corners: [Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
            opacity: 0.5,
            flipped: false,
        };
        let vertices = quad_vertices(&quad);
        // The bottom left corner reads the bottom left of the picture, which
        // is its last row.
        assert_eq!(vertices[0].uv, [0.0, 1.0]);
        // The top right reads the first row's right end.
        assert_eq!(vertices[2].uv, [1.0, 0.0]);
        assert!(vertices.iter().all(|vertex| vertex.opacity == 0.5));

        let flipped = quad_vertices(&ReferenceQuad { flipped: true, ..quad });
        assert_eq!(flipped[0].uv, [1.0, 1.0], "a flipped picture starts from its right");
        assert_eq!(flipped[0].position, vertices[0].position, "flipping moved the plane");
    }
}
