// The engine's 2D shapes: `// use: engine:shapes`. Every `///` function is a node in the graph.

fn lib_box(point: vec2<f32>, extent: vec2<f32>, radius: f32) -> f32 {
    let r = min(max(radius, 0.0), min(extent.x, extent.y));
    let q = abs(point) - extent + r;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn lib_line(distance: f32, width: f32, softness: f32) -> f32 {
    return clamp(0.5 - (abs(distance) - width) / max(softness, 0.00001), 0.0, 1.0);
}

/// Two lines around a rounded box, `gap` apart: a card's frame, a button's border.
fn lib_double_frame(
    uv: vec2<f32>,
    centre: vec2<f32>, // @default(0.5, 0.5)
    extent: vec2<f32>, // @default(0.45, 0.45)
    radius: f32, // @default(0.03)
    gap: f32, // @default(0.02)
    width: f32, // @default(0.004)
    softness: f32, // @default(0.002)
) -> f32 {
    let d = lib_box(uv - centre, extent, radius);
    return max(lib_line(d, width, softness), lib_line(d + gap, width, softness));
}

/// Dark along the inside of a rounded box, 1 past `reach`: a recessed window.
fn lib_inner_shadow(
    uv: vec2<f32>,
    centre: vec2<f32>, // @default(0.5, 0.5)
    extent: vec2<f32>, // @default(0.45, 0.45)
    radius: f32, // @default(0.03)
    reach: f32, // @default(0.04)
    darkness: f32, // @default(0.7)
) -> f32 {
    let d = -lib_box(uv - centre, extent, radius);
    return mix(1.0 - darkness, 1.0, smoothstep(0.0, max(reach, 0.00001), d));
}
