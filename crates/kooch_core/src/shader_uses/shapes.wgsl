// The engine's 2D shapes: `// use: engine:shapes`. Every `///` function is a node in the graph.

fn lib_box(point: vec2<f32>, extent: vec2<f32>, radius: f32) -> f32 {
    let r = min(max(radius, 0.0), min(extent.x, extent.y));
    let q = abs(point) - extent + r;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn lib_line(distance: f32, width: f32, softness: f32) -> f32 {
    return clamp(0.5 - (abs(distance) - width) / max(softness, 0.00001), 0.0, 1.0);
}

/// How filled a bar is at `along`, 0 to 1, with an edge `softness` wide: all of it at 1, none at 0.
fn lib_fill(along: f32, fill: f32, softness: f32) -> f32 {
    let soft = max(softness, 0.00001);
    // Past both ends by one edge, or a cap sitting at exactly 1 stops half way.
    let reach = fill * (1.0 + soft * 2.0) - soft;
    return clamp((reach - along) / soft + 0.5, 0.0, 1.0);
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

struct LibBezier {
    distance: f32,
    along: f32,
}

fn lib_bezier_at(d: vec2<f32>, b: vec2<f32>, c: vec2<f32>, t: f32) -> f32 {
    let off = d + (c + b * t) * t;
    return dot(off, off);
}

/// Distance to a curve from `start` to `end`, pulled toward `control`, and how far along it, 0 to 1.
fn lib_bezier(
    uv: vec2<f32>,
    start: vec2<f32>, // @default(0.1, 0.8)
    control: vec2<f32>, // @default(0.5, 0.1)
    end: vec2<f32>, // @default(0.9, 0.8)
) -> LibBezier {
    let a = control - start;
    let b = start - 2.0 * control + end;
    let c = a * 2.0;
    let d = start - uv;
    // A straight curve has no cubic to solve, and dividing by `b` would blow it up.
    if dot(b, b) < 0.0000001 {
        let line = end - start;
        let t = clamp(dot(uv - start, line) / max(dot(line, line), 0.0000001), 0.0, 1.0);
        return LibBezier(length(uv - start - line * t), t);
    }
    // The closest point is a root of a cubic, in closed form after Inigo Quilez (Shadertoy MlKcDD).
    let kk = 1.0 / dot(b, b);
    let kx = kk * dot(a, b);
    let ky = kk * (2.0 * dot(a, a) + dot(d, b)) / 3.0;
    let kz = kk * dot(d, a);
    let p = ky - kx * kx;
    let q = kx * (2.0 * kx * kx - 3.0 * ky) + kz;
    let h = q * q + 4.0 * p * p * p;
    if h >= 0.0 {
        let root = sqrt(h);
        let x = (vec2<f32>(root, -root) - q) / 2.0;
        let cube = sign(x) * pow(abs(x), vec2<f32>(1.0 / 3.0));
        let t = clamp(cube.x + cube.y - kx, 0.0, 1.0);
        return LibBezier(sqrt(lib_bezier_at(d, b, c, t)), t);
    }
    let z = sqrt(-p);
    let v = acos(clamp(q / (p * z * 2.0), -1.0, 1.0)) / 3.0;
    let m = cos(v);
    let n = sin(v) * 1.732050808;
    let t1 = clamp((m + m) * z - kx, 0.0, 1.0);
    let t2 = clamp((-n - m) * z - kx, 0.0, 1.0);
    let d1 = lib_bezier_at(d, b, c, t1);
    let d2 = lib_bezier_at(d, b, c, t2);
    return LibBezier(sqrt(min(d1, d2)), select(t2, t1, d1 < d2));
}
