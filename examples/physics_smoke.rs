//! Drives the physics subsystem through the real engine and reports what
//! the solver ended up with.
//!
//! The unit tests assert one thing each against a hand-built `Resources`.
//! This runs the actual `App` — plugins, schedule, fixed timestep — and
//! prints the numbers, which is the difference between "the function
//! returns what I expected" and "the engine does what I expected".
//!
//! Headless on purpose: no window, no GPU. Physics does not need either,
//! and a smoke test that needs a display is one that cannot run anywhere.
//!
//! Run with:
//!
//! ```text
//! cargo run --example physics_smoke --no-default-features \
//!     --features physics,physics-debug-render
//! ```

use glam::Vec3;

use ome_core::prelude::*;
use ome_core::run_state::Playing;
use ome_ecs::commands::Commands;
use ome_ecs::component::{Component, ComponentRegistry};
use ome_ecs::entity::Entity;
use ome_ecs::plugin::EcsPlugin;
use ome_ecs::transform::Transform;
use ome_physics::backend::DebugCategories;
use ome_physics::backend::PhysicsBackend;
use ome_physics::components::{
    Collider, JOINT_REVOLUTE, Joint, KIND_STATIC, RigidBody, SHAPE_CUBOID,
};
use ome_physics::plugin::{
    CollisionStarted, CollisionStopped, ContactForce, JointBroke, PhysicsBody, PhysicsPlugin,
    PhysicsWorld,
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
/// seconds of wall clock to get.
const STEPS: u64 = 240;

/// The entities the report reads back.
#[derive(Default)]
struct Cast {
    falling: Option<Entity>,
    big_sphere: Option<Entity>,
    compound: Option<Entity>,
    door: Option<Entity>,
    fuse: Option<Entity>,
    fuse_joint: Option<Entity>,
    /// Two identical cubes shoved at the same speed over floors that
    /// differ only in friction (#623).
    slippery: Option<Entity>,
    grippy: Option<Entity>,
    spinner: Option<Entity>,
    /// A trigger volume and the body falling through it (#561).
    trigger: Option<Entity>,
    /// Same collision groups as the wall, disjoint solver groups: detects
    /// it and is not stopped by it (#561).
    ghost: Option<Entity>,
}

/// What the solver reported over the whole run.
///
/// Accumulated as it arrives, because the event buffers are
/// double-buffered: reading only at the end would see the last frame's
/// events and nothing else.
#[derive(Default)]
struct Heard {
    started: usize,
    stopped: usize,
    sensor_started: usize,
    forces: usize,
    peak_force: f32,
    joint_breaks: usize,
    ghost_detections: usize,
}

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(EcsPlugin);
    app.add_plugin(PhysicsPlugin::new());
    app.insert_resource(Cast::default());
    app.add_system(Stage::Startup, build_scene);
    app.add_system(Stage::Update, launch);
    app.add_system(Stage::Update, listen);
    app.add_system(Stage::Last, report);
    Playing::set(app.resources_mut(), true);
    app.run();
}

// ---------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------

fn build_scene(resources: &mut Resources) {
    let mut cast = Cast::default();

    ground(resources);

    // #618: a 3 kg body has to weigh 3 kg whatever its collider is. The
    // big sphere is the case that used to weigh thirty-four.
    cast.falling = Some(body(
        resources,
        Vec3::new(0.0, 6.0, 0.0),
        RigidBody {
            mass: 3.0,
            ..Default::default()
        },
        cuboid(0.5),
    ));
    cast.big_sphere = Some(body(
        resources,
        Vec3::new(3.0, 6.0, 0.0),
        RigidBody {
            mass: 3.0,
            ..Default::default()
        },
        Collider {
            radius: 2.0,
            ..Default::default()
        },
    ));

    // #615 + #618: a child collider adds collision and no mass, so the
    // centre of mass stays on the parent rather than drifting out to it.
    let compound = body(
        resources,
        Vec3::new(-3.0, 6.0, 0.0),
        RigidBody {
            mass: 3.0,
            ..Default::default()
        },
        cuboid(0.5),
    );
    child_collider(resources, compound, Vec3::new(4.0, 0.0, 0.0));
    cast.compound = Some(compound);

    // #560: a hinge with limits. Anchored half a metre from the door's
    // centre of mass, because a hinge through the centre has no lever arm
    // and would not swing however free the joint is.
    let frame = static_body(resources, Vec3::new(0.0, 4.0, 6.0));
    let door = body(
        resources,
        Vec3::new(1.5, 4.0, 6.0),
        RigidBody {
            mass: 2.0,
            ..Default::default()
        },
        cuboid(0.5),
    );
    spawn_joint(
        resources,
        Joint {
            kind: JOINT_REVOLUTE,
            body_a: frame,
            body_b: door,
            axis: Vec3::Z,
            anchor_a: Vec3::new(1.0, 0.0, 0.0),
            anchor_b: Vec3::new(-0.5, 0.0, 0.0),
            limits_enabled: true,
            limit_min: -0.4,
            limit_max: 0.0,
            ..Default::default()
        },
    );
    cast.door = Some(door);

    // #560 again: a joint rated far below the load it is holding. It
    // should let go on the first loaded step and stay let go.
    let hook = static_body(resources, Vec3::new(0.0, 8.0, -6.0));
    let fuse = body(
        resources,
        Vec3::new(0.0, 7.0, -6.0),
        RigidBody {
            mass: 5.0,
            ..Default::default()
        },
        cuboid(0.5),
    );
    cast.fuse_joint = Some(spawn_joint(
        resources,
        Joint {
            body_a: hook,
            body_b: fuse,
            anchor_a: Vec3::new(0.0, -1.0, 0.0),
            breakable: true,
            break_impulse: 0.02,
            ..Default::default()
        },
    ));
    cast.fuse = Some(fuse);

    // #623: friction was not authorable at all until now — every collider
    // silently took rapier's 0.5. These two differ in nothing else.
    cast.slippery = Some(sled(resources, -20.0, 0.02));
    cast.grippy = Some(sled(resources, 20.0, 1.5));

    // #623: angular damping, which #618's "it rotates sluggishly" could
    // not have been, because nothing was damping anything.
    cast.spinner = Some(body(
        resources,
        Vec3::new(0.0, 20.0, 0.0),
        RigidBody {
            mass: 1.0,
            angular_damping: 4.0,
            ..Default::default()
        },
        cuboid(0.5),
    ));

    // #561: a trigger volume, which reports overlap and never pushes.
    body(
        resources,
        Vec3::new(0.0, 3.0, 12.0),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(3.0, 0.5, 3.0),
            sensor: true,
            collision_events: true,
            ..Default::default()
        },
    );
    cast.trigger = Some(body(
        resources,
        Vec3::new(0.0, 8.0, 12.0),
        RigidBody {
            mass: 1.0,
            ..Default::default()
        },
        cuboid(0.5),
    ));

    // #561: the floor everything else rests on reports hard landings.
    body(
        resources,
        Vec3::new(0.0, -1.0, -12.0),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(6.0, 1.0, 6.0),
            collision_events: true,
            contact_force_events: true,
            contact_force_threshold: 1.0,
            ..Default::default()
        },
    );
    body(
        resources,
        Vec3::new(0.0, 14.0, -12.0),
        RigidBody {
            mass: 20.0,
            ..Default::default()
        },
        cuboid(0.5),
    );

    // #561: matching collision groups, disjoint solver groups — the wall
    // is seen and does not stop anything.
    body(
        resources,
        Vec3::new(0.0, 0.0, 20.0),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(4.0, 0.5, 4.0),
            solver_memberships: 0b0001,
            solver_filter: 0b0001,
            collision_events: true,
            ..Default::default()
        },
    );
    cast.ghost = Some(body(
        resources,
        Vec3::new(0.0, 5.0, 20.0),
        RigidBody {
            mass: 1.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            solver_memberships: 0b0010,
            solver_filter: 0b0010,
            collision_events: true,
            ..Default::default()
        },
    ));

    resources.insert(cast);
    resources.insert(Heard::default());
    tracing::info!("scene built");
}

/// A cube on its own strip of floor, both at `friction`, shoved along +X.
///
/// Its own strip so the two comparisons cannot touch each other, and the
/// push is applied on the first report-free frame rather than here: the
/// body does not exist in the solver until the sync pass has run.
fn sled(resources: &mut Resources, z: f32, friction: f32) -> Entity {
    body(
        resources,
        Vec3::new(0.0, -1.0, z),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(200.0, 1.0, 4.0),
            friction,
            ..Default::default()
        },
    );
    body(
        resources,
        Vec3::new(0.0, 0.5, z),
        RigidBody {
            mass: 1.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            friction,
            ..Default::default()
        },
    )
}

fn ground(resources: &mut Resources) -> Entity {
    body(
        resources,
        Vec3::new(0.0, -1.0, 0.0),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(20.0, 1.0, 20.0),
            ..Default::default()
        },
    )
}

fn static_body(resources: &mut Resources, position: Vec3) -> Entity {
    body(
        resources,
        position,
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        cuboid(0.25),
    )
}

fn cuboid(half: f32) -> Collider {
    Collider {
        shape: SHAPE_CUBOID,
        half_extents: Vec3::splat(half),
        ..Default::default()
    }
}

fn body(
    resources: &mut Resources,
    position: Vec3,
    rigid_body: RigidBody,
    collider: Collider,
) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, Transform::from_position(position));
    insert(resources, entity, rigid_body);
    insert(resources, entity, collider);
    entity
}

fn child_collider(resources: &mut Resources, parent: Entity, offset: Vec3) {
    let child = spawn(resources);
    insert(resources, child, Transform::from_position(offset));
    insert(resources, child, cuboid(0.5));
    insert(
        resources,
        child,
        ome_ecs::hierarchy::Parent { entity: parent },
    );
}

fn spawn_joint(resources: &mut Resources, joint: Joint) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, joint);
    entity
}

fn spawn(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().expect("Commands");
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
    let Some(archetypes) = resources.get_mut::<ome_ecs::archetype_registry::ArchetypeRegistry>()
    else {
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

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// Shoves the sleds and spins the top, once, on the first step.
///
/// Not in `build_scene`: the bodies do not exist in the solver until the
/// sync pass has run, and there is no force component to author yet
/// (#567).
fn launch(resources: &mut Resources) {
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
fn listen(resources: &mut Resources) {
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

fn report(resources: &mut Resources) {
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
