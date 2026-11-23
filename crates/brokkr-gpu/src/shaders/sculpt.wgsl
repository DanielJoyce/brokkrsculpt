// SPDX-License-Identifier: AGPL-3.0-only
//
// Matcap shading for the sculpt viewport. The whole lighting model is a lookup
// into a small image by view space normal, which is why there are no lights,
// no shadows and no material parameters here.

struct Uniforms {
    view_projection: mat4x4<f32>,
    view: mat4x4<f32>,
    // Non zero when the render target encodes sRGB itself, in which case the
    // fragment must stay linear. iced picks the surface format, so this cannot
    // be assumed.
    srgb_target: u32,
    // Non zero when protection is read inverted. Resolved here rather than
    // baked into the mesh, which is what makes Invert one uniform write instead
    // of a remesh of the whole body.
    mask_inverted: u32,
    // How strongly the mask is tinted, 0..1. A VIEW strength: nothing about it
    // changes what a stroke does. Zero is the `show mask` toggle switched off,
    // and it draws the body exactly as an unmasked one.
    mask_tint: f32,
    // Non zero when painted slots are drawn in their filament colours. A VIEW
    // toggle like the tint: zero draws every body as bare clay.
    paint_shown: u32,
    // How the creases and edges are drawn. A VIEW setting like the two above:
    // nothing about it changes what a stroke does.
    //   x: strength, 0..1. Zero draws every body exactly as it was drawn
    //      before this existed.
    //   y: gain. The decoded curvature times this is 1.0 at the feature size
    //      the application calls a full crease.
    //   z: overhangs. Zero draws none. Otherwise the downward part of a
    //      normal past which a surface is an overhang: the sine of the
    //      steepest lean from upright the printer manages.
    //   w: the height, in the model's own Y, that nothing below is an
    //      overhang: a first layer above the lowest point, which rests on
    //      the bed. `brokkr_core::overhang` is the rule and the other copy.
    // A vec4 because a uniform struct aligns to 16 bytes and the four scalars
    // above have filled their row.
    cavity: vec4<f32>,
    // Linear RGB per filament slot, indexed by the stored slot byte. Slot 0 is
    // never read. Sixteen entries of 16 bytes, which keeps the struct's size a
    // multiple of 16 without a padding word; the Rust type asserts the total,
    // and `renderer::tests::the_shader_palette_is_sized_to_palette_slots`
    // reads this literal and the clamp below out of the source, because a
    // Rust-side change to PALETTE_SLOTS would otherwise pass validation with
    // the shader indexing past what it was given.
    palette: array<vec4<f32>, 16>,
    // The document's paints, by slot: what `brokkr_gpu::PaintBlock` lays out.
    // A paint is a base coat, a wash over it and a highlight on its edges,
    // and where `paint_base[slot].w` is non zero the slot names one and the
    // filament palette above is not consulted. All zeros is "no paints", and
    // draws every slot as the filament it always was.
    paint_base: array<vec4<f32>, 64>,
    // The wash's pigment, and in w its stain: the film's thickness on a flat.
    paint_wash: array<vec4<f32>, 64>,
    // x: how much more film a full crease holds. y: how much a full edge
    // sheds. z: how opaque the highlight is at a full edge. w: how sharp an
    // edge has to be before any highlight lands.
    paint_film: array<vec4<f32>, 64>,
    // The highlight's colour.
    paint_light: array<vec4<f32>, 64>,
}

// Luminance of the matcap's BASE clay colour in linear light, which is what a
// painted pixel divides its shading by: `luma / MATCAP_BASE_LUMA` recovers the
// lighting term the matcap multiplied into the clay, near enough, and the
// filament colour is then lit the way the clay was. One fact in two places;
// `matcap::tests::the_base_luma_the_shader_divides_by_is_the_clays` pins it.
const MATCAP_BASE_LUMA: f32 = 0.384;
// Filament colours are drawn a little under their full value. The lit side of
// the matcap runs to about 1.4x BASE's luminance, so a white filament still
// clips at the specular peak -- as white plastic does -- while the rest of the
// lit quarter keeps its gradient rather than flattening to one value.
const PAINT_EXPOSURE: f32 = 0.8;
// The least light a wash's pigment is taken to pass, so that pure black on an
// edge is `floor^0` and not `0^0`. `brokkr_core::paint::WASH_FLOOR` is the
// other copy and `paints::tests` reads this line.
const WASH_FLOOR: f32 = 0.003;
// The largest curvature the cavity byte carries, in 1/voxel. One fact in two
// places: `brokkr_core::signals::CAVITY_RANGE` is the other, and
// `renderer::tests::the_shader_decodes_cavity_the_way_the_engine_encodes_it`
// reads this line to hold them together.
const CAVITY_RANGE: f32 = 2.0;
// How far a full crease is darkened and a full edge is lifted, as fractions of
// the pixel's own colour. Uneven on purpose: a crease in shadow can go most of
// the way to black and still read as a crease, and an edge lifted that far
// would clip on the lit side.
const CREASE_DARKEN: f32 = 0.55;
const EDGE_LIFT: f32 = 0.30;
// An overhang is drawn toward this, a red-orange nothing else in the viewport
// is: the clay is a warm grey, the mask a cool blue. Most of the way and not
// all of it, so that the form under the tint can still be read.
const OVERHANG_TINT: vec3<f32> = vec3<f32>(0.90, 0.20, 0.06);
const OVERHANG_COVER: f32 = 0.8;
// The tint comes in over this much of a normal's downward part past the
// limit, which is about two degrees of lean near 45: a hard step there would
// crawl along the triangles as the model turned.
const OVERHANG_SOFT: f32 = 0.03;

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var matcap_texture: texture_2d<f32>;
@group(0) @binding(2) var matcap_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) view_normal: vec3<f32>,
    // The STORED protection at this vertex's own lattice cell, 0..1. Byte 0 of
    // the pool's attribute buffer.
    @location(1) mask: f32,
    // The painted filament slot at that cell, byte 1 of the attribute buffer.
    // FLAT, never interpolated: a slot is categorical, and a triangle whose
    // three corners disagree prints in ONE filament. Interpolating would draw
    // a blend of two that no printer can make. On Vulkan, Metal and DX12 the
    // flat value is the FIRST vertex's, which is the rule the 3MF export uses
    // for the same triangle, so what is drawn is what will be printed; wgpu's
    // GL backend takes the LAST vertex, so on that one backend a boundary
    // triangle can draw in the other filament. One triangle wide, at most.
    @location(2) @interpolate(flat) slot: u32,
    // How concave the surface is here, in 1/voxel: positive in a crease,
    // negative on an edge, zero on a flat and on a mesh that was made without
    // the signal. Byte 2 of the attribute buffer, decoded in the vertex stage
    // and interpolated, which is right for a continuous quantity where it
    // would be wrong for the slot.
    @location(3) cavity: f32,
    // The normal in the model's own space, where down is down whichever way
    // the camera is turned, and the vertex's height there. For overhangs.
    @location(4) world_normal: vec3<f32>,
    @location(5) height: f32,
}

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) attributes: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.view_projection * vec4<f32>(position, 1.0);
    // The view matrix is a rigid transform, so its upper 3x3 rotates normals
    // correctly without an inverse transpose.
    out.view_normal = (uniforms.view * vec4<f32>(normal, 0.0)).xyz;
    out.world_normal = normal;
    out.height = position.y;
    out.mask = attributes.r;
    // `Unorm8x4` hands the byte back as `n / 255`; the round undoes it exactly.
    out.slot = u32(round(attributes.g * 255.0));
    // Byte 0 means "not measured" and draws as flat. Anything else is 128 for
    // flat on a square root curve either side, which the product with its own
    // magnitude undoes.
    let cavity_byte = round(attributes.b * 255.0);
    let t = (cavity_byte - 128.0) / 127.0;
    out.cavity = select(CAVITY_RANGE * t * abs(t), 0.0, cavity_byte < 0.5);
    return out;
}

@fragment
fn fragment_main(in: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    var normal = normalize(in.view_normal);

    // Back faces are visible inside cavities and through openings. Flipping the
    // normal there shades them as surfaces rather than as whatever the matcap
    // holds at the mirrored coordinate, which reads as an inside out dent.
    if (!front_facing) {
        normal = -normal;
    }

    // Matcap lookup: the view space normal's xy maps onto the unit disc. The v
    // axis is flipped because texture row 0 is the top of the sphere.
    let uv = vec2<f32>(normal.x * 0.5 + 0.5, 0.5 - normal.y * 0.5);
    var colour = textureSample(matcap_texture, matcap_sampler, uv).rgb;

    // The paint, BEFORE the mask so protection still reads on a painted
    // surface: the clay's albedo is swapped for the filament's and the matcap's
    // lighting kept. A slot past the palette clamps to the last entry, which
    // the application fills with a colour no filament is.
    if (uniforms.paint_shown != 0u && in.slot != 0u) {
        var albedo = uniforms.palette[min(in.slot, 15u)].rgb;
        let base = uniforms.paint_base[min(in.slot, 63u)];
        if (base.w > 0.5) {
            // A paint, and not a filament: a recipe shaded from how concave
            // the surface is here. This is `brokkr_core::paint::Paint::shade`
            // line for line, and `paints::tests` reads these lines out of
            // this file to hold the two together.
            let wash = uniforms.paint_wash[min(in.slot, 63u)];
            let film = uniforms.paint_film[min(in.slot, 63u)];
            let light = uniforms.paint_light[min(in.slot, 63u)];
            let s = clamp(in.cavity * uniforms.cavity.y, -1.0, 1.0);
            let recess = max(s, 0.0);
            let edge = max(-s, 0.0);
            albedo = base.rgb;
            // The wash is a film of pigment, by Beer and Lambert: thin on an
            // edge, `stain` thick on a flat, pooled in a crease.
            let thickness = max(wash.w + film.x * recess - film.y * edge, 0.0);
            albedo = albedo * pow(max(wash.rgb, vec3<f32>(WASH_FLOOR)), vec3<f32>(thickness));
            let caught = clamp(smoothstep(film.w, 1.0, edge) * film.z, 0.0, 1.0);
            albedo = mix(albedo, light.rgb, caught);
        }
        let luma = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
        // Not clamped at 1: the lighting term is allowed to exceed the clay's
        // own, or every painted pixel on the key-lit side collapses to one
        // flat value and the form under the paint is gone.
        colour = albedo * (luma / MATCAP_BASE_LUMA) * PAINT_EXPOSURE;
        // Flat: the paint as it is, with no light on it. A wash is shading,
        // and it cannot be judged with the viewport's own shading on top.
        if (uniforms.paint_shown == 2u) {
            colour = albedo;
        }
    }

    // The creases and edges, AFTER the paint so they model a painted surface
    // the way they model clay, and BEFORE the mask so protection still reads
    // on top of them. A multiply on the pixel's own colour: it deepens a
    // shadow and brightens a highlight without moving the hue, which a blend
    // toward black or white would.
    if (uniforms.cavity.x > 0.0) {
        let s = clamp(in.cavity * uniforms.cavity.y, -1.0, 1.0);
        let shade = 1.0 - CREASE_DARKEN * max(s, 0.0) + EDGE_LIFT * max(-s, 0.0);
        colour = colour * mix(1.0, shade, uniforms.cavity.x);
    }

    // Overhangs, over everything above: this is a question asked of the
    // shape, and paint is not an answer to it.
    if (uniforms.cavity.z > 0.0 && in.height > uniforms.cavity.w) {
        let down = -normalize(in.world_normal).y;
        let over = smoothstep(uniforms.cavity.z, uniforms.cavity.z + OVERHANG_SOFT, down);
        // Lit as the pixel was, so that the tint has the model's form in it.
        let lit = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722)) / MATCAP_BASE_LUMA;
        colour = mix(colour, OVERHANG_TINT * clamp(lit, 0.35, 1.3), OVERHANG_COVER * over);
    }

    // The mask, as a CHROMA shift toward a cool blue the matcap cannot produce,
    // with luminance barely touched so the form stays readable underneath.
    //
    // Measured against the matcap's own constants rather than chosen by taste.
    // Its whole luminance swing is 102 levels and its coolest pixel is a
    // fill-lit cavity at b - r = +17, so darkening (ZBrush's answer) merges the
    // masked shadow into the viewport background and takes the silhouette with
    // it, and a gentle cool desaturation moves a lit pixel by only 36 levels --
    // a fully masked lit pixel would come out brighter than an unmasked
    // shadowed one. The hue below is theme::MASK, with blue driven past 1.0 so
    // the result sits outside anything the clay can be.
    let m = select(in.mask, 1.0 - in.mask, uniforms.mask_inverted != 0u);
    if (m > 0.0) {
        // The floor is what makes a first pass at brush strength visible at
        // all. It is a discontinuity at exactly zero and nowhere else, and that
        // is deliberate: it says "there is a mask here", and every value above
        // it is shown un-thresholded, which is the whole thing eight bits of
        // protection bought.
        let s = (0.30 + 0.70 * m) * uniforms.mask_tint;
        let luma = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
        colour = mix(colour, luma * vec3<f32>(0.345, 0.529, 1.15), s);
    }

    if (uniforms.srgb_target == 0u) {
        // Plain unorm target: encode here, since nothing downstream will.
        colour = pow(colour, vec3<f32>(1.0 / 2.2));
    }

    return vec4<f32>(colour, 1.0);
}
