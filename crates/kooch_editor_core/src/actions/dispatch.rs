//! ECS-action → undo command conversion + same-variant batching helpers.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::component::{ComponentId, ComponentNames, ComponentRegistry};

use crate::undo::{
    AddComponentCommand, AddDynamicComponentCommand, DespawnCommand, DuplicateCommand,
    EditorCommand, RemoveComponentCommand, RemoveDynamicComponentCommand, SetDynamicFieldCommand,
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
    // No warning when this misses: a plugin-declared component has no
    // local TypeId by construction, and callers fall through to the
    // by-name commands. Warning here fired once per keystroke.
    resources.get::<ComponentRegistry>()?.type_id_by_name(name)
}

/// The interned name behind a [`ComponentId`].
fn component_name(resources: &Resources, component: ComponentId) -> Option<String> {
    resources
        .get::<ComponentNames>()?
        .name(component)
        .map(str::to_owned)
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
            // A plugin's component has no local TypeId, so it is edited
            // by name against DynamicComponents instead.
            let Some(type_id) = resolve_component(resources, *component) else {
                let name = component_name(resources, *component)?;
                return SetDynamicFieldCommand::new(
                    resources,
                    *entity,
                    &name,
                    field.clone(),
                    value.clone(),
                )
                .map(|cmd| Box::new(cmd) as Box<dyn EditorCommand>);
            };
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
            let Some(type_id) = resolve_component(resources, *component) else {
                let name = component_name(resources, *component)?;
                return AddDynamicComponentCommand::new(resources, *entity, &name)
                    .map(|cmd| Box::new(cmd) as Box<dyn EditorCommand>);
            };
            Some(Box::new(AddComponentCommand::new(*entity, type_id)))
        }
        EditorAction::RemoveComponent { entity, component } => {
            let Some(type_id) = resolve_component(resources, *component) else {
                let name = component_name(resources, *component)?;
                return Some(Box::new(RemoveDynamicComponentCommand::new(
                    resources, *entity, &name,
                )));
            };
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
pub(super) fn batch_description(actions: &[&EditorAction]) -> String {
    let count = actions.len();
    match actions.first().copied() {
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
