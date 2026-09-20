// 🔴 `period` is taken for one signature across the bases and ignored: the lattice is skewed, so
// wrapping a cell repeats in skewed space and not in uv. The graph refuses to tile a simplex.
// Simplex noise in 3D: space cut into tetrahedra, four lattice points read instead of eight.
fn graph_simplex_noise3(p: vec3<f32>, period: vec2<f32>) -> f32 {
    let skew = 1.0 / 3.0;
    let unskew = 1.0 / 6.0;
    let cell = floor(p + dot(p, vec3<f32>(skew)));
    let x0 = p - cell + dot(cell, vec3<f32>(unskew));
    let e = step(vec3<f32>(0.0), x0 - x0.yzx);
    let i1 = e * (1.0 - e.zxy);
    let i2 = 1.0 - e.zxy * (1.0 - e);
    let x1 = x0 - i1 + unskew;
    let x2 = x0 - i2 + 2.0 * unskew;
    let x3 = x0 - 0.5;
    let w = max(vec4<f32>(0.6) - vec4<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), vec4<f32>(0.0));
    let d = vec4<f32>(
        dot(x0, graph_gradient3(cell)),
        dot(x1, graph_gradient3(cell + i1)),
        dot(x2, graph_gradient3(cell + i2)),
        dot(x3, graph_gradient3(cell + 1.0)),
    );
    let w2 = w * w;
    return dot(w2 * w2 * d, vec4<f32>(52.0)) * 0.5 + 0.5;
}

// Octaves of simplex noise. `mode` 0 sums them (fBm), 1 folds each about its middle (turbulence), 2 turns
// the fold into sharp crests (ridged). `roughness` is how much each octave keeps of the one before,
// `lacunarity` how much finer it is. Normalised back to 0..1.
fn graph_simplex_fractal3(p: vec3<f32>, octaves: f32, roughness: f32, lacunarity: f32, mode: f32, period: vec2<f32>) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 1.0;
    var at = p;
    for (var i = 0; i < count; i = i + 1) {
        let n = graph_simplex_noise3(at, period);
        let folded = abs(n * 2.0 - 1.0);
        let shaped = select(select(n, folded, mode > 0.5), (1.0 - folded) * (1.0 - folded), mode > 1.5);
        sum = sum + amplitude * shaped;
        weight = weight + amplitude;
        amplitude = amplitude * clamp(roughness, 0.0, 1.0);
        at = at * lacunarity;
    }
    return sum / max(weight, 0.00001);
}
