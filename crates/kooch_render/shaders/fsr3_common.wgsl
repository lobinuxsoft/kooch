// fsr3_common.wgsl — the shared half of FSR 3.1 (#481, the transliteration).
//
// Transliterated from AMD FidelityFX Super Resolution 3.1,
// `Kits/FidelityFX/upscalers/fsr3/include/gpu/fsr3upscaler/
//  ffx_fsr3upscaler_common.h` (AMD FSR SDK 2.3.0).
//
//   Copyright (C) 2026 Advanced Micro Devices, Inc.
//   SPDX-License-Identifier: MIT
//
// See NOTICE at the repository root.
//
// # This file is not a pass
//
// WGSL has no `#include`, so every FSR pass is built as
// `concat!(COMMON, THE_PASS)` in `fsr3.rs`. That is why the uniform
// block lives here and why binding 0 of group 0 means the same thing
// in all four of them.
//
// # The four decisions that make this a port and not a rewrite
//
// FSR's shaders are written against a set of compile-time options and
// host-supplied factors. Ours are fixed, so the branches collapse:
//
// 1. **`INVERTED_DEPTH` = on.** The engine's projection is reversed-Z
//    with an infinite far plane, so nearer is GREATER and the far
//    plane is exactly 0.
// 2. **`LOW_RESOLUTION_MOTION_VECTORS` = on.** The motion buffer is
//    written by the raster pass at render resolution, so FSR's
//    low-res-to-high-res position mapping is skipped on load.
// 3. **The motion vectors are NEGATED on load.** FSR reprojects with
//    `uv + mv`; this engine's buffer is signed so that history lives
//    at `uv - mv` (asserted in `motion_vectors.rs`). One sign, in one
//    place, and it is the single likeliest way for this port to be
//    subtly wrong — a flipped sign does not crash, it just smears.
// 4. **`DeviceToViewSpaceTransformFactors` collapses to the near
//    plane.** An infinite reversed-Z projection maps `d = near / z`
//    exactly, so view depth is `near / d` — no matrix, no far plane,
//    and the same identity the disocclusion test in `taa.wgsl` uses.
//    One engine unit is one metre, so FSR's `ViewSpaceToMetersFactor`
//    is 1 and is not carried.

const FSR3_FP16_MIN: f32 = 6.10e-05;
const FSR3_FP16_MAX: f32 = 65504.0;
const FSR3_EPSILON: f32 = 6.10e-05;
const FSR3_TONEMAP_EPSILON: f32 = 6.10e-05;
const FSR3_FP32_MIN: f32 = 1.175494351e-38;

/// A bilinear weight below this contributes nothing worth an atomic.
const RECONSTRUCTED_DEPTH_WEIGHT_THRESHOLD: f32 = FSR3_EPSILON * 10.0;

/// Lanczos weights are accumulated in units of this, so that a frame of
/// new samples and a frame of history are comparable quantities. The
/// 0.74 is the average weight a jittered sample actually earns — AMD's
/// measurement, and exactly the kind of constant that is not guessable.
const UPSAMPLE_LANCZOS_WEIGHT_SCALE: f32 = 1.0 / 16.0;
const AVERAGE_LANCZOS_WEIGHT_PER_FRAME: f32 = 0.74 * UPSAMPLE_LANCZOS_WEIGHT_SCALE;

/// Channels of the reactive-mask target, which is one texture because
/// all four are produced by one pass and consumed by the next.
const REACTIVE: i32 = 0;
const DISOCCLUSION: i32 = 1;
const SHADING_CHANGE: i32 = 2;
const ACCUMULATION: i32 = 3;

struct Fsr3Params {
    /// Render (input) resolution, in pixels.
    render_size: vec2<f32>,
    /// Display (output) resolution, in pixels.
    output_size: vec2<f32>,
    render_size_rcp: vec2<f32>,
    output_size_rcp: vec2<f32>,
    /// This frame's sub-pixel offset, in RENDER pixels.
    jitter: vec2<f32>,
    /// The previous frame's, for the reprojection of anything that was
    /// written at the previous jitter rather than resolved out of it.
    prev_jitter: vec2<f32>,
    /// `render_size / output_size`, per axis.
    downscale: vec2<f32>,
    /// The near plane, which is the whole of the depth transform under
    /// an infinite reversed-Z projection.
    near: f32,
    exposure: f32,
    /// 1.0 on the first frame after a camera cut or a resize.
    reset: f32,
    frame_index: u32,
    /// Previous frame's exposure over this one's. 1.0 while the engine
    /// hands FSR a fixed exposure; the term exists because a history
    /// resolved at a different exposure has to be rescaled before it is
    /// blended, and finding that out later means re-deriving it.
    delta_pre_exposure: f32,
    /// How many jitter phases the sequence has. A lock decays over the
    /// whole sequence, so a longer one holds thin features longer.
    jitter_sequence_length: f32,
    _pad: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Fsr3Params;

fn render_size_i() -> vec2<i32> {
    return vec2<i32>(params.render_size);
}

fn output_size_i() -> vec2<i32> {
    return vec2<i32>(params.output_size);
}

// ---------------------------------------------------------------- math

fn fsr3_luma(linear_rgb: vec3<f32>) -> f32 {
    return dot(linear_rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn rgb_to_ycocg(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * rgb.r + 0.5 * rgb.g + 0.25 * rgb.b,
        0.5 * rgb.r - 0.5 * rgb.b,
        -0.25 * rgb.r + 0.5 * rgb.g - 0.25 * rgb.b,
    );
}

fn ycocg_to_rgb(ycocg: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        ycocg.x + ycocg.y - ycocg.z,
        ycocg.x + ycocg.z,
        ycocg.x - ycocg.y - ycocg.z,
    );
}

/// Reinhard on the maximum channel. Not the engine's tonemap and not
/// meant to be: FSR resolves in a compressed range so that one fireball
/// pixel cannot drag a whole neighbourhood, then inverts it. The pair
/// has to round-trip, which is the only property that matters here.
fn fsr3_tonemap(rgb: vec3<f32>) -> vec3<f32> {
    return rgb / (max(max(0.0, rgb.r), max(rgb.g, rgb.b)) + 1.0);
}

fn fsr3_inverse_tonemap(rgb: vec3<f32>) -> vec3<f32> {
    return rgb / max(FSR3_TONEMAP_EPSILON, 1.0 - max(rgb.r, max(rgb.g, rgb.b)));
}

fn min_over_max(v0: f32, v1: f32, on_zero: f32) -> f32 {
    let m = max(v0, v1);
    if (m != 0.0) {
        return min(v0, v1) / m;
    }
    return on_zero;
}

/// A motion vector's length expressed in 4K pixels, which is how every
/// threshold in FSR is written — so that the tuning does not move when
/// the render resolution does.
fn velocity_4k(motion: vec2<f32>) -> f32 {
    return length(motion * vec2<f32>(3840.0, 2160.0));
}

fn is_on_screen(pos: vec2<i32>, size: vec2<i32>) -> bool {
    return all(vec2<u32>(pos) < vec2<u32>(size));
}

fn is_uv_inside(uv: vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

fn clamp_load(sample_pos: vec2<i32>, offset: vec2<i32>, size: vec2<i32>) -> vec2<i32> {
    return clamp(sample_pos + offset, vec2<i32>(0), size - vec2<i32>(1));
}

/// Pulls a uv half a texel inside the image.
///
/// FSR's own version takes a used size and an allocated size, because
/// its host may hand it a target larger than the region being rendered
/// (dynamic resolution). This engine reallocates every attachment on
/// resize — `Vbuf64Stage::resize` — so the two are always equal and the
/// second argument is dropped. 🔴 The day dynamic resolution lands,
/// this is one of the places that has to grow it back.
fn clamp_uv(uv: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    return clamp(uv * size, vec2<f32>(0.5), size - vec2<f32>(0.5)) / size;
}

// --------------------------------------------------------------- depth

/// View-space depth in metres. Zero (the infinite far plane) would
/// divide by zero, so it saturates at FP16's ceiling exactly as FSR's
/// call sites do after the transform.
fn view_depth(device_depth: f32) -> f32 {
    if (device_depth <= 0.0) {
        return FSR3_FP16_MAX;
    }
    return min(params.near / device_depth, FSR3_FP16_MAX);
}

/// Below this, a motion vector is discarded when reprojecting depth:
/// it is smaller than the error in the reprojection itself. Grows with
/// distance because a distant surface moves fewer pixels for the same
/// world motion.
fn reconstructed_depth_mv_threshold(nearest_in_metres: f32) -> f32 {
    return mix(0.25, 0.75, saturate(nearest_in_metres / 100.0));
}

/// The output pixel a render pixel lands on once its jitter is undone.
fn hr_pos_from_lr_pos(lr_pos: vec2<i32>) -> vec2<i32> {
    let jittered = vec2<f32>(lr_pos) + 0.5 - params.jitter;
    return vec2<i32>(floor(jittered * params.render_size_rcp * params.output_size));
}

// ------------------------------------------------------------ bilinear

/// The four taps and weights a bilinear sample would have used. FSR
/// needs them explicitly because it SCATTERS into them (the depth
/// reconstruction) and weights them by more than distance (the history
/// resample), neither of which a hardware sampler can do.
struct BilinearTaps {
    base_pos: vec2<i32>,
    weights: vec4<f32>,
}

fn bilinear_taps(uv: vec2<f32>, size: vec2<f32>) -> BilinearTaps {
    let sample_pos = uv * size - vec2<f32>(0.5);
    var data: BilinearTaps;
    data.base_pos = vec2<i32>(floor(sample_pos));
    let frac = fract(sample_pos);
    data.weights = vec4<f32>(
        (1.0 - frac.x) * (1.0 - frac.y),
        frac.x * (1.0 - frac.y),
        (1.0 - frac.x) * frac.y,
        frac.x * frac.y,
    );
    return data;
}

fn bilinear_offset(index: i32) -> vec2<i32> {
    return vec2<i32>(index & 1, index >> 1);
}
