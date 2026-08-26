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

/// 🔴 The ring has to cover the worst frame, and the worst frame is
/// every view's point-shadow cubes in one encoder.
///
/// This exists because the original sizing was off by the number of
/// views: 64 slots against 32 lamps read as "a factor of two" and was
/// exactly break-even once the editor's second viewport was counted.
/// The result was #853's symptom again — lamps culled with each other's
/// frusta, several shadows appearing as copies of one — and nothing
/// reported it, because a ring laps in silence.
///
/// It stayed hidden for as long as the shipped budget was 6. Twelve
/// dispatches into sixty-four cannot collide, so "it works" had been
/// measured on the one case that could not fail. Raising
/// `MAX_POINT_SHADOWS`, adding a viewport, or dispatching a cull object
/// once more per light all break this, and each of them would otherwise
/// be found by eye, weeks later, as "the shadows are wrong again".
#[test]
fn the_ring_covers_the_worst_case() {
    // 🔴 An OBSERVED number, not `VIEWS_ASSUMED`. Deriving the bound
    // from the same constant the ring is derived from makes both sides
    // move together and the assertion can never fail — which is what the
    // first version of this test did.
    //
    // Two is what the editor renders through one stage today: a Scene
    // panel and a Game panel. Anything that adds a third viewport has to
    // raise `VIEWS_ASSUMED`, and this is what says so.
    const EDITOR_VIEWS: u64 = 2;
    let per_frame = kooch_lighting::MAX_POINT_SHADOWS as u64 * EDITOR_VIEWS;
    assert!(
        super::PARAMS_RING >= per_frame * 2,
        "the params ring has {} slots and one encoder can dispatch a single cull object \
         {} times ({} lamps x {} views). It needs at least double that, because the \
         cursor is never rewound at the start of a frame and a frame beginning mid-ring \
         wraps onto its own earlier slots.",
        super::PARAMS_RING,
        per_frame,
        kooch_lighting::MAX_POINT_SHADOWS,
        EDITOR_VIEWS,
    );
}
