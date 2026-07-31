// hi_z_spd.wgsl — Hi-Z depth pyramid via FidelityFX SPD (Single-Pass
// Downsampler).
//
// Adapted from Bevy's `downsample_depth.wgsl`, which itself ports
// AMD's FFX SPD v2.1
// (https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/d7531ae47d8b36a5d4025663e731a47a38be882f/sdk/include/FidelityFX/gpu/spd/ffx_spd.h#L528).
//
// Two key adaptations from Bevy:
//
// 1. **Reduction function is `max`, not `min`.** Bevy's Hi-Z is built
//    for cluster occlusion where the conservative test is "sphere
//    bound's CLOSEST depth >= tile's MIN depth" (i.e. nothing
//    closer in the tile is rendered, so the sphere can't be
//    occluded). Our cull shader is the inverse — see
//    `meshlet_cull.wgsl::occluded_by_hi_z_atomic`: the test is
//    `meshlet_centre_depth > tile_MAX_depth` (i.e. every fragment in
//    the tile is in front of the meshlet, so the meshlet is
//    occluded). Max-reduction keeps the FARTHEST depth per tile and
//    that's what `occluded_by_hi_z_atomic` reads.
//
// 2. **No multisample / meshlet visibility variants.** We always
//    sample a single-sample Depth32Float attachment via the source
//    binding — the engine's depth attachment is single-sample
//    (`crates/kooch_render/src/meshlet/render_stage/mod.rs` creates
//    `meshlet_render_stage_depth` with `sample_count: 1`). Drop the
//    Bevy `#ifdef MULTISAMPLE` / `#ifdef MESHLET*` paths.
//
// Two compute entries (matches Bevy's split — wgpu doesn't yet
// expose globally coherent storage buffers needed for a true
// single-pass dispatch):
//
//   * `cs_downsample_first` — one workgroup per 64×64 source tile,
//     writes mips 1..6 from `mip_0` using workgroup memory.
//   * `cs_downsample_second` — one workgroup total, writes mips
//     7..12 from `mip_6` using workgroup memory.
//
// `mip_0` (the depth source) may be any size; `mip_1..mip_12` must
// have side lengths rounded DOWN to the previous power of two so the
// 2×2 reductions stay aligned. The cull shader's pixel-radius math
// already tolerates the divergence between actual viewport size and
// pyramid size (see `occluded_by_hi_z_atomic`).
//
// Refs: #486 (this port), #445 (the per-mip approach this replaces).

@group(0) @binding(0) var mip_0: texture_depth_2d;
@group(0) @binding(1) var mip_1: texture_storage_2d<r32float, write>;
@group(0) @binding(2) var mip_2: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var mip_3: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var mip_4: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var mip_5: texture_storage_2d<r32float, write>;
@group(0) @binding(6) var mip_6: texture_storage_2d<r32float, read_write>;
@group(0) @binding(7) var mip_7: texture_storage_2d<r32float, write>;
@group(0) @binding(8) var mip_8: texture_storage_2d<r32float, write>;
@group(0) @binding(9) var mip_9: texture_storage_2d<r32float, write>;
@group(0) @binding(10) var mip_10: texture_storage_2d<r32float, write>;
@group(0) @binding(11) var mip_11: texture_storage_2d<r32float, write>;
@group(0) @binding(12) var mip_12: texture_storage_2d<r32float, write>;
@group(0) @binding(13) var samplr: sampler;

struct Constants {
    /// Highest mip level the pyramid actually has (0-indexed). The
    /// shader early-outs once it walks past this so a 64-px pyramid
    /// (7 mips) doesn't try to write into mip_7+.
    max_mip_level: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}
@group(0) @binding(14) var<uniform> constants: Constants;

var<workgroup> intermediate_memory: array<array<f32, 16>, 16>;

@compute
@workgroup_size(256, 1, 1)
fn cs_downsample_first(
    @builtin(workgroup_id) workgroup_id: vec3u,
    @builtin(local_invocation_index) local_invocation_index: u32,
) {
    let sub_xy = remap_for_wave_reduction(local_invocation_index % 64u);
    let x = sub_xy.x + 8u * ((local_invocation_index >> 6u) % 2u);
    let y = sub_xy.y + 8u * (local_invocation_index >> 7u);

    downsample_mips_0_and_1(x, y, workgroup_id.xy, local_invocation_index);
    downsample_mips_2_to_5(x, y, workgroup_id.xy, local_invocation_index);
}

@compute
@workgroup_size(256, 1, 1)
fn cs_downsample_second(@builtin(local_invocation_index) local_invocation_index: u32) {
    let sub_xy = remap_for_wave_reduction(local_invocation_index % 64u);
    let x = sub_xy.x + 8u * ((local_invocation_index >> 6u) % 2u);
    let y = sub_xy.y + 8u * (local_invocation_index >> 7u);

    downsample_mips_6_and_7(x, y);
    downsample_mips_8_to_11(x, y, local_invocation_index);
}

fn downsample_mips_0_and_1(x: u32, y: u32, workgroup_id: vec2u, local_invocation_index: u32) {
    var v: vec4f;

    var tex = vec2(workgroup_id * 64u) + vec2(x * 2u, y * 2u);
    var pix = vec2(workgroup_id * 32u) + vec2(x, y);
    v[0] = reduce_load_mip_0(tex);
    textureStore(mip_1, pix, vec4(v[0]));

    tex = vec2(workgroup_id * 64u) + vec2(x * 2u + 32u, y * 2u);
    pix = vec2(workgroup_id * 32u) + vec2(x + 16u, y);
    v[1] = reduce_load_mip_0(tex);
    textureStore(mip_1, pix, vec4(v[1]));

    tex = vec2(workgroup_id * 64u) + vec2(x * 2u, y * 2u + 32u);
    pix = vec2(workgroup_id * 32u) + vec2(x, y + 16u);
    v[2] = reduce_load_mip_0(tex);
    textureStore(mip_1, pix, vec4(v[2]));

    tex = vec2(workgroup_id * 64u) + vec2(x * 2u + 32u, y * 2u + 32u);
    pix = vec2(workgroup_id * 32u) + vec2(x + 16u, y + 16u);
    v[3] = reduce_load_mip_0(tex);
    textureStore(mip_1, pix, vec4(v[3]));

    if constants.max_mip_level <= 1u { return; }

    for (var i = 0u; i < 4u; i++) {
        intermediate_memory[x][y] = v[i];
        workgroupBarrier();
        if local_invocation_index < 64u {
            v[i] = reduce_4(vec4(
                intermediate_memory[x * 2u + 0u][y * 2u + 0u],
                intermediate_memory[x * 2u + 1u][y * 2u + 0u],
                intermediate_memory[x * 2u + 0u][y * 2u + 1u],
                intermediate_memory[x * 2u + 1u][y * 2u + 1u],
            ));
            pix = (workgroup_id * 16u) + vec2(
                x + (i % 2u) * 8u,
                y + (i / 2u) * 8u,
            );
            textureStore(mip_2, pix, vec4(v[i]));
        }
        workgroupBarrier();
    }

    if local_invocation_index < 64u {
        intermediate_memory[x + 0u][y + 0u] = v[0];
        intermediate_memory[x + 8u][y + 0u] = v[1];
        intermediate_memory[x + 0u][y + 8u] = v[2];
        intermediate_memory[x + 8u][y + 8u] = v[3];
    }
}

fn downsample_mips_2_to_5(x: u32, y: u32, workgroup_id: vec2u, local_invocation_index: u32) {
    if constants.max_mip_level <= 2u { return; }
    workgroupBarrier();
    downsample_mip_2(x, y, workgroup_id, local_invocation_index);

    if constants.max_mip_level <= 3u { return; }
    workgroupBarrier();
    downsample_mip_3(x, y, workgroup_id, local_invocation_index);

    if constants.max_mip_level <= 4u { return; }
    workgroupBarrier();
    downsample_mip_4(x, y, workgroup_id, local_invocation_index);

    if constants.max_mip_level <= 5u { return; }
    workgroupBarrier();
    downsample_mip_5(workgroup_id, local_invocation_index);
}

fn downsample_mip_2(x: u32, y: u32, workgroup_id: vec2u, local_invocation_index: u32) {
    if local_invocation_index < 64u {
        let v = reduce_4(vec4(
            intermediate_memory[x * 2u + 0u][y * 2u + 0u],
            intermediate_memory[x * 2u + 1u][y * 2u + 0u],
            intermediate_memory[x * 2u + 0u][y * 2u + 1u],
            intermediate_memory[x * 2u + 1u][y * 2u + 1u],
        ));
        textureStore(mip_3, (workgroup_id * 8u) + vec2(x, y), vec4(v));
        intermediate_memory[x * 2u + y % 2u][y * 2u] = v;
    }
}

fn downsample_mip_3(x: u32, y: u32, workgroup_id: vec2u, local_invocation_index: u32) {
    if local_invocation_index < 16u {
        let v = reduce_4(vec4(
            intermediate_memory[x * 4u + 0u + 0u][y * 4u + 0u],
            intermediate_memory[x * 4u + 2u + 0u][y * 4u + 0u],
            intermediate_memory[x * 4u + 0u + 1u][y * 4u + 2u],
            intermediate_memory[x * 4u + 2u + 1u][y * 4u + 2u],
        ));
        textureStore(mip_4, (workgroup_id * 4u) + vec2(x, y), vec4(v));
        intermediate_memory[x * 4u + y][y * 4u] = v;
    }
}

fn downsample_mip_4(x: u32, y: u32, workgroup_id: vec2u, local_invocation_index: u32) {
    if local_invocation_index < 4u {
        let v = reduce_4(vec4(
            intermediate_memory[x * 8u + 0u + 0u + y * 2u][y * 8u + 0u],
            intermediate_memory[x * 8u + 4u + 0u + y * 2u][y * 8u + 0u],
            intermediate_memory[x * 8u + 0u + 1u + y * 2u][y * 8u + 4u],
            intermediate_memory[x * 8u + 4u + 1u + y * 2u][y * 8u + 4u],
        ));
        textureStore(mip_5, (workgroup_id * 2u) + vec2(x, y), vec4(v));
        intermediate_memory[x + y * 2u][0u] = v;
    }
}

fn downsample_mip_5(workgroup_id: vec2u, local_invocation_index: u32) {
    if local_invocation_index < 1u {
        let v = reduce_4(vec4(
            intermediate_memory[0u][0u],
            intermediate_memory[1u][0u],
            intermediate_memory[2u][0u],
            intermediate_memory[3u][0u],
        ));
        textureStore(mip_6, workgroup_id, vec4(v));
    }
}

fn downsample_mips_6_and_7(x: u32, y: u32) {
    var v: vec4f;

    var tex = vec2(x * 4u + 0u, y * 4u + 0u);
    var pix = vec2(x * 2u + 0u, y * 2u + 0u);
    v[0] = reduce_load_mip_6(tex);
    textureStore(mip_7, pix, vec4(v[0]));

    tex = vec2(x * 4u + 2u, y * 4u + 0u);
    pix = vec2(x * 2u + 1u, y * 2u + 0u);
    v[1] = reduce_load_mip_6(tex);
    textureStore(mip_7, pix, vec4(v[1]));

    tex = vec2(x * 4u + 0u, y * 4u + 2u);
    pix = vec2(x * 2u + 0u, y * 2u + 1u);
    v[2] = reduce_load_mip_6(tex);
    textureStore(mip_7, pix, vec4(v[2]));

    tex = vec2(x * 4u + 2u, y * 4u + 2u);
    pix = vec2(x * 2u + 1u, y * 2u + 1u);
    v[3] = reduce_load_mip_6(tex);
    textureStore(mip_7, pix, vec4(v[3]));

    if constants.max_mip_level <= 7u { return; }

    let vr = reduce_4(v);
    textureStore(mip_8, vec2(x, y), vec4(vr));
    intermediate_memory[x][y] = vr;
}

fn downsample_mips_8_to_11(x: u32, y: u32, local_invocation_index: u32) {
    if constants.max_mip_level <= 8u { return; }
    workgroupBarrier();
    downsample_mip_8(x, y, local_invocation_index);

    if constants.max_mip_level <= 9u { return; }
    workgroupBarrier();
    downsample_mip_9(x, y, local_invocation_index);

    if constants.max_mip_level <= 10u { return; }
    workgroupBarrier();
    downsample_mip_10(x, y, local_invocation_index);

    if constants.max_mip_level <= 11u { return; }
    workgroupBarrier();
    downsample_mip_11(local_invocation_index);
}

fn downsample_mip_8(x: u32, y: u32, local_invocation_index: u32) {
    if local_invocation_index < 64u {
        let v = reduce_4(vec4(
            intermediate_memory[x * 2u + 0u][y * 2u + 0u],
            intermediate_memory[x * 2u + 1u][y * 2u + 0u],
            intermediate_memory[x * 2u + 0u][y * 2u + 1u],
            intermediate_memory[x * 2u + 1u][y * 2u + 1u],
        ));
        textureStore(mip_9, vec2(x, y), vec4(v));
        intermediate_memory[x * 2u + y % 2u][y * 2u] = v;
    }
}

fn downsample_mip_9(x: u32, y: u32, local_invocation_index: u32) {
    if local_invocation_index < 16u {
        let v = reduce_4(vec4(
            intermediate_memory[x * 4u + 0u + 0u][y * 4u + 0u],
            intermediate_memory[x * 4u + 2u + 0u][y * 4u + 0u],
            intermediate_memory[x * 4u + 0u + 1u][y * 4u + 2u],
            intermediate_memory[x * 4u + 2u + 1u][y * 4u + 2u],
        ));
        textureStore(mip_10, vec2(x, y), vec4(v));
        intermediate_memory[x * 4u + y][y * 4u] = v;
    }
}

fn downsample_mip_10(x: u32, y: u32, local_invocation_index: u32) {
    if local_invocation_index < 4u {
        let v = reduce_4(vec4(
            intermediate_memory[x * 8u + 0u + 0u + y * 2u][y * 8u + 0u],
            intermediate_memory[x * 8u + 4u + 0u + y * 2u][y * 8u + 0u],
            intermediate_memory[x * 8u + 0u + 1u + y * 2u][y * 8u + 4u],
            intermediate_memory[x * 8u + 4u + 1u + y * 2u][y * 8u + 4u],
        ));
        textureStore(mip_11, vec2(x, y), vec4(v));
        intermediate_memory[x + y * 2u][0u] = v;
    }
}

fn downsample_mip_11(local_invocation_index: u32) {
    if local_invocation_index < 1u {
        let v = reduce_4(vec4(
            intermediate_memory[0u][0u],
            intermediate_memory[1u][0u],
            intermediate_memory[2u][0u],
            intermediate_memory[3u][0u],
        ));
        textureStore(mip_12, vec2(0u, 0u), vec4(v));
    }
}

fn remap_for_wave_reduction(a: u32) -> vec2u {
    return vec2(
        insertBits(extractBits(a, 2u, 3u), a, 0u, 1u),
        insertBits(extractBits(a, 3u, 3u), extractBits(a, 1u, 2u), 0u, 2u),
    );
}

fn reduce_load_mip_0(tex: vec2u) -> f32 {
    let actual_size = textureDimensions(mip_0).xy;
    let virtual_size = vec2<u32>(
        next_power_of_two(actual_size.x),
        next_power_of_two(actual_size.y),
    );
    let virtual_uv = (vec2<f32>(f32(tex.x), f32(tex.y)) + 0.5) / vec2<f32>(virtual_size);
    // textureGather on a depth_2d returns the depths of the 2×2 texel
    // square the UV lands in, in (R, G, B, A) = (T0, T1, T2, T3) order
    // (see WGSL spec). Reduce them with our chosen `reduce_4` (max).
    return reduce_4(textureGather(mip_0, samplr, virtual_uv));
}

fn reduce_load_mip_6(tex: vec2u) -> f32 {
    return reduce_4(vec4(
        textureLoad(mip_6, tex + vec2(0u, 0u)).r,
        textureLoad(mip_6, tex + vec2(0u, 1u)).r,
        textureLoad(mip_6, tex + vec2(1u, 0u)).r,
        textureLoad(mip_6, tex + vec2(1u, 1u)).r,
    ));
}

// Min-reduce under reversed-Z (#488): NDC depth 1 = near, 0 = far,
// so the FARTHEST fragment has the SMALLEST depth value. The Hi-Z
// conservative occlusion test on a reversed-Z pyramid is
// `aabb.max.z <= tile_min` — meshlet's nearest depth (= max ndc.z
// in reversed-Z) lies behind the tile's farthest fragment (= tile
// min). `min` keeps the right value.
fn reduce_4(v: vec4f) -> f32 {
    return min(min(v.x, v.y), min(v.z, v.w));
}

// Returns the next power of two of x. If x is a power of two,
// returns the *next* one (matches Bevy's port; the cull shader
// already tolerates the divergence).
fn next_power_of_two(x: u32) -> u32 {
    return 1u << (32u - countLeadingZeros(x));
}
