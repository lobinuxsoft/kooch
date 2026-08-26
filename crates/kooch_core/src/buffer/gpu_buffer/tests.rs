use super::*;

#[test]
fn elem_size_f32() {
    assert_eq!(GpuBuffer::<f32>::ELEM_SIZE, 4);
}

#[test]
fn byte_size_calculation() {
    // byte_size = capacity * elem_size, verified without GPU.
    // We test the formula directly since we can't create a Device.
    let capacity: u64 = 128;
    let expected = capacity * std::mem::size_of::<f32>() as u64;
    assert_eq!(expected, 512);
}

// -- GPU tests (require hardware) --

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
fn with_capacity_creates_empty_buffer() {
    let (device, _queue) = create_headless_device();

    let buf = GpuBuffer::<f32>::with_capacity(
        &device,
        "test",
        64,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
    );

    assert_eq!(buf.capacity(), 64);
    assert_eq!(buf.len(), 0);
    assert!(buf.is_empty());
    assert_eq!(buf.byte_size(), 64 * 4);
}

#[test]
#[ignore] // Requires GPU hardware.
fn from_data_and_readback() {
    let (device, queue) = create_headless_device();

    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let buf = GpuBuffer::<f32>::from_data(
        &device,
        "test",
        &data,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    );

    assert_eq!(buf.len(), 16);
    assert_eq!(buf.capacity(), 16);

    let staging = super::super::StagingBuffer::new(&device, buf.byte_size());
    let result: Vec<f32> = staging.read_buffer(&device, &queue, buf.buffer());
    assert_eq!(result, data);
}

#[test]
#[ignore] // Requires GPU hardware.
fn write_and_readback() {
    let (device, queue) = create_headless_device();

    let mut buf = GpuBuffer::<u32>::with_capacity(
        &device,
        "test",
        8,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    );

    let data = [10u32, 20, 30, 40];
    buf.write(&queue, &data);
    assert_eq!(buf.len(), 4);

    let staging = super::super::StagingBuffer::new(&device, 4 * 4);
    // Read only the first 4 elements (16 bytes).
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test"),
    });
    encoder.copy_buffer_to_buffer(buf.buffer(), 0, staging.buffer(), 0, 16);
    queue.submit(std::iter::once(encoder.finish()));

    let result: Vec<u32> = staging.read_back(&device);
    assert_eq!(result, vec![10, 20, 30, 40]);
}

#[test]
#[ignore] // Requires GPU hardware.
fn write_offset_partial() {
    let (device, queue) = create_headless_device();

    let initial = [1u32, 2, 3, 4, 5, 6, 7, 8];
    let buf = GpuBuffer::<u32>::from_data(
        &device,
        "test",
        &initial,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    );

    // Overwrite elements [2..4] with [99, 100].
    buf.write_offset(&queue, 2, &[99u32, 100]);

    let staging = super::super::StagingBuffer::new(&device, buf.byte_size());
    let result: Vec<u32> = staging.read_buffer(&device, &queue, buf.buffer());
    assert_eq!(result, vec![1, 2, 99, 100, 5, 6, 7, 8]);
}

#[test]
#[ignore] // Requires GPU hardware.
fn grow_increases_capacity() {
    let (device, queue) = create_headless_device();

    let mut buf = GpuBuffer::<f32>::with_capacity(
        &device,
        "test",
        4,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    );

    assert_eq!(buf.capacity(), 4);
    buf.grow(&device, "test", 32);
    assert_eq!(buf.capacity(), 32);
    assert_eq!(buf.len(), 0); // len reset after grow
    assert_eq!(buf.byte_size(), 32 * 4);

    // Write new data into the grown buffer.
    let data: Vec<f32> = (0..32).map(|i| i as f32).collect();
    buf.write(&queue, &data);
    assert_eq!(buf.len(), 32);

    let staging = super::super::StagingBuffer::new(&device, buf.byte_size());
    let result: Vec<f32> = staging.read_buffer(&device, &queue, buf.buffer());
    assert_eq!(result, data);
}
