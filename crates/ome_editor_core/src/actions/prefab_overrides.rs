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

use ome_core::resource::Resources;
use ome_ecs::component::{ComponentNames, ComponentRegistry};
use ome_ecs::entity::Entity;
use ome_ecs::prefab_instance::{OverrideAddress, PrefabInstance, PrefabMember};
use ome_ecs::reflect::ReflectValue;

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
                let transform = std::any::type_name::<ome_ecs::transform::Transform>().to_owned();
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
                    ome_ecs::prefab_instance::WHOLE_COMPONENT.to_owned(),
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
    component: ome_ecs::component::ComponentId,
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
mod tests {
    use super::*;
    use ome_ecs::transform::Transform;

    /// A world with one two-entity instance already linked.
    fn world_with_an_instance() -> (Resources, Entity, Entity) {
        let mut resources = Resources::new();
        resources.insert(ome_ecs::allocator::EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ome_ecs::archetype_registry::ArchetypeRegistry::new());
        resources.insert(ome_ecs::query::AccessTracker::new());
        resources.insert(ome_ecs::commands::Commands::new());

        let mut names = ComponentNames::new();
        names.intern(std::any::type_name::<PrefabInstance>());
        names.intern(std::any::type_name::<Transform>());
        resources.insert(names);

        let root = Entity::new(0, 0);
        let child = Entity::new(1, 0);
        ome_ecs::prefab_instance::attach(
            &mut resources,
            root,
            &[root, child],
            ome_core::Guid::new_v4(),
        );
        (resources, root, child)
    }

    fn component(resources: &Resources, name: &str) -> ome_ecs::component::ComponentId {
        resources.get::<ComponentNames>().unwrap().id(name).unwrap()
    }

    fn overrides_written(actions: &[EditorAction]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                EditorAction::SetField {
                    value: ReflectValue::String(s),
                    ..
                } => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// Most edits in a scene are not on a prefab instance, and paying
    /// anything for them would tax the common case.
    #[test]
    fn an_edit_outside_any_instance_records_nothing() {
        let (resources, _, _) = world_with_an_instance();
        let edit = EditorAction::SetField {
            entity: Entity::new(99, 0),
            component: component(&resources, std::any::type_name::<Transform>()),
            field: "position".to_owned(),
            value: ReflectValue::Vec3(glam::Vec3::ONE),
        };
        assert!(record(&resources, &[&edit]).is_empty());
    }

    /// An edit on a *child* of an instance is recorded against the root,
    /// which is where the set lives — and addressed by the child's index
    /// in the prefab, not by its handle.
    #[test]
    fn an_edit_on_a_member_is_recorded_against_its_instance() {
        let (resources, root, child) = world_with_an_instance();
        let edit = EditorAction::SetField {
            entity: child,
            component: component(&resources, std::any::type_name::<Transform>()),
            field: "position".to_owned(),
            value: ReflectValue::Vec3(glam::Vec3::ONE),
        };

        let out = record(&resources, &[&edit]);
        assert_eq!(out.len(), 1, "one write, on the instance root");
        match &out[0] {
            EditorAction::SetField { entity, field, .. } => {
                assert_eq!(*entity, root);
                assert_eq!(field, "overrides");
            }
            _ => panic!("expected a SetField on the instance root"),
        }
        assert_eq!(overrides_written(&out).len(), 1);
    }

    /// A translate must not claim the rotation was overridden. Marking a
    /// field pins it to its current value, so an over-eager mark quietly
    /// stops the prefab from ever reaching that field again.
    #[test]
    fn a_transform_drag_records_only_what_moved() {
        let (resources, _, child) = world_with_an_instance();
        let before = Transform::default();
        let after = Transform {
            position: glam::Vec3::X,
            ..Transform::default()
        };
        let edit = EditorAction::TransformEdit {
            entity: child,
            before,
            after,
            desc: "Move",
        };

        let written = overrides_written(&record(&resources, &[&edit]));
        assert_eq!(written.len(), 1);
        assert!(written[0].contains("position"));
        assert!(
            !written[0].contains("rotation") && !written[0].contains("scale"),
            "an untouched field was marked: {}",
            written[0],
        );
    }

    /// The component that stores overrides must not record its own write,
    /// or persisting a mark would mark persisting it.
    #[test]
    fn writing_the_override_set_is_not_itself_an_override() {
        let (resources, root, _) = world_with_an_instance();
        let edit = EditorAction::SetField {
            entity: root,
            component: component(&resources, std::any::type_name::<PrefabInstance>()),
            field: "overrides".to_owned(),
            value: ReflectValue::String("0\u{1f}T\u{1f}x".to_owned()),
        };
        assert!(record(&resources, &[&edit]).is_empty());
    }
}
