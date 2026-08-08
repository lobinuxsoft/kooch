//! Uniform buffer with automatic `encase` serialization.
//!
//! [`UniformBuffer<T>`] wraps a `wgpu::Buffer` sized for a single uniform
//! value of type `T` (which must implement [`ShaderType`]). It handles
//! encase serialization (std140 layout) automatically on [`write`](UniformBuffer::write).
//!
//! The internal scratch `Vec<u8>` is reused across frames to avoid
//! re-allocating each write.

use encase::{ShaderType, UniformBuffer as EncaseUniformBuffer};
use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device, Queue};

/// A GPU uniform buffer that serializes `T` via encase (std140 layout).
///
/// # Example
///
/// ```ignore
/// use encase::ShaderType;
///
/// #[derive(ShaderType)]
/// struct CameraUniforms {
///     view_proj: glam::Mat4,
///     position: glam::Vec3,
/// }
///
/// let mut uniform = UniformBuffer::<CameraUniforms>::new(device, "camera");
/// uniform.write(queue, &CameraUniforms { /* ... */ });
/// // Use uniform.buffer() in your bind group.
/// ```
pub struct UniformBuffer<T: ShaderType> {
    buffer: Buffer,
    scratch: Vec<u8>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: ShaderType + encase::internal::WriteInto> UniformBuffer<T> {
    /// Creates a uniform buffer sized to hold one instance of `T`.
    ///
    /// The size is determined by `T::METADATA.min_size()` which accounts
    /// for std140 padding rules.
    pub fn new(device: &Device, label: &str) -> Self {
        let min_size = T::min_size();
        let size = min_size.get();

        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            scratch: Vec::with_capacity(size as usize),
            _marker: std::marker::PhantomData,
        }
    }

    /// Serializes `value` with encase and uploads it to the GPU.
    ///
    /// The scratch buffer is reused between calls to avoid allocation.
    pub fn write(&mut self, queue: &Queue, value: &T) {
        self.scratch.clear();

        let mut encoder = EncaseUniformBuffer::new(&mut self.scratch);
        encoder.write(value).expect("failed to serialize uniform");

        let bytes = encoder.into_inner();
        queue.write_buffer(&self.buffer, 0, bytes);
    }

    /// Returns a reference to the underlying `wgpu::Buffer` for bind groups.
    #[inline]
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }
}

#[cfg(test)]
#[allow(dead_code)] // encase derive generates `check` fns that trigger this
mod tests;
