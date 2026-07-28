//! Reads back what the solver ended up with and prints it.
//!
//! Everything here is a query against the world after the steps have
//! run: masses, centres of mass, the door's angle, whether the fragile
//! joint let go, and the debug-render segment counts per category.

use glam::Vec3;

use ome_core::prelude::*;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::transform::Transform;
use ome_physics::backend::DebugCategories;
use ome_physics::plugin::{
    CollisionStarted, CollisionStopped, ContactForce, JointBroke, PhysicsBody, PhysicsWorld,
};

/// How many *fixed steps* to run before reporting and quitting.
///
/// Steps, not frames. The default runner spins as fast as it can and
/// accumulates fixed steps from real elapsed time, so a frame count
/// measures how fast the machine is, not how long the scene simulated —
/// the first version of this example ran 240 frames in ten milliseconds
/// and reported, correctly, that nothing had moved.
///
/// At 60 Hz this is four seconds of simulated time, and it costs four
use super::{Cast, Heard, STEPS};

/// Shoves the sleds and spins the top, once, on the first step.
///
/// Not in `build_scene`: the bodies do not exist in the solver until the
/// sync pass has run, and there is no force component to author yet
/// (#567).
pub(super) fn launch(resources: &mut Resources) {
    let steps = resources.get::<Time>().map(Time::fixed_count).unwrap_or(0);
    if steps != 1 {
        return;
    }
    let Some(cast) = resources.remove::<Cast>() else {
        return;
    };
    for entity in [cast.slippery, cast.grippy].into_iter().flatten() {
        if let Some(handle) = handle_of(resources, entity)
            && let Some(world) = resources.get_mut::<PhysicsWorld>()
        {
            world
                .backend_mut()
                .set_linear_velocity(handle, Vec3::new(8.0, 0.0, 0.0));
        }
    }
    if let Some(spinner) = cast.spinner
        && let Some(handle) = handle_of(resources, spinner)
        && let Some(world) = resources.get_mut::<PhysicsWorld>()
    {
        world
            .backend_mut()
            .set_angular_velocity(handle, Vec3::new(0.0, 10.0, 0.0));
    }
    resources.insert(cast);
}

fn handle_of(resources: &Resources, entity: Entity) -> Option<ome_physics::BodyHandle> {
    let slot = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<PhysicsBody>()?
        .get(entity)
        .map(PhysicsBody::slot)?;
    resources.get::<PhysicsWorld>()?.handle(slot)
}

/// Accumulates what the solver reported this frame.
///
/// Runs every frame because the buffers swap every frame: a tally taken
/// only at the end would see the last frame and call it the whole run.
pub(super) fn listen(resources: &mut Resources) {
    let ghost = resources.get::<Cast>().and_then(|cast| cast.ghost);
    let mut heard = match resources.remove::<Heard>() {
        Some(heard) => heard,
        None => return,
    };

    if let Some(events) = resources.get::<Events<CollisionStarted>>() {
        for event in events.read() {
            heard.started += 1;
            if event.sensor {
                heard.sensor_started += 1;
            }
            if Some(event.a) == ghost || Some(event.b) == ghost {
                heard.ghost_detections += 1;
            }
        }
    }
    if let Some(events) = resources.get::<Events<CollisionStopped>>() {
        heard.stopped += events.read().count();
    }
    if let Some(events) = resources.get::<Events<ContactForce>>() {
        for event in events.read() {
            heard.forces += 1;
            heard.peak_force = heard.peak_force.max(event.total_force_magnitude);
        }
    }
    if let Some(events) = resources.get::<Events<JointBroke>>() {
        heard.joint_breaks += events.read().count();
    }

    resources.insert(heard);
}

pub(super) fn report(resources: &mut Resources) {
    let steps = resources.get::<Time>().map(Time::fixed_count).unwrap_or(0);
    if steps < STEPS {
        return;
    }

    let Some(cast) = resources.remove::<Cast>() else {
        return;
    };

    println!(
        "\n=== after {steps} physics steps ({:.1} s simulated) ===\n",
        steps as f32 / 60.0
    );

    println!("-- #618 mass means kilograms --");
    for (label, entity) in [
        ("1 m cube, authored 3 kg ", cast.falling),
        ("r=2 sphere, authored 3 kg", cast.big_sphere),
        ("compound + child collider", cast.compound),
    ] {
        let Some(entity) = entity else { continue };
        match mass_of(resources, entity) {
            Some((mass, com)) => println!("  {label}  ->  {mass:.4} kg, centre of mass {com:?}"),
            None => println!("  {label}  ->  no body"),
        }
    }

    println!("\n-- #560 joints --");
    if let Some(door) = cast.door {
        let angle = rotation_of(resources, door);
        println!("  hinged door, limit 0.4 rad  ->  swung {angle:.4} rad");
    }
    if let (Some(fuse), Some(joint)) = (cast.fuse, cast.fuse_joint) {
        let built = resources
            .get::<PhysicsWorld>()
            .map(|w| w.joints().is_built(joint))
            .unwrap_or(false);
        let y = position_of(resources, fuse).y;
        println!("  joint rated 0.02 under 5 kg ->  still attached: {built}, load at y={y:.3}");
    }
    println!(
        "  live joints in the solver   ->  {}",
        resources
            .get::<PhysicsWorld>()
            .map(|w| w.backend().joint_count())
            .unwrap_or(0),
    );

    println!("\n-- #623 collider material --");
    for (label, entity) in [
        ("friction 0.02, shoved at 8 m/s", cast.slippery),
        ("friction 1.50, shoved at 8 m/s", cast.grippy),
    ] {
        let Some(entity) = entity else { continue };
        println!(
            "  {label}  ->  slid {:.3} m",
            position_of(resources, entity).x
        );
    }
    if let Some(spinner) = cast.spinner {
        let left = handle_of(resources, spinner)
            .and_then(|h| {
                resources
                    .get::<PhysicsWorld>()?
                    .backend()
                    .angular_velocity(h)
            })
            .map(|v| v.length())
            .unwrap_or(0.0);
        println!("  spun at 10 rad/s, damping 4    ->  {left:.4} rad/s left");
    }

    println!("\n-- #561 what the solver reported --");
    if let Some(heard) = resources.get::<Heard>() {
        println!(
            "  collisions started / stopped   ->  {} / {}",
            heard.started, heard.stopped
        );
        println!(
            "  of those, sensor overlaps      ->  {}",
            heard.sensor_started
        );
        println!(
            "  contact-force events           ->  {} (peak {:.1} N)",
            heard.forces, heard.peak_force
        );
        println!(
            "  joint breaks                   ->  {}",
            heard.joint_breaks
        );
        println!(
            "  ghost detected the wall        ->  {} times",
            heard.ghost_detections
        );
    }
    if let Some(ghost) = cast.ghost {
        println!(
            "  ghost passed through it        ->  now at y={:.3}",
            position_of(resources, ghost).y,
        );
    }
    if let Some(trigger) = cast.trigger {
        println!(
            "  body fell through the trigger  ->  now at y={:.3}",
            position_of(resources, trigger).y,
        );
    }

    println!("\n-- #563 what the solver will draw --");
    for (label, categories) in [
        ("contacts        ", one(|c| &mut c.contacts)),
        ("centre of mass  ", one(|c| &mut c.body_axes)),
        ("joint anchors   ", one(|c| &mut c.joints)),
        ("broad-phase AABB", one(|c| &mut c.collider_aabbs)),
        ("collider shapes ", one(|c| &mut c.collider_shapes)),
        ("everything off  ", DebugCategories::default()),
    ] {
        let mut lines = Vec::new();
        if let Some(world) = resources.get::<PhysicsWorld>() {
            world.backend().debug_lines(categories, &mut lines);
        }
        println!("  {label}  ->  {} line segments", lines.len());
    }

    println!();
    resources.insert(cast);
    if let Some(events) = resources.get_mut::<Events<AppExit>>() {
        events.send(AppExit);
    }
}

/// One category switched on, the rest off.
fn one(pick: fn(&mut DebugCategories) -> &mut bool) -> DebugCategories {
    let mut categories = DebugCategories::default();
    *pick(&mut categories) = true;
    categories
}

fn mass_of(resources: &Resources, entity: Entity) -> Option<(f32, Vec3)> {
    let world = resources.get::<PhysicsWorld>()?;
    let slot = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<PhysicsBody>()?
        .get(entity)
        .map(PhysicsBody::slot)?;
    let handle = world.handle(slot)?;
    Some((
        world.backend().mass(handle)?,
        world.backend().center_of_mass(handle)?,
    ))
}

fn position_of(resources: &Resources, entity: Entity) -> Vec3 {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(entity))
        .map(|t| t.position)
        .unwrap_or_default()
}

fn rotation_of(resources: &Resources, entity: Entity) -> f32 {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(entity))
        .map(|t| t.rotation.angle_between(glam::Quat::IDENTITY))
        .unwrap_or_default()
}
