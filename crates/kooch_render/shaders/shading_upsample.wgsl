// shading_upsample.wgsl — half-rate lighting back to full resolution (#825).

struct UpsampleUniforms {
    // Full resolution.
    size: vec2<u32>,
    // Shaded-target resolution, `ceil(size / rate)`.
    shaded_size: vec2<u32>,
}

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, read>;
@group(0) @binding(1) var shaded_color: texture_2d<f32>;
@group(0) @binding(2) var shaded_ids: texture_2d<u32>;
@group(0) @binding(3) var<uniform> up: UpsampleUniforms;

struct FsInput {
    @builtin(position) position: vec4<f32>,
}

// Fullscreen triangle cover — the same 3-vertex trick as
// `meshlet_blit.wgsl`.
@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FsInput {
    var out: FsInput;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_upsample(in: FsInput) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.position.xy);
    let visibility = textureLoad(vbuf64, pixel).x;
    if ((visibility >> 32u) == 0lu) {
        // Background. Transparent, so the blit shows the sky — the same
        // contract the full-rate path's cleared target has.
        return vec4<f32>(0.0);
    }
    let my_slot = u32(visibility) >> 7u;

    // Sample centres sit a quarter of a shaded texel from the shaded grid's own centres, because
    // the sample is a corner of its quad and not its middle.
    let coord = vec2<f32>(pixel) * 0.5;
    let base = vec2<i32>(floor(coord));
    let frac = coord - floor(coord);
    let last = vec2<i32>(up.shaded_size) - vec2<i32>(1);

    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let step = vec2<i32>(i32(i & 1u), i32(i >> 1u));
        let at = clamp(base + step, vec2<i32>(0), last);
        let w = mix(1.0 - frac.x, frac.x, f32(step.x))
              * mix(1.0 - frac.y, frac.y, f32(step.y));
        // A zero-weight neighbour contributes nothing; skipping it saves
        // two texture loads on every pixel that lands exactly on a
        // shaded sample, which at this rate is one pixel in four.
        if (w <= 0.0) {
            continue;
        }
        let id = textureLoad(shaded_ids, at, 0).x;
        // 0 is "this sample shaded nothing"; the rest is `slot + 1`.
        if (id != my_slot + 1u) {
            continue;
        }
        sum += textureLoad(shaded_color, at, 0).rgb * w;
        weight += w;
    }

    if (weight > 0.0) {
        return vec4<f32>(sum / weight, 1.0);
    }
    // Nothing in the neighbourhood came from this surface — a silhouette, or geometry thinner than
    // a quad. Take the pixel's own quad, which the shading pass guarantees was shaded whenever this
    // pixel is covered.
    let own = clamp(base, vec2<i32>(0), last);
    return vec4<f32>(textureLoad(shaded_color, own, 0).rgb, 1.0);
}
