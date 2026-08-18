// rcas.wgsl — Robust Contrast Adaptive Sharpening (#481, step 5).
//
// Copyright (c) 2022 Advanced Micro Devices, Inc. All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//
// Transliterated from Bevy's `robust_contrast_adaptive_sharpening.wesl`,
// which is itself FidelityFX FSR 1's `ffx_fsr1.h` RCAS. Ported rather
// than invented for the reason the whole of #481 is: the constant that
// decides where sharpening stops looking natural took someone a year to
// find, and it is 0.1875.
//
// # Why this pass exists at all
//
// A reconstructed image is soft by construction. The resolve builds each
// output pixel out of samples that landed near it rather than on it, and
// a weighted average of neighbours is a low-pass filter no matter how
// good the weights are. Every shipping upscaler ends in a sharpening
// pass; leaving it out is how an upscaler earns the verdict "we tried
// it, it looked worse, we turned it off".
//
// # 🔴 It runs AFTER the tonemap, and that is not an implementation detail
//
// RCAS solves for a filter weight by asking where the signal would clip
// out of the {0, 1} range — that is the whole of its adaptivity, and it
// is what the limiter constants below are calibrated against. Handed
// linear radiance in the hundreds, which is what this engine's shading
// produces, `mn4 / (4 * mx4)` is a ratio of two large numbers and the
// limiter stops limiting anything.
//
// So this reads what the tonemap wrote: display-referred, sRGB-encoded,
// inside {0, 1} by construction. Same reasoning as the exposure
// correction in `sgsr2_upscale.wgsl`, arrived at from the other side —
// there the fix was to bring the arithmetic to the data, here it is to
// put the pass where the data already is.

struct SharpenUniforms {
    // 0 is a pass-through and the pass is skipped entirely upstream, so
    // this is only ever the author's amount. See `sharpen.rs`.
    sharpness: f32,
    // Three scalars rather than a `vec3`, which would align to 16 and
    // make the block 32 bytes against the 16 the host writes.
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: SharpenUniforms;

// The limit at which sharpening starts producing unnatural results.
// Upstream's, unchanged: `0.25 - (1 / 16)`.
const RCAS_LIMIT: f32 = 0.1875;

// 4x the peak instead of 1x, which is how upstream keeps a gradient
// climbing in MSAA-sized steps from reading as an edge.
const PEAK: vec2<f32> = vec2<f32>(10.0, -40.0);

// The high-pass above doubles as a grain detector, and the engine feeds
// this a frame whose contact shadows are a dithered ray and whose lights
// are sampled stochastically. Sharpening that noise is sharpening the
// sampling pattern.
const DENOISE: bool = true;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// The usual three-vertex cover. No vertex buffer, no index buffer.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

// 🔴 Clamped, because `textureLoad` out of bounds returns ZERO in WGSL
// where `texelFetch` clamps in GLSL. The same difference put a one-pixel
// black border on SGSR 2's first run, and here it would read as four
// black neighbours — maximum local contrast, maximum sharpening, on
// every edge of the screen.
fn tap(texel: vec2<i32>, offset: vec2<i32>, limit: vec2<i32>) -> vec3<f32> {
    return textureLoad(image, clamp(texel + offset, vec2<i32>(0), limit), 0).rgb;
}

@fragment
fn fs_main(in: Varyings) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.position.xy);
    let limit = vec2<i32>(textureDimensions(image)) - vec2<i32>(1);

    // The 3x3 cross, which is the whole neighbourhood this reads.
    //    b
    //  d e f
    //    h
    let b = tap(texel, vec2<i32>(0, -1), limit);
    let d = tap(texel, vec2<i32>(-1, 0), limit);
    let e = textureLoad(image, texel, 0);
    let f = tap(texel, vec2<i32>(1, 0), limit);
    let h = tap(texel, vec2<i32>(0, 1), limit);

    // RCAS filters with a negative lobe in a cross:
    //   output = (w * (b + d + f + h) + e) / (4 * w + 1)
    // and solves for the `w` at which that would clip out of {0, 1},
    // using the ring's min and max rather than the individual taps so a
    // gradient does not read as an edge.
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));
    let hit_min = mn4 / (4.0 * mx4);
    let hit_max = (PEAK.x - mx4) / (PEAK.y + 4.0 * mn4);
    let lobe_rgb = max(-hit_min, hit_max);
    var lobe = max(-RCAS_LIMIT, min(0.0, max(lobe_rgb.r, max(lobe_rgb.g, lobe_rgb.b))))
        * params.sharpness;

    if (DENOISE) {
        // Luma times two, upstream's cheap form.
        let b_l = b.b * 0.5 + (b.r * 0.5 + b.g);
        let d_l = d.b * 0.5 + (d.r * 0.5 + d.g);
        let e_l = e.b * 0.5 + (e.r * 0.5 + e.g);
        let f_l = f.b * 0.5 + (f.r * 0.5 + f.g);
        let h_l = h.b * 0.5 + (h.r * 0.5 + h.g);
        // A high-pass normalised against the local range: near 1 where
        // the centre disagrees with its neighbours as much as they
        // disagree with each other, which is what grain looks like and
        // an edge does not.
        var noise = 0.25 * (b_l + d_l + f_l + h_l) - e_l;
        let range = max(max(b_l, d_l), max(f_l, h_l)) - min(min(b_l, d_l), min(f_l, h_l));
        // ⚠️ The one departure from upstream: the divisor is guarded.
        // A flat neighbourhood has a range of exactly zero and the
        // numerator with it, and 0/0 is a NaN that `saturate` is not
        // required to clean up — one NaN pixel here is a lobe of NaN and
        // a hole in the image. Sky is a flat neighbourhood.
        noise = saturate(abs(noise) / max(range, 1e-5));
        lobe *= 1.0 - 0.5 * noise;
    }

    // Alpha is coverage, not opacity: the tests read it to tell a shaded
    // pixel from the background, so the centre's passes through.
    return vec4<f32>(
        (lobe * (b + d + f + h) + e.rgb) / (4.0 * lobe + 1.0),
        e.a,
    );
}
