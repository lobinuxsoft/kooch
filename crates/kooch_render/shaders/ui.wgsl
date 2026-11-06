// ui.wgsl — the UI's panels, images and glyphs, one instanced quad each (#96, #1708). The rounded
// box is Bevy's `ui.wgsl` signed distance, antialiased over one physical pixel.

struct Screen {
    size: vec2<f32>,
    // Seconds since the pass was made: only a `kind: ui` shader reads it.
    time: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var images: sampler;

struct UiMask {
    min: vec2<f32>,
    size: vec2<f32>,
    radius: f32,
    // Index plus one, like `Instance.mask`.
    parent: u32,
    uv_min: vec2<u32>,
    uv_size: vec2<u32>,
    luminance: u32,
    pad: u32,
}

@group(0) @binding(4) var<storage, read> ui_masks: array<UiMask>;

// Masks nested deeper than this stop cutting. It also ends a cycle in a list sent over the wire.
const UI_MASK_DEPTH: u32 = 16u;

const QUAD_GLYPH: u32 = 1u;
const QUAD_IMAGE: u32 = 2u;

struct Instance {
    @location(0) min: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) border_color: vec4<f32>,
    @location(5) radius_border: vec2<f32>,
    @location(6) uv_min: vec2<u32>,
    @location(7) uv_size: vec2<u32>,
    @location(8) material_flags: vec2<u32>,
    @location(9) mask: u32,
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    // Pixels from the quad's centre.
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) radius_border: vec2<f32>,
    // Atlas texel under this fragment.
    @location(5) uv: vec2<f32>,
    @location(6) clip: vec4<f32>,
    // The quad's material index and flags.
    @location(7) @interpolate(flat) material_flags: vec2<u32>,
    // The image's atlas texels, `min` and `size`.
    @location(8) @interpolate(flat) atlas_rect: vec4<u32>,
    @location(9) @interpolate(flat) mask: u32,
}

@vertex
fn vs_ui(@builtin(vertex_index) vertex: u32, quad: Instance) -> Varyings {
    let corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u));
    let pixel = quad.min + corner * quad.size;
    var out: Varyings;
    // y flipped: pixels descend, clip space climbs.
    let ndc = pixel / screen.size * 2.0 - 1.0;
    out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.half_size = quad.size * 0.5;
    out.local = (corner - 0.5) * quad.size;
    out.color = quad.color;
    out.border_color = quad.border_color;
    out.radius_border = quad.radius_border;
    out.uv = vec2<f32>(quad.uv_min) + corner * vec2<f32>(quad.uv_size);
    out.clip = quad.clip;
    out.material_flags = quad.material_flags;
    out.atlas_rect = vec4<u32>(quad.uv_min, quad.uv_size);
    out.mask = quad.mask;
    return out;
}

// Distance to a box of `half` extents with corners rounded by `radius`; negative inside.
fn sd_rounded_box(point: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half.x, half.y));
    let q = abs(point) - half + r;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// How much of pixel `p` the chain of masks from `first` lets through.
fn ui_mask_coverage(p: vec2<f32>, first: u32) -> f32 {
    var coverage = 1.0;
    var at = first;
    for (var depth = 0u; depth < UI_MASK_DEPTH && at != 0u; depth++) {
        let mask = ui_masks[at - 1u];
        let half = mask.size * 0.5;
        let local = p - mask.min - half;
        coverage *= clamp(0.5 - sd_rounded_box(local, half, mask.radius), 0.0, 1.0);
        if mask.uv_size.x != 0u {
            let span = vec2<f32>(mask.uv_size);
            // Half a texel in: the filter would read the atlas neighbour past the edge.
            let texel = clamp((local + half) / mask.size * span, vec2<f32>(0.5), span - 0.5);
            let side = vec2<f32>(textureDimensions(atlas));
            let uv = (vec2<f32>(mask.uv_min) + texel) / side;
            let cut = textureSampleLevel(atlas, images, uv, 0.0);
            coverage *= cut.a;
            if mask.luminance != 0u {
                coverage *= dot(cut.rgb, vec3<f32>(0.2125, 0.7154, 0.0721));
            }
        }
        at = mask.parent;
    }
    return coverage;
}

@fragment
fn fs_ui(in: Varyings) -> @location(0) vec4<f32> {
    let p = in.position.xy;
    if p.x < in.clip.x || p.y < in.clip.y || p.x >= in.clip.z || p.y >= in.clip.w {
        discard;
    }
    return ui_quad_color(in) * ui_mask_coverage(p, in.mask);
}

fn ui_quad_color(in: Varyings) -> vec4<f32> {
    let flags = in.material_flags.y;
    if (flags & QUAD_IMAGE) != 0u {
        let side = vec2<f32>(textureDimensions(atlas));
        let texel = textureSampleLevel(atlas, images, in.uv / side, 0.0) * in.color;
        return vec4<f32>(texel.rgb * texel.a, texel.a);
    }
    if (flags & QUAD_GLYPH) != 0u {
        let coverage = textureLoad(atlas, vec2<i32>(floor(in.uv)), 0).r;
        let alpha = in.color.a * coverage;
        return vec4<f32>(in.color.rgb * alpha, alpha);
    }
    let radius = in.radius_border.x;
    let border = in.radius_border.y;
    let outer = sd_rounded_box(in.local, in.half_size, radius);
    let inside = clamp(0.5 - outer, 0.0, 1.0);
    // The fill stops where the border starts, `border` pixels in from the edge.
    let fill_mix = clamp(0.5 - (outer + border), 0.0, 1.0);
    let border_mix = select(0.0, 1.0, border > 0.0);
    let color = mix(in.border_color, in.color, mix(1.0, fill_mix, border_mix));
    let alpha = color.a * inside;
    return vec4<f32>(color.rgb * alpha, alpha);
}
