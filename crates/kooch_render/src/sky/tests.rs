use super::*;

#[test]
fn shader_parses() {
    let module = naga::front::wgsl::parse_str(SHADER_SOURCE).expect("sky shader should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("sky shader should validate");
}
