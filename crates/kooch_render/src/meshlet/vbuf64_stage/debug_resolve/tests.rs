use super::*;

#[test]
fn debug_resolve_shader_validates() {
    let src = include_str!("../../../../shaders/meshlet_debug_resolve.wgsl");
    let module = naga::front::wgsl::parse_str(src)
        .unwrap_or_else(|e| panic!("meshlet_debug_resolve.wgsl should parse: {e:?}"));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("meshlet_debug_resolve.wgsl should validate: {e:?}"));
}

#[test]
fn colorize_modes_are_the_replace_shading_set() {
    // MeshletIds/InstanceIds/TriangleDensity/Overdraw/CullPassthrough.
    for m in [1u32, 2, 3, 4, 7] {
        assert!(is_colorize_mode(m), "mode {m} should colorize");
    }
    // Off + normal-look modes shade through the two-pass path.
    for m in [0u32, 5, 6, 8, 9, 10] {
        assert!(!is_colorize_mode(m), "mode {m} keeps the normal look");
    }
}
