//! The `Reflect` derive over `Entity` and `Option<Entity>` fields.
//!
//! These cover the contract the scene paths rely on: a live component
//! always reflects `EntityRef::Live`, and never accepts anything else.

use crate::entity::Entity;
use crate::persistent_id::EntityGuid;
use crate::reflect::{EntityRef, FieldKind, Reflect, ReflectError, ReflectValue};
use ome_ecs_macros::Reflect as DeriveReflect;

/// Mirrors the shape #560 needs: a component pointing at two entities,
/// which is exactly what `parent_index` could never express.
#[derive(Debug, Default, Clone, PartialEq, DeriveReflect)]
struct Joint {
    body_a: Entity,
    body_b: Option<Entity>,
    stiffness: f32,
}

impl crate::component::traits::Component for Joint {}

fn joint() -> Joint {
    Joint {
        body_a: Entity::new(4, 1),
        body_b: Some(Entity::new(9, 3)),
        stiffness: 0.5,
    }
}

#[test]
fn entity_fields_are_reported_as_entity_refs() {
    let fields = joint().reflect_fields();

    let body_a = fields.iter().find(|f| f.name == "body_a").expect("body_a");
    assert_eq!(body_a.kind, FieldKind::EntityRef);
    assert_eq!(body_a.type_name, "Entity");

    let body_b = fields.iter().find(|f| f.name == "body_b").expect("body_b");
    assert_eq!(body_b.kind, FieldKind::EntityRef);
    assert_eq!(body_b.type_name, "Option<Entity>");

    // A plain field beside them must be untouched.
    let stiffness = fields.iter().find(|f| f.name == "stiffness").expect("f32");
    assert_eq!(stiffness.kind, FieldKind::F32);
}

#[test]
fn a_live_component_reflects_live_references() {
    let joint = joint();
    assert_eq!(
        joint.reflect_get("body_a"),
        Some(ReflectValue::EntityRef(Some(EntityRef::live(Entity::new(
            4, 1
        ))))),
    );
    assert_eq!(
        joint.reflect_get("body_b"),
        Some(ReflectValue::EntityRef(Some(EntityRef::live(Entity::new(
            9, 3
        ))))),
    );
}

#[test]
fn an_empty_optional_reference_reflects_as_none() {
    let joint = Joint {
        body_b: None,
        ..joint()
    };
    assert_eq!(
        joint.reflect_get("body_b"),
        Some(ReflectValue::EntityRef(None)),
    );
}

#[test]
fn setting_a_live_reference_writes_the_handle() {
    let mut joint = joint();
    let target = Entity::new(11, 2);

    joint
        .reflect_set(
            "body_a",
            ReflectValue::EntityRef(Some(EntityRef::live(target))),
        )
        .expect("a live reference is accepted");

    assert_eq!(joint.body_a, target);
}

/// Clearing means different things for the two shapes, and conflating
/// them would either lose the "no target" state or invent one.
#[test]
fn clearing_a_reference_respects_whether_it_is_optional() {
    let mut joint = joint();

    joint
        .reflect_set("body_b", ReflectValue::EntityRef(None))
        .expect("clearing an optional field is accepted");
    assert_eq!(joint.body_b, None);

    joint
        .reflect_set("body_a", ReflectValue::EntityRef(None))
        .expect("clearing a required field is accepted");
    assert_eq!(joint.body_a, Entity::INVALID);
}

/// The load path must resolve references before writing them back.
/// Storing an unresolved one would leave a component pointing nowhere,
/// so it is rejected rather than silently coerced.
#[test]
fn setting_an_unresolved_reference_is_rejected() {
    let mut joint = joint();
    let original = joint.body_a;
    let unresolved = EntityRef::same_scene(EntityGuid::new(7).expect("non-zero"));

    let error = joint
        .reflect_set("body_a", ReflectValue::EntityRef(Some(unresolved)))
        .expect_err("an unresolved reference must not be stored");

    assert!(matches!(error, ReflectError::UnresolvedEntityRef { .. }));
    assert_eq!(joint.body_a, original, "the field must be left alone");
}

/// The shape an authorable reference uses: it stores the reference, not a
/// handle extracted from it.
#[derive(Debug, Default, Clone, PartialEq, DeriveReflect)]
struct Link {
    target: Option<EntityRef>,
}

impl crate::component::traits::Component for Link {}

#[test]
fn an_entity_ref_field_is_reported_as_an_entity_ref() {
    let meta = Link::default()
        .reflect_fields()
        .iter()
        .find(|f| f.name == "target")
        .copied()
        .expect("target");

    assert_eq!(meta.kind, FieldKind::EntityRef);
    assert_eq!(meta.type_name, "Option<EntityRef>");
}

#[test]
fn an_entity_ref_field_round_trips_a_live_reference() {
    let mut link = Link::default();
    let target = EntityRef::live(Entity::new(6, 2));

    link.reflect_set("target", ReflectValue::EntityRef(Some(target)))
        .expect("a live reference is accepted");

    assert_eq!(link.target, Some(target));
    assert_eq!(
        link.reflect_get("target"),
        Some(ReflectValue::EntityRef(Some(target))),
    );
}

/// The difference that makes this shape worth having: an `Entity` field
/// rejects an unresolved reference because it has nowhere to put one.
/// Losing it would drop the link every time the target's scene is not
/// resident, which under world-cell streaming is ordinary.
#[test]
fn an_entity_ref_field_keeps_an_unresolved_reference() {
    let mut link = Link::default();
    let unresolved = EntityRef::same_scene(EntityGuid::new(4).expect("non-zero"));

    link.reflect_set("target", ReflectValue::EntityRef(Some(unresolved)))
        .expect("an unresolved reference is kept, not rejected");

    assert_eq!(link.target, Some(unresolved));
}

#[test]
fn clearing_an_entity_ref_field_leaves_no_target() {
    let mut link = Link {
        target: Some(EntityRef::live(Entity::new(1, 1))),
    };

    link.reflect_set("target", ReflectValue::EntityRef(None))
        .expect("clearing is accepted");

    assert_eq!(link.target, None);
    assert_eq!(
        link.reflect_get("target"),
        Some(ReflectValue::EntityRef(None)),
    );
}

#[test]
fn setting_a_wrong_value_type_is_a_type_mismatch() {
    let mut joint = joint();
    let error = joint
        .reflect_set("body_a", ReflectValue::U32(3))
        .expect_err("a u32 is not an entity reference");

    assert!(matches!(
        error,
        ReflectError::TypeMismatch {
            expected: FieldKind::EntityRef,
            ..
        },
    ));
}
