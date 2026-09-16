fn graph_rotate(uv: vec2<f32>, centre: vec2<f32>, angle: f32) -> vec2<f32> {
    let d = uv - centre;
    let s = sin(angle);
    let c = cos(angle);
    return centre + vec2<f32>(d.x * c - d.y * s, d.x * s + d.y * c);
}
