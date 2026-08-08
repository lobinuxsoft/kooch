use wgpu::{DeviceDescriptor, Instance, InstanceDescriptor, RequestAdapterOptions};

use super::*;

#[test]
#[ignore] // Requires GPU hardware.
fn vector_add_pipeline_creation() {
    let instance = Instance::new(InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });

    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("no GPU adapter found");

    let (device, _queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
        label: Some("test_device"),
        ..Default::default()
    }))
    .expect("failed to create device");

    // Should not panic.
    let _compute = VectorAddCompute::new(&device, None);
}
