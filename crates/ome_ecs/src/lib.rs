//! ome_ecs — GPU-driven Entity Component System
//!
//! Provides generational entity IDs with GPU alive-mask synchronisation
//! and dense component storage with lazy GPU buffer backing.
//!
//! - [`Entity`] — lightweight `(index, generation)` handle.
//! - [`EntityAllocator`] — spawn / despawn with FIFO slot recycling.
//! - [`EntityGpuState`] — GPU `StorageBuffer<u32>` alive mask.
//! - [`ComponentRegistry`] — type-erased registry of component storages.
//! - [`GpuComponentStorage`] — dense GPU-backed component storage.
//! - [`ComponentStorage`] — CPU-only `HashMap`-backed component storage.
//! - [`Archetype`] — entity group sharing the same component set.
//! - [`ArchetypeRegistry`] — archetype index with transition caching.
//! - [`EcsPlugin`] — one-liner integration into [`App`](ome_core::app::App).

pub mod allocator;
pub mod archetype;
pub mod archetype_registry;
pub mod component;
pub mod entity;
pub mod gpu_sync;
pub mod plugin;

pub use allocator::EntityAllocator;
pub use archetype::{Archetype, ArchetypeId};
pub use archetype_registry::ArchetypeRegistry;
pub use component::{
    Component, ComponentRegistry, ComponentStorage, GpuComponent, GpuComponentStorage,
};
pub use entity::Entity;
pub use gpu_sync::{EntityGpuState, entity_gpu_sync_system};
pub use plugin::EcsPlugin;
