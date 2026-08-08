//! Recording what the user changed on a prefab instance.
//!
//! # Why anything is recorded at all
//!
//! When a prefab changes, its instances have to follow — *except* where
//! the user deliberately made one different. Knowing which is which needs
//! a record of what they touched.
//!
//! Diffing the instance against the prefab instead looks simpler and is
//! wrong: it cannot tell "the user changed this" from "the prefab has
//! changed since", so the moment the prefab moves the diff starts lying.
//! It also leaves nothing to revert *to* — there is no way to distinguish
//! "the user reverted this field" from "the prefab drifted until they
//! happened to match".
//!
//! # Where it happens
//!
//! Before the local/remote split, so an edit is recorded once regardless
//! of which path applies it. Every field edit already arrives as one of
//! two actions — `SetField`, and `TransformEdit` from a gizmo drag — which
//! is what makes this cheap.

use kooch_core::resource::Resources;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::prefab_instance::{OverrideAddress, PrefabInstance, PrefabMember};
use kooch_ecs::reflect::ReflectValue;

use super::EditorAction;

/// Component types whose edits are bookkeeping rather than authored state.
///
/// Marking an override on the component that *stores* overrides would
/// record its own write, and `PrefabMember` is the mapping propagation
/// reads — a user "overriding" either is not a thing.
fn is_bookkeeping(type_name: &str) -> bool {
    matches!(
        type_name.rsplit("::").next().unwrap_or(type_name),
        "PrefabInstance" | "PrefabMember"
    )
}

/// Records the overrides implied by `actions`, and returns the edits that
/// persist them.
///
/// The returned actions are ordinary `SetField`s on the instance root's
/// `PrefabInstance`, so they travel to the project the same way any other
/// edit does. That matters in remote mode: the override set is saved with
/// the scene, and the scene belongs to the project.
pub(super) fn record(resources: &Resources, actions: &[&EditorAction]) -> Vec<EditorAction> {
    let mut marked: Vec<(Entity, OverrideAddress, Option<ReflectValue>)> = Vec::new();
    for action in actions.iter().copied() {
        match action {
            EditorAction::SetField {
                entity,
                component,
                field,
                value,
            } => {
                let Some(type_name) = component_name(resources, *component) else {
                    continue;
                };
                if is_bookkeeping(&type_name) {
                    continue;
                }
                push(
                    resources,
                    &mut marked,
                    *entity,
                    type_name,
                    field.clone(),
                    Some(value.clone()),
                );
            }
            // A drag replaces the whole transform, but the user only moved
            // one thing. Comparing before and after keeps a translate from
            // claiming the rotation was overridden too — which would pin a
            // field to its current value and quietly stop the prefab from
            // ever reaching it again.
            EditorAction::TransformEdit {
                entity,
                before,
                after,
                ..
            } => {
                let transform = std::any::type_name::<kooch_ecs::transform::Transform>().to_owned();
                for (field, changed, value) in [
                    (
                        "position",
                        before.position != after.position,
                        ReflectValue::Vec3(after.position),
                    ),
                    (
                        "rotation",
                        before.rotation != after.rotation,
                        ReflectValue::Quat(after.rotation),
                    ),
                    (
                        "scale",
                        before.scale != after.scale,
                        ReflectValue::Vec3(after.scale),
                    ),
                ] {
                    if changed {
                        push(
                            resources,
                            &mut marked,
                            *entity,
                            transform.clone(),
                            field.to_owned(),
                            Some(value),
                        );
                    }
                }
            }
            // Adding or removing a component on an instance is a decision
            // about its *presence*, and propagation has to respect it both
            // ways: it must not delete what the user added, and must not
            // restore what they took off. Without this, removing a
            // component from an instance lasted exactly until the next
            // time the prefab was saved.
            EditorAction::AddComponent { entity, component }
            | EditorAction::RemoveComponent { entity, component } => {
                let Some(type_name) = component_name(resources, *component) else {
                    continue;
                };
                if is_bookkeeping(&type_name) {
                    continue;
                }
                // Presence carries no value — it is a decision about
                // whether the component belongs, not about what it holds.
                // A component the user *added* also gets a record per
                // field below, so its values survive a scene that no
                // longer writes the entity out.
                push(
                    resources,
                    &mut marked,
                    *entity,
                    type_name.clone(),
                    kooch_ecs::prefab_instance::WHOLE_COMPONENT.to_owned(),
                    None,
                );
                if matches!(action, EditorAction::AddComponent { .. })
                    && let Some(defaults) = default_fields(resources, &type_name)
                {
                    for (field, value) in defaults {
                        push(
                            resources,
                            &mut marked,
                            *entity,
                            type_name.clone(),
                            field,
                            Some(value),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    if marked.is_empty() {
        return Vec::new();
    }
    persist(resources, marked)
}

/// Records one address, if the entity is part of an instance at all.
#[allow(clippy::too_many_arguments)]
fn push(
    resources: &Resources,
    marked: &mut Vec<(Entity, OverrideAddress, Option<ReflectValue>)>,
    entity: Entity,
    component: String,
    field: String,
    value: Option<ReflectValue>,
) {
    let Some(member) = member_of(resources, entity) else {
        // Not part of any instance. Most edits are this.
        return;
    };
    marked.push((
        member.root,
        OverrideAddress {
            entity: member.index as usize,
            component,
            field,
        },
        value,
    ));
}

/// The values a freshly-added component starts with.
fn default_fields(resources: &Resources, type_name: &str) -> Option<Vec<(String, ReflectValue)>> {
    let registry = resources.get::<ComponentRegistry>()?;
    let type_id = registry.type_id_by_name(type_name)?;
    registry.reflect_default_fields(&type_id)
}

/// Folds the new marks into each instance's existing set and emits the
/// edits that write them back.
///
/// Grouped by instance so a drag that touched two entities of one prefab
/// produces one write rather than two that overwrite each other.
fn persist(
    resources: &Resources,
    marked: Vec<(Entity, OverrideAddress, Option<ReflectValue>)>,
) -> Vec<EditorAction> {
    let Some(component) = resources
        .get::<ComponentNames>()
        .and_then(|names| names.id(std::any::type_name::<PrefabInstance>()))
    else {
        return Vec::new();
    };

    let mut by_root: std::collections::HashMap<
        Entity,
        Vec<(OverrideAddress, Option<ReflectValue>)>,
    > = std::collections::HashMap::new();
    for (root, address, value) in marked {
        by_root.entry(root).or_default().push((address, value));
    }

    by_root
        .into_iter()
        .filter_map(|(root, addresses)| {
            let mut instance = instance_at(resources, root)?;
            let before = instance.overrides.clone();
            for (address, value) in addresses {
                instance.mark(address, value);
            }
            // Nothing new: the user re-touched a field they had already
            // overridden. Emitting the write anyway would put an identical
            // edit on the wire every frame of a drag.
            if instance.overrides == before {
                return None;
            }
            Some(EditorAction::SetField {
                entity: root,
                component,
                field: "overrides".to_owned(),
                value: ReflectValue::String(instance.overrides),
            })
        })
        .collect()
}

fn component_name(
    resources: &Resources,
    component: kooch_ecs::component::ComponentId,
) -> Option<String> {
    resources
        .get::<ComponentNames>()?
        .name(component)
        .map(str::to_owned)
}

fn member_of(resources: &Resources, entity: Entity) -> Option<PrefabMember> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<PrefabMember>()?
        .get(entity)
        .cloned()
}

fn instance_at(resources: &Resources, root: Entity) -> Option<PrefabInstance> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<PrefabInstance>()?
        .get(root)
        .cloned()
}

#[cfg(test)]
mod tests;
