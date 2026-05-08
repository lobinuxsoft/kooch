use bytemuck::Zeroable;
use ome_core::buffer::GpuBuffer;

use super::storage::GpuComponentStorage;
use crate::component::traits::{AnyStorage, GpuComponent};
use crate::entity::Entity;

// A simple Pod type for testing.
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct TestTransform {
    x: f32,
    y: f32,
    z: f32,
    _pad: f32,
}

impl GpuComponent for TestTransform {}

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn insert_and_get() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    let e = entity(0);
    let val = TestTransform { x: 1.0, y: 2.0, z: 3.0, _pad: 0.0 };

    storage.insert(e, val);
    assert_eq!(storage.get(e), Some(&val));
    assert_eq!(storage.len(), 1);
    assert!(storage.contains(e));
}

#[test]
fn insert_overwrites() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    let e = entity(0);
    let v1 = TestTransform { x: 1.0, y: 0.0, z: 0.0, _pad: 0.0 };
    let v2 = TestTransform { x: 9.0, y: 0.0, z: 0.0, _pad: 0.0 };

    storage.insert(e, v1);
    storage.insert(e, v2);
    assert_eq!(storage.get(e), Some(&v2));
    // Count should still be 1 (overwrite, not new insert).
    assert_eq!(storage.len(), 1);
}

#[test]
fn remove_zeros_slot() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    let e = entity(0);
    let val = TestTransform { x: 1.0, y: 2.0, z: 3.0, _pad: 0.0 };

    storage.insert(e, val);
    let old = storage.remove(e);
    assert_eq!(old, Some(val));
    assert!(!storage.contains(e));
    assert_eq!(storage.len(), 0);

    // Underlying data should be zeroed.
    assert_eq!(storage.data[0], TestTransform::zeroed());
}

#[test]
fn remove_nonexistent_returns_none() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    assert_eq!(storage.remove(entity(42)), None);
}

#[test]
fn get_mut_marks_dirty() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    let e = entity(5);
    let val = TestTransform { x: 1.0, y: 0.0, z: 0.0, _pad: 0.0 };

    storage.insert(e, val);
    // Clear dirty state.
    storage.dirty_min = None;
    storage.dirty_max = None;

    let _m = storage.get_mut(e).unwrap();
    assert_eq!(storage.dirty_min, Some(5));
    assert_eq!(storage.dirty_max, Some(5));
}

#[test]
fn auto_grows_on_high_index() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    let e = entity(100);
    let val = TestTransform { x: 1.0, y: 0.0, z: 0.0, _pad: 0.0 };

    storage.insert(e, val);
    assert!(storage.data.len() >= 101);
    assert_eq!(storage.get(e), Some(&val));
}

#[test]
fn dirty_range_tracks_min_max() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    let zero = TestTransform::zeroed();

    storage.insert(entity(10), zero);
    storage.insert(entity(5), zero);
    storage.insert(entity(20), zero);

    assert_eq!(storage.dirty_min, Some(5));
    assert_eq!(storage.dirty_max, Some(20));
}

#[test]
fn is_empty_when_no_components() {
    let storage = GpuComponentStorage::<TestTransform>::new("test");
    assert!(storage.is_empty());
}

#[test]
fn any_storage_remove_entity() {
    let mut storage = GpuComponentStorage::<TestTransform>::new("test");
    let e = entity(0);
    let val = TestTransform { x: 1.0, y: 0.0, z: 0.0, _pad: 0.0 };

    storage.insert(e, val);
    assert_eq!(storage.len(), 1);

    // Use the type-erased interface.
    let any_storage: &mut dyn AnyStorage = &mut storage;
    any_storage.remove_entity(e);
    assert_eq!(storage.len(), 0);
}

// -- GPU integration tests (require hardware) --

fn create_headless_device() -> (wgpu::Device, wgpu::Queue) {
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
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .expect("failed to create device")
}

fn readback_buffer<T: bytemuck::Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu_buf: &GpuBuffer<T>,
    count: usize,
) -> Vec<T> {
    use ome_core::buffer::StagingBuffer;

    let byte_size = (count * std::mem::size_of::<T>()) as u64;
    let staging = StagingBuffer::new(device, byte_size);
    staging.read_buffer(device, queue, gpu_buf.buffer())
}

#[test]
#[ignore] // Requires GPU hardware.
fn sync_gpu_creates_buffer_and_uploads() {
    let (device, queue) = create_headless_device();
    let mut storage = GpuComponentStorage::<TestTransform>::new("test_sync");

    let e0 = entity(0);
    let e1 = entity(1);
    let v0 = TestTransform { x: 1.0, y: 2.0, z: 3.0, _pad: 0.0 };
    let v1 = TestTransform { x: 4.0, y: 5.0, z: 6.0, _pad: 0.0 };

    storage.insert(e0, v0);
    storage.insert(e1, v1);

    assert!(storage.gpu_buffer().is_none());

    // First sync — creates buffer.
    storage.sync_gpu(&device, &queue, 4);
    assert!(storage.gpu_buffer().is_some());

    let data = readback_buffer(&device, &queue, storage.gpu_buffer().unwrap(), 4);
    assert_eq!(data[0], v0);
    assert_eq!(data[1], v1);
    assert_eq!(data[2], TestTransform::zeroed());
    assert_eq!(data[3], TestTransform::zeroed());
}

#[test]
#[ignore] // Requires GPU hardware.
fn sync_gpu_partial_dirty_upload() {
    let (device, queue) = create_headless_device();
    let mut storage = GpuComponentStorage::<TestTransform>::new("test_dirty");

    let v0 = TestTransform { x: 1.0, y: 0.0, z: 0.0, _pad: 0.0 };
    storage.insert(entity(0), v0);
    storage.insert(entity(1), v0);
    storage.insert(entity(2), v0);

    // Initial sync.
    storage.sync_gpu(&device, &queue, 4);

    // Modify only entity 1.
    let v_new = TestTransform { x: 99.0, y: 0.0, z: 0.0, _pad: 0.0 };
    *storage.get_mut(entity(1)).unwrap() = v_new;

    // Partial sync — only dirty range uploaded.
    storage.sync_gpu(&device, &queue, 4);

    let data = readback_buffer(&device, &queue, storage.gpu_buffer().unwrap(), 4);
    assert_eq!(data[0], v0);
    assert_eq!(data[1], v_new);
    assert_eq!(data[2], v0);
}

#[test]
#[ignore] // Requires GPU hardware.
fn sync_gpu_grow_re_uploads_all() {
    let (device, queue) = create_headless_device();
    let mut storage = GpuComponentStorage::<TestTransform>::new("test_grow");

    let v0 = TestTransform { x: 1.0, y: 2.0, z: 3.0, _pad: 0.0 };
    storage.insert(entity(0), v0);

    // Initial sync with capacity 2.
    storage.sync_gpu(&device, &queue, 2);
    assert_eq!(storage.gpu_buffer().unwrap().capacity(), 2);

    // Grow to capacity 8 — should re-upload everything.
    storage.sync_gpu(&device, &queue, 8);
    assert_eq!(storage.gpu_buffer().unwrap().capacity(), 8);

    let data = readback_buffer(&device, &queue, storage.gpu_buffer().unwrap(), 8);
    assert_eq!(data[0], v0);
    // Rest should be zeroed.
    for i in 1..8 {
        assert_eq!(data[i], TestTransform::zeroed());
    }
}

#[test]
#[ignore] // Requires GPU hardware.
fn sync_gpu_remove_zeros_on_gpu() {
    let (device, queue) = create_headless_device();
    let mut storage = GpuComponentStorage::<TestTransform>::new("test_remove_gpu");

    let v0 = TestTransform { x: 42.0, y: 0.0, z: 0.0, _pad: 0.0 };
    storage.insert(entity(0), v0);
    storage.sync_gpu(&device, &queue, 2);

    // Remove and sync — slot 0 should be zeroed on GPU.
    storage.remove(entity(0));
    storage.sync_gpu(&device, &queue, 2);

    let data = readback_buffer(&device, &queue, storage.gpu_buffer().unwrap(), 2);
    assert_eq!(data[0], TestTransform::zeroed());
}
