// fsr3_accumulate.wgsl — FSR 3.1's last pass, and the one that makes
// the image (#481).
//
// Transliterated from `ffx_fsr3upscaler_accumulate.h`,
// `ffx_fsr3upscaler_upsample.h`, `ffx_fsr3upscaler_reproject.h` and
// `ffx_fsr3upscaler_sample.h` (AMD FSR SDK 2.3.0).
//
//   Copyright (C) 2026 Advanced Micro Devices, Inc.
//   SPDX-License-Identifier: MIT
//
// See NOTICE at the repository root. Concatenated after
// `fsr3_common.wgsl`.
//
// # This runs at OUTPUT resolution, and that is the whole trick
//
// Every pass before this one is at render resolution. This one is not:
// each output pixel asks which render samples land near it, weights
// them by a Lanczos-2 kernel measured in RENDER pixels, and adds that
// to a history that already lives at output resolution. The upscaling
// is not a resample of a finished image — the low-resolution samples
// are accumulated straight into the high-resolution grid, with their
// jitter offset as the weight. That is why it needs sixteen phases of
// jitter and why it beats a spatial upscaler.
//
// # Four mechanisms, in the order they run
//
// 1. **Reproject** the history with a separable Lanczos-2 over a 4×4,
//    clamped to its inner 2×2 so the negative lobes cannot ring.
// 2. **Lock** — a pixel flagged as a thin feature last pass gets a
//    lifetime, and while it holds, the rectification below is allowed
//    to be overruled. This is what stops a wire from dissolving.
// 3. **Upsample** the 3×3 of render samples around the output pixel,
//    building the YCoCg variance box as it goes.
// 4. **Rectify** the history against that box — normalise, and if it
//    lands outside the ellipsoid, pull it back to the surface. Not a
//    hard clamp: the pull is lerped back by the lock and the luma
//    instability, which is the part a naive TAA does not have.
//
// # Divergences from the original, all forced and all listed
//
// - **No `FFX_HALF`.** FSR leans on packed 16-bit math. `SHADER_F16`
//   exists in wgpu, but naga's `f16` support is not a drop-in for
//   HLSL's `min16float` semantics, and correctness comes before the
//   ALU saving. 🔴 This is the first optimisation to try if the pass
//   measures badly on the handheld.
// - **No Xbox paired-16-bit path**, which is most of the line count of
//   the original upsample file and none of its behaviour.
// - **HDR input is assumed**, because the engine's colour target is
//   linear `Rgba16Float` and there is no LDR path to select.

@group(0) @binding(1) var input_colour: texture_2d<f32>;
@group(0) @binding(2) var dilated: texture_2d<f32>;
@group(0) @binding(3) var reactive_masks: texture_2d<f32>;
@group(0) @binding(4) var luma_instability: texture_2d<f32>;
@group(0) @binding(5) var farthest_depth_mip1: texture_2d<f32>;
@group(0) @binding(6) var history_prev: texture_2d<f32>;
/// What the tonemap reads. Separate from the history because this
/// one's alpha is COVERAGE for the blit and the history's is the lock —
/// see `targets.rs`.
@group(0) @binding(10) var resolved: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var linear_sampler: sampler;
@group(0) @binding(8) var new_locks: texture_storage_2d<r32float, read_write>;
/// Next frame's history: `rgb` the accumulated colour, `a` the feature
/// lock. Private, and never presented — which is what lets the lock
/// ride along in the alpha for free, sixteen taps at a time.
@group(0) @binding(9) var history_next: texture_storage_2d<rgba16float, write>;

/// A lock below this contributes nothing; above `LOCK_MAX` it saturates.
const LOCK_THRESHOLD: f32 = 1.0;
const LOCK_MAX: f32 = 2.0;

/// How hard velocity is allowed to suppress the accumulated history.
/// FSR exposes this to the application and defaults it to 1.0; there is
/// nothing in this engine that would set it yet, so it is a constant
/// here rather than a uniform field nobody writes.
const VELOCITY_FACTOR: f32 = 1.0;

/// The half twins of the colour helpers.
///
/// 🎯 This is FSR's `FFX_HALF` path, and it is the optimisation the
/// technique is built around: `accumulate` measured 11.982 of the
/// technique's 14.704 ms, and it carries 25 colours through YCoCg, a
/// tonemap round trip and a variance box for every output pixel. In
/// half that is half the registers, which on a 10 W part is occupancy,
/// which is latency hiding — the thing a pass doing thirty texture
/// fetches per pixel is actually short of.
///
/// WGSL has no overloading, so these are twins rather than the same
/// name. ⚠️ Only the SAMPLE LOOPS are half. The rectification and the
/// blend stay f32, because two of their constants (`FSR3_FP32_MIN` and
/// the 1.193e-7 box floor) are below the smallest normal half and would
/// quietly become zero.
fn rgb_to_ycocg_h(rgb: vec3<f16>) -> vec3<f16> {
    return vec3<f16>(
        0.25h * rgb.r + 0.5h * rgb.g + 0.25h * rgb.b,
        0.5h * rgb.r - 0.5h * rgb.b,
        -0.25h * rgb.r + 0.5h * rgb.g - 0.25h * rgb.b,
    );
}

fn ycocg_to_rgb_h(ycocg: vec3<f16>) -> vec3<f16> {
    return vec3<f16>(
        ycocg.x + ycocg.y - ycocg.z,
        ycocg.x + ycocg.z,
        ycocg.x - ycocg.y - ycocg.z,
    );
}

fn fsr3_tonemap_h(rgb: vec3<f16>) -> vec3<f16> {
    return rgb / (max(max(0.0h, rgb.r), max(rgb.g, rgb.b)) + 1.0h);
}

// ------------------------------------------------------- Lanczos

fn lanczos2_no_clamp(x: f32) -> f32 {
    const PI: f32 = 3.141592653589793;
    if (abs(x) < FSR3_EPSILON) {
        return 1.0;
    }
    return (sin(PI * x) / (PI * x)) * (sin(0.5 * PI * x) / (0.5 * PI * x));
}

fn lanczos2(x_in: f32) -> f32 {
    return lanczos2_no_clamp(min(abs(x_in), 2.0));
}

/// The polynomial FSR uses for the upsample kernel instead of the real
/// thing: two multiply-adds against a sine, and the difference does not
/// survive a 16-bit colour target. Takes the SQUARED distance, which is
/// why the call site never takes a square root.
fn lanczos2_approx_sq(x2_in: f32) -> f32 {
    let x2 = min(x2_in, 4.0);
    let a = (2.0 / 5.0) * x2 - 1.0;
    let b = (1.0 / 4.0) * x2 - 1.0;
    return ((25.0 / 16.0) * a * a - (25.0 / 16.0 - 1.0)) * (b * b);
}

/// The weights come from f32 offsets and the colours are half: the
/// kernel's shape is geometry and stays exact, the data it carries does
/// not need to be.
fn lanczos2_row(c0: vec4<f16>, c1: vec4<f16>, c2: vec4<f16>, c3: vec4<f16>, t: f32) -> vec4<f16> {
    let w0 = lanczos2(-1.0 - t);
    let w1 = lanczos2(-0.0 - t);
    let w2 = lanczos2(1.0 - t);
    let w3 = lanczos2(2.0 - t);
    let n = f16(1.0 / (w0 + w1 + w2 + w3));
    return (f16(w0) * c0 + f16(w1) * c1 + f16(w2) * c2 + f16(w3) * c3) * n;
}

/// Separable Lanczos-2 over the 4×4 around `uv`, then clamped to the
/// range of the inner 2×2. The clamp is the deringing: Lanczos has
/// negative lobes, and without it a bright edge grows a dark halo that
/// the history then remembers forever.
fn sample_history(uv: vec2<f32>, size: vec2<f32>) -> vec4<f16> {
    var px_sample = uv * size - vec2<f32>(0.5);
    let frac = fract(px_sample);
    px_sample = clamp(px_sample, vec2<f32>(0.0), size - vec2<f32>(1.0));
    let base = vec2<i32>(floor(px_sample));
    let isize = vec2<i32>(size);

    var taps: array<vec4<f16>, 16>;
    for (var row = 0; row < 4; row++) {
        for (var col = 0; col < 4; col++) {
            let offset = vec2<i32>(col - 1, row - 1);
            // One fetch, and the lock is filtered by the same kernel as
            // the colour — which is the whole reason FSR keeps it here.
            taps[row * 4 + col] =
                vec4<f16>(textureLoad(history_prev, clamp_load(base, offset, isize), 0));
        }
    }

    let row0 = lanczos2_row(taps[0], taps[1], taps[2], taps[3], frac.x);
    let row1 = lanczos2_row(taps[4], taps[5], taps[6], taps[7], frac.x);
    let row2 = lanczos2_row(taps[8], taps[9], taps[10], taps[11], frac.x);
    let row3 = lanczos2_row(taps[12], taps[13], taps[14], taps[15], frac.x);
    var colour = lanczos2_row(row0, row1, row2, row3, frac.y);

    var lo = taps[5];
    var hi = taps[5];
    lo = min(lo, taps[6]);
    hi = max(hi, taps[6]);
    lo = min(lo, taps[9]);
    hi = max(hi, taps[9]);
    lo = min(lo, taps[10]);
    hi = max(hi, taps[10]);

    return clamp(colour, lo, hi);
}

// ------------------------------------------------ rectification box

/// The neighbourhood's mean and standard deviation in YCoCg, plus the
/// hard min/max used for deringing. Accumulated weighted, so a sample
/// further from the output pixel counts for less.
struct RectificationBox {
    centre: vec3<f16>,
    vec: vec3<f16>,
    aabb_min: vec3<f16>,
    aabb_max: vec3<f16>,
    weight: f16,
}

fn box_add(box_in: RectificationBox, initial: bool, colour: vec3<f16>, weight: f16) -> RectificationBox {
    var box = box_in;
    let weighted = colour * weight;
    if (initial) {
        box.aabb_min = colour;
        box.aabb_max = colour;
        box.centre = weighted;
        box.vec = colour * weighted;
        box.weight = weight;
    } else {
        box.aabb_min = min(box.aabb_min, colour);
        box.aabb_max = max(box.aabb_max, colour);
        box.centre += weighted;
        box.vec += colour * weighted;
        box.weight += weight;
    }
    return box;
}

fn box_finish(box_in: RectificationBox) -> RectificationBox {
    var box = box_in;
    // ⚠️ FSR compares against FP32_MIN here. That is below the
    // smallest normal half, so in this path it would be a comparison
    // against zero — the half floor is the honest substitute.
    if (abs(box.weight) <= f16(FSR3_FP16_MIN)) {
        box.weight = 1.0h;
    }
    box.centre /= box.weight;
    box.vec /= box.weight;
    // E[x²] − E[x]², which is the variance, and its root is the sigma
    // the history is measured against.
    box.vec = sqrt(abs(box.vec - box.centre * box.centre));
    return box;
}

// ------------------------------------------------------- pass state

struct Common {
    hr_pos: vec2<i32>,
    hr_uv: vec2<f32>,
    lr_uv_jittered: vec2<f32>,
    lr_uv_hw: vec2<f32>,
    motion: vec2<f32>,
    reprojected_hr_uv: vec2<f32>,
    velocity_4k: f32,
    disocclusion: f32,
    reactive: f32,
    shading_change: f32,
    accumulation: f32,
    luma_instability: f32,
    farthest_metres: f32,
    existing_sample: bool,
    new_sample: bool,
}

struct Data {
    box: RectificationBox,
    upsampled_colour: vec3<f16>,
    upsampled_weight: f16,
    history_colour: vec3<f16>,
    history_weight: f16,
    lock: f32,
    lock_contribution: f32,
    /// The Lanczos sum BEFORE the epsilon gate zeroes it, kept only so
    /// the last debug step can tell "the kernel summed to nothing" apart
    /// from "the gate threw it away".
    raw_weight: f32,
    /// The two numbers that decide every tap's weight: where the render
    /// grid sits relative to this output pixel, and how wide the kernel
    /// is. Measured rather than reasoned about, because the sum came
    /// back zero with both of them apparently in range.
    base_offset: vec2<f32>,
    kernel_bias: f32,
}

fn load_prepared_colour(pos: vec2<i32>) -> vec3<f16> {
    let rgb = max(vec3<f32>(0.0), textureLoad(input_colour, pos, 0).rgb) * params.exposure;
    return rgb_to_ycocg_h(vec3<f16>(rgb));
}

fn init_common(hr_pos: vec2<i32>) -> Common {
    var c: Common;
    c.hr_pos = hr_pos;
    c.hr_uv = (vec2<f32>(hr_pos) + 0.5) * params.output_size_rcp;
    c.lr_uv_jittered = c.hr_uv + params.jitter * params.render_size_rcp;
    c.lr_uv_hw = clamp_uv(c.lr_uv_jittered, params.render_size);

    // Low-resolution motion vectors: read the dilated field at the
    // render pixel this output pixel falls in.
    c.motion = textureLoad(dilated, vec2<i32>(c.hr_uv * params.render_size), 0).xy;
    c.velocity_4k = velocity_4k(c.motion);

    c.reprojected_hr_uv = c.hr_uv + c.motion;
    c.existing_sample = is_uv_inside(c.reprojected_hr_uv);

    c.luma_instability = textureSampleLevel(
        luma_instability,
        linear_sampler,
        clamp_uv(c.hr_uv, params.render_size),
        0.0,
    ).x;

    let half_size = max(params.render_size * 0.5, vec2<f32>(1.0));
    c.farthest_metres = textureSampleLevel(
        farthest_depth_mip1,
        linear_sampler,
        clamp_uv(c.lr_uv_jittered, half_size),
        0.0,
    ).x;

    // A reset frame has no usable history however valid the uv is.
    c.new_sample = !c.existing_sample || params.frame_index == 0u || params.reset > 0.5;

    let masks = textureSampleLevel(reactive_masks, linear_sampler, c.lr_uv_hw, 0.0);
    c.reactive = saturate(masks[REACTIVE]);
    c.disocclusion = saturate(masks[DISOCCLUSION]);
    c.shading_change = saturate(masks[SHADING_CHANGE]);
    c.accumulation = saturate(masks[ACCUMULATION]);
    c.accumulation *= f32(round(c.accumulation * 100.0) > 1.0);
    return c;
}

fn reproject_history(c: Common, d_in: Data) -> Data {
    var d = d_in;
    let history = sample_history(c.reprojected_hr_uv, params.output_size);
    // The exposure pair is a host constant, so it folds to one half
    // multiply rather than two.
    let rescale = f16(params.delta_pre_exposure * params.exposure);
    d.history_colour = rgb_to_ycocg_h(history.rgb * rescale);
    d.lock = f32(history.w);
    return d;
}

fn update_lock_status(c: Common, d_in: Data) -> Data {
    var d = d_in;
    d.lock *= f32(!c.new_sample);

    let decrease_factor = max(saturate(c.shading_change), max(c.reactive, c.disocclusion));
    d.lock = max(0.0, d.lock - decrease_factor * LOCK_MAX);

    d.lock_contribution = saturate(saturate(d.lock - LOCK_THRESHOLD) * (LOCK_MAX - LOCK_THRESHOLD));

    // ⚠️ `shading_change * 0` is FSR's own, not a transcription slip —
    // the term was disabled in place rather than deleted, and it is
    // left visible so the next reader does not "restore" it.
    let intensity = textureLoad(new_locks, c.hr_pos).x * (1.0 - max(c.shading_change * 0.0, c.reactive));
    d.lock = max(0.0, min(d.lock + intensity, LOCK_MAX));

    // A lock is meant to survive one pass of the jitter sequence, so it
    // decays over exactly that many frames.
    let lifetime_decrease = (0.1 / params.jitter_sequence_length) * (1.0 - decrease_factor);
    d.lock = max(0.0, d.lock - lifetime_decrease);

    // Kill a lock that is about to leave the screen, or it gets pinned
    // to the border and smears along it.
    let next_frame_uv = c.hr_uv - c.motion;
    d.lock *= f32(is_uv_inside(next_frame_uv));
    return d;
}

fn base_accumulation_weight(c: Common, d_in: Data) -> Data {
    var d = d_in;
    var base = c.accumulation;
    // Fast motion means the history is mostly wrong, so cap the trust
    // at 0.15 of a frame no matter how much was earned standing still.
    base = min(
        base,
        mix(base, 0.15, saturate(max(0.0, (c.velocity_4k * VELOCITY_FACTOR) / 0.5))),
    );
    d.history_weight = f16(base);
    return d;
}

/// The kernel narrows as the upscale ratio grows: at 1:1 it may reach
/// two render pixels, and at a large ratio a wide kernel would blur
/// what the accumulation is trying to recover.
fn max_kernel_weight() -> f32 {
    let bias = 1.0 + (1.0 / params.downscale.x - 1.0);
    return min(1.99, bias);
}

fn upsample(c: Common, d_in: Data) -> Data {
    var d = d_in;

    let dst_pos = vec2<f32>(c.hr_pos) + 0.5;
    let src_pos = dst_pos * params.downscale;
    let src_input_pos = vec2<i32>(floor(src_pos));
    // The un-jittered position of the sample at offset (0,0).
    let src_unjittered = (vec2<f32>(src_input_pos) + 0.5) - params.jitter;
    let base_offset = src_unjittered - src_pos;

    // Which side of the output pixel the render grid falls on decides
    // which 3 of the 4 candidate columns are closest. Flipping the
    // iteration order instead of the offsets keeps the first sample of
    // the loop the nearest one, which is what the rectification box
    // wants for its initial sample.
    let flip_col = src_unjittered.x > src_pos.x;
    let flip_row = src_unjittered.y > src_pos.y;
    var offset_tl: vec2<i32>;
    offset_tl.x = select(-1, -2, flip_col);
    offset_tl.y = select(-1, -2, flip_row);
    let f_offset_tl = vec2<f32>(offset_tl);

    let initial_frame = c.accumulation == 0.0;
    let size = render_size_i();

    var samples: array<vec3<f16>, 9>;
    var index = 0;
    for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 3; col++) {
            let col_row = vec2<i32>(
                select(col, 3 - col, flip_col),
                select(row, 3 - row, flip_row),
            );
            let src_sample_pos = src_input_pos + offset_tl + col_row;
            samples[index] = load_prepared_colour(clamp_load(src_sample_pos, vec2<i32>(0), size));
            index++;
        }
    }

    // On the very first frame there is no history to protect, so the
    // samples are resolved in the compressed range instead — a single
    // fireball pixel would otherwise set the whole box.
    if (initial_frame) {
        for (var i = 0; i < 9; i++) {
            samples[i] = rgb_to_ycocg_h(fsr3_tonemap_h(ycocg_to_rgb_h(samples[i])));
        }
    }

    let kernel_max = max_kernel_weight();
    let kernel_min = max(1.0, (1.0 + kernel_max) * 0.3);
    let kernel_weight = min(
        1.0 - c.disocclusion * 0.5,
        min(1.0 - c.shading_change, saturate(f32(d.history_weight) * 5.0)),
    );
    let kernel_bias = mix(kernel_min, kernel_max, kernel_weight);
    d.base_offset = base_offset;
    d.kernel_bias = kernel_bias;

    index = 0;
    for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 3; col++) {
            let col_row = vec2<i32>(
                select(col, 3 - col, flip_col),
                select(row, 3 - row, flip_row),
            );
            let offset = f_offset_tl + vec2<f32>(col_row);
            let src_sample_offset = base_offset + offset;

            let src_sample_pos = src_input_pos + offset_tl + col_row;
            let on_screen = f32(is_on_screen(src_sample_pos, size));

            if (!initial_frame) {
                let biased = src_sample_offset * kernel_bias;
                let weight = f16(on_screen * lanczos2_approx_sq(dot(biased, biased)));
                d.upsampled_colour += samples[index] * weight;
                d.upsampled_weight += weight;
            }

            // The box uses a gaussian rather than the Lanczos kernel:
            // it is measuring the neighbourhood's spread, not resampling
            // it, so it must stay positive.
            const RECTIFICATION_CURVE_BIAS: f32 = -2.3;
            let offset_sq = dot(src_sample_offset, src_sample_offset);
            let box_weight = f16(exp(RECTIFICATION_CURVE_BIAS * offset_sq) * on_screen);
            d.box = box_add(d.box, row == 0 && col == 0, samples[index], box_weight);
            index++;
        }
    }

    d.box = box_finish(d.box);

    d.raw_weight = f32(d.upsampled_weight);
    let live = f16(d.upsampled_weight > f16(FSR3_EPSILON));
    d.upsampled_weight *= live;
    if (d.upsampled_weight > f16(FSR3_EPSILON)) {
        d.upsampled_colour = d.upsampled_colour / d.upsampled_weight;
        d.upsampled_weight *= f16(AVERAGE_LANCZOS_WEIGHT_PER_FRAME);
        // Deringing, again: the Lanczos result cannot leave the range
        // of the samples that produced it.
        d.upsampled_colour = clamp(d.upsampled_colour, d.box.aabb_min, d.box.aabb_max);
    }

    if (initial_frame) {
        // The inverse tonemap divides by `1 - max`, which runs away as
        // max approaches 1 — that one stays f32.
        let centre = ycocg_to_rgb(vec3<f32>(d.box.centre));
        d.upsampled_colour = rgb_to_ycocg_h(vec3<f16>(fsr3_inverse_tonemap(centre)));
        d.upsampled_weight = 1.0h;
        d.history_weight = 0.0h;
    }
    return d;
}

fn rectify_history(c: Common, d_in: Data) -> Data {
    var d = d_in;

    let velocity_factor = saturate(c.velocity_4k / 20.0);
    let distance_factor = saturate(0.75 - c.farthest_metres / 20.0);
    let accumulation_factor = 1.0 - c.accumulation;
    let reactive_factor = sqrt(c.reactive);
    let scale_t = max(
        velocity_factor,
        max(distance_factor, max(accumulation_factor, max(reactive_factor, c.shading_change))),
    );

    // A settled, slow, distant pixel gets a box three sigma wide; a
    // moving or freshly disoccluded one gets one sigma and is rectified
    // hard.
    let box_scale = mix(3.0, 1.0, scale_t);
    // Luma is stretched because the eye forgives a chroma error and
    // does not forgive a luma one.
    let scaled = vec3<f32>(d.box.vec) * vec3<f32>(1.7, 1.0, 1.0) * box_scale;
    let clamped_scaled = max(scaled, vec3<f32>(1.193e-7));
    let centre = vec3<f32>(d.box.centre);
    let transformed = (vec3<f32>(d.history_colour) - centre) / clamped_scaled;

    if (length(transformed) > 1.0) {
        let clamped = normalize(transformed);
        let final_colour = clamped * scaled + centre;

        // 🎯 The line that separates this from a neighbourhood clamp:
        // a locked or oscillating pixel is allowed to KEEP its history
        // even though it fell outside the box, because the box is what
        // is wrong in those two cases.
        let contribution =
            max(c.luma_instability, d.lock_contribution) * c.accumulation * (1.0 - c.disocclusion);
        d.history_colour = vec3<f16>(mix(
            final_colour,
            vec3<f32>(d.history_colour),
            saturate(contribution),
        ));
    }
    return d;
}

fn accumulate_colour(d_in: Data) -> Data {
    var d = d_in;
    d.history_weight *= f16(d.history_weight > f16(FSR3_FP16_MIN));
    d.history_weight = max(f16(FSR3_EPSILON), d.history_weight + d.upsampled_weight);

    // Blend in the compressed range so that a bright new sample cannot
    // dominate by magnitude, then invert.
    d.upsampled_colour = rgb_to_ycocg_h(fsr3_tonemap_h(ycocg_to_rgb_h(d.upsampled_colour)));
    d.history_colour = rgb_to_ycocg_h(fsr3_tonemap_h(ycocg_to_rgb_h(d.history_colour)));

    let alpha = saturate(d.upsampled_weight / d.history_weight);
    d.history_colour = mix(d.history_colour, d.upsampled_colour, vec3<f16>(alpha));
    // ⚠️ The inverse tonemap divides by `1 - max`. Near white that is a
    // small number and half has four decimal digits, so this one leaves
    // the half path deliberately.
    let linear = fsr3_inverse_tonemap(ycocg_to_rgb(vec3<f32>(d.history_colour)));
    d.history_colour = vec3<f16>(linear);
    return d;
}

@compute @workgroup_size(8, 8, 1)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    let hr_pos = vec2<i32>(id.xy);
    if (!is_on_screen(hr_pos, output_size_i())) {
        return;
    }

    let c = init_common(hr_pos);

    var d: Data;
    d.upsampled_colour = vec3<f16>(0.0h);
    d.history_colour = vec3<f16>(0.0h);
    d.history_weight = 1.0h;
    d.upsampled_weight = 0.0h;
    d.lock = 0.0;
    d.lock_contribution = 0.0;
    d.raw_weight = 0.0;
    d.base_offset = vec2<f32>(0.0);
    d.kernel_bias = 0.0;

    if (c.existing_sample && !c.new_sample) {
        d = reproject_history(c, d);
    }
    // Snapshotted here because `accumulate_colour` turns `history_colour`
    // from YCoCg into the final RGB, and the debug step that wants to
    // know whether there IS a history has to ask before that.
    let reprojected = vec3<f32>(d.history_colour);

    d = update_lock_status(c, d);
    d = base_accumulation_weight(c, d);
    d = upsample(c, d);
    d = rectify_history(c, d);
    d = accumulate_colour(d);

    // Back to f32 for the store: the target is `rgba16float`, so the
    // narrowing happens in hardware either way, but the exposure divide
    // is by a number around 1e-3 and half would lose it.
    var out = max(vec3<f32>(d.history_colour) / params.exposure, vec3<f32>(0.0));
    if (params.debug != 0u) {
        out = debug_stage(c, d, reprojected);
    }

    textureStore(history_next, hr_pos, vec4<f32>(out, d.lock));
    // Alpha 1: the blit composites on it, and this pixel WAS drawn.
    textureStore(resolved, hr_pos, vec4<f32>(out, 1.0));
    textureStore(new_locks, hr_pos, vec4<f32>(0.0));
}

/// The staircase, for finding which stage produced a wrong frame.
///
/// Selected by the editor's debug dropdown, and the order is the order
/// the data flows: the first mode that looks wrong is the first stage
/// that IS wrong, and everything after it is downstream of the same
/// fault.
///
/// 🎯 Two kinds of step, and they leave through different doors.
///
/// The steps that show a 0..1 QUANTITY bypass the tonemap, so what is
/// returned lands on screen as a grey ramp of the same number. The three
/// that show RADIANCE keep the ordinary tonemap — exposure and filmic
/// curve — so they are directly comparable with the real image. Handing
/// radiance to a bypassed tonemap was the first attempt, and it painted
/// a perfectly good frame black.
fn debug_stage(c: Common, d: Data, reprojected: vec3<f32>) -> vec3<f32> {
    switch params.debug {
        // 1 — the HDR frame FSR was handed. Black here means the fault
        // is upstream of this technique entirely.
        case 1u: {
            let rgb = textureLoad(input_colour, vec2<i32>(c.hr_uv * params.render_size), 0).rgb;
            return max(rgb, vec3<f32>(0.0));
        }
        // 2 — dilated motion, biased so zero is grey. ⚠️ Uniform grey
        // with a still camera is CORRECT.
        case 2u: {
            return vec3<f32>(c.motion * 50.0 + 0.5, 0.5);
        }
        // 3 — red reactive, green disocclusion, blue frames of history
        // earned. Blue climbs to full over three still frames; staying
        // black is the accumulation counter never advancing.
        case 3u: {
            return vec3<f32>(c.reactive, c.disocclusion, c.accumulation);
        }
        // 4 — THIS frame's upsample alone, no history. If the scene is
        // here, the inputs and the kernel are fine.
        case 4u: {
            // The internal buffers hold radiance already multiplied by
            // the exposure; the tonemap is about to multiply again.
            return max(ycocg_to_rgb(vec3<f32>(d.upsampled_colour)), vec3<f32>(0.0))
                / params.exposure;
        }
        // 5 — the reprojected history alone, before anything this frame
        // is blended into it. Black here with a settled camera means the
        // history never survives from one frame to the next.
        case 5u: {
            return max(ycocg_to_rgb(vec3<f32>(reprojected)), vec3<f32>(0.0)) / params.exposure;
        }
        // 6 — red the lock, green the luma instability, blue the
        // upsample's total weight.
        case 6u: {
            return vec3<f32>(
                d.lock / LOCK_MAX,
                c.luma_instability,
                f32(d.upsampled_weight) / AVERAGE_LANCZOS_WEIGHT_PER_FRAME,
            );
        }
        // 7 — why is this pixel black. Red is the first-frame branch,
        // green the RAW Lanczos sum before the epsilon gate, blue the
        // history weight. A black frame with no green anywhere means the
        // kernel summed to nothing and the accumulation kept a history
        // that was never written.
        case 7u: {
            // 🔴 The two INPUTS to the weight, now that the output is
            // known to be zero. Red and green are how far the render
            // grid sits from this output pixel, in render pixels — both
            // must stay under 1 or no tap of the 3x3 can land in the
            // kernel's positive lobe. Blue is the kernel width, halved
            // so that FSR's 1.99 ceiling reads as full.
            return vec3<f32>(abs(d.base_offset), d.kernel_bias * 0.5);
        }
        default: {
            return vec3<f32>(d.history_colour);
        }
    }
}
