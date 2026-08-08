use super::*;

/// Acquires a headless device with default features/limits. Returns
/// `None` when no adapter is available (CI without a GPU) so the test
/// skips rather than fails, matching the crate's other GPU tests.
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
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("material_texture_pool_test_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

fn checker(format: ImageFormat) -> Image {
    Image::solid_color([200, 100, 50, 255], format)
}

#[test]
fn register_and_lookup_roundtrip() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut pool = MaterialTexturePool::new(&device, &queue);
    assert!(pool.is_empty());

    let g = Guid::new_v4();
    assert!(!pool.contains(g));
    pool.register(&device, &queue, g, &checker(ImageFormat::Rgba8UnormSrgb));
    assert!(pool.contains(g));
    assert_eq!(pool.len(), 1);

    // Re-register same GUID replaces, does not grow.
    pool.register(&device, &queue, g, &checker(ImageFormat::Rgba8UnormSrgb));
    assert_eq!(pool.len(), 1);
}

#[test]
fn material_bind_group_builds_with_and_without_textures() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut pool = MaterialTexturePool::new(&device, &queue);
    let albedo = Guid::new_v4();
    pool.register(
        &device,
        &queue,
        albedo,
        &checker(ImageFormat::Rgba8UnormSrgb),
    );

    // All-fallback (no maps) must build against the same layout.
    let _bg_none = pool.material_bind_group(&device, None, None, None);
    // Mixed: real albedo, fallback normal + metal_roughness.
    let _bg_mixed = pool.material_bind_group(&device, Some(albedo), None, None);
    // Unregistered GUID silently falls back.
    let _bg_missing = pool.material_bind_group(&device, Some(Guid::new_v4()), None, None);
}
