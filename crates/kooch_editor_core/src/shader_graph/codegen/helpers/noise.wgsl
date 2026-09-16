fn graph_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.547);
}

fn graph_noise(uv: vec2<f32>) -> f32 {
    let cell = floor(uv);
    let f = fract(uv);
    let a = graph_hash(cell);
    let b = graph_hash(cell + vec2<f32>(1.0, 0.0));
    let c = graph_hash(cell + vec2<f32>(0.0, 1.0));
    let d = graph_hash(cell + vec2<f32>(1.0, 1.0));
    let w = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}
