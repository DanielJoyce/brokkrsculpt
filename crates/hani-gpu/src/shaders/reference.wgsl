// SPDX-License-Identifier: AGPL-3.0-only
//
// Reference pictures on their planes, drawn behind the sculpt. Each vertex
// carries the picture's opacity, so one uniform serves all three planes and
// only the texture changes between them.

struct Uniforms {
    view_projection: mat4x4<f32>,
    // Non zero when the target encodes sRGB itself. As in overlay.wgsl.
    srgb_target: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(0) var picture: texture_2d<f32>;
@group(1) @binding(1) var picture_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
}

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) opacity: f32,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.view_projection * vec4<f32>(position, 1.0);
    out.uv = uv;
    out.opacity = opacity;
    return out;
}

@fragment
fn fragment_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The texture is sRGB, so this sample is already linear.
    let texel = textureSample(picture, picture_sampler, in.uv);
    var colour = texel.rgb;
    if (uniforms.srgb_target == 0u) {
        colour = pow(colour, vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(colour, texel.a * in.opacity);
}
