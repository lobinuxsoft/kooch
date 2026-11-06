// kind: ui
// use: engine:shapes
// A bar along a curve, filling from `start` to `end`. `background-color` is the fill,
// `border-color` the track behind it. The UI Builder drags the three points.
struct SurfaceParams {
    fill: f32,          // @range(0, 1)
    thickness: f32,     // @range(0, 1)
    start: vec2<f32>,   // @point
    control: vec2<f32>, // @point
    end: vec2<f32>,     // @point
}

const SURFACE_DEFAULTS = SurfaceParams(
    0.75,
    0.2,
    vec2<f32>(0.1, 0.75),
    vec2<f32>(0.5, 0.1),
    vec2<f32>(0.9, 0.75),
);

fn ui(input: UiInput) -> vec4<f32> {
    let p = surface_params(input.material_id);
    // In pixels: the points are fractions of the node, and a wide node would squash the thickness.
    let size = input.size;
    let curve = lib_bezier(input.uv * size, p.start * size, p.control * size, p.end * size);
    let coverage = clamp(0.5 - (curve.distance - p.thickness * 0.5 * size.y), 0.0, 1.0);
    let filled = lib_fill(curve.along, p.fill, 1.0 / max(size.x, size.y));
    let color = mix(input.border_color, input.color, filled);
    return vec4<f32>(color.rgb, color.a * coverage);
}
