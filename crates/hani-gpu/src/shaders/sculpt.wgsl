// SPDX-License-Identifier: AGPL-3.0-only
//
// The sculpt viewport's shading: a rig of lights and a global light, and at
// each pixel a surface (the model's material, a paint, a varnish or a metal)
// lit by them with one model, a wrapped diffuse term and a GGX highlight.
// `hani_gpu::lights` is where the design is, and its `shade` is this file's
// `shade` in Rust. Until 2026-11-27 this was a matcap lookup, which is kept
// behind `uniforms.lights.matcap` for the test that holds the default to it.

// The rig and the model's material: `hani_gpu::LightBlock`, field for field.
// `lights::tests::the_block_is_laid_out_as_the_shader_reads_it` reads these
// lines.
struct Lights {
    direction: array<vec4<f32>, 4>,
    colour: array<vec4<f32>, 4>,
    wrap: vec4<f32>,
    falloff: vec4<f32>,
    sky: vec4<f32>,
    ground: vec4<f32>,
    key: vec4<f32>,
    shade: vec4<f32>,
    f0: vec4<f32>,
    rim: vec4<f32>,
    through: vec4<f32>,
    paint: vec4<f32>,
    matcap: vec4<u32>,
}

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
    //      the bed. `hani_core::overhang` is the rule and the other copy.
    // A vec4 because a uniform struct aligns to 16 bytes and the four scalars
    // above have filled their row.
    cavity: vec4<f32>,
    // The ring cut's see-through view, `docs/CUT-RING.md` stage 2: what is
    // kept solid is on the front of `ghost_plane` (xyz its normal, w its
    // offset, `dot(n, p) >= w`) and within `ghost_keep.w` of `ghost_keep.xyz`;
    // everything else has `ghost.x` of its pixels screened away, and what
    // is kept `ghost.y`. `ghost.x` zero is off and draws every body as it
    // always was.
    ghost_plane: vec4<f32>,
    ghost_keep: vec4<f32>,
    ghost: vec4<f32>,
    // Linear RGB per filament slot, indexed by the stored slot byte. Slot 0 is
    // never read. Sixteen entries of 16 bytes, which keeps the struct's size a
    // multiple of 16 without a padding word; the Rust type asserts the total,
    // and `renderer::tests::the_shader_palette_is_sized_to_palette_slots`
    // reads this literal and the clamp below out of the source, because a
    // Rust-side change to PALETTE_SLOTS would otherwise pass validation with
    // the shader indexing past what it was given.
    palette: array<vec4<f32>, 16>,
    // The document's paints, by slot: what `hani_gpu::PaintBlock` lays out.
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
    // The lights and the model's material. See `Lights` above.
    lights: Lights,
}

// What a pixel is made of, for `shade`: `hani_gpu::lights::Surface`.
struct Surface {
    key: vec3<f32>,
    shade: vec3<f32>,
    wrap: f32,
    f0: vec3<f32>,
    roughness: f32,
    metal: f32,
    reflect: f32,
    rim: vec3<f32>,
    through: vec3<f32>,
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
// lit quarter keeps its gradient rather than flattening to one value. Only
// the matcap path reads this; the lit path has `lights.paint.x`, the same
// number, from `hani_gpu::lights::PAINT_EXPOSURE`.
const PAINT_EXPOSURE: f32 = 0.8;
// The shading model's constants. `hani_gpu::lights` has the other copy of
// each, and `lights::tests::the_shaders_constants_are_these` reads these lines.
const LIGHT_SIZE: f32 = 0.05;
const HORIZON: f32 = 0.75;
const THIN_GLOW: f32 = 0.6;
const ROOM: f32 = 0.7;
const ROOM_POWER: f32 = 3.0;
const RIM_POWER: f32 = 3.5;
const PI: f32 = 3.14159265;
// A metallic paint's flakes: how big each is, in millimetres of the model,
// and how far one leans from the surface it lies in.
const FLAKE_SIZE: f32 = 0.2;
const FLAKE_TILT: f32 = 0.35;
// The least light a wash's pigment is taken to pass, so that pure black on an
// edge is `floor^0` and not `0^0`. `hani_core::paint::WASH_FLOOR` is the
// other copy and `paints::tests` reads this line.
const WASH_FLOOR: f32 = 0.003;
// How far a paint's zenithal undercoat lifts a surface facing straight up and
// darkens one facing straight down. `hani_core::paint` has the other copy
// of both and the reason they are uneven, and `paints::tests` reads these.
const ZENITH_LIFT: f32 = 0.35;
const ZENITH_SHADE: f32 = 0.55;
// The largest curvature the cavity byte carries, in 1/voxel. One fact in two
// places: `hani_core::signals::CAVITY_RANGE` is the other, and
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
    // The body's own paint at this vertex: a colour in linear light, already
    // multiplied by its coverage, and the coverage. Multiplied BEFORE it is
    // interpolated, because bare is stored as black with no coverage and a
    // triangle from paint to bare would otherwise fade through a darkened
    // version of the paint on its way out: the dark fringe every painting
    // program has had once.
    @location(6) paint: vec4<f32>,
    // The finish a varnish laid here, byte 3 of the attribute buffer, by
    // `hani_core::paint::Finish::code`: nought for none. Flat: a finish is
    // one or another.
    @location(7) @interpolate(flat) finish: u32,
    // The vertex in the model's own space, for the ring cut's see-through
    // view.
    @location(8) world: vec3<f32>,
    // The share of this body screened away: the hierarchy's "others
    // see-through", `hani_gpu::spread`. Nought draws it whole.
    @location(9) @interpolate(flat) screened: f32,
    // A boolean group's row drawn see-through, tinted by what it does: 1 a
    // cut, 2 a keep, nought none. `hani_gpu::tint`.
    @location(10) @interpolate(flat) tint: u32,
    // The vertex where it is in the body, before the gizmo turns it: what a
    // metallic paint's flakes are fixed to, so that they neither swim as the
    // camera moves nor crawl as the model turns.
    @location(11) surface: vec3<f32>,
}

// How rough and how metal a finish is, by its code.
// `hani_core::paint::Finish::roughness_and_metal` is the other copy.
fn finish_of(code: u32) -> vec2<f32> {
    switch code {
        case 1u: { return vec2<f32>(0.9, 0.0); }
        case 2u: { return vec2<f32>(0.5, 0.0); }
        case 3u: { return vec2<f32>(0.15, 0.0); }
        case 4u: { return vec2<f32>(0.5, 1.0); }
        case 5u: { return vec2<f32>(0.05, 1.0); }
        default: { return vec2<f32>(1.0, 0.0); }
    }
}

// sRGB to linear light. `hani_core::paint::linear`, and the same numbers.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// One light's GGX highlight with Smith's height-correlated visibility, times
// the light's cosine. The eye is down the view's own axis, as the matcap had
// it. `hani_gpu::lights::microfacet` is the other copy.
fn microfacet(n: vec3<f32>, l: vec3<f32>, alpha: f32) -> f32 {
    let h = normalize(l + vec3<f32>(0.0, 0.0, 1.0));
    let nl = max(dot(n, l), 0.0);
    let nv = max(n.z, 1.0e-3);
    let nh = max(dot(n, h), 0.0);
    let a2 = alpha * alpha;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    let distribution = a2 / (PI * d * d);
    let visibility = 0.5 / (nl * sqrt(nv * nv * (1.0 - a2) + a2) + nv * sqrt(nl * nl * (1.0 - a2) + a2) + 1.0e-5);
    return distribution * visibility * nl;
}

// Schlick's Fresnel term, by the angle between the eye and the half way vector.
fn fresnel(f0: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let vh = max(normalize(l + vec3<f32>(0.0, 0.0, 1.0)).z, 0.0);
    return f0 + (vec3<f32>(1.0) - f0) * pow(max(1.0 - vh, 0.0), 5.0);
}

// How much of the surrounding a surface reflects: Karis's fit to the
// split-sum term ("Physically Based Shading on Mobile", 2014).
fn environment_brdf(f0: vec3<f32>, roughness: f32, nv: f32) -> vec3<f32> {
    let r = roughness * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    return f0 * (-1.04 * a004 + r.z) + vec3<f32>(1.04 * a004 + r.w);
}

// The surrounding a mirror sees in a direction in the world: the sky above a
// horizon, lighter toward the top, and the ground below it, the horizon
// blurred and the difference faded as the surface roughens. No image.
fn surrounding(r: vec3<f32>, roughness: f32) -> vec3<f32> {
    let blur = 0.01 + 1.2 * roughness * roughness;
    let t = smoothstep(-blur, blur, r.y);
    let contrast = HORIZON * (1.0 - roughness);
    let above = uniforms.lights.sky.rgb * (1.0 + contrast * (0.4 + 0.6 * max(r.y, 0.0)));
    let below = uniforms.lights.ground.rgb * (1.0 - contrast);
    return mix(below, above, t);
}

// A surface lit by the rig: `n` the normal in view space, `nw` in the world,
// `glint` the normal the lights' highlights are taken from (a flake's, on a
// metallic paint), `thin` how thin the surface is here, 0..1.
// `hani_gpu::lights::shade` is this in Rust, and the offscreen test
// `the_rust_model_is_the_shaders` holds the two together.
fn shade(s: Surface, n: vec3<f32>, nw: vec3<f32>, glint: vec3<f32>, thin: f32) -> vec3<f32> {
    let ground = uniforms.lights.ground.rgb;
    let hemisphere = mix(ground, uniforms.lights.sky.rgb, clamp(nw.y * 0.5 + 0.5, 0.0, 1.0));
    let alpha = max(s.roughness * s.roughness, LIGHT_SIZE);
    // Reflected about the normal from the eye, and bent toward the normal as
    // the surface roughens.
    let r = normalize(mix(n * (2.0 * n.z) - vec3<f32>(0.0, 0.0, 1.0), n, s.roughness * s.roughness));
    var diffuse = s.key * hemisphere;
    var specular = vec3<f32>(0.0);
    var behind = hemisphere;
    var room = vec3<f32>(0.0);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let colour = uniforms.lights.colour[i];
        // A light that is off costs nothing past this: the same for every
        // pixel, so the branch is free.
        if (colour.r + colour.g + colour.b <= 0.0) {
            continue;
        }
        let d = uniforms.lights.direction[i];
        let wrap = uniforms.lights.wrap[i];
        let falloff = uniforms.lights.falloff[i];
        // A light fixed in the world is turned into view space here.
        var l = d.xyz;
        if (d.w > 0.5) {
            l = (uniforms.view * vec4<f32>(l, 0.0)).xyz;
        }
        l = normalize(l);
        let x = dot(n, l);
        let plain = pow(clamp((x + wrap) / (1.0 + wrap), 0.0, 1.0), falloff);
        var wrapped = plain;
        if (s.wrap > 0.0) {
            let w = wrap + s.wrap;
            wrapped = pow(clamp((x + w) / (1.0 + w), 0.0, 1.0), falloff);
        }
        let albedo = select(s.shade, s.key, i == 0u);
        diffuse = diffuse + colour.rgb * (albedo * plain + s.through * (wrapped - plain));
        behind = behind + colour.rgb * max(-x, 0.0);
        // A light that casts no highlight (the default fill) skips both.
        if (colour.w > 0.0) {
            specular = specular + colour.rgb * colour.w * microfacet(glint, l, alpha) * fresnel(s.f0, l);
            // ROOM_POWER, 3, as a cube: a power is an exp2 and a log2.
            let toward = max(dot(r, l) * 0.5 + 0.5, 0.0);
            room = room + colour.rgb * colour.w * (toward * toward * toward);
        }
    }
    let nv = max(n.z, 0.0);
    let rim = s.rim * hemisphere * pow(max(1.0 - nv, 0.0), RIM_POWER);
    // What it reflects: the sky and the ground, in the world, by the view's
    // rotation turned back, and the room the lights light. Nothing, at no
    // cost, for the clay and unvarnished paint, which reflect nothing.
    var environment = vec3<f32>(0.0);
    if (s.reflect > 0.0) {
        let turn = mat3x3<f32>(uniforms.view[0].xyz, uniforms.view[1].xyz, uniforms.view[2].xyz);
        let seen = surrounding(normalize(transpose(turn) * r), s.roughness) + room * ROOM;
        environment = seen * environment_brdf(s.f0, s.roughness, nv) * s.reflect;
    }
    let glow = s.through * thin * behind * THIN_GLOW;
    return diffuse * (1.0 - s.metal) + specular + rim + environment + glow;
}

// The model's material, as the block holds it.
fn material() -> Surface {
    let m = uniforms.lights;
    return Surface(m.key.rgb, m.shade.rgb, m.key.w, m.f0.rgb, m.f0.w, m.rim.w, m.shade.w, m.rim.rgb, m.through.rgb);
}

// Unvarnished paint of a colour, linear, already multiplied by its coverage:
// lit as the clay is, with its own colour in place of every one of the
// clay's. `hani_gpu::lights::Surface::paint`.
fn paint_surface(colour: vec3<f32>) -> Surface {
    let p = uniforms.lights.paint;
    let albedo = colour * p.x;
    return Surface(albedo, albedo, 0.0, albedo * p.y, p.w, 0.0, 0.0, albedo * p.z, vec3<f32>(0.0));
}

// `b` laid over `a` by `share`, field by field: linear, so that a pixel half
// painted is lit half as the paint and half as what is under it.
fn over(a: Surface, b: Surface, share: f32) -> Surface {
    return Surface(
        mix(a.key, b.key, share), mix(a.shade, b.shade, share), mix(a.wrap, b.wrap, share),
        mix(a.f0, b.f0, share), mix(a.roughness, b.roughness, share), mix(a.metal, b.metal, share),
        mix(a.reflect, b.reflect, share), mix(a.rim, b.rim, share), mix(a.through, b.through, share),
    );
}

// Three numbers 0..1 from a cell of the model: Jarzynski and Olano's pcg3d
// ("Hash Functions for GPU Rendering", 2020). A hash and not a texture.
fn flake_hash(cell: vec3<i32>) -> vec3<f32> {
    var v = bitcast<vec3<u32>>(cell) * 1664525u + vec3<u32>(1013904223u);
    v.x = v.x + v.y * v.z;
    v.y = v.y + v.z * v.x;
    v.z = v.z + v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x = v.x + v.y * v.z;
    v.y = v.y + v.z * v.x;
    v.z = v.z + v.x * v.y;
    return vec3<f32>(v >> vec3<u32>(8u)) / 16777216.0;
}

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) attributes: vec4<f32>,
    @location(3) paint: vec4<f32>,
    // How far this body is drawn from where it is: the hierarchy's Explode,
    // `hani_gpu::spread`. Zero for every body not exploded. Explode moves
    // only where it is DRAWN: `world` and `height` take its offset back off,
    // so the overhang tint and the ring cut's see-through read the real
    // body. In `w`, the share of the body screened away.
    //
    // And a body the transform gizmo is dragging, turned and scaled where
    // the drag has it: three columns of a linear part, each less the
    // identity's so that the zeroed entry every other body draws with is
    // the identity. `hani_gpu::spread`.
    @location(4) turn_x: vec4<f32>,
    @location(5) turn_y: vec4<f32>,
    @location(6) turn_z: vec4<f32>,
    @location(7) spread: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    let a = turn_x.xyz + vec3<f32>(1.0, 0.0, 0.0);
    let b = turn_y.xyz + vec3<f32>(0.0, 1.0, 0.0);
    let c = turn_z.xyz + vec3<f32>(0.0, 0.0, 1.0);
    let drawn = a * position.x + b * position.y + c * position.z + spread.xyz;
    out.clip_position = uniforms.view_projection * vec4<f32>(drawn, 1.0);
    // The share screened away, and above it the tint. `hani_gpu::tint`.
    let tint = floor(spread.w * 0.5);
    out.screened = spread.w - tint * 2.0;
    out.tint = u32(tint);
    // The normal through the cofactors of that linear part, which is its
    // inverse transpose times its determinant: the right direction under
    // any scale, and the identity's for every body not being dragged.
    let turned_normal = normalize(
        cross(b, c) * normal.x + cross(c, a) * normal.y + cross(a, b) * normal.z,
    );
    // The view matrix is a rigid transform, so its upper 3x3 rotates normals
    // correctly without an inverse transpose.
    out.view_normal = (uniforms.view * vec4<f32>(turned_normal, 0.0)).xyz;
    // Where the part IS, for the overhang tint and the ring cut's
    // see-through: where it is drawn, less Explode's offset, which is in
    // the columns' `w` (`hani_gpu::spread`). A part the gizmo is dragging
    // is read where the drag has it; an exploded one where it stands.
    out.world_normal = turned_normal;
    out.world = drawn - vec3<f32>(turn_x.w, turn_y.w, turn_z.w);
    out.height = out.world.y;
    out.mask = attributes.r;
    // `Unorm8x4` hands the byte back as `n / 255`; the round undoes it exactly.
    out.slot = u32(round(attributes.g * 255.0));
    // Byte 0 means "not measured" and draws as flat. Anything else is 128 for
    // flat on a square root curve either side, which the product with its own
    // magnitude undoes.
    let cavity_byte = round(attributes.b * 255.0);
    let t = (cavity_byte - 128.0) / 127.0;
    out.cavity = select(CAVITY_RANGE * t * abs(t), 0.0, cavity_byte < 0.5);
    out.paint = vec4<f32>(srgb_to_linear(paint.rgb) * paint.a, paint.a);
    out.finish = u32(round(attributes.a * 255.0));
    out.surface = position;
    return out;
}

@fragment
fn fragment_main(in: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // The ring cut's see-through view: all but the piece being cut screened
    // by a 4x4 ordered dither, so what is behind (the cut's band and face)
    // shows through with no sorting and the depth buffer still right.
    // A body the hierarchy draws see-through is screened the same way, by
    // its own share.
    // How much of the model one pixel spans, in millimetres: taken first,
    // where every pixel of a quad is still running, since a derivative
    // needs its neighbours. A metallic paint's flakes fade out as they
    // shrink below a pixel, where they would only shimmer.
    let footprint = length(fwidth(in.surface));
    var amount = in.screened;
    if (uniforms.ghost.x > 0.0) {
        let kept = dot(uniforms.ghost_plane.xyz, in.world) >= uniforms.ghost_plane.w
            && distance(in.world, uniforms.ghost_keep.xyz) <= uniforms.ghost_keep.w;
        // The piece screened by `ghost.y`, less than the rest, so the cut
        // face inside it and the keys on the face show through it too.
        amount = max(amount, select(uniforms.ghost.x, uniforms.ghost.y, kept));
    }
    if (amount > 0.0) {
        var bayer = array<f32, 16>(
            0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0,
            3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0
        );
        let cell = vec2<u32>(in.clip_position.xy) % vec2<u32>(4u, 4u);
        if ((bayer[cell.y * 4u + cell.x] + 0.5) / 16.0 < amount) {
            discard;
        }
    }
    var normal = normalize(in.view_normal);

    // Back faces are visible inside cavities and through openings. Flipping the
    // normal there shades them as surfaces rather than as whatever the matcap
    // holds at the mirrored coordinate, which reads as an inside out dent.
    if (!front_facing) {
        normal = -normal;
    }

    // The same normal in the world, turned on a back face as the view's is:
    // what the global light lights by.
    let world_normal = select(-1.0, 1.0, front_facing) * normalize(in.world_normal);

    // The slot's colour, BEFORE the mask so protection still reads on a
    // painted surface: a filament's, or a paint's recipe. A slot past the
    // palette clamps to the last entry, which the application fills with a
    // colour no filament is.
    let slotted = uniforms.paint_shown != 0u && in.slot != 0u;
    var albedo = vec3<f32>(0.0);
    if (slotted) {
        albedo = uniforms.palette[min(in.slot, 15u)].rgb;
        let base = uniforms.paint_base[min(in.slot, 63u)];
        if (base.w > 0.5) {
            // A paint, and not a filament: a recipe shaded from how concave
            // the surface is here. This is `hani_core::paint::Paint::shade`
            // line for line, and `paints::tests` reads these lines out of
            // this file to hold the two together.
            let wash = uniforms.paint_wash[min(in.slot, 63u)];
            let film = uniforms.paint_film[min(in.slot, 63u)];
            let light = uniforms.paint_light[min(in.slot, 63u)];
            let s = clamp(in.cavity * uniforms.cavity.y, -1.0, 1.0);
            let recess = max(s, 0.0);
            let edge = max(-s, 0.0);
            // The undercoat first, under everything: lighter where the
            // surface faces up, darker where it faces down, in the model's
            // own space and not the camera's.
            let up = normalize(in.world_normal).y;
            let undercoat = 1.0 + light.w * (ZENITH_LIFT * max(up, 0.0) - ZENITH_SHADE * max(-up, 0.0));
            albedo = base.rgb * undercoat;
            // The wash is a film of pigment, by Beer and Lambert: thin on an
            // edge, `stain` thick on a flat, pooled in a crease.
            let thickness = max(wash.w + film.x * recess - film.y * edge, 0.0);
            albedo = albedo * pow(max(wash.rgb, vec3<f32>(WASH_FLOOR)), vec3<f32>(thickness));
            let caught = clamp(smoothstep(film.w, 1.0, edge) * film.z, 0.0, 1.0);
            albedo = mix(albedo, light.rgb, caught);
        }
    }
    // The body's own paint, over the clay or over the slot's recipe, by its
    // coverage: a colour in linear light already multiplied by it.
    let coated = uniforms.paint_shown != 0u && in.paint.a > 0.0;

    var colour = vec3<f32>(0.0);
    if (uniforms.lights.matcap.x != 0u) {
        // The matcap, exactly as before 2026-11-27, and no finish: kept for
        // the offscreen test that holds the default rig and clay to it.
        // The view space normal's xy maps onto the unit disc, the v axis
        // flipped because texture row 0 is the top of the sphere. Paint is
        // its colour times how brightly the clay is lit here.
        let uv = vec2<f32>(normal.x * 0.5 + 0.5, 0.5 - normal.y * 0.5);
        colour = textureSample(matcap_texture, matcap_sampler, uv).rgb;
        let brightness = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722)) / MATCAP_BASE_LUMA;
        if (slotted) {
            colour = select(albedo * brightness * PAINT_EXPOSURE, albedo, uniforms.paint_shown == 2u);
        }
        if (coated) {
            let coat = select(in.paint.rgb * brightness * PAINT_EXPOSURE, in.paint.rgb, uniforms.paint_shown == 2u);
            colour = colour * (1.0 - in.paint.a) + coat;
        }
    } else if (uniforms.paint_shown == 2u) {
        // Flat: the paint as it is, with no light on it. A wash is shading,
        // and it cannot be judged with the viewport's own shading on top.
        // What is not painted is the model, lit.
        colour = shade(material(), normal, world_normal, normal, 0.0);
        if (slotted) {
            colour = albedo;
        }
        if (coated) {
            colour = colour * (1.0 - in.paint.a) + in.paint.rgb;
        }
    } else {
        // Lit: the model's material, the slot's colour as paint in its
        // place, the body's paint over either by its coverage, and over all
        // of it the finish a varnish or a metal laid.
        var surface = material();
        if (slotted) {
            surface = paint_surface(albedo);
        }
        if (coated) {
            surface = over(surface, paint_surface(in.paint.rgb / in.paint.a), in.paint.a);
        }
        var glint = normal;
        if (uniforms.paint_shown == 1u && in.finish != 0u) {
            let finish = finish_of(in.finish);
            if (finish.y > 0.5) {
                // A metal: its colour is what it reflects, nothing of it
                // diffuse. A metallic paint's flakes, each a cell of the
                // model a fraction of a millimetre across and leant its own
                // way, catch the lights each at its own angle.
                let paint = in.paint.rgb / max(in.paint.a, 1.0e-3);
                let metal_colour = select(surface.key / uniforms.lights.paint.x, paint, coated);
                let share = select(1.0, in.paint.a, coated);
                let metal = Surface(vec3<f32>(0.0), vec3<f32>(0.0), 0.0, metal_colour, finish.x, 1.0, 1.0, vec3<f32>(0.0), vec3<f32>(0.0));
                surface = over(surface, metal, share);
                if (in.finish == 4u) {
                    let lean = flake_hash(vec3<i32>(floor(in.surface / FLAKE_SIZE))) - vec3<f32>(0.5);
                    let flake = normalize(world_normal + lean * FLAKE_TILT);
                    let seen = normalize((uniforms.view * vec4<f32>(flake, 0.0)).xyz);
                    let shown = clamp(FLAKE_SIZE * 1.5 / max(footprint, 1.0e-6) - 0.5, 0.0, 1.0);
                    glint = normalize(mix(normal, seen, share * shown));
                }
            } else {
                // A varnish: its own clear coat over what is under it, as
                // rough as its finish, reflecting the surrounding.
                surface.f0 = vec3<f32>(0.04);
                surface.roughness = finish.x;
                surface.reflect = 1.0;
            }
        }
        // How thin the surface is here, from the edge signal: where light
        // let into a resin comes through.
        let thin = clamp(-in.cavity * uniforms.cavity.y, 0.0, 1.0);
        colour = shade(surface, normal, world_normal, glint, thin);
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

    // A boolean row that cuts drawn reddened, one that keeps bluer, lit as
    // the pixel was. `hani_gpu::tint`.
    if (in.tint != 0u) {
        let luma = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
        let hue = select(vec3<f32>(0.35, 0.65, 1.25), vec3<f32>(1.3, 0.4, 0.35), in.tint == 1u);
        colour = mix(colour, luma * hue, 0.6);
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
        // A light mask lifted, twice as strong as itself near nought, and
        // with no step there. Until 2026-11-27 this had a floor of 0.3 at
        // any protection at all, which drew a hard line round every mask
        // at the brush's radius however finely the mask itself faded: the
        // maintainer saw "some kind of edge / border" there.
        let s = (1.0 - (1.0 - m) * (1.0 - m)) * uniforms.mask_tint;
        let luma = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
        colour = mix(colour, luma * vec3<f32>(0.345, 0.529, 1.15), s);
    }

    if (uniforms.srgb_target == 0u) {
        // Plain unorm target: encode here, since nothing downstream will.
        colour = pow(colour, vec3<f32>(1.0 / 2.2));
    }

    return vec4<f32>(colour, 1.0);
}
