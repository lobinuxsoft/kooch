//! ome_ecs — GPU-driven Entity Component System
//!
//! Provides generational entity IDs with GPU alive-mask synchronisation.
//!
//! - [`Entity`] — lightweight `(index, generation)` handle.
//! - [`EntityAllocator`] — spawn / despawn with FIFO slot recycling.
//! - [`EntityGpuState`] — GPU `StorageBuffer<u32>` alive mask.
//! - [`EcsPlugin`] — one-liner integration into [`App`](ome_core::app::App).

pub mod allocator;
pub mod entity;
pub mod gpu_sync;
pub mod plugin;

pub use allocator::EntityAllocator;
pub use entity::Entity;
pub use gpu_sync::{EntityGpuState, entity_gpu_sync_system};
pub use plugin::EcsPlugin;
