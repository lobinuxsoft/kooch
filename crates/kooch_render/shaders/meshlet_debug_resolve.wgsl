// meshlet_debug_resolve.wgsl — fullscreen fragment debug visualizations
// for the R64 path (#440 two-pass migration).
//
// Ports the "colorize" debug branches that used to live in the compute
// deferred (meshlet_deferred_r64.wgsl): MeshletIds (1), InstanceIds (2),
// TriangleDensity (3), Overdraw (4), CullPassthrough (7). Normal-look
// modes (Off, OnlyLod0/Roots, *Rejected) render through the two-pass
// material path instead; the reject overlay is a separate dispatch.
//
// vbuf64 packing mirrors the raster pass: (depth<<32) | (slot<<7 | tri).

struct ScreenUniforms {
    size: vec2<u32>,
    material_id: u32,
    debug_mode: u32,
}

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, read>;
@group(0) @binding(1) var<uniform> screen: ScreenUniforms;
@group(0) @binding(2) var density_accumulator: texture_storage_2d<r32uint, read>;
@group(0) @binding(3) var<storage, read> visible_meshlets: array<u32>;

struct FsInput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FsInput {
    var out: FsInput;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

fn hash_to_rgb(x: u32) -> vec3<f32> {
    var h = x;
    h ^= h >> 16u;
    h = h * 0x7feb352du;
    h ^= h >> 15u;
    h = h * 0x846ca68bu;
    h ^= h >> 16u;
    let r = f32(h & 0xffu) / 255.0;
    let g = f32((h >> 8u) & 0xffu) / 255.0;
    let b = f32((h >> 16u) & 0xffu) / 255.0;
    return vec3<f32>(r, g, b) * 0.8 + 0.2;
}

// 5-stop perceptual gradient for density / overdraw heatmaps.
fn density_heatmap(t: f32) -> vec3<f32> {
    let r = clamp(2.0 * t - 1.0, 0.0, 1.0);
    let g = clamp(1.0 - 2.0 * abs(t - 0.5), 0.0, 1.0);
    let b = clamp(1.0 - 2.0 * t, 0.0, 1.0);
    return vec3<f32>(r, g, b);
}

@fragment
fn fs_debug(in: FsInput) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.position.xy);
    let packed = textureLoad(vbuf64, pixel).x;
    if (packed == 0lu) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let packed_ids = u32(packed);
    let visible_slot = packed_ids >> 7u;
    let packed_visible = visible_meshlets[visible_slot];
    let inst_id = packed_visible >> 16u;
    let meshlet_id = packed_visible & 0xffffu;

    var rgb: vec3<f32>;
    if (screen.debug_mode == 1u) {
        rgb = hash_to_rgb(meshlet_id);
    } else if (screen.debug_mode == 2u) {
        rgb = hash_to_rgb(inst_id);
    } else if (screen.debug_mode == 3u) {
        // TriangleDensity — per-pixel contribution count.
        let count = textureLoad(density_accumulator, pixel).x;
        const MAX_DENSITY: f32 = 32.0;
        let t = clamp(f32(count) / MAX_DENSITY, 0.0, 1.0);
        rgb = density_heatmap(t);
    } else if (screen.debug_mode == 4u) {
        // Overdraw — winning-fragment overwrite count.
        let wins = textureLoad(density_accumulator, pixel).x;
        const MAX_OVERDRAW: f32 = 8.0;
        let t = clamp(f32(wins) / MAX_OVERDRAW, 0.0, 1.0);
        rgb = density_heatmap(t);
    } else {
        // CullPassthrough (7) and any other colorize mode routed here:
        // flat green for every vbuf-covered pixel.
        rgb = vec3<f32>(0.0, 1.0, 0.0);
    }

    return vec4<f32>(rgb, 1.0);
}
