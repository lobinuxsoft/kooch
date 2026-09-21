//! kooch_ecs — GPU-driven Entity Component System

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
pub mod order;
pub mod orthographic_camera;
pub mod persistent_id;
pub mod perspective_camera;
pub mod plugin;
pub mod point_light;
pub mod post_process;
pub mod post_process_volume;
pub mod prefab_instance;
pub mod query;
pub mod reflect;
pub mod scene;
pub mod scene_manager;
pub mod scene_member;
pub mod sensor_occupancy;
pub mod sky_renderer;
pub mod spot_light;
pub mod storage;
// 🔴 NOT behind `testing`, despite the name. The name is the serialised type path of what lives here
// (`kooch_ecs::testing::spin::Spin`) and a scene resolves a component by that string, so renaming
// the module would drop `Spin` from every entity that has one, silently.
pub mod testing;
pub mod transform;
pub mod tween;
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
/// Declares a system's stage. Inert at compile time — read by the
/// editor's codegen. See the macro's own docs.
pub use kooch_ecs_macros::system;
pub use lod_force_level::LodForceLevel;
pub use mesh_renderer::MeshRenderer;
pub use name::Name;
pub use order::Order;
pub use orthographic_camera::OrthographicCamera;
pub use persistent_id::{EntityGuid, PersistentId, PersistentIdAllocator};
pub use perspective_camera::{PerspectiveCamera, ViewAspect};
pub use plugin::EcsPlugin;
pub use point_light::PointLight;
pub use post_process::{PostEffect, PostProcess};
pub use post_process_volume::PostProcessVolume;
pub use query::{AccessTracker, Query, QueryFilter, With, Without, WorldQuery};
pub use reflect::{FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue};
pub use scene::{
    ComponentDescription, EntityDescription, SceneDocument, SceneError, sync_scene_to_ecs,
};
pub use scene_manager::SceneManager;
pub use scene_member::SceneMember;
pub use sensor_occupancy::{Occupant, SensorOccupancy};
pub use sky_renderer::SkyRenderer;
pub use spot_light::SpotLight;
pub use transform::Transform;
