// ui_material_frame.wgsl — the entry point around a `kind: ui` shader's `ui`.

@fragment
fn fs_material(in: Varyings) -> @location(0) vec4<f32> {
    let p = in.straight;
    if p.x < in.clip.x || p.y < in.clip.y || p.x >= in.clip.z || p.y >= in.clip.w {
        discard;
    }
    var input: UiInput;
    input.size = in.half_size * 2.0;
    input.uv = in.local / input.size + 0.5;
    input.color = in.color;
    input.border_color = in.border_color;
    input.time = screen.time;
    input.material_id = in.material_flags.x;
    input.image = (in.material_flags.y & QUAD_IMAGE) != 0u;
    input.atlas_min = vec2<f32>(in.atlas_rect.xy);
    input.atlas_size = vec2<f32>(in.atlas_rect.zw);
    let color = ui(input);
    let edge = sd_rounded_box(in.local, in.half_size, in.radius_border.x);
    let alpha = color.a * clamp(0.5 - edge, 0.0, 1.0) * ui_mask_coverage(p, in.mask);
    return vec4<f32>(color.rgb * alpha, alpha);
}
