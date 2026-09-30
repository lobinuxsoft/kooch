//! Does a vcam's `LensOverride` reach the render camera's `fov` through the REAL schedule?
//!
//! 🔴 The unit harness builds its `Resources` by hand and passed while the smoke test failed. This
//! drives `App` with `CameraPlugin`, the way a project does.

use glam::Vec3;
use kooch_camera::{CameraBrain, CameraTarget, Follow, HardLookAt, LensOverride, VirtualCamera};
use kooch_core::app::App;
use kooch_core::run_state::Playing;
use kooch_core::stage::Stage;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::transform::Transform;

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

fn fov(app: &App, camera: Entity) -> f32 {
    app.resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<PerspectiveCamera>()
        .unwrap()
        .get(camera)
        .unwrap()
        .fov
}

/// A vcam following a target, and a separate camera carrying the brain — the shape `New → Cameras`
/// makes, and the shape a project's scene has.
fn rig() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.add_plugin(kooch_ecs::plugin::EcsPlugin);
    app.add_plugin(kooch_camera::CameraPlugin);
    app.schedule.run_stage(Stage::Startup, &mut app.resources);
    Playing::set(&mut app.resources, true);

    let camera = spawn(&mut app);
    put(&mut app, camera, Transform::default());
    put(&mut app, camera, PerspectiveCamera::default());
    put(&mut app, camera, CameraBrain::default());

    let target = spawn(&mut app);
    put(&mut app, target, Transform::default());
    put(&mut app, target, CameraTarget::default());

    let vcam = spawn(&mut app);
    put(&mut app, vcam, Transform::default());
    put(&mut app, vcam, VirtualCamera::default());
    put(
        &mut app,
        vcam,
        Follow {
            offset: Vec3::Z * 5.0,
        },
    );
    put(&mut app, vcam, HardLookAt);
    (app, vcam, camera)
}

fn run(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.schedule.run_pre_physics(&mut app.resources);
        app.schedule.run_fixed_stages(&mut app.resources);
        app.schedule.run_post_physics(&mut app.resources);
    }
}

#[test]
fn an_asked_fov_reaches_the_camera() {
    let (mut app, vcam, camera) = rig();
    let authored = fov(&app, camera);
    put(
        &mut app,
        vcam,
        LensOverride {
            fov: 30.0,
            ..Default::default()
        },
    );

    run(&mut app, 300);

    let now = fov(&app, camera);
    assert!(
        (now - 30.0).abs() < 0.01,
        "asked for 30, the camera is at {now} (authored {authored})",
    );
}

/// The other half: asking for nothing must not drag the camera's own fov anywhere.
#[test]
fn asking_nothing_leaves_the_fov() {
    let (mut app, _vcam, camera) = rig();
    let authored = fov(&app, camera);
    run(&mut app, 300);
    let now = fov(&app, camera);
    assert!(
        (now - authored).abs() < 0.01,
        "nothing asked, yet the fov moved from {authored} to {now}",
    );
}
