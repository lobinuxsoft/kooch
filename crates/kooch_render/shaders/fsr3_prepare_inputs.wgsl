// fsr3_prepare_inputs.wgsl — FSR 3.1's first pass (#481).
//
// Transliterated from `ffx_fsr3upscaler_prepare_inputs.h`
// (AMD FSR SDK 2.3.0).
//
//   Copyright (C) 2026 Advanced Micro Devices, Inc.
//   SPDX-License-Identifier: MIT
//
// See NOTICE at the repository root. Concatenated after
// `fsr3_common.wgsl`, which owns group 0 binding 0.
//
// # What this pass is for
//
// Everything downstream reprojects, and reprojection is only as good as
// the velocity it is handed. This pass produces four things from the
// raw depth, motion and colour of the frame:
//
// - **dilated motion** — the velocity of the NEAREST of a 3×3, not the
//   pixel's own. A thin foreground edge otherwise gets reprojected with
//   the background's velocity and smears.
// - **dilated depth** — that same nearest depth, kept because the next
//   pass needs to know what surface won.
// - **reconstructed previous depth** — this frame's depth SCATTERED
//   into the previous frame's grid, which is what makes FSR's
//   disocclusion test exact under any camera motion. 🎯 This is the
//   part the engine's own resolve approximates with a ratio-of-depths
//   test, and the reason that one is documented as "the cheap test".
// - **luma** — for the feature-locking pass, which needs the previous
//   frame's as well.

@group(0) @binding(1) var input_depth: texture_depth_2d;
@group(0) @binding(2) var input_motion: texture_2d<f32>;
@group(0) @binding(3) var input_colour: texture_2d<f32>;
@group(0) @binding(4) var reconstructed_depth: texture_storage_2d<r32uint, atomic>;
/// `.xy` dilated motion (UV, FSR-signed), `.z` farthest depth in
/// metres, `.w` unused. One `rgba16float` rather than FSR's three
/// separate targets because `r16float` and `rg16float` are storage
/// formats only under `TEXTURE_FORMATS_TIER1`, and this engine does not
/// require that feature — `rgba16float` is guaranteed everywhere.
@group(0) @binding(5) var dilated: texture_storage_2d<rgba16float, write>;
/// Device depth, reversed-Z. Kept at full width on purpose: near
/// values are `near / distance`, which at a kilometre is below the
/// smallest normal half.
@group(0) @binding(6) var dilated_depth: texture_storage_2d<r32float, write>;
@group(0) @binding(7) var current_luma: texture_storage_2d<r32float, write>;

/// FSR loads the engine's motion buffer through a callback; ours is the
/// place the sign flips. See decision 3 in `fsr3_common.wgsl`.
fn load_motion(pos: vec2<i32>) -> vec2<f32> {
    return -textureLoad(input_motion, pos, 0).xy;
}

struct DepthExtents {
    nearest: f32,
    nearest_coord: vec2<i32>,
    farthest: f32,
}

fn find_depth_extents(px_pos: vec2<i32>) -> DepthExtents {
    let offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0),
        vec2<i32>(1, 0),
        vec2<i32>(0, 1),
        vec2<i32>(0, -1),
        vec2<i32>(-1, 0),
        vec2<i32>(-1, 1),
        vec2<i32>(1, 1),
        vec2<i32>(-1, -1),
        vec2<i32>(1, -1),
    );

    var depth: array<f32, 9>;
    for (var i = 0; i < 9; i++) {
        depth[i] = textureLoad(input_depth, px_pos + offsets[i], 0);
    }

    var extents: DepthExtents;
    extents.nearest_coord = px_pos;
    extents.nearest = depth[0];
    extents.farthest = depth[0];

    let size = render_size_i();
    for (var i = 1; i < 9; i++) {
        let pos = px_pos + offsets[i];
        if (is_on_screen(pos, size)) {
            // Reversed-Z: greater is nearer.
            if (depth[i] > extents.nearest) {
                // ⚠️ Faithful to FSR, and it is a no-op there too. The
                // branch only runs when `depth[i]` is NEARER than the
                // running nearest, which is itself >= `depth[0]`, so
                // this `min` can never pick anything but `depth[0]`.
                // `farthest` is therefore the CENTRE pixel's depth, not
                // the neighbourhood's farthest, in AMD's shipping code
                // as much as in this one. Left as written rather than
                // "fixed", because every threshold downstream was tuned
                // against this behaviour.
                extents.farthest = min(extents.farthest, depth[i]);
                extents.nearest_coord = pos;
                extents.nearest = depth[i];
            }
        }
    }

    return extents;
}

/// Projects this frame's nearest depth into the PREVIOUS frame's grid,
/// pushing to every pixel a bilinear read would have touched. The next
/// frame reads it at its own address and compares: a surface that is
/// missing from the reconstruction was not visible last frame, which is
/// a disocclusion.
fn reconstruct_prev_depth(px_pos: vec2<i32>, depth: f32, motion_in: vec2<f32>) {
    var motion = motion_in;
    let nearest_metres = min(view_depth(depth), FSR3_FP16_MAX);
    let threshold = reconstructed_depth_mv_threshold(nearest_metres);

    // Discard motion too small to be worth the scatter.
    motion *= f32(velocity_4k(motion) > threshold);

    let uv = (vec2<f32>(px_pos) + 0.5) * params.render_size_rcp;
    let taps = bilinear_taps(uv + motion, params.render_size);

    let size = render_size_i();
    let bits = bitcast<u32>(depth);
    for (var i = 0; i < 4; i++) {
        if (taps.weights[i] > RECONSTRUCTED_DEPTH_WEIGHT_THRESHOLD) {
            let store_pos = taps.base_pos + bilinear_offset(i);
            if (is_on_screen(store_pos, size)) {
                // Reversed-Z, so the NEAREST writer must win: `max` on
                // the raw bits, which orders correctly for positive
                // floats. FSR's `InterlockedMin` is the standard-depth
                // half of the same `#if`.
                textureAtomicMax(reconstructed_depth, store_pos, bits);
            }
        }
    }
}

@compute @workgroup_size(8, 8, 1)
fn prepare_inputs(@builtin(global_invocation_id) id: vec3<u32>) {
    let px_pos = vec2<i32>(id.xy);
    if (!is_on_screen(px_pos, render_size_i())) {
        return;
    }

    let extents = find_depth_extents(px_pos);
    // Low-resolution motion vectors: the buffer is written by the
    // raster pass at render resolution, so the dilated sample is read
    // at the winning neighbour's own address.
    let dilated_motion = load_motion(extents.nearest_coord);

    reconstruct_prev_depth(px_pos, extents.nearest, dilated_motion);

    let farthest_metres = min(view_depth(extents.farthest), FSR3_FP16_MAX);
    textureStore(dilated, px_pos, vec4<f32>(dilated_motion, farthest_metres, 0.0));
    textureStore(dilated_depth, px_pos, vec4<f32>(extents.nearest, 0.0, 0.0, 0.0));

    // Linear in, linear out. A non-linear input would have to be
    // converted here and converted back on write, which is FSR's own
    // note and is not this engine's case — the HDR target is linear.
    let rgb = max(vec3<f32>(0.0), textureLoad(input_colour, px_pos, 0).rgb);
    textureStore(current_luma, px_pos, vec4<f32>(fsr3_luma(rgb), 0.0, 0.0, 0.0));
}

@compute @workgroup_size(8, 8, 1)
fn clear_reconstructed_depth(@builtin(global_invocation_id) id: vec3<u32>) {
    let px_pos = vec2<i32>(id.xy);
    if (is_on_screen(px_pos, render_size_i())) {
        // Zero is the far plane under reversed-Z, so every pixel starts
        // as "nothing was here" and any scatter beats it.
        textureStore(reconstructed_depth, px_pos, vec4<u32>(0u));
    }
}
