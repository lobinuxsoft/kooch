//! Component trait definitions.

use std::any::Any;

use wgpu::{Device, Queue};

use crate::entity::Entity;

/// Marker for components stored in GPU buffers.
///
/// Requires [`bytemuck::Pod`] for safe byte reinterpretation when writing
/// to GPU storage buffers.
pub trait GpuComponent: bytemuck::Pod + Send + Sync + 'static {}

/// Marker for CPU-only components.
///
/// Any `Send + Sync + 'static` type can implement this.
pub trait Component: Send + Sync + 'static {}

/// Type-erased interface for component storages.
///
/// Used by [`ComponentRegistry`](super::ComponentRegistry) to operate on
/// heterogeneous storages without knowing the concrete component type.
pub(crate) trait AnyStorage: Send + Sync + 'static {
    /// Removes the component for `entity`, if present.
    fn remove_entity(&mut self, entity: Entity);

    /// Syncs CPU shadow data to the GPU buffer.
    ///
    /// Default is no-op (for CPU-only storages).
    fn sync_gpu(&mut self, _device: &Device, _queue: &Queue, _capacity: u32) {}

    /// Returns `self` as `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Returns `self` as `&mut dyn Any` for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
