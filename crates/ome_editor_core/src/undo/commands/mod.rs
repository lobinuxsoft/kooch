//! Concrete [`crate::undo::EditorCommand`] implementations, one per
//! kind of edit operation. The parent `undo` module re-exports them
//! all so callers see a flat namespace.

mod component;
mod despawn;
mod duplicate;
mod dynamic_component;
mod set_field;
mod spawn;
mod spawn_mesh;
mod transform_edit;

pub(crate) use component::{AddComponentCommand, RemoveComponentCommand};
pub(crate) use despawn::DespawnCommand;
pub(crate) use duplicate::DuplicateCommand;
pub(crate) use dynamic_component::{
    AddDynamicComponentCommand, RemoveDynamicComponentCommand, SetDynamicFieldCommand,
};
pub(crate) use set_field::SetFieldCommand;
pub(crate) use spawn::SpawnCommand;
pub(crate) use spawn_mesh::SpawnMeshCommand;
pub(crate) use transform_edit::TransformEditCommand;
