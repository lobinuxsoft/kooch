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
