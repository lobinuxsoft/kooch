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
mod tests {
    use super::*;

    fn create_headless_device() -> (Device, Queue) {
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

    #[test]
    #[ignore] // Requires GPU hardware.
    fn staging_readback_roundtrip() {
        use wgpu::util::DeviceExt;

        let (device, queue) = create_headless_device();

        let data: Vec<f32> = (0..64).map(|i| i as f32 * 0.5).collect();
        let byte_size = (data.len() * std::mem::size_of::<f32>()) as u64;

        // Create source buffer with data.
        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("source"),
            contents: bytemuck::cast_slice(&data),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        });

        let staging = StagingBuffer::new(&device, byte_size);
        assert_eq!(staging.size(), byte_size);

        let result: Vec<f32> = staging.read_buffer(&device, &queue, &source);
        assert_eq!(result, data);
    }

    #[test]
    #[ignore] // Requires GPU hardware.
    fn staging_manual_copy_and_readback() {
        use wgpu::util::DeviceExt;

        let (device, queue) = create_headless_device();

        let data = [42u32, 7, 13, 99];
        let byte_size = std::mem::size_of_val(&data) as u64;

        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("source"),
            contents: bytemuck::cast_slice(&data),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        });

        let staging = StagingBuffer::new(&device, byte_size);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test"),
        });
        staging.copy_from(&mut encoder, &source);
        queue.submit(std::iter::once(encoder.finish()));

        let result: Vec<u32> = staging.read_back(&device);
        assert_eq!(result, vec![42, 7, 13, 99]);
    }
}
