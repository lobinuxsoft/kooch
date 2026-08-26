use super::*;

#[test]
fn blit_shader_parses_and_validates() {
    let module =
        naga::front::wgsl::parse_str(SHADER_SOURCE).expect("meshlet_blit.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("meshlet_blit.wgsl should validate");
}
