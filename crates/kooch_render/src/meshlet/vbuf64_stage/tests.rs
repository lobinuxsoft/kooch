const RASTER_SOURCE: &str = include_str!("../../../shaders/meshlet_vbuf64.wgsl");
const CLEAR_SOURCE: &str = include_str!("../../../shaders/meshlet_clear_vbuf64.wgsl");

fn validate(source: &str, label: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{label} should parse: {e:?}"));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label} should validate: {e:?}"));
}

#[test]
fn vbuf64_raster_shader_validates() {
    validate(RASTER_SOURCE, "meshlet_vbuf64.wgsl");
}

#[test]
fn vbuf64_clear_shader_validates() {
    validate(CLEAR_SOURCE, "meshlet_clear_vbuf64.wgsl");
}

#[test]
fn vbuf64_format_is_r64uint() {
    assert_eq!(super::VBUF64_FORMAT, wgpu::TextureFormat::R64Uint);
}
