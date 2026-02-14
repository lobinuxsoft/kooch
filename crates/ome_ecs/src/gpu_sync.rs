//! GPU synchronisation of the entity alive mask.
//!
//! [`EntityGpuState`] owns a `wgpu::Buffer` that mirrors the CPU-side
//! alive flags as a `StorageBuffer<u32>` (one `u32` per slot, `0` or `1`).
//!
//! The [`entity_gpu_sync_system`] runs in [`Stage::GpuSync`] and uploads
//! only the changed slots each frame.

use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use wgpu::util::DeviceExt;
use wgpu::{Buffer, BufferUsages, Device, Queue};

use crate::allocator::EntityAllocator;

/// GPU-side alive mask state.
///
/// Holds the `wgpu::Buffer` that shaders read from and a CPU-side shadow
/// copy used to compute partial updates.
pub struct EntityGpuState {
    alive_buffer: Buffer,
    alive_data: Vec<u32>,
    capacity: u32,
}

impl EntityGpuState {
    /// Creates a new GPU state with `capacity` slots, all initialised to `0`.
    pub fn new(device: &Device, capacity: u32) -> Self {
        let alive_data = vec![0u32; capacity as usize];

        let alive_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("entity_alive_mask"),
            contents: bytemuck::cast_slice(&alive_data),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        });

        Self {
            alive_buffer,
            alive_data,
            capacity,
        }
    }

    /// Grows the buffer to `new_capacity`, preserving existing data.
    ///
    /// Allocates a new buffer and copies the old CPU data into it.
    pub fn grow(&mut self, device: &Device, new_capacity: u32) {
        self.alive_data.resize(new_capacity as usize, 0);

        self.alive_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("entity_alive_mask"),
            contents: bytemuck::cast_slice(&self.alive_data),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        });

        self.capacity = new_capacity;
    }

    /// Applies a set of `(index, 0|1)` updates to the CPU shadow copy and
    /// uploads the affected byte range to the GPU buffer.
    pub fn apply_updates(&mut self, queue: &Queue, updates: &[(u32, u32)]) {
        if updates.is_empty() {
            return;
        }

        let mut min_idx = u32::MAX;
        let mut max_idx = 0u32;

        for &(idx, val) in updates {
            self.alive_data[idx as usize] = val;
            min_idx = min_idx.min(idx);
            max_idx = max_idx.max(idx);
        }

        // Upload the contiguous range [min..=max].
        let byte_offset = (min_idx as u64) * std::mem::size_of::<u32>() as u64;
        let slice = &self.alive_data[min_idx as usize..=max_idx as usize];
        queue.write_buffer(&self.alive_buffer, byte_offset, bytemuck::cast_slice(slice));
    }

    /// Returns a reference to the GPU buffer.
    #[inline]
    pub fn alive_buffer(&self) -> &Buffer {
        &self.alive_buffer
    }

    /// Returns the current capacity (number of slots).
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// System that synchronises `EntityAllocator` changes to the GPU alive mask.
///
/// Runs once per frame in [`Stage::GpuSync`].
///
/// - **No `GpuContext`?** — silently drains pending changes (headless mode).
/// - **No `EntityAllocator`?** — no-op.
/// - **No pending changes?** — skips the GPU upload entirely.
pub fn entity_gpu_sync_system(resources: &mut Resources) {
    // 1. Drain pending indices from the allocator (mut borrow, scoped).
    let Some(pending) = resources
        .get_mut::<EntityAllocator>()
        .map(|alloc| alloc.take_pending_sync())
    else {
        return;
    };

    if pending.is_empty() {
        return;
    }

    // 2. Build (index, alive) update pairs (immutable borrow, scoped).
    let updates: Vec<(u32, u32)> = {
        let alloc = resources.get::<EntityAllocator>().unwrap();
        pending
            .iter()
            .map(|&idx| {
                let val = if alloc.is_index_alive(idx) { 1 } else { 0 };
                (idx, val)
            })
            .collect()
    };

    let allocator_slots = resources.get::<EntityAllocator>().unwrap().total_slots();

    // 3. Remove EntityGpuState from resources to avoid borrow conflicts
    //    with GpuContext.
    let mut gpu_state: Option<EntityGpuState> = resources.remove::<EntityGpuState>();

    // 4. Lazy-init / grow / upload.
    if let Some(gpu) = resources.get::<GpuContext>() {
        let state = gpu_state.get_or_insert_with(|| {
            tracing::debug!(capacity = allocator_slots, "EntityGpuState lazy-init");
            EntityGpuState::new(gpu.device(), allocator_slots)
        });

        if allocator_slots > state.capacity() {
            tracing::debug!(
                old = state.capacity(),
                new = allocator_slots,
                "EntityGpuState growing"
            );
            state.grow(gpu.device(), allocator_slots);
        }

        state.apply_updates(gpu.queue(), &updates);
    }

    // 5. Re-insert state if it exists.
    if let Some(state) = gpu_state {
        resources.insert(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_panic_without_gpu_context() {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::with_capacity(4));

        // Spawn an entity so there are pending changes.
        resources.get_mut::<EntityAllocator>().unwrap().spawn();

        // Should not panic — headless mode drains pending silently.
        entity_gpu_sync_system(&mut resources);

        // Pending should be drained.
        let alloc = resources.get_mut::<EntityAllocator>().unwrap();
        assert!(alloc.take_pending_sync().is_empty());
    }

    #[test]
    fn no_panic_without_allocator() {
        let mut resources = Resources::new();
        // No allocator inserted — system is a no-op.
        entity_gpu_sync_system(&mut resources);
    }

    #[test]
    fn no_op_with_empty_pending() {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::with_capacity(4));

        // No spawns → nothing pending.
        entity_gpu_sync_system(&mut resources);

        // Allocator still there, no EntityGpuState created (no GPU).
        assert!(resources.get::<EntityGpuState>().is_none());
    }
}
