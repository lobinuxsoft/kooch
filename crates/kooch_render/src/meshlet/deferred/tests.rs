use super::*;

#[test]
fn deferred_shader_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(&shader_source())
        .expect("composed meshlet_deferred shader should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("composed meshlet_deferred shader should validate");
}

#[test]
fn screen_ubo_layout() {
    assert_eq!(std::mem::size_of::<ScreenUbo>(), 16);
}
