use wgpu::DeviceDescriptor;

use super::*;

#[test]
#[ignore] // Requires GPU hardware.
fn vector_add_pipeline_creation() {
    let adapter = crate::gpu::test_adapter().expect("no GPU adapter found");

    let (device, _queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
        label: Some("test_device"),
        ..Default::default()
    }))
    .expect("failed to create device");

    // Should not panic.
    let _compute = VectorAddCompute::new(&device, None);
}
