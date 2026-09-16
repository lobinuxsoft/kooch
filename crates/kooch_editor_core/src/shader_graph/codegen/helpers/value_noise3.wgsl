// Value noise in three dimensions: the third is the phase, so the pattern changes where it stands.
fn graph_value_noise3(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = mix(graph_hash3(cell), graph_hash3(cell + vec3<f32>(1.0, 0.0, 0.0)), w.x);
    let b = mix(graph_hash3(cell + vec3<f32>(0.0, 1.0, 0.0)), graph_hash3(cell + vec3<f32>(1.0, 1.0, 0.0)), w.x);
    let c = mix(graph_hash3(cell + vec3<f32>(0.0, 0.0, 1.0)), graph_hash3(cell + vec3<f32>(1.0, 0.0, 1.0)), w.x);
    let d = mix(graph_hash3(cell + vec3<f32>(0.0, 1.0, 1.0)), graph_hash3(cell + vec3<f32>(1.0, 1.0, 1.0)), w.x);
    return mix(mix(a, b, w.y), mix(c, d, w.y), w.z);
}

// Octaves of value noise. `mode` 0 sums them (fBm), 1 folds each about its middle (turbulence), 2 turns
// the fold into sharp crests (ridged). `roughness` is how much each octave keeps of the one before,
// `lacunarity` how much finer it is. Normalised back to 0..1.
fn graph_value_fractal3(p: vec3<f32>, octaves: f32, roughness: f32, lacunarity: f32, mode: f32) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 1.0;
    var at = p;
    for (var i = 0; i < count; i = i + 1) {
        let n = graph_value_noise3(at);
        let folded = abs(n * 2.0 - 1.0);
        let shaped = select(select(n, folded, mode > 0.5), (1.0 - folded) * (1.0 - folded), mode > 1.5);
        sum = sum + amplitude * shaped;
        weight = weight + amplitude;
        amplitude = amplitude * clamp(roughness, 0.0, 1.0);
        at = at * lacunarity;
    }
    return sum / max(weight, 0.00001);
}
