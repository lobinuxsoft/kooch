struct GraphUiPoint {
    point: vec2<f32>,
    aspect: vec2<f32>,
    pixel: f32,
}

fn graph_ui_point(uv: vec2<f32>, size: vec2<f32>) -> GraphUiPoint {
    let height = max(size.y, 1.0);
    let aspect = vec2<f32>(size.x / height, 1.0);
    return GraphUiPoint((uv - vec2<f32>(0.5)) * aspect, aspect, 1.0 / height);
}
