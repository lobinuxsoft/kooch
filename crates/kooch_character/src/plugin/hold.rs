//! The spring, the sweep and the torque — one step of holding a
//! character up.

use glam::{Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::transform::Transform;
use kooch_gravity::gravity_at;
use kooch_physics::backend::{CollisionShape, ShapeAt};
use kooch_physics::plugin::{PhysicsWorld, SolverBody};

use crate::controller::CharacterController;
use crate::grounded::Grounded;

/// One character's worth of work, read before the world is borrowed.
struct Planned {
    entity: Entity,
    body: SolverBody,
    controller: CharacterController,
    position: Vec3,
    rotation: Quat,
    /// Which way is up *here* — the same answer the solver is using.
    up: Vec3,
    /// How hard the field is pulling, which the spring has to hold
    /// against before it holds anything else.
    weight: f32,
}

/// Sweeps for ground, holds the body at its ride height, keeps it
/// upright, and writes [`Grounded`].
pub fn hold_characters(resources: &mut Resources) {
    let planned = plan(resources);
    if planned.is_empty() {
        return;
    }
    let dt = resources
        .get::<Time>()
        .map(|time| time.fixed_delta_secs())
        .unwrap_or(1.0 / 60.0);

    // Taken out rather than borrowed: writing `Grounded` afterwards needs
    // the registry mutably, and the sweep needs the world.
    let Some(mut world) = resources.remove::<PhysicsWorld>() else {
        return;
    };
    let found: Vec<(Entity, Grounded)> = planned
        .iter()
        .map(|plan| (plan.entity, hold_one(&mut world, plan, dt)))
        .collect();
    resources.insert(world);

    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(storage) = registry.get_cpu_mut::<Grounded>() else {
        return;
    };
    for (entity, grounded) in found {
        storage.insert(entity, grounded);
    }
}

/// Every character, with the up that applies where it stands.
fn plan(resources: &Resources) -> Vec<Planned> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let (Some(controllers), Some(bodies), Some(transforms)) = (
        registry.get_cpu::<CharacterController>(),
        registry.get_cpu::<SolverBody>(),
        registry.get_cpu::<Transform>(),
    ) else {
        return Vec::new();
    };

    // Read first, then ask the field: `gravity_up` walks the same
    // storages this is holding.
    let standing: Vec<(Entity, SolverBody, CharacterController, Vec3, Quat)> = controllers
        .iter()
        .filter_map(|(&entity, controller)| {
            let body = *bodies.get(entity)?;
            let transform = transforms.get(entity)?;
            Some((
                entity,
                body,
                *controller,
                transform.position,
                transform.rotation,
            ))
        })
        .collect();

    standing
        .into_iter()
        .map(|(entity, body, controller, position, rotation)| {
            let pull = gravity_at(resources, position);
            Planned {
                entity,
                body,
                controller,
                position,
                rotation,
                // World up where nothing reaches, which is what the
                // solver is doing there too — not a fallback.
                up: (-pull).try_normalize().unwrap_or(Vec3::Y),
                weight: pull.length(),
            }
        })
        .collect()
}

/// The whole mechanism, for one body.
fn hold_one(world: &mut PhysicsWorld, plan: &Planned, dt: f32) -> Grounded {
    let controller = &plan.controller;
    let probe = CollisionShape::Sphere {
        radius: controller
            .probe_radius
            .max(kooch_physics::backend::MIN_EXTENT),
    };
    // Blind to itself. A downward sweep from a character's own centre
    // finds the character, every time.
    let filter = world.without(plan.body);
    let hit = world.sweep(
        ShapeAt::new(&probe, plan.position),
        -plan.up,
        controller.probe.max(0.0),
        filter,
    );

    upright(world, plan, dt);

    let Some(hit) = hit else {
        return Grounded::default();
    };
    // Measured to the contact point rather than from the sweep's own
    // distance: on a slope the sphere stops early and `t` understates
    // the gap, which would make the spring shove the character off
    // every ramp it walked onto.
    let gap = (plan.position - hit.point).dot(plan.up);
    let standing = controller.stands_on(hit.normal, plan.up);

    // The spring pulls both ways. Only pushing would let the character
    // sail off the top of every bump instead of following the ground
    // down the far side, which is the whole reason this is a spring and
    // not a floor.
    let error = controller.ride_height - gap;
    let speed = world
        .linear_velocity(plan.body)
        .map(|velocity| velocity.dot(plan.up))
        .unwrap_or(0.0);
    // Gravity is cancelled before the spring is asked for anything.
    // Without it the spring has to *lean* to hold the body up — it
    // settles wherever `error · stiffness` happens to equal `g`, so the
    // rest height is never the height in the Inspector, and it changes
    // with every planet the character walks onto.
    let acceleration = plan.weight + error * controller.stiffness - speed * controller.damping;
    if let Some(mass) = world.mass(plan.body) {
        world.apply_impulse(plan.body, plan.up * acceleration * mass * dt);
    }

    Grounded {
        standing,
        normal: hit.normal,
        distance: gap,
    }
}

/// Turns the body back towards the local up.
///
/// A torque rather than a written rotation: a character knocked over has
/// to *recover*, which is a correction that composes with whatever hit
/// it. Setting the rotation would teleport it upright through the hit.
fn upright(world: &mut PhysicsWorld, plan: &Planned, dt: f32) {
    let controller = &plan.controller;
    let facing = plan.rotation * Vec3::Y;
    let axis = facing.cross(plan.up);
    // Already upright, or exactly inverted. Inverted has no shortest arc
    // — every perpendicular axis turns it the same distance — so it is
    // nudged along one rather than left balanced on its head forever.
    let lean = match axis.try_normalize() {
        Some(axis) => axis * facing.angle_between(plan.up),
        None if facing.dot(plan.up) < 0.0 => {
            plan.up.any_orthonormal_vector() * std::f32::consts::PI
        }
        None => Vec3::ZERO,
    };

    // Only the tilt is damped, not the spin about up: damping that too
    // would fight the character turning to face where it is going, and
    // read as controls made of treacle.
    let spin = world.angular_velocity(plan.body).unwrap_or(Vec3::ZERO);
    let tilting = spin - plan.up * spin.dot(plan.up);

    let torque = lean * controller.upright_stiffness - tilting * controller.upright_damping;
    if torque.length_squared() > 1e-12 {
        world.apply_torque_impulse(plan.body, torque * dt);
    }
}
