// transparent_layers.wgsl — the four nearest transparent fragments of each pixel (#452).
//
// A layer is one 64-bit word. Before shading it is a key: the depth's bits above, the fragment's
// (list slot, triangle) below — reversed-Z, so a larger key is nearer, and 0 is empty. Shading
// replaces it with the lit colour and coverage as four halves, marked by `SHADED` in the sign bit
// of the coverage, which a depth's bits never have.

const LAYERS: u32 = 4u;
const SHADED: u32 = 0x80000000u;
const LOW_WORD: u64 = 0xfffffffflu;

fn layer_at(pixel: vec2<u32>, width: u32, layer: u32) -> u32 {
    return (pixel.y * width + pixel.x) * LAYERS + layer;
}

fn layer_key(depth: f32, slot: u32, triangle: u32) -> u64 {
    return (u64(bitcast<u32>(depth)) << 32u) | u64((slot << 7u) | (triangle & 0x7fu));
}

fn key_slot(key: u64) -> u32 {
    return u32(key & LOW_WORD) >> 7u;
}

fn key_triangle(key: u64) -> u32 {
    return u32(key & LOW_WORD) & 0x7fu;
}

fn is_shaded(layer: u64) -> bool {
    return (u32(layer >> 32u) & SHADED) != 0u;
}

// Halves: colour to 65504, far past anything the tonemap keeps.
fn pack_shaded(lit: vec4<f32>) -> u64 {
    let colour = min(lit.rgb, vec3<f32>(65000.0));
    let low = pack2x16float(colour.rg);
    let high = pack2x16float(vec2<f32>(colour.b, lit.a)) | SHADED;
    return (u64(high) << 32u) | u64(low);
}

fn unpack_shaded(layer: u64) -> vec4<f32> {
    let low = unpack2x16float(u32(layer & LOW_WORD));
    let high = unpack2x16float(u32(layer >> 32u) & ~SHADED);
    return vec4<f32>(low, high);
}
