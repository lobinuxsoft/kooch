//! Test code for `post_process`, in its own file.

use crate::material::Shader;
use crate::meshlet::{MATERIAL_POST_FRAME, compose_post_shader, validate_post};

const VIGNETTE: &str = "// kind: post_process
fn post_process(input: SurfaceInput) -> vec4<f32> {
    return sample_scene(input.uv);
}";

/// The frame the pass builds its pipeline from has the entry points the pipeline asks for by name.
#[test]
fn the_post_frame_has_its_entries() {
    assert!(MATERIAL_POST_FRAME.contains("fn vs_fullscreen("));
    assert!(MATERIAL_POST_FRAME.contains("fn fs_post("));
}

/// 🔴 The bindings the pass sets must be the ones the composed shader declares, or wgpu rejects the
/// pipeline at build time. Group 0 is the frame's, 2 the material storage, 4 its textures.
#[test]
fn the_groups_are_where_the_pass_binds_them() {
    let shader = Shader::parse(VIGNETTE).unwrap();
    let composed = compose_post_shader(&shader.params_wgsl(), &shader.source);
    for declaration in [
        "@group(0) @binding(0) var scene_color",
        "@group(2) @binding(0) var<storage, read> materials",
        "@group(2) @binding(1) var<storage, read> material_values",
        "@group(4) @binding(3) var material_sampler",
    ] {
        assert!(composed.contains(declaration), "missing {declaration}");
    }
    validate_post(&shader.params_wgsl(), &shader.source).unwrap();
}
