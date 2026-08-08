use super::*;

#[test]
fn meshlet_shader_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(MESHLET_SHADER_SOURCE)
        .expect("meshlet_main.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("meshlet_main.wgsl should validate");
}

#[test]
fn camera_ubo_size_is_64_bytes() {
    // Mirror of the shader's CameraUniforms; the bind-group layout
    // declares `min_binding_size = 64`, so a drift here would fail
    // pipeline creation at runtime instead of compile time.
    assert_eq!(std::mem::size_of::<CameraUbo>(), 64);
}

#[test]
fn model_ubo_size_is_64_bytes() {
    assert_eq!(std::mem::size_of::<ModelUbo>(), 64);
}
