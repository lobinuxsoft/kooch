use super::*;

fn create_headless_device() -> (Device, Queue) {
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
    .expect("no GPU adapter");

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("test_device"),
        ..Default::default()
    }))
    .expect("failed to create device")
}

#[test]
#[ignore] // Requires GPU hardware.
fn staging_readback_roundtrip() {
    use wgpu::util::DeviceExt;

    let (device, queue) = create_headless_device();

    let data: Vec<f32> = (0..64).map(|i| i as f32 * 0.5).collect();
    let byte_size = (data.len() * std::mem::size_of::<f32>()) as u64;

    // Create source buffer with data.
    let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("source"),
        contents: bytemuck::cast_slice(&data),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    });

    let staging = StagingBuffer::new(&device, byte_size);
    assert_eq!(staging.size(), byte_size);

    let result: Vec<f32> = staging.read_buffer(&device, &queue, &source);
    assert_eq!(result, data);
}

#[test]
#[ignore] // Requires GPU hardware.
fn staging_manual_copy_and_readback() {
    use wgpu::util::DeviceExt;

    let (device, queue) = create_headless_device();

    let data = [42u32, 7, 13, 99];
    let byte_size = std::mem::size_of_val(&data) as u64;

    let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("source"),
        contents: bytemuck::cast_slice(&data),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    });

    let staging = StagingBuffer::new(&device, byte_size);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test"),
    });
    staging.copy_from(&mut encoder, &source);
    queue.submit(std::iter::once(encoder.finish()));

    let result: Vec<u32> = staging.read_back(&device);
    assert_eq!(result, vec![42, 7, 13, 99]);
}
