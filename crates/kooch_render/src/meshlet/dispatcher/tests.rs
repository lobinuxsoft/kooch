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
        include_str!("../../../shaders/meshlet_cull/two_level.wgsl"),
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

/// The number in #1002, on the scene it was measured on.
///
/// `dense.scene` is 2026 instances against a heaviest mesh of 4755
/// meshlets. The one-level cull dispatched that rectangle — 9 633 630
/// threads for ~116 000 real meshlets. The chunk list is the same
/// rectangle over the workgroup, and it sizes a BUFFER rather than a
/// dispatch: 608 KB instead of nine million lanes.
#[test]
fn the_dense_scene_fits_in_chunks() {
    let rectangle = 2026u32 * 4755;
    assert_eq!(rectangle, 9_633_630, "the number the issue reports");

    let chunks = chunks_for(2026, 4755);
    assert_eq!(chunks, 2026 * 75);
    assert!(
        chunks < rectangle / 60,
        "{chunks} chunks against {rectangle} threads",
    );
}

/// A one-meshlet mesh gets one chunk, not a fraction and not zero.
#[test]
fn a_single_meshlet_takes_one_chunk() {
    assert_eq!(chunks_for(2000, 1), 2000);
    // An empty pool still has to produce a legal dispatch size.
    assert_eq!(chunks_for(0, 0), 1);
}

/// 🔴 A chunk is a workgroup, and the two constants saying so live in
/// two languages.
///
/// `CULL_CHUNK_MESHLETS` sizes the buffer on the CPU and `CULL_GROUP`
/// slices the meshlets on the GPU. If they drift, the list is too
/// small and the instance pass silently drops the tail of the scene —
/// which reads as geometry that vanishes at a certain instance count
/// and nothing else.
#[test]
fn the_chunk_constants_agree() {
    const WGSL: &str = include_str!("../../../shaders/meshlet_cull/two_level.wgsl");

    let named = |name: &str| -> u64 {
        let at = WGSL
            .find(&format!("const {name}: u32 = "))
            .unwrap_or_else(|| panic!("{name} is not declared in two_level.wgsl"));
        WGSL[at..]
            .split_once("= ")
            .and_then(|(_, rest)| rest.split_once('u'))
            .and_then(|(digits, _)| digits.trim().parse().ok())
            .unwrap_or_else(|| panic!("{name} is not a plain literal"))
    };

    assert_eq!(named("CULL_GROUP"), CULL_CHUNK_MESHLETS as u64);
    assert_eq!(named("CHUNK_LIST"), CHUNK_HEADER_WORDS);
    assert_eq!(named("CHUNK_ARGS") * 4, CHUNK_ARGS_OFFSET);
    assert_eq!(named("MAX_GROUPS_PER_DIM"), 65_535);
}

/// The chunk word packs an instance and a chunk index into 32 bits,
/// and the low field has to hold the heaviest mesh the tree ships.
///
/// 256 chunks is 16 384 meshlets in ONE mesh; `dense.scene`'s dragon
/// is 4755. The assertion is that the headroom is real, not that the
/// dragon fits by luck.
#[test]
fn the_chunk_index_holds_a_heavy_mesh() {
    const WGSL: &str = include_str!("../../../shaders/meshlet_cull/two_level.wgsl");
    assert!(WGSL.contains("const MAX_CHUNKS_PER_INSTANCE: u32 = 256u;"));
    assert!(WGSL.contains("const CHUNK_INDEX_MASK: u32 = 255u;"));
    assert!(
        256 * CULL_CHUNK_MESHLETS >= 4755 * 3,
        "the cap must clear the heaviest asset with room over",
    );
}

/// The two GPU structs are 16 and 208 bytes, and the fields #1002
/// added went into padding rather than past it.
///
/// A struct that grew is not a compile error — wgpu reports it as
/// `min_binding_size`, which reads like a binding problem and not like
/// a layout one.
#[test]
fn the_param_structs_did_not_grow() {
    use crate::meshlet::cull::CullParams;
    use crate::meshlet::scene::SceneCullParams;

    assert_eq!(std::mem::size_of::<SceneCullParams>(), 16);
    assert_eq!(std::mem::size_of::<CullParams>(), 208);
}

/// The reach is off unless somebody authored it, and it never goes
/// negative — a negative threshold would reject nothing while reading
/// as if it rejected everything.
#[test]
fn the_reach_defaults_to_off() {
    use crate::meshlet::cull::CullParams;
    use glam::{Mat4, Vec3};

    let params = CullParams::new(Mat4::IDENTITY, Vec3::ZERO, 1);
    assert_eq!(params.min_screen_pixels, 0.0);
    assert_eq!(
        params.with_min_screen_pixels(-4.0).min_screen_pixels,
        0.0,
        "a negative reach is clamped, not honoured",
    );
    assert_eq!(
        CullParams::new(Mat4::IDENTITY, Vec3::ZERO, 1)
            .with_min_screen_pixels(8.0)
            .min_screen_pixels,
        8.0,
    );
}
