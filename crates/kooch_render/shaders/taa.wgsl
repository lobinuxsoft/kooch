// taa.wgsl — temporal anti-aliasing (#481).
//
// A port of Bevy's `taa.wesl`, which is itself the Playdead / Karis
// lineage that every shipped TAA descends from. Ported rather than
// invented, per the standing rule: the failure modes here are subtle,
// well documented and already solved, and a hand-rolled resolve
// rediscovers ghosting, swimming and fireflies one at a time.
//
// # What it is for, here
//
// Not edge quality. The froxel sampler (#826) chooses a subset of the
// lights per pixel and pays for it in noise; the contact shadows march
// a dithered ray and pay for it in noise; half-rate shading (#825)
// interpolates and pays for it in blocks. All three are cheap because
// they are stochastic, and stochastic is only cheap if something
// averages the samples. This is that something.
//
// # 🔴 Three departures from upstream, each with a number behind it
//
// **1. The history holds LINEAR radiance, not the compressed value.**
// Bevy stores the range-compressed colour and reads it back straight.
// Measured here, that costs precision where it is most visible: the
// compressed value of a bright pixel sits near 1, fp16 resolves it to
// about 0.0005, and `reverse_tonemap` multiplies that error by
// `1/(1-t)²` — a hundredfold at t = 0.9. Storing linear and compressing
// on read moves the quantisation to where the amplification is not.
//
// Same still scene, twenty frames, frame-to-frame change of the final
// image: **0.09 storing linear against 0.85 storing compressed.** It
// also means the resolve IS the history — one texture instead of two —
// which saves eight bytes a pixel a frame on a device that measures as
// memory-bound.
//
// The hazard this creates is real and guarded rather than ignored: see
// `YCoCg_to_RGB`, whose upstream `saturate` would otherwise hand
// `reverse_tonemap` its own pole.
//
// **2. The variance clip is two sigma, not one.** Upstream's one sigma
// rejected the history on 11 % of pixels of a scene that was not moving
// at all, and a rejected history is placed on the box boundary — which
// is a contrast boost, not a rejection. Measured on the still scene, as
// the edge energy of the final image against 123 for a single
// unresolved frame:
//
//     no clip   114    settles, frame-to-frame change 0.003
//     2 sigma   124    settles, 0.049
//     1 sigma   135    does not settle, 0.09 and holding
//
// One sigma leaves the image with MORE edge contrast than no temporal
// pass at all, which is the opposite of the thing being bought. Two is
// inside the range shipped renderers use and is the widest the still
// scene needed; wider ghosts.
//
// **3. The reset path is a uniform rather than a shader variant**, and
// it seeds the confidence at 1 rather than at `1 /
// MIN_HISTORY_BLEND_RATE`. See where each is set.
//
// Blend weights, empirical, from Bevy and unchanged. Lower means less of
// the current sample and more of the past — more smoothing.
const DEFAULT_HISTORY_BLEND_RATE: f32 = 0.1;
const MIN_HISTORY_BLEND_RATE: f32 = 0.015;

struct TaaUniforms {
    // 1 on the first frame after the history was allocated or resized.
    // There is nothing to reproject from, and blending against a cleared
    // texture is a black frame that fades in over sixty frames.
    reset: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var view_target: texture_2d<f32>;
@group(0) @binding(1) var history: texture_2d<f32>;
@group(0) @binding(2) var history_confidence: texture_2d<f32>;
@group(0) @binding(3) var motion_vectors: texture_2d<f32>;
@group(0) @binding(4) var depth: texture_depth_2d;
@group(0) @binding(5) var nearest_sampler: sampler;
@group(0) @binding(6) var linear_sampler: sampler;
@group(0) @binding(7) var<uniform> taa: TaaUniforms;

struct Output {
    // Resolved LINEAR radiance — what the tonemap reads, and what next
    // frame reads back as its history. Alpha is coverage, untouched.
    @location(0) color: vec4<f32>,
    // How many still frames this pixel has accumulated. Its own target
    // because alpha above means coverage by the time the tonemap gets
    // it; two bytes a pixel to keep the two meanings apart.
    @location(1) confidence: vec4<f32>,
}

// The reversible tonemapper from GPUOpen, via Bevy. Blending has to
// happen in a range-compressed space or one firefly drags the whole
// neighbourhood: a pixel at radiance 400 and its neighbour at 4 average
// to 202, which is a bright dot that then survives in the history.
fn rcp(x: f32) -> f32 { return 1.0 / x; }
fn max3(x: vec3<f32>) -> f32 { return max(x.r, max(x.g, x.b)); }
fn tonemap(color: vec3<f32>) -> vec3<f32> { return color * rcp(max3(color) + 1.0); }

// The inverse, and 1.0 is its pole: `1 / (1 - 1)`. Nothing that reaches
// here should be there — the forward operator maps `[0, inf)` into
// `[0, 1)` and `TONEMAP_CEILING` keeps the clip short of the end — and
// the floor is the guard rail for the case nobody thought of, because
// one infinity written into the history stays in the history.
fn reverse_tonemap(color: vec3<f32>) -> vec3<f32> {
    return color * rcp(max(1.0 - max3(color), 1.0 / 65504.0));
}

// 🔴 How close to 1 the compressed space is allowed to get, and
// therefore the brightest radiance a clipped history can describe:
// `t/(1-t)` at this value is 1023.
//
// Upstream's `YCoCg_to_RGB` ends in a plain `saturate`, which is free
// there because the value never leaves the compressed space. Here it
// does — through `reverse_tonemap`, whose pole is exactly the number
// `saturate` clamps to. A single bright pixel landing on it comes back
// as 65504, poisons its neighbourhood's variance the next frame, and
// spreads.
const TONEMAP_CEILING: f32 = 1023.0 / 1024.0;

// Playdead's YCoCg pair (MIT). The clip happens in a luminance-chroma
// basis because the neighbourhood of a pixel varies far more in
// brightness than in hue, and an RGB box around it is loose in the
// direction that matters and tight in the two that do not.
fn RGB_to_YCoCg(rgb: vec3<f32>) -> vec3<f32> {
    let y = (rgb.r / 4.0) + (rgb.g / 2.0) + (rgb.b / 4.0);
    let co = (rgb.r / 2.0) - (rgb.b / 2.0);
    let cg = (-rgb.r / 4.0) + (rgb.g / 2.0) - (rgb.b / 4.0);
    return vec3(y, co, cg);
}

fn YCoCg_to_RGB(ycocg: vec3<f32>) -> vec3<f32> {
    let r = ycocg.x + ycocg.y - ycocg.z;
    let g = ycocg.x + ycocg.z;
    let b = ycocg.x - ycocg.y - ycocg.z;
    // Not `saturate`: see TONEMAP_CEILING.
    return clamp(vec3(r, g, b), vec3(0.0), vec3(TONEMAP_CEILING));
}

// Pulls the history towards the centre of the neighbourhood's box until
// it is inside it. Clipping rather than clamping: clamping each channel
// independently moves the colour's hue, which is what makes a rejected
// history flash a wrong colour instead of just a wrong brightness.
fn clip_towards_aabb_center(
    history_color: vec3<f32>,
    current_color: vec3<f32>,
    aabb_min: vec3<f32>,
    aabb_max: vec3<f32>,
) -> vec3<f32> {
    let p_clip = 0.5 * (aabb_max + aabb_min);
    let e_clip = 0.5 * (aabb_max - aabb_min) + 0.00000001;
    let v_clip = history_color - p_clip;
    let v_unit = v_clip / e_clip;
    let a_unit = abs(v_unit);
    let ma_unit = max3(a_unit);
    if (ma_unit > 1.0) {
        return p_clip + (v_clip / ma_unit);
    }
    return history_color;
}

// 🔴 Compressed on read, because the history holds linear radiance —
// departure 1 in the header. The Catmull-Rom weighting below has to
// happen in the compressed space or a firefly in the history survives
// the filter the same way it would survive the blend, and weighting the
// compressed taps here is exactly what upstream does with taps it
// compressed on write.
fn sample_history(u: f32, v: f32) -> vec3<f32> {
    return tonemap(textureSample(history, linear_sampler, vec2(u, v)).rgb);
}

fn sample_view_target(uv: vec2<f32>) -> vec3<f32> {
    let s = textureSample(view_target, nearest_sampler, uv).rgb;
    return RGB_to_YCoCg(tonemap(s));
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_taa(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

@fragment
fn fs_taa(in: Varyings) -> Output {
    let uv = in.uv;
    let texture_size = vec2<f32>(textureDimensions(view_target));
    let texel_size = 1.0 / texture_size;

    let original_color = textureSample(view_target, nearest_sampler, uv);
    var current_color = tonemap(original_color.rgb);
    var confidence = 1.0;

    if (taa.reset == 0u) {
        // Closest of five depths, not the pixel's own vector. A pixel on
        // the silhouette of a moving object reads the background's
        // vector as often as the object's, and reprojecting the edge of
        // a car to where the road was is the classic TAA smear.
        let offset = texel_size * 2.0;
        let d_uv_tl = uv + vec2(-offset.x, offset.y);
        let d_uv_tr = uv + vec2(offset.x, offset.y);
        let d_uv_bl = uv + vec2(-offset.x, -offset.y);
        let d_uv_br = uv + vec2(offset.x, -offset.y);
        var closest_uv = uv;
        var closest_depth = textureSample(depth, nearest_sampler, uv);
        let d_tl = textureSample(depth, nearest_sampler, d_uv_tl);
        let d_tr = textureSample(depth, nearest_sampler, d_uv_tr);
        let d_bl = textureSample(depth, nearest_sampler, d_uv_bl);
        let d_br = textureSample(depth, nearest_sampler, d_uv_br);
        if (d_tl > closest_depth) { closest_uv = d_uv_tl; closest_depth = d_tl; }
        if (d_tr > closest_depth) { closest_uv = d_uv_tr; closest_depth = d_tr; }
        if (d_bl > closest_depth) { closest_uv = d_uv_bl; closest_depth = d_bl; }
        if (d_br > closest_depth) { closest_uv = d_uv_br; }
        let closest_motion_vector = textureSample(motion_vectors, nearest_sampler, closest_uv).rg;

        // 🔴 MINUS. A vector says where the surface came FROM, so the
        // history lives at `uv - motion`. This sign is derived and
        // asserted in `motion_vectors.rs`; flipping it here reprojects
        // every pass away from its own history and looks like the
        // history simply not working.
        let history_uv = uv - closest_motion_vector;

        // Five-tap Catmull-Rom rather than a bilinear read. Reprojection
        // resamples the history every single frame, and a bilinear tap
        // loses a little sharpness each time — sixty frames of that is a
        // still image that slowly goes soft. Corners skipped, per
        // Activision: nine taps for a difference nobody sees.
        let sample_position = history_uv * texture_size;
        let texel_center = floor(sample_position - 0.5) + 0.5;
        let f = sample_position - texel_center;
        let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
        let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
        let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
        let w3 = f * f * (-0.5 + 0.5 * f);
        let w12 = w1 + w2;
        let texel_position_0 = (texel_center - 1.0) * texel_size;
        let texel_position_3 = (texel_center + 2.0) * texel_size;
        let texel_position_12 = (texel_center + (w2 / w12)) * texel_size;
        var history_color = sample_history(texel_position_12.x, texel_position_0.y) * w12.x * w0.y;
        history_color += sample_history(texel_position_0.x, texel_position_12.y) * w0.x * w12.y;
        history_color += sample_history(texel_position_12.x, texel_position_12.y) * w12.x * w12.y;
        history_color += sample_history(texel_position_3.x, texel_position_12.y) * w3.x * w12.y;
        history_color += sample_history(texel_position_12.x, texel_position_3.y) * w12.x * w3.y;

        // 3x3 variance clipping. This is the whole anti-ghosting story:
        // a history that no longer belongs — a surface that became
        // occluded, a light that switched off — lands outside the
        // neighbourhood's distribution and gets pulled back into it.
        let s_tl = sample_view_target(uv + vec2(-texel_size.x,  texel_size.y));
        let s_tm = sample_view_target(uv + vec2( 0.0,           texel_size.y));
        let s_tr = sample_view_target(uv + vec2( texel_size.x,  texel_size.y));
        let s_ml = sample_view_target(uv + vec2(-texel_size.x,  0.0));
        let s_mm = RGB_to_YCoCg(current_color);
        let s_mr = sample_view_target(uv + vec2( texel_size.x,  0.0));
        let s_bl = sample_view_target(uv + vec2(-texel_size.x, -texel_size.y));
        let s_bm = sample_view_target(uv + vec2( 0.0,          -texel_size.y));
        let s_br = sample_view_target(uv + vec2( texel_size.x, -texel_size.y));
        let moment_1 = s_tl + s_tm + s_tr + s_ml + s_mm + s_mr + s_bl + s_bm + s_br;
        let moment_2 = (s_tl * s_tl) + (s_tm * s_tm) + (s_tr * s_tr) + (s_ml * s_ml)
            + (s_mm * s_mm) + (s_mr * s_mr) + (s_bl * s_bl) + (s_bm * s_bm) + (s_br * s_br);
        let mean = moment_1 / 9.0;
        let variance = (moment_2 / 9.0) - (mean * mean);
        // Two sigma — departure 2 in the header, with the measurements.
        let std_deviation = sqrt(max(variance, vec3(0.0))) * 2.0;
        history_color = RGB_to_YCoCg(history_color);
        history_color = clip_towards_aabb_center(
            history_color, s_mm, mean - std_deviation, mean + std_deviation);
        history_color = YCoCg_to_RGB(history_color);

        // A pixel that has been still for a while has accumulated many
        // samples and its history is worth more than this frame's one.
        // This is where the noise from #826 actually goes: at a blend
        // rate of 1/64 the resolve is averaging sixty-odd independent
        // light choices per pixel.
        confidence = textureSample(history_confidence, nearest_sampler, uv).r;
        let pixel_motion_vector = abs(closest_motion_vector) * texture_size;
        if (pixel_motion_vector.x < 0.01 && pixel_motion_vector.y < 0.01) {
            confidence += 10.0;
        } else {
            confidence = 1.0;
        }

        var current_color_factor =
            clamp(1.0 / confidence, MIN_HISTORY_BLEND_RATE, DEFAULT_HISTORY_BLEND_RATE);

        // Off-screen history is not history. Reprojecting past the edge
        // clamps to the border texel, which smears the frame's outermost
        // row inwards for as long as the camera keeps turning.
        if (any(saturate(history_uv) != history_uv)) {
            current_color_factor = 1.0;
            confidence = 1.0;
        }

        current_color = mix(history_color, current_color, current_color_factor);
    } else {
        // 🔴 One, where Bevy writes `1 / MIN_HISTORY_BLEND_RATE`.
        //
        // The counter is how many samples this pixel is claimed to have
        // averaged. Bevy's reset credits a single frame with
        // sixty-seven of them, so the frame after a reset already blends
        // at the floor and the reset frame keeps most of its weight for
        // the next hundred — 69 % after sixteen frames, which is a
        // resolve that has barely resolved.
        //
        // Starting at one gives the ramp the increment was designed for
        // — 1/11, 1/21, 1/31 — reaching the floor in about seven frames,
        // which is the schedule of an actual running average. It costs a
        // noisier first half-dozen frames after a reset, and that is not
        // a cost so much as an admission: nothing has been accumulated
        // yet, and saying otherwise is what made the reset frame stick.
        confidence = 1.0;
    }

    var out: Output;
    // Back to linear: the tonemap expects radiance, and so does the next
    // frame reading this same texture as its history.
    out.color = vec4<f32>(reverse_tonemap(current_color), original_color.a);
    out.confidence = vec4<f32>(confidence, 0.0, 0.0, 0.0);
    return out;
}
