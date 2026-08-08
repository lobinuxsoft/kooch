use super::*;

use std::sync::OnceLock;

// Shared device per test binary — see issue #334. Mesa radv races
// when many threads invoke `request_adapter` concurrently.
static SHARED_DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();

fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    SHARED_DEVICE
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
                flags: wgpu::InstanceFlags::default(),
                backend_options: wgpu::BackendOptions::default(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                display: None,
            });
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                }))
                .ok()?;
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("material_pipeline_test_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            }))
            .ok()
        })
        .clone()
}

#[test]
fn lookup_or_fallback_returns_zero_for_unknown_guid() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let pipeline = MaterialPipeline::new(&device, &queue);
    assert_eq!(pipeline.lookup_or_fallback(None), FALLBACK_MATERIAL_ID);
    assert_eq!(
        pipeline.lookup_or_fallback(Some(Guid::new_v4())),
        FALLBACK_MATERIAL_ID,
    );
}

#[test]
fn register_assigns_distinct_slots_starting_at_one() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let mut pipeline = MaterialPipeline::new(&device, &queue);
    let g1 = Guid::new_v4();
    let g2 = Guid::new_v4();
    let s1 = pipeline.register(&queue, g1, &Material::default());
    let s2 = pipeline.register(&queue, g2, &Material::default());
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
    assert_eq!(pipeline.lookup(g1), Some(1));
    assert_eq!(pipeline.lookup(g2), Some(2));
    assert_eq!(pipeline.registered_count(), 2);
}

#[test]
fn register_is_idempotent_on_same_guid() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let mut pipeline = MaterialPipeline::new(&device, &queue);
    let g = Guid::new_v4();
    let s0 = pipeline.register(&queue, g, &Material::default());
    let s1 = pipeline.register(
        &queue,
        g,
        &Material::new([1.0, 0.0, 0.0, 1.0], 0.0, 0.5, 0.0),
    );
    assert_eq!(s0, s1, "same GUID must reuse the same slot");
    assert_eq!(pipeline.registered_count(), 1);
}

#[test]
fn register_falls_back_when_capacity_exhausted() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    // capacity = 2 → slot 0 fallback + slot 1 = the only spawnable slot.
    let mut pipeline = MaterialPipeline::with_capacity(&device, &queue, 2);
    let g1 = Guid::new_v4();
    let g2 = Guid::new_v4();
    let s1 = pipeline.register(&queue, g1, &Material::default());
    let s2 = pipeline.register(&queue, g2, &Material::default());
    assert_eq!(s1, 1, "first registration takes slot 1");
    assert_eq!(
        s2, FALLBACK_MATERIAL_ID,
        "overflowing registration falls back to slot 0",
    );
}

#[test]
fn register_records_texture_refs_per_slot() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let mut pipeline = MaterialPipeline::new(&device, &queue);
    let (albedo, mr) = (Guid::new_v4(), Guid::new_v4());
    let mat = Material::default()
        .with_albedo(albedo)
        .with_metal_roughness(mr);
    let slot = pipeline.register(&queue, Guid::new_v4(), &mat);

    assert_eq!(
        pipeline.slot_texture_refs(slot),
        [Some(albedo), None, Some(mr)]
    );
    // Fallback slot 0 and out-of-range slots reference nothing.
    assert_eq!(pipeline.slot_texture_refs(FALLBACK_MATERIAL_ID), [None; 3]);
    assert_eq!(pipeline.slot_texture_refs(999), [None; 3]);
    // Two shading passes to issue: fallback slot 0 + registered slot 1.
    assert_eq!(pipeline.shading_slots(), 0..2);
}
