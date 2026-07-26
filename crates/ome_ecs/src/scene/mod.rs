//! Scene document and serialization.
//!
//! [`SceneDocument`] is the source-of-truth scene format (`.ome_scene`).
//! It stores entity descriptions with reflected component data that can be
//! serialized to/from RON files. The live ECS is just a mirror — the scene
//! file is what persists between sessions.

mod document;
mod entity_refs;
mod error;
mod sync;

#[cfg(test)]
mod tests;

pub use document::{ComponentDescription, EntityDescription, SceneDocument};
pub use error::SceneError;
pub use sync::{despawn_scene, spawn_scene_into, sync_scene_to_ecs};
