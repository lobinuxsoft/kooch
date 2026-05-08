//! [`GpuComponentStorage`] struct, inherent impl, and [`AnyStorage`] impl.

use std::any::Any;

use ome_core::buffer::GpuBuffer;
use wgpu::{BufferUsages, Device, Queue};

use crate::component::traits::{AnyStorage, GpuComponent};
use crate::entity::Entity;

/// Dense component storage backed by a GPU buffer.
///
/// - **CPU side:** `Vec<T>` shadow copy indexed by `entity.index()`.
/// - **GPU side:** lazy `GpuBuffer<T>` created on first [`sync_gpu`](AnyStorage::sync_gpu).
/// - **Dirty tracking:** range-based (min/max) for partial uploads.
pub struct GpuComponentStorage<T: GpuComponent> {
    pub(super) data: Vec<T>,
    pub(super) present: Vec<bool>,
    pub(super) gpu_buffer: Option<GpuBuffer<T>>,
    pub(super) dirty_min: Option<u32>,
    pub(super) dirty_max: Option<u32>,
    pub(super) count: u32,
    pub(super) label: String,
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

    fn contains_entity(&self, entity: Entity) -> bool {
        self.contains(entity)
    }

    fn get_ptr(&self, entity: Entity) -> Option<*const u8> {
        self.get(entity).map(|v| v as *const T as *const u8)
    }

    fn get_mut_ptr(&mut self, _entity: Entity) -> Option<*mut u8> {
        // GPU components are read-only from CPU side in query context.
        // Mutations go through GpuComponentStorage::get_mut() which tracks dirty state.
        None
    }

    fn is_mutable(&self) -> bool {
        false
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
