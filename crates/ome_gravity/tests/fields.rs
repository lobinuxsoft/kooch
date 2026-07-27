//! Gravity fields against a real solver: #624's acceptance list.
//!
//! An integration test rather than unit tests over the maths, because the
//! thing worth checking is that a body *falls the right way* — the maths
//! being right and the impulse reaching the solver are two different
//! claims, and only the second one is new.

use glam::Vec3;

use ome_core::resource::Resources;
use ome_core::run_state::Playing;
use ome_core::time::Time;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::{Component, ComponentRegistry};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::entity::Entity;
use ome_ecs::query::AccessTracker;
use ome_ecs::transform::Transform;
use ome_gravity::{AreaGravity, BoxGravity, GlobalGravity, PointGravity, plugin};
use ome_physics::components::{Collider, KIND_STATIC, RigidBody, SHAPE_CUBOID};
use ome_physics::plugin::{
    PhysicsWorld, physics_step_system, physics_sync_system, physics_writeback_system,
};
use ome_physics::rapier_backend::RapierBackend;

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(Time::new());
    r.insert(PhysicsWorld::new(Box::new(RapierBackend::new())));

    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<RigidBody>();
    registry.register_cpu_reflected::<Collider>();
    registry.register_cpu::<ome_physics::plugin::PhysicsBody>();
    registry.register_cpu_reflected::<GlobalGravity>();
    registry.register_cpu_reflected::<PointGravity>();
    registry.register_cpu_reflected::<AreaGravity>();
    registry.register_cpu_reflected::<BoxGravity>();
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
    use std::any::TypeId;
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

fn body_at(resources: &mut Resources, position: Vec3) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, Transform::from_position(position));
    insert(resources, entity, RigidBody::default());
    insert(resources, entity, Collider::default());
    entity
}

fn source_at<T: Component>(resources: &mut Resources, transform: Transform, source: T) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, transform);
    insert(resources, entity, source);
    entity
}

fn position(resources: &Resources, entity: Entity) -> Vec3 {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(entity))
        .map(|t| t.position)
        .expect("no transform")
}

/// The frame the gravity plugin schedules.
fn simulate(resources: &mut Resources, steps: u32) {
    for _ in 0..steps {
        plugin::reconcile_world_gravity_for_test(resources);
        physics_sync_system(resources);
        if Playing::is_playing(resources) {
            plugin::apply_gravity_sources(resources);
            physics_step_system(resources);
            physics_writeback_system(resources);
        }
    }
}

/// Acceptance: "bodies dropped around a point source fall towards it from
/// every direction, not downward."
#[test]
fn a_point_source_pulls_from_every_direction() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::default(),
        PointGravity {
            strength: 20.0,
            radius: 10.0,
            range: 0.0,
            inverse_square: false,
        },
    );

    let starts = [
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(-10.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 10.0),
        Vec3::new(0.0, 10.0, 0.0),
    ];
    let bodies: Vec<Entity> = starts
        .iter()
        .map(|&start| body_at(&mut resources, start))
        .collect();

    Playing::set(&mut resources, true);
    simulate(&mut resources, 60);

    for (body, start) in bodies.iter().zip(starts) {
        let now = position(&resources, *body);
        assert!(
            now.length() < start.length() - 0.5,
            "a body at {start} did not fall towards the planet; it is at {now}",
        );
        // Falling *down* rather than *inwards* would show as the +X body
        // gaining -Y instead of -X. This is the assertion that separates a
        // field from a world vector.
        assert!(
            (now - start).normalize().dot(-start.normalize()) > 0.9,
            "a body at {start} moved to {now}, which is not towards the centre",
        );
    }
}

/// Acceptance: "a body inside two overlapping sources feels their sum, and
/// neither one twice."
#[test]
fn overlapping_sources_sum() {
    fn drift(with_second: bool) -> Vec3 {
        let mut resources = world();
        source_at(
            &mut resources,
            Transform::default(),
            GlobalGravity {
                acceleration: Vec3::new(0.0, -10.0, 0.0),
            },
        );
        if with_second {
            source_at(
                &mut resources,
                Transform::default(),
                GlobalGravity {
                    acceleration: Vec3::new(10.0, 0.0, 0.0),
                },
            );
        }
        let body = body_at(&mut resources, Vec3::new(0.0, 100.0, 0.0));
        Playing::set(&mut resources, true);
        simulate(&mut resources, 60);
        position(&resources, body) - Vec3::new(0.0, 100.0, 0.0)
    }

    let one = drift(false);
    let two = drift(true);

    assert!(
        one.x.abs() < 1e-3,
        "one field should not push sideways: {one}",
    );
    assert!(
        (two.x + one.y).abs() < 0.05,
        "two equal fields at right angles should drift equally on both axes: {two}",
    );
    // The second field must not change the first: "neither one twice".
    assert!(
        (two.y - one.y).abs() < 0.05,
        "adding a sideways field changed the downward fall: {one} then {two}",
    );
}

/// The world vector and a source must not both apply, or a planet pulls
/// diagonally.
#[test]
fn the_world_vector_is_off_while_a_source_exists() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::default(),
        PointGravity::default(),
    );

    plugin::reconcile_world_gravity_for_test(&mut resources);

    assert_eq!(
        resources.get::<PhysicsWorld>().unwrap().backend().gravity(),
        Vec3::ZERO,
    );
}

/// And a scene with no sources keeps the vector it always had, so adding
/// the plugin changes nothing until something asks it to.
#[test]
fn a_scene_without_sources_is_untouched() {
    let mut resources = world();
    plugin::reconcile_world_gravity_for_test(&mut resources);

    assert_eq!(
        resources.get::<PhysicsWorld>().unwrap().backend().gravity(),
        Vec3::new(0.0, -9.81, 0.0),
    );
}

/// Phase A still applies in phase B. Rapier's own gravity is off while a
/// source exists, so its `gravity_scale` multiplies nothing — the field
/// system has to honour the multiplier itself or it would silently stop
/// working the moment anyone added a planet.
#[test]
fn the_per_body_scale_still_applies_to_fields() {
    fn fall(gravity_scale: f32) -> f32 {
        let mut resources = world();
        source_at(
            &mut resources,
            Transform::default(),
            GlobalGravity::default(),
        );

        let body = spawn(&mut resources);
        insert(
            &mut resources,
            body,
            Transform::from_position(Vec3::new(0.0, 100.0, 0.0)),
        );
        insert(
            &mut resources,
            body,
            RigidBody {
                gravity_scale,
                ..Default::default()
            },
        );
        insert(&mut resources, body, Collider::default());

        Playing::set(&mut resources, true);
        simulate(&mut resources, 60);
        100.0 - position(&resources, body).y
    }

    assert!(
        fall(0.0).abs() < 1e-3,
        "a weightless body fell {}",
        fall(0.0),
    );
    let normal = fall(1.0);
    let double = fall(2.0);
    assert!(normal > 1.0, "the normal body barely fell: {normal}");
    assert!(
        (double / normal - 2.0).abs() < 0.05,
        "scale 2 fell {double} against {normal}",
    );
}

/// An area's direction is in its own space, so turning the entity turns
/// which way is down inside it. A level that flips over is one rotation.
#[test]
fn an_area_field_rotates_with_its_entity() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform {
            rotation: glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            ..Default::default()
        },
        AreaGravity {
            half_extents: Vec3::splat(50.0),
            ..Default::default()
        },
    );

    let body = body_at(&mut resources, Vec3::ZERO);
    Playing::set(&mut resources, true);
    simulate(&mut resources, 60);

    // Local -Y turned a quarter turn about +Z points along +X.
    let moved = position(&resources, body);
    assert!(
        moved.x > 0.1 && moved.y.abs() < 0.05,
        "a rotated zone should pull along +X, body went to {moved}",
    );
}

/// A body outside every field is left alone — not handed a zero impulse,
/// which would wake it every step for nothing.
#[test]
fn a_body_beyond_every_source_does_not_move() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::default(),
        PointGravity {
            range: 10.0,
            ..Default::default()
        },
    );

    let far = body_at(&mut resources, Vec3::new(0.0, 500.0, 0.0));
    Playing::set(&mut resources, true);
    simulate(&mut resources, 60);

    assert!(
        (position(&resources, far).y - 500.0).abs() < 1e-3,
        "a body out of range moved to {}",
        position(&resources, far),
    );
}

/// A cube planet: each face pulls along its own normal, so where a body
/// falls depends on which face it started over. This is the whole claim,
/// and it is one a world gravity vector cannot make.
#[test]
fn a_box_source_pulls_towards_the_nearest_face() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::default(),
        BoxGravity {
            half_extents: Vec3::splat(10.0),
            rounding: 0.0,
            range: 0.0,
            falloff: 0.0,
            ..Default::default()
        },
    );

    // One body over each face, off-centre so "fell straight down" and
    // "fell towards the middle of the planet" give different answers.
    let starts = [
        (Vec3::new(3.0, 15.0, 0.0), Vec3::NEG_Y),
        (Vec3::new(15.0, 3.0, 0.0), Vec3::NEG_X),
        (Vec3::new(0.0, -15.0, 3.0), Vec3::Y),
        (Vec3::new(0.0, 3.0, -15.0), Vec3::Z),
    ];
    let bodies: Vec<Entity> = starts
        .iter()
        .map(|&(start, _)| body_at(&mut resources, start))
        .collect();

    Playing::set(&mut resources, true);
    simulate(&mut resources, 30);

    for (body, (start, wanted)) in bodies.iter().zip(starts) {
        let moved = position(&resources, *body) - start;
        assert!(moved.length() > 0.1, "a body at {start} did not fall");
        assert!(
            moved.normalize().dot(wanted) > 0.99,
            "a body at {start} moved {moved}, not along {wanted}",
        );
    }
}

/// The zone rotates with its entity, the same as an area: a cube planet
/// turned on its side has its faces somewhere else.
#[test]
fn a_box_source_rotates_with_its_entity() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform {
            rotation: glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
            ..Default::default()
        },
        BoxGravity {
            half_extents: Vec3::new(10.0, 2.0, 10.0),
            rounding: 0.0,
            range: 0.0,
            falloff: 0.0,
            ..Default::default()
        },
    );

    // Straight up from a slab turned 45°: the nearest face is the tilted
    // top, so the pull is along its tilted normal rather than straight down.
    let body = body_at(&mut resources, Vec3::new(0.0, 15.0, 0.0));
    Playing::set(&mut resources, true);
    simulate(&mut resources, 30);

    let moved = position(&resources, body) - Vec3::new(0.0, 15.0, 0.0);
    let expected = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_4) * Vec3::NEG_Y;
    assert!(
        moved.normalize().dot(expected) > 0.99,
        "a slab turned 45° pulled along {}, wanted {expected}",
        moved.normalize(),
    );
}

/// A resting body has to be allowed to fall asleep.
///
/// Rapier excludes a sleeping body from the island solver — that is how a
/// scene of settled crates costs nothing. A field that hands every dynamic
/// body an impulse every step, waking it to do so, turns off sleeping for
/// the whole world: the CPU then simulates a pile of boxes that have not
/// moved in a minute, forever.
///
/// The world vector never had this problem, because rapier's own gravity
/// does not wake anything. Matching that is the point.
#[test]
fn a_settled_body_still_falls_asleep() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::default(),
        GlobalGravity::default(),
    );

    // A floor to settle on, and something to settle.
    let floor = spawn(&mut resources);
    insert(&mut resources, floor, Transform::default());
    insert(
        &mut resources,
        floor,
        RigidBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        floor,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(20.0, 0.5, 20.0),
            ..Default::default()
        },
    );

    let crate_body = body_at(&mut resources, Vec3::new(0.0, 1.2, 0.0));

    Playing::set(&mut resources, true);
    // Long enough to fall, bounce out its energy, and cross rapier's sleep
    // timer, which is a wall-clock threshold rather than a step count.
    simulate(&mut resources, 600);

    let handle = resources
        .get::<PhysicsWorld>()
        .and_then(|w| w.iter().find(|(_, e, _, _)| *e == crate_body).map(|t| t.3))
        .expect("the crate has no body");
    let sleeping = resources
        .get::<PhysicsWorld>()
        .and_then(|w| w.backend().is_sleeping(handle))
        .expect("stale handle");

    assert!(
        sleeping,
        "the crate is still awake after ten seconds of resting on a floor \
         — the field is waking every body every step, so nothing in the \
         scene ever sleeps",
    );
}

/// …but a field that changes has to reach what has already settled.
///
/// The counterweight to the test above. Never waking anything is cheap and
/// wrong: switching a gravity zone on, or moving a planet, would leave the
/// crates already lying there asleep and floating. So the step that sees a
/// changed field wakes what it pulls on, and only that step.
#[test]
fn a_moved_source_wakes_what_it_pulls_on() {
    let mut resources = world();
    let planet = source_at(
        &mut resources,
        Transform::default(),
        GlobalGravity::default(),
    );

    let floor = spawn(&mut resources);
    insert(&mut resources, floor, Transform::default());
    insert(
        &mut resources,
        floor,
        RigidBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        floor,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(20.0, 0.5, 20.0),
            ..Default::default()
        },
    );

    let crate_body = body_at(&mut resources, Vec3::new(0.0, 1.2, 0.0));
    Playing::set(&mut resources, true);
    simulate(&mut resources, 600);

    let handle = |resources: &Resources| {
        resources
            .get::<PhysicsWorld>()
            .and_then(|w| w.iter().find(|(_, e, _, _)| *e == crate_body).map(|t| t.3))
            .expect("the crate has no body")
    };
    let sleeping = |resources: &Resources| {
        let handle = handle(resources);
        resources
            .get::<PhysicsWorld>()
            .and_then(|w| w.backend().is_sleeping(handle))
            .expect("stale handle")
    };
    assert!(
        sleeping(&resources),
        "it never settled, so the test is moot"
    );

    // Gravity flips upward. A crate that stays asleep through that is a
    // crate glued to the floor.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<GlobalGravity>()
        && let Some(field) = storage.get_mut(planet)
    {
        field.acceleration = Vec3::new(0.0, 9.81, 0.0);
    }
    simulate(&mut resources, 2);

    assert!(
        !sleeping(&resources),
        "the field changed and the settled crate slept through it",
    );
}
