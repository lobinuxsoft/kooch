use super::{SHADER_SOURCE, SPD_SHADER_SOURCE};

#[test]
fn shader_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(SHADER_SOURCE).expect("hi_z_build.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("hi_z_build.wgsl should validate");
}

#[test]
fn spd_shader_parses_and_validates() {
    let module =
        naga::front::wgsl::parse_str(SPD_SHADER_SOURCE).expect("hi_z_spd.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("hi_z_spd.wgsl should validate");
}
