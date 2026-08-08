use super::*;

#[test]
fn draw_indirect_args_layout_is_pod() {
    // Must match wgpu::DrawIndirectArgs exactly so we can write
    // straight into an INDIRECT-usage buffer.
    assert_eq!(std::mem::size_of::<DrawIndirectArgs>(), 16);
}

#[test]
fn draw_indirect_args_default_is_zero() {
    let args = DrawIndirectArgs::default();
    assert_eq!(args.vertex_count, 0);
    assert_eq!(args.instance_count, 0);
    assert_eq!(args.first_vertex, 0);
    assert_eq!(args.first_instance, 0);
}

#[test]
fn hi_z_test_params_layout() {
    // 64-byte mat4 + 8-byte vec2 + 4-byte u32 + 4-byte pad = 80 B.
    assert_eq!(std::mem::size_of::<HiZTestParams>(), 80);
}

#[test]
fn cull_shader_parses_and_validates() {
    const CULL_SHADER_SOURCE: &str = concat!(
        include_str!("../../../shaders/meshlet_cull/common.wgsl"),
        include_str!("../../../shaders/meshlet_cull/basic.wgsl"),
        include_str!("../../../shaders/meshlet_cull/scene.wgsl"),
        include_str!("../../../shaders/meshlet_cull/pool.wgsl"),
        include_str!("../../../shaders/meshlet_cull/atomic.wgsl"),
        include_str!("../../../shaders/meshlet_cull/atomic_hi_z.wgsl"),
    );
    let module =
        naga::front::wgsl::parse_str(CULL_SHADER_SOURCE).expect("meshlet_cull.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("meshlet_cull.wgsl should validate");
}
