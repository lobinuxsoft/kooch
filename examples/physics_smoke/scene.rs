//! Builds the scene the smoke drives: the sleds, the ground, the door,
//! the breakable joint and the compound body.
//!
//! Separate from the reporting half because they fail differently — a
//! wrong number here means the scene was authored wrong, and a wrong
//! number there means the solver disagreed with what was authored.

use glam::Vec3;

use kooch_core::prelude::*;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::EntityRef;
use kooch_ecs::transform::Transform;
use kooch_physics::components::{
    Collider, JOINT_REVOLUTE, Joint, KIND_STATIC, RigidBody, SHAPE_CUBOID,
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
use super::{Cast, Heard};

pub(super) fn build_scene(resources: &mut Resources) {
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
            body_a: Some(EntityRef::live(frame)),
            body_b: Some(EntityRef::live(door)),
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
            body_a: Some(EntityRef::live(hook)),
            body_b: Some(EntityRef::live(fuse)),
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
        kooch_ecs::hierarchy::Parent { entity: parent },
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
    let Some(archetypes) = resources.get_mut::<kooch_ecs::archetype_registry::ArchetypeRegistry>()
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
