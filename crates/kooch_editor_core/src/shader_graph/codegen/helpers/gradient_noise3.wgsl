// Gradient noise in three dimensions, the third being the phase.
fn graph_gradient_noise3(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let fade = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(graph_gradient3(cell), f);
    let b = dot(graph_gradient3(cell + vec3<f32>(1.0, 0.0, 0.0)), f - vec3<f32>(1.0, 0.0, 0.0));
    let c = dot(graph_gradient3(cell + vec3<f32>(0.0, 1.0, 0.0)), f - vec3<f32>(0.0, 1.0, 0.0));
    let d = dot(graph_gradient3(cell + vec3<f32>(1.0, 1.0, 0.0)), f - vec3<f32>(1.0, 1.0, 0.0));
    let e = dot(graph_gradient3(cell + vec3<f32>(0.0, 0.0, 1.0)), f - vec3<f32>(0.0, 0.0, 1.0));
    let g = dot(graph_gradient3(cell + vec3<f32>(1.0, 0.0, 1.0)), f - vec3<f32>(1.0, 0.0, 1.0));
    let h = dot(graph_gradient3(cell + vec3<f32>(0.0, 1.0, 1.0)), f - vec3<f32>(0.0, 1.0, 1.0));
    let k = dot(graph_gradient3(cell + vec3<f32>(1.0, 1.0, 1.0)), f - vec3<f32>(1.0, 1.0, 1.0));
    let near = mix(mix(a, b, fade.x), mix(c, d, fade.x), fade.y);
    let far = mix(mix(e, g, fade.x), mix(h, k, fade.x), fade.y);
    return mix(near, far, fade.z) * 0.5 + 0.5;
}

// Octaves of gradient noise. `mode` 0 sums them (fBm), 1 folds each about its middle (turbulence), 2 turns
// the fold into sharp crests (ridged). `roughness` is how much each octave keeps of the one before,
// `lacunarity` how much finer it is. Normalised back to 0..1.
fn graph_gradient_fractal3(p: vec3<f32>, octaves: f32, roughness: f32, lacunarity: f32, mode: f32) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 1.0;
    var at = p;
    for (var i = 0; i < count; i = i + 1) {
        let n = graph_gradient_noise3(at);
        let folded = abs(n * 2.0 - 1.0);
        let shaped = select(select(n, folded, mode > 0.5), (1.0 - folded) * (1.0 - folded), mode > 1.5);
        sum = sum + amplitude * shaped;
        weight = weight + amplitude;
        amplitude = amplitude * clamp(roughness, 0.0, 1.0);
        at = at * lacunarity;
    }
    return sum / max(weight, 0.00001);
}
