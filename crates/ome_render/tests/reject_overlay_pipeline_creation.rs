//! Integration test that exercises `MeshletRejectOverlay::new` on a
//! real device and asserts no uncaptured wgpu errors are raised during
//! pipeline creation. Catches "ComputePipeline … is invalid" failures
//! that surface only at submit time on the editor — the validation
//! error is reported asynchronously via `on_uncaptured_error` and the
//! pipeline is then a black hole.

use ome_render::meshlet::{MeshletCull, MeshletRejectOverlay, DEFAULT_MAX_TRIANGLES};
use std::sync::{Arc, Mutex};

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
    let needed = wgpu::Features::TEXTURE_ATOMIC
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(needed) {
        return None;
    }

    let mut limits = wgpu::Limits::default();
    limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);

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
    let Some((device, _queue)) = try_acquire_device() else {
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
    let _overlay = MeshletRejectOverlay::new(&device, &cull);

    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });

    let captured = errors.lock().expect("poisoned");
    assert!(
        captured.is_empty(),
        "MeshletRejectOverlay construction raised wgpu validation errors:\n  - {}",
        captured.join("\n  - ")
    );
}
