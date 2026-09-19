// transparent_tail.wgsl — what lies behind a pixel's four layers, blended without order (#452):
// McGuire and Bavoil's weighted blended OIT, over a tail that only dense transparency reaches.

@group(0) @binding(5) var<storage, read_write> layers: array<u64>;

struct TailOut {
    @location(0) accum: vec4<f32>,
    @location(1) reveal: f32,
}

@fragment
fn fs_tail(in: ForwardOut) -> TailOut {
    if (opaque_covers(in.position)) {
        discard;
    }
    let pixel = vec2<u32>(in.position.xy);
    let key = layer_key(in.position.z, in.slot, in.triangle);
    // At or in front of the fourth layer: one of the four, drawn exactly by the layers.
    if (key >= layers[layer_at(pixel, screen.size.x, LAYERS - 1u)]) {
        discard;
    }
    let lit = transparent_lit(in.slot, in.triangle, in.position.xy);
    let world = resolve_surface(in.slot, in.triangle, in.position.xy).world_position;
    let distance = length(inti.camera_position - world);
    // Equation 9 of the paper, in metres: nearer and more opaque weighs more.
    let weight = lit.a * clamp(0.03 / (1e-5 + pow(distance / 200.0, 4.0)), 1e-2, 3e3);
    var out: TailOut;
    out.accum = vec4<f32>(lit.rgb * lit.a, lit.a) * weight;
    out.reveal = lit.a;
    return out;
}
