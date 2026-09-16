fn graph_box(uv: vec2<f32>, size: vec2<f32>, softness: f32) -> f32 {
    let d = abs(uv - vec2<f32>(0.5)) - size * 0.5;
    let s = max(softness, 0.00001);
    let m = vec2<f32>(1.0) - smoothstep(vec2<f32>(-s), vec2<f32>(s), d);
    return m.x * m.y;
}
