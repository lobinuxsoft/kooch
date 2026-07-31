//! Tests for [`warnings_for`](super::warnings_for) and the checks behind it.

use super::*;
use crate::state::ComponentDisplayInfo;
use kooch_ecs::reflect::ReflectValue;
use std::any::TypeId;

fn component(name: &str, fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
    ComponentDisplayInfo {
        type_id: TypeId::of::<()>(),
        component: kooch_ecs::ComponentId(0),
        short_name: name.to_owned(),
        fields: Some(fields),
        field_metas: None,
        visibility: Default::default(),
    }
}

fn rigid_body(kind: u32) -> ComponentDisplayInfo {
    component("RigidBody", vec![("kind".into(), ReflectValue::U32(kind))])
}

fn collider() -> ComponentDisplayInfo {
    component("Collider", Vec::new())
}

fn global(matrix: glam::Mat4) -> ComponentDisplayInfo {
    component(
        "GlobalTransform",
        vec![("matrix".into(), ReflectValue::Mat4(matrix))],
    )
}

fn entity(
    index: u32,
    parent: Option<Entity>,
    components: Vec<ComponentDisplayInfo>,
) -> EntityDisplayInfo {
    EntityDisplayInfo {
        is_prefab_instance: false,
        entity: Entity::new(index, 0),
        components,
        parent,
        children: Vec::new(),
        depth: 0,
        global_rotation: None,
        scene: None,
        parent_global_rotation: None,
    }
}

#[test]
fn a_dynamic_body_under_another_body_is_flagged() {
    let root = entity(
        0,
        None,
        vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
    );
    let child = entity(
        1,
        Some(root.entity),
        vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
    );
    let entities = vec![root, child];

    assert_eq!(
        warnings_for(entities[1].entity, &entities),
        vec![PhysicsWarning::NestedDynamicBody],
    );
}

/// The configuration compound colliders exist for. Warning about it
/// would tell the author to stop doing the right thing.
#[test]
fn a_child_with_only_a_collider_is_not_flagged() {
    let root = entity(
        0,
        None,
        vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
    );
    let child = entity(1, Some(root.entity), vec![collider()]);
    let entities = vec![root, child];

    assert!(warnings_for(entities[1].entity, &entities).is_empty());
}

/// A static or kinematic child is author-driven anyway, so "the
/// solver ignores your parent" is not news about it.
#[test]
fn a_non_dynamic_child_body_is_not_flagged() {
    for kind in [
        kooch_physics::components::KIND_STATIC,
        kooch_physics::components::KIND_KINEMATIC,
    ] {
        let root = entity(
            0,
            None,
            vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
        );
        let child = entity(1, Some(root.entity), vec![rigid_body(kind)]);
        let entities = vec![root, child];

        assert!(
            warnings_for(entities[1].entity, &entities).is_empty(),
            "kind {kind} should not warn",
        );
    }
}

/// The ancestor need not be the immediate parent — an intervening
/// entity with no body does not shield the one above it.
#[test]
fn a_body_ancestor_is_found_through_a_plain_parent() {
    let root = entity(
        0,
        None,
        vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
    );
    let middle = entity(1, Some(root.entity), Vec::new());
    let leaf = entity(
        2,
        Some(middle.entity),
        vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
    );
    let entities = vec![root, middle, leaf];

    assert_eq!(
        warnings_for(entities[2].entity, &entities),
        vec![PhysicsWarning::NestedDynamicBody],
    );
}

/// A root dynamic body is the ordinary case and must stay silent, or
/// the warning becomes noise everyone learns to ignore.
#[test]
fn an_unparented_dynamic_body_is_not_flagged() {
    let entities = vec![entity(
        0,
        None,
        vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
    )];
    assert!(warnings_for(entities[0].entity, &entities).is_empty());
}

#[test]
fn a_sheared_collider_is_flagged() {
    // Non-uniform scale composed with a rotation: the matrix cannot
    // be decomposed back into TRS, so the shape will not match.
    let sheared = glam::Mat4::from_scale(glam::Vec3::new(2.0, 1.0, 1.0))
        * glam::Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4);
    let entities = vec![entity(0, None, vec![collider(), global(sheared)])];

    assert_eq!(
        warnings_for(entities[0].entity, &entities),
        vec![PhysicsWarning::ShearedCollider],
    );
}

#[test]
fn an_unsheared_collider_is_not_flagged() {
    let plain = glam::Mat4::from_scale_rotation_translation(
        glam::Vec3::splat(3.0),
        glam::Quat::from_rotation_y(0.7),
        glam::Vec3::new(1.0, 2.0, 3.0),
    );
    let entities = vec![entity(0, None, vec![collider(), global(plain)])];

    assert!(warnings_for(entities[0].entity, &entities).is_empty());
}

fn joint(fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
    component("Joint", fields)
}

/// A motor switched on with no coefficients is built, accepted, and
/// does nothing — the author has no way to tell it apart from a motor
/// that is off.
#[test]
fn a_motor_with_no_coefficients_is_flagged() {
    let entities = vec![entity(
        0,
        None,
        vec![joint(vec![
            ("motor_enabled".into(), ReflectValue::Bool(true)),
            ("motor_stiffness".into(), ReflectValue::F32(0.0)),
            ("motor_damping".into(), ReflectValue::F32(0.0)),
        ])],
    )];

    assert_eq!(
        warnings_for(entities[0].entity, &entities),
        vec![PhysicsWarning::InertJointMotor],
    );
}

#[test]
fn a_motor_with_either_coefficient_is_not_flagged() {
    for (stiffness, damping) in [(1.0, 0.0), (0.0, 1.0)] {
        let entities = vec![entity(
            0,
            None,
            vec![joint(vec![
                ("motor_enabled".into(), ReflectValue::Bool(true)),
                ("motor_stiffness".into(), ReflectValue::F32(stiffness)),
                ("motor_damping".into(), ReflectValue::F32(damping)),
            ])],
        )];
        assert!(
            warnings_for(entities[0].entity, &entities).is_empty(),
            "stiffness {stiffness} damping {damping} should not warn",
        );
    }
}

/// A motor that is simply off is the ordinary case, and warning about
/// it would make the warning noise.
#[test]
fn a_disabled_motor_is_not_flagged() {
    let entities = vec![entity(
        0,
        None,
        vec![joint(vec![
            ("motor_enabled".into(), ReflectValue::Bool(false)),
            ("motor_stiffness".into(), ReflectValue::F32(0.0)),
            ("motor_damping".into(), ReflectValue::F32(0.0)),
        ])],
    )];
    assert!(warnings_for(entities[0].entity, &entities).is_empty());
}

/// Reduced coordinates produce no constraint impulse, so the threshold
/// has nothing to compare against and the joint never breaks.
#[test]
fn an_articulated_breakable_joint_is_flagged() {
    let entities = vec![entity(
        0,
        None,
        vec![joint(vec![
            ("articulated".into(), ReflectValue::Bool(true)),
            ("breakable".into(), ReflectValue::Bool(true)),
        ])],
    )];

    assert_eq!(
        warnings_for(entities[0].entity, &entities),
        vec![PhysicsWarning::ArticulatedJointCannotBreak],
    );
}

#[test]
fn an_impulse_breakable_joint_is_not_flagged() {
    let entities = vec![entity(
        0,
        None,
        vec![joint(vec![
            ("articulated".into(), ReflectValue::Bool(false)),
            ("breakable".into(), ReflectValue::Bool(true)),
        ])],
    )];
    assert!(warnings_for(entities[0].entity, &entities).is_empty());
}

/// A hierarchy cycle must not hang the UI thread. The Inspector is
/// the wrong place to discover one, so the walk is simply bounded.
#[test]
fn a_parent_cycle_terminates() {
    let a = Entity::new(0, 0);
    let b = Entity::new(1, 0);
    let entities = vec![
        entity(
            0,
            Some(b),
            vec![rigid_body(kooch_physics::components::KIND_DYNAMIC)],
        ),
        entity(1, Some(a), Vec::new()),
    ];

    // The assertion is that this returns at all.
    let _ = warnings_for(a, &entities);
}
