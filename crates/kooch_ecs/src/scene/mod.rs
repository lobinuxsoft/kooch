//! Scene document and serialization.

mod document;
mod entity_refs;
mod error;
mod sync;

#[cfg(test)]
mod tests;

pub mod prefab;
pub mod propagate;

pub use document::{ComponentDescription, EntityDescription, SceneDocument};
pub use error::SceneError;
pub use prefab::{PrefabLoader, spawn as spawn_prefab, spawn_members as spawn_prefab_members};
pub use sync::{
    despawn_scene, instantiate, instantiate_members, loading_from, spawn_scene_as,
    spawn_scene_into, sync_scene_to_ecs,
};
