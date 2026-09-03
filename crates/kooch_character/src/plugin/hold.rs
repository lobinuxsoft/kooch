//! The sweep, the walk, the turn and the spring — one step of a
//! character, in the order they depend on each other.

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
use crate::facing::Facing;
use crate::grounded::Grounded;
use crate::plugin::{turn, walk};
use crate::walk::Walk;

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
    /// Where gameplay is steering, or [`Vec3::ZERO`] for "keep looking".
    /// Its length is the throttle.
    facing: Vec3,
    /// How this character walks, or `None` for one that is only held up.
    walk: Option<Walk>,
}

/// Rising faster than this and the spring lets go, in m/s.
///
/// Without it a jump is fought by its own damping the frame after it
/// starts — at 18 damping a 5 m/s launch is met with 90 m/s² of "come
/// back", and the character never leaves the floor. Above the threshold
/// the body is simply in the air, and gravity is the only thing acting
/// on it.
const RISING: f32 = 0.5;

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
    let mut goals = resources.remove::<walk::WalkGoals>().unwrap_or_default();
    let found: Vec<(Entity, Grounded)> = planned
        .iter()
        .map(|plan| (plan.entity, hold_one(&mut world, &mut goals, plan, dt)))
        .collect();
    resources.insert(world);
    resources.insert(goals);

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
    let facings = registry.get_cpu::<Facing>();
    let walks = registry.get_cpu::<Walk>();

    // Read first, then ask the field: `gravity_up` walks the same
    // storages this is holding.
    type Read = (
        Entity,
        SolverBody,
        CharacterController,
        Vec3,
        Quat,
        Vec3,
        Option<Walk>,
    );
    let standing: Vec<Read> = controllers
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
                facings
                    .and_then(|facings| facings.get(entity))
                    .map(|facing| facing.direction)
                    .unwrap_or(Vec3::ZERO),
                walks.and_then(|walks| walks.get(entity)).copied(),
            ))
        })
        .collect();

    standing
        .into_iter()
        .map(
            |(entity, body, controller, position, rotation, facing, walk)| {
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
                    facing,
                    walk,
                }
            },
        )
        .collect()
}

/// The whole mechanism, for one body.
fn hold_one(
    world: &mut PhysicsWorld,
    goals: &mut walk::WalkGoals,
    plan: &Planned,
    dt: f32,
) -> Grounded {
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
    let standing = hit
        .map(|hit| controller.stands_on(hit.normal, plan.up))
        .unwrap_or(false);

    // Stood on the surface, not on the field. A ramp is something to
    // stand *on*, and a character that walks up one bolt upright reads
    // as a sprite. Only when it counts as ground: aligning to a wall the
    // spring is merely pushing off would lay the character on its side.
    let stand = match (standing, hit) {
        (true, Some(hit)) => hit.normal.try_normalize().unwrap_or(plan.up),
        _ => plan.up,
    };

    // Before the turn, because the lean is drawn from it.
    let pushed = walk_one(world, goals, plan, standing, dt);
    turn_one(world, plan, stand, pushed, dt);

    let Some(hit) = hit else {
        return Grounded::default();
    };
    let gap = (plan.position - hit.point).dot(plan.up);

    let speed = world
        .linear_velocity(plan.body)
        .map(|velocity| velocity.dot(plan.up))
        .unwrap_or(0.0);
    // Leaving the ground under its own power. The spring would spend the
    // next frames pulling it straight back down, which is a jump that
    // never happens — see `RISING`. Ground is still reported as *found*
    // and not stood on, so a second jump has nothing to push off.
    if speed > RISING {
        return Grounded {
            standing: false,
            normal: hit.normal,
            distance: gap,
        };
    }

    // The spring pulls both ways. Only pushing would let the character
    // sail off the top of every bump instead of following the ground
    // down the far side, which is the whole reason this is a spring and
    // not a floor.
    //
    // Measured to the contact point rather than from the sweep's own
    // distance: on a slope the sphere stops early and `t` understates
    // the gap, which would make the spring shove the character off
    // every ramp it walked onto.
    let error = controller.ride_height - gap;
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

/// Chases the goal velocity, and reports the acceleration it used.
///
/// The report is what the lean is drawn from: a body tilts into what is
/// actually being applied to it, not into what was asked for and
/// clipped.
fn walk_one(
    world: &mut PhysicsWorld,
    goals: &mut walk::WalkGoals,
    plan: &Planned,
    standing: bool,
    dt: f32,
) -> Vec3 {
    let Some(steps) = plan.walk else {
        return Vec3::ZERO;
    };
    let velocity = world.linear_velocity(plan.body).unwrap_or(Vec3::ZERO);
    let across = velocity - plan.up * velocity.dot(plan.up);

    let pushed = match standing {
        true => {
            let wanted = walk::goal(plan.facing, plan.up, &steps);
            let goal = goals.chase(plan.entity, wanted, steps.acceleration, dt);
            walk::needed(goal, across, steps.max_force, dt)
        }
        // Nothing to push against, so nothing to brake with. The goal is
        // held at the real velocity so the landing frame chases reality
        // rather than spending a goal from before the jump.
        false => {
            goals.hold(plan.entity, across);
            walk::drift(plan.facing, across, plan.up, &steps, dt)
        }
    };

    if let Some(mass) = world.mass(plan.body) {
        world.apply_impulse(plan.body, pushed * mass * dt);
    }
    pushed
}

/// Stands the body on the local up, facing where it is steered.
///
/// Set rather than torqued, and the angular velocity zeroed with it: the
/// solver would otherwise keep whatever spin it had and turn the body
/// straight back out of the pose. See
/// [`turn_speed`](CharacterController::turn_speed) for why a character's
/// orientation is authored.
fn turn_one(world: &mut PhysicsWorld, plan: &Planned, stand: Vec3, pushed: Vec3, dt: f32) {
    let lean = plan.walk.map(|steps| steps.lean).unwrap_or(0.0);
    let up = turn::leaned(stand, pushed, plan.weight, lean);
    let wanted = turn::wanted(up, plan.facing, plan.rotation);
    let turned = turn::towards(plan.rotation, wanted, plan.controller.turn_speed, dt);
    if turned.abs_diff_eq(plan.rotation, 1e-6) {
        return;
    }
    world.set_rotation(plan.body, turned);
    world.set_angular_velocity(plan.body, Vec3::ZERO);
}
