// ui_material.wgsl — what a `kind: ui` shader sees (#1690), composed after ui.wgsl and ahead of the
// shader. `ui` returns straight alpha; the frame cuts the rounded corners and premultiplies.

@group(0) @binding(3) var<storage, read> material_values: array<f32>;

struct UiInput {
    // 0..1 across the node.
    uv: vec2<f32>,
    // The node, in physical pixels.
    size: vec2<f32>,
    // `background-color`, `border-color`.
    color: vec4<f32>,
    border_color: vec4<f32>,
    // Seconds.
    time: f32,
    // What `surface_params` takes.
    material_id: u32,
    image: bool,
    // Drawn by Vello rather than packed as texels (#1865).
    vector: bool,
    atlas_min: vec2<f32>,
    atlas_size: vec2<f32>,
}

// The node's `background-image` at `uv`; white when it has none.
fn sample_image(input: UiInput, uv: vec2<f32>) -> vec4<f32> {
    if !input.image {
        return vec4<f32>(1.0);
    }
    return ui_sample(input.vector, input.atlas_min + uv * input.atlas_size);
}
