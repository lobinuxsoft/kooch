use wgpu::{Instance, InstanceDescriptor, RequestAdapterOptions};

// Display/Debug/Error::source impls for `GpuError` are exhaustive
// match arms over wgpu-typed payloads, so their correctness is
// proven by compilation. Synthetic-instance tests removed when wgpu
// 29 made `RequestAdapterError` non-constructible from user code
// (issue #218). The headless smoke test below still exercises the
// happy path with a real adapter.

#[test]
#[ignore] // Requires GPU hardware.
fn create_adapter_headless() {
    let instance = Instance::new(InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });

    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }));

    assert!(adapter.is_ok(), "expected a GPU adapter to be available");

    let info = adapter.unwrap().get_info();
    println!("Adapter: {} ({:?})", info.name, info.backend);
}
