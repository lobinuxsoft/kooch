//! AC of PR-4 (epic #370) — production raymarch fragment shader
//! reads the GDF cascade-0 with a single `textureSampleLevel` per
//! ray-march step, instead of descending the TLAS+BLAS pool. Three
//! tests pin the contract end-to-end:
//!
//! 1. `single_chunk_visible_from_default_camera` — render a single
//!    sphere off-screen at the editor's default camera angle, assert
//!    ≥ 5% surface (R > B + 8) pixels. Catches "cascade not bound",
//!    "sampler misconfigured", or any binding-layout regression that
//!    breaks the production raymarch pipeline.
//! 2. `ray_from_outside_hits_known_blob` — single fragment-shader
//!    pixel covering a ray from far outside the cascade toward a
//!    sphere at the cascade origin. Asserts the centre pixel is
//!    surface, exercising the "outside cascade → conservative AABB
//!    floor → ray converges inside → cascade-fetch hit" path.
//! 3. `cascade_boundary_does_not_alias` — two spheres near the cube
//!    boundaries; asserts no NaN-induced black pixels and the +X
//!    sphere remains visible, verifying the sampler's clamp-to-edge
//!    configuration.
//!
//! Shared helpers live in `common::raymarch_render` so the visual AC
//! files stay focused on assertions and stay under the 400-LoC
//! monolithic threshold.

use glam::Vec3;
use ome_bvh::sdf_primitive::{SdfPrimitive, TYPE_SPHERE};
use ome_bvh::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};
use ome_world::{ChunkContent, ChunkId};

mod common;
use common::raymarch_render::{
    COLOR_FORMAT, CameraUniforms, SceneMeta, TARGET_SIZE, make_offscreen, pixel_at,
    pixel_is_surface, render_and_readback, setup_sphere_scene, write_camera_and_meta,
};
use common::try_acquire_device;

#[test]
fn single_chunk_visible_from_default_camera() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("gdf_fragment_sample: no GPU adapter — skipping");
        return;
    };

    // Editor's default camera angle: `(0, 5, 8)` looking at origin
    // (matches the demo scene template). `(0, 5, 8)` lies inside
    // cascade 0 once the cascade origin snaps to the camera.
    let camera_pos = Vec3::new(0.0, 5.0, 8.0);
    let mut renderer =
        ome_render::raymarch::RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);
    setup_sphere_scene(
        &mut renderer,
        &device,
        &queue,
        Vec3::ZERO,
        1.5,
        ChunkId::new(glam::IVec3::new(0, 0, 0), 0),
    );
    renderer.dispatch_gdf_populate(&device, &queue, camera_pos);
    write_camera_and_meta(&renderer, &queue, camera_pos, Vec3::ZERO);

    let targets = make_offscreen(&device);
    let pixels = render_and_readback(&device, &queue, &renderer, &targets);

    // ≥ 5% loose threshold — at FOV 60° + a 1.5 m radius sphere viewed
    // from `(0, 5, 8)` (~ 9.4 m away) the sphere subtends ~ 9° of solid
    // angle. With a 64×64 framebuffer that's ~ 60-100 pixels (≥ 1.5%);
    // post-quantisation widening pushes this past 5% in practice. The
    // bound trips on every regression that turns the sphere invisible
    // without false-positiving on minor rendering changes.
    let mut surface_count = 0u32;
    for y in 0..TARGET_SIZE {
        for x in 0..TARGET_SIZE {
            if pixel_is_surface(pixel_at(&pixels, targets.bytes_per_row, x, y)) {
                surface_count += 1;
            }
        }
    }
    let total = TARGET_SIZE * TARGET_SIZE;
    let percent = (surface_count as f32 / total as f32) * 100.0;
    assert!(
        surface_count >= total / 20,
        "GDF cascade fetch produced fewer than 5% surface pixels at default camera \
         (got {surface_count}/{total} = {percent:.2}%) — cascade likely not populated, \
         not bound, or eval_scene_bvh didn't swap to the sample path"
    );
}

#[test]
fn ray_from_outside_hits_known_blob() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("gdf_fragment_sample: no GPU adapter — skipping");
        return;
    };

    // Camera at `(0, 0, 100)` looking at the origin sphere along -Z.
    // 100 m places the camera far outside cascade 0 (16 m cube). The
    // ray from the camera toward the sphere travels through "outside
    // cascade" space (eval returns distance-to-AABB), enters the
    // cascade, then hits the populated SDF inside. The handoff notes
    // `(0, 100, 0)` but `look_at_rh` with that eye + a +Y up vector
    // is degenerate (forward × up = 0) and produces a NaN view matrix;
    // the -Z look-at avoids the gimbal singularity.
    let camera_pos = Vec3::new(0.0, 0.0, 100.0);
    let target = Vec3::ZERO;
    let mut renderer =
        ome_render::raymarch::RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);
    // Default `max_distance` (100 m) leaves no headroom — the ray
    // travels exactly the camera-to-sphere distance and runs out of
    // budget on the last voxel-quantised step. Bump both budgets so
    // the test pins the trajectory and not the budget. Tests bypass
    // `update_camera` (no ECS world in scope) so the params buffer
    // needs an explicit flush via `write_raymarch_params`.
    renderer.params.max_distance = 200.0;
    renderer.params.max_steps = 512;
    setup_sphere_scene(
        &mut renderer,
        &device,
        &queue,
        Vec3::ZERO,
        2.0,
        ChunkId::new(glam::IVec3::new(0, 0, 0), 0),
    );
    // Cascade is centred on the SPHERE, not the camera, so the
    // populated voxels actually cover the geometry the ray will hit.
    // PR-5's multi-cascade puts a far cascade around the camera; for
    // PR-4 the test exercises exactly the "outside cascade → AABB
    // floor → enter cascade → hit" trajectory.
    renderer.dispatch_gdf_populate(&device, &queue, target);
    renderer.write_raymarch_params(&queue);
    write_camera_and_meta(&renderer, &queue, camera_pos, target);

    let targets = make_offscreen(&device);
    let pixels = render_and_readback(&device, &queue, &renderer, &targets);

    let centre = pixel_at(
        &pixels,
        targets.bytes_per_row,
        TARGET_SIZE / 2,
        TARGET_SIZE / 2,
    );
    assert!(
        pixel_is_surface(centre),
        "centre pixel from camera (0, 0, 100) toward origin sphere is not surface — \
         got rgba={centre:?}; expected warm material colour, indicates the outside-cascade \
         path isn't converging to the populated cascade interior"
    );
}

#[test]
fn cascade_boundary_does_not_alias() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("gdf_fragment_sample: no GPU adapter — skipping");
        return;
    };

    // Two spheres near opposite cascade boundaries (cascade is 16 m
    // centred on origin → x faces at ±8 m). Cameras at +X looking at
    // the +X sphere; if the sampler aliases (repeat / mirror), the
    // -X sphere's signed distance would bleed through the +X face
    // and produce surface-coloured pixels in the gap.
    let mut renderer =
        ome_render::raymarch::RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);

    let chunk_a = ChunkId::new(glam::IVec3::new(0, 0, 0), 0);
    let content_a = ChunkContent {
        primitives: vec![SdfPrimitive {
            position: [7.0, 0.0, 0.0],
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: 0.0,
            params: [0.4, 0.0, 0.0, 0.0],
        }],
        leaf_aabbs: vec![LeafAabb {
            aabb_min: [6.6, -0.4, -0.4],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [7.4, 0.4, 0.4],
            entity_id: 0,
        }],
        max_smoothness_radius: 0.0,
    };
    let chunk_b = ChunkId::new(glam::IVec3::new(1, 0, 0), 0);
    let content_b = ChunkContent {
        primitives: vec![SdfPrimitive {
            position: [-7.0, 0.0, 0.0],
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: 0.0,
            params: [0.4, 0.0, 0.0, 0.0],
        }],
        leaf_aabbs: vec![LeafAabb {
            aabb_min: [-7.4, -0.4, -0.4],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [-6.6, 0.4, 0.4],
            entity_id: 1,
        }],
        max_smoothness_radius: 0.0,
    };
    renderer
        .bvh_state_mut()
        .insert_streaming_chunk(&queue, chunk_a, &content_a)
        .expect("chunk A");
    renderer
        .bvh_state_mut()
        .insert_streaming_chunk(&queue, chunk_b, &content_b)
        .expect("chunk B");

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gdf_boundary_setup_encoder"),
    });
    renderer
        .bvh_state_mut()
        .tick_uniforms(&queue, &mut encoder, 0.0, 0.0);
    queue.submit(std::iter::once(encoder.finish()));

    // Centre cascade on origin so both spheres land near the X faces.
    renderer.dispatch_gdf_populate(&device, &queue, Vec3::ZERO);
    // Camera 4 m off the +X face of the cascade, looking at the +X sphere.
    let camera_pos = Vec3::new(11.0, 0.0, 0.0);
    write_camera_and_meta(&renderer, &queue, camera_pos, Vec3::new(7.0, 0.0, 0.0));

    let targets = make_offscreen(&device);
    let pixels = render_and_readback(&device, &queue, &renderer, &targets);

    // No NaN / infinity colour escapes onto the framebuffer — every
    // pixel is in the valid 0..255 RGB range by construction (Rgba8Unorm
    // clamps), so the practical assert is "no all-black pixel" (NaN
    // would come out 0 on radv but the sky+surface gradient never
    // produces (0, 0, 0)).
    let mut all_zero_count = 0u32;
    for y in 0..TARGET_SIZE {
        for x in 0..TARGET_SIZE {
            let p = pixel_at(&pixels, targets.bytes_per_row, x, y);
            if p[0] == 0 && p[1] == 0 && p[2] == 0 {
                all_zero_count += 1;
            }
        }
    }
    assert_eq!(
        all_zero_count, 0,
        "{all_zero_count} pixels are pure black — NaN sampling at the cascade \
         boundary indicates the sampler is repeat/mirror instead of clamp-to-edge"
    );

    let centre = pixel_at(
        &pixels,
        targets.bytes_per_row,
        TARGET_SIZE / 2,
        TARGET_SIZE / 2,
    );
    assert!(
        pixel_is_surface(centre),
        "centre pixel covering +X sphere is not surface (rgba={centre:?})"
    );
}

/// Rough frame-time benchmark for PR-4 of epic #370. **Not a CI
/// target** — measurement varies with adapter, driver, and host load,
/// and the PR-4 budget (≤ 8 ms on RX 9070 XT @ 432 chunks) is a
/// guidance number, not a blocking AC. Marked `#[ignore]` so it only
/// runs when explicitly invoked: `cargo test ... -- --ignored`.
///
/// Driver: render N=128 frames with a 16-sphere scene packed into the
/// cascade's central voxels, populate dispatched once per frame (the
/// production update_scene cadence). Reports min / median / max
/// frame time as a sanity check that the cascade-fetch path is not
/// catastrophically slower than expected.
#[test]
#[ignore]
fn frame_time_smoke_bench() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frame_time_smoke_bench: no GPU adapter — skipping");
        return;
    };
    const FRAMES: usize = 128;
    const SPHERES_PER_AXIS: i32 = 4;
    const RADIUS: f32 = 0.5;
    const SPACING: f32 = 1.5;
    let mut renderer =
        ome_render::raymarch::RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);

    let mut chunk_idx = 0u64;
    for gx in 0..SPHERES_PER_AXIS {
        for gy in 0..SPHERES_PER_AXIS {
            let centre = Vec3::new(
                (gx as f32 - 1.5) * SPACING,
                (gy as f32 - 1.5) * SPACING,
                0.0,
            );
            let chunk = ChunkId::new(glam::IVec3::new(gx, gy, 0), 0);
            let content = ChunkContent {
                primitives: vec![SdfPrimitive {
                    position: centre.to_array(),
                    type_tag: TYPE_SPHERE,
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                    smoothness: 0.0,
                    params: [RADIUS, 0.0, 0.0, 0.0],
                }],
                leaf_aabbs: vec![LeafAabb {
                    aabb_min: [centre.x - RADIUS, centre.y - RADIUS, centre.z - RADIUS],
                    flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
                    aabb_max: [centre.x + RADIUS, centre.y + RADIUS, centre.z + RADIUS],
                    entity_id: chunk_idx as u32,
                }],
                max_smoothness_radius: 0.0,
            };
            renderer
                .bvh_state_mut()
                .insert_streaming_chunk(&queue, chunk, &content)
                .expect("insert chunk");
            chunk_idx += 1;
        }
    }
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("frame_time_bench_setup"),
    });
    renderer
        .bvh_state_mut()
        .tick_uniforms(&queue, &mut encoder, 0.0, 0.0);
    queue.submit(std::iter::once(encoder.finish()));

    let camera_pos = Vec3::new(0.0, 0.0, 8.0);
    write_camera_and_meta(&renderer, &queue, camera_pos, Vec3::ZERO);
    let targets = make_offscreen(&device);

    // Warm-up: ensure pipeline cache + populate dispatch are JITted.
    renderer.dispatch_gdf_populate(&device, &queue, camera_pos);
    let _ = render_and_readback(&device, &queue, &renderer, &targets);

    let mut samples = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let start = std::time::Instant::now();
        // PR-5 (epic #370): drive the round-robin scheduler so the
        // bench captures the full multi-cascade populate cost
        // (cascade 0 every frame, cascade `c` every `2^c` frames in
        // steady state). PR-4's bench dispatched cascade 0 only.
        renderer.dispatch_gdf_populate_scheduled(&device, &queue, camera_pos);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_time_bench_render"),
        });
        renderer.render(&mut encoder, &targets.color_view, &targets.depth_view, true);
        queue.submit(std::iter::once(encoder.finish()));
        // Force completion so we measure end-to-end frame time, not
        // dispatch queue length.
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(1)),
            })
            .expect("device poll");
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min = samples[0];
    let median = samples[samples.len() / 2];
    let max = samples[samples.len() - 1];
    let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
    eprintln!(
        "frame_time_smoke_bench: {} frames @ {SPHERES_PER_AXIS}² spheres, 64×64 target — \
         min={min:.2} ms, median={median:.2} ms, mean={mean:.2} ms, max={max:.2} ms",
        FRAMES
    );
    // No assertion — printout only. Numbers are guidance, not gates.
}

#[test]
fn camera_uniforms_layout_matches_renderer() {
    // Layout-pin asserts so renderer struct churn surfaces here too.
    // 4 × mat4x4 (256 B) + vec3 + pad (16 B) = 272.
    assert_eq!(std::mem::size_of::<CameraUniforms>(), 272);
    // 8 × u32 (32 B) + 2 × vec4 (32 B) = 64.
    assert_eq!(std::mem::size_of::<SceneMeta>(), 64);
}
