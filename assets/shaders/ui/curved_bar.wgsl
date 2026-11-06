// kind: ui
// use: engine:shapes
// A bar bent along an arc, filling left to right. `background-color` is the fill,
// `border-color` the track behind it.
struct SurfaceParams {
    fill: f32,       // @range(0, 1)
    bend: f32,       // @range(0, 1)
    thickness: f32,  // @range(0, 1)
}

const SURFACE_DEFAULTS = SurfaceParams(0.75, 0.35, 0.4);

fn ui(input: UiInput) -> vec4<f32> {
    let p = surface_params(input.material_id);
    let across = input.uv.x * 2.0 - 1.0;
    let centre = 0.5 + p.bend * 0.5 * (across * across - 0.5);
    let pixel = 1.0 / input.size.y;
    let band = abs(input.uv.y - centre) - p.thickness * 0.5;
    let coverage = clamp(0.5 - band / pixel, 0.0, 1.0);
    let filled = lib_fill(input.uv.x, p.fill, 1.0 / input.size.x);
    let color = mix(input.border_color, input.color, filled);
    return vec4<f32>(color.rgb, color.a * coverage);
}
