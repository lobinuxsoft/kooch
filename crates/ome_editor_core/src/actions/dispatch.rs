//! ECS-action → undo command conversion + same-variant batching helpers.

use std::any::TypeId;

use ome_core::resource::Resources;
use ome_ecs::component::{ComponentId, ComponentNames, ComponentRegistry};

use crate::undo::{
    AddComponentCommand, DespawnCommand, DuplicateCommand, EditorCommand, RemoveComponentCommand,
    SetFieldCommand, SpawnCommand, SpawnMeshCommand, TransformEditCommand,
};

use super::EditorAction;

/// Resolves a portable [`ComponentId`] to the local `TypeId` the undo
/// commands and reflection registry operate on.
///
/// Returns `None` when this binary has no Rust type for the component —
/// e.g. the standalone hub asked to mutate a project's own component.
/// The action is dropped rather than misapplied; in the remote design
/// the project's server process handles it instead.
fn resolve_component(resources: &Resources, component: ComponentId) -> Option<TypeId> {
    let name = resources.get::<ComponentNames>()?.name(component)?;
    let type_id = resources.get::<ComponentRegistry>()?.type_id_by_name(name);
    if type_id.is_none() {
        tracing::warn!(
            component = name,
            "no local type for component; action dropped"
        );
    }
    type_id
}

/// Converts an action into an undoable command, capturing before-state.
///
/// Returns `None` for non-ECS actions (scene I/O, play, project management).
pub(super) fn action_to_command(
    action: &EditorAction,
    resources: &Resources,
) -> Option<Box<dyn EditorCommand>> {
    match action {
        EditorAction::Spawn { extra, name } => {
            Some(Box::new(SpawnCommand::new(extra.clone(), name.clone())))
        }
        EditorAction::SpawnMesh { path, name } => {
            Some(Box::new(SpawnMeshCommand::new(path.clone(), name.clone())))
        }
        EditorAction::Despawn(entity) => Some(Box::new(DespawnCommand::new(resources, *entity))),
        EditorAction::Duplicate(entity) => {
            Some(Box::new(DuplicateCommand::new(resources, *entity)))
        }
        EditorAction::SetField {
            entity,
            component,
            field,
            value,
        } => {
            let type_id = resolve_component(resources, *component)?;
            if let Some(cmd) =
                SetFieldCommand::new(resources, *entity, type_id, field.clone(), value.clone())
            {
                Some(Box::new(cmd))
            } else {
                tracing::warn!("failed to create SetFieldCommand for '{field}'");
                None
            }
        }
        EditorAction::AddComponent { entity, component } => {
            let type_id = resolve_component(resources, *component)?;
            Some(Box::new(AddComponentCommand::new(*entity, type_id)))
        }
        EditorAction::RemoveComponent { entity, component } => {
            let type_id = resolve_component(resources, *component)?;
            Some(Box::new(RemoveComponentCommand::new(
                resources, *entity, type_id,
            )))
        }
        EditorAction::TransformEdit {
            entity,
            before,
            after,
            desc,
        } => Some(Box::new(TransformEditCommand::new(
            *entity, *before, *after, desc,
        ))),
        _ => None,
    }
}

/// Returns a description for a group of same-variant actions.
pub(super) fn batch_description(actions: &[EditorAction]) -> String {
    let count = actions.len();
    match actions.first() {
        Some(EditorAction::Spawn { .. }) => format!("Spawn {count} Entities"),
        Some(EditorAction::SpawnMesh { .. }) => format!("Spawn {count} Mesh Entities"),
        Some(EditorAction::Despawn(_)) => format!("Despawn {count} Entities"),
        Some(EditorAction::Duplicate(_)) => format!("Duplicate {count} Entities"),
        Some(EditorAction::SetField { .. }) => format!("Set {count} Fields"),
        Some(EditorAction::AddComponent { .. }) => format!("Add {count} Components"),
        Some(EditorAction::RemoveComponent { .. }) => format!("Remove {count} Components"),
        _ => "Batch".to_owned(),
    }
}

/// Returns `true` if two actions are the same ECS variant (ignoring payload).
pub(super) fn same_ecs_variant(a: &EditorAction, b: &EditorAction) -> bool {
    matches!(
        (a, b),
        (EditorAction::Spawn { .. }, EditorAction::Spawn { .. })
            | (
                EditorAction::SpawnMesh { .. },
                EditorAction::SpawnMesh { .. }
            )
            | (EditorAction::Despawn(_), EditorAction::Despawn(_))
            | (EditorAction::Duplicate(_), EditorAction::Duplicate(_))
            | (EditorAction::SetField { .. }, EditorAction::SetField { .. })
            | (
                EditorAction::AddComponent { .. },
                EditorAction::AddComponent { .. }
            )
            | (
                EditorAction::RemoveComponent { .. },
                EditorAction::RemoveComponent { .. }
            )
    )
}
