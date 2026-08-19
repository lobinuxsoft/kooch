// fsr3_luma_instability.wgsl — FSR 3.1's third pass (#481).
//
// Transliterated from `ffx_fsr3upscaler_luma_instability.h`
// (AMD FSR SDK 2.3.0).
//
//   Copyright (C) 2026 Advanced Micro Devices, Inc.
//   SPDX-License-Identifier: MIT
//
// See NOTICE at the repository root. Concatenated after
// `fsr3_common.wgsl`.
//
// # The failure this exists to catch
//
// A pixel whose luma oscillates — up, down, up — across four frames is
// not converging, it is flickering. That is what a sub-pixel feature
// does when the jitter sequence keeps half-hitting it, and a history
// clamp cannot tell it apart from a legitimate change.
//
// The test is deliberately crude: compare this frame against N−1, and
// if any of N−2, N−3, N−4 is a CLOSER match while moving in the same
// direction, the sequence has come back to where it was. That is an
// oscillation, and the accumulation is told to hold its history rather
// than rectify it.
//
// # ⚠️ One input FSR reads and never uses
//
// The original samples mip 1 of the farthest depth here and passes it
// into the factor computation, which ignores the parameter entirely.
// Not carried. The same mip IS used for real by the accumulation pass.

@group(0) @binding(1) var dilated: texture_2d<f32>;
@group(0) @binding(2) var current_luma: texture_2d<f32>;
@group(0) @binding(3) var luma_history_prev: texture_2d<f32>;
@group(0) @binding(4) var reactive_masks: texture_2d<f32>;
@group(0) @binding(5) var linear_sampler: sampler;
@group(0) @binding(6) var luma_history_next: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var luma_instability_out: texture_storage_2d<rgba16float, write>;

struct Instability {
    history: vec4<f32>,
    factor: f32,
}

fn compute_instability(history_in: vec4<f32>, current: f32) -> Instability {
    // Channel N holds the luma of frame N−1−N.
    var history = history_in;
    var unstable = 0.0;

    let diff0 = current - history[0];
    let similarity0 = min_over_max(current, history[0], 1.0);
    var max_similarity = similarity0;

    if (similarity0 < 1.0) {
        for (var i = 1; i < 4; i++) {
            let diff1 = current - history[i];
            let similarity1 = min_over_max(current, history[i], 0.0);
            if (sign(diff0) == sign(diff1)) {
                max_similarity = max(max_similarity, similarity1);
            }
        }
        unstable = f32(max_similarity > similarity0);
    }

    history[3] = history[2];
    history[2] = history[1];
    history[1] = history[0];
    history[0] = current;
    history /= params.exposure;

    var out: Instability;
    out.history = history;
    // Four frames of history are needed before the answer means
    // anything; the oldest channel being zero says there are not four.
    out.factor = unstable * f32(history[3] != 0.0);
    return out;
}

@compute @workgroup_size(8, 8, 1)
fn luma_instability(@builtin(global_invocation_id) id: vec3<u32>) {
    let px_pos = vec2<i32>(id.xy);
    if (!is_on_screen(px_pos, render_size_i())) {
        return;
    }

    var data: Instability;
    data.history = vec4<f32>(0.0);
    data.factor = 0.0;

    let motion = textureLoad(dilated, px_pos, 0).xy;
    let uv = (vec2<f32>(px_pos) + 0.5) * params.render_size_rcp;
    let uv_jittered = uv + params.jitter * params.render_size_rcp;
    let uv_prev_jittered = uv + params.prev_jitter * params.render_size_rcp;
    let reprojected_uv = uv_prev_jittered + motion;

    if (is_uv_inside(reprojected_uv)) {
        let masks_uv = clamp_uv(uv_jittered, params.render_size);
        let masks = textureSampleLevel(reactive_masks, linear_sampler, masks_uv, 0.0);
        let reactive = saturate(masks[REACTIVE]);
        let disocclusion = saturate(masks[DISOCCLUSION]);
        let shading_change = saturate(masks[SHADING_CHANGE]);
        let accumulation = saturate(masks[ACCUMULATION]);

        // Only ask the question of a pixel that has actually settled.
        if (accumulation > 0.9) {
            let luma_uv = clamp_uv(uv_jittered, params.render_size);
            let current = textureSampleLevel(current_luma, linear_sampler, luma_uv, 0.0).x
                * params.exposure;

            let history_uv = clamp_uv(reprojected_uv, params.render_size);
            let history = textureSampleLevel(luma_history_prev, linear_sampler, history_uv, 0.0)
                * params.delta_pre_exposure
                * params.exposure;

            data = compute_instability(history, current);

            let velocity_weight = 1.0 - saturate(velocity_4k(motion) / 20.0);
            data.factor *= velocity_weight
                * (1.0 - disocclusion)
                * (1.0 - reactive)
                * (1.0 - shading_change);
        }
    }

    textureStore(luma_history_next, px_pos, data.history);
    textureStore(luma_instability_out, px_pos, vec4<f32>(data.factor, 0.0, 0.0, 0.0));
}
