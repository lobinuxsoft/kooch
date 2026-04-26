//! ECS-action → undo command conversion + same-variant batching helpers.

use ome_core::resource::Resources;

use crate::undo::{
    AddComponentCommand, DespawnCommand, EditorCommand, RemoveComponentCommand, SetFieldCommand,
    SpawnCommand,
};

use super::EditorAction;

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
        EditorAction::Despawn(entity) => Some(Box::new(DespawnCommand::new(resources, *entity))),
        EditorAction::SetField {
            entity,
            type_id,
            field,
            value,
        } => {
            if let Some(cmd) =
                SetFieldCommand::new(resources, *entity, *type_id, field.clone(), value.clone())
            {
                Some(Box::new(cmd))
            } else {
                tracing::warn!("failed to create SetFieldCommand for '{field}'");
                None
            }
        }
        EditorAction::AddComponent { entity, type_id } => {
            Some(Box::new(AddComponentCommand::new(*entity, *type_id)))
        }
        EditorAction::RemoveComponent { entity, type_id } => {
            Some(Box::new(RemoveComponentCommand::new(resources, *entity, *type_id)))
        }
        _ => None,
    }
}

/// Returns a description for a group of same-variant actions.
pub(super) fn batch_description(actions: &[EditorAction]) -> String {
    let count = actions.len();
    match actions.first() {
        Some(EditorAction::Spawn { .. }) => format!("Spawn {count} Entities"),
        Some(EditorAction::Despawn(_)) => format!("Despawn {count} Entities"),
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
            | (EditorAction::Despawn(_), EditorAction::Despawn(_))
            | (EditorAction::SetField { .. }, EditorAction::SetField { .. })
            | (EditorAction::AddComponent { .. }, EditorAction::AddComponent { .. })
            | (EditorAction::RemoveComponent { .. }, EditorAction::RemoveComponent { .. })
    )
}
