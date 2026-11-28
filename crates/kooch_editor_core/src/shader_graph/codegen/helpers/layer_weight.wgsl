// How much of `layer` a terrain shows here (#1951). Each corner's uv packs its four heaviest layers:
// ids in 5 bits each, then three shares in 15ths, the fourth what they leave. The ids are matched per
// corner and the shares blended by where the fragment sits, so no id is ever interpolated.
fn graph_layer_weight(input: SurfaceInput, layer: u32) -> f32 {
    var total = 0.0;
    for (var c = 0u; c < 3u; c++) {
        let ids = u32(input.corner_uvs[c].x);
        let packed = u32(input.corner_uvs[c].y);
        let shares = vec3<f32>(vec3<u32>(packed, packed >> 4u, packed >> 8u) & vec3<u32>(15u)) / 15.0;
        let last = max(1.0 - shares.x - shares.y - shares.z, 0.0);
        let weights = vec4<f32>(shares, last);
        for (var k = 0u; k < 4u; k++) {
            if (((ids >> (5u * k)) & 31u) == layer) {
                total += input.barycentrics[c] * weights[k];
            }
        }
    }
    return total;
}
