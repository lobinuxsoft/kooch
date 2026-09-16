// Value noise: a random value at each lattice point, smoothed between them. 0..1.
fn graph_value_noise(uv: vec2<f32>) -> f32 {
    let cell = floor(uv);
    let f = fract(uv);
    let w = f * f * (3.0 - 2.0 * f);
    let a = graph_hash(cell);
    let b = graph_hash(cell + vec2<f32>(1.0, 0.0));
    let c = graph_hash(cell + vec2<f32>(0.0, 1.0));
    let d = graph_hash(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

// Octaves of value noise. `mode` 0 sums them (fBm), 1 folds each about its middle (turbulence), 2 turns
// the fold into sharp crests (ridged). `roughness` is how much each octave keeps of the one before,
// `lacunarity` how much finer it is. Normalised back to 0..1.
fn graph_value_fractal(p: vec2<f32>, octaves: f32, roughness: f32, lacunarity: f32, mode: f32) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 1.0;
    var at = p;
    for (var i = 0; i < count; i = i + 1) {
        let n = graph_value_noise(at);
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
fn graph_value_warp(p: vec2<f32>, amount: f32) -> vec2<f32> {
    let push = vec2<f32>(graph_value_noise(p + vec2<f32>(5.2, 1.3)), graph_value_noise(p + vec2<f32>(1.7, 9.2)));
    return p + (push * 2.0 - 1.0) * amount;
}
