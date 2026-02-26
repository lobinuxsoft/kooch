//! Dense GPU-backed component storage with CPU shadow copy.
//!
//! [`GpuComponentStorage<T>`] stores component data in a `Vec<T>` indexed by
//! `entity.index()`, with a lazy [`GpuBuffer<T>`] that is created on first
//! sync. Dirty tracking uses a min/max range for efficient partial uploads.

use std::any::Any;

use ome_core::buffer::GpuBuffer;
use wgpu::{BufferUsages, Device, Queue};

use crate::entity::Entity;

use super::traits::{AnyStorage, GpuComponent};

/// Dense component storage backed by a GPU buffer.
///
/// - **CPU side:** `Vec<T>` shadow copy indexed by `entity.index()`.
/// - **GPU side:** lazy `GpuBuffer<T>` created on first [`sync_gpu`](AnyStorage::sync_gpu).
/// - **Dirty tracking:** range-based (min/max) for partial uploads.
pub struct GpuComponentStorage<T: GpuComponent> {
    data: Vec<T>,
    present: Vec<bool>,
    gpu_buffer: Option<GpuBuffer<T>>,
    dirty_min: Option<u32>,
    dirty_max: Option<u32>,
    count: u32,
    label: String,
}

impl<T: GpuComponent> GpuComponentStorage<T> {
    /// Creates empty storage with no GPU buffer.
    pub fn new(label: &str) -> Self {
        Self {
            data: Vec::new(),
            present: Vec::new(),
            gpu_buffer: None,
            dirty_min: None,
            dirty_max: None,
            count: 0,
            label: label.to_string(),
        }
    }

    /// Inserts a component value for `entity`, overwriting any previous value.
    pub fn insert(&mut self, entity: Entity, value: T) {
        let idx = entity.index() as usize;
        self.ensure_capacity((idx + 1) as u32);

        if !self.present[idx] {
            self.count += 1;
        }

        self.data[idx] = value;
        self.present[idx] = true;
        self.mark_dirty(entity.index());
    }

    /// Removes the component for `entity`, zeroing the slot.
    ///
    /// Returns the previous value if the entity had this component.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let idx = entity.index() as usize;
        if idx >= self.present.len() || !self.present[idx] {
            return None;
        }

        let old = self.data[idx];
        self.data[idx] = T::zeroed();
        self.present[idx] = false;
        self.count -= 1;
        self.mark_dirty(entity.index());

        Some(old)
    }

    /// Returns an immutable reference to the component, if present.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        let idx = entity.index() as usize;
        if idx < self.present.len() && self.present[idx] {
            Some(&self.data[idx])
        } else {
            None
        }
    }

    /// Returns a mutable reference to the component, if present.
    ///
    /// Marks the slot as dirty even if the caller doesn't modify the value.
    /// This is a deliberate trade-off (Bevy does the same).
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        let idx = entity.index() as usize;
        if idx < self.present.len() && self.present[idx] {
            self.mark_dirty(entity.index());
            Some(&mut self.data[idx])
        } else {
            None
        }
    }

    /// Returns `true` if this storage has a component for `entity`.
    pub fn contains(&self, entity: Entity) -> bool {
        let idx = entity.index() as usize;
        idx < self.present.len() && self.present[idx]
    }

    /// Returns a reference to the underlying GPU buffer, if created.
    pub fn gpu_buffer(&self) -> Option<&GpuBuffer<T>> {
        self.gpu_buffer.as_ref()
    }

    /// Number of entities with this component.
    #[inline]
    pub fn len(&self) -> u32 {
        self.count
    }

    /// Returns `true` if no entities have this component.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Grows the CPU-side vectors to hold at least `min_capacity` elements.
    fn ensure_capacity(&mut self, min_capacity: u32) {
        let min = min_capacity as usize;
        if min > self.data.len() {
            self.data.resize(min, T::zeroed());
            self.present.resize(min, false);
        }
    }

    /// Expands the dirty range to include `index`.
    fn mark_dirty(&mut self, index: u32) {
        self.dirty_min = Some(self.dirty_min.map_or(index, |m| m.min(index)));
        self.dirty_max = Some(self.dirty_max.map_or(index, |m| m.max(index)));
    }
}

impl<T: GpuComponent> AnyStorage for GpuComponentStorage<T> {
    fn remove_entity(&mut self, entity: Entity) {
        self.remove(entity);
    }

    fn sync_gpu(&mut self, device: &Device, queue: &Queue, capacity: u32) {
        // 1. Resize CPU vecs to match allocator capacity.
        self.ensure_capacity(capacity);

        let cap = capacity as u64;
        let usage = BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC;

        if self.gpu_buffer.is_none() {
            // 2. First sync — create buffer and do a full upload.
            let mut buf = GpuBuffer::with_capacity(device, &self.label, cap, usage);
            buf.write(queue, &self.data);
            self.gpu_buffer = Some(buf);
            self.dirty_min = None;
            self.dirty_max = None;
            return;
        }

        let buf = self.gpu_buffer.as_mut().unwrap();

        if buf.capacity() < cap {
            // 3. Allocator grew — reallocate and full re-upload.
            buf.grow(device, &self.label, cap);
            buf.write(queue, &self.data);
            self.dirty_min = None;
            self.dirty_max = None;
            return;
        }

        // 4. Partial upload of dirty range.
        if let (Some(min), Some(max)) = (self.dirty_min, self.dirty_max) {
            let slice = &self.data[min as usize..=max as usize];
            buf.write_offset(queue, min as u64, slice);
            self.dirty_min = None;
            self.dirty_max = None;
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

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
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("no GPU adapter");

        pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("test_device"),
                ..Default::default()
            },
            None,
        ))
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
}
