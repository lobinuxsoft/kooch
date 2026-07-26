//! Inspector warnings for physics configurations Rapier cannot honour.
//!
//! The project's rule is to implement what Rapier offers and **warn** for
//! what it does not, rather than building around the solver. A warning is
//! only useful where the author is looking, though, and that is the
//! Inspector — not the log.
//!
//! Godot does the same thing on the node; these are the two cases #612
//! turned up.

use ome_ecs::entity::Entity;

use crate::state::EntityDisplayInfo;

/// A configuration worth telling the author about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PhysicsWarning {
    /// A dynamic body under another body. The solver owns its pose, so it
    /// will not follow the parent — the case no engine supports.
    NestedDynamicBody,
    /// A collider whose world matrix is sheared. Rapier's shapes are built
    /// from dimensions and cannot be sheared, so the collider will not
    /// match the mesh.
    ShearedCollider,
    /// A joint whose motor is switched on with both coefficients at zero.
    /// It contributes nothing to the solve, which is indistinguishable
    /// from a motor that is off — except that the author thinks it is on.
    InertJointMotor,
    /// A joint asked to be both articulated and breakable. Reduced
    /// coordinates produce no constraint impulse, so there is nothing to
    /// compare a threshold against.
    ArticulatedJointCannotBreak,
}

impl PhysicsWarning {
    /// One line for the panel; the full explanation lives in
    /// [`Self::message`] and appears on hover.
    pub(super) fn summary(self) -> &'static str {
        match self {
            Self::NestedDynamicBody => "Dynamic body will not follow its parent",
            Self::ShearedCollider => "Sheared collider will not match the mesh",
            Self::InertJointMotor => "Motor is on but does nothing",
            Self::ArticulatedJointCannotBreak => "Articulated joints cannot break",
        }
    }

    pub(super) fn message(self) -> &'static str {
        match self {
            Self::NestedDynamicBody => {
                "This RigidBody is dynamic and sits under another body. The solver owns \
                 its pose, so it will NOT follow its parent — no engine supports that \
                 configuration. For one body with several shapes, remove this RigidBody \
                 and keep the Collider; to link two bodies that both simulate, add a \
                 Joint component naming them both. See issues #612 and #560."
            }
            Self::ShearedCollider => {
                "This entity's world matrix is sheared — a non-uniform parent scale \
                 composed with a rotated child. Rapier builds shapes from dimensions and \
                 has no sheared form, so the collider is a best-fit approximation and \
                 will not match the mesh. Make the parent's scale uniform, or unrotate \
                 the child. See issues #612 and #214."
            }
            Self::InertJointMotor => {
                "This Joint's motor is enabled, but both Motor Stiffness and Motor \
                 Damping are zero — so it applies nothing and the joint behaves exactly \
                 as if the motor were off. Stiffness pulls towards Motor Target \
                 Position; damping holds Motor Target Velocity. Set whichever one you \
                 meant."
            }
            Self::ArticulatedJointCannotBreak => {
                "This Joint is both Articulated and Breakable. An articulated joint is \
                 solved in reduced coordinates, where the stretched configuration is not \
                 representable — so there is no constraint impulse to compare Break \
                 Impulse against, and it will never break. Turn off Articulated to use a \
                 break threshold."
            }
        }
    }
}

/// Which warnings apply to `entity`.
///
/// Reads the display snapshot rather than the ECS: the Inspector runs
/// inside the egui pass, where `Resources` is already borrowed.
pub(super) fn warnings_for(entity: Entity, entities: &[EntityDisplayInfo]) -> Vec<PhysicsWarning> {
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        return Vec::new();
    };

    let mut warnings = Vec::new();

    if is_dynamic_body(info) && has_body_ancestor(info, entities) {
        warnings.push(PhysicsWarning::NestedDynamicBody);
    }

    if has_collider(info) && is_sheared(info) {
        warnings.push(PhysicsWarning::ShearedCollider);
    }

    warnings.extend(joint_warnings(info));

    warnings
}

/// The joint mistakes an author cannot see from the panel.
///
/// Both are configurations the solver accepts and then quietly ignores,
/// which is the worst kind: the joint is built, nothing errors, and the
/// behaviour is simply absent.
fn joint_warnings(info: &EntityDisplayInfo) -> Vec<PhysicsWarning> {
    let Some(fields) = info
        .components
        .iter()
        .find(|component| component.short_name == "Joint")
        .and_then(|component| component.fields.as_ref())
    else {
        return Vec::new();
    };

    let mut warnings = Vec::new();
    if flag(fields, "motor_enabled")
        && number(fields, "motor_stiffness") == Some(0.0)
        && number(fields, "motor_damping") == Some(0.0)
    {
        warnings.push(PhysicsWarning::InertJointMotor);
    }
    if flag(fields, "articulated") && flag(fields, "breakable") {
        warnings.push(PhysicsWarning::ArticulatedJointCannotBreak);
    }
    warnings
}

/// Reads a reflected `bool` field, absent reading as `false`.
fn flag(fields: &[(String, ome_ecs::reflect::ReflectValue)], name: &str) -> bool {
    use ome_ecs::reflect::ReflectValue;

    fields
        .iter()
        .any(|(field, value)| field == name && matches!(value, ReflectValue::Bool(true)))
}

/// Reads a reflected `f32` field.
fn number(fields: &[(String, ome_ecs::reflect::ReflectValue)], name: &str) -> Option<f32> {
    use ome_ecs::reflect::ReflectValue;

    fields.iter().find_map(|(field, value)| match value {
        ReflectValue::F32(number) if field == name => Some(*number),
        _ => None,
    })
}

/// Matched by `short_name`, which is what the display snapshot carries —
/// a remote client has no Rust type for a project's components, so the
/// name is all there is on either side of the wire.
fn has_component(info: &EntityDisplayInfo, name: &str) -> bool {
    info.components
        .iter()
        .any(|component| component.short_name == name)
}

fn has_collider(info: &EntityDisplayInfo) -> bool {
    has_component(info, "Collider")
}

/// Whether the entity carries a `RigidBody` whose kind is dynamic.
///
/// Static and kinematic bodies are deliberately not warned about: they
/// are author-driven anyway, so "the solver ignores your parent" is not
/// news about them.
fn is_dynamic_body(info: &EntityDisplayInfo) -> bool {
    use ome_ecs::reflect::ReflectValue;

    info.components
        .iter()
        .filter(|component| component.short_name == "RigidBody")
        .filter_map(|component| component.fields.as_ref())
        .flatten()
        .any(|(name, value)| {
            name == "kind"
                && matches!(
                    value,
                    ReflectValue::U32(kind) if *kind == ome_physics::components::KIND_DYNAMIC
                )
        })
}

/// Walks up the parent chain looking for another `RigidBody`.
///
/// Bounded by the number of entities: a cycle in the hierarchy would
/// otherwise hang the UI thread, and the Inspector is the wrong place to
/// discover one.
fn has_body_ancestor(info: &EntityDisplayInfo, entities: &[EntityDisplayInfo]) -> bool {
    let mut current = info.parent;
    for _ in 0..entities.len() {
        let Some(entity) = current else {
            return false;
        };
        let Some(ancestor) = entities.iter().find(|e| e.entity == entity) else {
            return false;
        };
        if has_component(ancestor, "RigidBody") {
            return true;
        }
        current = ancestor.parent;
    }
    false
}

/// Whether the entity's world matrix carries shear.
///
/// Uses the same detector and epsilon the Transform readout already uses
/// for #214, so the two cannot disagree about the same matrix.
fn is_sheared(info: &EntityDisplayInfo) -> bool {
    use ome_ecs::reflect::ReflectValue;

    info.components
        .iter()
        .filter_map(|component| component.fields.as_ref())
        .flatten()
        .any(|(name, value)| match value {
            ReflectValue::Mat4(matrix) if name == "matrix" => {
                ome_ecs::hierarchy::GlobalTransform { matrix: *matrix }.has_shear(1e-4)
            }
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ComponentDisplayInfo;
    use ome_ecs::reflect::ReflectValue;
    use std::any::TypeId;

    fn component(name: &str, fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
        ComponentDisplayInfo {
            type_id: TypeId::of::<()>(),
            component: ome_ecs::ComponentId(0),
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
            vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
        );
        let child = entity(
            1,
            Some(root.entity),
            vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
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
            vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
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
            ome_physics::components::KIND_STATIC,
            ome_physics::components::KIND_KINEMATIC,
        ] {
            let root = entity(
                0,
                None,
                vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
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
            vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
        );
        let middle = entity(1, Some(root.entity), Vec::new());
        let leaf = entity(
            2,
            Some(middle.entity),
            vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
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
            vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
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
                vec![rigid_body(ome_physics::components::KIND_DYNAMIC)],
            ),
            entity(1, Some(a), Vec::new()),
        ];

        // The assertion is that this returns at all.
        let _ = warnings_for(a, &entities);
    }
}
