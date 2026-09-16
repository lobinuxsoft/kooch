// Gradient (Perlin) noise: a random slope at each lattice point, blended with a quintic fade. 0..1.
fn graph_gradient_noise(uv: vec2<f32>) -> f32 {
    let cell = floor(uv);
    let f = fract(uv);
    let fade = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(graph_gradient(cell), f);
    let b = dot(graph_gradient(cell + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(graph_gradient(cell + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(graph_gradient(cell + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, fade.x), mix(c, d, fade.x), fade.y) * 0.5 + 0.5;
}

fn graph_gradient_fbm(uv: vec2<f32>, octaves: f32) -> f32 {
    let count = i32(clamp(round(octaves), 1.0, 8.0));
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 0.5;
    var p = uv;
    for (var i = 0; i < count; i = i + 1) {
        sum = sum + amplitude * graph_gradient_noise(p);
        weight = weight + amplitude;
        amplitude = amplitude * 0.5;
        p = p * 2.0;
    }
    return sum / weight;
}
