//! kooch_ecs — GPU-driven Entity Component System
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
//! - [`EcsPlugin`] — one-liner integration into [`App`](kooch_core::app::App).

// Allow the derive macro to use `::kooch_ecs::reflect::` paths from any crate.
extern crate self as kooch_ecs;

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
pub mod light_consts;
pub mod lod_force_level;
pub mod mesh_renderer;
pub mod name;
pub mod orthographic_camera;
pub mod persistent_id;
pub mod perspective_camera;
pub mod plugin;
pub mod point_light;
pub mod prefab_instance;
pub mod query;
pub mod reflect;
pub mod scene;
pub mod scene_manager;
pub mod scene_member;
pub mod sky_renderer;
pub mod spot_light;
pub mod storage;
pub mod transform;
pub mod world_snapshot;

pub use allocator::EntityAllocator;
pub use archetype::{Archetype, ArchetypeId};
pub use archetype_registry::ArchetypeRegistry;
pub use commands::Commands;
pub use component::{
    Component, ComponentId, ComponentNames, ComponentRegistry, ComponentStorage, StorageId,
};
pub use directional_light::DirectionalLight;
pub use entity::Entity;
pub use ephemeral::EphemeralComponents;
pub use hierarchy::{Children, GlobalTransform, Parent};
pub use kooch_ecs_macros::Reflect;
pub use lod_force_level::LodForceLevel;
pub use mesh_renderer::MeshRenderer;
pub use name::Name;
pub use orthographic_camera::OrthographicCamera;
pub use persistent_id::{EntityGuid, PersistentId, PersistentIdAllocator};
pub use perspective_camera::PerspectiveCamera;
pub use plugin::EcsPlugin;
pub use point_light::PointLight;
pub use query::{AccessTracker, Query, QueryFilter, With, Without, WorldQuery};
pub use reflect::{FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue};
pub use scene::{
    ComponentDescription, EntityDescription, SceneDocument, SceneError, sync_scene_to_ecs,
};
pub use scene_manager::SceneManager;
pub use scene_member::SceneMember;
pub use sky_renderer::SkyRenderer;
pub use spot_light::SpotLight;
pub use transform::Transform;
