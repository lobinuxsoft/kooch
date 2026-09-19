// transparent_composite.wgsl — the four layers front to back, then the tail, over the opaque
// radiance (#452). Premultiplied: the blend adds the colour and scales what is behind by the
// light that gets through.

struct CompositeUbo {
    size: vec2<u32>,
    _pad: vec2<u32>,
}

@group(0) @binding(0) var<storage, read> layers: array<u64>;
@group(0) @binding(1) var accum_texture: texture_2d<f32>;
@group(0) @binding(2) var reveal_texture: texture_2d<f32>;
@group(0) @binding(3) var<uniform> composite: CompositeUbo;

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(position.xy);
    var colour = vec3<f32>(0.0);
    var through = 1.0;
    for (var layer = 0u; layer < LAYERS; layer += 1u) {
        let held = layers[layer_at(pixel, composite.size.x, layer)];
        if (!is_shaded(held)) {
            break;
        }
        let lit = unpack_shaded(held);
        colour += through * lit.rgb * lit.a;
        through *= 1.0 - lit.a;
    }
    let accum = textureLoad(accum_texture, pixel, 0);
    let reveal = textureLoad(reveal_texture, pixel, 0).r;
    let tail = select(vec3<f32>(0.0), accum.rgb / accum.a, accum.a > 1e-5) * (1.0 - reveal);
    colour += through * tail;
    through *= reveal;
    return vec4<f32>(colour, 1.0 - through);
}
