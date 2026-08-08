use super::*;

#[test]
fn bucket_size_rounds_up() {
    assert_eq!(bucket_size(1), 256);
    assert_eq!(bucket_size(100), 256);
    assert_eq!(bucket_size(256), 256);
    assert_eq!(bucket_size(257), 512);
    assert_eq!(bucket_size(1024), 1024);
    assert_eq!(bucket_size(1025), 2048);
}

#[test]
fn pool_starts_empty() {
    let pool = BufferPool::new();
    assert_eq!(pool.held_count(), 0);
}

#[test]
fn pool_clear_empties() {
    let mut pool = BufferPool::new();
    // No actual buffers to insert without a device, but clear shouldn't panic.
    pool.clear();
    assert_eq!(pool.held_count(), 0);
}

fn create_headless_device() -> wgpu::Device {
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

    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("test_device"),
        ..Default::default()
    }))
    .expect("failed to create device");

    device
}

#[test]
#[ignore] // Requires GPU hardware.
fn pool_reuse_returns_same_bucket() {
    let device = create_headless_device();
    let mut pool = BufferPool::new();
    let usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;

    // Get a buffer, return it, get another of the same size.
    let buf = pool.get_or_create(&device, 100, usage);
    assert_eq!(pool.held_count(), 0);

    pool.return_buffer(buf, 100, usage);
    assert_eq!(pool.held_count(), 1);

    // Should reuse the returned buffer.
    let _reused = pool.get_or_create(&device, 100, usage);
    assert_eq!(pool.held_count(), 0);
}

#[test]
#[ignore] // Requires GPU hardware.
fn pool_different_usage_no_reuse() {
    let device = create_headless_device();
    let mut pool = BufferPool::new();

    let buf = pool.get_or_create(&device, 256, BufferUsages::STORAGE);
    pool.return_buffer(buf, 256, BufferUsages::STORAGE);
    assert_eq!(pool.held_count(), 1);

    // Different usage — should NOT reuse.
    let _new = pool.get_or_create(&device, 256, BufferUsages::UNIFORM);
    assert_eq!(pool.held_count(), 1); // original still in pool

    pool.clear();
    assert_eq!(pool.held_count(), 0);
}
