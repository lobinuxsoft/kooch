// video.wgsl — a video frame's YUV planes drawn as RGB where it lands (#1793): a UI atlas slot or a
// material texture. BT.709, as `Picture::rgba` on the CPU.

struct Params {
    // The target pixel of the frame's top-left border texel.
    origin: vec2<f32>,
    // 1 for full 0–255 levels, 0 for video's 16–235.
    full: u32,
    // 1 when the target has a border texel around the frame (an atlas slot).
    border: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var luma: texture_2d<f32>;
@group(0) @binding(2) var cb: texture_2d<f32>;
@group(0) @binding(3) var cr: texture_2d<f32>;
@group(0) @binding(4) var chroma: sampler;

@vertex
fn vs_video(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn video_rgb(position: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(luma));
    // The border repeats the edge, as the atlas's CPU blit does.
    let pixel = clamp(position - params.origin - f32(params.border), vec2(0.5), size - 0.5);
    var y = textureLoad(luma, vec2<i32>(pixel), 0).r;
    var c = vec2<f32>(
        textureSampleLevel(cb, chroma, pixel / size, 0.0).r,
        textureSampleLevel(cr, chroma, pixel / size, 0.0).r,
    ) - 128.0 / 255.0;
    if params.full == 0u {
        y = (y - 16.0 / 255.0) * (255.0 / 219.0);
        c = c * (255.0 / 224.0);
    }
    let rgb = vec3<f32>(y + 1.5748 * c.y, y - 0.1873 * c.x - 0.4681 * c.y, y + 1.8556 * c.x);
    return clamp(rgb, vec3(0.0), vec3(1.0));
}

@fragment
fn fs_video(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(video_rgb(position.xy), 1.0);
}

// For an sRGB target, which encodes on write: linear in, the same bytes out.
@fragment
fn fs_video_srgb(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let rgb = video_rgb(position.xy);
    let low = rgb / 12.92;
    let high = pow((rgb + 0.055) / 1.055, vec3(2.4));
    return vec4<f32>(select(high, low, rgb <= vec3(0.04045)), 1.0);
}
