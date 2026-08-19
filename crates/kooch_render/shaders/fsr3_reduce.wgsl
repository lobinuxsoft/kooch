// fsr3_reduce.wgsl — the two small passes FSR hides inside bigger ones.
//
// Derived from `ffx_fsr3upscaler_luma_pyramid.h` (AMD FSR SDK 2.3.0).
//
//   Copyright (C) 2026 Advanced Micro Devices, Inc.
//   SPDX-License-Identifier: MIT
//
// See NOTICE at the repository root. Concatenated after
// `fsr3_common.wgsl`.
//
// # Why these are separate here and not there
//
// FSR builds a six-level luma pyramid with SPD — one dispatch, group
// atomics, the whole apparatus — because it needs the 1×1 level for
// auto-exposure and the middle levels for shading-change detection.
// This port takes exposure from the engine and does not do
// shading-change detection, so **exactly one level of that pyramid is
// still load-bearing**: mip 1 of the farthest depth, which the
// accumulation reads to widen its clipping box on nearby surfaces.
//
// One level of a pyramid is a 2×2 average. SPD would be a very large
// hammer for it.

@group(0) @binding(1) var dilated: texture_2d<f32>;
@group(0) @binding(2) var farthest_mip1_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var new_locks: texture_storage_2d<r32float, write>;

/// Half-resolution farthest depth, in metres. The `.z` of the dilated
/// target, box-filtered, which is what SPD's first reduction does.
@compute @workgroup_size(8, 8, 1)
fn farthest_depth_mip1(@builtin(global_invocation_id) id: vec3<u32>) {
    let px_pos = vec2<i32>(id.xy);
    let half_size = max(render_size_i() / 2, vec2<i32>(1));
    if (!is_on_screen(px_pos, half_size)) {
        return;
    }

    let base = px_pos * 2;
    let size = render_size_i();
    var sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let pos = clamp_load(base, bilinear_offset(i), size);
        sum += textureLoad(dilated, pos, 0).z;
    }

    textureStore(farthest_mip1_out, px_pos, vec4<f32>(sum * 0.25, 0.0, 0.0, 0.0));
}

/// Locks live at OUTPUT resolution and are written by a pass that runs
/// at render resolution, so most output pixels are never visited and
/// would otherwise keep last frame's value forever.
@compute @workgroup_size(8, 8, 1)
fn clear_new_locks(@builtin(global_invocation_id) id: vec3<u32>) {
    let px_pos = vec2<i32>(id.xy);
    if (is_on_screen(px_pos, output_size_i())) {
        textureStore(new_locks, px_pos, vec4<f32>(0.0));
    }
}
