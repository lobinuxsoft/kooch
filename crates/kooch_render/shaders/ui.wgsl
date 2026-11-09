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
// SVGs, drawn by Vello (#1865); 1×1 and never sampled when nothing is vector.
@group(0) @binding(7) var vectors: texture_2d<f32>;

// The image atlas or the vector one, at `uv` in texels.
fn ui_sample(vector: bool, uv: vec2<f32>) -> vec4<f32> {
    if vector {
        return textureSampleLevel(vectors, images, uv / vec2<f32>(textureDimensions(vectors)), 0.0);
    }
    return textureSampleLevel(atlas, images, uv / vec2<f32>(textureDimensions(atlas)), 0.0);
}

struct UiMask {
    min: vec2<f32>,
    size: vec2<f32>,
    radius: f32,
    // Index plus one, like `Instance.mask`.
    parent: u32,
    uv_min: vec2<u32>,
    uv_size: vec2<u32>,
    luminance: u32,
    fill_from: u32,
    fill: f32,
    pad: f32,
}

@group(0) @binding(4) var<storage, read> ui_masks: array<UiMask>;

// `-kooch-fill-from: image`, and how wide its edge is in brightness: about two of 8 bits' steps.
const UI_FILL_IMAGE: u32 = 6u;
const UI_FILL_SOFT: f32 = 0.008;

const UI_CURVE_POINTS: u32 = 128u;

struct UiCurve {
    points: array<vec2<f32>, 128>,
    left: f32,
    width: f32,
    middle: f32,
    closed: u32,
}

@group(0) @binding(5) var<storage, read> ui_curves: array<UiCurve>;

// Flips and transforms: the matrix before a quad's curve bends it, then the one after.
struct UiMirror {
    before: mat3x3<f32>,
    after: mat3x3<f32>,
}

@group(0) @binding(6) var<storage, read> ui_mirrors: array<UiMirror>;

// Strips a curved quad is cut into. `UiPass` draws two vertices more than twice this.
const UI_CURVE_STRIPS: u32 = 64u;

// Masks nested deeper than this stop cutting. It also ends a cycle in a list sent over the wire.
const UI_MASK_DEPTH: u32 = 16u;

const QUAD_GLYPH: u32 = 1u;
const QUAD_IMAGE: u32 = 2u;
// Set here, not by the list: a glyph at an angle cannot be read texel by texel.
const QUAD_CURVED: u32 = 4u;
const QUAD_VECTOR: u32 = 8u;
// `UiMask.luminance` bits: cut by brightness, and sampled from the vector atlas.
const MASK_LUMINANCE: u32 = 1u;
const MASK_VECTOR: u32 = 2u;

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
    @location(10) curve: u32,
    @location(11) mirror: u32,
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
    // Where the layout put this fragment: clip and masks are tested here, so they bend too.
    @location(10) straight: vec2<f32>,
}

// `pixel` of the layout on the line: across becomes along, past its ends it carries on straight.
fn ui_bent(pixel: vec2<f32>, curve: u32) -> vec2<f32> {
    // Through the buffer each time: an array copied to a value cannot take a runtime index.
    let at = curve - 1u;
    let last = UI_CURVE_POINTS - 1u;
    let along = (pixel.x - ui_curves[at].left) / max(ui_curves[at].width, 1.0) * f32(last);
    let index = u32(clamp(along, 0.0, f32(last - 1u)));
    let part = along - f32(index);
    let here = ui_curves[at].points[index];
    let next = ui_curves[at].points[index + 1u];
    // From both neighbours, or each stretch would turn at once and leave a crease.
    let closed = ui_curves[at].closed != 0u;
    // A closed line's first and last points are the same one: past it is the other end's neighbour.
    let behind = select(max(index, 1u) - 1u, last - 1u, closed && index == 0u);
    let ahead = select(min(index + 2u, last), 1u, closed && index + 2u > last);
    let before = next - ui_curves[at].points[behind];
    let after = ui_curves[at].points[ahead] - here;
    let tangent = mix(before, after, clamp(part, 0.0, 1.0));
    let unit = tangent / max(length(tangent), 0.00001);
    let away = pixel.y - ui_curves[at].middle;
    return mix(here, next, part) + vec2<f32>(-unit.y, unit.x) * away;
}

@vertex
fn vs_ui(@builtin(vertex_index) vertex: u32, quad: Instance) -> Varyings {
    var corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u));
    if quad.curve != 0u {
        corner = vec2<f32>(f32(vertex >> 1u) / f32(UI_CURVE_STRIPS), f32(vertex & 1u));
    }
    let straight = quad.min + corner * quad.size;
    // Homogeneous: a tilt's perspective reaches `w`, so the GPU interpolates uv and all correctly.
    var pixel = vec3<f32>(straight, 1.0);
    var out: Varyings;
    out.material_flags = quad.material_flags;
    // Only where it lands: clip, masks and uv stay in the layout, so they mirror with it.
    if quad.mirror != 0u {
        pixel = ui_mirrors[quad.mirror - 1u].before * pixel;
    }
    if quad.curve != 0u {
        pixel = vec3<f32>(ui_bent(pixel.xy / pixel.z, quad.curve), 1.0);
        out.material_flags.y |= QUAD_CURVED;
    }
    if quad.mirror != 0u {
        pixel = ui_mirrors[quad.mirror - 1u].after * pixel;
    }
    out.straight = straight;
    // y flipped: pixels descend, clip space climbs.
    let ndc = pixel.xy / screen.size * 2.0 - pixel.z;
    out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, pixel.z);
    out.half_size = quad.size * 0.5;
    out.local = (corner - 0.5) * quad.size;
    out.color = quad.color;
    out.border_color = quad.border_color;
    out.radius_border = quad.radius_border;
    out.uv = vec2<f32>(quad.uv_min) + corner * vec2<f32>(quad.uv_size);
    out.clip = quad.clip;
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

// How filled a mask is at `along`: all of it at 1 and none at 0, as `lib_fill` in the library.
fn ui_filled(along: f32, fill: f32, soft: f32) -> f32 {
    let reach = fill * (1.0 + soft * 2.0) - soft;
    return clamp((reach - along) / soft + 0.5, 0.0, 1.0);
}

// How far a fill from `start` has to go to reach `across`, 0 to 1 over the mask.
fn ui_fill_along(start: u32, across: vec2<f32>, local: vec2<f32>) -> f32 {
    if start == 1u {
        return across.x;
    }
    if start == 2u {
        return 1.0 - across.x;
    }
    if start == 3u {
        return across.y;
    }
    if start == 4u {
        return 1.0 - across.y;
    }
    return fract(atan2(local.x, -local.y) / 6.2831853 + 1.0);
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
        if mask.fill_from != 0u && mask.fill_from != UI_FILL_IMAGE {
            let along = ui_fill_along(mask.fill_from, (local + half) / mask.size, local);
            coverage *= ui_filled(along, mask.fill, 1.0 / max(max(mask.size.x, mask.size.y), 1.0));
        }
        if mask.uv_size.x != 0u {
            let span = vec2<f32>(mask.uv_size);
            // Half a texel in: the filter would read the atlas neighbour past the edge.
            let texel = clamp((local + half) / mask.size * span, vec2<f32>(0.5), span - 0.5);
            let vector = (mask.luminance & MASK_VECTOR) != 0u;
            let cut = ui_sample(vector, vec2<f32>(mask.uv_min) + texel);
            coverage *= cut.a;
            let bright = dot(cut.rgb, vec3<f32>(0.2125, 0.7154, 0.0721));
            if mask.fill_from == UI_FILL_IMAGE {
                // Over its alpha: the filter mixes the map's edge with the black around it, and a
                // darker edge would fill first, a thin line ahead of the bar.
                let order = bright / max(cut.a, 0.004);
                coverage *= ui_filled(order, mask.fill, UI_FILL_SOFT);
            } else if (mask.luminance & MASK_LUMINANCE) != 0u {
                coverage *= bright;
            }
        }
        at = mask.parent;
    }
    return coverage;
}

@fragment
fn fs_ui(in: Varyings) -> @location(0) vec4<f32> {
    let p = in.straight;
    if p.x < in.clip.x || p.y < in.clip.y || p.x >= in.clip.z || p.y >= in.clip.w {
        discard;
    }
    return ui_quad_color(in) * ui_mask_coverage(p, in.mask);
}

fn ui_quad_color(in: Varyings) -> vec4<f32> {
    let flags = in.material_flags.y;
    if (flags & QUAD_IMAGE) != 0u {
        let texel = ui_sample((flags & QUAD_VECTOR) != 0u, in.uv) * in.color;
        return vec4<f32>(texel.rgb * texel.a, texel.a);
    }
    if (flags & QUAD_GLYPH) != 0u {
        var coverage = textureLoad(atlas, vec2<i32>(floor(in.uv)), 0).r;
        if (flags & QUAD_CURVED) != 0u {
            // Half a texel in: the filter would read the next glyph.
            let first = vec2<f32>(in.atlas_rect.xy);
            let inside = clamp(in.uv, first + 0.5, first + vec2<f32>(in.atlas_rect.zw) - 0.5);
            let side = vec2<f32>(textureDimensions(atlas));
            coverage = textureSampleLevel(atlas, images, inside / side, 0.0).r;
        }
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
