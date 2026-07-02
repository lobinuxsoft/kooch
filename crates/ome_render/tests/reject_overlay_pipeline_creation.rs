//! Integration test that exercises `MeshletRejectOverlay::new` on a
//! real device and asserts no uncaptured wgpu errors are raised during
//! pipeline creation. Catches "ComputePipeline … is invalid" failures
//! that surface only at submit time on the editor — the validation
//! error is reported asynchronously via `on_uncaptured_error` and the
//! pipeline is then a black hole.
//!
//! Also dispatches the overlay against a 1-instance / 1-mesh scene so
//! the per-frame bind-group construction (debug_bg, pool_bg, scene_bg)
//! goes through wgpu's validator. Adding a binding to the cull-side
//! `debug_bgl` without updating the overlay's `debug_bg` entries was
//! caught by the editor smoke in #454.6 — this dispatch ensures the
//! same shape mismatch trips here first.

use glam::{Mat4, Vec3};
use ome_render::mesh::{Mesh, MeshVertex};
use ome_render::meshlet::{
    DEFAULT_MAX_TRIANGLES, GlobalMeshPool, MeshInstance, MeshletCull, MeshletRejectOverlay,
    MeshletScene, RejectReason, build_default_meshlets,
};
use std::sync::{Arc, Mutex};

fn build_unit_quad() -> Mesh {
    let vertices = vec![
        MeshVertex {
            position: [-0.5, -0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        MeshVertex {
            position: [0.5, -0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 0.0],
        },
        MeshVertex {
            position: [0.5, 0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 1.0],
        },
        MeshVertex {
            position: [-0.5, 0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
    ];
    let indices = vec![0, 1, 2, 0, 2, 3];
    Mesh::from_arrays(vertices, indices)
}

fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;

    // Reject overlay needs TEXTURE_ATOMIC for parity with the
    // density / overdraw heatmaps, plus the Rgba8Unorm storage
    // format the deferred shader already uses.
    let needed =
        wgpu::Features::TEXTURE_ATOMIC | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(needed) {
        return None;
    }

    let mut limits = wgpu::Limits::default();
    limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);
    limits.max_storage_buffers_per_shader_stage =
        16.min(adapter.limits().max_storage_buffers_per_shader_stage);

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("reject_overlay_creation_test_device"),
        required_features: needed,
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

#[test]
fn reject_overlay_creates_without_uncaptured_errors() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!(
            "TEXTURE_ATOMIC unavailable on this adapter — skipping reject overlay creation test"
        );
        return;
    };

    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let errors_capture = errors.clone();
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        errors_capture
            .lock()
            .expect("poisoned")
            .push(format!("{error}"));
    }));

    let cull = MeshletCull::new(&device, 4096, DEFAULT_MAX_TRIANGLES as u32);
    let overlay = MeshletRejectOverlay::new(&device, &cull);

    // Dispatch a real frame so debug_bg / pool_bg / scene_bg are
    // constructed against the same BGLs the cull pipeline owns. A
    // shape mismatch (BGL grew a binding, bind_group entries didn't
    // follow) shows up here as a "Number of bindings ... does not
    // match" validation error rather than at editor submit time.
    let mesh = build_unit_quad();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let mut pool = GlobalMeshPool::new();
    let mesh_handle = pool.register(&meshlet_mesh);
    let gpu_pool = pool.upload(&device);

    let scene = MeshletScene::new(&device, 1);
    scene.upload_instances(
        &queue,
        &[MeshInstance::new(Mat4::IDENTITY, mesh_handle.mesh_id, 0)],
    );

    let (color_texture, color_view) = {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reject_overlay_test_color"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    };

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("reject_overlay_test_encoder"),
    });
    overlay.dispatch(
        &device,
        &queue,
        &mut encoder,
        &color_view,
        &cull,
        &scene,
        &gpu_pool,
        Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y),
        (64, 64),
        RejectReason::Frustum,
        2,
        gpu_pool.max_meshlets_per_mesh,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    drop(color_texture);

    let captured = errors.lock().expect("poisoned");
    assert!(
        captured.is_empty(),
        "MeshletRejectOverlay construction or dispatch raised wgpu validation errors:\n  - {}",
        captured.join("\n  - ")
    );
}
