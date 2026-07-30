//! Tests for prefab propagation.
//!
//! The plan is the whole of the decision — what follows the prefab and
//! what the user's own changes protect — so it is what these hold.
//! Applying it is a loop over `reflect_set_field`.

use super::*;
use crate::prefab_instance::WHOLE_COMPONENT;
use crate::scene::{ComponentDescription, EntityDescription};

fn document() -> SceneDocument {
    SceneDocument {
        id: Guid::new_v4(),
        name: "Enemy".into(),
        version: "0.1.0".into(),
        entities: vec![EntityDescription {
            name: "Root".into(),
            parent_index: None,
            parent: None,
            components: vec![ComponentDescription {
                type_name: "test::Health".into(),
                fields: vec![
                    ("hp".into(), ReflectValue::U32(50)),
                    ("max_hp".into(), ReflectValue::U32(50)),
                ],
            }],
        }],
    }
}

/// The planning rule against one entity, without needing a world.
fn plan_against(instance: &PrefabInstance, document: &SceneDocument) -> Vec<PlannedWrite> {
    let entity = Entity::new(0, 0);
    let mut writes = Vec::new();
    for component in &document.entities[0].components {
        if instance.owns_component(0, &component.type_name) {
            continue;
        }
        for (field, value) in &component.fields {
            let address = OverrideAddress {
                entity: 0,
                component: component.type_name.clone(),
                field: field.clone(),
            };
            if instance.is_overridden(&address) {
                continue;
            }
            writes.push(PlannedWrite {
                entity,
                component: component.type_name.clone(),
                field: field.clone(),
                value: value.clone(),
                add_component: false,
            });
        }
    }
    writes
}

fn address(field: &str) -> OverrideAddress {
    OverrideAddress {
        entity: 0,
        component: "test::Health".into(),
        field: field.into(),
    }
}

#[test]
fn an_untouched_instance_takes_every_value() {
    let instance = PrefabInstance::new(Guid::new_v4());
    assert_eq!(plan_against(&instance, &document()).len(), 2);
}

/// The rule the whole feature rests on: a field the user changed on this
/// instance is left alone, and the ones they did not still follow.
#[test]
fn an_overridden_field_is_left_alone_and_the_rest_are_not() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(address("hp"), None);

    let writes = plan_against(&instance, &document());
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].field, "max_hp",
        "the overridden field was overwritten",
    );
}

/// Overriding everything is the same as detaching, and must not
/// half-apply.
#[test]
fn an_instance_that_overrode_everything_takes_nothing() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(address("hp"), None);
    instance.mark(address("max_hp"), None);
    assert!(plan_against(&instance, &document()).is_empty());
}

/// A component the user took off this instance stays off. Without this,
/// removing one lasted exactly until the next time the prefab was saved
/// and then came back with no explanation.
#[test]
fn a_component_the_user_removed_is_not_written_back() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(address(WHOLE_COMPONENT), None);
    assert!(plan_against(&instance, &document()).is_empty());
}

/// Presence and a field on the same component are separate decisions:
/// overriding a value must not read as owning the component.
#[test]
fn overriding_a_field_does_not_claim_the_component() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(address("hp"), None);
    assert!(!instance.owns_component(0, "test::Health"));
}

/// An address carries its entity too, so a decision about a child does not
/// speak for the root.
#[test]
fn owning_a_component_is_per_entity() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(address(WHOLE_COMPONENT), None);
    assert!(instance.owns_component(0, "test::Health"));
    assert!(!instance.owns_component(1, "test::Health"));
}

/// The link and the hierarchy hold an instance together. A prefab edit
/// must never strip them, whatever the document says.
///
/// Held by a test because this is the kind of list that gets extended by
/// whoever adds the next structural component and forgets.
#[test]
fn the_components_that_hold_an_instance_together_are_never_removed() {
    for name in [
        "ome_ecs::prefab_instance::PrefabInstance",
        "ome_ecs::prefab_instance::PrefabMember",
        "ome_ecs::hierarchy::Parent",
        "ome_ecs::hierarchy::Children",
        "ome_ecs::transform::GlobalTransform",
    ] {
        assert!(is_bookkeeping(name), "{name} would have been stripped");
    }
    assert!(!is_bookkeeping("ome_ecs::transform::Transform"));
}

#[test]
fn nothing_is_planned_for_a_prefab_with_no_instances() {
    let mut resources = Resources::new();
    resources.insert(crate::component::ComponentRegistry::new());
    resources.insert(crate::archetype_registry::ArchetypeRegistry::new());
    resources.insert(crate::query::AccessTracker::new());

    let (writes, removals) = plan(&resources, Guid::new_v4());
    assert!(writes.is_empty() && removals.is_empty());
}

/// A world with no instances has nothing to refresh, and must not panic
/// reaching for resources a headless host never inserted.
#[test]
fn refreshing_a_world_with_no_instances_is_a_noop() {
    let mut resources = Resources::new();
    resources.insert(crate::component::ComponentRegistry::new());
    resources.insert(crate::archetype_registry::ArchetypeRegistry::new());
    resources.insert(crate::query::AccessTracker::new());
    refresh_all(&mut resources);
}
