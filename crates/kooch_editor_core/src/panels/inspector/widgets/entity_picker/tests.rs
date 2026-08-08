use super::*;
use crate::state::ComponentDisplayInfo;
use crate::state::ReflectedFields;

fn component(name: &str, fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
    ComponentDisplayInfo {
        type_id: std::any::TypeId::of::<()>(),
        component: kooch_ecs::ComponentId(0),
        short_name: name.to_owned().into(),
        fields: ReflectedFields::Values(fields),
        field_metas: None,
        visibility: Default::default(),
    }
}

fn entity(index: u32, components: Vec<ComponentDisplayInfo>) -> EntityDisplayInfo {
    EntityDisplayInfo {
        is_prefab_instance: false,
        entity: Entity::new(index, 0),
        components,
        parent: None,
        children: Vec::new(),
        depth: 0,
        global_rotation: None,
        scene: None,
        parent_global_rotation: None,
    }
}

fn named(index: u32, name: &str) -> EntityDisplayInfo {
    entity(
        index,
        vec![component(
            "Name",
            vec![("value".into(), ReflectValue::String(name.to_owned()))],
        )],
    )
}

/// `4:1` is not an answer to "which body is this".
#[test]
fn an_entity_reads_as_its_name() {
    assert_eq!(label_for(&named(4, "Door frame")), "Door frame");
}

/// An entity with no name still has to be pickable, and the handle is
/// the only thing left to call it.
#[test]
fn a_nameless_entity_falls_back_to_its_handle() {
    assert_eq!(label_for(&entity(7, Vec::new())), "Entity 7:0");
}

/// A joint body without a rigid body is not a body. Accepting it would
/// leave the joint inert, which looks exactly like it being broken.
#[test]
fn a_requirement_excludes_what_does_not_carry_it() {
    let plain = named(1, "Marker");
    let body = entity(
        2,
        vec![
            component("PhysicsBody", Vec::new()),
            component("Name", vec![]),
        ],
    );

    assert!(!accepts(&plain, "PhysicsBody"));
    assert!(accepts(&body, "PhysicsBody"));
}

/// A field with no requirement takes anything — most references have
/// nothing in particular to demand.
#[test]
fn no_requirement_accepts_anything() {
    assert!(accepts(&named(1, "Marker"), ""));
}
