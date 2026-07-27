//! GPU buffer abstractions for the engine.
//!
//! This module provides typed, safe wrappers around `wgpu::Buffer` for
//! common GPU buffer patterns:
//!
//! - [`GpuBuffer<T>`] — Generic typed buffer with Vec-like capacity/len tracking.
//! - [`StagingBuffer`] — GPU-to-CPU synchronous readback.
//! - [`BufferPool`] — Power-of-two bucketed buffer recycling.
//! - [`UniformBuffer<T>`] — Uniform buffer with automatic encase (std140) serialization.
//!
//! # Memory allocation
//!
//! All buffers are currently allocated directly via `wgpu::Device::create_buffer`.
//! Consider integrating [`gpu-allocator`](https://crates.io/crates/gpu-allocator)
//! when any of the following apply:
//!
//! - Hundreds of short-lived buffer allocations per frame.
//! - Measurable memory fragmentation on target hardware.
//! - Need for explicit memory type control (dedicated vs shared heaps).
//!
//! # Bindless resources
//!
//! When the engine adds texture and material systems, a bindless approach
//! (descriptor indexing) can replace per-draw bind group switches. The
//! pattern: maintain a large `StorageBuffer` of material/texture indices,
//! index into it from the shader via `instance_id` or a draw-call parameter.
//! This module does not implement bindless yet — it will be added alongside
//! the texture/material pipeline.

mod gpu_buffer;
mod pool;
mod staging;
mod uniform;

pub use gpu_buffer::GpuBuffer;
pub use pool::{BufferPool, bucket_size};
pub use staging::StagingBuffer;
pub use uniform::UniformBuffer;
