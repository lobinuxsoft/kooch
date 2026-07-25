//! ome_ecs — GPU-driven Entity Component System
//!
//! Provides generational entity IDs with GPU alive-mask synchronisation
//! and dense component storage with lazy GPU buffer backing.
//!
//! - [`Entity`] — lightweight `(index, generation)` handle.
//! - [`EntityAllocator`] — spawn / despawn with FIFO slot recycling.
//! - [`ComponentRegistry`] — type-erased registry of component storages.
//! - [`ComponentStorage`] — CPU-only `HashMap`-backed component storage.
//! - [`Archetype`] — entity group sharing the same component set.
//! - [`ArchetypeRegistry`] — archetype index with transition caching.
//! - [`Query`] — type-safe queries with runtime borrow checking.
//! - [`EcsPlugin`] — one-liner integration into [`App`](ome_core::app::App).

// Allow the derive macro to use `::ome_ecs::reflect::` paths from any crate.
extern crate self as ome_ecs;

pub mod allocator;
pub mod archetype;
pub mod archetype_registry;
pub mod commands;
pub mod component;
pub mod directional_light;
pub mod dynamic_components;
pub mod entity;
pub mod ephemeral;
pub mod hierarchy;
pub mod lod_force_level;
pub mod mesh_renderer;
pub mod name;
pub mod orthographic_camera;
pub mod perspective_camera;
pub mod plugin;
pub mod point_light;
pub mod query;
pub mod reflect;
pub mod scene;
pub mod scene_manager;
pub mod sky_renderer;
pub mod spot_light;
pub mod transform;
pub mod world_snapshot;

pub use allocator::EntityAllocator;
pub use archetype::{Archetype, ArchetypeId};
pub use archetype_registry::ArchetypeRegistry;
pub use commands::Commands;
pub use component::{Component, ComponentId, ComponentNames, ComponentRegistry, ComponentStorage};
pub use directional_light::DirectionalLight;
pub use entity::Entity;
pub use ephemeral::EphemeralComponents;
pub use hierarchy::{Children, GlobalTransform, Parent};
pub use lod_force_level::LodForceLevel;
pub use mesh_renderer::MeshRenderer;
pub use name::Name;
pub use ome_ecs_macros::Reflect;
pub use orthographic_camera::OrthographicCamera;
pub use perspective_camera::PerspectiveCamera;
pub use plugin::EcsPlugin;
pub use point_light::PointLight;
pub use query::{AccessTracker, Query, QueryFilter, With, Without, WorldQuery};
pub use reflect::{FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue};
pub use scene::{
    ComponentDescription, EntityDescription, SceneDocument, SceneError, sync_scene_to_ecs,
};
pub use scene_manager::SceneManager;
pub use sky_renderer::SkyRenderer;
pub use spot_light::SpotLight;
pub use transform::Transform;
