fn graph_height(input: UiInput, uv: vec2<f32>) -> f32 {
    let texel = sample_image(input, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    return dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
}
