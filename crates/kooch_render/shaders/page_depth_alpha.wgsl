// page_depth_alpha.wgsl — the depth pass's fragment where a caster is transparent (#1224): its
// baked coverage against an ordered dither, per atlas texel. Composed only while a transparent
// material casts, so an opaque scene's pages keep the fragment-less path.

// The fragment path: the page scissor, then the coverage.
@fragment
fn fs_page_alpha(in: PageVertex) {
    let at = in.clip.xy;
    if at.x < in.rect.x || at.x >= in.rect.x + in.rect.z {
        discard;
    }
    if at.y < in.rect.y || at.y >= in.rect.y + in.rect.w {
        discard;
    }
    if !shadow_alpha_keeps(in.material, in.uv, at) {
        discard;
    }
}

// The clipped path: the hardware already cut the triangle to its page.
@fragment
fn fs_page_alpha_clipped(
    @builtin(position) position: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) material: u32,
) {
    if !shadow_alpha_keeps(material, uv, position.xy) {
        discard;
    }
}
