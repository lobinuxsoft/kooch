//! The editor's half of a parametric block (#1106).
//!
//! 🔴 The derive itself moved to the engine in #1567 — [`kooch_blockmesh::sync_shapes`] runs in
//! `PrePhysics` wherever the world lives, so the project rebuilds its own geometry instead of
//! receiving parameters it cannot act on. What is left here is the one gesture the engine cannot
//! know about: giving up the parameters.

use kooch_blockmesh::BlockShape;
use kooch_core::resource::Resources;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::entity::Entity;

use crate::actions::EditorAction;

/// Removes the parameters from a block edited by hand, so the next parameter change cannot
/// overwrite the edit. `None` when the block has none.
pub(crate) fn bake(resources: &Resources, entity: Entity) -> Option<EditorAction> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<BlockShape>()?
        .get(entity)?;
    let component = resources
        .get::<ComponentNames>()?
        .id(std::any::type_name::<BlockShape>())?;
    Some(EditorAction::RemoveComponent { entity, component })
}
