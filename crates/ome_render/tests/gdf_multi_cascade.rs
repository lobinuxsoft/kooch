//! AC of PR-5 (epic #370) — multi-cascade GDF: six cascades with
//! voxel pitch 0.25 m → 8 km, round-robin update, cone-radius cascade
//! selection. Four tests pin the contract:
//!
//! 1. `cascade_selection_picks_finest_for_close_rays` — programmatic
//!    `pick_cascade_cpu` calls at `t = 1 m`, `100 m`, and `10 km`
//!    along a ray. Asserts the cone-matched cascade index escalates
//!    with `t`.
//! 2. `round_robin_schedule_holds_60_frames` — drive the scheduler
//!    for 60 frames with a stationary camera; assert the dispatch
//!    counts match the steady-state 1, 1/2, 1/4, 1/8, 1/16, 1/32
//!    cadence (with first-frame stagger).
//! 3. `dirty_chunk_forces_off_schedule_dispatch` — mark a chunk dirty
//!    on a frame where cascade 3 isn't due; assert the dispatch
//!    fires anyway.
//! 4. `camera_drift_re_snaps_far_cascade` — move the camera past
//!    cascade 1's drift threshold (8 m) on an off-schedule frame;
//!    assert cascade 1 re-snaps.
//!
//! The scheduler tests have unit-level mirrors in
//! `gdf::scheduler::tests` — these integration tests pin the same
//! behaviour via the public API as a contract guard against future
//! refactors that move the scheduler to a different ownership model.

mod common;
use common::try_acquire_device;

use glam::Vec3;
use ome_render::gdf::{
    CASCADE_COUNT, CASCADE_VOXEL_SIZES, CASCADE_VOXELS_PER_AXIS, GdfScheduler, GdfUniforms,
    cascade_cube_extent, pick_cascade_cpu,
};

/// Build a `GdfUniforms` whose every cascade is centred on the
/// origin (snapped origin = `-cube_extent/2` so each cube is centred
/// on `(0, 0, 0)`). Mirrors `GdfState::dispatch_populate_cascade`
/// without touching the GPU.
fn cascades_centred_on_origin() -> GdfUniforms {
    let origins: [Vec3; CASCADE_COUNT] = std::array::from_fn(|c| {
        let cube_extent = cascade_cube_extent(c);
        Vec3::splat(-cube_extent * 0.5)
    });
    GdfUniforms::from_origins(&origins)
}

#[test]
fn cascade_selection_picks_finest_for_close_rays() {
    let uniforms = cascades_centred_on_origin();
    // Pixel cone half-angle for a 720p 60° vertical FOV: ~1.6 mrad.
    // Cone radius at distance `t` = `t * 1.6e-3`.
    let pixel_cone_angle: f32 = (60.0_f32.to_radians() * 0.5).tan() * 2.0 / 720.0;

    // Close ray (t = 1 m): cone footprint ~1.6 mm. Cascade 0 voxel
    // pitch is 0.25 m — first cascade with `voxel_size >= 1.6e-3 m`
    // is cascade 0.
    let close = Vec3::new(0.0, 0.0, 1.0);
    let cone_close = close.length() * pixel_cone_angle;
    let pick_close = pick_cascade_cpu(close, cone_close, &uniforms);
    assert_eq!(pick_close, Some(0), "close ray (t=1m) should pick cascade 0");

    // Mid ray (t = 100 m): cone footprint ~16 cm. Cascade 0 (pitch
    // 0.25 m) still qualifies on `voxel_size >= cone`, BUT t=100m
    // is outside cascade 0's 16 m cube — so the walk advances. The
    // first cascade containing 100 m AND with adequate pitch is
    // cascade 2 (1 km cube, 16 m pitch).
    let mid = Vec3::new(0.0, 0.0, 100.0);
    let cone_mid = mid.length() * pixel_cone_angle;
    let pick_mid = pick_cascade_cpu(mid, cone_mid, &uniforms);
    assert_eq!(
        pick_mid,
        Some(2),
        "mid ray (t=100m) should pick cascade 2 (1 km cube, 16 m pitch); cone={cone_mid:.4}"
    );

    // Far ray (t = 10 000 m): cone footprint 16 m. Cascade 4 (1024 m
    // pitch, 64 km cube) is the first that qualifies on both.
    let far = Vec3::new(0.0, 0.0, 10_000.0);
    let cone_far = far.length() * pixel_cone_angle;
    let pick_far = pick_cascade_cpu(far, cone_far, &uniforms);
    assert_eq!(
        pick_far,
        Some(4),
        "far ray (t=10km) should pick cascade 4 (64 km cube, 1024 m pitch); cone={cone_far:.4}"
    );

    // Beyond cascade 5 (524 km): no cascade qualifies → None
    // (sentinel for shader fallback to AABB-distance floor).
    let beyond = Vec3::new(0.0, 0.0, 1_000_000.0);
    let cone_beyond = beyond.length() * pixel_cone_angle;
    let pick_beyond = pick_cascade_cpu(beyond, cone_beyond, &uniforms);
    assert_eq!(pick_beyond, None, "beyond cascade 5 should fall through");
}

#[test]
fn round_robin_schedule_holds_60_frames() {
    let mut sched = GdfScheduler::new();
    let mut counts = [0u32; CASCADE_COUNT];
    for _ in 0..60 {
        for c in sched.cascades_to_update(Vec3::ZERO) {
            counts[c as usize] += 1;
        }
    }
    // Bootstrap @ frame `c`, schedule every `2^c` thereafter:
    // c=0: 60. c=1: 1+29=30. c=2: 1+14=15. c=3: 1+7=8.
    // c=4: 1+3=4. c=5: 1+1=2.
    assert_eq!(counts, [60, 30, 15, 8, 4, 2], "counts: {counts:?}");
}

#[test]
fn dirty_chunk_forces_off_schedule_dispatch() {
    let mut sched = GdfScheduler::new();
    // Bootstrap all cascades.
    for _ in 0..32 {
        sched.cascades_to_update(Vec3::ZERO);
    }
    sched.cascades_to_update(Vec3::ZERO); // frame 33
    sched.mark_chunk_dirty(7);
    let dispatched = sched.cascades_to_update(Vec3::ZERO); // frame 34
    // Cascade 3's schedule is `% 8 == 0`. Frame 34 % 8 == 2 → not
    // due, but the dirty mark forces a dispatch.
    assert!(
        dispatched.contains(&3),
        "dirty chunk did not force cascade 3 off-schedule dispatch: {dispatched:?}"
    );
}

#[test]
fn camera_drift_re_snaps_far_cascade() {
    let mut sched = GdfScheduler::new();
    // Bootstrap all six.
    for _ in 0..6 {
        sched.cascades_to_update(Vec3::ZERO);
    }
    sched.cascades_to_update(Vec3::ZERO); // frame 6 (cascade 1 was due here)
    sched.cascades_to_update(Vec3::ZERO); // frame 7
    sched.cascades_to_update(Vec3::ZERO); // frame 8 (cascade 1 due, c2 due, c3 due)
    // Frame 9 — cascade 1 schedule % 2 != 0. Move camera 100 m =
    // past cascade 1's drift threshold (4 voxels × 2 m = 8 m).
    let dispatched = sched.cascades_to_update(Vec3::new(100.0, 0.0, 0.0));
    assert!(
        dispatched.contains(&1),
        "100 m camera drift past cascade-1 8 m threshold did not force re-snap: \
         {dispatched:?}"
    );
}

#[test]
fn cascade_voxel_table_consistency() {
    // PR-5 cascade pitch table: 0.25 m → 8 km at ×8 ratio.
    // Cube extent = pitch × 64. Pin the numbers so any future
    // table tweak surfaces here.
    assert_eq!(CASCADE_COUNT, 6);
    assert_eq!(CASCADE_VOXELS_PER_AXIS, 64);
    assert!((CASCADE_VOXEL_SIZES[0] - 0.25).abs() < 1.0e-6);
    assert!((CASCADE_VOXEL_SIZES[5] - 8192.0).abs() < 1.0e-3);
    assert!((cascade_cube_extent(0) - 16.0).abs() < 1.0e-3);
    assert!((cascade_cube_extent(5) - 524_288.0).abs() < 1.0);
}

#[test]
fn multi_cascade_render_does_not_crash() {
    // Smoke-only end-to-end: builds a renderer (which constructs
    // `GdfState` with six 64³ R32Float textures), inserts a single
    // sphere chunk at the origin, drives one frame through the
    // round-robin scheduler. Pin: no validation error, no panic on
    // the bind group with 15 entries.
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("multi_cascade_render: no GPU adapter — skipping");
        return;
    };
    let mut renderer = ome_render::raymarch::RayMarchRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8Unorm,
        None,
    );

    use ome_bvh::sdf_primitive::{SdfPrimitive, TYPE_SPHERE};
    use ome_bvh::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};
    use ome_world::{ChunkContent, ChunkId};
    let chunk = ChunkId::new(glam::IVec3::ZERO, 0);
    let content = ChunkContent {
        primitives: vec![SdfPrimitive {
            position: [0.0, 0.0, 0.0],
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: 0.0,
            params: [1.0, 0.0, 0.0, 0.0],
        }],
        leaf_aabbs: vec![LeafAabb {
            aabb_min: [-1.0, -1.0, -1.0],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [1.0, 1.0, 1.0],
            entity_id: 0,
        }],
        max_smoothness_radius: 0.0,
    };
    renderer
        .bvh_state_mut()
        .insert_streaming_chunk(&queue, chunk, &content)
        .expect("insert sphere chunk");

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("multi_cascade_smoke_setup"),
    });
    renderer
        .bvh_state_mut()
        .tick_uniforms(&queue, &mut encoder, 0.0, 0.0);
    queue.submit(std::iter::once(encoder.finish()));

    // Bootstrap path: dispatch_gdf_populate runs cascade 0 on its
    // own. To exercise the multi-cascade path, run several frames
    // through the scheduler — each tick advances `frame_idx` and
    // brings successive cascades online.
    for _ in 0..6 {
        renderer.dispatch_gdf_populate(&device, &queue, Vec3::ZERO);
    }
    // Force submission and wait — surfaces any deferred validation
    // errors as a panic in `device.poll`.
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(5)),
        })
        .expect("device poll");
}
