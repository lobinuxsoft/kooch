// kind: ui
// An arc that fills clockwise from the top: a circular health bar or cooldown.
// `background-color` is the fill, `border-color` the track behind it.
struct SurfaceParams {
    fill: f32,       // @range(0, 1) @default(0.75)
    thickness: f32,  // @range(0, 0.5) @default(0.14)
}

fn ui(input: UiInput) -> vec4<f32> {
    let p = surface_params(input.material_id);
    let point = (input.uv - 0.5) * 2.0;
    let pixel = 2.0 / min(input.size.x, input.size.y);
    let ring = abs(length(point) - (1.0 - p.thickness)) - p.thickness;
    let coverage = clamp(0.5 - ring / pixel, 0.0, 1.0);
    let turn = fract(atan2(point.x, -point.y) / 6.2831853 + 1.0);
    let filled = clamp((p.fill - turn) * 6.2831853 / pixel + 0.5, 0.0, 1.0);
    let color = mix(input.border_color, input.color, filled);
    return vec4<f32>(color.rgb, color.a * coverage);
}
