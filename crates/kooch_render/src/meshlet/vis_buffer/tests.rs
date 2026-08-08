use super::*;

#[test]
fn vbuf_shader_parses_and_validates() {
    let module =
        naga::front::wgsl::parse_str(SHADER_SOURCE).expect("meshlet_vbuf.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("meshlet_vbuf.wgsl should validate");
}

#[test]
fn vbuf_format_is_r32uint() {
    assert_eq!(VISIBILITY_BUFFER_FORMAT, wgpu::TextureFormat::R32Uint);
}
