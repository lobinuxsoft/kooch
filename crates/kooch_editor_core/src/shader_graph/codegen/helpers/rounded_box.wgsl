fn graph_rounded_box(point: vec2<f32>, extent: vec2<f32>, radius: f32) -> f32 {
    let r = min(max(radius, 0.0), min(extent.x, extent.y));
    let q = abs(point) - extent + r;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
