// Three texels apart: noise read one texel apart is grain, not shape.
fn graph_image_normal(input: UiInput, uv: vec2<f32>, strength: f32) -> vec3<f32> {
    let gap = 1.5 / max(input.atlas_size, vec2<f32>(1.0));
    let dx = graph_height(input, uv + vec2<f32>(gap.x, 0.0)) - graph_height(input, uv - vec2<f32>(gap.x, 0.0));
    let dy = graph_height(input, uv + vec2<f32>(0.0, gap.y)) - graph_height(input, uv - vec2<f32>(0.0, gap.y));
    return normalize(vec3<f32>(-dx * strength, -dy * strength, 1.0));
}
