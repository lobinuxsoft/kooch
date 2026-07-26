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
}

impl PhysicsWarning {
    /// One line for the panel; the full explanation lives in
    /// [`Self::message`] and appears on hover.
    pub(super) fn summary(self) -> &'static str {
        match self {
            Self::NestedDynamicBody => "Dynamic body will not follow its parent",
            Self::ShearedCollider => "Sheared collider will not match the mesh",
        }
    }

    pub(super) fn message(self) -> &'static str {
        match self {
            Self::NestedDynamicBody => {
                "This RigidBody is dynamic and sits under another body. The solver owns \
                 its pose, so it will NOT follow its parent — no engine supports that \
                 configuration. For one body with several shapes, remove this RigidBody \
                 and keep the Collider; to link two bodies that both simulate, use a \
                 joint. See issue #612."
            }
            Self::ShearedCollider => {
                "This entity's world matrix is sheared — a non-uniform parent scale \
                 composed with a rotated child. Rapier builds shapes from dimensions and \
                 has no sheared form, so the collider is a best-fit approximation and \
                 will not match the mesh. Make the parent's scale uniform, or unrotate \
                 the child. See issues #612 and #214."
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

    warnings
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
