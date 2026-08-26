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
mod tests;
