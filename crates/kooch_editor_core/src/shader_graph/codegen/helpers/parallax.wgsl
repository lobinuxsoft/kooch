// A layer at `focus` stays put: nearer ones slide with `view`, farther ones against it.
fn graph_parallax(input: UiInput, start: vec2<f32>, view: vec2<f32>, focus: f32) -> vec2<f32> {
    let stride = 1.0 / 32.0;
    var layer = 1.0;
    var uv = start + view * (1.0 - focus);
    var last = uv;
    for (var i = 0; i < 32; i++) {
        if graph_height(input, uv) >= layer {
            break;
        }
        last = uv;
        uv -= view * stride;
        layer -= stride;
    }
    // Between the last two samples, or the layers show as steps.
    let below = graph_height(input, uv) - layer;
    let above = graph_height(input, last) - (layer + stride);
    return mix(uv, last, clamp(below / max(below - above, 0.0001), 0.0, 1.0));
}
