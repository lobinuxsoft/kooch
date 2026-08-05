//! Computing a body's mass from the volume of its collision shapes.
//!
//! The simulation takes `PhysicsBody.mass` literally — shapes carry no mass
//! (#618). That is deterministic, and it gives up the one thing deriving
//! mass from volume was good for: a bigger rock being heavier without
//! anyone typing a number.
//!
//! This buys it back as an authoring action rather than a rule. The button
//! multiplies the author's density by the volume of the shapes that belong
//! to this body and *writes* the answer into `mass`, once. Afterwards the
//! number is the author's: resizing a collider does not silently change
//! what the body weighs, which is exactly what a continuously-derived mass
//! would do.
//!
//! # Which shapes belong to this body
//!
//! The same ones the solver will weld into it: this entity's own collider,
//! plus every descendant's, **stopping at any descendant that carries its
//! own `PhysicsBody`**. That one is an independent body, and the shapes
//! beneath it are its, not ours. `kooch_physics::plugin::compound` applies
//! the identical rule when it builds the compound — if these two ever
//! disagree, the button reports a mass for a body the solver never builds.

use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;

use crate::state::{ComponentDisplayInfo, EntityDisplayInfo};

/// Reads a reflected `f32`.
fn f32_field(component: &ComponentDisplayInfo, name: &str) -> Option<f32> {
    component
        .fields
        .values()?
        .iter()
        .find_map(|(field, value)| match value {
            ReflectValue::F32(number) if field == name => Some(*number),
            _ => None,
        })
}

/// Reads a reflected `u32`.
fn u32_field(component: &ComponentDisplayInfo, name: &str) -> Option<u32> {
    component
        .fields
        .values()?
        .iter()
        .find_map(|(field, value)| match value {
            ReflectValue::U32(number) if field == name => Some(*number),
            _ => None,
        })
}

/// Reads a reflected `Vec3`.
fn vec3_field(component: &ComponentDisplayInfo, name: &str) -> Option<glam::Vec3> {
    component
        .fields
        .values()?
        .iter()
        .find_map(|(field, value)| match value {
            ReflectValue::Vec3(vector) if field == name => Some(*vector),
            _ => None,
        })
}

fn component<'a>(info: &'a EntityDisplayInfo, name: &str) -> Option<&'a ComponentDisplayInfo> {
    info.components.iter().find(|c| c.short_name == name)
}

/// The entity's world-space scale, or ones when it has no transform yet.
///
/// World scale rather than local, because that is what the solver builds
/// the shape at: a body folds its own `Transform.scale` into its shape,
/// and a child's contribution is composed through the hierarchy, which
/// leaves each shape sized by its own world scale either way.
fn world_scale(info: &EntityDisplayInfo) -> glam::Vec3 {
    component(info, "GlobalTransform")
        .and_then(|global| {
            global
                .fields
                .values()?
                .iter()
                .find_map(|(field, value)| match value {
                    ReflectValue::Mat4(matrix) if field == "matrix" => {
                        Some(matrix.to_scale_rotation_translation().0)
                    }
                    _ => None,
                })
        })
        .unwrap_or(glam::Vec3::ONE)
}

/// Volume of one collider in cubic metres, scale folded in.
///
/// The scale rules match `kooch_physics`'s: a box scales per axis exactly, a
/// sphere takes the largest axis, and a capsule takes the larger
/// horizontal for its radius and the vertical for its height. A volume
/// computed from unscaled dimensions would disagree with the shape the
/// solver actually builds, which is worse than no button.
fn collider_volume(collider: &ComponentDisplayInfo, scale: glam::Vec3) -> f32 {
    use std::f32::consts::PI;

    // Mirrors `kooch_physics::components::SHAPE_*`. Matched by value rather
    // than imported so a remote client, which has no Rust type for the
    // project's components, computes the same thing.
    const SPHERE: u32 = 0;
    const CUBOID: u32 = 1;
    const CAPSULE: u32 = 2;

    let s = scale.abs();
    let radius = f32_field(collider, "radius").unwrap_or(0.5);
    let half_height = f32_field(collider, "half_height").unwrap_or(0.5);
    let half_extents = vec3_field(collider, "half_extents").unwrap_or(glam::Vec3::splat(0.5));

    match u32_field(collider, "shape").unwrap_or(SPHERE) {
        CUBOID => {
            let half = half_extents * s;
            8.0 * half.x * half.y * half.z
        }
        CAPSULE => {
            let radius = radius * s.x.max(s.z);
            let half_height = half_height * s.y;
            // A cylinder plus the two hemispherical caps, which together
            // are one sphere.
            PI * radius * radius * 2.0 * half_height + (4.0 / 3.0) * PI * radius.powi(3)
        }
        _ => (4.0 / 3.0) * PI * (radius * s.max_element()).powi(3),
    }
    .abs()
}

/// Total volume of the shapes that will be welded into `entity`'s body.
///
/// `None` when there are none — which is what greys the button out. An
/// author who has added a `PhysicsBody` and no `Collider` yet should be told
/// what is missing, not handed a mass of zero.
pub(super) fn collider_volume_for(entity: Entity, entities: &[EntityDisplayInfo]) -> Option<f32> {
    let find = |target: Entity| entities.iter().find(|e| e.entity == target);
    let root = find(entity)?;

    let mut total = 0.0;
    let mut found = false;
    let mut add = |info: &EntityDisplayInfo| {
        if let Some(collider) = component(info, "Collider") {
            total += collider_volume(collider, world_scale(info));
            found = true;
        }
    };
    add(root);

    // Bounded by the entity count: a cycle in the hierarchy must not hang
    // the UI thread, and the Inspector is the wrong place to discover one.
    let mut stack: Vec<Entity> = root.children.clone();
    let mut visited = 0;
    while let Some(next) = stack.pop() {
        visited += 1;
        if visited > entities.len() {
            break;
        }
        let Some(info) = find(next) else { continue };
        // Its own body: an independent simulation. Not ours, and neither
        // is anything beneath it.
        if component(info, "PhysicsBody").is_some() {
            continue;
        }
        add(info);
        stack.extend(info.children.iter().copied());
    }

    found.then_some(total)
}

/// The mass the button would write, or `None` when there is nothing to
/// measure.
pub(super) fn mass_from_colliders(entity: Entity, entities: &[EntityDisplayInfo]) -> Option<f32> {
    let density = entities
        .iter()
        .find(|e| e.entity == entity)
        .and_then(|info| component(info, "PhysicsBody"))
        .and_then(|body| f32_field(body, "density"))
        .unwrap_or(1000.0);
    let volume = collider_volume_for(entity, entities)?;
    Some(density * volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ReflectedFields;
    use std::any::TypeId;
    use std::f32::consts::PI;

    fn component_with(name: &str, fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
        ComponentDisplayInfo {
            type_id: TypeId::of::<()>(),
            component: kooch_ecs::ComponentId(0),
            short_name: name.to_owned().into(),
            fields: ReflectedFields::Values(fields),
            field_metas: None,
            visibility: Default::default(),
        }
    }

    fn sphere(radius: f32) -> ComponentDisplayInfo {
        component_with(
            "Collider",
            vec![
                ("shape".into(), ReflectValue::U32(0)),
                ("radius".into(), ReflectValue::F32(radius)),
            ],
        )
    }

    fn cuboid(half: glam::Vec3) -> ComponentDisplayInfo {
        component_with(
            "Collider",
            vec![
                ("shape".into(), ReflectValue::U32(1)),
                ("half_extents".into(), ReflectValue::Vec3(half)),
            ],
        )
    }

    fn physics_body(density: f32) -> ComponentDisplayInfo {
        component_with(
            "PhysicsBody",
            vec![("density".into(), ReflectValue::F32(density))],
        )
    }

    fn global(scale: glam::Vec3) -> ComponentDisplayInfo {
        component_with(
            "GlobalTransform",
            vec![(
                "matrix".into(),
                ReflectValue::Mat4(glam::Mat4::from_scale(scale)),
            )],
        )
    }

    fn entity(
        index: u32,
        children: Vec<Entity>,
        components: Vec<ComponentDisplayInfo>,
    ) -> EntityDisplayInfo {
        EntityDisplayInfo {
            is_prefab_instance: false,
            entity: Entity::new(index, 0),
            components,
            parent: None,
            children,
            depth: 0,
            global_rotation: None,
            scene: None,
            parent_global_rotation: None,
        }
    }

    #[test]
    fn a_sphere_is_measured_as_a_sphere() {
        let entities = vec![entity(
            0,
            Vec::new(),
            vec![physics_body(1000.0), sphere(0.5)],
        )];
        let volume = collider_volume_for(entities[0].entity, &entities).expect("has a collider");
        assert!((volume - (4.0 / 3.0) * PI * 0.125).abs() < 1e-5, "{volume}");
    }

    #[test]
    fn a_cuboid_is_measured_as_a_box() {
        let entities = vec![entity(
            0,
            Vec::new(),
            vec![physics_body(1000.0), cuboid(glam::Vec3::new(1.0, 2.0, 0.5))],
        )];
        let volume = collider_volume_for(entities[0].entity, &entities).expect("has a collider");
        assert!((volume - 8.0).abs() < 1e-5, "{volume}");
    }

    /// A cube scaled by two has eight times the volume, and a button that
    /// ignored that would report a mass for a shape nobody can see.
    #[test]
    fn scale_is_folded_into_the_volume() {
        let entities = vec![entity(
            0,
            Vec::new(),
            vec![
                physics_body(1000.0),
                cuboid(glam::Vec3::splat(0.5)),
                global(glam::Vec3::splat(2.0)),
            ],
        )];
        let volume = collider_volume_for(entities[0].entity, &entities).expect("has a collider");
        assert!((volume - 8.0).abs() < 1e-4, "{volume}");
    }

    #[test]
    fn descendant_colliders_are_included() {
        let child = Entity::new(1, 0);
        let entities = vec![
            entity(
                0,
                vec![child],
                vec![physics_body(1000.0), cuboid(glam::Vec3::splat(0.5))],
            ),
            entity(1, Vec::new(), vec![cuboid(glam::Vec3::splat(0.5))]),
        ];
        let volume = collider_volume_for(entities[0].entity, &entities).expect("has colliders");
        assert!((volume - 2.0).abs() < 1e-5, "{volume}");
    }

    /// A descendant with its own body is a separate simulation. Counting
    /// its shapes would report a mass for a body the solver never builds.
    #[test]
    fn a_descendant_with_its_own_body_is_not_counted() {
        let child = Entity::new(1, 0);
        let grandchild = Entity::new(2, 0);
        let entities = vec![
            entity(
                0,
                vec![child],
                vec![physics_body(1000.0), cuboid(glam::Vec3::splat(0.5))],
            ),
            entity(
                1,
                vec![grandchild],
                vec![physics_body(1000.0), cuboid(glam::Vec3::splat(0.5))],
            ),
            // Beneath the nested body, so it belongs to that one too.
            entity(2, Vec::new(), vec![cuboid(glam::Vec3::splat(0.5))]),
        ];
        let volume = collider_volume_for(entities[0].entity, &entities).expect("has a collider");
        assert!((volume - 1.0).abs() < 1e-5, "{volume}");
    }

    /// Nothing to measure: the button greys out rather than writing zero.
    #[test]
    fn a_body_with_no_collider_anywhere_has_no_volume() {
        let child = Entity::new(1, 0);
        let entities = vec![
            entity(0, vec![child], vec![physics_body(1000.0)]),
            entity(1, Vec::new(), Vec::new()),
        ];
        assert_eq!(collider_volume_for(entities[0].entity, &entities), None);
    }

    /// A collider on a child alone is enough — the author asked for this
    /// case specifically.
    #[test]
    fn a_collider_only_on_a_child_still_enables_the_button() {
        let child = Entity::new(1, 0);
        let entities = vec![
            entity(0, vec![child], vec![physics_body(1000.0)]),
            entity(1, Vec::new(), vec![sphere(0.5)]),
        ];
        assert!(collider_volume_for(entities[0].entity, &entities).is_some());
    }

    #[test]
    fn mass_is_density_times_volume() {
        let entities = vec![entity(
            0,
            Vec::new(),
            vec![physics_body(7850.0), cuboid(glam::Vec3::splat(0.5))],
        )];
        let mass = mass_from_colliders(entities[0].entity, &entities).expect("has a collider");
        assert!(
            (mass - 7850.0).abs() < 1e-2,
            "a steel cubic metre is {mass}"
        );
    }

    /// A cycle in the hierarchy must not hang the UI thread.
    #[test]
    fn a_hierarchy_cycle_terminates() {
        let a = Entity::new(0, 0);
        let b = Entity::new(1, 0);
        let entities = vec![
            entity(0, vec![b], vec![physics_body(1000.0), sphere(0.5)]),
            entity(1, vec![a], vec![sphere(0.5)]),
        ];
        // The assertion is that this returns at all.
        let _ = collider_volume_for(a, &entities);
    }
}
