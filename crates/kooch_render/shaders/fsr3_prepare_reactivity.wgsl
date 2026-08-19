// fsr3_prepare_reactivity.wgsl — FSR 3.1's second pass (#481).
//
// Transliterated from `ffx_fsr3upscaler_prepare_reactivity.h`
// (AMD FSR SDK 2.3.0).
//
//   Copyright (C) 2026 Advanced Micro Devices, Inc.
//   SPDX-License-Identifier: MIT
//
// See NOTICE at the repository root. Concatenated after
// `fsr3_common.wgsl`.
//
// # What this pass decides
//
// One `rgba16float` per render pixel, holding the four numbers that
// tell the accumulation how much it is allowed to trust the past:
//
// - **reactive** — this pixel's motion vector disagrees with the one
//   at the place it claims to come from, so the velocity field is
//   lying here (particles, transparencies, anything the raster pass
//   cannot write a velocity for).
// - **disocclusion** — the scattered depth from the previous frame
//   says nothing was here, so there IS no history to blend.
// - **shading change** — 🔴 always zero in this port, see below.
// - **accumulation** — how many frames of history this pixel has
//   earned, in units of a third of a frame.
//
// And it writes the **locks**: FSR's answer to thin geometry, and step
// 6 of #481. A one-pixel wire is a ridge in luma that no neighbour
// shares; `thin_feature_confidence` finds that pattern and pins the
// pixel so the history cannot dissolve it.
//
// # 🔴 Two inputs this engine does not have, stated rather than hidden
//
// 1. **The application reactive mask.** FSR expects the game to mark
//    its own particles and transparencies. Nothing renders one here,
//    so the term is dropped and reactivity comes from motion
//    divergence alone.
// 2. **The shading-change pyramid.** FSR runs two more SPD passes to
//    detect a surface whose SHADING changed while its geometry did
//    not — a light turning on, a shadow sweeping past — and resets the
//    accumulation there. Without it, such a pixel keeps blending its
//    stale colour for a few frames and reads as a smear.
//
// Both are additive: every threshold that consumes them behaves as if
// the answer were "no reaction needed", which is the correct default
// and not a broken one. They are the first thing to add if the image
// disappoints.

@group(0) @binding(1) var dilated: texture_2d<f32>;
@group(0) @binding(2) var dilated_depth: texture_2d<f32>;
@group(0) @binding(3) var reconstructed_depth: texture_2d<u32>;
@group(0) @binding(4) var current_luma: texture_2d<f32>;
@group(0) @binding(5) var accumulation_prev: texture_2d<f32>;
@group(0) @binding(6) var linear_sampler: sampler;
@group(0) @binding(7) var reactive_masks: texture_storage_2d<rgba16float, write>;
@group(0) @binding(8) var accumulation_next: texture_storage_2d<rgba16float, write>;
@group(0) @binding(9) var new_locks: texture_storage_2d<r32float, write>;

/// FSR's default. A pixel earns a third of a frame of trust per frame,
/// so three clean frames buy full accumulation.
const ACCUMULATION_ADDED_PER_FRAME: f32 = 0.333;
/// A disocclusion does not merely reset the counter, it puts it in
/// debt: one frame of penalty before accumulation may start again.
const MIN_DISOCCLUSION_ACCUMULATION: f32 = -0.333;

fn load_dilated_motion(pos: vec2<i32>) -> vec2<f32> {
    return textureLoad(dilated, pos, 0).xy;
}

fn load_reconstructed_prev_depth(pos: vec2<i32>) -> f32 {
    return bitcast<f32>(textureLoad(reconstructed_depth, pos, 0).x);
}

/// How much of the history at this pixel was actually visible last
/// frame. 1.0 means fully disoccluded — nothing to blend.
///
/// The test is a depth comparison in VIEW space against the depth this
/// frame scattered into the previous grid, with a separation threshold
/// that grows with distance: two surfaces a centimetre apart at forty
/// metres are the same surface as far as a 720p pixel is concerned.
fn compute_disocclusion(uv: vec2<f32>, motion_in: vec2<f32>, current_view_depth: f32) -> f32 {
    var motion = motion_in;
    let nearest_metres = min(current_view_depth, FSR3_FP16_MAX);
    motion *= f32(velocity_4k(motion) > reconstructed_depth_mv_threshold(nearest_metres));

    let taps = bilinear_taps(uv + motion, params.render_size);
    let size = render_size_i();

    var disocclusion = 0.0;
    var weight_sum = 0.0;
    var potential = true;

    for (var i = 0; i < 4 && potential; i++) {
        let sample_pos = clamp_load(taps.base_pos, bilinear_offset(i), size);
        if (is_on_screen(sample_pos, size)) {
            let weight = taps.weights[i];
            if (weight > RECONSTRUCTED_DEPTH_WEIGHT_THRESHOLD) {
                let prev_view_depth = view_depth(load_reconstructed_prev_depth(sample_pos));
                let difference = current_view_depth - prev_view_depth;

                potential = potential && (difference > FSR3_FP32_MIN);

                if (potential) {
                    let half_viewport = length(params.render_size * 0.5);
                    let threshold = max(current_view_depth, prev_view_depth);

                    const K_SEP: f32 = 1.37e-05;
                    let required = K_SEP * half_viewport * threshold;

                    disocclusion += saturate(required / difference) * weight;
                    weight_sum += weight;
                }
            }
        }
    }

    if (potential && weight_sum > 0.0) {
        return saturate(1.0 - disocclusion / weight_sum);
    }
    return 0.0;
}

/// Reactivity without an application mask: how much this pixel's own
/// velocity disagrees with the velocity found where it says it came
/// from. A particle drawn without motion vectors reprojects onto
/// something moving at a different rate, and that disagreement is the
/// signal.
fn compute_motion_divergence(uv: vec2<f32>, motion: vec2<f32>, current_depth: f32) -> f32 {
    let velocity = velocity_4k(motion);
    // ⚠️ Divergence from FSR, deliberately. Theirs divides by this
    // without a guard, and at zero velocity that is 0/0 — NaN times the
    // zero velocity factor is still NaN, which would leak into the
    // reactive mask. The value the arithmetic INTENDS at zero velocity
    // is zero, so it is returned directly.
    if (velocity <= 0.0) {
        return 0.0;
    }

    let reprojected_pos = vec2<i32>((uv + motion) * params.render_size);
    let reprojected_depth = textureLoad(dilated_depth, reprojected_pos, 0).x;
    let reprojected_motion = load_dilated_motion(reprojected_pos);

    let reprojected_velocity = velocity_4k(reprojected_motion);

    let nucleus_metres = view_depth(reprojected_depth);
    let current_metres = view_depth(current_depth);

    let distance_factor = min_over_max(nucleus_metres, current_metres, 0.0);
    let velocity_factor = saturate(velocity / 10.0);

    return (1.0 - saturate(reprojected_velocity / velocity)) * distance_factor * velocity_factor;
}

/// The thin-feature detector — step 6 of #481.
///
/// Nine luma samples. The centre is a "ridge" if it is brighter than
/// every dissimilar neighbour or darker than all of them. That alone
/// would fire on any edge, so four quadrant masks reject it: if a whole
/// corner of the 3×3 agrees with the centre, this is the inside of a
/// shape rather than a wire across it, and no lock is placed.
fn thin_feature_confidence(px_pos: vec2<i32>) -> f32 {
    // 1 2 3
    // 4 0 5
    // 6 7 8
    let offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0),
        vec2<i32>(-1, -1),
        vec2<i32>(0, -1),
        vec2<i32>(1, -1),
        vec2<i32>(-1, 0),
        vec2<i32>(1, 0),
        vec2<i32>(-1, 1),
        vec2<i32>(0, 1),
        vec2<i32>(1, 1),
    );

    let size = render_size_i();
    var samples: array<f32, 9>;
    var luma_min = 3.402823466e+38;
    var luma_max = FSR3_FP32_MIN;

    for (var i = 0; i < 9; i++) {
        let pos = clamp_load(px_pos, offsets[i], size);
        samples[i] = textureLoad(current_luma, pos, 0).x * params.exposure;
        luma_min = min(luma_min, samples[i]);
        luma_max = max(luma_max, samples[i]);
    }

    const THRESHOLD: f32 = 0.9;
    var dissimilar_min = 3.402823466e+38;
    var dissimilar_max = 0.0;

    // Bit 0 is the centre, always "similar to itself".
    var pattern = 1u;
    for (var i = 1; i < 9; i++) {
        let difference = abs(samples[i] - samples[0]) / (luma_max - luma_min);
        if (difference < THRESHOLD) {
            pattern |= 1u << u32(i);
        } else {
            dissimilar_min = min(dissimilar_min, samples[i]);
            dissimilar_max = max(dissimilar_max, samples[i]);
        }
    }

    let is_ridge = samples[0] > dissimilar_max || samples[0] < dissimilar_min;
    if (!is_ridge) {
        return 0.0;
    }

    let rejection = array<u32, 4>(
        (1u << 1u) | (1u << 2u) | (1u << 4u) | 1u, // upper left
        (1u << 2u) | (1u << 3u) | (1u << 5u) | 1u, // upper right
        (1u << 4u) | (1u << 6u) | (1u << 7u) | 1u, // lower left
        (1u << 5u) | (1u << 7u) | (1u << 8u) | 1u, // lower right
    );
    for (var i = 0; i < 4; i++) {
        if ((pattern & rejection[i]) == rejection[i]) {
            return 0.0;
        }
    }

    return 1.0 - luma_min / luma_max;
}

/// Advances this pixel's trust counter and stores next frame's.
///
/// FSR's own worked example: with a shading change at frame N and
/// nothing after it, accumulation runs 0.000, 0.333, 0.666, 0.999. With
/// a disocclusion instead it starts at −0.333, so the first frame after
/// is 0.000 — the penalty costs exactly one frame.
fn update_accumulation(
    px_pos: vec2<i32>,
    uv: vec2<f32>,
    motion: vec2<f32>,
    disocclusion: f32,
    shading_change: f32,
) -> f32 {
    let reprojected_uv = uv + motion;
    var accumulation = 0.0;

    if (is_uv_inside(reprojected_uv)) {
        let hw_uv = clamp_uv(reprojected_uv, params.render_size);
        accumulation = saturate(textureSampleLevel(accumulation_prev, linear_sampler, hw_uv, 0.0).x);
    }

    accumulation = mix(accumulation, 0.0, shading_change);
    accumulation = mix(
        accumulation,
        min(MIN_DISOCCLUSION_ACCUMULATION, accumulation),
        disocclusion,
    );
    accumulation *= f32(round(accumulation * 100.0) > 1.0);

    textureStore(
        accumulation_next,
        px_pos,
        vec4<f32>(saturate(accumulation + ACCUMULATION_ADDED_PER_FRAME), 0.0, 0.0, 0.0),
    );

    return accumulation;
}

@compute @workgroup_size(8, 8, 1)
fn prepare_reactivity(@builtin(global_invocation_id) id: vec3<u32>) {
    let px_pos = vec2<i32>(id.xy);
    if (!is_on_screen(px_pos, render_size_i())) {
        return;
    }

    let uv = (vec2<f32>(px_pos) + 0.5) * params.render_size_rcp;
    let motion = load_dilated_motion(px_pos);
    let depth = textureLoad(dilated_depth, px_pos, 0).x;

    let disocclusion = compute_disocclusion(uv, motion, view_depth(depth));
    // Would be `max(dilated application mask, sampled shading change)`.
    // Both are absent; see the header.
    let shading_change = 0.0;
    let reactive = compute_motion_divergence(uv, motion, depth);
    let accumulation = update_accumulation(px_pos, uv, motion, disocclusion, shading_change);

    var out: vec4<f32>;
    out[REACTIVE] = reactive;
    out[DISOCCLUSION] = disocclusion;
    out[SHADING_CHANGE] = shading_change;
    out[ACCUMULATION] = accumulation;
    textureStore(reactive_masks, px_pos, out);

    let lock_strength = thin_feature_confidence(px_pos);
    if (lock_strength > 0.01) {
        let hr_pos = hr_pos_from_lr_pos(px_pos);
        if (is_on_screen(hr_pos, output_size_i())) {
            // Several render pixels can map to one output pixel and the
            // last writer wins, which is FSR's behaviour too — a lock is
            // a hint, not an accumulator.
            textureStore(new_locks, hr_pos, vec4<f32>(lock_strength, 0.0, 0.0, 0.0));
        }
    }
}
