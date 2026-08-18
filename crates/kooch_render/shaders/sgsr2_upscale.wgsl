// sgsr2_upscale.wgsl — pass 2 of the temporal upscaler (#481, step 4).
//
// Transliterated from Snapdragon Game Super Resolution 2,
// `sgsr/v2/include/glsl_2_pass_fs/sgsr2_upscale.fs`.
//
//   Copyright (c) 2024, Qualcomm Innovation Center, Inc. All rights reserved.
//   SPDX-License-Identifier: BSD-3-Clause
//
// See NOTICE at the repository root. This is not a Qualcomm product and
// Qualcomm has not endorsed it.
//
// # What actually makes this an upscaler
//
// The reprojection and the neighbourhood box are ordinary TAA. The part
// that reconstructs detail is the weight: each low-resolution sample is
// accumulated into the OUTPUT grid with a Lanczos weight taken from how
// far its jittered position landed from the output pixel being written.
// A pixel the jitter happened to land on gets a confident sample; one it
// missed gets a weak one and leans on its history instead. That weight
// sum is `upsampled.w`, and it doubles as the blend factor further down
// — which is why the two are the same number and not two heuristics.
//
// At a ratio of 1:1 the offsets are small, every weight is high, and
// this degenerates into a temporal resolve. That is the property the
// port is validated with.
//
// # 🔴 The departure that is NOT optional: exposure
//
// Upstream is a mobile renderer and its colours are display-referred.
// It adds a bare `0.075` to the neighbourhood box, mixes in linear
// space, and none of that means anything against radiance in the
// HUNDREDS, which is what this engine produces (#254): 0.075 on top of
// 400 is not a tolerance, it is a rounding error, and the box would
// clamp nothing.
//
// So the same correction the resolve already needed applies here, for
// the same reason and with the same operator: multiply by exposure,
// compress, do the arithmetic there, expand on write. This was already
// paid for once — feeding the range compressor raw radiance posterised
// the whole image and read as a broken toon shader. Anything in this
// engine that compresses range has to see the exposure FIRST.

struct UpscaleUniforms {
    render_size: vec2<f32>,
    output_size: vec2<f32>,
    render_size_rcp: vec2<f32>,
    output_size_rcp: vec2<f32>,
    // This frame's sub-pixel offset, in RENDER pixels. Same value the
    // projection was jittered by.
    jitter_offset: vec2<f32>,
    // `.x` the linear upscale ratio, `.y` the cube of the area ratio
    // capped at 20 — both upstream's. See `sgsr2.rs`.
    scale_ratio: vec2<f32>,
    min_lerp: f32,
    // 1 on the first frame after allocation or resize; blends nothing.
    reset: f32,
    // 🔴 The same multiplier the tonemap pass applies. See the header.
    exposure: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> params: UpscaleUniforms;
@group(0) @binding(1) var prev_output: texture_2d<f32>;
@group(0) @binding(2) var motion_depth_clip: texture_2d<f32>;
@group(0) @binding(3) var input_color: texture_2d<f32>;
@group(0) @binding(4) var linear_sampler: sampler;
@group(0) @binding(5) var point_sampler: sampler;

// Upstream leaves its four diagonal taps behind `if (false)`, with the
// note "maybe disable this for ultra performance, true could generate
// more realistic output". Their default is the five-tap cross and this
// keeps it: the extra four are four more full-rate loads per output
// pixel on a device that measures as bandwidth-bound.
const DIAGONAL_TAPS: bool = false;

fn rcp(x: f32) -> f32 { return 1.0 / x; }
fn max3(x: vec3<f32>) -> f32 { return max(x.r, max(x.g, x.b)); }

// The reversible range compressor, exposed first. Identical to the one
// in `taa.wgsl` on purpose: two techniques that disagree about what
// "bright" means cannot be compared against each other, and comparing
// them is the entire point of having both.
fn compress(color: vec3<f32>) -> vec3<f32> {
    let c = color * params.exposure;
    return c * rcp(max3(c) + 1.0);
}

fn expand(color: vec3<f32>) -> vec3<f32> {
    let c = color * rcp(max(1.0 - max3(color), 1.0 / 65504.0));
    return c / max(params.exposure, 1.0 / 65504.0);
}

/// A render-resolution texel, compressed, with the coordinate clamped.
///
/// 🔴 The clamp is not defensive. GLSL's `texelFetch` out of range is
/// undefined and in practice clamps; WGSL's `textureLoad` is DEFINED to
/// return ZERO. Transliterating it literally would ring a one-pixel
/// black border into the accumulation at every screen edge — a dark
/// frame that looks like a vignette and is a missing clamp.
fn load_color(pos: vec2<i32>) -> vec3<f32> {
    let limit = vec2<i32>(params.render_size) - vec2<i32>(1);
    let clamped = clamp(pos, vec2<i32>(0), limit);
    return compress(textureLoad(input_color, clamped, 0).rgb);
}

fn fast_lanczos(base: f32) -> f32 {
    let y = base - 1.0;
    let y2 = y * y;
    return (0.75 * y + y2) * y2;
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_upscale(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

@fragment
fn fs_upscale(in: Varyings) -> @location(0) vec4<f32> {
    let bias_max_scale = params.scale_ratio.x;
    let box_scale_factor = params.scale_ratio.y;

    let out_uv = in.uv;
    // Where this output pixel's sample was actually taken, once the
    // jitter is accounted for.
    let jittered_uv = clamp(
        out_uv + params.jitter_offset * params.render_size_rcp,
        vec2<f32>(0.0),
        vec2<f32>(1.0),
    );
    let input_pos = vec2<i32>(jittered_uv * params.render_size);

    let mda = textureSampleLevel(motion_depth_clip, point_sampler, jittered_uv, 0.0).xyz;
    let motion = mda.xy;
    let depth_factor = mda.z;

    // 🔴 Ours is UV and upstream's is NDC, so their `-0.5 *` and their
    // Y flip are both gone. Keeping the 0.5 would reproject at half
    // speed, which reads as a smear rather than as a wrong constant.
    let prev_uv = clamp(out_uv - motion, vec2<f32>(0.0), vec2<f32>(1.0));
    var history = compress(textureSampleLevel(prev_output, linear_sampler, prev_uv, 0.0).rgb);

    // The kernel narrows where the depth-clip pass says this pixel sits
    // on an edge, and where the frame is moving fast.
    let bias_min = max(1.0, 0.3 + 0.3 * bias_max_scale);
    let kernel_bias = mix(bias_max_scale, bias_min, 0.25 * depth_factor) * 0.5;
    let kernel_bias2 = kernel_bias * kernel_bias;
    let motion_len = length(motion * params.output_size);
    let curve_bias = mix(-2.0, -3.0, clamp(motion_len * 0.02, 0.0, 1.0));

    // Where the input sample sits relative to the output pixel, in
    // render pixels. This is the quantity the whole reconstruction turns
    // on: it is the jitter, made geometric.
    let src_pos = vec2<f32>(input_pos) + vec2<f32>(0.5) - params.jitter_offset;
    let src_delta = src_pos - out_uv * params.render_size;

    var upsampled = vec4<f32>(0.0);
    var box_centre = vec3<f32>(0.0);
    var box_var = vec3<f32>(0.0);
    var box_weight = 0.0;
    var box_min = vec3<f32>(0.0);
    var box_max = vec3<f32>(0.0);

    // Upstream unrolls this five times by hand. A loop over the same
    // offsets in the same order is the same arithmetic; the taps are
    // listed rather than computed so the diagonal set can be appended
    // without renumbering anything.
    var offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 1), vec2<i32>(1, 0), vec2<i32>(-1, 0),
        vec2<i32>(0, 0), vec2<i32>(0, -1),
        vec2<i32>(1, 1), vec2<i32>(-1, 1), vec2<i32>(1, -1), vec2<i32>(-1, -1),
    );
    var count = 5;
    if (DIAGONAL_TAPS) { count = 9; }

    for (var i = 0; i < count; i++) {
        let offset = offsets[i];
        let sample = load_color(input_pos + offset);
        let base_offset = src_delta + vec2<f32>(offset);
        let dot_offset = dot(base_offset, base_offset);

        let weight = fast_lanczos(clamp(dot_offset * kernel_bias2, 0.0, 1.0));
        upsampled += vec4<f32>(sample * weight, weight);

        // A second, gaussian weighting builds the neighbourhood's mean
        // and variance — near taps describe this pixel better than far
        // ones, which a flat box ignores.
        let gaussian = exp(dot_offset * curve_bias);
        box_centre += sample * gaussian;
        box_var += sample * sample * gaussian;
        box_weight += gaussian;

        if (i == 0) {
            box_min = sample;
            box_max = sample;
        } else {
            box_min = min(box_min, sample);
            box_max = max(box_max, sample);
        }
    }

    let inv_box_weight = 1.0 / box_weight;
    box_centre *= inv_box_weight;
    box_var *= inv_box_weight;
    let deviation = sqrt(abs(box_var - box_centre * box_centre));

    // 🔴 The 0.075 is upstream's and it is in COMPRESSED space, which is
    // the only place it means anything. Applied to raw radiance it would
    // widen a box around the value 400 by two ten-thousandths.
    upsampled = vec4<f32>(
        clamp(upsampled.xyz / upsampled.w, box_min - vec3<f32>(0.075), box_max + vec3<f32>(0.075)),
        upsampled.w * (1.0 / 3.0),
    );

    // How much this frame is worth: the weight it accumulated, cut back
    // where the pixel is on an edge or moving.
    var base_alpha = 1.0 - depth_factor;
    base_alpha = min(base_alpha, mix(base_alpha, upsampled.w * 10.0, clamp(10.0 * motion_len, 0.0, 1.0)));
    base_alpha = min(base_alpha, mix(base_alpha, upsampled.w, clamp(motion_len * 0.05, 0.0, 1.0)));

    // The box widens with the upscale ratio, with motion, and on edges.
    // A box built from fewer input samples is a worse estimate of the
    // neighbourhood, and clamping the history hard to a bad estimate is
    // what makes an upscaler flicker.
    let widen = max(depth_factor, clamp(motion_len * 0.05, 0.0, 1.0));
    let scaled = deviation * mix(box_scale_factor, 1.0, widen);
    let clamp_max = min(box_max, box_centre + scaled);
    let clamp_min = max(box_min, box_centre - scaled);

    let clamped = clamp(history, clamp_min, clamp_max);
    // A still pixel keeps some of its out-of-box history; a moving one
    // keeps none. Upstream's asymmetry, and it is the anti-ghosting
    // rule: a pixel that is not moving cannot be ghosting.
    var lerp_start = params.min_lerp;
    if ((abs(motion.x) + abs(motion.y)) > 0.000001) {
        lerp_start = 0.0;
    }
    var contribution = 1.0;
    if (any(clamp_min > history) || any(history > clamp_max)) {
        contribution = lerp_start;
    }
    contribution = clamp(contribution, 0.0, 1.0);

    history = mix(clamped, history, contribution);
    base_alpha = mix(min(base_alpha, 0.1), base_alpha, contribution);

    const EPSILON: f32 = 1.192e-07;
    let alpha = clamp(upsampled.w / max(EPSILON, base_alpha + upsampled.w) + params.reset, 0.0, 1.0);

    // Back to linear radiance: the tonemap downstream expects it, and so
    // does the next frame reading this as its history.
    return vec4<f32>(expand(mix(history, upsampled.xyz, alpha)), 1.0);
}
