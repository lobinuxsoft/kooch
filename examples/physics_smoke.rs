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
use ome_physics::components::{
    Collider, JOINT_REVOLUTE, Joint, KIND_STATIC, RigidBody, SHAPE_CUBOID,
};
use ome_physics::plugin::{PhysicsBody, PhysicsPlugin, PhysicsWorld};

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
}

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(EcsPlugin);
    app.add_plugin(PhysicsPlugin::new());
    app.insert_resource(Cast::default());
    app.add_system(Stage::Startup, build_scene);
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

    resources.insert(cast);
    tracing::info!("scene built");
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
