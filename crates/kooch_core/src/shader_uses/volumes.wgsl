// The engine's 3D shapes, signed distances for `volume` and `terrain` graphs (#1957):
// `// use: engine:volumes`. Every `///` function is a node in the graph. Negative is inside.

/// Distance to a ball of `radius` around `centre`.
fn lib_ball(
    point: vec3<f32>,
    centre: vec3<f32>, // @default(0.0, 0.0, 0.0)
    radius: f32, // @default(5.0)
) -> f32 {
    return length(point - centre) - radius;
}

/// Distance to a box `extent` from `centre` on each axis, its edges rounded by `rounding`.
fn lib_block(
    point: vec3<f32>,
    centre: vec3<f32>, // @default(0.0, 0.0, 0.0)
    extent: vec3<f32>, // @default(5.0, 5.0, 5.0)
    rounding: f32, // @default(0.0)
) -> f32 {
    let r = clamp(rounding, 0.0, min(extent.x, min(extent.y, extent.z)));
    let q = abs(point - centre) - extent + r;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}

/// Distance to a ring lying flat around `centre`: `radius` to the tube's middle, `tube` thick.
fn lib_torus(
    point: vec3<f32>,
    centre: vec3<f32>, // @default(0.0, 0.0, 0.0)
    radius: f32, // @default(5.0)
    tube: f32, // @default(1.5)
) -> f32 {
    let p = point - centre;
    return length(vec2<f32>(length(p.xz) - radius, p.y)) - tube;
}

/// Distance to a rod from `start` to `end` with round caps, `radius` thick.
fn lib_capsule(
    point: vec3<f32>,
    start: vec3<f32>, // @default(0.0, -5.0, 0.0)
    end: vec3<f32>, // @default(0.0, 5.0, 0.0)
    radius: f32, // @default(1.5)
) -> f32 {
    let pa = point - start;
    let ba = end - start;
    let t = clamp(dot(pa, ba) / max(dot(ba, ba), 1.0e-12), 0.0, 1.0);
    return length(pa - ba * t) - radius;
}

/// Distance to a flat-ended cylinder from `start` to `end`, `radius` thick. Along z it cuts an arch.
fn lib_cylinder(
    point: vec3<f32>,
    start: vec3<f32>, // @default(0.0, -5.0, 0.0)
    end: vec3<f32>, // @default(0.0, 5.0, 0.0)
    radius: f32, // @default(3.0)
) -> f32 {
    let ba = end - start;
    let pa = point - start;
    let baba = max(dot(ba, ba), 1.0e-12);
    let paba = dot(pa, ba);
    let x = length(pa * baba - ba * paba) - radius * baba;
    let y = abs(paba - baba * 0.5) - baba * 0.5;
    let x2 = x * x;
    let y2 = y * y * baba;
    let outside = select(0.0, x2, x > 0.0) + select(0.0, y2, y > 0.0);
    let d = select(outside, -min(x2, y2), max(x, y) < 0.0);
    return sign(d) * sqrt(abs(d)) / baba;
}
