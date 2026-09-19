// material_frame_forward.wgsl — the sorted fallback (#452): transparent instances far to near,
// blended over the opaque radiance before the temporal resolve. Used where the layered passes
// cannot run.

@fragment
fn fs_forward(in: ForwardOut) -> @location(0) vec4<f32> {
    let lit = transparent_lit(in.slot, in.triangle, in.position.xy);
    if (lit.a < 0.0) {
        discard;
    }
    return lit;
}
