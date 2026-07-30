//! Scene document and serialization.
//!
//! [`SceneDocument`] is the source-of-truth scene format (`.scene`, and `.prefab` — the same format).
//! It stores entity descriptions with reflected component data that can be
//! serialized to/from RON files. The live ECS is just a mirror — the scene
//! file is what persists between sessions.

mod document;
mod entity_refs;
mod error;
mod sync;

#[cfg(test)]
mod tests;

pub mod prefab;

pub use document::{ComponentDescription, EntityDescription, SceneDocument};
pub use error::SceneError;
pub use prefab::{PrefabLoader, spawn as spawn_prefab, spawn_members as spawn_prefab_members};
pub use sync::{
    despawn_scene, instantiate, instantiate_members, spawn_scene_into, sync_scene_to_ecs,
};
