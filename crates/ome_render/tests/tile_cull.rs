//! AC of PR-6 (epic #370) — tile-based ray bounds pre-pass.
//!
//! A compute pass walks the coarsest GDF cascade (cascade 5, voxel
//! pitch 8 km, cube extent 524 km) once per 8×8 viewport tile and
//! writes `(t_min, t_max, flags)` to a persistent SSBO; the fragment
//! shader reads its tile entry to early-discard sky tiles or clamp
//! the ray-march loop to the relevant `t` window.
//!
//! Four tests pin the contract:
//!
//! 1. `tile_cull_empty_scene_marks_all_tiles_empty` — populate cascade 5
//!    with no chunks resident; assert every tile's `flags == 0`.
//! 2. `tile_cull_full_scene_marks_central_tiles_non_empty` — large
//!    sphere placed where cascade 5 voxels actually register a hit;
//!    assert at least one centre tile is `flags == 1`.
//! 3. `tile_cull_t_bounds_within_cascade_extent` — single sphere at a
//!    known distance; assert non-empty tiles satisfy
//!    `0 <= t_min < t_max <= cascade_5_extent_diagonal`.
//! 4. `tile_cull_consistency_with_fragment_sample` — render the scene
//!    off-screen with a separate sky pass disabled (internal sky
//!    fallback active); assert pixels of empty tiles take the sky path.

mod common;

use common::raymarch_render::{
    COLOR_FORMAT, TARGET_SIZE, dispatch_and_readback_tile_bounds, make_offscreen, pixel_at,
    pixel_is_surface, render_and_readback, setup_sphere_scene, write_camera_and_meta,
};
use common::try_acquire_device;
use glam::Vec3;
use ome_render::gdf::cascade_cube_extent;
use ome_render::raymarch::RayMarchRenderer;
use ome_render::tile_cull::TILE_FLAG_NON_EMPTY;
use ome_world::ChunkId;

const VIEWPORT: u32 = TARGET_SIZE; // 64×64 → 8×8 tile grid.
const TILE_GRID: u32 = VIEWPORT / 8;

/// Populate every cascade in one submission. Tile cull samples cascade 5,
/// which the round-robin scheduler only updates every 32 frames in
/// steady state — tests bypass the scheduler so cascade 5 reflects
/// the inserted geometry on the first dispatch.
fn populate_all_cascades(
    renderer: &mut RayMarchRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera_pos: Vec3,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tile_cull_test::populate_all_cascades"),
    });
    for c in 0..6usize {
        renderer.gdf_state_mut().dispatch_populate_cascade(
            &mut encoder, queue, c, camera_pos,
        );
    }
    queue.submit(std::iter::once(encoder.finish()));
}

#[test]
fn tile_cull_empty_scene_marks_all_tiles_empty() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("tile_cull: no GPU adapter — skipping");
        return;
    };
    let mut renderer = RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);

    // No chunks inserted. Cascade 5 populate over an empty pool returns
    // the union identity (per `gdf_populate_empty_pool`), so every
    // sampled distance is large; the coarse march never reports a hit.
    populate_all_cascades(&mut renderer, &device, &queue, Vec3::ZERO);
    write_camera_and_meta(&renderer, &queue, Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);

    renderer.set_viewport_size(VIEWPORT, VIEWPORT);
    let bounds = dispatch_and_readback_tile_bounds(&device, &queue, &mut renderer);

    assert_eq!(bounds.len(), (TILE_GRID * TILE_GRID) as usize);
    for (i, b) in bounds.iter().enumerate() {
        assert_eq!(
            b.flags & TILE_FLAG_NON_EMPTY,
            0,
            "tile #{i} flagged non-empty in an empty scene; bounds = {b:?}"
        );
    }
}

#[test]
fn tile_cull_full_scene_marks_central_tiles_non_empty() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("tile_cull: no GPU adapter — skipping");
        return;
    };
    let mut renderer = RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);

    // Cascade 5 voxel pitch 8 km, step floor 4 km — small near-field
    // spheres do not register. Place a 16 km radius sphere 50 km in
    // front of the camera so the coarse march reads SDF values inside
    // the step floor at multiple cascade-5 voxel centres along the
    // line of sight.
    let sphere_pos = Vec3::new(0.0, 0.0, 50_000.0);
    let camera_pos = Vec3::new(0.0, 0.0, 0.0);
    setup_sphere_scene(
        &mut renderer,
        &device,
        &queue,
        sphere_pos,
        16_000.0,
        ChunkId::new(glam::IVec3::ZERO, 0),
    );
    populate_all_cascades(&mut renderer, &device, &queue, camera_pos);
    write_camera_and_meta(&renderer, &queue, camera_pos, sphere_pos);

    renderer.set_viewport_size(VIEWPORT, VIEWPORT);
    let bounds = dispatch_and_readback_tile_bounds(&device, &queue, &mut renderer);

    // Tile (3, 3) of an 8×8 grid — covers viewport pixel (24, 24)
    // through (31, 31), i.e. straight ahead of the centred camera.
    let mid = TILE_GRID / 2 - 1; // 3 for an 8-wide grid.
    let centre_idx = (mid * TILE_GRID + mid) as usize;
    let centre = bounds[centre_idx];
    assert!(
        centre.flags & TILE_FLAG_NON_EMPTY != 0,
        "centre tile [{mid},{mid}] not flagged non-empty for a 16 km sphere at \
         50 km in front of the camera; got flags={} t_min={} t_max={}",
        centre.flags,
        centre.t_min,
        centre.t_max,
    );
}

#[test]
fn tile_cull_t_bounds_within_cascade_extent() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("tile_cull: no GPU adapter — skipping");
        return;
    };
    let mut renderer = RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);

    let sphere_pos = Vec3::new(0.0, 0.0, 50_000.0);
    let camera_pos = Vec3::new(0.0, 0.0, 0.0);
    setup_sphere_scene(
        &mut renderer,
        &device,
        &queue,
        sphere_pos,
        16_000.0,
        ChunkId::new(glam::IVec3::ZERO, 0),
    );
    populate_all_cascades(&mut renderer, &device, &queue, camera_pos);
    write_camera_and_meta(&renderer, &queue, camera_pos, sphere_pos);

    renderer.set_viewport_size(VIEWPORT, VIEWPORT);
    let bounds = dispatch_and_readback_tile_bounds(&device, &queue, &mut renderer);

    // Cube diagonal of cascade 5 — the AABB exit `t` for a ray
    // starting inside the cube cannot exceed the half-diagonal for an
    // axis-aligned ray, but worst-case slab math for skew rays caps
    // at the full diagonal. Pin the loose bound so any future change
    // to cascade 5 sizing surfaces.
    let cascade_5_diagonal = cascade_cube_extent(5) * f32::sqrt(3.0);
    let mid = TILE_GRID / 2 - 1;
    let centre_idx = (mid * TILE_GRID + mid) as usize;
    let centre = bounds[centre_idx];

    // Centre tile must report a hit — the sphere is dead ahead.
    assert!(
        centre.flags & TILE_FLAG_NON_EMPTY != 0,
        "centre tile must hit the sphere; got {centre:?}"
    );
    assert!(
        centre.t_min >= 0.0 && centre.t_min < 50_000.0,
        "centre tile t_min outside (0, sphere distance); got {centre:?}"
    );
    assert!(
        centre.t_max > centre.t_min && centre.t_max <= cascade_5_diagonal,
        "centre tile t_max out of cascade 5 range; got t_max={} (diagonal={})",
        centre.t_max,
        cascade_5_diagonal,
    );
}

#[test]
fn tile_cull_consistency_with_fragment_sample() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("tile_cull: no GPU adapter — skipping");
        return;
    };
    let mut renderer = RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);

    let sphere_pos = Vec3::new(0.0, 0.0, 50_000.0);
    let camera_pos = Vec3::new(0.0, 0.0, 0.0);
    setup_sphere_scene(
        &mut renderer,
        &device,
        &queue,
        sphere_pos,
        16_000.0,
        ChunkId::new(glam::IVec3::ZERO, 0),
    );
    populate_all_cascades(&mut renderer, &device, &queue, camera_pos);
    write_camera_and_meta(&renderer, &queue, camera_pos, sphere_pos);

    // Match `max_distance` to the cascade-5 reach so the ray budget
    // doesn't terminate before the sphere on near-axis rays.
    renderer.params.max_distance = 200_000.0;
    renderer.params.max_steps = 512;
    renderer.write_raymarch_params(&queue);

    renderer.set_viewport_size(VIEWPORT, VIEWPORT);
    // Dispatch tile cull first so the SSBO is up to date for the
    // fragment shader's lookup.
    let bounds = dispatch_and_readback_tile_bounds(&device, &queue, &mut renderer);

    // Now render off-screen. The fragment shader reads the same SSBO
    // tile cull just wrote — empty tiles take the internal-sky branch.
    let targets = make_offscreen(&device);
    let pixels = render_and_readback(&device, &queue, &renderer, &targets);

    // For every empty tile (flags=0), every pixel inside the tile must
    // NOT be surface (R > B + 8). Tiles not adjacent to the sphere are
    // empty; pixels there must be sky (cool blue). One mismatch means
    // the fragment didn't gate on the tile flag.
    let mut empty_tiles = 0u32;
    let mut nonempty_tiles = 0u32;
    let mut surface_pixel_in_empty_tile = 0u32;
    for ty in 0..TILE_GRID {
        for tx in 0..TILE_GRID {
            let tile = bounds[(ty * TILE_GRID + tx) as usize];
            if tile.flags & TILE_FLAG_NON_EMPTY == 0 {
                empty_tiles += 1;
                for py in (ty * 8)..((ty + 1) * 8) {
                    for px in (tx * 8)..((tx + 1) * 8) {
                        let pixel = pixel_at(&pixels, targets.bytes_per_row, px, py);
                        if pixel_is_surface(pixel) {
                            surface_pixel_in_empty_tile += 1;
                        }
                    }
                }
            } else {
                nonempty_tiles += 1;
            }
        }
    }
    assert!(
        empty_tiles > 0,
        "no empty tiles in this scene — test geometry must produce at least \
         a few sky tiles to validate the consistency direction"
    );
    assert!(
        nonempty_tiles > 0,
        "no non-empty tiles — sphere must register in cascade 5 for this test \
         to mean anything"
    );
    assert_eq!(
        surface_pixel_in_empty_tile, 0,
        "{surface_pixel_in_empty_tile} surface pixels found inside empty tiles; \
         fragment shader must take the sky branch when flags == 0"
    );
}

/// Wall-clock frame-time bench. Run with `cargo test -p ome_render --test
/// tile_cull -- --ignored tile_cull_frame_time_smoke_bench --nocapture`
/// to log a median over 128 frames against a single big sphere. The
/// PR-6 target is `≤ 6 ms` on the RX 9070 XT at 432 chunks resident;
/// this smoke variant uses one chunk so the number is a regression
/// floor, not the production-scene budget.
#[test]
#[ignore = "wall-clock bench, run on demand"]
fn tile_cull_frame_time_smoke_bench() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("tile_cull_bench: no GPU adapter — skipping");
        return;
    };
    let mut renderer = RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);
    let sphere_pos = Vec3::new(0.0, 0.0, 50_000.0);
    let camera_pos = Vec3::new(0.0, 0.0, 0.0);
    setup_sphere_scene(
        &mut renderer,
        &device,
        &queue,
        sphere_pos,
        16_000.0,
        ChunkId::new(glam::IVec3::ZERO, 0),
    );
    populate_all_cascades(&mut renderer, &device, &queue, camera_pos);
    write_camera_and_meta(&renderer, &queue, camera_pos, sphere_pos);
    renderer.params.max_distance = 200_000.0;
    renderer.write_raymarch_params(&queue);
    renderer.set_viewport_size(VIEWPORT, VIEWPORT);
    let targets = make_offscreen(&device);

    // Warm-up frame so pipeline / bind-group caches are hot.
    let _ = render_and_readback(&device, &queue, &renderer, &targets);

    let n = 128u32;
    let mut samples: Vec<f64> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        renderer.dispatch_tile_cull(&device, &queue);
        let start = std::time::Instant::now();
        let _ = render_and_readback(&device, &queue, &renderer, &targets);
        samples.push(start.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples[(n / 2) as usize];
    let max = *samples.last().unwrap();
    let min = samples[0];
    eprintln!(
        "tile_cull frame_time: median {median:.3} ms / min {min:.3} ms / max {max:.3} ms \
         over {n} frames at {VIEWPORT}x{VIEWPORT} (1 chunk, 16 km sphere)"
    );
}
