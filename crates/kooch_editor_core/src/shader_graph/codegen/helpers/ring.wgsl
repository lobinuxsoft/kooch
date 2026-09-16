fn graph_ring(uv: vec2<f32>, radius: f32, thickness: f32) -> f32 {
    let d = abs(length(uv - vec2<f32>(0.5)) - radius);
    let t = max(thickness, 0.00001) * 0.5;
    return 1.0 - smoothstep(t * 0.8, t, d);
}
