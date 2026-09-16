// Simplex noise in 2D, after Stefan Gustavson's construction: the plane cut into triangles rather than
// squares, so it has no axis-aligned streaks and costs three lattice points instead of four. 0..1.
fn graph_simplex_noise(uv: vec2<f32>) -> f32 {
    // Skew into the triangle lattice and back: (sqrt(3) - 1) / 2 and (3 - sqrt(3)) / 6.
    let skew = 0.36602540;
    let unskew = 0.21132487;
    let cell = floor(uv + (uv.x + uv.y) * skew);
    let a = uv - cell + (cell.x + cell.y) * unskew;
    let corner = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), a.x > a.y);
    let b = a - corner + unskew;
    let c = a - 1.0 + 2.0 * unskew;
    let falloff = max(vec3<f32>(0.5) - vec3<f32>(dot(a, a), dot(b, b), dot(c, c)), vec3<f32>(0.0));
    let contribution = falloff * falloff * falloff * falloff * vec3<f32>(
        dot(a, graph_gradient(cell)),
        dot(b, graph_gradient(cell + corner)),
        dot(c, graph_gradient(cell + vec2<f32>(1.0))),
    );
    return dot(contribution, vec3<f32>(70.0)) * 0.5 + 0.5;
}

fn graph_simplex_fbm(uv: vec2<f32>, octaves: f32) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 0.5;
    var p = uv;
    for (var i = 0; i < count; i = i + 1) {
        sum = sum + amplitude * graph_simplex_noise(p);
        weight = weight + amplitude;
        amplitude = amplitude * 0.5;
        p = p * 2.0;
    }
    return sum / weight;
}
