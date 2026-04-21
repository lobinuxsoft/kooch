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
//! - [`Query`] — type-safe queries with runtime borrow checking.
//! - [`EcsPlugin`] — one-liner integration into [`App`](ome_core::app::App).

// Allow the derive macro to use `::ome_ecs::reflect::` paths from any crate.
extern crate self as ome_ecs;

pub mod allocator;
pub mod archetype;
pub mod archetype_registry;
pub mod commands;
pub mod component;
pub mod entity;
pub mod gpu_sync;
pub mod plugin;
pub mod query;
pub mod reflect;
pub mod scene;
pub mod hierarchy;
pub mod directional_light;
pub mod dynamic_body;
pub mod kinematic_body;
pub mod static_body;
pub mod velocity;
pub mod mesh_renderer;
pub mod name;
pub mod orthographic_camera;
pub mod perspective_camera;
pub mod point_light;
pub mod sdf_blend;
pub mod sdf_box;
pub mod sdf_capsule;
pub mod sdf_cylinder;
pub mod sdf_plane;
pub mod sdf_sphere;
pub mod sdf_torus;
pub mod spot_light;
pub mod transform;

pub use allocator::EntityAllocator;
pub use archetype::{Archetype, ArchetypeId};
pub use archetype_registry::ArchetypeRegistry;
pub use commands::Commands;
pub use component::{
    Component, ComponentRegistry, ComponentStorage, GpuComponent, GpuComponentStorage,
};
pub use entity::Entity;
pub use gpu_sync::{EntityGpuState, entity_gpu_sync_system};
pub use plugin::EcsPlugin;
pub use query::{AccessTracker, Query, QueryFilter, With, Without, WorldQuery};
pub use ome_ecs_macros::Reflect;
pub use reflect::{FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue};
pub use scene::{
    ComponentDescription, EntityDescription, SceneDocument, SceneError, sync_scene_to_ecs,
};
pub use directional_light::DirectionalLight;
pub use dynamic_body::DynamicBody;
pub use kinematic_body::KinematicBody;
pub use static_body::StaticBody;
pub use velocity::Velocity;
pub use hierarchy::{Children, GlobalTransform, Parent};
pub use mesh_renderer::MeshRenderer;
pub use name::Name;
pub use orthographic_camera::OrthographicCamera;
pub use perspective_camera::PerspectiveCamera;
pub use point_light::PointLight;
pub use sdf_blend::SdfBlend;
pub use sdf_box::SdfBox;
pub use sdf_capsule::SdfCapsule;
pub use sdf_cylinder::SdfCylinder;
pub use sdf_plane::SdfPlane;
pub use sdf_sphere::SdfSphere;
pub use sdf_torus::SdfTorus;
pub use spot_light::SpotLight;
pub use transform::Transform;
