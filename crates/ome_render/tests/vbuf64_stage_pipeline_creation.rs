//! Integration test that exercises the production `Vbuf64Stage::new`
//! path on a real device and asserts no uncaptured wgpu errors are
//! raised during pipeline creation. Catches "RenderPipeline … is
//! invalid" failures that surface only at submit time on the editor —
//! the validation error is reported asynchronously via
//! `on_uncaptured_error` and the pipeline is then a black hole.
//!
//! Skips when the adapter does not advertise the int64-atomic feature
//! bundle (#493).

use ome_render::meshlet::Vbuf64Stage;
use std::sync::{Arc, Mutex};

fn try_acquire_device_vbuf64() -> Option<(wgpu::Device, wgpu::Queue)> {
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

    let needed = wgpu::Features::TEXTURE_ATOMIC
        | wgpu::Features::TEXTURE_INT64_ATOMIC
        | wgpu::Features::SHADER_INT64
        | wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(needed) {
        return None;
    }

    let mut limits = wgpu::Limits::default();
    limits.max_storage_textures_per_shader_stage = 16
        .min(adapter.limits().max_storage_textures_per_shader_stage);
    // #493 vbuf64 raster pipeline uses 5 bind groups (camera, pool,
    // visible, instances, vbuf64); #454 adds bind group 5 for the
    // triangle-density accumulator + the uniform that gates the
    // atomicAdd, raising the total to 6. Default is 4. Mirror the
    // production GpuContext setup in `elevated_compute_limits`.
    limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vbuf64_stage_creation_test_device"),
        required_features: needed,
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

#[test]
fn vbuf64_stage_creates_without_uncaptured_errors() {
    let Some((device, _queue)) = try_acquire_device_vbuf64() else {
        eprintln!(
            "vbuf64 features unavailable on this adapter — skipping pipeline creation test"
        );
        return;
    };

    // Trap any wgpu validation error that fires during construction.
    // The editor smoke surfaced "RenderPipeline … is invalid" at submit
    // time; the actual cause is logged asynchronously via this hook
    // when the pipeline is built.
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let errors_capture = errors.clone();
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        errors_capture
            .lock()
            .expect("poisoned")
            .push(format!("{error}"));
    }));

    // Same arguments the production render plugin uses: the meshlet
    // pool BGL comes from `MeshletCull::meshlet_bind_group_layout`,
    // depth format is the engine's reversed-Z target, size matches the
    // default render stage. No surface needed — this is a headless
    // construction smoke.
    let cull = ome_render::meshlet::MeshletCull::new(
        &device,
        4096,
        ome_render::meshlet::DEFAULT_MAX_TRIANGLES as u32,
    );
    let meshlet_bgl = cull.meshlet_bind_group_layout();

    let stage = Vbuf64Stage::new(
        &device,
        meshlet_bgl,
        wgpu::TextureFormat::Depth32Float,
        (256, 256),
        None,
    );

    // Force the device to surface any deferred validation work before
    // we look at the trap. Pipeline creation in wgpu 29 may queue work
    // until the next poll.
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });

    drop(stage);

    let captured = errors.lock().expect("poisoned");
    assert!(
        captured.is_empty(),
        "Vbuf64Stage construction raised wgpu validation errors:\n  - {}",
        captured.join("\n  - ")
    );
}
