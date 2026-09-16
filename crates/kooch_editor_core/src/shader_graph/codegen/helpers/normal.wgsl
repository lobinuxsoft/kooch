fn graph_normalize(v: vec3<f32>) -> vec3<f32> {
    return select(normalize(v), vec3<f32>(0.0, 0.0, 1.0), length(v) < 0.000001);
}
