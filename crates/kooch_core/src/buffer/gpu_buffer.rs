//! Typed GPU buffer abstraction with Vec-like semantics.
//!
//! [`GpuBuffer<T>`] wraps a `wgpu::Buffer` and tracks `len` / `capacity`
//! in elements of type `T` (which must be [`Pod`] for safe byte reinterpretation).
//!
//! # Alignment
//!
//! `wgpu::Queue::write_buffer` requires offsets aligned to
//! [`COPY_BUFFER_ALIGNMENT`](wgpu::COPY_BUFFER_ALIGNMENT) (4 bytes).
//! Types smaller than 4 bytes will work for full writes, but
//! `write_offset` callers must ensure alignment themselves.

use std::marker::PhantomData;

use bytemuck::Pod;
use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device, Queue};

/// A typed GPU buffer that tracks element count and capacity.
///
/// Behaves conceptually like `Vec<T>` on the GPU: it has a `capacity`
/// (allocated element slots) and a `len` (elements actually written).
///
/// This is a **pure GPU** buffer — no CPU shadow copy is maintained.
/// For CPU readback, use [`StagingBuffer`](super::StagingBuffer).
pub struct GpuBuffer<T: Pod> {
    buffer: Buffer,
    len: u64,
    capacity: u64,
    usage: BufferUsages,
    _marker: PhantomData<T>,
}

impl<T: Pod> GpuBuffer<T> {
    const ELEM_SIZE: u64 = std::mem::size_of::<T>() as u64;

    /// Creates an empty buffer with room for `capacity` elements.
    ///
    /// The buffer is allocated on the GPU but contains no valid data
    /// (`len` starts at 0).
    pub fn with_capacity(device: &Device, label: &str, capacity: u64, usage: BufferUsages) -> Self {
        let byte_size = capacity * Self::ELEM_SIZE;

        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: byte_size,
            usage,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            len: 0,
            capacity,
            usage,
            _marker: PhantomData,
        }
    }

    /// Creates a buffer initialized with `data`.
    ///
    /// Both `len` and `capacity` are set to `data.len()`.
    pub fn from_data(device: &Device, label: &str, data: &[T], usage: BufferUsages) -> Self {
        use wgpu::util::DeviceExt;

        let count = data.len() as u64;

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage,
        });

        Self {
            buffer,
            len: count,
            capacity: count,
            usage,
            _marker: PhantomData,
        }
    }

    /// Overwrites the entire buffer contents and updates `len`.
    ///
    /// If `data.len() > capacity`, this is a **silent truncation** — only
    /// `capacity` elements are written. Prefer calling [`grow`](Self::grow)
    /// first if you need more space.
    pub fn write(&mut self, queue: &Queue, data: &[T]) {
        let count = (data.len() as u64).min(self.capacity);
        let bytes = bytemuck::cast_slice(&data[..count as usize]);
        queue.write_buffer(&self.buffer, 0, bytes);
        self.len = count;
    }

    /// Writes `data` starting at element `offset` without changing `len`.
    ///
    /// # Panics
    ///
    /// Panics if `offset + data.len()` exceeds `capacity`.
    pub fn write_offset(&self, queue: &Queue, offset: u64, data: &[T]) {
        assert!(
            offset + data.len() as u64 <= self.capacity,
            "write_offset out of bounds: offset({offset}) + len({}) > capacity({})",
            data.len(),
            self.capacity,
        );

        let byte_offset = offset * Self::ELEM_SIZE;
        queue.write_buffer(&self.buffer, byte_offset, bytemuck::cast_slice(data));
    }

    /// Reallocates the buffer with `new_capacity` elements.
    ///
    /// **Does not preserve GPU-side data.** The old buffer is dropped and
    /// a fresh one is created. `len` is reset to 0.
    ///
    /// The caller re-uploads its data after growing — the buffer does not
    /// preserve contents across a resize.
    pub fn grow(&mut self, device: &Device, label: &str, new_capacity: u64) {
        if new_capacity <= self.capacity {
            return;
        }

        let byte_size = new_capacity * Self::ELEM_SIZE;

        self.buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: byte_size,
            usage: self.usage,
            mapped_at_creation: false,
        });

        self.capacity = new_capacity;
        self.len = 0;
    }

    /// Returns a reference to the underlying `wgpu::Buffer`.
    #[inline]
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Number of elements last written via [`write`](Self::write) or
    /// [`from_data`](Self::from_data).
    #[inline]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Returns `true` if no elements have been written.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Total element slots allocated on the GPU.
    #[inline]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Total byte size of the allocated GPU buffer.
    #[inline]
    pub fn byte_size(&self) -> u64 {
        self.capacity * Self::ELEM_SIZE
    }

    /// The `BufferUsages` this buffer was created with.
    #[inline]
    pub fn usage(&self) -> BufferUsages {
        self.usage
    }
}

#[cfg(test)]
mod tests;
