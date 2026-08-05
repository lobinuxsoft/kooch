//! The ECS walk, against a real component + archetype registry.
//!
//! The interesting failures here are not arithmetic — they are a light
//! that exists and does not reach the buffer. Every assertion below is
//! some version of "the thing on screen and the thing in the Inspector
//! are the same thing".

use std::any::TypeId;

use glam::{Mat4, Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::spot_light::SpotLight;

use kooch_lighting::{LIGHT_KIND_DIRECTIONAL, LIGHT_KIND_POINT, LIGHT_KIND_SPOT, extract_lights};

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());

    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry.register_cpu_reflected::<DirectionalLight>();
    registry.register_cpu_reflected::<PointLight>();
    registry.register_cpu_reflected::<SpotLight>();
    r
}

fn spawn(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

fn insert<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<T>()
    {
        storage.insert(entity, value);
    }
    let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
        return;
    };
    let current = match archetypes.entity_archetype(entity) {
        Some(current) => current,
        None => {
            let empty = archetypes.get_or_create(Default::default());
            archetypes.register_entity(entity, empty);
            empty
        }
    };
    let next = archetypes.archetype_after_add_dynamic(current, TypeId::of::<T>());
    archetypes.register_entity(entity, next);
}

fn light_at<T: Component>(resources: &mut Resources, matrix: Mat4, light: T) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, GlobalTransform { matrix });
    insert(resources, entity, light);
    entity
}

#[test]
fn extracts_every_kind() {
    let mut r = world();
    light_at(&mut r, Mat4::IDENTITY, DirectionalLight::default());
    light_at(
        &mut r,
        Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        PointLight::default(),
    );
    light_at(
        &mut r,
        Mat4::from_translation(Vec3::new(-4.0, 0.0, 0.0)),
        SpotLight::default(),
    );

    let lights = extract_lights(&r);
    assert_eq!(lights.len(), 3);
    assert!(lights.iter().any(|l| l.kind == LIGHT_KIND_DIRECTIONAL));
    assert!(lights.iter().any(|l| l.kind == LIGHT_KIND_POINT));
    assert!(lights.iter().any(|l| l.kind == LIGHT_KIND_SPOT));
}

#[test]
fn a_point_light_carries_its_world_position() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        PointLight::default(),
    );
    let lights = extract_lights(&r);
    assert_eq!(lights[0].position, [1.0, 2.0, 3.0]);
}

/// The scope correction that #441 was rewritten around: the light's
/// direction is its transform's, not a field and not the sky's sun.
/// Rotating the entity has to move the light.
#[test]
fn a_directional_lights_direction_follows_its_transform() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        DirectionalLight::default(),
    );
    let dir = Vec3::from(extract_lights(&r)[0].direction);
    assert!(
        dir.abs_diff_eq(Vec3::NEG_Y, 1e-5),
        "expected -Y, got {dir:?}"
    );
}

#[test]
fn inactive_lights_do_not_reach_the_gpu() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::IDENTITY,
        DirectionalLight {
            active: false,
            ..Default::default()
        },
    );
    assert!(extract_lights(&r).is_empty());
}

/// A light with no transform has no direction and no position. Placing
/// it at the origin pointing down would be an invention, and an
/// invention that renders is one nobody goes looking for.
#[test]
fn a_light_without_a_transform_is_skipped_not_invented() {
    let mut r = world();
    let entity = spawn(&mut r);
    insert(&mut r, entity, PointLight::default());
    assert!(extract_lights(&r).is_empty());
}

#[test]
fn an_empty_world_extracts_nothing_rather_than_panicking() {
    assert!(extract_lights(&world()).is_empty());
}
