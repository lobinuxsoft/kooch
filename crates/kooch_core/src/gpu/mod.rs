//! GPU context initialization via wgpu.
//!
//! [`GpuContext`] is the central GPU infrastructure for the engine,
//! holding the wgpu Instance, Adapter, Device, Queue, and Surface.
//!
//! # Example
//! ```ignore
//! // GpuContext is created automatically by WindowPlugin during resumed().
//! // Access it as a resource in any system:
//! fn my_system(resources: &mut Resources) {
//!     if let Some(gpu) = resources.get::<GpuContext>() {
//!         tracing::info!("Using GPU: {:?}", gpu.adapter_info());
//!     }
//! }
//! ```

mod context;
mod error;
mod features;
mod limits;

#[cfg(test)]
mod tests;

pub use context::GpuContext;
pub use error::GpuError;
pub use features::vbuf64_features;
