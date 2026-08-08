//! GPU-to-CPU readback via a staging buffer.
//!
//! Storage and vertex buffers cannot be mapped directly — data must first
//! be copied into a `MAP_READ | COPY_DST` staging buffer, then mapped for
//! CPU access.
//!
//! [`StagingBuffer`] encapsulates this pattern with a synchronous API
//! (via `pollster::block_on`), matching the engine's synchronous convention.

use bytemuck::Pod;
use wgpu::{Buffer, BufferDescriptor, BufferUsages, CommandEncoder, Device, Queue};

/// A staging buffer for synchronous GPU-to-CPU data readback.
///
/// # Workflow
///
/// 1. Record a copy command with [`copy_from`](Self::copy_from).
/// 2. Submit the encoder to the queue.
/// 3. Call [`read_back`](Self::read_back) to map, read, and unmap.
///
/// Or use the convenience [`read_buffer`](Self::read_buffer) for a one-shot
/// copy-submit-read cycle.
pub struct StagingBuffer {
    buffer: Buffer,
    size: u64,
}

impl StagingBuffer {
    /// Creates a staging buffer of `size` bytes.
    ///
    /// Usage flags are `MAP_READ | COPY_DST`.
    pub fn new(device: &Device, size: u64) -> Self {
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("staging"),
            size,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self { buffer, size }
    }

    /// Records a buffer-to-buffer copy from `source` into this staging buffer.
    ///
    /// The caller must submit the encoder to the queue after this call.
    pub fn copy_from(&self, encoder: &mut CommandEncoder, source: &Buffer) {
        encoder.copy_buffer_to_buffer(source, 0, &self.buffer, 0, self.size);
    }

    /// Maps the staging buffer, reads it as `&[T]`, and unmaps.
    ///
    /// This blocks synchronously via `pollster::block_on` + `device.poll(Wait)`.
    ///
    /// # Panics
    ///
    /// Panics if the map operation fails (e.g. device lost).
    pub fn read_back<T: Pod>(&self, device: &Device) -> Vec<T> {
        let slice = self.buffer.slice(..);

        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("failed to map staging buffer");
        });
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });

        let data = slice.get_mapped_range();
        let result = bytemuck::cast_slice(&data).to_vec();

        drop(data);
        self.buffer.unmap();

        result
    }

    /// One-shot convenience: copy `source` → staging → CPU as `Vec<T>`.
    ///
    /// Creates a temporary command encoder, copies, submits, and reads back.
    pub fn read_buffer<T: Pod>(&self, device: &Device, queue: &Queue, source: &Buffer) -> Vec<T> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("staging_readback"),
        });

        self.copy_from(&mut encoder, source);
        queue.submit(std::iter::once(encoder.finish()));

        self.read_back(device)
    }

    /// Returns a reference to the underlying `wgpu::Buffer`.
    #[inline]
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Returns the size in bytes of this staging buffer.
    #[inline]
    pub fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests;
