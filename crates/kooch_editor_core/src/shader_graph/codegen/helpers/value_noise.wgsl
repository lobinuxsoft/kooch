// Value noise: a random value at each lattice point, smoothed between them. 0..1.
fn graph_value_noise(uv: vec2<f32>) -> f32 {
    let cell = floor(uv);
    let f = fract(uv);
    let a = graph_hash(cell);
    let b = graph_hash(cell + vec2<f32>(1.0, 0.0));
    let c = graph_hash(cell + vec2<f32>(0.0, 1.0));
    let d = graph_hash(cell + vec2<f32>(1.0, 1.0));
    let w = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

// Octaves of it, each at twice the frequency and half the weight, normalised back to 0..1.
fn graph_value_fbm(uv: vec2<f32>, octaves: f32) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 0.5;
    var p = uv;
    for (var i = 0; i < count; i = i + 1) {
        sum = sum + amplitude * graph_value_noise(p);
        weight = weight + amplitude;
        amplitude = amplitude * 0.5;
        p = p * 2.0;
    }
    return sum / weight;
}
