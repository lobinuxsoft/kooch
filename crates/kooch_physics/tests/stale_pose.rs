//! The third failure of #1316, measured through the real schedule.
//!
//! 🔴 This is an integration test on purpose. The unit harness calls the systems by hand, so it
//! would pass whatever stage they are registered in — and the bug was never in the systems, it was
//! in WHEN they run. Only a frame driven by `Schedule` can tell the difference.

use glam::Vec3;
use kooch_core::app::App;
use kooch_core::run_state::Playing;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::Parent;
use kooch_ecs::transform::Transform;
use kooch_physics::components::KIND_STATIC;
use kooch_physics::{Collider, PhysicsBody, PhysicsPlugin, PhysicsWorld, SolverBody};

fn spawn(app: &mut App) -> Entity {
    let mut commands = app.resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(&mut app.resources).id();
    commands.apply(&mut app.resources);
    app.resources.insert(commands);
    entity
}

fn put<T: Component>(app: &mut App, entity: Entity, value: T) {
    if let Some(registry) = app.resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<T>()
    {
        storage.insert(entity, value);
    }
    let Some(archetypes) = app.resources.get_mut::<ArchetypeRegistry>() else {
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
    let next = archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<T>());
    archetypes.register_entity(entity, next);
}

fn solver_pose(app: &App, entity: Entity) -> Vec3 {
    let slot = app
        .resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<SolverBody>())
        .and_then(|storage| storage.get(entity))
        .map(SolverBody::slot)
        .expect("the body reached the solver");
    app.resources
        .get::<PhysicsWorld>()
        .and_then(|world| world.backend().get_transform(world.handle(slot)?))
        .expect("the slot has a pose")
        .0
}

/// A parent moved this frame reaches the solver this frame.
///
/// With the sync in `PreUpdate` and propagation in `PostUpdate`, the solver read the global
/// published before the previous step: the move landed a frame late, silently.
#[test]
fn a_parent_moved_this_frame_lands() {
    let mut app = App::new();
    app.add_plugin(kooch_ecs::plugin::EcsPlugin);
    app.add_plugin(PhysicsPlugin::new());
    // Where both plugins register their components.
    app.schedule
        .run_stage(kooch_core::stage::Stage::Startup, &mut app.resources);
    Playing::set(&mut app.resources, true);

    let parent = spawn(&mut app);
    put(&mut app, parent, Transform::default());
    // Static, so the solver never moves it: whatever pose it has came from the author.
    let child = spawn(&mut app);
    put(&mut app, child, Transform::default());
    put(
        &mut app,
        child,
        PhysicsBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
    );
    put(&mut app, child, Collider::default());
    put(&mut app, child, Parent { entity: parent });

    app.schedule.run_pre_physics(&mut app.resources);
    app.schedule.run_post_physics(&mut app.resources);

    // The move, the way gameplay makes it: the parent's LOCAL transform. The child's own transform
    // never changes, so nothing but propagation can tell the solver.
    if let Some(registry) = app.resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Transform>()
    {
        storage.insert(parent, Transform::from_position(Vec3::new(0.0, 0.0, -42.0)));
    }

    app.schedule.run_pre_physics(&mut app.resources);

    let pose = solver_pose(&app, child);
    assert!(
        pose.abs_diff_eq(Vec3::new(0.0, 0.0, -42.0), 1e-4),
        "the solver has it at {pose}, a frame behind the move",
    );
}
