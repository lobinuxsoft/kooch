// sgsr2_convert.wgsl — pass 1 of the temporal upscaler (#481, step 4).
//
// Transliterated from Snapdragon Game Super Resolution 2,
// `sgsr/v2/include/glsl_2_pass_fs/sgsr2_convert.fs`.
//
//   Copyright (c) 2024, Qualcomm Innovation Center, Inc. All rights reserved.
//   SPDX-License-Identifier: BSD-3-Clause
//
// See NOTICE at the repository root for the full licence text. Ported
// rather than invented for the reason the whole plan rests on: the
// constants below took someone a year to find and cannot be guessed.
//
// # Why SGSR 2 and not FSR 3.1
//
// Both are open and both are analytical. FSR 3.1 is thousands of lines
// of HLSL behind the `FFX_` macro system across seven passes and is
// designed for a desktop discrete GPU; SGSR 2 is 398 lines of plain
// GLSL ES in two passes and is designed for a phone's power budget,
// which is the budget this engine is actually failing to meet — 40.7 ms
// against 13.9. Quality is FSR's; cost is what we are short of.
//
// # 🔴 Three things that do NOT transliterate, and all fail silently
//
// **1. Depth is reversed here and standard there.** Their `min` is our
// `max`. Rather than invert every comparison — which is the version that
// looks right and is wrong in one place nobody finds — the taps are
// converted on read: `d_std = 1 - d_rev`. That identity is EXACT for
// this engine, and only because the camera builds its matrix with
// `perspective_infinite_rh_reverse_z`: an infinite far plane maps
// reversed-Z to `near/distance` and standard depth to `1 - near/distance`,
// so the two are exact complements. ⚠️ A finite far plane breaks this,
// and it breaks it by a small amount everywhere, which is the worst kind.
//
// **2. Their colour is display-referred; ours is radiance in the
// hundreds.** That does not bite in this pass — no colour here — but it
// is the whole story in the upscale pass. Written down here because the
// two shaders are a pair.
//
// **3. Their motion is NDC, ours is UV.** Theirs carries a `0.5` factor
// at the point of use for exactly that reason. Ours does not need it,
// and the missing factor of two would read as a reprojection that
// tracks at half speed — a smear, not an error.
//
// # What this pass does NOT do, unlike upstream
//
// SGSR 2 reconstructs camera motion from a `clipToPrevClip` matrix when
// no velocity texture is bound. This engine always has real per-pixel
// motion vectors (#868), which also carry OBJECT motion that a camera
// matrix cannot know, so that half is dropped rather than ported.

struct ConvertUniforms {
    render_size: vec2<f32>,
    render_size_rcp: vec2<f32>,
    // `tan(fov_horizontal / 2)`. Upstream calls it `cameraFovAngleHor`
    // and feeds it `tan(fov_vertical / 2) * aspect`, which is the same
    // number. It scales the depth-separation threshold below, so a wider
    // lens tolerates a larger depth step before calling it an edge.
    fov_k: f32,
    _pad: f32,
}

@group(0) @binding(0) var input_depth: texture_depth_2d;
@group(0) @binding(1) var input_motion: texture_2d<f32>;
@group(0) @binding(2) var point_sampler: sampler;
@group(0) @binding(3) var<uniform> params: ConvertUniforms;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_convert(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

@fragment
fn fs_convert(in: Varyings) -> @location(0) vec4<f32> {
    let uv = in.uv;

    // Four gathers cover a 4x4 depth neighbourhood in four fetches
    // instead of sixteen. The naming and the component picks are
    // upstream's, kept literal: the four texels called out of the four
    // gathers are the CENTRE 2x2 of that 4x4, and getting the component
    // indices wrong samples a plausible but different neighbourhood.
    //
    //      a  b  c  d
    //      e  f  g  h
    //      i  j  k  l
    //      m  n  o  p
    let gather_uv = uv - vec2<f32>(0.5) * params.render_size_rcp;
    let dx = vec2<f32>(params.render_size_rcp.x * 2.0, 0.0);
    let dy = vec2<f32>(0.0, params.render_size_rcp.y * 2.0);

    // 🔴 The `1.0 -` on each gather is the reversed-Z conversion, and it
    // is done here rather than by inverting every comparison below —
    // which is the version that looks right and is wrong in one place
    // nobody finds. Exact for this engine's infinite far plane; see the
    // header.
    let btm_left = vec4<f32>(1.0) - textureGather(input_depth, point_sampler, gather_uv);
    let btm_right = vec4<f32>(1.0) - textureGather(input_depth, point_sampler, gather_uv + dx);
    let top_left = vec4<f32>(1.0) - textureGather(input_depth, point_sampler, gather_uv + dy);
    let top_right = vec4<f32>(1.0) - textureGather(input_depth, point_sampler, gather_uv + dx + dy);

    // Converted above, so from here down `min` means nearest, exactly as
    // it does upstream.
    let nearest_centre = min(min(min(btm_left.z, btm_right.w), top_left.y), top_right.x);

    var depth_clip = 0.0;
    if (nearest_centre < 1.0 - 1.0e-05) {
        let btm_left_4 = min(min(min(btm_left.y, btm_left.x), btm_left.z), btm_left.w);
        let btm_right_4 = min(min(min(btm_right.y, btm_right.x), btm_right.z), btm_right.w);
        let top_left_4 = min(min(min(top_left.y, top_left.x), top_left.z), top_left.w);
        let top_right_4 = min(min(min(top_right.y, top_right.x), top_right.z), top_right.w);

        // 🔴 AMD's and Qualcomm's tuned constant, and the reason this is
        // a transliteration. `K_SEP` scaled by the lens and by the
        // screen diagonal turns into a depth step measured in units of
        // the depth buffer: how far two surfaces must separate before
        // the pixel between them is an edge rather than a slope.
        //
        // 🔴 This is the analytical answer to the same question the
        // hand-rolled 10 % ratio in `taa.wgsl` answers. Theirs knows
        // about the lens and the resolution; ours does not.
        const K_SEP: f32 = 1.37e-05;
        const EPSILON: f32 = 1.19e-07;
        let diagonal = length(params.render_size);
        let separation = K_SEP * params.fov_k * diagonal * (1.0 - nearest_centre);

        var w_depth = 0.0;
        w_depth += clamp(separation / (abs(nearest_centre - btm_left_4) + EPSILON), 0.0, 1.0);
        w_depth += clamp(separation / (abs(nearest_centre - btm_right_4) + EPSILON), 0.0, 1.0);
        w_depth += clamp(separation / (abs(nearest_centre - top_left_4) + EPSILON), 0.0, 1.0);
        w_depth += clamp(separation / (abs(nearest_centre - top_right_4) + EPSILON), 0.0, 1.0);
        depth_clip = clamp(1.0 - w_depth * 0.25, 0.0, 1.0);
    }

    // Motion dilated to the nearest of a 3x3, which is the half of
    // upstream's dilation that survives having real velocity vectors.
    // Same reasoning as the resolve's, and the same reach: one texel,
    // because a winner two texels away can belong to a surface that
    // does not touch this pixel.
    var closest_uv = uv;
    var closest = textureSampleLevel(input_depth, point_sampler, uv, 0i);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let tap_uv = uv + vec2<f32>(f32(x), f32(y)) * params.render_size_rcp;
            let tap = textureSampleLevel(input_depth, point_sampler, tap_uv, 0i);
            // Reversed-Z again — this reads the depth texture directly,
            // so here greater IS nearer.
            if (tap > closest) {
                closest = tap;
                closest_uv = tap_uv;
            }
        }
    }
    let motion = textureSampleLevel(input_motion, point_sampler, closest_uv, 0.0).rg;

    // xy: dilated motion, in UV. z: how much this pixel is allowed to
    // trust its history. w: unused, and kept so the target is a shape
    // every backend agrees on.
    return vec4<f32>(motion, depth_clip, 0.0);
}
