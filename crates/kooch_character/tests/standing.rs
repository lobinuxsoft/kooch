//! #94's acceptance list against a real solver: only this shows a character actually stands.

use glam::Vec3;

use kooch_character::plugin::{cling_and_leap, hold_characters};
use kooch_character::{
    CharacterController, Facing, Grounded, Jump, Sprint, Touching, Walk, WallJump, WallRun,
    WallSlide,
};
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_core::time::Time;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::transform::Transform;
use kooch_gravity::{GlobalGravity, PointGravity, plugin};
use kooch_physics::components::{Collider, KIND_STATIC, PhysicsBody, SHAPE_CUBOID};
use kooch_physics::plugin::{
    PhysicsWorld, physics_step_system, physics_sync_system, physics_writeback_system,
};
use kooch_physics::rapier_backend::RapierBackend;

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
    registry.register_cpu_reflected::<PhysicsBody>();
    registry.register_cpu_reflected::<Collider>();
    registry.register_cpu::<kooch_physics::plugin::SolverBody>();
    registry.register_cpu_reflected::<GlobalGravity>();
    registry.register_cpu_reflected::<PointGravity>();
    registry.register_cpu_reflected::<CharacterController>();
    registry.register_cpu_reflected::<Facing>();
    registry.register_cpu_reflected::<Grounded>();
    registry.register_cpu_reflected::<Jump>();
    registry.register_cpu_reflected::<Sprint>();
    registry.register_cpu_reflected::<Touching>();
    registry.register_cpu_reflected::<WallJump>();
    registry.register_cpu_reflected::<WallRun>();
    registry.register_cpu_reflected::<WallSlide>();
    registry.register_cpu_reflected::<Walk>();
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
    insert(resources, entity, PhysicsBody::default());
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

/// The frame the gravity and character plugins schedule: the character after gravity and before the
/// step, so the spring fights this step's gravity.
fn simulate(resources: &mut Resources, steps: u32) {
    for _ in 0..steps {
        plugin::reconcile_world_gravity_for_test(resources);
        physics_sync_system(resources);
        if Playing::is_playing(resources) {
            plugin::apply_gravity_sources(resources);
            hold_characters(resources);
            cling_and_leap(resources);
            physics_step_system(resources);
            physics_writeback_system(resources);
        }
    }
}

/// A capsule that holds itself up, at `position`.
fn character(resources: &mut Resources, position: Vec3) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, Transform::from_position(position));
    insert(resources, entity, PhysicsBody::default());
    insert(
        resources,
        entity,
        Collider {
            shape: kooch_physics::components::SHAPE_CAPSULE,
            radius: 0.4,
            half_height: 0.5,
            ..Default::default()
        },
    );
    insert(resources, entity, CharacterController::default());
    insert(resources, entity, Grounded::default());
    entity
}

/// A slab to stand on, centred at `at`.
fn floor(resources: &mut Resources, at: Vec3, half: Vec3) -> Entity {
    slab(resources, at, half, glam::Quat::IDENTITY)
}

/// The same, turned — a wall is a floor stood on its edge.
fn slab(resources: &mut Resources, at: Vec3, half: Vec3, rotation: glam::Quat) -> Entity {
    let entity = spawn(resources);
    insert(
        resources,
        entity,
        Transform {
            position: at,
            rotation,
            scale: Vec3::ONE,
        },
    );
    insert(
        resources,
        entity,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        resources,
        entity,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: half,
            ..Default::default()
        },
    );
    entity
}

fn grounded(resources: &Resources, entity: Entity) -> Grounded {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Grounded>())
        .and_then(|s| s.get(entity))
        .copied()
        .expect("no Grounded")
}

/// A flat world with a character standing on it, settled.
fn on_the_floor(resources: &mut Resources) -> Entity {
    Playing::set(resources, true);
    source_at(
        resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
    );
    let hero = character(resources, Vec3::new(0.0, 0.5, 0.0));
    simulate(resources, 180);
    hero
}

/// Steers a character for `steps` frames and returns its ground speed.
fn walked(resources: &mut Resources, hero: Entity, steered: Vec3, steps: u32) -> f32 {
    insert(resources, hero, Facing { direction: steered });
    simulate(resources, steps);
    let velocity = resources
        .get::<PhysicsWorld>()
        .and_then(|world| {
            let body = resources
                .get::<ComponentRegistry>()
                .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
                .and_then(|s| s.get(hero))
                .copied()?;
            world.linear_velocity(body)
        })
        .unwrap_or(Vec3::ZERO);
    Vec3::new(velocity.x, 0.0, velocity.z).length()
}

/// A tall wall with its face at `x`, and a character beside it.
fn against_a_wall(resources: &mut Resources) -> Entity {
    Playing::set(resources, true);
    source_at(
        resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    // Far enough down that two seconds of falling never reaches it — a
    // character that lands mid-test measures the floor, not the wall.
    floor(
        resources,
        Vec3::new(0.0, -200.0, 0.0),
        Vec3::new(40.0, 0.5, 40.0),
    );
    // Face at x = 1, tall enough to fall down all of and long enough to
    // run along without reaching the end.
    let wall = spawn(resources);
    insert(
        resources,
        wall,
        Transform::from_position(Vec3::new(3.0, -60.0, 0.0)),
    );
    insert(
        resources,
        wall,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        resources,
        wall,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(2.0, 80.0, 60.0),
            // Frictionless, so a wall test measures the mechanic rather than rapier's Coulomb
            // friction — which alone holds a character pressed into a wall almost still.
            friction: 0.0,
            ..Default::default()
        },
    );

    let hero = character(resources, Vec3::new(0.2, 0.0, 0.0));
    // Frictionless on both sides: the combining rule takes the larger,
    // so a slick wall alone still leaves the character's own 0.4.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Collider>()
        && let Some(collider) = storage.get_mut(hero)
    {
        collider.friction = 0.0;
    }
    insert(resources, hero, Walk::default());
    insert(resources, hero, Touching::default());
    insert(resources, hero, Facing { direction: Vec3::X });
    hero
}

#[path = "standing/ground.rs"]
mod ground;
#[path = "standing/walking.rs"]
mod walking;
#[path = "standing/walls.rs"]
mod walls;
#[path = "standing/wall_run.rs"]
mod wall_run;
