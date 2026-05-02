//! End-to-end bench: cull → vbuf → deferred shade for a sphere mesh,
//! measuring wall-clock frame time over N iterations.
//!
//! Marked `#[ignore]` by default; run with:
//!   cargo test -p ome_render --test meshlet_bench -- --ignored --test-threads=1
//!
//! The test asserts the median frame time stays below a generous
//! target (16 ms = 60 Hz). Real numbers on RX 9070 XT for the
//! current pipeline + a 1024-triangle sphere are well under 1 ms;
//! the loose bound catches catastrophic regressions without making
//! the test driver-fragile on weaker hardware (Steam Deck APU).

mod common;

use common::{build_sphere_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use ome_render::material::{MaterialParams, MaterialPool};
use ome_render::meshlet::{
    build_default_meshlets, meshlet_bind_group, meshlet_bind_group_layout, CullParams,
    MeshletCull, MeshletDeferredShader, MeshletVisRasterizer, DEFAULT_MAX_TRIANGLES,
    DEFERRED_COLOR_FORMAT, VISIBILITY_BUFFER_FORMAT,
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
    let vbuf_raster = MeshletVisRasterizer::new(
        &device,
        Some(DEPTH_FORMAT),
        cull.meshlet_bind_group_layout(),
        None,
    );
    let deferred = MeshletDeferredShader::new(&device, cull.meshlet_bind_group_layout());

    let meshlet_bgl = meshlet_bind_group_layout(&device);
    let meshlet_bg = meshlet_bind_group(&device, &meshlet_bgl, &gpu_mesh);

    let materials = MaterialPool::new(
        &device,
        &[MaterialParams::new([0.8, 0.6, 0.4, 1.0], 0.0, 0.4, 0.0)],
    );
    let material_bg = materials.bind_group(&device);

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
    let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view_proj = proj * view;
    let model = Mat4::IDENTITY;
    let cull_params = CullParams::new(view_proj, cam, gpu_mesh.meshlet_count);

    // Warm-up: pipeline-cache + first compile + first GPU upload.
    let mut warmup = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("bench_warmup"),
    });
    cull.dispatch(&device, &queue, &mut warmup, &gpu_mesh, &cull_params);
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
        cull.dispatch(&device, &queue, &mut enc, &gpu_mesh, &cull_params);
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
