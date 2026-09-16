fn graph_overlay(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let low = 2.0 * a * b;
    let high = vec4<f32>(1.0) - 2.0 * (vec4<f32>(1.0) - a) * (vec4<f32>(1.0) - b);
    return select(low, high, a > vec4<f32>(0.5));
}
