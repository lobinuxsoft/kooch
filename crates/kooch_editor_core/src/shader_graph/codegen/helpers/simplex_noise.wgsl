// Simplex noise in 2D, after Stefan Gustavson's construction: the plane cut into triangles rather than
// squares, so it has no axis-aligned streaks and reads three lattice points instead of four. 0..1.
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

// Octaves of simplex noise. `mode` 0 sums them (fBm), 1 folds each about its middle (turbulence), 2 turns
// the fold into sharp crests (ridged). `roughness` is how much each octave keeps of the one before,
// `lacunarity` how much finer it is. Normalised back to 0..1.
fn graph_simplex_fractal(p: vec2<f32>, octaves: f32, roughness: f32, lacunarity: f32, mode: f32) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 1.0;
    var at = p;
    for (var i = 0; i < count; i = i + 1) {
        let n = graph_simplex_noise(at);
        let folded = abs(n * 2.0 - 1.0);
        let shaped = select(select(n, folded, mode > 0.5), (1.0 - folded) * (1.0 - folded), mode > 1.5);
        sum = sum + amplitude * shaped;
        weight = weight + amplitude;
        amplitude = amplitude * clamp(roughness, 0.0, 1.0);
        at = at * lacunarity;
    }
    return sum / max(weight, 0.00001);
}

// The coordinate pushed around by the noise itself before it is read: smoke, marble, flame.
fn graph_simplex_warp(p: vec2<f32>, amount: f32) -> vec2<f32> {
    let push = vec2<f32>(graph_simplex_noise(p + vec2<f32>(5.2, 1.3)), graph_simplex_noise(p + vec2<f32>(1.7, 9.2)));
    return p + (push * 2.0 - 1.0) * amount;
}
