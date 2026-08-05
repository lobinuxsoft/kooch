//! End-to-end bench: cull → vbuf → deferred shade for a sphere mesh,
//! measuring wall-clock frame time over N iterations.
//!
//! Marked `#[ignore]` by default; run with:
//!   cargo test -p kooch_render --test meshlet_bench -- --ignored
//!
//! The test asserts the median frame time stays below a generous
//! target (16 ms = 60 Hz). Real numbers on RX 9070 XT for the
//! current pipeline + a 1024-triangle sphere are well under 1 ms;
//! the loose bound catches catastrophic regressions without making
//! the test driver-fragile on weaker hardware (Steam Deck APU).

mod common;

use common::{build_sphere_mesh, read_u32, try_acquire_device, try_acquire_device_with_timer};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, DEFERRED_COLOR_FORMAT, MeshletCull, MeshletCullPipelines,
    MeshletDeferredShader, MeshletGpuTimers, MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT,
    build_default_meshlets, meshlet_bind_group, meshlet_bind_group_layout,
};

const RT_SIZE: u32 = 256;
const FRAME_COUNT: usize = 32;
const TARGET_MEDIAN_MS: f64 = 16.0;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[test]
#[ignore = "bench: long-running, needs GPU"]
fn meshlet_bench_sphere_renders_under_target_frame_time() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Sphere with 32×32 quads = 2048 triangles. meshopt clusters into
    // ~16 meshlets (well below the cull dispatcher's 64-element wave),
    // exercising the multi-meshlet code path without tipping into a
    // long benchmark.
    let mesh = build_sphere_mesh(32, 32);
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);
    eprintln!(
        "bench mesh: {} verts, {} tris, {} meshlets",
        meshlet_mesh.total_vertex_count(),
        meshlet_mesh.total_triangle_count(),
        gpu_mesh.meshlet_count
    );

    let cull = MeshletCull::new(
        &device,
        gpu_mesh.meshlet_count.max(1) * 2,
        DEFAULT_MAX_TRIANGLES as u32,
    );
    let cull_pipelines = MeshletCullPipelines::new(&device);
    let vbuf_raster = MeshletVisRasterizer::new(
        &device,
        Some(DEPTH_FORMAT),
        cull_pipelines.meshlet_bind_group_layout(),
        None,
    );
    let deferred = MeshletDeferredShader::new(&device, cull_pipelines.meshlet_bind_group_layout());

    let meshlet_bgl = meshlet_bind_group_layout(&device);
    let meshlet_bg = meshlet_bind_group(&device, &meshlet_bgl, &gpu_mesh);

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    materials.register(
        &queue,
        Guid::new_v4(),
        &Material::new([0.8, 0.6, 0.4, 1.0], 0.0, 0.4, 0.0),
    );
    let material_bg = materials.pool().bind_group(&device);

    let vbuf_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_vbuf"),
        size: wgpu::Extent3d {
            width: RT_SIZE,
            height: RT_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VISIBILITY_BUFFER_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let vbuf_view = vbuf_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_color"),
        size: wgpu::Extent3d {
            width: RT_SIZE,
            height: RT_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEFERRED_COLOR_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_depth"),
        size: wgpu::Extent3d {
            width: RT_SIZE,
            height: RT_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let cam = Vec3::new(0.0, 0.0, 3.0);
    let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view_proj = proj * view;
    let model = Mat4::IDENTITY;
    let cull_params = CullParams::new(view_proj, cam, gpu_mesh.meshlet_count);

    // Warm-up: pipeline-cache + first compile + first GPU upload.
    let mut warmup = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("bench_warmup"),
    });
    cull.dispatch(
        &cull_pipelines,
        &device,
        &queue,
        &mut warmup,
        &gpu_mesh,
        &cull_params,
    );
    vbuf_raster.render(
        &device,
        &queue,
        &mut warmup,
        &vbuf_view,
        Some(&depth_view),
        &meshlet_bg,
        &cull,
        view_proj,
        model,
        0,
    );
    deferred.shade(
        &device,
        &queue,
        &mut warmup,
        &vbuf_view,
        &color_view,
        &meshlet_bg,
        &material_bg,
        view_proj,
        model,
        (RT_SIZE, RT_SIZE),
        0,
    );
    queue.submit(std::iter::once(warmup.finish()));
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });

    // Measured iterations.
    let mut samples_ms = Vec::with_capacity(FRAME_COUNT);
    for _ in 0..FRAME_COUNT {
        let t0 = std::time::Instant::now();
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bench_frame"),
        });
        cull.dispatch(
            &cull_pipelines,
            &device,
            &queue,
            &mut enc,
            &gpu_mesh,
            &cull_params,
        );
        vbuf_raster.render(
            &device,
            &queue,
            &mut enc,
            &vbuf_view,
            Some(&depth_view),
            &meshlet_bg,
            &cull,
            view_proj,
            model,
            0,
        );
        deferred.shade(
            &device,
            &queue,
            &mut enc,
            &vbuf_view,
            &color_view,
            &meshlet_bg,
            &material_bg,
            view_proj,
            model,
            (RT_SIZE, RT_SIZE),
            0,
        );
        queue.submit(std::iter::once(enc.finish()));
        // Block until GPU drains so the timer measures wall time the
        // pipeline actually took, not CPU encoder cost.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        let elapsed = t0.elapsed();
        samples_ms.push(elapsed.as_secs_f64() * 1000.0);
    }

    samples_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples_ms[samples_ms.len() / 2];
    let p99 = samples_ms[(samples_ms.len() * 99 / 100).min(samples_ms.len() - 1)];

    eprintln!(
        "bench frames={FRAME_COUNT} median={median:.3}ms p99={p99:.3}ms target={TARGET_MEDIAN_MS}ms"
    );

    assert!(
        median < TARGET_MEDIAN_MS,
        "median frame time {median:.3} ms exceeded target {TARGET_MEDIAN_MS} ms"
    );
}

// ---------------------------------------------------------------------
// #335 — mesh-frame end-to-end bench at three meshlet-count scales.
//
// AC: drive the cull → vbuf → deferred path at N ∈ {1k, 10k, 65k}
// meshlets in a 1280×720 offscreen viewport, capture per-pass GPU
// time via TIMESTAMP_QUERY, and report the cull ratio.
// ---------------------------------------------------------------------

/// Three pass timestamps per frame in encoder order: cull, vbuf
/// raster, deferred shade.
const BENCH_STAGE_COUNT: u32 = 3;
const BENCH_STAGE_CULL: u32 = 0;
const BENCH_STAGE_VBUF: u32 = 1;
const BENCH_STAGE_DEFERRED: u32 = 2;

/// 1280×720 — AC. Viewport target the editor / runtime would use on
/// a 720p display; the offscreen textures match so per-pass times are
/// representative of a real frame budget, not a 256² toy.
const BENCH_RT_WIDTH: u32 = 1280;
const BENCH_RT_HEIGHT: u32 = 720;

/// 32 measured frames per scale — same as the smoke bench above.
/// Median and p99 stay stable; adding more frames does not move the
/// numbers meaningfully and lengthens CI for no signal.
const BENCH_FRAME_COUNT: usize = 32;

/// AC targets the GPU should land at. The sphere builder below picks
/// `lat_segments` to approximate each count under the default
/// `MAX_TRIANGLES = 128` meshlet packing.
const BENCH_TARGETS: &[u32] = &[1_024, 10_240, 65_536];

/// Picks a `lat_segments` value so `build_sphere_mesh(lat, 2*lat)`
/// yields roughly `target_meshlets` after `build_default_meshlets`.
/// Empirical fit: meshopt packs the sphere at ~115 tris per meshlet
/// with the default 128-triangle / 64-vertex budget, so a sphere of
/// `lat × 2lat` quads (= 4·lat² triangles) emits ~`(4·lat²)/115`
/// meshlets. Solve for lat: `lat ≈ sqrt(115·target/4)`.
fn pick_lat_segments_for_target(target_meshlets: u32) -> u32 {
    let lat = ((115.0 * target_meshlets as f64 / 4.0).sqrt()).round() as u32;
    lat.max(8)
}

#[test]
#[ignore = "bench: long-running + needs GPU with TIMESTAMP_QUERY"]
fn meshlet_bench_scaling_per_pass_timings() {
    let Some((device, queue, adapter)) = try_acquire_device_with_timer() else {
        eprintln!(
            "no GPU adapter with TIMESTAMP_QUERY + TIMESTAMP_QUERY_INSIDE_ENCODERS; \
             skipping #335 scaling bench"
        );
        return;
    };

    eprintln!(
        "#335 mesh-frame bench: {BENCH_RT_WIDTH}×{BENCH_RT_HEIGHT}, \
         {BENCH_FRAME_COUNT} frames, 3-stage GPU timer (cull / vbuf / deferred)"
    );
    eprintln!(
        "{:>9} | {:>7} | {:>7} | {:>7} | {:>7} | {:>7} | {:>6}",
        "target", "meshls", "total", "cull", "vbuf", "deferr", "visible"
    );
    eprintln!("{}", "-".repeat(70));

    for &target in BENCH_TARGETS {
        let lat = pick_lat_segments_for_target(target);
        let mesh = build_sphere_mesh(lat, lat * 2);
        let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
        let gpu_mesh = meshlet_mesh.upload(&device);

        let cull = MeshletCull::new(
            &device,
            gpu_mesh.meshlet_count.max(1) * 2,
            DEFAULT_MAX_TRIANGLES as u32,
        );
        let cull_pipelines = MeshletCullPipelines::new(&device);
        let vbuf_raster = MeshletVisRasterizer::new(
            &device,
            Some(DEPTH_FORMAT),
            cull_pipelines.meshlet_bind_group_layout(),
            None,
        );
        let deferred =
            MeshletDeferredShader::new(&device, cull_pipelines.meshlet_bind_group_layout());

        let meshlet_bgl = meshlet_bind_group_layout(&device);
        let meshlet_bg = meshlet_bind_group(&device, &meshlet_bgl, &gpu_mesh);

        let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
        materials.register(
            &queue,
            Guid::new_v4(),
            &Material::new([0.8, 0.6, 0.4, 1.0], 0.0, 0.4, 0.0),
        );
        let material_bg = materials.pool().bind_group(&device);

        let (vbuf_view, color_view, depth_view) = bench_offscreen_targets(&device);

        let cam = Vec3::new(0.0, 0.0, 3.0);
        let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
        let aspect = BENCH_RT_WIDTH as f32 / BENCH_RT_HEIGHT as f32;
        let proj =
            kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), aspect, 0.1, 100.0);
        let view_proj = proj * view;
        let model = Mat4::IDENTITY;
        let cull_params = CullParams::new(view_proj, cam, gpu_mesh.meshlet_count);

        let mut timers =
            MeshletGpuTimers::new_with_stages(&device, &queue, &adapter, BENCH_STAGE_COUNT);
        assert!(
            timers.is_enabled(),
            "TIMESTAMP_QUERY adapter reported but timer disabled"
        );

        // Warm-up frame: compiles pipelines + uploads + first-use
        // driver costs. Discarded so the measured iterations see
        // steady-state behaviour.
        bench_run_frame(
            &device,
            &queue,
            &cull,
            &cull_pipelines,
            &vbuf_raster,
            &deferred,
            &meshlet_bg,
            &material_bg,
            &gpu_mesh,
            &vbuf_view,
            &color_view,
            &depth_view,
            view_proj,
            model,
            &cull_params,
            &mut timers,
            /* record_timing */ false,
        );

        let mut totals_ms = Vec::with_capacity(BENCH_FRAME_COUNT);
        let mut cull_ms = Vec::with_capacity(BENCH_FRAME_COUNT);
        let mut vbuf_ms = Vec::with_capacity(BENCH_FRAME_COUNT);
        let mut deferred_ms = Vec::with_capacity(BENCH_FRAME_COUNT);
        for _ in 0..BENCH_FRAME_COUNT {
            bench_run_frame(
                &device,
                &queue,
                &cull,
                &cull_pipelines,
                &vbuf_raster,
                &deferred,
                &meshlet_bg,
                &material_bg,
                &gpu_mesh,
                &vbuf_view,
                &color_view,
                &depth_view,
                view_proj,
                model,
                &cull_params,
                &mut timers,
                /* record_timing */ true,
            );
            // Spin the ring until the most-recent slot drains its
            // map_async callback. The bench blocks per-frame, so 1-2
            // poll iterations are enough; cap at 10 to guard against
            // a stuck driver thread without hanging the test.
            for _ in 0..10 {
                timers.drain_ready();
                if timers.last_frame_stage_timings().is_some() {
                    break;
                }
                let _ = device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_millis(100)),
                });
            }
            let Some(timings) = timers.last_frame_stage_timings() else {
                panic!("GPU timer never produced a reading after 10 polls");
            };
            cull_ms.push(timings[BENCH_STAGE_CULL as usize] as f64);
            vbuf_ms.push(timings[BENCH_STAGE_VBUF as usize] as f64);
            deferred_ms.push(timings[BENCH_STAGE_DEFERRED as usize] as f64);
            totals_ms.push(timings.iter().map(|t| *t as f64).sum());
        }

        let visible = read_u32(&device, &queue, cull.visible_count_buffer(), 0);
        let cull_ratio = visible as f64 / gpu_mesh.meshlet_count as f64;
        eprintln!(
            "{:>9} | {:>7} | {:>6.2}ms | {:>6.3}ms | {:>5.3}ms | {:>5.3}ms | {:>4.0}%",
            target,
            gpu_mesh.meshlet_count,
            median(&totals_ms),
            median(&cull_ms),
            median(&vbuf_ms),
            median(&deferred_ms),
            cull_ratio * 100.0,
        );

        // Loose sanity bound: even the 65k case on a Steam Deck APU
        // is expected to stay under 100 ms median. A regression
        // catastrophic enough to blow past that needs to fail this
        // bench, not lurk under "still under 16 ms on my desktop".
        assert!(
            median(&totals_ms) < 100.0,
            "target {target} meshlets: median total {:.2} ms > 100 ms catastrophic bound",
            median(&totals_ms),
        );
    }
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

fn bench_offscreen_targets(
    device: &wgpu::Device,
) -> (wgpu::TextureView, wgpu::TextureView, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: BENCH_RT_WIDTH,
        height: BENCH_RT_HEIGHT,
        depth_or_array_layers: 1,
    };
    let vbuf = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_scaling_vbuf"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VISIBILITY_BUFFER_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_scaling_color"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEFERRED_COLOR_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_scaling_depth"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    (
        vbuf.create_view(&wgpu::TextureViewDescriptor::default()),
        color.create_view(&wgpu::TextureViewDescriptor::default()),
        depth.create_view(&wgpu::TextureViewDescriptor::default()),
    )
}

#[allow(clippy::too_many_arguments)]
fn bench_run_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cull: &MeshletCull,
    cull_pipelines: &MeshletCullPipelines,
    vbuf_raster: &MeshletVisRasterizer,
    deferred: &MeshletDeferredShader,
    meshlet_bg: &wgpu::BindGroup,
    material_bg: &wgpu::BindGroup,
    gpu_mesh: &kooch_render::meshlet::GpuMeshletMesh,
    vbuf_view: &wgpu::TextureView,
    color_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    view_proj: Mat4,
    model: Mat4,
    cull_params: &CullParams,
    timers: &mut MeshletGpuTimers,
    record_timing: bool,
) {
    let timer_slot = if record_timing {
        timers.acquire_slot()
    } else {
        None
    };
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("bench_scaling_frame"),
    });
    if timer_slot.is_some() {
        timers.write_stage_start(&mut enc, BENCH_STAGE_CULL);
    }
    cull.dispatch(
        cull_pipelines,
        device,
        queue,
        &mut enc,
        gpu_mesh,
        cull_params,
    );
    if timer_slot.is_some() {
        timers.write_stage_end(&mut enc, BENCH_STAGE_CULL);
        timers.write_stage_start(&mut enc, BENCH_STAGE_VBUF);
    }
    vbuf_raster.render(
        device,
        queue,
        &mut enc,
        vbuf_view,
        Some(depth_view),
        meshlet_bg,
        cull,
        view_proj,
        model,
        0,
    );
    if timer_slot.is_some() {
        timers.write_stage_end(&mut enc, BENCH_STAGE_VBUF);
        timers.write_stage_start(&mut enc, BENCH_STAGE_DEFERRED);
    }
    deferred.shade(
        device,
        queue,
        &mut enc,
        vbuf_view,
        color_view,
        meshlet_bg,
        material_bg,
        view_proj,
        model,
        (BENCH_RT_WIDTH, BENCH_RT_HEIGHT),
        0,
    );
    if let Some(slot_idx) = timer_slot {
        timers.write_stage_end(&mut enc, BENCH_STAGE_DEFERRED);
        timers.resolve_and_copy(&mut enc, slot_idx);
    }
    queue.submit(std::iter::once(enc.finish()));
    if let Some(slot_idx) = timer_slot {
        timers.submit_readback(slot_idx);
    }
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
}
