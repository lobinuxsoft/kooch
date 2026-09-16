fn graph_polygon(uv: vec2<f32>, sides: f32, radius: f32) -> f32 {
    let p = uv - vec2<f32>(0.5);
    let n = max(floor(sides + 0.5), 3.0);
    let segment = 6.2831855 / n;
    let a = atan2(p.y, p.x + 0.000000001);
    let d = length(p) * cos(a - segment * floor(a / segment + 0.5)) / cos(segment * 0.5);
    return 1.0 - smoothstep(radius - 0.005, radius + 0.005, d);
}
