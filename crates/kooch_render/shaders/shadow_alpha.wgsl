// shadow_alpha.wgsl — whether a caster's fragment stands in a shadow view (#1224). An opaque
// material always does; a transparent one does where its baked coverage beats an ordered dither,
// which the shadow filter then averages into partial shadow.

@group({{ALPHA_GROUP}}) @binding(0) var shadow_alpha_atlas: texture_2d_array<f32>;
@group({{ALPHA_GROUP}}) @binding(1) var shadow_alpha_sampler: sampler;
// Per material slot: 0 casts solid, n reads layer n - 1 of the atlas. The top bit marks a masked
// material (#452), whose coverage is a cut rather than an opacity.
@group({{ALPHA_GROUP}}) @binding(2) var<storage, read> shadow_alpha_layers: array<u32>;

const SHADOW_ALPHA_MASKED: u32 = 0x80000000u;

fn shadow_alpha_keeps(material: u32, uv: vec2<f32>, texel: vec2<f32>) -> bool {
    if material >= arrayLength(&shadow_alpha_layers) {
        return true;
    }
    let entry = shadow_alpha_layers[material];
    let layer = entry & ~SHADOW_ALPHA_MASKED;
    if layer == 0u {
        return true;
    }
    let alpha = textureSampleLevel(shadow_alpha_atlas, shadow_alpha_sampler, uv, layer - 1u, 0.0).r;
    if (entry & SHADOW_ALPHA_MASKED) != 0u {
        return alpha >= 0.5;
    }
    // 4×4 Bayer: sixteen evenly spread thresholds, so a 30% pane blocks 30% of any 4×4 block.
    var bayer = array<f32, 16>(
        0.0, 8.0, 2.0, 10.0,
        12.0, 4.0, 14.0, 6.0,
        3.0, 11.0, 1.0, 9.0,
        15.0, 7.0, 13.0, 5.0,
    );
    let at = vec2<u32>(texel) & vec2<u32>(3u);
    return alpha > (bayer[at.y * 4u + at.x] + 0.5) / 16.0;
}
