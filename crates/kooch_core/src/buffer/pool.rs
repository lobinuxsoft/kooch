//! Buffer pool with power-of-two bucketing for buffer reuse.
//!
//! Temporary GPU buffers (e.g. per-frame staging or scratch buffers) are
//! expensive to allocate. [`BufferPool`] recycles them using size buckets
//! rounded up to the next power of two (minimum 256 bytes).
//!
//! # When to consider `gpu-allocator`
//!
//! This pool is a simple free-list. Replace it with a sub-allocator like
//! [`gpu-allocator`](https://crates.io/crates/gpu-allocator) when:
//! - The engine needs hundreds of short-lived buffers per frame.
//! - Memory fragmentation becomes measurable.
//! - You need memory type control (e.g. dedicated vs shared heaps).

use std::collections::HashMap;

use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device};

/// Minimum bucket size in bytes (avoids tiny allocations).
const MIN_BUCKET_SIZE: u64 = 256;

/// Recycles GPU buffers by rounding requested sizes to power-of-two buckets.
///
/// Buffers are keyed by `(bucket_size, usage)`.
#[derive(Default)]
pub struct BufferPool {
    pools: HashMap<(u64, BufferUsages), Vec<Buffer>>,
}

impl BufferPool {
    /// Creates an empty pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a buffer of at least `size` bytes with the given `usage`,
    /// reusing a pooled buffer if one is available.
    pub fn get_or_create(&mut self, device: &Device, size: u64, usage: BufferUsages) -> Buffer {
        let bucket = bucket_size(size);
        let key = (bucket, usage);

        if let Some(buf) = self.pools.get_mut(&key).and_then(|v| v.pop()) {
            return buf;
        }

        device.create_buffer(&BufferDescriptor {
            label: Some("pooled_buffer"),
            size: bucket,
            usage,
            mapped_at_creation: false,
        })
    }

    /// Returns a buffer to the pool for future reuse.
    ///
    /// `size` must be the **original requested size** (not the bucket size) —
    /// the pool will round it to the correct bucket internally.
    pub fn return_buffer(&mut self, buffer: Buffer, size: u64, usage: BufferUsages) {
        let bucket = bucket_size(size);
        let key = (bucket, usage);
        self.pools.entry(key).or_default().push(buffer);
    }

    /// Total number of buffers currently held in the pool.
    pub fn held_count(&self) -> usize {
        self.pools.values().map(|v| v.len()).sum()
    }

    /// Drops all pooled buffers, releasing GPU memory.
    pub fn clear(&mut self) {
        self.pools.clear();
    }
}

/// Rounds `size` up to the next power of two, with a minimum of
/// [`MIN_BUCKET_SIZE`] (256 bytes).
pub fn bucket_size(size: u64) -> u64 {
    let min = size.max(MIN_BUCKET_SIZE);
    min.next_power_of_two()
}

#[cfg(test)]
mod tests {
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

        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
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
}
